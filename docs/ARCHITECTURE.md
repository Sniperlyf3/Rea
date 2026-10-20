# QuicMedia architecture

A QUIC-based real-time **audio + reliable data** transport. The entire media
path lives in a native Rust core; the C# layer is **control-plane only** and
never touches a media byte.

```
┌───────────────────────────── .NET process ─────────────────────────────┐
│  C# (control plane)                                                      │
│   QuicMediaSession  ── start / stop / SendData / GetStats / events       │
│         │  P/Invoke (LibraryImport, C ABI)   ▲ callbacks (state/data/err)│
├─────────┼────────────────────────────────────┼──────────────────────────┤
│         ▼  quicmedia-ffi (C ABI, cdylib/staticlib)                       │
│   ┌──────────────────────── quicmedia-core (Rust) ────────────────────┐ │
│   │  mic ─▶ capture ─▶ Opus encode ─▶ framing ─▶ QUIC DATAGRAM ─────┐  │ │
│   │                                                                 │  │ │
│   │  speaker ◀─ render ◀─ decode ◀─ jitter buffer ◀─ QUIC DATAGRAM ◀┘  │ │
│   │                                                                    │ │
│   │  data channel  ◀────────────▶  QUIC bidirectional stream           │ │
│   └────────────────────────────────────────────────────────────────── ┘ │
└──────────────────────────────────────────────────────────────────────────┘
                              │ QUIC / TLS 1.3 / UDP
                              ▼
                           remote peer
```

## Design goals

- **Zero media on the managed heap.** Capture, encode, transport, jitter
  buffering, decode and render all happen in native code. C# only configures,
  starts/stops, reads stats, and exchanges small data-channel messages.
- **Lowest sensible latency.** Media rides **unreliable QUIC DATAGRAMs**, so a
  lost or late packet is concealed rather than head-of-line-blocking the stream.
- **One connection, two planes.** Real-time media (datagrams) and reliable
  control/data (a bidirectional stream) share a single QUIC connection, so they
  share congestion state, path and handshake but never block each other.
- **Portable packaging.** One NuGet package carries a native binary per runtime
  identifier (RID); `[LibraryImport]` loads the right one automatically.

## Crate layout (`rust/`)

| Crate | Output | Responsibility |
|-------|--------|----------------|
| `quicmedia-core` | rlib | All transport + media logic. |
| `quicmedia-ffi`  | cdylib + staticlib (`quicmedia`) | Flat C ABI over the core. |

### `quicmedia-core` modules

- **`config`** — `Config`, `AudioParams`, `SecurityConfig`, roles and enums.
- **`security`** — self-signed cert generation (rcgen), `rustls` server/client
  config building, and a pluggable `ServerCertVerifier` implementing the pinning
  policy (insecure / exact cert / SHA-256 cert pin).
- **`transport`** — quinn endpoint setup, connection establishment, datagram
  buffers, keep-alive and idle timeout.
- **`framing`** — a compact 8-byte RTP-like header (version, marker, payload
  type, 16-bit sequence, 32-bit timestamp) over each datagram. QUIC already
  provides identity, encryption, path validation and congestion control, so the
  per-packet header carries only what the media pipeline consumes. This mirrors
  the intent of IETF **RTP-over-QUIC (RoQ)** while staying minimal.
- **`jitter`** — a fixed-target playout buffer keyed by an extended (rollover-
  tracked) sequence number. It reorders, de-duplicates, bounds latency, and on
  each playout tick returns exactly one outcome: a frame, a concealment, or
  starvation (pre-buffering).
- **`codec`** — a `Codec` trait with an Opus implementation (`opus-codec`
  feature, bundled libopus) and an always-available 16-bit PCM passthrough used
  for tests/LAN and as a graceful fallback.
- **`audio`** — an `AudioBackend` trait. `NullBackend` (always compiled)
  captures silence and discards playout for headless/server/CI use;
  `CpalBackend` (`cpal-backend` feature) drives real input/output devices.
- **`datachannel`** — length-delimited reliable messaging over a QUIC
  bidirectional stream.
- **`session`** — the orchestrator and the public `Session` handle.

## Threading model

- The session owns a multi-threaded `tokio` runtime (control plane + async I/O).
- Tasks: **capture→encode→send**, **receive→jitter-push**, **playout→decode→
  render**, plus **data-channel reader/writer** and an **RTT sampler**.
- Audio hardware runs on the backend's own native OS audio threads. Capture and
  playout cross into the async world over bounded `tokio::mpsc` channels of
  interleaved PCM frames; the realtime audio callbacks never block (they
  `try_send` / `try_recv`).
- The jitter buffer is shared between the receive and playout tasks behind a
  short-critical-section `std::Mutex`.

## Transport details

