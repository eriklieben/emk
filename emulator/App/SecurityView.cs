using Avalonia;
using Avalonia.Controls;
using KbEmulator.Bench;

namespace KbEmulator;

/// <summary>Security (read-only): the state of the defences for one keyboard.</summary>
public sealed class SecurityView : DevicePage
{
    public SecurityView(DeviceSources sources) : base(sources)
    {
    }

    /// <summary>"Open Flash" was clicked.</summary>
    public event Action? OpenFlash;

    protected override IEnumerable<(string, LampState)> Lamps(KeyboardFacts facts, SigningKeyStatus key)
    {
        var alarms = Sides.Count(side => facts.Alarm(side) != null);
        var todo = Sides.Count(side => facts.Half(side)?.IsSigned != true)
                   + (key.Present && key.OwnerOnly ? 0 : 1)
                   + (facts.Boot is { DebugPort: "locked" } b && b.Keyboard == facts.Name ? 0 : 1);
        yield return (todo == 0 ? "Nothing to do" : $"{todo} items to do", todo == 0 ? LampState.Good : LampState.Warn);
        yield return alarms > 0 ? ($"{alarms} ALARM", LampState.Crit) : ("No alarms", LampState.Off);
    }

    protected override Control Build(KeyboardFacts facts, SigningKeyStatus key)
    {
        var top = Rack("*,*,*", Bootloader(facts), SigningKey(key), DebugPort(facts));
        var bottom = Rack("*,*,1.3*", Pairing(), Halves(facts), Tamper());
        if (NarrowColumns > 0)
        {
            bottom.MinHeight = 0;
            return Scrolling(W.Column(0, top, bottom));
        }
        var topBox = new Border { BorderThickness = new Thickness(0, 0, 0, 1), Child = top }.Tok(Border.BorderBrushProperty, "ln");
        var grid = new Grid { RowDefinitions = new RowDefinitions("Auto,*") };
        Grid.SetRow(bottom, 1);
        grid.Children.Add(topBox);
        grid.Children.Add(bottom);
        return grid;
    }

    private static string Describe(HalfRecord? h) => h?.Bootloader switch
    {
        BootloaderKind.Signed => "signed · serial DFU only",
        BootloaderKind.Uf2 => "UF2 · accepts any file",
        null when h != null => "not recorded (./hil bootinfo)",
        null => "half not recorded",
        var other => other,
    };

    /// <summary>The bootloader module's title and tone.</summary>
    private static (string Title, string Tone) BootloaderState(HalfRecord? left, HalfRecord? right, bool alarm)
    {
        if (alarm) return ("ALARM", "crit");
        var (leftSigned, rightSigned) = (left?.IsSigned == true, right?.IsSigned == true);
        if (leftSigned && rightSigned) return ("Both halves signed", "good");
        if (leftSigned) return ("Left signed, right not yet", "warn");
        if (rightSigned) return ("Right signed, left not yet", "warn");
        if (left?.Bootloader == null && right?.Bootloader == null) return ("Not recorded yet", "warn");
        return ("Neither half signed", "warn");
    }

    private StackPanel Bootloader(KeyboardFacts f)
    {
        var (left, right) = (f.Half("left"), f.Half("right"));
        var alarm = f.Alarm("left") ?? f.Alarm("right");
        var (title, tone) = BootloaderState(left, right, alarm != null);
        var open = new BenchButton("Open Flash");
        open.Click += () => OpenFlash?.Invoke();
        var children = new List<Control> { W.Lbl("Bootloader"), W.Stripe(tone), W.Head(title, 20) };
        if (alarm != null) children.Add(Alarm(alarm));
        children.Add(W.Kv([("Left", W.Text(Describe(left))), ("Right", W.Text(Describe(right)))], 80));
        if (left?.IsSigned == true && right?.IsSigned != true) children.Add(W.Hint("Do the right half after a few days on the left one."));
        if (left?.Bootloader == null && right?.Bootloader == null)
        {
            children.Add(W.Hint($"{HalfRecord.FileName} has no bootloader column for {f.Name} yet. ./hil flash adds it; ./hil bootinfo left|right checks a half now."));
        }
        children.Add(open);
        return W.Column(9, children.ToArray());
    }

