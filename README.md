# Rea — QuicMedia

A **QUIC-based real-time audio + reliable data transport**. The full media path
is implemented in native **Rust** and exposed to **C#/.NET** through a NuGet
package. All media and latency-sensitive work (capture, Opus encode/decode,
transport, jitter buffering, render) stays in the native layer — **no media ever
flows through the managed C# layer**, which is control-plane only.

## Layout

```
rust/                      Cargo workspace (native core)
  crates/quicmedia-core/   transport + media logic (lib)
  crates/quicmedia-ffi/    flat C ABI -> libquicmedia (cdylib + staticlib)
  include/quicmedia.h      authoritative C header
dotnet/                    C# binding + NuGet
  QuicMedia/               the package (LibraryImport P/Invoke + safe wrapper)
  QuicMedia.Example/       console demo (server/client over QUIC)
build/                     native build + NuGet packaging scripts
docs/ARCHITECTURE.md       design, threading, security, packaging, roadmap
```

## What it does

- **Transport:** QUIC (quinn + rustls, TLS 1.3). Media rides **unreliable
  DATAGRAMs** (lowest latency, loss concealed not blocked); a reliable
  **data channel** rides a bidirectional stream on the same connection.
- **Audio:** native capture → Opus encode → wire → jitter/playout buffer →
  decode → render. Opus (bundled libopus) with a PCM passthrough fallback.
- **Security:** self-signed certs + pluggable pinning (insecure / exact cert /
  SHA-256). TLS 1.3 enforced; moving to full PKI/mTLS is a localized change.
- **Interop:** C ABI (cbindgen) consumed via `[LibraryImport]`; one NuGet with a
  native binary per RID (win-x64, linux-x64, osx, Android; iOS static-linked).

## Build & test (native)

```bash
cd rust
cargo test                              # headless: PCM passthrough + null audio + QUIC loopback
cargo build -p quicmedia-ffi --release --features opus-codec,cpal-backend
```

> The `cpal-backend` feature needs platform audio dev headers at build time
> (e.g. `libasound2-dev` on Linux). `opus-codec` needs a C toolchain + cmake.
> The default build is headless-safe (neither feature), which is how CI runs the
> full QUIC pipeline test.

## Build the NuGet (host RID)

```bash
build/build-native.sh                   # stages runtimes/<rid>/native/
build/pack-nuget.sh                     # -> artifacts/QuicMedia.0.1.0.nupkg  (needs dotnet SDK)
```

## Run the demo

```bash
# Terminal A
dotnet run --project dotnet/QuicMedia.Example -- server 0.0.0.0:5000
# copy the printed pin, then Terminal B
dotnet run --project dotnet/QuicMedia.Example -- client 127.0.0.1:5000 <pin-base64>
```

See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for the full design.

## Status

Core transport, audio pipeline, data channel, security and the C#/NuGet layer
are implemented. Verified by unit tests plus a two-peer QUIC loopback
integration test (pinned TLS 1.3 handshake, bidirectional media datagrams,
bidirectional data channel). Video, mobile cross-build packaging, and advanced
congestion/FEC are on the roadmap.

## License

Dual-licensed under MIT OR Apache-2.0.
