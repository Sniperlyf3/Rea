# QuicMedia architecture

A QUIC-based real-time **voice engine**. One engine owns many QUIC connections,
each carrying several logical audio streams. The entire media path lives in a
native Rust core; the C# and C++ bindings are **control-plane only** and never
touch a media byte.

```
                 ┌──────────────── one Engine (native) ────────────────┐
   mic ──capture─┤ conn A ─▶ stream ─encode─▶ DATAGRAM ─┐               │
                 │                                       │ route (native)│
                 │ conn B ◀───── DATAGRAM ◀──────────────┤──▶ conn C ───┼─▶ speaker
                 │ conn C ◀───── DATAGRAM ◀──────────────┘   (decode)   │   (receiver)
                 │        (SFU: forward encoded packets, no decode)     │
                 └──────────────────────────────────────────────────────┘
   control plane (open / set-active / route / bind)  ◀── C# or C++ (no media)
```

## Roles (one library, three jobs)

- **Capturing client** — `add_capture_stream`: mic → Opus → the connection's
  outbound stream.
- **Receiving client** — `bind_playback`: an inbound stream → jitter buffer →
  decode → speaker.
- **SFU** — `add_route(src → dst)`: an inbound stream's **encoded** packets are
  fanned out to one or many downstream connections entirely in native code (no
  decode, no managed-heap crossing).

A node can mix roles (e.g. capture + forward). Tuned for low-data voice: Opus
VoIP, **120 ms** ptime, **~8 kbps** by default.

## Per-stream active flag

