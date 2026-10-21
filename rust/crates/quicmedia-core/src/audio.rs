//! Native audio capture and render backends.
//!
//! Audio frames are exchanged with the session over bounded `tokio` mpsc
//! channels of interleaved 16-bit PCM. Everything here runs on native threads;
//! no PCM ever reaches the managed layer.
//!
//! * [`NullBackend`] — always compiled. Captures silence at the frame cadence
//!   and discards playout. Lets the whole pipeline run headless (servers, CI,
//!   tests) with no audio hardware.
//! * `MiniaudioBackend` — behind the `miniaudio-backend` feature. Drives a real
//!   full-duplex device via the vendored miniaudio single-header (compiled by
//!   `build.rs`). miniaudio loads the platform audio API at runtime, so no audio
//!   `-dev` packages are needed at build time, and it covers Windows, macOS,
//!   Linux, Android and iOS from one source.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use tokio::sync::mpsc;

use crate::config::{AudioBackendKind, AudioParams};
use crate::{Error, Result};

/// One frame of interleaved 16-bit PCM.
pub type Frame = Box<[i16]>;

/// Depth of the capture and playout channels (in frames).
const CHANNEL_DEPTH: usize = 16;

/// Handle that keeps a running backend alive. Dropping it stops the devices.
pub trait AudioDevice: Send {
    /// Stop the device and release resources. Also called on drop.
    fn stop(&mut self);
}

/// The session's view of a started backend.
pub struct AudioStreams {
    /// Frames captured from the microphone (backend → session).
    pub captured: mpsc::Receiver<Frame>,
    /// Frames to render to the speaker (session → backend).
    pub to_play: mpsc::Sender<Frame>,
    device: Box<dyn AudioDevice>,
}

impl AudioStreams {
    /// Explicitly stop the underlying device.
    pub fn stop(mut self) {
        self.device.stop();
    }

    /// Split into (capture receiver, playout sender, device-keepalive).
    /// The device must be kept alive for capture/render to continue.
    pub fn into_parts(
        self,
    ) -> (
        mpsc::Receiver<Frame>,
        mpsc::Sender<Frame>,
        Box<dyn AudioDevice>,
    ) {
        (self.captured, self.to_play, self.device)
    }
}

/// A capture/render backend.
pub trait AudioBackend: Send + Sync {
    fn start(&self, params: &AudioParams) -> Result<AudioStreams>;
}

/// Select a backend according to `kind` and compiled features.
pub fn make_backend(kind: AudioBackendKind) -> Result<Box<dyn AudioBackend>> {
    match kind {
        AudioBackendKind::Null => Ok(Box::new(NullBackend)),
        AudioBackendKind::Miniaudio => {
            #[cfg(feature = "miniaudio-backend")]
            {
                Ok(Box::new(miniaudio_backend::MiniaudioBackend))
            }
            #[cfg(not(feature = "miniaudio-backend"))]
            {
                Err(Error::Unsupported(
                    "miniaudio backend requested but the `miniaudio-backend` feature was not compiled in"
                        .into(),
                ))
            }
        }
        AudioBackendKind::Auto => {
            #[cfg(feature = "miniaudio-backend")]
            {
                Ok(Box::new(miniaudio_backend::MiniaudioBackend))
            }
            #[cfg(not(feature = "miniaudio-backend"))]
            {
                Ok(Box::new(NullBackend))
            }
        }
    }
}

/// Headless backend: silence in, discard out.
pub struct NullBackend;

struct NullDevice {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl AudioDevice for NullDevice {
    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for NullDevice {
    fn drop(&mut self) {
        self.stop();
    }
}

impl AudioBackend for NullBackend {
    fn start(&self, params: &AudioParams) -> Result<AudioStreams> {
        let (cap_tx, cap_rx) = mpsc::channel::<Frame>(CHANNEL_DEPTH);
        let (play_tx, mut play_rx) = mpsc::channel::<Frame>(CHANNEL_DEPTH);

        let frame_len = params.frame_len();
        let frame_ms = params.frame_ms as u64;
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();

        let handle = std::thread::Builder::new()
            .name("qm-null-audio".into())
            .spawn(move || {
                let period = std::time::Duration::from_millis(frame_ms);
                while !stop_thread.load(Ordering::Relaxed) {
                    std::thread::sleep(period);
                    // Produce one silent capture frame (best-effort).
                    let frame = vec![0i16; frame_len].into_boxed_slice();
                    let _ = cap_tx.try_send(frame);
                    // Drain and discard anything queued for playout.
                    while play_rx.try_recv().is_ok() {}
                }
            })
            .map_err(|e| Error::Audio(format!("spawn null audio thread: {e}")))?;

        Ok(AudioStreams {
            captured: cap_rx,
            to_play: play_tx,
            device: Box::new(NullDevice {
                stop,
                handle: Some(handle),
            }),
        })
    }
}

#[cfg(feature = "miniaudio-backend")]
mod miniaudio_backend {
    use super::*;
    use std::os::raw::c_void;