    private static StackPanel SigningKey(SigningKeyStatus key)
    {
        var rows = new List<(string, Control)> { ("File", W.Mono(key.Display, 12)) };
        if (key.Present)
        {
            rows.Add(("Permissions", W.Text(key.OwnerOnly ? "owner only" : "readable by others: chmod 600").Tok(TextBlock.ForegroundProperty, key.OwnerOnly ? "ink" : "crit")));
            rows.Add(("Changed", W.Text($"{key.Modified:yyyy-MM-dd HH:mm}")));
        }
        return W.Column(9,
            W.Lbl("Signing key"),
            W.Stripe(key.Present && key.OwnerOnly ? "good" : "crit"),
            W.Head(key.Present ? "Key found" : "No key", 20),
            W.Kv(rows, 110),
            W.Hint(key.Present
                ? "The app only checks that the file is there; it never reads the key. Keep an encrypted offline copy: without it, signed halves can't be updated."
                : "Packages can't be signed without it (docs/signed-bootloader.md, 2.2)."));
    }

    private static StackPanel DebugPort(KeyboardFacts f)
    {
        var boot = f.Boot is { } b && b.Keyboard == f.Name ? b : null;
        var state = boot?.DebugPort;
        var (title, tone) = state switch
        {
            "locked" => ("Locked", "good"),
            "open" => ("Not locked", "warn"),
            null => ("Unknown", "dim"),
            _ => (state, "warn"),
        };
        return W.Column(9,
            W.Lbl("Debug port"),
            W.Stripe(tone),
            W.Head(title, 20),
            W.Text(boot != null
                ? $"The {boot.Half} half's boot line says: debug port {boot.DebugPort} (seen {boot.Seen:HH:mm} in Debug)."
                : "The firmware says it in its boot line (debug port open|locked). Open Debug with the left half on USB and press Replay log (d), or restart it.", 13),
            W.Hint("Lock it with LOCK=1 only at the end, and only with a working SWD probe (docs/signed-bootloader.md, 5.1)."));
    }

    private static StackPanel Pairing() => W.Column(9,
        W.Lbl("Pairing with computers"),
        W.Stripe("good"),
        W.Kv([
            ("Method", W.Text("Code typed on the keyboard (passkey): the computer shows 6 digits; type them on the symbols layer, then Enter. Escape rejects.")),
            ("Profiles", W.Text("The app can't read the real keyboard's bonds. Debug shows the selected profile: ✓ connected, ✗ paired, + open.")),
            ("Watch for", W.Text("+ on a profile you didn't forget: an alarm (see the rules).")),
        ], 100));

    private static StackPanel Halves(KeyboardFacts f) => W.Column(9,
        W.Lbl("Halves"),
        W.Stripe(f.Halves.Count == 2 ? "good" : "warn"),
        W.Kv([
            ("Left", W.Mono(f.Half("left")?.Serial ?? "not recorded", 12)),
            ("Right", W.Mono(f.Half("right")?.Serial ?? "not recorded", 12)),
            ("Source", W.Text($"{HalfRecord.FileName}: ./hil refuses to flash a nice!nano recorded as another half.")),
        ], 80),
        W.Hint("The halves are bonded to each other: the right half accepts only its left. Halves that don't link (Link? key, split: linked missing) are an alarm."));

    private DockPanel Tamper()
    {
        var (intro, rules) = TamperRules.Load(Sources.Repo);
        var items = new List<Control>();
        if (rules.Count == 0)
        {
            items.Add(W.Text($"No tamper rules found in {TamperRules.Doc}.", 13.5).Tok(TextBlock.ForegroundProperty, "log"));
        }
        else
        {
            if (intro.Length > 0) items.Add(Runs(13.5, (intro, "log-dim", false)));
            foreach (var rule in rules) items.Add(Runs(13.5, (rule.Lead + " ", rule.Alarm ? "log-a" : "log-g", true), (rule.Rest, "log", false)));
        }
        var well = W.Well(new ScrollViewer { Content = W.Column(8, items.ToArray()) });
        if (NarrowColumns > 0) well.MinHeight = 200;
        well.Padding = new Thickness(14, 12);
        var panel = new DockPanel();
        var head = W.Column(2, W.Lbl("Tamper rules"), W.Hint($"From {TamperRules.Doc}, section 6.", 11.5));
        head.Margin = new Thickness(0, 0, 0, 9);
        SetDock(head, Dock.Top);
        panel.Children.Add(head);
        panel.Children.Add(well);
        return panel;
    }
}
