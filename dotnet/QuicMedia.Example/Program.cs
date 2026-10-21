using QuicMedia;

// A minimal two-peer voice demo on the engine API.
//   Terminal A:  dotnet run -- server 0.0.0.0:5000
//   Terminal B:  dotnet run -- client 127.0.0.1:5000 <pin-base64-from-server>
//
// Audio flows entirely in the native layer (mic → Opus → QUIC → jitter → speaker);
// this process only drives the control plane.

if (args.Length < 2)
{
    PrintUsage();
    return 1;
}

QuicMediaEngine.Initialize();
using var engine = new QuicMediaEngine(new QuicAudioParams(), QuicBackend.Auto);
engine.ConnectionOpened += (c, incoming) => Console.WriteLine($"[conn {c}] {(incoming ? "incoming" : "outgoing")}");
engine.ErrorOccurred += (code, msg) => Console.WriteLine($"[error {code}] {msg}");

switch (args[0])
{
    case "server":
    {
        var (cert, key) = QuicMediaCertificate.GenerateSelfSigned("localhost");
        var pin = QuicMediaCertificate.Sha256(cert);
        Console.WriteLine($"Server certificate pin (base64 SHA-256):\n  {Convert.ToBase64String(pin)}\n");

        // Play back whatever streams clients open.
        engine.StreamOpened += (conn, stream, p) =>
        {
            Console.WriteLine($"[conn {conn}] stream {stream} opened ({p.Codec}); binding playback");
            engine.BindPlayback(conn, stream);
        };

        engine.Listen(args[1], new QuicSecurity
        {
            Verification = QuicVerification.Insecure,
            Certificate = cert,
            PrivateKey = key,
        });
        Console.WriteLine($"Listening on {args[1]}. Press Enter to quit.");
        Console.ReadLine();
        break;
    }
    case "client":
    {
        if (args.Length < 3)
        {
            PrintUsage();
            return 1;
        }
        var conn = engine.Connect(
            bind: "0.0.0.0:0",
            peer: args[1],
            serverName: "localhost",
            sec: new QuicSecurity
            {
                Verification = QuicVerification.PinnedSha256,
                Pin = Convert.FromBase64String(args[2]),
            });
        var stream = engine.AddCaptureStream(conn, QuicCodec.Opus, 8_000, active: true);
        Console.WriteLine($"Connected (conn {conn}, stream {stream}); sending mic audio.");
        Console.WriteLine("Type 'mute'/'unmute' to toggle the stream, or 'quit' to exit.");
        for (string? line = Console.ReadLine(); line is not null; line = Console.ReadLine())
        {
            if (line == "quit") break;
            if (line == "mute") engine.SetStreamActive(conn, stream, false);
            else if (line == "unmute") engine.SetStreamActive(conn, stream, true);
        }
        break;
    }
    default:
        PrintUsage();
        return 1;
}

return 0;

static void PrintUsage()
{
    Console.Error.WriteLine("Usage:");
    Console.Error.WriteLine("  server <bind-addr>              e.g.  server 0.0.0.0:5000");
    Console.Error.WriteLine("  client <peer-addr> <pin-b64>    e.g.  client 127.0.0.1:5000 AbCd...==");
}
