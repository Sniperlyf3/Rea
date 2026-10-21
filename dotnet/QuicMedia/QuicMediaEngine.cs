using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using QuicMedia.Interop;

namespace QuicMedia;

/// <summary>
/// A QUIC media engine: owns many connections, each with several audio streams,
/// and can capture, play back, and forward encoded packets between connections
/// natively (SFU). This is a control-plane wrapper — no media crosses into C#.
/// </summary>
/// <remarks>Events are raised from native runtime threads; keep handlers short.</remarks>
public sealed unsafe class QuicMediaEngine : IDisposable
{
    public event Action<ulong, bool>? ConnectionOpened;
    public event Action<ulong>? ConnectionClosed;
    public event Action<ulong, ushort, QuicStreamParams>? StreamOpened;
    public event Action<ulong, ushort, bool>? StreamActiveChanged;
    public event Action<ulong, ushort>? StreamClosed;
    public event Action<ulong, byte[]>? DataReceived;
    public event Action<int, string>? ErrorOccurred;

    private IntPtr _engine;
    private GCHandle _self;
    private readonly object _lock = new();
    private bool _disposed;

    /// <summary>Initialize process-wide crypto defaults. Optional; idempotent.</summary>
    public static void Initialize() => NativeMethods.qm_init();

    public QuicMediaEngine(QuicAudioParams? audio = null, QuicBackend backend = QuicBackend.Auto)
    {
        audio ??= new QuicAudioParams();
        _self = GCHandle.Alloc(this);

        var p = new QmAudioParams
        {
            SampleRate = audio.SampleRate,
            Channels = audio.Channels,
            FrameMs = audio.FrameMs,
            Bitrate = audio.Bitrate,
            Codec = (int)audio.Codec,
        };
        var cbs = new QmCallbacks
        {
            User = (void*)GCHandle.ToIntPtr(_self),
            OnConnection = &OnConnectionNative,
            OnConnectionClosed = &OnConnectionClosedNative,
            OnStreamOpened = &OnStreamOpenedNative,
            OnStreamActive = &OnStreamActiveNative,
            OnStreamClosed = &OnStreamClosedNative,
            OnData = &OnDataNative,
            OnError = &OnErrorNative,
        };
        int rc = NativeMethods.qm_engine_create(&p, (int)backend, &cbs, out _engine);
        if (rc != 0)
        {
            _self.Free();
            throw new QuicMediaException(rc, NativeError.Message(rc));
        }
    }

    /// <summary>Dial a peer; returns the new connection id.</summary>
    public ulong Connect(string bind, string peer, string serverName, QuicSecurity sec, bool enableDataChannel = false)
    {
        ThrowIfDisposed();
        IntPtr b = Marshal.StringToCoTaskMemUTF8(bind);
        IntPtr pr = Marshal.StringToCoTaskMemUTF8(peer);
        IntPtr sn = Marshal.StringToCoTaskMemUTF8(serverName);
        GCHandle cert = Pin(sec.Certificate), key = Pin(sec.PrivateKey), pin = Pin(sec.Pin);
        try
        {
            var s = ToNative(sec, cert, key, pin);
            int rc = NativeMethods.qm_engine_connect(_engine, (byte*)b, (byte*)pr, (byte*)sn, &s,
                enableDataChannel ? 1 : 0, out ulong conn);
            Check(rc);
            return conn;
        }
        finally
        {
            FreeUtf8(b, pr, sn);
            Unpin(cert, key, pin);
        }
    }

    /// <summary>Accept incoming connections on <paramref name="bind"/>.</summary>
    public void Listen(string bind, QuicSecurity sec, bool enableDataChannel = false)
    {
        ThrowIfDisposed();
        IntPtr b = Marshal.StringToCoTaskMemUTF8(bind);
        GCHandle cert = Pin(sec.Certificate), key = Pin(sec.PrivateKey), pin = Pin(sec.Pin);
        try
        {
            var s = ToNative(sec, cert, key, pin);
            Check(NativeMethods.qm_engine_listen(_engine, (byte*)b, &s, enableDataChannel ? 1 : 0));
        }
        finally
        {
            if (b != IntPtr.Zero) Marshal.FreeCoTaskMem(b);
            Unpin(cert, key, pin);
        }
    }

