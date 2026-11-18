using System.Diagnostics.CodeAnalysis;
using System.Runtime.InteropServices;

namespace KbEmulator;

/// <summary>
/// The real keyboard's log port (left half, USB serial), found by its USB ID
/// and opened raw (no echo, no line editing) with DTR raised, which starts a
/// session on the keyboard. Reads on a thread of its own; reconnects by
/// itself when the keyboard restarts or is plugged in again.
/// </summary>
[SuppressMessage("Security", "S6640:Using unsafe code blocks is security-sensitive",
    Justification = "libc read/write/poll/termios take pointers; buffers are pinned for the call only.")]
public sealed unsafe partial class HardwarePort : IDisposable
{
    public const string VendorId = "1209";
    public const string ProductId = "0001";

    private readonly Lock _lock = new();
    private readonly Func<string?> _find;
    private Thread? _thread;
    private volatile bool _running;
    private int _fd = -1;

    /// <param name="find">Where the port is now (null: not there); default <see cref="Find"/>.</param>
    public HardwarePort(Func<string?>? find = null) => _find = find ?? Find;

    /// <summary>Opened this device (on the reader thread).</summary>
    public event Action<string>? Connected;

    /// <summary>The port went away or could not be used (on the reader thread).</summary>
    public event Action<string>? Disconnected;

    /// <summary>Bytes read (on the reader thread).</summary>
    public event Action<byte[]>? Received;

    public bool IsRunning => _running;

    /// <summary>Sent as soon as the port is open, before <see cref="Connected"/>.</summary>
    public string? OnConnectSend { get; set; }

    public void Start()
    {
        if (_running) return;
        _running = true;
        _thread = new Thread(Run) { IsBackground = true, Name = "keyboard log port" };
        _thread.Start();
    }

    public void Stop()
    {
        _running = false;
        _thread?.Join(TimeSpan.FromSeconds(1));
        _thread = null;
        Close();
    }

    public void Dispose() => Stop();

    /// <summary>Send a command; false when not connected.</summary>
    public bool Send(string text)
    {
        var bytes = System.Text.Encoding.ASCII.GetBytes(text);
        lock (_lock)
        {
            if (_fd < 0) return false;
            fixed (byte* p = bytes)
            {
                var done = 0;
                while (done < bytes.Length)
                {
                    var n = (int)Libc.write(_fd, p + done, (nint)(bytes.Length - done));
                    if (n <= 0) return false;
                    done += n;
                }
            }
            return true;
        }
    }

    /// <summary>
    /// The keyboard's port: <c>/dev/serial/by-id/usb-erik_*</c> (interface 1 on
    /// older firmware, 2 since the mouse got its own interface), else any
    /// <c>ttyACM</c> with USB ID 1209:0001. Null when none.
    /// </summary>
    public static string? Find()
    {
        const string byId = "/dev/serial/by-id";
        if (Directory.Exists(byId))
        {
            var link = Directory.GetFiles(byId, "usb-erik_*-if0*").OrderBy(f => f).FirstOrDefault();
            if (link != null) return link;
        }
        if (!Directory.Exists("/sys/class/tty")) return null;
        foreach (var tty in Directory.GetDirectories("/sys/class/tty", "ttyACM*").OrderBy(d => d))
        {
            try
            {
                // device = the USB interface; its parent is the USB device.
                var vid = File.ReadAllText(Path.Combine(tty, "device/../idVendor")).Trim();
                var pid = File.ReadAllText(Path.Combine(tty, "device/../idProduct")).Trim();
                if (vid == VendorId && pid == ProductId) return "/dev/" + Path.GetFileName(tty);
            }
            catch (Exception e) when (e is IOException or UnauthorizedAccessException)
            {
                // Not a USB tty we can read the IDs of: not the keyboard's.
            }
        }
        return null;
    }

    private void Run()
    {
        var buffer = new byte[4096];
        string? lastError = null;
        while (_running)
        {
            var path = _find();
            string? error = null;
            if (path == null || !TryOpen(path, out error))
            {
                var why = error ?? "no keyboard on USB (plug in the left half, a usb-log build)";
                if (why != lastError) Disconnected?.Invoke(why);
                lastError = why;
                Thread.Sleep(500);
                continue;
            }
            if (OnConnectSend is { } greeting) Send(greeting);
            Connected?.Invoke(path);
            var reason = ReadUntilGone(buffer);
            Close();
            Disconnected?.Invoke(reason);
            lastError = reason;
            Thread.Sleep(300);
        }
    }

    private const string Gone = "keyboard disconnected";
    private const int PollTimeoutMs = 200;