Each `(connection, stream)` has `want_local` (our desire) and `want_remote`
(the peer's, from control). The producer transmits only while **both** are true;
both default true, so media flows on open until a side opts out. Either peer can
call `set_stream_active(false)` → a `SetActive` control message → the producer
stops sending **and** stops encoding that stream (CPU + bandwidth). When nothing
is active, the engine stops the audio device entirely. For an SFU this is
per-subscriber fan-out control: a downstream deactivating only stops its own
forward. The control plane (which stream is active) is driven from C#/C++; the
data plane (forwarding, encode/decode) stays native.

## Crate layout (`rust/`)

| Crate | Output | Responsibility |
|-------|--------|----------------|
| `quicmedia-core` | rlib | Engine, connections, routing, codecs, audio. |
| `quicmedia-ffi`  | cdylib + staticlib (`quicmedia`) | Flat C ABI over the engine. |

### `quicmedia-core` modules

- **`config`** — `AudioParams`, `SecurityConfig`, enums.
- **`security`** — self-signed cert generation (rcgen), rustls config, and the
  pluggable pinning `ServerCertVerifier` (insecure / exact cert / SHA-256).
- **`transport`** — quinn client/server endpoint construction.
- **`framing`** — the 10-byte datagram header with `stream_id` (framing v2).
- **`control`** — the reliable stream-management protocol (`Open`/`SetActive`/
  `Close`), self-delimiting tag+body messages; a 1-byte purpose tag separates
  the control stream from the user data channel.
- **`jitter`** — the fixed-target playout/jitter buffer (reorder, dedup,
  conceal), per inbound stream.
- **`codec`** — Opus (`opus-codec` feature, bundled libopus) + PCM passthrough;
  codec ↔ payload-type mapping.
- **`audio`** — native capture/render backends: `NullBackend` (always) and
  `MiniaudioBackend` (`miniaudio-backend` feature; vendored single-header).
- **`datachannel`** — the optional reliable data channel.
- **`conn`** — `PeerConn`: one QUIC connection as encoded-packet transport +
  control. Codec/device-free, so it is reusable by the SFU.
- **`engine`** — `Engine`: owns the connections, the native routing table, and
  the audio hub; the public API + events.

## Data flow

- **Inbound** (`dispatch_task`): a datagram is parsed to `(stream_id, header,
  payload)`; the payload is (a) pushed into the jitter buffer of a bound playback
  stream, and/or (b) forwarded — a zero-copy `Bytes` slice — to each routed
  downstream connection via `send_packet`, which drops it if that downstream
  stream is inactive.
- **Outbound capture** (`audio_pump`): the engine opens one miniaudio device
  when a binding is active; each captured frame is encoded per active capture
  stream and sent, and active playback streams are decoded and mixed to the
  speaker. Dropping the device stops capture when nothing is active.

## Threading model

- The engine owns a multi-threaded `tokio` runtime.
- `dispatch_task` routes inbound packets; `event_task` turns connection events
  into app callbacks; `audio_pump` owns the device lifecycle and per-frame
  encode/decode.
- Each `PeerConn` runs a datagram receive loop, a control reader/writer, an
  optional data-channel reader/writer, and a disconnect watcher.
- Audio hardware runs on miniaudio's own native thread; PCM crosses to the async
  world over bounded `tokio::mpsc` channels. Lock ordering is hub → connections
  (pump) with no inverse, so there is no deadlock.

## Transport details

- **QUIC via quinn + rustls (ring), TLS 1.3 only.**
- **Media:** one unreliable `DATAGRAM` per frame = `[10-byte header][Opus
  payload]`. At 120 ms / 8 kbps that is ≈ 130 bytes; oversized frames are dropped
  and counted rather than fragmented.
- **Control:** a bidirectional stream (purpose tag 0); **data channel**: a
  bidirectional stream (purpose tag 1), `[u32 len][payload]` framed.
- Keep-alive 5 s; idle timeout 30 s.

## Security

Prototype default is a self-signed certificate plus a pluggable verifier:
`Insecure` (dev), `PinnedCert` (exact DER), `PinnedSha256` (compact cert pin).
The verifier is an ordinary rustls component, so moving to full WebPKI/mTLS is a
localized change. Exchanging the pin out of band is the application's job.

## C ABI boundary (`quicmedia-ffi`)

Opaque `QmEngine*`; all functions return `int32_t` status and never unwind.
Config/stats pass as `#[repr(C)]` structs; strings are UTF-8; cert/key/pin are
`(ptr, len)` pairs copied on entry. Events are C function pointers with an opaque
`user` pointer (`on_connection`, `on_stream_opened`, `on_stream_active`,
`on_stream_closed`, `on_data`, `on_error`, …). Authoritative header:
`rust/include/quicmedia.h` (regenerable with cbindgen).

## Bindings

- **C# (`dotnet/QuicMedia`)** — `QuicMediaEngine` over source-generated
  `[LibraryImport]` P/Invoke; events via `[UnmanagedCallersOnly]` trampolines
  (the `user` pointer is a `GCHandle`). Packaged as a multi-RID NuGet
  (`runtimes/<rid>/native`).
- **C++ (`cpp/`)** — header-only `quicmedia::Engine` RAII wrapper
  (`quicmedia/quicmedia.hpp`) over the same C ABI, with `std::function`
  callbacks. Shipped as a CMake package: `find_package(quicmedia)` →
  `quicmedia::quicmedia`, bundling the headers + the prebuilt native library,
  analogous to the NuGet.

Media never appears in either binding by construction — the ABI exposes only
control, stats, and data-channel bytes.

## Packaging & build

- `build/build-native.(sh|ps1)` → `dotnet/QuicMedia/runtimes/<rid>/native/`.
- `build/build-android.sh` → Android ABIs via cargo-ndk.
- `build/pack-nuget.sh` → the NuGet. `build/build-cpp.sh` → native lib + C++
  example; `cmake --install` stages the C++ package.

### Target matrix

| RID | Rust target | Notes |
|-----|-------------|-------|
| `win-x64` | `x86_64-pc-windows-msvc` | `quicmedia.dll` |
| `linux-x64` | `x86_64-unknown-linux-gnu` | `libquicmedia.so`; miniaudio needs no `-dev` packages |
| `osx-arm64` / `osx-x64` | `aarch64/x86_64-apple-darwin` | `libquicmedia.dylib` |
| `android-arm64` / `android-x64` | `aarch64/x86_64-linux-android` | `.so` via cargo-ndk |
| `ios-arm64` | `aarch64-apple-ios` | static `libquicmedia.a` (roadmap) |

## Feature flags

- `opus-codec` — real Opus (bundled libopus; needs a C toolchain + cmake).
- `miniaudio-backend` — real device audio via vendored miniaudio (header-only C,
  built by `build.rs`; no audio `-dev` packages).

Without them the core still builds headless with PCM passthrough + the null
backend — how CI exercises the full QUIC/SFU pipeline.

## Verification

- Unit tests: framing v2 round-trip, control (de)serialize, sequence rollover,
  jitter ordering/dedup/conceal, codec round-trip, security config.
- Integration (`tests/engine_sfu.rs`): three engines over loopback — sender →
  SFU → receiver — complete pinned TLS 1.3 handshakes, the SFU forwards encoded
  packets natively, and deactivating the receiver's stream stops the forward.

## Roadmap

1. **Audio:** capture/playback-only device selection (don't open the mic on a
   pure receiver), adaptive jitter target, Opus in-band FEC + DTX, echo
   cancellation.
2. **Congestion & pacing:** media-aware pacing, bandwidth estimates in stats.
3. **Video:** a video payload type, hardware codecs, keyframe/NACK/PLI.
4. **Mobile/iOS:** iOS static-link packaging + a MAUI sample.
5. **Security:** optional full PKI / mutual TLS verifier.
