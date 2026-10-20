using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using QuicMedia.Interop;

namespace QuicMedia;

/// <summary>
/// A running QUIC media session. This is a thin control-plane wrapper over the
/// native transport: lifecycle, configuration, stats, and small data-channel
/// messages. Audio/media never cross into managed memory.
/// </summary>
/// <remarks>
/// Events are raised from native runtime threads. Marshal to your UI/sync
/// context if needed, and keep handlers short and non-blocking.
/// </remarks>
public sealed unsafe class QuicMediaSession : IDisposable
{
    /// <summary>Raised when the connection state changes.</summary>
    public event Action<QuicConnectionState>? StateChanged;

    /// <summary>Raised when a data-channel message arrives.</summary>
    public event Action<byte[]>? DataReceived;

    /// <summary>Raised when the native layer reports an error.</summary>
    public event Action<int, string>? ErrorOccurred;

    private IntPtr _handle;
    private GCHandle _self;
    private readonly object _lock = new();
    private bool _disposed;

    private QuicMediaSession() { }

    /// <summary>Initialize process-wide crypto defaults. Optional; idempotent.</summary>
    public static void Initialize() => NativeMethods.qm_init();

    /// <summary>Start a session with the given options.</summary>
    public static QuicMediaSession Start(QuicMediaOptions options)
    {
        ArgumentNullException.ThrowIfNull(options);

        var session = new QuicMediaSession();
        session._self = GCHandle.Alloc(session);

        IntPtr bind = Marshal.StringToCoTaskMemUTF8(options.BindAddress);
        IntPtr peer = options.PeerAddress is null ? IntPtr.Zero : Marshal.StringToCoTaskMemUTF8(options.PeerAddress);
        IntPtr name = Marshal.StringToCoTaskMemUTF8(options.ServerName);
        GCHandle certH = Pin(options.CertificateDer);
        GCHandle keyH = Pin(options.PrivateKeyDer);
        GCHandle pinH = Pin(options.Pin);
        try
        {
            var cfg = new QmConfigNative
            {
                Role = (int)options.Role,
                BindAddr = bind,
                PeerAddr = peer,
                ServerName = name,
                SampleRate = options.SampleRate,
                Channels = options.Channels,
                FrameMs = options.FrameMs,
                Bitrate = options.Bitrate,
                Codec = (int)options.Codec,
                AudioBackend = (int)options.AudioBackend,
                EnableAudio = options.EnableAudio ? 1 : 0,
                EnableDataChannel = options.EnableDataChannel ? 1 : 0,
                Verification = (int)options.Verification,
                CertDer = AddrOf(certH),
                CertDerLen = (nuint)(options.CertificateDer?.Length ?? 0),
                KeyDer = AddrOf(keyH),
                KeyDerLen = (nuint)(options.PrivateKeyDer?.Length ?? 0),
                Pin = AddrOf(pinH),
                PinLen = (nuint)(options.Pin?.Length ?? 0),
            };

            var cbs = new QmCallbacksNative
            {
                User = (void*)GCHandle.ToIntPtr(session._self),
                OnState = &OnStateNative,
                OnData = &OnDataNative,
                OnError = &OnErrorNative,
            };

            int rc = NativeMethods.qm_session_start(&cfg, &cbs, out session._handle);
            if (rc != 0)
            {
                session._self.Free();
                throw new QuicMediaException(rc, NativeError.Message(rc));
            }
            return session;
        }
        finally
        {
            if (bind != IntPtr.Zero) Marshal.FreeCoTaskMem(bind);
            if (peer != IntPtr.Zero) Marshal.FreeCoTaskMem(peer);
            if (name != IntPtr.Zero) Marshal.FreeCoTaskMem(name);
            if (certH.IsAllocated) certH.Free();
            if (keyH.IsAllocated) keyH.Free();
            if (pinH.IsAllocated) pinH.Free();
        }
    }

    private static GCHandle Pin(byte[]? data) =>
        data is { Length: > 0 } ? GCHandle.Alloc(data, GCHandleType.Pinned) : default;

    private static IntPtr AddrOf(GCHandle h) =>
        h.IsAllocated ? h.AddrOfPinnedObject() : IntPtr.Zero;

    /// <summary>Queue a message on the reliable data channel. Non-blocking.</summary>
    public void SendData(ReadOnlySpan<byte> data)
    {
        ThrowIfDisposed();
        fixed (byte* p = data)
        {
            int rc = NativeMethods.qm_session_send_data(_handle, p, (nuint)data.Length);
            if (rc != 0) throw new QuicMediaException(rc, NativeError.Message(rc));
        }
    }

    /// <summary>Current statistics snapshot.</summary>
    public QuicMediaStats GetStats()
    {
        ThrowIfDisposed();
        int rc = NativeMethods.qm_session_get_stats(_handle, out var s);
        if (rc != 0) throw new QuicMediaException(rc, NativeError.Message(rc));
        return new QuicMediaStats(
            (QuicConnectionState)s.State,
            s.DatagramsSent, s.DatagramsRecv,
            s.BytesSent, s.BytesRecv,
            s.SendErrors, s.RttUs,
            s.JitterDepth, s.JitterPlayed, s.JitterConcealed, s.JitterLateDrops);
    }

    /// <summary>Current connection state.</summary>
    public QuicConnectionState State
    {
        get
        {
            ThrowIfDisposed();
            return (QuicConnectionState)NativeMethods.qm_session_state(_handle);
        }
    }

    // ---- native callbacks (invoked on runtime threads) ----

    private static QuicMediaSession? Recover(void* user)
    {
        if (user == null) return null;
        var h = GCHandle.FromIntPtr((IntPtr)user);
        return h.IsAllocated ? h.Target as QuicMediaSession : null;
    }

    [UnmanagedCallersOnly(CallConvs = new[] { typeof(CallConvCdecl) })]
    private static void OnStateNative(void* user, int state) =>
        Recover(user)?.StateChanged?.Invoke((QuicConnectionState)state);

    [UnmanagedCallersOnly(CallConvs = new[] { typeof(CallConvCdecl) })]
    private static void OnDataNative(void* user, byte* data, nuint len)
    {
        var s = Recover(user);
        if (s is null) return;
        var buf = new byte[(int)len];
        if (len > 0) Marshal.Copy((IntPtr)data, buf, 0, (int)len);
        s.DataReceived?.Invoke(buf);
    }

    [UnmanagedCallersOnly(CallConvs = new[] { typeof(CallConvCdecl) })]
    private static void OnErrorNative(void* user, int code, byte* message)
    {
        var s = Recover(user);
        s?.ErrorOccurred?.Invoke(code, Marshal.PtrToStringUTF8((IntPtr)message) ?? string.Empty);
    }

    private void ThrowIfDisposed()
    {
        if (_disposed) throw new ObjectDisposedException(nameof(QuicMediaSession));
    }

    public void Dispose()
    {
        lock (_lock)
        {
            if (_disposed) return;
            _disposed = true;
        }
        if (_handle != IntPtr.Zero)
        {
            NativeMethods.qm_session_stop(_handle);
            _handle = IntPtr.Zero;
        }
        if (_self.IsAllocated) _self.Free();
    }
}
