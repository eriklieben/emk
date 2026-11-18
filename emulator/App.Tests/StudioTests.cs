namespace KbEmulator.Tests;

public class StudioLineTests
{
    [Test]
    public async Task Version_line_carries_the_keymap_shape()
    {
        // Arrange
        const string line = "#V 1 layers=4 keys=42 store=flash max=40 names=default,symbols,nav,wm\r\n";

        // Act
        var v = (StudioVersion)StudioLine.Parse(line)!;

        // Assert
        await Assert.That((v.Version, v.Layers, v.Keys, v.Store, v.Max, string.Join(",", v.Names)))
            .IsEqualTo((1, 4, 42, "flash", 40, "default,symbols,nav,wm"));
    }

    [Test]
    public async Task Layer_line_has_one_action_per_position()
    {
        // Arrange
        const string line = "#L 2 trans kp:F1 sys:StudioUnlock mo:2";

        // Act
        var l = (StudioLayer)StudioLine.Parse(line)!;

        // Assert
        await Assert.That((l.Layer, string.Join(" ", l.Actions))).IsEqualTo((2, "trans kp:F1 sys:StudioUnlock mo:2"));
    }

    [Test]
    [Arguments("#A set 0 12 ok mt:LCTRL/TAB", "set", 0, 12, true, "mt:LCTRL/TAB")]
    [Arguments("#A set 2 35 err lockout", "set", 2, 35, false, "lockout")]
    [Arguments("#A save err locked", "save", null, null, false, "locked")]
    [Arguments("#A revert ok", "revert", null, null, true, "")]
    [Arguments("#A ? err syntax", "?", null, null, false, "syntax")]
    public async Task Acknowledgements_are_parsed(string line, string command, int? layer, int? pos, bool ok, string detail)
    {
        // Arrange
        var text = line;

        // Act
        var a = (StudioAck)StudioLine.Parse(text)!;

        // Assert
        await Assert.That(a).IsEqualTo(new StudioAck(command, layer, pos, ok, detail));
    }

    [Test]
    public async Task Unlock_and_dirty_lines_are_parsed()
    {
        // Arrange
        string[] lines = ["#U unlocked 287", "#U locked", "#D 1", "#D 0"];

        // Act
        var parsed = lines.Select(StudioLine.Parse).ToArray();

        // Assert
        await Assert.That(parsed).IsEquivalentTo(new StudioLine?[]
        {
            new StudioUnlock(true, 287), new StudioUnlock(false, 0), new StudioDirty(true), new StudioDirty(false),
        });
    }

    [Test]
    public async Task Debug_lines_are_not_studio_lines()
    {
        // Arrange
        string[] lines = ["#H 1 kb_b left ble", "#S layer=0", "  1204 boot: x"];

        // Act
        var studio = lines.Any(StudioLine.IsStudio);

        // Assert
        await Assert.That(studio).IsFalse();
    }
}

public class StudioActionTests
{
    [Test]
    [Arguments("kp:LC(C)")]
    [Arguments("kp:LG(LS(N1))")]
    [Arguments("mt:LALT/TAB")]
    [Arguments("mo:2")]
    [Arguments("caps")]
    [Arguments("sys:StudioUnlock")]
    [Arguments("trans")]
    [Arguments("none")]
    [Arguments("kp:EXCL")]
    public async Task Parse_then_encode_gives_the_canonical_form_back(string canonical)
    {
        // Arrange
        var model = StudioAction.Parse(canonical);

        // Act
        var encoded = model.Encode();

        // Assert
        await Assert.That(StudioCodec.Canonical(encoded)).IsEqualTo(canonical);
    }

    [Test]
    public async Task Modifiers_are_written_in_canonical_order()
    {
        // Arrange: chosen in another order than canonical
        var model = new StudioAction { Kind = StudioKind.Key, Key = "N1" };
        model.Mods.Add("LS");
        model.Mods.Add("LG");

        // Act
        var encoded = model.Encode();

        // Assert
        await Assert.That((encoded, StudioCodec.Canonical(encoded))).IsEqualTo(("kp:LG(LS(N1))", "kp:LG(LS(N1))"));
    }

