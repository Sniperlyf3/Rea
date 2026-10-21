using System.Runtime.InteropServices;

namespace QuicMedia.Interop;

/// <summary>Blittable mirror of the C <c>QmAudioParams</c> struct.</summary>
[StructLayout(LayoutKind.Sequential)]
internal struct QmAudioParams
{
    public uint SampleRate;
    public ushort Channels;
    public uint FrameMs;
    public uint Bitrate;
    public int Codec;
}

/// <summary>Blittable mirror of the C <c>QmSecurity</c> struct.</summary>
[StructLayout(LayoutKind.Sequential)]
internal struct QmSecurity
{
    public int Verification;
    public IntPtr CertDer;
    public nuint CertDerLen;
    public IntPtr KeyDer;
    public nuint KeyDerLen;
    public IntPtr Pin;
    public nuint PinLen;
}

/// <summary>Blittable mirror of the C <c>QmConnStats</c> struct.</summary>
[StructLayout(LayoutKind.Sequential)]
internal struct QmConnStats
{
    public ulong DatagramsSent;
    public ulong DatagramsRecv;
    public ulong BytesSent;
    public ulong BytesRecv;
    public ulong SendErrors;
}

/// <summary>Blittable mirror of the C <c>QmCallbacks</c> struct (unmanaged function pointers).</summary>
[StructLayout(LayoutKind.Sequential)]
internal unsafe struct QmCallbacks
{
    public void* User;
    public delegate* unmanaged[Cdecl]<void*, ulong, int, void> OnConnection;
    public delegate* unmanaged[Cdecl]<void*, ulong, void> OnConnectionClosed;
    public delegate* unmanaged[Cdecl]<void*, ulong, ushort, int, uint, ushort, uint, uint, void> OnStreamOpened;
    public delegate* unmanaged[Cdecl]<void*, ulong, ushort, int, void> OnStreamActive;
    public delegate* unmanaged[Cdecl]<void*, ulong, ushort, void> OnStreamClosed;
    public delegate* unmanaged[Cdecl]<void*, ulong, byte*, nuint, void> OnData;
    public delegate* unmanaged[Cdecl]<void*, int, byte*, void> OnError;
}