    /// <summary>Open an outbound stream; returns the allocated stream id.</summary>
    public ushort OpenStream(ulong conn, QuicCodec codec, uint bitrate, bool active)
    {
        ThrowIfDisposed();
        Check(NativeMethods.qm_engine_open_stream(_engine, conn, (int)codec, bitrate, active ? 1 : 0, out ushort s));
        return s;
    }

    /// <summary>Open an outbound stream fed by the local microphone (capture → encode).</summary>
    public ushort AddCaptureStream(ulong conn, QuicCodec codec, uint bitrate, bool active)
    {
        ThrowIfDisposed();
        Check(NativeMethods.qm_engine_add_capture_stream(_engine, conn, (int)codec, bitrate, active ? 1 : 0, out ushort s));
        return s;
    }

    /// <summary>Decode and play an inbound stream on the speaker.</summary>
    public void BindPlayback(ulong conn, ushort stream)
    {
        ThrowIfDisposed();
        Check(NativeMethods.qm_engine_bind_playback(_engine, conn, stream));
    }

    /// <summary>Set our desired active state for a stream (signals the peer).</summary>
    public void SetStreamActive(ulong conn, ushort stream, bool active)
    {
        ThrowIfDisposed();
        Check(NativeMethods.qm_engine_set_stream_active(_engine, conn, stream, active ? 1 : 0));
    }

    public void CloseStream(ulong conn, ushort stream)
    {
        ThrowIfDisposed();
        Check(NativeMethods.qm_engine_close_stream(_engine, conn, stream));
    }

    /// <summary>Route an inbound stream's encoded packets to an outbound stream (native fan-out).</summary>
    public void AddRoute(ulong srcConn, ushort srcStream, ulong dstConn, ushort dstStream)
    {
        ThrowIfDisposed();
        Check(NativeMethods.qm_engine_add_route(_engine, srcConn, srcStream, dstConn, dstStream));
    }

    public void RemoveRoute(ulong srcConn, ushort srcStream, ulong dstConn, ushort dstStream)
    {
        ThrowIfDisposed();
        Check(NativeMethods.qm_engine_remove_route(_engine, srcConn, srcStream, dstConn, dstStream));
    }

    /// <summary>Send a reliable data-channel message on a connection.</summary>
    public void SendData(ulong conn, ReadOnlySpan<byte> data)
    {
        ThrowIfDisposed();
        fixed (byte* p = data)
        {
            Check(NativeMethods.qm_engine_send_data(_engine, conn, p, (nuint)data.Length));
        }
    }

    public void CloseConnection(ulong conn)
    {
        ThrowIfDisposed();
        Check(NativeMethods.qm_engine_close_connection(_engine, conn));
    }

    public QuicConnectionStats GetConnectionStats(ulong conn)
    {
        ThrowIfDisposed();
        Check(NativeMethods.qm_engine_connection_stats(_engine, conn, out QmConnStats s));
        return new QuicConnectionStats(s.DatagramsSent, s.DatagramsRecv, s.BytesSent, s.BytesRecv, s.SendErrors);
    }

    // ---- marshalling helpers ----

    private static GCHandle Pin(byte[]? data) =>
        data is { Length: > 0 } ? GCHandle.Alloc(data, GCHandleType.Pinned) : default;

    private static IntPtr AddrOf(GCHandle h) => h.IsAllocated ? h.AddrOfPinnedObject() : IntPtr.Zero;

