using Avalonia.Headless;

namespace KbEmulator.Tests;

/// <summary>Studio protocol version 2: combos, check, defaults (docs/studio-protocol.md).</summary>
public class StudioV2LineTests
{
    [Test]
    public async Task Version_two_lists_its_capabilities()
    {
        // Arrange
        const string line = "#V 2 layers=4 keys=42 store=flash max=40 combos=16 caps=check,combos,defaults names=default,symbols,nav,wm";

        // Act
        var v = (StudioVersion)StudioLine.Parse(line)!;

        // Assert
        await Assert.That((v.MaxCombos, v.Has("check"), v.Has("combos"), v.Has("defaults"), v.Names.Count)).IsEqualTo((16, true, true, true, 4));
    }

    [Test]
    public async Task Version_one_has_no_capabilities()
    {
        // Arrange
        const string line = "#V 1 layers=4 keys=42 store=flash max=40 names=default,symbols,nav,wm";

        // Act
        var v = (StudioVersion)StudioLine.Parse(line)!;

        // Assert
        await Assert.That((v.Has("check"), v.Has("combos"))).IsEqualTo((false, false));
    }

    [Test]
    [Arguments("#LD 2 trans kp:F1", "StudioLayer { Layer = 2, Default = True }")]
    [Arguments("#C 0 cut 25,26 0 50 150 kp:LC(X)", "StudioComboLine { Index = 0, Combo = cut 25,26 0 50 150 kp:LC(X), Default = False }")]
    [Arguments("#C end 6", "StudioComboEnd { Count = 6, Default = False }")]
    [Arguments("#CD end 6", "StudioComboEnd { Count = 6, Default = True }")]
    public async Task Combo_and_default_lines_are_parsed(string line, string expected)
    {
        // Arrange
        var text = line;

        // Act
        var parsed = StudioLine.Parse(text);

        // Assert
        var shown = parsed is StudioLayer l ? $"StudioLayer {{ Layer = {l.Layer}, Default = {l.Default} }}" : parsed?.ToString();
        await Assert.That(shown).IsEqualTo(expected);
    }

    [Test]
    [Arguments("#A check 2 35 err lockout", "check", 2, 35, null, false, "lockout")]
    [Arguments("#A combo add 6 ok x 1,2 * 40 0 caps", "combo add", null, null, 6, true, "x 1,2 * 40 0 caps")]
    [Arguments("#A combo add new err full", "combo add", null, null, null, false, "full")]
    [Arguments("#A combo remove 3 ok", "combo remove", null, null, 3, true, "")]
    [Arguments("#A combo check new err positions", "combo check", null, null, null, false, "positions")]
    public async Task Version_two_acks_are_parsed(string line, string command, int? layer, int? pos, int? combo, bool ok, string detail)
    {
        // Arrange
        var text = line;

        // Act
        var a = (StudioAck)StudioLine.Parse(text)!;

        // Assert
        await Assert.That((a.Command, a.Layer, a.Pos, a.Combo, a.Ok, a.Detail)).IsEqualTo((command, layer, pos, combo, ok, detail));
    }

    [Test]
    public async Task Unknown_tags_are_skipped()
    {
        // Arrange
        string[] lines = ["#XY 1 2", "#H 1 kb_b left ble"];

        // Act
        var parsed = lines.Select(StudioLine.Parse).ToArray();

        // Assert
        await Assert.That((parsed[0], parsed[1], StudioLine.IsStudio("#LD 0 kp:A"), StudioLine.IsStudio(lines[1])))
            .IsEqualTo(((StudioLine?)null, (StudioLine?)null, true, false));
    }
}

public class StudioComboModelTests
{
    [Test]
    [Arguments("copy 26,27 0 50 150 kp:LC(C)")]
    [Arguments("- 25,26 * 50 0 sys:BtClear")]
    [Arguments("x 1,2,3 0,2 40 0 caps")]
    public async Task Six_words_round_trip(string text)
    {
        // Arrange
        var model = StudioCombo.Parse(text)!;

        // Act
        var encoded = model.Encode();

        // Assert
        await Assert.That(encoded).IsEqualTo(text);
    }

    [Test]
    public async Task Positions_are_written_ascending()
    {
        // Arrange
        var model = StudioCombo.Parse("x 27,26 * 50 0 kp:A")!;

        // Act
        var encoded = model.Encode();

        // Assert
        await Assert.That(encoded).IsEqualTo("x 26,27 * 50 0 kp:A");
    }

