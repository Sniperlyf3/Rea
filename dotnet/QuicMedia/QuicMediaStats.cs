namespace QuicMedia;

/// <summary>A point-in-time snapshot of session statistics.</summary>
public readonly record struct QuicMediaStats(
    QuicConnectionState State,
    ulong DatagramsSent,
    ulong DatagramsReceived,
    ulong BytesSent,
    ulong BytesReceived,
    ulong SendErrors,
    ulong RttMicroseconds,
    uint JitterDepth,
    ulong JitterPlayed,
    ulong JitterConcealed,
    ulong JitterLateDrops);