    private static QmSecurity ToNative(QuicSecurity sec, GCHandle cert, GCHandle key, GCHandle pin) => new()
    {
        Verification = (int)sec.Verification,
        CertDer = AddrOf(cert),
        CertDerLen = (nuint)(sec.Certificate?.Length ?? 0),
        KeyDer = AddrOf(key),
        KeyDerLen = (nuint)(sec.PrivateKey?.Length ?? 0),
        Pin = AddrOf(pin),
        PinLen = (nuint)(sec.Pin?.Length ?? 0),
    };

    private static void FreeUtf8(params IntPtr[] ptrs)
    {
        foreach (var p in ptrs)
            if (p != IntPtr.Zero) Marshal.FreeCoTaskMem(p);
    }

    private static void Unpin(params GCHandle[] handles)
    {
        foreach (var h in handles)
            if (h.IsAllocated) h.Free();
    }

    private static void Check(int rc)
    {
        if (rc != 0) throw new QuicMediaException(rc, NativeError.Message(rc));
    }

    // ---- native callbacks (invoked on engine threads) ----

    private static QuicMediaEngine? Recover(void* user)
    {
        if (user == null) return null;
        var h = GCHandle.FromIntPtr((IntPtr)user);
        return h.IsAllocated ? h.Target as QuicMediaEngine : null;
    }

    [UnmanagedCallersOnly(CallConvs = new[] { typeof(CallConvCdecl) })]
    private static void OnConnectionNative(void* u, ulong conn, int incoming) =>
        Recover(u)?.ConnectionOpened?.Invoke(conn, incoming != 0);

    [UnmanagedCallersOnly(CallConvs = new[] { typeof(CallConvCdecl) })]
    private static void OnConnectionClosedNative(void* u, ulong conn) =>
        Recover(u)?.ConnectionClosed?.Invoke(conn);

    [UnmanagedCallersOnly(CallConvs = new[] { typeof(CallConvCdecl) })]
    private static void OnStreamOpenedNative(void* u, ulong conn, ushort stream, int codec,
        uint sampleRate, ushort channels, uint frameMs, uint bitrate) =>
        Recover(u)?.StreamOpened?.Invoke(conn, stream,
            new QuicStreamParams((QuicCodec)codec, sampleRate, channels, frameMs, bitrate));

    [UnmanagedCallersOnly(CallConvs = new[] { typeof(CallConvCdecl) })]
    private static void OnStreamActiveNative(void* u, ulong conn, ushort stream, int active) =>
        Recover(u)?.StreamActiveChanged?.Invoke(conn, stream, active != 0);

    [UnmanagedCallersOnly(CallConvs = new[] { typeof(CallConvCdecl) })]
    private static void OnStreamClosedNative(void* u, ulong conn, ushort stream) =>
        Recover(u)?.StreamClosed?.Invoke(conn, stream);

    [UnmanagedCallersOnly(CallConvs = new[] { typeof(CallConvCdecl) })]
    private static void OnDataNative(void* u, ulong conn, byte* data, nuint len)
    {
        var e = Recover(u);
        if (e is null) return;
        var buf = new byte[(int)len];
        if (len > 0) Marshal.Copy((IntPtr)data, buf, 0, (int)len);
        e.DataReceived?.Invoke(conn, buf);
    }

    [UnmanagedCallersOnly(CallConvs = new[] { typeof(CallConvCdecl) })]
    private static void OnErrorNative(void* u, int code, byte* message) =>
        Recover(u)?.ErrorOccurred?.Invoke(code, Marshal.PtrToStringUTF8((IntPtr)message) ?? string.Empty);

    private void ThrowIfDisposed()
    {
        if (_disposed) throw new ObjectDisposedException(nameof(QuicMediaEngine));
    }

    public void Dispose()
    {
        lock (_lock)
        {
            if (_disposed) return;
            _disposed = true;
        }
        if (_engine != IntPtr.Zero)
        {
            NativeMethods.qm_engine_destroy(_engine);
            _engine = IntPtr.Zero;
        }
        if (_self.IsAllocated) _self.Free();
    }
}
