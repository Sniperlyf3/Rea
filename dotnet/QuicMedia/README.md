# QuicMedia

QUIC-based real-time **audio + reliable data** transport. The full media path
(capture → Opus → QUIC datagrams → jitter buffer → render) runs in a native
Rust core; **C# is control-plane only and never touches a media byte**.

## Install

```
dotnet add package QuicMedia
```

The package ships a native binary per runtime (`runtimes/<rid>/native`), loaded
automatically via `[LibraryImport]`.

## Quick start

```csharp
using QuicMedia;

QuicMediaSession.Initialize();

// Server: generate a cert and publish its pin out of band.
var (cert, key) = QuicMediaCertificate.GenerateSelfSigned("localhost");
byte[] pin = QuicMediaCertificate.Sha256(cert);

using var server = QuicMediaSession.Start(new QuicMediaOptions
{
    Role = QuicRole.Server,
    BindAddress = "0.0.0.0:5000",
    CertificateDer = cert,
    PrivateKeyDer = key,
    Codec = QuicCodec.Opus,
});

// Client: verify the server by pinned SHA-256.
using var client = QuicMediaSession.Start(new QuicMediaOptions
{
    Role = QuicRole.Client,
    PeerAddress = "127.0.0.1:5000",
    Verification = QuicVerification.PinnedSha256,
    Pin = pin,
    Codec = QuicCodec.Opus,
});

client.StateChanged += s => Console.WriteLine($"state: {s}");
client.DataReceived += d => Console.WriteLine($"data: {d.Length} bytes");
client.SendData("hello"u8);
```

Audio capture/playout happen entirely in native code. You configure, start/stop,
read `GetStats()`, and exchange small messages on the reliable data channel.

See the repository for the full architecture and the Rust source.