    [Test]
    public async Task Anything_but_six_words_is_no_combo()
    {
        // Arrange
        const string text = "x 26,27 * 50 kp:A";

        // Act
        var model = StudioCombo.Parse(text);

        // Assert
        await Assert.That(model).IsNull();
    }
}

/// <summary>The client against the emulator, which speaks version 2.</summary>
public class StudioV2ClientTests
{
    private static (EmulatedKeyboard Keyboard, StudioClient Client) Connected(string? storage = null, bool unlocked = false)
    {
        var keyboard = new EmulatedKeyboard(() => 0);
        if (storage != null) keyboard.SetStudioStorage(storage);
        var client = new StudioClient(new EmulatorStudioTransport(keyboard));
        client.Get();
        client.Pump();
        client.Pump(); // the defaults asked for after #V
        if (unlocked)
        {
            keyboard.PressStudioUnlock();
            client.Pump();
        }
        return (keyboard, client);
    }

    private static StudioAck Do(StudioClient client, Action send)
    {
        send();
        client.Pump();
        return client.LastAck!;
    }

    [Test]
    public async Task Get_brings_the_combos_and_the_compiled_keymap()
    {
        // Arrange
        var (keyboard, client) = Connected();
        using var _ = keyboard;

        // Act
        var (combos, defaultCombos, defaultLayers) = (client.Combos.Count, client.DefaultCombos.Count, client.DefaultLayers.Count);

        // Assert
        await Assert.That((combos, defaultCombos, defaultLayers, client.Combos[0])).IsEqualTo((6, 6, 4, "cut 25,26 0 50 150 kp:LC(X)"));
    }

    [Test]
    public async Task Adding_a_combo_while_locked_is_refused()
    {
        // Arrange
        var (keyboard, client) = Connected();
        using var _ = keyboard;

        // Act
        var ack = Do(client, () => client.ComboAdd("x 1,2 * 40 0 caps"));

        // Assert
        await Assert.That((ack.Command, ack.Ok, ack.Detail)).IsEqualTo(("combo add", false, "locked"));
    }

    [Test]
    public async Task Adding_a_combo_appends_it()
    {
        // Arrange
        var (keyboard, client) = Connected(unlocked: true);
        using var _ = keyboard;

        // Act
        var ack = Do(client, () => client.ComboAdd("x 2,1 * 40 0 caps"));

        // Assert
        await Assert.That((ack.Combo, ack.Detail, client.Combos.Count, keyboard.ReadStudioCombos().Count))
            .IsEqualTo(((int?)6, "x 1,2 * 40 0 caps", 7, 7));
    }

    [Test]
    public async Task Setting_a_combo_replaces_it()
    {
        // Arrange
        var (keyboard, client) = Connected(unlocked: true);
        using var _ = keyboard;

        // Act
        Do(client, () => client.ComboSet(0, "cutting 25,26 0 60 150 kp:LC(X)"));

        // Assert
        await Assert.That(client.Combos[0]).IsEqualTo("cutting 25,26 0 60 150 kp:LC(X)");
    }

    [Test]
    public async Task Removing_a_combo_moves_the_later_ones_down()
    {
        // Arrange
        var (keyboard, client) = Connected(unlocked: true);
        using var _ = keyboard;

        // Act
        Do(client, () => client.ComboRemove(0));

        // Assert
        await Assert.That((client.Combos.Count, client.Combos[0].StartsWith("copy ", StringComparison.Ordinal))).IsEqualTo((5, true));
    }

    [Test]
    [Arguments("x 1 * 40 0 caps", "positions")]
    [Arguments("bad-name! 1,2 * 40 0 caps", "name")]
    [Arguments("x 1,2 * 0 0 caps", "timing")]
    [Arguments("x 1,2 * 40 0", "syntax")]
    [Arguments("x 1,2 7 40 0 caps", "range")]
    public async Task Combo_check_reports_why_it_is_refused(string combo, string why)
    {
        // Arrange: a dry run works locked
        var (keyboard, client) = Connected();
        using var _ = keyboard;

        // Act
        var ack = Do(client, () => client.ComboCheck(null, combo));

        // Assert
        await Assert.That((ack.Command, ack.Ok, ack.Detail)).IsEqualTo(("combo check", false, why));
    }