    [Test]
    public async Task Every_default_binding_survives_the_editor()
    {
        // Arrange
        using var keyboard = new EmulatedKeyboard(() => 0);
        var bindings = Enumerable.Range(0, 4).SelectMany(l => Enumerable.Range(0, 42).Select(p => keyboard.StudioDefault(l, p))).ToList();

        // Act
        var roundTrip = bindings.Select(b => StudioCodec.Canonical(StudioAction.Parse(b).Encode())).ToList();

        // Assert
        await Assert.That(roundTrip).IsEquivalentTo(bindings.Select(b => (string?)b));
    }

    [Test]
    public async Task Lists_come_from_the_firmware()
    {
        // Arrange
        var keys = StudioCodec.KeyNames;

        // Act
        var systems = StudioCodec.SystemNames;

        // Assert
        await Assert.That((keys.Contains("A"), keys.Contains("EXCL"), systems.Contains("StudioUnlock"))).IsEqualTo((true, true, true));
    }

    [Test]
    public async Task Legends_follow_the_encoding()
    {
        // Arrange
        const string modTap = "mt:LALT/TAB";

        // Act
        var labels = (StudioCodec.Label("kp:EXCL"), StudioCodec.Label(modTap), StudioCodec.Label("mo:2"));

        // Assert
        await Assert.That(labels).IsEqualTo(("!", "Tab\nAlt", "nav"));
    }
}

/// <summary>The protocol client against the emulator (same text as the keyboard) and fake ports.</summary>
[NotInParallel("ui-session")]
public class StudioClientTests
{
    private static string TempKeymap() => Path.Combine(Path.GetTempPath(), $"emk-keymap-{Guid.NewGuid():N}", "emulator-keymap.txt");

    private static (EmulatedKeyboard Keyboard, StudioClient Client) ConnectedEmulator(string? storage = null)
    {
        var keyboard = new EmulatedKeyboard(() => 0);
        if (storage != null) keyboard.SetStudioStorage(storage);
        var client = new StudioClient(new EmulatorStudioTransport(keyboard));
        client.Get();
        client.Pump();
        return (keyboard, client);
    }

    [Test]
    public async Task Get_reads_the_version_and_every_layer()
    {
        // Arrange
        var (keyboard, client) = ConnectedEmulator();
        using var _ = keyboard;

        // Act
        var (support, layers, first) = (client.Support, client.Layers.Count, client.Binding(0, 0));

        // Assert
        await Assert.That((support, layers, first)).IsEqualTo((StudioSupport.Yes, 4, "kp:ESC"));
    }

    [Test]
    public async Task A_locked_keyboard_refuses_writes()
    {
        // Arrange
        var (keyboard, client) = ConnectedEmulator();
        using var _ = keyboard;

        // Act
        client.Set(0, 1, "kp:LC(C)");
        client.Pump();

        // Assert
        await Assert.That(client.LastAck?.Explain()).StartsWith("err locked");
    }

    [Test]
    public async Task The_guard_refuses_removing_the_unlock_key()
    {
        // Arrange
        var (keyboard, client) = ConnectedEmulator();
        using var _ = keyboard;
        keyboard.PressStudioUnlock();

        // Act
        client.Set(2, 35, "trans");
        client.Pump();

        // Assert
        await Assert.That((client.LastAck?.Ok, client.LastAck?.Detail)).IsEqualTo(((bool?)false, "lockout"));
    }

    [Test]
    public async Task The_dry_run_sees_the_guard_too()
    {
        // Arrange
        using var keyboard = new EmulatedKeyboard(() => 0);

        // Act
        var code = keyboard.StudioCheck(0, 41, "kp:A");

        // Assert: 5 = lockout
        await Assert.That(code).IsEqualTo(5);
    }