    private string ReadUntilGone(byte[] buffer)
    {
        while (_running)
        {
            var (ready, gone) = Poll();
            if (gone != null) return gone;
            if (!ready) continue;
            int n;
            fixed (byte* p = buffer)
            {
                n = (int)Libc.read(_fd, p, buffer.Length);
            }
            if (n < 0 && Marshal.GetLastPInvokeError() is Libc.EINTR or Libc.EAGAIN) continue;
            if (n <= 0) return Gone;
            Received?.Invoke(buffer[..n]);
        }
        return "stopped";
    }

    /// <summary>Wait a moment for input: whether there is some to read, or why the port is gone (else null).</summary>
    private (bool Ready, string? Gone) Poll()
    {
        var poll = new Libc.PollFd { Fd = _fd, Events = Libc.POLLIN };
        var ready = Libc.poll(&poll, 1, PollTimeoutMs);
        if (ready < 0) return (false, Marshal.GetLastPInvokeError() == Libc.EINTR ? null : "poll failed");
        if (ready == 0) return (false, null);
        if ((poll.Revents & (Libc.POLLERR | Libc.POLLHUP | Libc.POLLNVAL)) != 0) return (false, Gone);
        return (true, null);
    }

    private bool TryOpen(string path, out string? error)
    {
        var fd = Libc.open(path, Libc.O_RDWR | Libc.O_NOCTTY | Libc.O_CLOEXEC | Libc.O_NONBLOCK);
        if (fd < 0)
        {
            var errno = Marshal.GetLastPInvokeError();
            error = errno == Libc.EACCES
                ? $"no access to {path} (see the udev rule in README.md)"
                : $"cannot open {path} (errno {errno})";
            return false;
        }
        // Raw: no echo (a terminal would send the log back), no line editing,
        // no newline translation.
        var termios = stackalloc byte[256];
        if (Libc.tcgetattr(fd, termios) == 0)
        {
            Libc.cfmakeraw(termios);
            _ = Libc.tcsetattr(fd, Libc.TCSANOW, termios); // best effort: a cooked port still reads
        }
        // No tcflush: the keyboard starts sending as soon as DTR rises, while
        // open() is still running, and a flush here would drop that.
        // Opening raises DTR already (HUPCL); make sure, since DTR is what
        // tells the keyboard a reader is there.
        var dtr = Libc.TIOCM_DTR | Libc.TIOCM_RTS;
        _ = Libc.ioctl(fd, Libc.TIOCMBIS, ref dtr); // best effort, see above
        lock (_lock)
        {
            _fd = fd;
        }
        error = null;
        return true;
    }

    private void Close()
    {
        lock (_lock)
        {
            if (_fd >= 0) _ = Libc.close(_fd); // nothing to do about a failed close
            _fd = -1;
        }
    }

    private static partial class Libc
    {
        public const int O_RDWR = 2, O_NOCTTY = 0x100, O_NONBLOCK = 0x800, O_CLOEXEC = 0x80000;
        public const int TCSANOW = 0;
        public const short POLLIN = 1, POLLERR = 8, POLLHUP = 0x10, POLLNVAL = 0x20;
        public const int EINTR = 4, EAGAIN = 11, EACCES = 13;
        public const ulong TIOCMBIS = 0x5416;
        public const int TIOCM_DTR = 0x002, TIOCM_RTS = 0x004;

        [StructLayout(LayoutKind.Sequential)]
        public struct PollFd
        {
            public int Fd;
            public short Events;
            public short Revents;
        }

        [LibraryImport("libc", EntryPoint = "open", SetLastError = true, StringMarshalling = StringMarshalling.Utf8)]
        public static partial int open(string path, int flags);

        [DllImport("libc", SetLastError = true)] public static extern int close(int fd);
        [DllImport("libc", SetLastError = true)] public static extern nint read(int fd, byte* buf, nint count);
        [DllImport("libc", SetLastError = true)] public static extern nint write(int fd, byte* buf, nint count);
        [DllImport("libc", SetLastError = true)] public static extern int poll(PollFd* fds, ulong nfds, int timeout);
        [DllImport("libc", SetLastError = true)] public static extern int tcgetattr(int fd, byte* termios);
        [DllImport("libc", SetLastError = true)] public static extern int tcsetattr(int fd, int action, byte* termios);
        [DllImport("libc", SetLastError = true)] public static extern void cfmakeraw(byte* termios);
        [DllImport("libc", SetLastError = true)] public static extern int ioctl(int fd, ulong request, ref int arg);
    }
}
