using System.Runtime.InteropServices;

namespace QuicMedia.Interop;

internal static class NativeError
{
    public static unsafe string Message(int code)
    {
        byte* p = NativeMethods.qm_error_message(code);
        return Marshal.PtrToStringUTF8((IntPtr)p) ?? $"error {code}";
    }
}
