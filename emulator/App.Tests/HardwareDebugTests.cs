using System.Runtime.InteropServices;
using System.Text;
using Avalonia.Headless;
using Kb.HostProtocol;

namespace KbEmulator.Tests;

/// <summary>The DEBUG tab's model of the real keyboard, fed with debug stream events.</summary>
public class HardwareMirrorTests
{
    private const string Status =
        "#S layer=0 caps=0 mods=00 out=ble profile=1 host=1 open=0 linked=1 stage=- " +
        "bat=87,90 chg=1,0 wpm=0 power=active presses=0 mute=0 trace=0 lit=1";

    private static HardwareMirror NewMirror() => new(layer => $"layer{layer}", () => 0);

    private static int LitPixels(Kb.Ssd1306.Ssd1306 display)
    {
        var n = 0;
        for (var y = 0; y < display.PanelHeight; y++)
        {
            for (var x = 0; x < Kb.Ssd1306.Ssd1306.Columns; x++)
            {
                if (display.GetPixel(x, y)) n++;
            }
        }
        return n;
    }

    [Test]
    public async Task Frame_from_the_keyboard_shows_on_the_left_oled()
    {
        // Arrange
        using var mirror = NewMirror();
        var frame = new byte[512];
        frame[0] = 0xFF; // 8 pixels in the top left corner
        mirror.Apply(new DebugFrame(frame));

        // Act
        mirror.UpdateDisplays();

        // Assert
        await Assert.That(LitPixels(mirror.Displays[0])).IsEqualTo(8);
    }

    [Test]
    public async Task Frame_off_switches_the_left_oled_off()
    {
        // Arrange
        using var mirror = NewMirror();
        mirror.Apply(new DebugFrame(new byte[512]));
        mirror.UpdateDisplays();
        mirror.Apply(new DebugFrame(null));

        // Act
        mirror.UpdateDisplays();

        // Assert
        await Assert.That(mirror.Displays[0].IsLit).IsFalse();
    }

    [Test]
    public async Task Right_oled_is_rendered_from_the_status()
    {
        // Arrange
        using var mirror = NewMirror();
        mirror.Apply(DebugProtocol.Parse(Status));

        // Act
        mirror.UpdateDisplays();

        // Assert
        await Assert.That(LitPixels(mirror.Displays[1])).IsGreaterThan(50);
    }

    [Test]
    public async Task Held_keys_become_key_events()
    {
        // Arrange
        using var mirror = NewMirror();
        var events = new List<string>();
        mirror.Event += events.Add;

        // Act
        mirror.Apply(new DebugKeys(1UL << 1));

        // Assert
        await Assert.That(events).IsEquivalentTo(new[] { "key 1 down: Q" });
    }

    [Test]
    public async Task Reports_type_into_the_computer_pane()
    {
        // Arrange
        using var mirror = NewMirror();
        mirror.Apply(DebugProtocol.Parse(Status));
        mirror.Apply(DebugProtocol.Parse("#R 0000040000000000")); // a

        // Act
        mirror.Apply(DebugProtocol.Parse("#R 0000000000000000"));

        // Assert
        await Assert.That(mirror.Host.Text).IsEqualTo("a");
    }

    [Test]
    public async Task Muted_reports_do_not_type()
    {
        // Arrange
        using var mirror = NewMirror();
        mirror.Apply(DebugProtocol.Parse(Status.Replace("mute=0", "mute=1")));

        // Act
        mirror.Apply(DebugProtocol.Parse("#R 0000040000000000"));

        // Assert
        await Assert.That(mirror.Host.Text).IsEqualTo("");
    }

    [Test]
    public async Task Labels_follow_the_keyboards_layer()
    {
        // Arrange
        using var mirror = NewMirror();
        mirror.Apply(DebugProtocol.Parse(Status.Replace("layer=0", "layer=1")));

        // Act
        var label = mirror.Label(1);

        // Assert: Q is `!` on symbols
        await Assert.That(label).IsEqualTo("!");
    }

    [Test]
    public async Task Boot_report_keys_become_the_nkro_bitmap()
    {
        // Arrange
        var report = new DebugReport(0x02, [0x04, 0x1D, 0, 0, 0, 0]);

        // Act
        var hid = HardwareMirror.ToHidReport(report);

        // Assert
        await Assert.That((hid.Mods, hid.Contains(0x04), hid.Contains(0x1D), hid.Contains(0x05)))
            .IsEqualTo(((byte)0x02, true, true, false));
    }
}

