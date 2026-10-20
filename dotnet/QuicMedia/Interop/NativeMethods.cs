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
        byte* sansCsv,
        byte** certOut, nuint* certLen,
        byte** keyOut, nuint* keyLen);

    [LibraryImport(Lib)]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial int qm_cert_sha256(byte* cert, nuint certLen, byte* outHash);

    [LibraryImport(Lib)]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial int qm_session_start(QmConfigNative* cfg, QmCallbacksNative* cbs, out IntPtr handle);

    [LibraryImport(Lib)]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial int qm_session_send_data(IntPtr session, byte* data, nuint len);

    [LibraryImport(Lib)]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial int qm_session_get_stats(IntPtr session, out QmStatsNative stats);

    [LibraryImport(Lib)]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial int qm_session_state(IntPtr session);

    [LibraryImport(Lib)]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial void qm_session_stop(IntPtr session);

    [LibraryImport(Lib)]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial void qm_buffer_free(byte* ptr, nuint len);

    [LibraryImport(Lib)]
    [UnmanagedCallConv(CallConvs = new[] { typeof(CallConvCdecl) })]
    internal static partial byte* qm_error_message(int code);
}
