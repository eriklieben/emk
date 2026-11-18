using Avalonia.Headless;
using Kb.HostProtocol;

namespace KbEmulator.Tests;

/// <summary>Debug's buttons and toggles against a pseudo-terminal standing in for the keyboard.</summary>
[NotInParallel("ui-session")]
public class DebugControlTests
{
    private const string Status =
        "#S layer=0 caps=0 mods=00 out=ble profile=1 host=1 open=0 linked=1 stage=- " +
        "bat=87,90 chg=1,0 wpm=0 power=active presses=0 mute=0 trace=0 lit=1\r\n";

    private static HeadlessUnitTestSession Session => TestAppBuilder.Session;

    /// <summary>A connected view: the "keyboard" got its #debug 1 and (optionally) sent a status.</summary>
    private static async Task<(FakeKeyboard Pty, DebugView View)> Connected(bool withStatus = false)
    {
        var pty = new FakeKeyboard();
        var view = await Session.Dispatch(() => new DebugView(_ => "base", 42, new HardwarePort(() => pty.Path)), CancellationToken.None);
        await Session.Dispatch(view.Activate, CancellationToken.None);
        await pty.ReadUntil(DebugProtocol.DebugOn);
        pty.Write("#H 1 kb_b left ble\r\n" + (withStatus ? Status : ""));
        await WaitFor(() => view.Mirror.Hello != null && (!withStatus || view.Mirror.Status != null), view);
        return (pty, view);
    }

    private static async Task WaitFor(Func<bool> done, DebugView view)
    {
        var until = DateTime.UtcNow + TimeSpan.FromSeconds(10);
        while (!await Session.Dispatch(() =>
               {
                   view.Pump();
                   return done();
               }, CancellationToken.None))
        {
            if (DateTime.UtcNow > until) throw new TimeoutException();
            await Task.Delay(20);
        }
    }

    private static async Task Close(FakeKeyboard pty, DebugView view)
    {
        await Session.Dispatch(view.Dispose, CancellationToken.None);
        pty.Dispose();
    }

    [Test]
    [Arguments("status", "s\n")]
    [Arguments("dump", "d\n")]
    [Arguments("resync", "#debug 1\n")]
    public async Task Each_button_sends_its_command(string button, string expected)
    {
        // Arrange
        var (pty, view) = await Connected();

        // Act
        await Session.Dispatch(() =>
        {
            var c = view.Controls;
            (button switch { "status" => c.Status, "dump" => c.Dump, _ => c.Resync }).PerformClick();
        }, CancellationToken.None);

        // Assert
        var sent = await pty.ReadUntil("\n");
        await Close(pty, view);
        await Assert.That(sent).IsEqualTo(expected);
    }

    [Test]
    public async Task Trace_sends_t_once_the_status_is_known()
    {
        // Arrange
        var (pty, view) = await Connected(withStatus: true);

        // Act
        await Session.Dispatch(() => view.TraceButton.PerformClick(), CancellationToken.None);

        // Assert
        var sent = await pty.ReadUntil("\n");
        await Close(pty, view);
        await Assert.That(sent).IsEqualTo(DebugProtocol.Trace);
    }

    [Test]
    public async Task Mute_sends_m()
    {
        // Arrange
        var (pty, view) = await Connected(withStatus: true);

        // Act
        await Session.Dispatch(() => view.MuteButton.PerformClick(), CancellationToken.None);

        // Assert
        var sent = await pty.ReadUntil("\n");
        await Close(pty, view);
        await Assert.That(sent).IsEqualTo(DebugProtocol.Mute);
    }

    [Test]
    public async Task Driving_with_this_pc_mutes_the_keyboard()
    {
        // Arrange
        var (pty, view) = await Connected(withStatus: true);

        // Act
        await Session.Dispatch(() => view.Driving = true, CancellationToken.None);

        // Assert
        var sent = await pty.ReadUntil("\n");
        await Close(pty, view);
        await Assert.That(sent).IsEqualTo(DebugProtocol.Mute);
    }