- **QUIC via quinn + rustls (ring), TLS 1.3 only.** QUIC mandates TLS 1.3.
- **Media:** one unreliable `DATAGRAM` per audio frame = `[8-byte header][Opus
  payload]`. Oversized frames (payload + header exceeding the negotiated
  datagram size) are dropped and counted rather than fragmented.
- **Data channel:** a single bidirectional stream, `[u32 length][payload]`
  framing, 16 MiB message cap.
- **Keep-alive** every 5 s; **idle timeout** 30 s.

## Security

Prototype default is a fresh self-signed certificate plus a pluggable verifier:

- `Insecure` — accept any cert (dev only).
- `PinnedCert` — exact end-entity DER match.
- `PinnedSha256` — SHA-256 of the end-entity DER (compact, easy to carry out of
  band; what the example uses).

The verifier is an ordinary `rustls` component, so moving to full WebPKI/CA
validation or mutual TLS later is a localized change. The out-of-band exchange
of the pin (or trust anchor) is the application's responsibility.

## C ABI boundary (`quicmedia-ffi`)

- Opaque `QmSession*` handle; all functions return `int32_t` status codes
  (`qm_error_message` maps them) and never unwind (`catch_unwind` at the entry).
- Config and stats are passed as `#[repr(C)]` structs; strings are UTF-8
  `const char*`; cert/key/pin are `(ptr, len)` pairs copied on entry.
- Events are delivered via C function pointers (`on_state`, `on_data`,
  `on_error`) with an opaque `user` pointer. The managed side passes a `GCHandle`
  as `user` and recovers the session inside `[UnmanagedCallersOnly]` trampolines.
- The authoritative header is `rust/include/quicmedia.h` (regenerable with
  cbindgen).

## C# layer (`dotnet/QuicMedia`)

- `NativeMethods` — source-generated `[LibraryImport]` P/Invoke (cdecl).
- `QuicMediaSession` — `IDisposable` wrapper: `Start`, `SendData`, `GetStats`,
  `State`, and `StateChanged` / `DataReceived` / `ErrorOccurred` events.
- `QuicMediaCertificate` — self-signed generation and SHA-256 pinning helpers.
- Media never appears in this layer by construction: the ABI exposes no media
  buffers, only control, stats, and data-channel bytes.

## Packaging & build

- `build/build-native.(sh|ps1)` compiles `quicmedia-ffi` and stages the binary
  into `dotnet/QuicMedia/runtimes/<rid>/native/`.
- `build/pack-nuget.sh` builds the host RID and runs `dotnet pack`.
- A full multi-RID package is assembled in CI: each OS/arch agent stages its RID,
  the `runtimes/` tree is collected, and a final `dotnet pack` yields one package
  with every platform's binary.

### Target matrix

| RID | Rust target | Notes |
|-----|-------------|-------|
| `win-x64` | `x86_64-pc-windows-msvc` | `quicmedia.dll` |
| `linux-x64` | `x86_64-unknown-linux-gnu` | `libquicmedia.so`; cpal needs `libasound2-dev` |
| `osx-arm64` / `osx-x64` | `aarch64-apple-darwin` / `x86_64-apple-darwin` | `libquicmedia.dylib` |
| `android-arm64` etc. | `aarch64-linux-android` … | build with cargo-ndk + NDK; `.so` |
| `ios-arm64` | `aarch64-apple-ios` | **static** `libquicmedia.a`, linked at MAUI app build |

> iOS forbids loading external dynamic libraries, so the `staticlib` output is
> linked into the app; the other platforms ship the `cdylib`.

## Feature flags

- `opus-codec` — real Opus (bundled libopus; needs a C toolchain + cmake).
- `cpal-backend` — real audio devices (needs platform audio dev headers).

Without them the core still builds and runs headless with PCM passthrough and
the null backend — which is exactly how the test suite exercises the full QUIC
pipeline in CI.

## Verification

- Unit tests: framing round-trip, sequence-rollover extension, jitter buffer
  ordering/dedup/concealment, codec round-trip, security config building.
- Integration test (`tests/loopback.rs`): two real sessions over `127.0.0.1`
  complete a pinned TLS 1.3 handshake, exchange media datagrams in both
  directions (null backend + passthrough), and exchange data-channel messages
  both ways.

## Roadmap

1. **Audio hardening:** device resampling / format negotiation in `CpalBackend`,
   adaptive jitter target, Opus in-band FEC + DTX, RED/redundancy.
2. **Congestion & pacing:** media-aware pacing, optional BBR via quinn transport
   config, bandwidth estimation surfaced in stats.
3. **Video:** add a payload type, hardware encode/decode (Media Foundation /
   VideoToolbox / Android MediaCodec), keyframe request + NACK/PLI.
4. **RoQ interop:** optional exact RTP-over-QUIC wire format.
5. **Mobile:** finish cargo-ndk / iOS static-link packaging in CI and a MAUI
   sample.
