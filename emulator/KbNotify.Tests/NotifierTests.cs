using System.ComponentModel;
using System.Net.Sockets;
using Kb.HostProtocol;

namespace KbNotify.Tests;

public class NotifierTests
{
    private static readonly RawHidDevice Usb = new("/dev/hidraw4", HidTransport.Usb, 0, NotificationPackets.ReportSize);
    private static readonly RawHidDevice Bluetooth = new("/dev/hidraw1", HidTransport.Bluetooth, 3, NotificationPackets.ReportSize);

    private const string WindowsPath = @"\\?\hid#vid_1209&pid_0001&mi_04#8&2a1b3c4d&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}";

    [Test]
    public async Task Help_prints_the_usage_and_succeeds()
    {
        // Arrange
        var fake = new FakeComputer();

        // Act
        var code = await fake.RunAsync("--help");

        // Assert
        await Assert.That(code).IsEqualTo(0);
        await Assert.That(fake.Out.ToString()).IsEqualTo(Options.Usage + Environment.NewLine);
    }

    [Test]
    public async Task No_text_is_nothing_to_send()
    {
        // Arrange
        var fake = new FakeComputer { Keyboards = [Usb] };

        // Act
        var code = await fake.RunAsync("--emulator");

        // Assert
        await Assert.That(code).IsEqualTo(2);
        await Assert.That(fake.Error.ToString().Trim()).IsEqualTo("nothing to send (try --help)");
        await Assert.That(fake.Opened).IsEmpty();
    }

    [Test]
    public async Task List_prints_each_keyboard_in_order()
    {
        // Arrange
        var fake = new FakeComputer { Keyboards = [Usb, Bluetooth] };

        // Act
        var code = await fake.RunAsync("--list");

        // Assert
        await Assert.That(code).IsEqualTo(0);
        await Assert.That(fake.OutLines()).IsEquivalentTo(new[]
        {
            "Usb       report 0  /dev/hidraw4",
            "Bluetooth report 3  /dev/hidraw1",
        });
    }

    [Test]
    public async Task List_without_a_keyboard_fails_with_the_linux_hint()
    {
        // Arrange
        var fake = new FakeComputer();

        // Act
        var code = await fake.RunAsync("--list");

        // Assert: the emulator is not mentioned, it isn't listed
        await Assert.That(code).IsEqualTo(1);
        await Assert.That(fake.Error.ToString().Trim()).IsEqualTo(
            "kbnotify: no keyboard with Raw HID found (is the firmware flashed? can you read /sys/class/hidraw and write /dev/hidraw*? over Bluetooth: paired again since the update?)");
    }

    [Test]
    public async Task List_without_a_keyboard_on_windows_has_no_hidraw_hint()
    {
        // Arrange
        var fake = new FakeComputer { IsWindows = true };

        // Act
        await fake.RunAsync("--list");

        // Assert
        await Assert.That(fake.Error.ToString().Trim()).IsEqualTo(
            "kbnotify: no keyboard with Raw HID found (is the firmware flashed? over Bluetooth: paired again since the update?)");
    }

    [Test]
    public async Task Text_goes_to_the_first_keyboard_when_no_emulator_runs()
    {
        // Arrange
        var fake = new FakeComputer { Keyboards = [Usb, Bluetooth] };

        // Act
        var code = await fake.RunAsync("Build", "passed");

        // Assert
        await Assert.That(code).IsEqualTo(0);
        await Assert.That(fake.Opened.Single()).IsEqualTo(Usb);
        await Assert.That(fake.Out.ToString().Trim()).IsEqualTo("sent 1 packet(s) to keyboard over USB (/dev/hidraw4)");
    }

    [Test]
    public async Task Long_text_is_sent_in_as_many_packets_as_it_encodes_to()
    {
        // Arrange
        var text = new string('x', 100);
        var fake = new FakeComputer { Keyboards = [Usb] };

        // Act
        await fake.RunAsync(text);

        // Assert
        var expected = NotificationPackets.Encode(text).Count;
        await Assert.That(expected).IsGreaterThan(1);
        await Assert.That(fake.Link.Sent.Count).IsEqualTo(expected);
        await Assert.That(fake.Link.Sent.All(p => p.Length == NotificationPackets.ReportSize)).IsTrue();
        await Assert.That(fake.Out.ToString().Trim()).IsEqualTo($"sent {expected} packet(s) to keyboard over USB (/dev/hidraw4)");
    }

