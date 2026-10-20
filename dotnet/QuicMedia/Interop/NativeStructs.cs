using System.Runtime.InteropServices;

namespace QuicMedia.Interop;

/// <summary>Blittable mirror of the C <c>QmConfig</c> struct.</summary>
[StructLayout(LayoutKind.Sequential)]
internal struct QmConfigNative
{
    public int Role;
    public IntPtr BindAddr;
    public IntPtr PeerAddr;
    public IntPtr ServerName;
    public uint SampleRate;
    public ushort Channels;
    public uint FrameMs;
    public uint Bitrate;
    public int Codec;
    public int AudioBackend;
    public int EnableAudio;
    public int EnableDataChannel;
    public int Verification;
    public IntPtr CertDer;
    public nuint CertDerLen;
    public IntPtr KeyDer;
    public nuint KeyDerLen;
    public IntPtr Pin;
    public nuint PinLen;
}

/// <summary>Blittable mirror of the C <c>QmCallbacks</c> struct.</summary>
[StructLayout(LayoutKind.Sequential)]
internal unsafe struct QmCallbacksNative
{
    public void* User;
    public delegate* unmanaged[Cdecl]<void*, int, void> OnState;
    public delegate* unmanaged[Cdecl]<void*, byte*, nuint, void> OnData;
    public delegate* unmanaged[Cdecl]<void*, int, byte*, void> OnError;
}

/// <summary>Blittable mirror of the C <c>QmStats</c> struct.</summary>
[StructLayout(LayoutKind.Sequential)]
internal struct QmStatsNative
{
    public byte State;
    public ulong DatagramsSent;
    public ulong DatagramsRecv;
    public ulong BytesSent;
    public ulong BytesRecv;
    public ulong SendErrors;
    public ulong RttUs;
    public uint JitterDepth;
    public ulong JitterPlayed;
    public ulong JitterConcealed;
    public ulong JitterLateDrops;
}
