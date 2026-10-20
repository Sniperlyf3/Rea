namespace QuicMedia;

/// <summary>Raised when a native <c>quicmedia</c> call returns a non-zero status.</summary>
public sealed class QuicMediaException : Exception
{
    /// <summary>The native error code (see <c>qm_error_message</c>).</summary>
    public int Code { get; }

    public QuicMediaException(int code, string message) : base(message)
    {
        Code = code;
    }
}