    [Test]
    public async Task Clear_sends_one_clear_packet()
    {
        // Arrange
        var fake = new FakeComputer { Keyboards = [Bluetooth] };

        // Act
        var code = await fake.RunAsync("--clear");

        // Assert
        await Assert.That(code).IsEqualTo(0);
        await Assert.That(fake.Link.Sent.Single()).IsEquivalentTo(NotificationPackets.Clear());
        await Assert.That(fake.Out.ToString().Trim()).IsEqualTo("cleared on keyboard over Bluetooth (/dev/hidraw1)");
    }

    [Test]
    public async Task Link_is_closed_after_sending()
    {
        // Arrange
        var fake = new FakeComputer { Keyboards = [Usb] };

        // Act
        await fake.RunAsync("Hi");

        // Assert
        await Assert.That(fake.Link.Disposed).IsTrue();
    }

    [Test]
    public async Task Running_emulator_is_preferred_over_the_keyboard()
    {
        // Arrange
        var fake = new FakeComputer { Keyboards = [Usb], SocketExists = true };

        // Act
        var code = await fake.RunAsync("Hi");

        // Assert
        await Assert.That(code).IsEqualTo(0);
        await Assert.That(fake.Opened).IsEmpty();
        await Assert.That(fake.Out.ToString().Trim()).IsEqualTo("sent 1 packet(s) to emulator (test.sock)");
    }

    [Test]
    public async Task Stale_emulator_socket_falls_through_to_the_keyboard()
    {
        // Arrange
        var fake = new FakeComputer { Keyboards = [Usb], SocketExists = true, EmulatorRefuses = true };

        // Act
        var code = await fake.RunAsync("Hi");

        // Assert
        await Assert.That(code).IsEqualTo(0);
        await Assert.That(fake.Opened.Single()).IsEqualTo(Usb);
    }

    [Test]
    public async Task Emulator_asked_for_but_not_running_is_an_error()
    {
        // Arrange
        var fake = new FakeComputer { Keyboards = [Usb], EmulatorRefuses = true };

        // Act
        var code = await fake.RunAsync("--emulator", "Hi");

        // Assert: no fall-through to the keyboard
        await Assert.That(code).IsEqualTo(1);
        await Assert.That(fake.Opened).IsEmpty();
        await Assert.That(fake.Error.ToString()).StartsWith("kbnotify: ");
    }

    [Test]
    public async Task No_emulator_and_no_keyboard_says_so()
    {
        // Arrange
        var fake = new FakeComputer();

        // Act
        var code = await fake.RunAsync("Hi");

        // Assert
        await Assert.That(code).IsEqualTo(1);
        await Assert.That(fake.Error.ToString()).StartsWith("kbnotify: no emulator running and no keyboard with Raw HID found");
    }

    [Test]
    public async Task Device_path_on_linux_opens_that_hidraw_node()
    {
        // Arrange: an emulator runs, but --device wins
        var fake = new FakeComputer { Keyboards = [Usb], SocketExists = true };

        // Act
        await fake.RunAsync("--device", "/dev/hidraw9", "Hi");

        // Assert
        await Assert.That(fake.Opened.Single().Path).IsEqualTo("/dev/hidraw9");
        await Assert.That(fake.LookedUpPaths).IsEquivalentTo(new[] { "/dev/hidraw9" });
    }

    [Test]
    public async Task Device_path_on_windows_is_one_of_the_keyboards_found()
    {
        // Arrange: Windows paths compare without case
        var windowsUsb = Usb with { Path = WindowsPath };
        var fake = new FakeComputer { IsWindows = true, Keyboards = [windowsUsb] };

        // Act
        await fake.RunAsync("--device", WindowsPath.ToUpperInvariant(), "Hi");

        // Assert
        await Assert.That(fake.Opened.Single()).IsEqualTo(windowsUsb);
        await Assert.That(fake.LookedUpPaths).IsEmpty();
    }

    [Test]
    public async Task Device_path_on_windows_that_is_not_the_keyboard_is_an_error()
    {
        // Arrange
        var fake = new FakeComputer { IsWindows = true, Keyboards = [Usb with { Path = WindowsPath }] };

        // Act
        var code = await fake.RunAsync("--device", @"\\?\hid#other", "Hi");

        // Assert
        await Assert.That(code).IsEqualTo(1);
        await Assert.That(fake.Error.ToString().Trim())
            .IsEqualTo(@"kbnotify: \\?\hid#other is not the keyboard's Raw HID collection (see --list)");
    }