    #[repr(C)]
    struct QmAudioDevice {
        _private: [u8; 0],
    }

    extern "C" {
        fn qm_audio_open(
            sample_rate: u32,
            channels: u16,
            cb: extern "C" fn(*mut c_void, *const i16, *mut i16, u32),
            user: *mut c_void,
            out: *mut *mut QmAudioDevice,
        ) -> i32;
        fn qm_audio_close(dev: *mut QmAudioDevice);
    }

    /// Per-device state owned by Rust and borrowed by the C audio thread.
    /// Only the audio thread touches it, and only between open and close.
    struct MaState {
        cap_tx: mpsc::Sender<Frame>,
        cap_acc: Vec<i16>,
        frame_len: usize,
        play_rx: mpsc::Receiver<Frame>,
        play_leftover: Vec<i16>,
        play_pos: usize,
        channels: usize,
    }

    /// Invoked on miniaudio's audio thread with interleaved s16 in/out buffers.
    extern "C" fn data_callback(
        user: *mut c_void,
        input: *const i16,
        output: *mut i16,
        frames: u32,
    ) {
        if user.is_null() {
            return;
        }
        // SAFETY: `user` is the Box<MaState> pointer passed to qm_audio_open; it
        // stays valid until qm_audio_close (which joins the audio thread) returns,
        // and only this callback (on one thread) dereferences it.
        let st = unsafe { &mut *(user as *mut MaState) };
        let total = frames as usize * st.channels;

        // Capture: accumulate into frame_len chunks and hand them to the session.
        if !input.is_null() {
            let inp = unsafe { std::slice::from_raw_parts(input, total) };
            st.cap_acc.extend_from_slice(inp);
            while st.cap_acc.len() >= st.frame_len {
                let frame: Frame = st
                    .cap_acc
                    .drain(..st.frame_len)
                    .collect::<Vec<i16>>()
                    .into_boxed_slice();
                let _ = st.cap_tx.try_send(frame);
            }
        }

        // Playout: fill from the session, concealing underruns with silence.
        if !output.is_null() {
            let out = unsafe { std::slice::from_raw_parts_mut(output, total) };
            for slot in out.iter_mut() {
                if st.play_pos >= st.play_leftover.len() {
                    match st.play_rx.try_recv() {
                        Ok(frame) => {
                            st.play_leftover = frame.into_vec();
                            st.play_pos = 0;
                        }
                        Err(_) => {
                            *slot = 0;
                            continue;
                        }
                    }
                }
                *slot = st.play_leftover[st.play_pos];
                st.play_pos += 1;
            }
        }
    }

    pub struct MiniaudioBackend;

    struct MiniaudioDevice {
        handle: *mut QmAudioDevice,
        state: *mut MaState,
    }

    // The raw pointers are owned solely by this struct; the audio thread only
    // borrows `state` between open and close.
    unsafe impl Send for MiniaudioDevice {}

    impl AudioDevice for MiniaudioDevice {
        fn stop(&mut self) {
            if !self.handle.is_null() {
                // Joins the audio thread; no callback runs after this returns.
                unsafe { qm_audio_close(self.handle) };
                self.handle = std::ptr::null_mut();
            }
            if !self.state.is_null() {
                // SAFETY: reclaim the Box now that no callback can reference it.
                unsafe { drop(Box::from_raw(self.state)) };
                self.state = std::ptr::null_mut();
            }
        }
    }

    impl Drop for MiniaudioDevice {
        fn drop(&mut self) {
            self.stop();
        }
    }

    impl AudioBackend for MiniaudioBackend {
        fn start(&self, params: &AudioParams) -> Result<AudioStreams> {
            let (cap_tx, cap_rx) = mpsc::channel::<Frame>(CHANNEL_DEPTH);
            let (play_tx, play_rx) = mpsc::channel::<Frame>(CHANNEL_DEPTH);

            let state = Box::new(MaState {
                cap_tx,
                cap_acc: Vec::with_capacity(params.frame_len() * 2),
                frame_len: params.frame_len(),
                play_rx,
                play_leftover: Vec::new(),
                play_pos: 0,
                channels: params.channels as usize,
            });
            let state_ptr = Box::into_raw(state);

            let mut handle: *mut QmAudioDevice = std::ptr::null_mut();
            let rc = unsafe {
                qm_audio_open(
                    params.sample_rate,
                    params.channels,
                    data_callback,
                    state_ptr as *mut c_void,
                    &mut handle,
                )
            };
            if rc != 0 {
                // SAFETY: open failed, so no callback will ever run; reclaim the Box.
                unsafe { drop(Box::from_raw(state_ptr)) };
                return Err(Error::Audio(format!(
                    "miniaudio device open failed (code {rc})"
                )));
            }

            Ok(AudioStreams {
                captured: cap_rx,
                to_play: play_tx,
                device: Box::new(MiniaudioDevice {
                    handle,
                    state: state_ptr,
                }),
            })
        }
    }
}
