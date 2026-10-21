namespace QuicMedia;

/// <summary>Device/audio parameters shared by all streams on an engine.</summary>
public sealed class QuicAudioParams
{
    public uint SampleRate { get; set; } = 48_000;
    public ushort Channels { get; set; } = 1;
    /// <summary>Opus ptime in milliseconds (120 ms by default for low per-packet overhead).</summary>
    public uint FrameMs { get; set; } = 120;
    /// <summary>Opus bitrate in bits/sec (8 kbps by default for low-data voice).</summary>
    public uint Bitrate { get; set; } = 8_000;
    public QuicCodec Codec { get; set; } = QuicCodec.Opus;
}

/// <summary>TLS material and peer verification policy for a connection.</summary>
public sealed class QuicSecurity
{
    public QuicVerification Verification { get; set; } = QuicVerification.Insecure;
    /// <summary>Server certificate (DER). Required for <see cref="QuicMediaEngine.Listen"/>.</summary>
    public byte[]? Certificate { get; set; }
    /// <summary>Server private key, PKCS#8 (DER). Required for <see cref="QuicMediaEngine.Listen"/>.</summary>
    public byte[]? PrivateKey { get; set; }
    /// <summary>Pin material: the certificate DER for <see cref="QuicVerification.PinnedCert"/>,
    /// or a 32-byte SHA-256 for <see cref="QuicVerification.PinnedSha256"/>.</summary>
    public byte[]? Pin { get; set; }
}

/// <summary>Parameters of a stream announced by a peer.</summary>
public readonly record struct QuicStreamParams(
    QuicCodec Codec,
    uint SampleRate,
    ushort Channels,
    uint FrameMs,
    uint Bitrate);

/// <summary>Per-connection counters.</summary>
public readonly record struct QuicConnectionStats(
    ulong DatagramsSent,
    ulong DatagramsReceived,
    ulong BytesSent,
    ulong BytesReceived,
    ulong SendErrors);
