using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;

namespace QuicMedia.Interop;

/// <summary>
/// Source-generated P/Invoke bindings to <c>libquicmedia</c>. The .NET runtime
/// resolves <c>"quicmedia"</c> to <c>quicmedia.dll</c> / <c>libquicmedia.so</c> /
/// <c>libquicmedia.dylib</c> shipped under <c>runtimes/&lt;rid&gt;/native</c>.
/// </summary>
internal static unsafe partial class NativeMethods
{
    private const string Lib = "quicmedia";

    [LibraryImport(Lib)]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial int qm_init();

    [LibraryImport(Lib)]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial int qm_generate_self_signed(
        byte* sansCsv, byte** certOut, nuint* certLen, byte** keyOut, nuint* keyLen);

    [LibraryImport(Lib)]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial int qm_cert_sha256(byte* cert, nuint certLen, byte* outHash);

    [LibraryImport(Lib)]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial int qm_engine_create(QmAudioParams* parms, int backend, QmCallbacks* cbs, out IntPtr engine);

    [LibraryImport(Lib)]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial void qm_engine_destroy(IntPtr engine);

    [LibraryImport(Lib)]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial int qm_engine_connect(
        IntPtr engine, byte* bind, byte* peer, byte* serverName, QmSecurity* sec, int enableData, out ulong outConn);

    [LibraryImport(Lib)]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial int qm_engine_listen(IntPtr engine, byte* bind, QmSecurity* sec, int enableData);

    [LibraryImport(Lib)]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial int qm_engine_open_stream(
        IntPtr engine, ulong conn, int codec, uint bitrate, int active, out ushort outStream);

    [LibraryImport(Lib)]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial int qm_engine_add_capture_stream(
        IntPtr engine, ulong conn, int codec, uint bitrate, int active, out ushort outStream);

    [LibraryImport(Lib)]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial int qm_engine_bind_playback(IntPtr engine, ulong conn, ushort stream);

    [LibraryImport(Lib)]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial int qm_engine_set_stream_active(IntPtr engine, ulong conn, ushort stream, int active);

    [LibraryImport(Lib)]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial int qm_engine_close_stream(IntPtr engine, ulong conn, ushort stream);

    [LibraryImport(Lib)]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial int qm_engine_add_route(
        IntPtr engine, ulong srcConn, ushort srcStream, ulong dstConn, ushort dstStream);

    [LibraryImport(Lib)]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial int qm_engine_remove_route(
        IntPtr engine, ulong srcConn, ushort srcStream, ulong dstConn, ushort dstStream);

    [LibraryImport(Lib)]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial int qm_engine_send_data(IntPtr engine, ulong conn, byte* data, nuint len);

    [LibraryImport(Lib)]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial int qm_engine_close_connection(IntPtr engine, ulong conn);

    [LibraryImport(Lib)]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial int qm_engine_connection_stats(IntPtr engine, ulong conn, out QmConnStats outStats);

    [LibraryImport(Lib)]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial void qm_buffer_free(byte* ptr, nuint len);

    [LibraryImport(Lib)]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial byte* qm_error_message(int code);
}