    [Test]
    public async Task Key_check_works_locked_and_sees_the_guard()
    {
        // Arrange
        var (keyboard, client) = Connected();
        using var _ = keyboard;

        // Act
        var ack = Do(client, () => client.Check(2, 35, "trans"));

        // Assert
        await Assert.That((ack.Command, ack.Layer, ack.Pos, ack.Detail)).IsEqualTo(("check", (int?)2, (int?)35, "lockout"));
    }

    [Test]
    public async Task Sixteen_combos_is_full()
    {
        // Arrange
        var (keyboard, client) = Connected(unlocked: true);
        using var _ = keyboard;
        for (var i = client.Combos.Count; i < 16; i++) Do(client, () => client.ComboAdd($"c{i} 1,{i + 2} * 40 0 caps"));

        // Act
        var ack = Do(client, () => client.ComboAdd("one_more 1,2 * 40 0 caps"));

        // Assert
        await Assert.That((ack.Ok, ack.Detail)).IsEqualTo((false, "full"));
    }

    [Test]
    public async Task Saved_combos_survive_a_restart()
    {
        // Arrange
        var storage = Path.Combine(Path.GetTempPath(), $"emk-combos-{Guid.NewGuid():N}", "keymap.txt");
        var (keyboard, client) = Connected(storage, unlocked: true);
        Do(client, () => client.ComboAdd("x 1,2 * 40 0 caps"));
        Do(client, () => client.Save());
        keyboard.Dispose();

        // Act
        using var restarted = new EmulatedKeyboard(() => 0);
        restarted.SetStudioStorage(storage);

        // Assert
        await Assert.That(restarted.ReadStudioCombos()[^1]).IsEqualTo("x 1,2 * 40 0 caps");
    }

    private sealed class FakePort : IStudioTransport
    {
        public List<string> Sent { get; } = new();
        public Queue<string> Incoming { get; } = new();

        public bool Send(string line)
        {
            Sent.Add(line);
            return true;
        }

        public IEnumerable<string> Receive()
        {
            var lines = Incoming.ToList();
            Incoming.Clear();
            return lines;
        }
    }

    [Test]
    public async Task Version_one_firmware_is_not_asked_for_defaults()
    {
        // Arrange
        var port = new FakePort();
        var client = new StudioClient(port);
        client.Get();
        port.Incoming.Enqueue("#V 1 layers=4 keys=42 store=flash max=40 names=default,symbols,nav,wm");

        // Act
        client.Pump();

        // Assert
        await Assert.That((port.Sent.Contains("#studio defaults"), client.Has("combos"))).IsEqualTo((false, false));
    }

    [Test]
    public async Task Version_two_firmware_is_asked_for_its_defaults()
    {
        // Arrange
        var port = new FakePort();
        var client = new StudioClient(port);
        client.Get();
        port.Incoming.Enqueue("#V 2 layers=4 keys=42 store=flash max=40 combos=16 caps=check,combos,defaults names=default,symbols,nav,wm");

        // Act
        client.Pump();

        // Assert
        await Assert.That(port.Sent).Contains("#studio defaults");
    }
}

/// <summary>The combo editor and the capability-aware key editor.</summary>
[NotInParallel("ui-session")]
public class StudioV2EditorTests
{
    private static HeadlessUnitTestSession Session => TestAppBuilder.Session;

    private static Task<T> Run<T>(bool unlocked, Func<StudioView, EmulatedKeyboard, T> act) => Session.Dispatch(() =>
    {
        using var keyboard = new EmulatedKeyboard(() => 0);
        keyboard.SetStudioStorage(Path.Combine(Path.GetTempPath(), $"emk-studio-{Guid.NewGuid():N}", "keymap.txt"));
        var studio = new StudioView(keyboard);
        studio.Activate();
        studio.Pump();
        studio.Pump();
        if (unlocked)
        {
            studio.UnlockButton.PerformClick();
            studio.Pump();
        }
        return act(studio, keyboard);
    }, CancellationToken.None);

    [Test]
    public async Task Combos_are_editable_on_the_emulator()
    {
        // Arrange
        var editable = await Run(false, (s, _) =>
        {
            // Act
            return s.CombosEditable;
        });

        // Assert
        await Assert.That(editable).IsTrue();
    }

