//! Native audio capture and render backends.
//!
//! Audio frames are exchanged with the session over bounded `tokio` mpsc
//! channels of interleaved 16-bit PCM. Everything here runs on native threads;
//! no PCM ever reaches the managed layer.
//!
//! * [`NullBackend`] — always compiled. Captures silence at the frame cadence
//!   and discards playout. Lets the whole pipeline run headless (servers, CI,
//!   tests) with no audio hardware.
//! * `CpalBackend` — behind the `cpal-backend` feature. Drives real input/output
//!   devices. Requires platform audio dev headers at build time (e.g.
//!   `libasound2-dev` on Linux).

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
        AudioBackendKind::Cpal => {
            #[cfg(feature = "cpal-backend")]
            {
                Ok(Box::new(cpal_backend::CpalBackend))
            }
            #[cfg(not(feature = "cpal-backend"))]
            {
                Err(Error::Unsupported(
                    "Cpal backend requested but the `cpal-backend` feature was not compiled in"
                        .into(),
                ))
            }
        }
        AudioBackendKind::Auto => {
            #[cfg(feature = "cpal-backend")]
            {
                Ok(Box::new(cpal_backend::CpalBackend))
            }
            #[cfg(not(feature = "cpal-backend"))]
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

#[cfg(feature = "cpal-backend")]
mod cpal_backend {
    use super::*;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    pub struct CpalBackend;

    struct CpalDevice {
        stop: Arc<AtomicBool>,
        handle: Option<JoinHandle<()>>,
    }

    impl AudioDevice for CpalDevice {
        fn stop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
        }
    }

    impl Drop for CpalDevice {
        fn drop(&mut self) {
            self.stop();
        }
    }

    impl AudioBackend for CpalBackend {
        fn start(&self, params: &AudioParams) -> Result<AudioStreams> {
            let (cap_tx, cap_rx) = mpsc::channel::<Frame>(CHANNEL_DEPTH);
            let (play_tx, play_rx) = mpsc::channel::<Frame>(CHANNEL_DEPTH);

            let frame_len = params.frame_len();
            let channels = params.channels as usize;
            let config = cpal::StreamConfig {
                channels: params.channels,
                sample_rate: cpal::SampleRate(params.sample_rate),
                buffer_size: cpal::BufferSize::Default,
            };

            let stop = Arc::new(AtomicBool::new(false));
            let stop_thread = stop.clone();

            // cpal streams are not `Send` on some platforms, so build, run and
            // hold them on one dedicated thread for the device's whole lifetime.
            let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();

            let handle = std::thread::Builder::new()
                .name("qm-cpal-audio".into())
                .spawn(move || {
                    let host = cpal::default_host();

                    let build = || -> Result<(cpal::Stream, cpal::Stream)> {
                        let input = host
                            .default_input_device()
                            .ok_or_else(|| Error::Audio("no default input device".into()))?;
                        let output = host
                            .default_output_device()
                            .ok_or_else(|| Error::Audio("no default output device".into()))?;

                        // Capture: accumulate f32 samples into i16 frames.
                        let mut acc: Vec<i16> = Vec::with_capacity(frame_len);
                        let cap_tx_cb = cap_tx.clone();
                        let input_stream = input
                            .build_input_stream(
                                &config,
                                move |data: &[f32], _| {
                                    for &s in data {
                                        acc.push((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16);
                                        if acc.len() == frame_len {
                                            let frame = std::mem::replace(
                                                &mut acc,
                                                Vec::with_capacity(frame_len),
                                            );
                                            let _ = cap_tx_cb.try_send(frame.into_boxed_slice());
                                        }
                                    }
                                },
                                |err| tracing::warn!("cpal input error: {err}"),
                                None,
                            )
                            .map_err(|e| Error::Audio(format!("build input stream: {e}")))?;

                        // Playout: feed from the session, concealing underruns with silence.
                        let mut play_rx = play_rx;
                        let mut leftover: Vec<i16> = Vec::new();
                        let mut pos = 0usize;
                        let output_stream = output
                            .build_output_stream(
                                &config,
                                move |out: &mut [f32], _| {
                                    for slot in out.iter_mut() {
                                        if pos >= leftover.len() {
                                            match play_rx.try_recv() {
                                                Ok(frame) => {
                                                    leftover = frame.into_vec();
                                                    pos = 0;
                                                }
                                                Err(_) => {
                                                    *slot = 0.0;
                                                    continue;
                                                }
                                            }
                                        }
                                        *slot = leftover[pos] as f32 / i16::MAX as f32;
                                        pos += 1;
                                    }
                                },
                                |err| tracing::warn!("cpal output error: {err}"),
                                None,
                            )
                            .map_err(|e| Error::Audio(format!("build output stream: {e}")))?;

                        input_stream
                            .play()
                            .map_err(|e| Error::Audio(format!("play input: {e}")))?;
                        output_stream
                            .play()
                            .map_err(|e| Error::Audio(format!("play output: {e}")))?;
                        let _ = channels; // reserved for future per-channel handling
                        Ok((input_stream, output_stream))
                    };

                    match build() {
                        Ok(streams) => {
                            let _ = ready_tx.send(Ok(()));
                            // Keep streams alive until stop is requested.
                            while !stop_thread.load(Ordering::Relaxed) {
                                std::thread::sleep(std::time::Duration::from_millis(50));
                            }
                            drop(streams);
                        }
                        Err(e) => {
                            let _ = ready_tx.send(Err(e));
                        }
                    }
                })
                .map_err(|e| Error::Audio(format!("spawn cpal thread: {e}")))?;

            // Surface device-build errors synchronously.
            match ready_rx.recv() {
                Ok(Ok(())) => {}
                Ok(Err(e)) => return Err(e),
                Err(_) => return Err(Error::Audio("cpal thread exited before readiness".into())),
            }

            Ok(AudioStreams {
                captured: cap_rx,
                to_play: play_tx,
                device: Box::new(CpalDevice {
                    stop,
                    handle: Some(handle),
                }),
            })
        }
    }
}