    [Test]
    public async Task Bootloader_needs_a_second_click()
    {
        // Arrange
        var (pty, view) = await Connected();
        var armed = await Session.Dispatch(() =>
        {
            view.BootButton.PerformClick();
            return view.BootButton.Text;
        }, CancellationToken.None);

        // Act
        await Session.Dispatch(() => view.BootButton.PerformClick(), CancellationToken.None);

        // Assert
        var sent = await pty.ReadUntil("\n");
        await Close(pty, view);
        await Assert.That((armed, sent)).IsEqualTo(("Click again: restart into the bootloader", DebugProtocol.Bootloader));
    }

    [Test]
    public async Task Esc_disarms_the_bootloader_button()
    {
        // Arrange
        var (pty, view) = await Connected();
        await Session.Dispatch(() => view.BootButton.PerformClick(), CancellationToken.None);

        // Act
        var (closed, text) = await Session.Dispatch(() => (view.Escape(), view.BootButton.Text), CancellationToken.None);

        // Assert
        await Close(pty, view);
        await Assert.That((closed, text)).IsEqualTo((true, "Bootloader…"));
    }

    [Test]
    public async Task Disconnect_lets_go_of_the_port()
    {
        // Arrange
        var (pty, view) = await Connected();

        // Act
        var (running, connection, button) = await Session.Dispatch(() =>
        {
            view.ConnectButton.PerformClick();
            return (view.PortRunning, view.ConnectionText, view.ConnectButton.Text);
        }, CancellationToken.None);

        // Assert
        await Close(pty, view);
        await Assert.That((running, connection.StartsWith("Not connected: disconnected (button)", StringComparison.Ordinal), button))
            .IsEqualTo((false, true, "Connect"));
    }

    [Test]
    public async Task Connect_opens_the_port_again()
    {
        // Arrange
        var (pty, view) = await Connected();
        await Session.Dispatch(() => view.ConnectButton.PerformClick(), CancellationToken.None);

        // Act
        await Session.Dispatch(() => view.ConnectButton.PerformClick(), CancellationToken.None);

        // Assert: it asks for the debug stream again
        var sent = await pty.ReadUntil("\n");
        await Close(pty, view);
        await Assert.That(sent).IsEqualTo(DebugProtocol.DebugOn);
    }

    [Test]
    public async Task Release_latched_releases_on_the_keyboard()
    {
        // Arrange
        var (pty, view) = await Connected();
        await Session.Dispatch(() => view.View.LatchForTest(5), CancellationToken.None);
        await pty.ReadUntil("p5\n");

        // Act
        await Session.Dispatch(() => view.Controls.Release.PerformClick(), CancellationToken.None);

        // Assert
        var sent = await pty.ReadUntil("\n");
        await Close(pty, view);
        await Assert.That(sent).IsEqualTo("r5\n");
    }

    [Test]
    public async Task Clear_panes_empties_them()
    {
        // Arrange
        var (pty, view) = await Connected();
        pty.Write("  100 split: linked\r\n");
        await WaitFor(() => view.LogText.Contains("split"), view);

        // Act
        var (log, events) = await Session.Dispatch(() =>
        {
            view.Controls.Clear.PerformClick();
            return (view.LogText, view.EventText);
        }, CancellationToken.None);

        // Assert
        await Close(pty, view);
        await Assert.That((log, events)).IsEqualTo(("", ""));
    }

    [Test]
    public async Task Unticking_follow_log_pauses_the_log()
    {
        // Arrange
        var view = await Session.Dispatch(() => new DebugView(_ => "base", 42, new HardwarePort(() => null)), CancellationToken.None);

        // Act
        var follow = await Session.Dispatch(() =>
        {
            view.FollowToggle.IsChecked = false;
            return view.LogFollows;
        }, CancellationToken.None);

        // Assert
        await Session.Dispatch(view.Dispose, CancellationToken.None);
        await Assert.That(follow).IsFalse();
    }

    [Test]
    public async Task Reports_type_into_the_computer_pane()
    {
        // Arrange
        var (pty, view) = await Connected(withStatus: true);

        // Act
        pty.Write("#R 0000040000000000\r\n#R 0000000000000000\r\n");

        // Assert
        await WaitFor(() => view.HostText.Length > 0, view);
        var host = await Session.Dispatch(() => view.HostText, CancellationToken.None);
        await Close(pty, view);
        await Assert.That(host).IsEqualTo("a");
    }
}