/// <summary>
/// The DEBUG tab against a pseudo-terminal standing in for the keyboard's
/// log port: what the "keyboard" writes shows up, what the tab sends arrives.
/// </summary>
[NotInParallel("ui-session")] // timing tests must not wait behind renders on the one UI thread
public class DebugTabTests
{
    private static HeadlessUnitTestSession Session => TestAppBuilder.Session;

    [Test]
    public async Task Port_reads_what_the_keyboard_writes()
    {
        // Arrange
        using var pty = new FakeKeyboard();
        using var port = new HardwarePort(() => pty.Path);
        var received = new StringBuilder();
        port.Received += bytes => { lock (received) received.Append(Encoding.ASCII.GetString(bytes)); };
        var connected = new TaskCompletionSource();
        port.Connected += _ => connected.TrySetResult();
        port.Start();
        await connected.Task.WaitAsync(TimeSpan.FromSeconds(5));

        // Act
        pty.Write("  1234 boot\r\n");

        // Assert
        await WaitFor(() => { lock (received) return received.ToString().Contains("boot"); });
        await Assert.That(received.ToString()).IsEqualTo("  1234 boot\r\n");
    }

    [Test]
    public async Task Tab_asks_for_the_debug_stream_when_it_connects()
    {
        // Arrange
        using var pty = new FakeKeyboard();
        var view = await Session.Dispatch(() => new DebugView(_ => "base", 42, new HardwarePort(() => pty.Path)), CancellationToken.None);

        // Act
        await Session.Dispatch(view.Activate, CancellationToken.None);

        // Assert
        var sent = await pty.ReadUntil("\n");
        await Session.Dispatch(view.Dispose, CancellationToken.None);
        await Assert.That(sent).IsEqualTo(DebugProtocol.DebugOn);
    }

    [Test]
    public async Task Debug_stream_shows_the_keyboard_and_its_held_keys()
    {
        // Arrange
        using var pty = new FakeKeyboard();
        var view = await Session.Dispatch(() => new DebugView(_ => "base", 42, new HardwarePort(() => pty.Path)), CancellationToken.None);
        await Session.Dispatch(view.Activate, CancellationToken.None);
        await pty.ReadUntil("\n");

        // Act
        pty.Write("  5000 split: linked\r\n#H 1 kb_a left ble\r\n#K 0000000000001000\r\n");

        // Assert
        await WaitFor(async () => await Session.Dispatch(() =>
        {
            view.Pump();
            return view.Mirror.Held != 0;
        }, CancellationToken.None));
        var (connection, held, log) = await Session.Dispatch(() => (view.ConnectionText, view.Mirror.Held, view.LogText), CancellationToken.None);
        await Session.Dispatch(view.Dispose, CancellationToken.None);
        await Assert.That(connection).StartsWith("Connected: kb_a (left half, ble)");
        await Assert.That(held).IsEqualTo(1UL << 12);
        await Assert.That(log).Contains("split: linked");
    }

    [Test]
    public async Task Clicking_a_key_presses_it_on_the_keyboard()
    {
        // Arrange
        using var pty = new FakeKeyboard();
        var view = await Session.Dispatch(() => new DebugView(_ => "base", 42, new HardwarePort(() => pty.Path)), CancellationToken.None);
        await Session.Dispatch(view.Activate, CancellationToken.None);
        await pty.ReadUntil("\n");

        // Act
        await Session.Dispatch(() => view.View.Click(5), CancellationToken.None);

        // Assert
        var sent = await pty.ReadUntil("r5\n");
        await Session.Dispatch(view.Dispose, CancellationToken.None);
        await Assert.That(sent).IsEqualTo("p5\nr5\n");
    }

