using System.Runtime.InteropServices;
using QuicMedia.Interop;

namespace QuicMedia;

/// <summary>Helpers for self-signed certificates and certificate pinning.</summary>
public static class QuicMediaCertificate
{
    /// <summary>
    /// Generate a fresh self-signed certificate. Returns the certificate and its
    /// PKCS#8 private key, both DER-encoded.
    /// </summary>
    public static unsafe (byte[] Certificate, byte[] PrivateKey) GenerateSelfSigned(params string[] subjectAltNames)
    {
        IntPtr csv = subjectAltNames.Length == 0
            ? IntPtr.Zero
            : Marshal.StringToCoTaskMemUTF8(string.Join(',', subjectAltNames));
        try
        {
            byte* certPtr;
            nuint certLen;
            byte* keyPtr;
            nuint keyLen;
            int rc = NativeMethods.qm_generate_self_signed((byte*)csv, &certPtr, &certLen, &keyPtr, &keyLen);
            if (rc != 0) throw new QuicMediaException(rc, NativeError.Message(rc));

            var cert = new byte[(int)certLen];
            Marshal.Copy((IntPtr)certPtr, cert, 0, cert.Length);
            var key = new byte[(int)keyLen];
            Marshal.Copy((IntPtr)keyPtr, key, 0, key.Length);

            NativeMethods.qm_buffer_free(certPtr, certLen);
            NativeMethods.qm_buffer_free(keyPtr, keyLen);
            return (cert, key);
        }
        finally
        {
            if (csv != IntPtr.Zero) Marshal.FreeCoTaskMem(csv);
        }
    }

    /// <summary>SHA-256 digest of a certificate DER, for <see cref="QuicVerification.PinnedSha256"/>.</summary>
    public static unsafe byte[] Sha256(byte[] certificateDer)
    {
        ArgumentNullException.ThrowIfNull(certificateDer);
        var outHash = new byte[32];
        fixed (byte* c = certificateDer)
        fixed (byte* o = outHash)
        {
            int rc = NativeMethods.qm_cert_sha256(c, (nuint)certificateDer.Length, o);
            if (rc != 0) throw new QuicMediaException(rc, NativeError.Message(rc));
        }
        return outHash;
    }
}
