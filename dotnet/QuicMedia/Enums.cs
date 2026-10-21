namespace QuicMedia;

/// <summary>Audio codec selection.</summary>
public enum QuicCodec
{
    /// <summary>Opus (requires the native library built with the <c>opus-codec</c> feature).</summary>
    Opus = 0,
    /// <summary>Uncompressed 16-bit PCM passthrough (tests/LAN).</summary>
    Passthrough = 1,
}

/// <summary>Native audio capture/render backend.</summary>
public enum QuicBackend
{
    /// <summary>Real device via miniaudio if compiled in, otherwise the null backend.</summary>
    Auto = 0,
    /// <summary>Real full-duplex hardware via miniaudio (requires the <c>miniaudio-backend</c> feature).</summary>
    Miniaudio = 1,
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