    [Test]
    public async Task Losing_the_port_dims_the_keyboard()
    {
        // Arrange
        var pty = new FakeKeyboard();
        var view = await Session.Dispatch(() => new DebugView(_ => "base", 42, new HardwarePort(() => File.Exists(pty.Path) ? pty.Path : null)), CancellationToken.None);
        await Session.Dispatch(view.Activate, CancellationToken.None);
        await pty.ReadUntil("\n");
        pty.Write("#H 1 kb_a left ble\r\n");
        await WaitFor(async () => await Session.Dispatch(() =>
        {
            view.Pump();
            return !view.View.Dimmed;
        }, CancellationToken.None));

        // Act: the keyboard restarts (its port goes away)
        pty.Dispose();

        // Assert
        await WaitFor(async () => await Session.Dispatch(() =>
        {
            view.Pump();
            return view.View.Dimmed;
        }, CancellationToken.None));
        var connection = await Session.Dispatch(() => view.ConnectionText, CancellationToken.None);
        await Session.Dispatch(view.Dispose, CancellationToken.None);
        await Assert.That(connection).StartsWith("Not connected");
    }

    [Test]
    public async Task A_silent_keyboard_is_told_apart_from_a_log_only_one()
    {
        // Arrange
        const string device = "/dev/ttyACM0";

        // Act
        var (silent, logOnly) = (DebugView.NoStreamMessage(device, 0), DebugView.NoStreamMessage(device, 120));

        // Assert
        await Assert.That(silent).Contains("sends nothing");
        await Assert.That(logOnly).Contains("log only");
    }

    [Test]
    public async Task Firmware_without_the_debug_stream_says_so()
    {
        // Arrange: a keyboard that only logs
        using var pty = new FakeKeyboard();
        var view = await Session.Dispatch(() => new DebugView(_ => "base", 42, new HardwarePort(() => pty.Path)), CancellationToken.None);
        await Session.Dispatch(view.Activate, CancellationToken.None);
        await pty.ReadUntil("\n");
        pty.Write("  5000 split: linked\r\n");

        // Act: past the time the debug stream should have started
        await Task.Delay(TimeSpan.FromSeconds(2.5));

        // Assert
        await WaitFor(async () => await Session.Dispatch(() =>
        {
            view.Pump();
            return view.ConnectionText.Contains("log only");
        }, CancellationToken.None));
        var dimmed = await Session.Dispatch(() => view.View.Dimmed, CancellationToken.None);
        await Session.Dispatch(view.Dispose, CancellationToken.None);
        await Assert.That(dimmed).IsTrue();
    }

    [Test]
    public async Task Window_has_the_five_modes()
    {
        // Arrange
        var socket = Path.Combine(Path.GetTempPath(), $"kb-tab-test-{Guid.NewGuid():N}.sock");

        // Act
        var tabs = await Session.Dispatch(() =>
        {
            var window = new MainWindow(socket, new HardwarePort(() => null));
            var names = window.ModeNames();
            window.Close();
            return names;
        }, CancellationToken.None);

        // Assert
        await Assert.That(tabs).IsEquivalentTo(new[] { "Emulator", "Debug", "Studio", "Flash", "Security" }); // the test app switches Security on
    }

    [Test]
    public async Task Debug_tab_renders_the_keyboards_frame()
    {
        // Arrange: a frame with the top quarter of the module lit
        var frame = new byte[512];
        Array.Fill(frame, (byte)0xFF, 0, 128);
        var stream = "#H 1 kb_a left ble\r\n#K 0000000000001002\r\n" +
                     "#S layer=1 caps=0 mods=02 out=ble profile=2 host=1 open=0 linked=1 stage=linked bat=87,90 chg=1,0 wpm=40 power=active presses=12 mute=1 trace=0 lit=1\r\n" +
                     "#R 0200040000000000\r\n  31337 split: linked\r\n#F " + Convert.ToBase64String(frame) + "\r\n";
        var path = Path.Combine(AppContext.BaseDirectory, "main-window-debug.png");
        var socket = Path.Combine(Path.GetTempPath(), $"kb-debug-render-{Guid.NewGuid():N}.sock");

        // Act
        var lit = await Session.Dispatch(() =>
        {
            var window = new MainWindow(socket, new HardwarePort(() => null));
            window.ShowDebug(true);
            // The port's "not found" first (from its thread), then the stream.
                var until = DateTime.UtcNow + TimeSpan.FromSeconds(3);
                do { window.Debug.Pump(); Thread.Sleep(5); } while (!window.Debug.ConnectionText.StartsWith("Not connected:", StringComparison.Ordinal) && DateTime.UtcNow < until);
            window.Debug.Feed(stream);
            window.Debug.Pump();
            window.Show();
            Avalonia.Threading.Dispatcher.UIThread.RunJobs();
            var size = new Avalonia.PixelSize((int)window.Bounds.Width, (int)window.Bounds.Height);
            using var bitmap = new Avalonia.Media.Imaging.RenderTargetBitmap(size);
            bitmap.Render(window);
            using (var file = File.Create(path)) bitmap.Save(file, Avalonia.Media.Imaging.PngBitmapEncoderOptions.Default);
            var leftLit = window.Debug.Mirror.Displays[0].IsLit;
            window.Close();
            return leftLit;
        }, CancellationToken.None);

        // Assert
        await Assert.That(lit).IsTrue();
    }

