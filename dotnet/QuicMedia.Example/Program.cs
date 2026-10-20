using System.Text;
using QuicMedia;

// A minimal two-peer demo over QUIC.
//   Terminal A:  dotnet run -- server 0.0.0.0:5000
//   Terminal B:  dotnet run -- client 127.0.0.1:5000 <pin-base64-from-server>
//
// Audio flows entirely in the native layer (capture → Opus → QUIC → render);
// this process only drives lifecycle and the text data channel.

if (args.Length < 2)
{
    PrintUsage();
    return 1;
}

QuicMediaSession.Initialize();

QuicMediaOptions options;
switch (args[0])
{
    case "server":
    {
        var (cert, key) = QuicMediaCertificate.GenerateSelfSigned("localhost");
        var pin = QuicMediaCertificate.Sha256(cert);
        Console.WriteLine($"Server certificate pin (base64 SHA-256):\n  {Convert.ToBase64String(pin)}");
        Console.WriteLine("Pass that pin to the client as the third argument.\n");
        options = new QuicMediaOptions
        {
            Role = QuicRole.Server,
            BindAddress = args[1],
            ServerName = "localhost",
            Codec = QuicCodec.Opus,
            AudioBackend = QuicAudioBackend.Auto,
            Verification = QuicVerification.Insecure,
            CertificateDer = cert,
            PrivateKeyDer = key,
        };
        break;
    }
    case "client":
    {
        if (args.Length < 3)
        {
            PrintUsage();
            return 1;
        }
        options = new QuicMediaOptions
        {
            Role = QuicRole.Client,
            BindAddress = "0.0.0.0:0",
            PeerAddress = args[1],
            ServerName = "localhost",
            Codec = QuicCodec.Opus,
            AudioBackend = QuicAudioBackend.Auto,
            Verification = QuicVerification.PinnedSha256,
            Pin = Convert.FromBase64String(args[2]),
        };
        break;
    }
    default:
        PrintUsage();
        return 1;
}

using var session = QuicMediaSession.Start(options);
session.StateChanged += s => Console.WriteLine($"[state] {s}");
session.DataReceived += d => Console.WriteLine($"[data]  {Encoding.UTF8.GetString(d)}  ({d.Length} bytes)");
session.ErrorOccurred += (code, msg) => Console.WriteLine($"[error {code}] {msg}");

Console.WriteLine("Running. Type text to send on the data channel; 'stats' to print stats; 'quit' to exit.");
for (string? line = Console.ReadLine(); line is not null; line = Console.ReadLine())
{
    if (line == "quit") break;
    if (line == "stats")
    {
        Console.WriteLine(session.GetStats());
        continue;
    }
    session.SendData(Encoding.UTF8.GetBytes(line));
}

return 0;

static void PrintUsage()
{
    Console.Error.WriteLine("Usage:");
    Console.Error.WriteLine("  server <bind-addr>              e.g.  server 0.0.0.0:5000");
    Console.Error.WriteLine("  client <peer-addr> <pin-b64>    e.g.  client 127.0.0.1:5000 AbCd...==");
}
