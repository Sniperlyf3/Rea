namespace QuicMedia;

/// <summary>Which side of the QUIC connection a session plays.</summary>
public enum QuicRole
{
    Server = 0,
    Client = 1,
}

/// <summary>Audio codec selection.</summary>
public enum QuicCodec
{
    /// <summary>Opus (requires the native library built with the <c>opus-codec</c> feature).</summary>
    Opus = 0,
    /// <summary>Uncompressed 16-bit PCM passthrough (always available).</summary>
    Passthrough = 1,
}

/// <summary>Native audio capture/render backend.</summary>
public enum QuicAudioBackend
{
    /// <summary>Hardware devices if compiled in, otherwise the null backend.</summary>
    Auto = 0,
    /// <summary>Real hardware devices (requires the <c>cpal-backend</c> feature).</summary>
    Cpal = 1,
    /// <summary>Headless: silence in, discard out.</summary>
    Null = 2,
}

/// <summary>How this peer verifies the other end's certificate.</summary>
public enum QuicVerification
{
    /// <summary>Accept any certificate. INSECURE — development only.</summary>
    Insecure = 0,
    /// <summary>Pin the exact end-entity certificate DER.</summary>
    PinnedCert = 1,
    /// <summary>Pin the SHA-256 digest of the end-entity certificate DER.</summary>
    PinnedSha256 = 2,
}

/// <summary>Lifecycle state of the connection.</summary>
public enum QuicConnectionState
{
    Connecting = 0,
    Connected = 1,
    Disconnected = 2,
    Failed = 3,
}