    private static async Task WaitFor(Func<bool> condition) => await WaitFor(() => Task.FromResult(condition()));

    private static async Task WaitFor(Func<Task<bool>> condition)
    {
        var until = DateTime.UtcNow + TimeSpan.FromSeconds(5);
        while (!await condition())
        {
            if (DateTime.UtcNow > until) throw new TimeoutException("condition not met in 5 s");
            await Task.Delay(20);
        }
    }
}

/// <summary>The keyboard's side of a pseudo-terminal: the app opens <see cref="Path"/>.</summary>
internal sealed class FakeKeyboard : IDisposable
{
    private const short PollIn = 1;

    private readonly int _master;
    private readonly StringBuilder _read = new();
    private bool _disposed;

    public FakeKeyboard()
    {
        _master = posix_openpt(2 | 0x100 | 0x80000); // O_RDWR | O_NOCTTY | O_CLOEXEC
        if (_master < 0 || grantpt(_master) != 0 || unlockpt(_master) != 0) throw new IOException("no pty");
        var name = new byte[128];
        if (ptsname_r(_master, name, (nuint)name.Length) != 0) throw new IOException("no pty name");
        Path = Encoding.UTF8.GetString(name, 0, Array.IndexOf(name, (byte)0));
    }

    public string Path { get; }

    public void Write(string text)
    {
        var bytes = Encoding.ASCII.GetBytes(text);
        write(_master, bytes, bytes.Length);
    }

    /// <summary>What the app sent, up to and including <paramref name="end"/>.</summary>
    public async Task<string> ReadUntil(string end)
    {
        var until = DateTime.UtcNow + TimeSpan.FromSeconds(5);
        var buf = new byte[256];
        while (DateTime.UtcNow < until)
        {
            var text = _read.ToString();
            var at = text.IndexOf(end, StringComparison.Ordinal);
            if (at >= 0)
            {
                _read.Remove(0, at + end.Length);
                return text[..(at + end.Length)];
            }
            if ((Poll(PollIn, 20) & PollIn) != 0)
            {
                var n = read(_master, buf, buf.Length);
                if (n > 0) _read.Append(Encoding.ASCII.GetString(buf, 0, (int)n));
            }
            await Task.Yield();
        }
        throw new TimeoutException($"never got {end.Replace("\n", "\\n")}; got {_read}");
    }

    private short Poll(short events, int timeoutMs)
    {
        var fds = new[] { new PollFd { Fd = _master, Events = events } };
        return poll(fds, 1, timeoutMs) > 0 ? fds[0].Revents : (short)0;
    }

    public void Dispose()
    {
        if (_disposed) return;
        _disposed = true;
        _ = close(_master); // a test pty: nothing to do about a failed close
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct PollFd
    {
        public int Fd;
        public short Events;
        public short Revents;
    }

    [DllImport("libc")] private static extern int posix_openpt(int flags);
    [DllImport("libc")] private static extern int grantpt(int fd);
    [DllImport("libc")] private static extern int unlockpt(int fd);
    [DllImport("libc")] private static extern int ptsname_r(int fd, byte[] buf, nuint len);
    [DllImport("libc")] private static extern int close(int fd);
    [DllImport("libc")] private static extern nint read(int fd, byte[] buf, nint count);
    [DllImport("libc")] private static extern nint write(int fd, byte[] buf, nint count);
    [DllImport("libc")] private static extern int poll([In, Out] PollFd[] fds, ulong nfds, int timeout);
}