    [Test]
    public async Task A_new_combo_takes_the_clicked_keys()
    {
        // Arrange
        var (count, index, last) = await Run(true, (s, k) =>
        {
            s.NewCombo();
            s.ToggleComboKey(2);
            s.ToggleComboKey(1);
            s.EditForTest(e =>
            {
                e.Kind = StudioKind.CapsWord;
            });

            // Act
            s.ApplyCombo();
            s.Pump();
            return (s.Client.Combos.Count, s.ComboIndex, s.Client.Combos[^1]);
        });

        // Assert
        await Assert.That((count, index, last)).IsEqualTo((7, (int?)6, "- 1,2 0 50 150 caps"));
    }

    [Test]
    public async Task Clicking_a_key_twice_takes_it_out_of_the_combo()
    {
        // Arrange
        var positions = await Run(false, (s, _) =>
        {
            s.NewCombo();
            s.ToggleComboKey(5);

            // Act
            s.ToggleComboKey(5);
            return s.Combo!.Positions.Count;
        });

        // Assert
        await Assert.That(positions).IsEqualTo(0);
    }

    [Test]
    public async Task Editing_a_combo_changes_it()
    {
        // Arrange
        var first = await Run(true, (s, _) =>
        {
            s.EditCombo(0);
            s.Combo!.Name = "snip";

            // Act
            s.ApplyCombo();
            s.Pump();
            return s.Client.Combos[0];
        });

        // Assert
        await Assert.That(first).IsEqualTo("snip 25,26 0 50 150 kp:LC(X)");
    }

    [Test]
    public async Task Remove_needs_a_second_click()
    {
        // Arrange
        var (afterFirst, afterSecond) = await Run(true, (s, _) =>
        {
            s.EditCombo(0);
            s.RemoveCombo();
            s.Pump();
            var first = s.Client.Combos.Count;

            // Act
            s.RemoveCombo();
            s.Pump();
            return (first, s.Client.Combos.Count);
        });

        // Assert
        await Assert.That((afterFirst, afterSecond)).IsEqualTo((6, 5));
    }

    [Test]
    public async Task Esc_disarms_remove_then_leaves_the_combo()
    {
        // Arrange
        var (disarmed, left, editing) = await Run(true, (s, _) =>
        {
            s.EditCombo(0);
            s.RemoveCombo();
            var d = s.Escape();

            // Act
            var l = s.Escape();
            return (d, l, s.Combo != null);
        });

        // Assert
        await Assert.That((disarmed, left, editing)).IsEqualTo((true, true, false));
    }

    [Test]
    public async Task Combo_check_shows_the_refusal()
    {
        // Arrange
        var message = await Run(false, (s, _) =>
        {
            s.NewCombo();
            s.ToggleComboKey(1);

            // Act
            s.CheckCombo();
            s.Pump();
            return s.Message;
        });

        // Assert
        await Assert.That(message).StartsWith("combo check err positions");
    }

    [Test]
    public async Task Key_check_uses_the_targets_own_dry_run()
    {
        // Arrange
        var message = await Run(false, (s, _) =>
        {
            s.Select(0, 13);
            s.EditForTest(e => e.Key = "B");

            // Act
            s.Check();
            s.Pump();
            return s.Message;
        });

        // Assert
        await Assert.That(message).IsEqualTo("check ok: kp:B");
    }

    [Test]
    public async Task Changes_are_marked_against_the_keyboards_own_defaults()
    {
        // Arrange: a keyboard whose compiled key 13 is B (the emulator's is A)
        var changed = await Session.Dispatch(() =>
        {
            using var keyboard = new EmulatedKeyboard(() => 0);
            var debug = new DebugView(_ => "base", 42, new HardwarePort(() => null));
            var studio = new StudioView(keyboard, debug);
            studio.SelectTarget(1);
            var layer0 = Enumerable.Range(0, 42).Select(p => p == 13 ? "kp:B" : keyboard.StudioDefault(0, p)).ToArray();
            var line = string.Join(' ', layer0);

            // Act
            debug.Feed("#V 2 layers=4 keys=42 store=flash max=40 combos=16 caps=check,combos,defaults names=default,symbols,nav,wm\r\n" +
                       $"#L 0 {line}\r\n#LD 0 {line}\r\n");
            studio.Pump();
            var c = studio.View.Changed.Count > 13 && studio.View.Changed[13];
            debug.Dispose();
            return (c, studio.DefaultsFromTarget);
        }, CancellationToken.None);

        // Assert
        await Assert.That(changed).IsEqualTo((false, true));
    }
}