    [Test]
    public async Task An_edit_changes_the_legend_and_survives_a_restart_once_saved()
    {
        // Arrange
        var storage = TempKeymap();
        var (keyboard, client) = ConnectedEmulator(storage);
        keyboard.PressStudioUnlock();
        client.Set(0, 1, "kp:LC(C)");
        client.Pump();
        var legend = keyboard.Label(1);

        // Act
        client.Save();
        client.Pump();
        var dirtyAfterSave = client.Dirty;
        keyboard.Dispose();
        using var restarted = new EmulatedKeyboard(() => 0);
        restarted.SetStudioStorage(storage);

        // Assert
        await Assert.That((legend, dirtyAfterSave, restarted.StudioBinding(0, 1)))
            .IsEqualTo((StudioCodec.Label("kp:LC(C)"), false, "kp:LC(C)"));
    }

    [Test]
    public async Task Unlocking_shows_the_time_left()
    {
        // Arrange
        var (keyboard, client) = ConnectedEmulator();
        using var _ = keyboard;

        // Act
        keyboard.PressStudioUnlock();
        client.Pump();

        // Assert
        await Assert.That((client.Unlocked, client.UnlockRemaining > TimeSpan.FromMinutes(4))).IsEqualTo((true, true));
    }

    private sealed class FakePort : IStudioTransport
    {
        public bool Connected { get; init; } = true;
        public List<string> Sent { get; } = new();
        public Queue<string> Incoming { get; } = new();

        public bool Send(string line)
        {
            Sent.Add(line);
            return Connected;
        }

        public IEnumerable<string> Receive()
        {
            var lines = Incoming.ToList();
            Incoming.Clear();
            return lines;
        }
    }

    [Test]
    public async Task Old_firmware_without_a_version_line_has_no_studio()
    {
        // Arrange: the keyboard answers nothing (its line buffer drops #studio)
        var now = new DateTime(2026, 11, 16, 12, 0, 0, DateTimeKind.Utc);
        var port = new FakePort();
        var client = new StudioClient(port, () => now);
        client.Get();

        // Act
        now += TimeSpan.FromSeconds(1.5);
        client.Pump();

        // Assert
        await Assert.That((client.Support, port.Sent.Single())).IsEqualTo((StudioSupport.No, "#studio get"));
    }

    [Test]
    public async Task A_keyboard_that_answers_in_time_has_studio()
    {
        // Arrange
        var now = new DateTime(2026, 11, 16, 12, 0, 0, DateTimeKind.Utc);
        var port = new FakePort();
        var client = new StudioClient(port, () => now);
        client.Get();
        port.Incoming.Enqueue("#V 1 layers=4 keys=42 store=flash max=40 names=default,symbols,nav,wm");
        port.Incoming.Enqueue("#U locked");
        port.Incoming.Enqueue("#D 0");

        // Act
        now += TimeSpan.FromSeconds(0.3);
        client.Pump();

        // Assert
        await Assert.That((client.Support, client.Version?.Store, client.Unlocked)).IsEqualTo((StudioSupport.Yes, "flash", false));
    }

    [Test]
    public async Task No_keyboard_on_the_port_is_not_connected()
    {
        // Arrange
        var port = new FakePort { Connected = false };
        var client = new StudioClient(port);

        // Act
        client.Get();

        // Assert
        await Assert.That(client.Support).IsEqualTo(StudioSupport.NotConnected);
    }

    [Test]
    public async Task Studio_lines_from_the_debug_port_reach_the_client()
    {
        // Arrange
        var transport = new PortStudioTransport(_ => true);
        var client = new StudioClient(transport);
        client.Get();
        var view = await TestAppBuilder.Session.Dispatch(() => new DebugView(_ => "base", 42, new HardwarePort(() => null)), CancellationToken.None);
        view.StudioLineReceived += transport.Deliver;

        // Act
        await TestAppBuilder.Session.Dispatch(() => view.Feed("#H 1 kb_b left ble\r\n#V 1 layers=4 keys=42 store=flash max=40 names=a,b,c,d\r\n#L 0 kp:ESC\r\n"), CancellationToken.None);
        client.Pump();

        // Assert
        await TestAppBuilder.Session.Dispatch(view.Dispose, CancellationToken.None);
        await Assert.That((client.Support, client.Binding(0, 0))).IsEqualTo((StudioSupport.Yes, "kp:ESC"));
    }
}
