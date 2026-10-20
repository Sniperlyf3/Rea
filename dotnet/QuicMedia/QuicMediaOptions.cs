namespace QuicMedia;

/// <summary>
/// Configuration for a <see cref="QuicMediaSession"/>. Both peers must agree on
/// the audio parameters out of band.
/// </summary>
public sealed class QuicMediaOptions
{
    /// <summary>Server listens; client dials.</summary>
    public QuicRole Role { get; set; } = QuicRole.Client;

    /// <summary>Local UDP socket to bind, e.g. <c>"0.0.0.0:0"</c> or <c>"0.0.0.0:5000"</c>.</summary>
    public string BindAddress { get; set; } = "0.0.0.0:0";

    /// <summary>Remote peer address; required for <see cref="QuicRole.Client"/>.</summary>
    public string? PeerAddress { get; set; }

    /// <summary>Expected server name (SNI / certificate name).</summary>
    public string ServerName { get; set; } = "localhost";

    public uint SampleRate { get; set; } = 48_000;
    public ushort Channels { get; set; } = 1;
    public uint FrameMs { get; set; } = 20;
    public uint Bitrate { get; set; } = 24_000;
    public QuicCodec Codec { get; set; } = QuicCodec.Opus;
    public QuicAudioBackend AudioBackend { get; set; } = QuicAudioBackend.Auto;

    /// <summary>Enable the native audio data path (capture/encode/send + recv/decode/render).</summary>
    public bool EnableAudio { get; set; } = true;

    /// <summary>Enable the reliable generic data channel.</summary>
    public bool EnableDataChannel { get; set; } = true;

    public QuicVerification Verification { get; set; } = QuicVerification.Insecure;

    /// <summary>Server certificate (DER). Required for <see cref="QuicRole.Server"/>.</summary>
    public byte[]? CertificateDer { get; set; }

    /// <summary>Server private key, PKCS#8 (DER). Required for <see cref="QuicRole.Server"/>.</summary>
    public byte[]? PrivateKeyDer { get; set; }

    /// <summary>
    /// Pin material for the client: the exact certificate DER for
    /// <see cref="QuicVerification.PinnedCert"/>, or a 32-byte SHA-256 for
    /// <see cref="QuicVerification.PinnedSha256"/>.
    /// </summary>
    public byte[]? Pin { get; set; }
}
