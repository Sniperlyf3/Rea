# QuicMedia

A QUIC-based real-time **voice engine**. One engine owns many QUIC connections,
each carrying several logical audio streams; it can **capture** (mic → Opus),
**play back** (decode → speaker), and **forward** encoded packets between
connections natively (**SFU**). The full media path runs in a native Rust core —
**C# is control-plane only and never touches a media byte**.

Tuned for low-data voice: Opus, 120 ms ptime, ~8 kbps by default.

## Install

```
dotnet add package QuicMedia
```

The package ships a native binary per runtime (`runtimes/<rid>/native`), loaded
automatically via `[LibraryImport]`.

## Quick start

```csharp
using QuicMedia;

QuicMediaEngine.Initialize();
using var engine = new QuicMediaEngine(new QuicAudioParams(), QuicBackend.Auto);

// Server: play back whatever streams clients open.
var (cert, key) = QuicMediaCertificate.GenerateSelfSigned("localhost");
engine.StreamOpened += (conn, stream, p) => engine.BindPlayback(conn, stream);
engine.Listen("0.0.0.0:5000", new QuicSecurity
{
    Certificate = cert,
    PrivateKey = key,
});

// Client: pin the server and send the mic.
ulong c = engine.Connect("0.0.0.0:0", "127.0.0.1:5000", "localhost", new QuicSecurity
{
    Verification = QuicVerification.PinnedSha256,
    Pin = QuicMediaCertificate.Sha256(cert),
});
ushort s = engine.AddCaptureStream(c, QuicCodec.Opus, 8_000, active: true);

// Either side can pause the stream; the sender then stops transmitting entirely.
engine.SetStreamActive(c, s, false);
```

### SFU (native fan-out)

```csharp
// Forward an inbound stream's encoded packets to a downstream connection —
// no decode, entirely in native code. Control (who is active) stays in C#.
ushort fwd = engine.OpenStream(downstreamConn, QuicCodec.Opus, 8_000, active: true);
engine.AddRoute(upstreamConn, upstreamStream, downstreamConn, fwd);
```

See the repository for the full architecture, the Rust source, and the C++ binding.
