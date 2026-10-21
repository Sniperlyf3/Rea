# Rea — QuicMedia

A **QUIC-based real-time voice engine**, implemented in native **Rust** and
exposed to **C#/.NET** (NuGet) and **C++** (CMake). All media and latency-sensitive
work (capture, Opus encode/decode, transport, jitter buffering, render, and SFU
forwarding) stays in the native layer — **no media ever flows through the managed
C# layer**, which is control-plane only.

One engine owns many QUIC connections, each carrying several logical audio
streams, and can play any of three roles (or mix them):

- **Capturing client** — mic → Opus → QUIC.
- **Receiving client** — QUIC → jitter buffer → decode → speaker.
- **SFU** — forward a stream's **encoded** packets to many downstream connections
  natively (1→many fan-out), no decode.

Each stream has a **per-connection active flag** controllable from either side:
when a receiver (or the SFU's downstream subscriber) sets it false, the sender
stops transmitting **and** stops encoding that stream. Tuned for low-data voice:
Opus, **120 ms** ptime, **~8 kbps** by default.

## Layout

```
rust/                      Cargo workspace (native core)
  crates/quicmedia-core/   engine, connections, routing, codecs, audio (lib)
  crates/quicmedia-ffi/    flat C ABI -> libquicmedia (cdylib + staticlib)
  include/quicmedia.h      authoritative C header
dotnet/                    C# binding + NuGet (QuicMediaEngine)
cpp/                       header-only C++ wrapper + CMake package
build/                     native build + packaging scripts
docs/ARCHITECTURE.md       design, threading, security, packaging, roadmap
```

## Core model

- **Transport:** QUIC (quinn + rustls, TLS 1.3). Media rides **unreliable
  DATAGRAMs** with a 10-byte header `(version, marker, payload_type, stream_id,
  sequence, timestamp)`; a reliable **control** channel carries stream open /
  set-active / close; an optional reliable **data channel** is also available.
- **Audio:** native capture → Opus encode → wire → per-stream jitter buffer →
  decode → render, via the vendored **miniaudio** single-header device.
- **SFU:** `add_route(src → dst)` fans encoded packets out to downstream
  connections in native code; the per-connection active flag gates each
  subscriber independently.
- **Security:** self-signed certs + pluggable pinning (insecure / exact cert /
  SHA-256), TLS 1.3 enforced.

## Build & test (native)

```bash
cd rust
cargo test                              # headless: passthrough + null audio + QUIC/SFU loopback
cargo build -p quicmedia-ffi --release --features opus-codec,miniaudio-backend
```

> `miniaudio-backend` needs no audio `-dev` packages (miniaudio loads the OS
> audio API at runtime). `opus-codec` needs a C toolchain + cmake.

## Bindings

```bash
# C# NuGet (stages the native lib, then dotnet pack)
build/build-native.sh && build/pack-nuget.sh        # -> artifacts/QuicMedia.0.1.0.nupkg

# C++ (CMake): builds libquicmedia + the example; installs a find_package() package
build/build-cpp.sh
cmake --install cpp/build --prefix /your/prefix      # -> find_package(quicmedia)

# Android native libs (arm64-v8a, x86_64)
build/build-android.sh --abis "arm64-v8a x86_64" --features miniaudio-backend
```

## Run the demo

```bash
# C#
dotnet run --project dotnet/QuicMedia.Example -- server 0.0.0.0:5000
dotnet run --project dotnet/QuicMedia.Example -- client 127.0.0.1:5000 <pin-base64>

# C++ (easy VoiceAgent — two lines; or `server`/`client` for the low-level API)
cpp/build/quicmedia_demo host 0.0.0.0:5000
cpp/build/quicmedia_demo join 127.0.0.1:5000 <pin-base64>
```

See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for the full design.

## Status

Engine, multi-stream transport, per-stream active control, native SFU
forwarding, Opus/miniaudio, and the C#/NuGet + C++/CMake bindings are
implemented and verified by unit tests plus an in-process **sender → SFU →
receiver** loopback (forwarding + receiver-driven deactivation). CI builds and
tests across Linux/Windows/macOS, Android cross-build, the NuGet, and the C++
package. iOS packaging and video are on the roadmap.

## License

Dual-licensed under MIT OR Apache-2.0.