    [Test]
    public async Task No_access_on_linux_points_to_the_udev_rule()
    {
        // Arrange
        var fake = new FakeComputer { Keyboards = [Usb], OpenFails = new UnauthorizedAccessException("denied") };

        // Act
        var code = await fake.RunAsync("Hi");

        // Assert
        await Assert.That(code).IsEqualTo(1);
        await Assert.That(fake.Error.ToString().Trim()).IsEqualTo(
            "kbnotify: no access to /dev/hidraw4: add the udev rule from the README (or run with sudo once to check)");
    }

    [Test]
    public async Task No_access_on_windows_names_the_device()
    {
        // Arrange
        var fake = new FakeComputer
        {
            IsWindows = true,
            Keyboards = [Usb with { Path = WindowsPath }],
            OpenFails = new UnauthorizedAccessException("denied"),
        };

        // Act
        await fake.RunAsync("Hi");

        // Assert
        await Assert.That(fake.Error.ToString().Trim()).IsEqualTo($"kbnotify: no access to {WindowsPath}");
    }

    [Test]
    public async Task Other_open_failures_are_one_line_with_their_message()
    {
        // Arrange
        var fake = new FakeComputer { Keyboards = [Usb], OpenFails = new IOException("device gone") };

        // Act
        var code = await fake.RunAsync("Hi");

        // Assert
        await Assert.That(code).IsEqualTo(1);
        await Assert.That(fake.Error.ToString().Trim()).IsEqualTo("kbnotify: device gone");
    }

    [Test]
    [MethodDataSource(nameof(SendFailures))]
    public async Task Send_failure_is_reported_with_the_device(Exception failure)
    {
        // Arrange
        var fake = new FakeComputer { Keyboards = [Usb] };
        fake.Link.SendFails = failure;

        // Act
        var code = await fake.RunAsync("Hi");

        // Assert
        await Assert.That(code).IsEqualTo(1);
        await Assert.That(fake.Error.ToString().Trim())
            .IsEqualTo($"kbnotify: sending to keyboard over USB (/dev/hidraw4) failed: {failure.Message}");
        await Assert.That(fake.Link.Disposed).IsTrue();
    }

    public static IEnumerable<Func<Exception>> SendFailures()
    {
        yield return () => new IOException("broken pipe");
        yield return () => new UnauthorizedAccessException("denied");
        yield return () => new Win32Exception(5, "access denied");
    }

    /// <summary>A computer with the given keyboards and emulator, and what kbnotify did on it.</summary>
    private sealed class FakeComputer
    {
        public bool IsWindows { get; init; }

        public IReadOnlyList<RawHidDevice> Keyboards { get; init; } = [];

        public bool SocketExists { get; init; }

        public bool EmulatorRefuses { get; init; }

        public Exception? OpenFails { get; init; }

        public StringWriter Out { get; } = new();

        public StringWriter Error { get; } = new();

        public List<RawHidDevice> Opened { get; } = [];

        public List<string> LookedUpPaths { get; } = [];

        public FakeLink Link { get; } = new();

        public string[] OutLines() => Out.ToString().Split(Environment.NewLine, StringSplitOptions.RemoveEmptyEntries);

        public Task<int> RunAsync(params string[] args) => new Notifier(new NotifierEnvironment(
            Out,
            Error,
            IsWindows,
            () => Keyboards,
            path =>
            {
                LookedUpPaths.Add(path);
                return new RawHidDevice(path, HidTransport.Unknown, 0, NotificationPackets.ReportSize);
            },
            device =>
            {
                if (OpenFails is { } failure) throw failure;
                Opened.Add(device);
                Link.Description = device.Description;
                return Link;
            },
            () => SocketExists,
            () =>
            {
                if (EmulatorRefuses) throw new SocketException((int)SocketError.ConnectionRefused);
                Link.Description = "emulator (test.sock)";
                return Task.FromResult<IKeyboardLink>(Link);
            })).RunAsync(args);
    }

    private sealed class FakeLink : IKeyboardLink
    {
        public string Description { get; set; } = "";

        public List<byte[]> Sent { get; } = [];

        public Exception? SendFails { get; set; }

        public bool Disposed { get; private set; }

        public ValueTask SendAsync(ReadOnlyMemory<byte> report, CancellationToken cancellationToken = default)
        {
            if (SendFails is { } failure) throw failure;
            Sent.Add(report.ToArray());
            return ValueTask.CompletedTask;
        }

        public ValueTask DisposeAsync()
        {
            Disposed = true;
            return ValueTask.CompletedTask;
        }
    }
}
