using Avalonia;
using Avalonia.Controls;
using Avalonia.Media;
using KbEmulator.Bench;

namespace KbEmulator;

/// <summary>
/// Flash (read-only for now): per half its serial, recorded bootloader,
/// what is on USB now, the packages that fit, and the <c>./hil</c> commands.
/// </summary>
public sealed class FlashView : DevicePage
{
    private readonly List<string> _jobLines = new();
    private FlashJob? _job;
    private string? _confirmSide;
    private LogWell? _jobWell;
    private ContentControl? _jobHost;

    public FlashView(DeviceSources sources) : base(sources)
    {
    }

    /// <summary>How a run is made (tests pass fake scripts).</summary>
    public Func<string, string, string, bool, FlashJob> JobFactory { get; set; } =
        (repo, keyboard, side, build) => new FlashJob(repo, keyboard, side, build);

    /// <summary>Before ./hil runs: let go of the left half's log port (true if it was open).</summary>
    public Func<string, bool> BeforeFlash { get; set; } = _ => false;

    /// <summary>After the run, when <see cref="BeforeFlash"/> let go of the port: take it again.</summary>
    public Action AfterFlash { get; set; } = () => { };

    /// <summary>Raised on the UI thread when a run starts or ends (the top bar, the mode switch).</summary>
    public event Action? JobChanged;

    internal FlashJob? Job => _job;

    internal string? ConfirmSide => _confirmSide;

    internal bool BuildFirst { get; set; }

    internal IReadOnlyList<string> JobLines => _jobLines;

    /// <summary>First click on "Flash left/right half": ask to confirm.</summary>
    internal void RequestFlash(string side)
    {
        if (_job is { Running: true }) return;
        _confirmSide = side;
        Refresh();
    }

    /// <summary>Esc: close the confirm panel.</summary>
    public bool Escape()
    {
        if (_confirmSide == null) return false;
        CancelConfirm();
        return true;
    }

    internal void CancelConfirm()
    {
        _confirmSide = null;
        Refresh();
    }

    /// <summary>Confirmed: run ./hil (after the build, if asked). Returns the run, or null when one is running.</summary>
    internal FlashJob? ConfirmFlash()
    {
        if (_confirmSide is not { } side || Selected is not { } keyboard || Sources.Repo is not { } repo) return null;
        if (_job is { Running: true }) return null;
        _confirmSide = null;
        _jobLines.Clear();
        var job = JobFactory(repo, keyboard, side, BuildFirst);
        _job = job;
        var released = BeforeFlash(side);
        if (released) _jobLines.Add("(the app let go of the log port for ./hil; it reconnects afterwards)");
        foreach (var command in job.CommandLines()) _jobLines.Add("$ " + command);
        job.Line += line => Post(() =>
        {
            _jobLines.Add(line);
            _jobWell?.Add(line);
        });
        job.StateChanged += (_, _) => Post(UpdateJobPanel);
        _ = Run(job, released);
        Refresh();
        JobChanged?.Invoke();
        return job;
    }

    private async Task Run(FlashJob job, bool released)
    {
        await Task.Run(job.RunAsync).ConfigureAwait(false);
        Post(() =>
        {
            if (released) AfterFlash();
            Refresh();
            JobChanged?.Invoke();
        });
    }

    internal void CancelFlash() => _job?.Cancel();

    /// <summary>Close a finished run's panel.</summary>
    internal void Dismiss()
    {
        if (_job is { Running: true }) return;
        _job = null;
        Refresh();
    }

    /// <summary>Run on the UI thread, in order (job events come from worker threads).</summary>
    private static void Post(Action action) => Avalonia.Threading.Dispatcher.UIThread.Post(action);

    private void UpdateJobPanel()
    {
        if (_jobHost != null && _job != null) _jobHost.Content = JobPanel(_job);
    }

    /// <summary>The flash button, the confirm panel, or the run's progress, for one half.</summary>
    private Control FlashArea(KeyboardFacts f, string side)
    {
        if (_job is { } job && job.Keyboard == f.Name && job.Side == side)
        {
            _jobHost = new ContentControl { Content = JobPanel(job) };
            return _jobHost;
        }
        if (_confirmSide == side) return Confirm(f, side);
        var busy = _job is { Running: true };
        var flash = new BenchButton($"Flash {side} half", ButtonKind.Primary) { Enabled = !busy };
        flash.Click += () => RequestFlash(side);
        var note = W.Hint(busy ? $"Another flash is running ({_job!.Keyboard} {_job.Side})." : "Runs ./hil flash for you; you confirm first.");
        return W.Row(10, flash, note);
    }

    private Border Confirm(KeyboardFacts f, string side)
    {
        var record = f.Half(side);
        var file = record?.Bootloader switch
        {
            BootloaderKind.Signed => $"target/fw-nrf-{side}-ble-{f.Name}.zip (signed bootloader, serial DFU)",
            BootloaderKind.Uf2 => $"target/fw-nrf-{side}-ble-{f.Name}.uf2 (UF2 bootloader)",
            _ => $"target/fw-nrf-{side}-ble-{f.Name}.zip or .uf2: ./hil picks by the bootloader it finds",
        };
        var build = new Toggle($"Build first ({FlashJob.BuildCommand(f.Name, side)})", BuildFirst);
        build.CheckedChanged += on => BuildFirst = on;
        var go = new BenchButton("Flash now", ButtonKind.Primary);
        go.Click += () => ConfirmFlash();
        var cancel = new BenchButton("Cancel");
        cancel.Click += CancelConfirm;
        var how = side == "left"
            ? "The left half restarts into its bootloader by itself (log port)."
            : "When asked, double-tap the reset button on the right half.";
        return new Border
        {
            BorderThickness = new Thickness(1.5),
            CornerRadius = new CornerRadius(3),
            Padding = new Thickness(12, 10),
            Child = W.Column(8,
                W.Head($"Flash {f.Name} {side}?", 17),
                W.Kv([
                    ("Half", W.Mono($"{f.Name} {side} · {record?.Serial ?? "serial not recorded"}", 12)),
                    ("File", W.Mono(file, 12)),
                ], 70),
                W.Text(how, 13),
                build,
                W.Row(8, go, cancel)),
        }.Tok(Border.BorderBrushProperty, "acc").Tok(Border.BackgroundProperty, "acc-soft");
    }

    private StackPanel JobPanel(FlashJob job)
    {
        var (text, tone) = job.State switch
        {
            FlashState.Building => ("Building", "warn"),
            FlashState.Restarting => ("Restarting", "warn"),
            FlashState.WaitingForBootloader => ("Waiting for the bootloader", "warn"),
            FlashState.Writing => ("Writing", "warn"),
            FlashState.WaitingForKeyboard => ("Waiting for the keyboard", "warn"),
            FlashState.Done => ("Done", "good"),
            FlashState.Alarm => ("ALARM", "crit"),
            FlashState.Busy => ("Another flash is running", "crit"),
            FlashState.Cancelled => ("Cancelled", "crit"),
            FlashState.Failed => ("Failed", "crit"),
            _ => ("Starting", "warn"),
        };
        _jobWell = new LogWell(2000) { MinHeight = 160, Height = 200 };
        foreach (var line in _jobLines) _jobWell.Add(line);
        var detail = W.Text(job.Detail, 13.5);
        if (job.State == FlashState.WaitingForBootloader) detail.FontWeight = FontWeight.SemiBold;
        var buttons = W.Row(8);
        if (job.Running)
        {
            var cancel = new BenchButton("Cancel", ButtonKind.Danger);
            cancel.Click += CancelFlash;
            buttons.Children.Add(cancel);
        }
        else
        {
            var close = new BenchButton("Close");
            close.Click += Dismiss;
            buttons.Children.Add(close);
        }
        var children = new List<Control> { W.Split(W.Lbl($"./hil flash {job.Side} · {job.Keyboard}"), new Pill(text, tone)), detail };
        if (job.State == FlashState.Alarm) children.Add(Alarm(job.Detail));
        children.Add(_jobWell);
        children.Add(buttons);
        return W.Column(8, children.ToArray());
    }

    protected override void AddStripLamps(Panel strip, KeyboardFacts facts, SigningKeyStatus key)
    {
        foreach (var b in facts.OnUsb)
        {
            var owner = facts.Owner(b.Serial);
            var whose = owner != null ? $" ({owner})" : $" (not in {HalfRecord.FileName})";
            strip.Children.Add(new Lamp($"nice!nano in its {(b.Kind == BootloaderKind.Uf2 ? "UF2" : "signed")} bootloader on USB: {b.Serial}{whose}",
                owner == null ? LampState.Warn : LampState.Accent));
        }
        var zips = facts.Packages.Where(p => p.Kind == "zip").ToList();
        if (zips.Count == 0) return;
        var ok = zips.All(p => p.Verify is { Ok: true });
        var which = zips.Count == 2 ? "both packages" : $"{zips.Count} package(s)";
        strip.Children.Add(new Lamp(ok ? $"dfu-verify passes for {which}" : "dfu-verify: a package fails or wasn't checked",
            ok ? LampState.Good : LampState.Crit));
    }

    protected override IEnumerable<(string, LampState)> Lamps(KeyboardFacts facts, SigningKeyStatus key)
    {
        if (_job is { Running: true } job) yield return ($"Flashing {job.Keyboard} {job.Side}", LampState.Warn);
        foreach (var side in Sides) yield return HalfLamp(facts, side);
    }

    private static (string, LampState) HalfLamp(KeyboardFacts facts, string side)
    {
        if (facts.Alarm(side) != null) return ($"{Side(side)} ALARM", LampState.Crit);
        return facts.Half(side)?.Bootloader switch
        {
            BootloaderKind.Signed => ($"{Side(side)} signed", LampState.Good),
            BootloaderKind.Uf2 => ($"{Side(side)} UF2", LampState.Warn),
            _ => ($"{Side(side)} not recorded", LampState.Off),
        };
    }

    protected override Control Build(KeyboardFacts facts, SigningKeyStatus key) =>
        Scrolling(Rack("*,*,340", NarrowColumns >= 3 ? 2 : 1, HalfModule(facts, "left"), HalfModule(facts, "right"), Versions(facts)));

    private Control HalfModule(KeyboardFacts f, string side)
    {
        var record = f.Half(side);
        var live = f.LiveBootloader(side);
        var alarm = f.Alarm(side);
        var (pillText, tone) = alarm != null
            ? ("ALARM", "crit")
            : record?.Bootloader switch
            {
                BootloaderKind.Signed => ("Signed bootloader", "good"),
                BootloaderKind.Uf2 => ("UF2 bootloader", "warn"),
                null when record != null => ("Bootloader not recorded", "warn"),
                null => ("Half not recorded", "warn"),
                var other => ($"{other} bootloader", "warn"),
            };
        var head = W.Split(W.Head(side.ToUpperInvariant(), 28, FontWeight.Bold), new Pill(pillText, tone));

        var rows = new List<(string, Control)>
        {
            ("Serial", W.Mono(record?.Serial ?? $"unknown: ./hil flash {side} records it", 12)),
            ("Bootloader", W.Text(record?.Bootloader switch
            {
                BootloaderKind.Signed => "signed · serial DFU, no drive (recorded by ./hil)",
                BootloaderKind.Uf2 => "UF2 · NICENANO drive, accepts any file (recorded by ./hil)",
                null when record != null => $"not recorded yet: ./hil bootinfo {side} checks, ./hil flash {side} records it",
                null => "—",
                var other => other,
            })),
            ("On USB now", W.Text(live switch
            {
                { Kind: BootloaderKind.Uf2 } => "in its bootloader: UF2 drive",
                { } => "in its bootloader: signed, serial DFU only",
                _ => "not in its bootloader",
            })),
            ("Port", W.Text(side == "left"
                ? Sources.LogPort() switch
                {
                    { } port when f.IsOwnPort(port) => $"{port} (log port)",
                    { } port => $"not on USB; {Path.GetFileName(port)} is another keyboard's log port",
                    _ => "log port not found (left half not on USB)",
                }
                : "none. The right half has no log port.")),
        };
        if (f.Boot is { } boot && boot.Keyboard == f.Name && boot.Half == side)
        {
            rows.Add(("Boot line", W.Text($"{boot.Version}, entry {boot.Entry}, debug port {boot.DebugPort} (seen {boot.Seen:HH:mm}; a hint only)")));
        }
        var packages = f.PackagesFor(side).ToList();
        rows.Add(("Package", packages.Count == 0
            ? W.Text($"none in target/: BLE=1 SIDE={side} ./fw-nrf/build-uf2 {f.Name}")
            : W.Column(4, packages.Select(Package).ToArray())));

        var steps = record?.Bootloader switch
        {
            BootloaderKind.Signed => new[] { "Restart into the bootloader (log port on the left, else double-tap reset)", "Check the package with dfu-verify", "Send the .zip over serial DFU, then wait for split: linked" },
            BootloaderKind.Uf2 => ["Restart into the bootloader (log port on the left, else double-tap reset)", "The NICENANO drive appears", "./hil copies the .uf2 onto it"],
            _ => [$"./hil bootinfo {side} tells which bootloader this half has", $"./hil flash {side} then picks the .zip (signed) or .uf2 and records it"],
        };
        var stepList = W.Column(3, steps.Select((s, i) => (Control)W.Text($"{i + 1}. {s}", 13)).ToArray());

        var children = new List<Control> { head, W.Stripe(tone) };
        if (alarm != null) children.Add(Alarm(alarm));
        children.Add(W.Kv(rows));
        children.Add(stepList);
        children.Add(W.Lbl("Command to run (in the repo)"));
        children.Add(CommandRow(f.Command("flash", side)));
        children.Add(CommandRow(f.Command("bootinfo", side)));
        children.Add(FlashArea(f, side));
        var column = W.Column(12, children.ToArray());
        return NarrowColumns > 0 ? column : new ScrollViewer { Content = column };
    }

    private static Control Package(FirmwarePackage p)
    {
        var line = W.Mono($"{p.FileName} · {KeyboardFacts.Size(p.Size)} · {p.Modified:yyyy-MM-dd HH:mm}", 12);
        if (p.Kind != "zip") return line;
        var verify = p.Verify is { } v
            ? W.Text(v.Summary, 12).Tok(TextBlock.ForegroundProperty, VerifyTone(v))
            : W.Text("not verified (no .verify.txt)", 12).Tok(TextBlock.ForegroundProperty, "warn");
        return W.Column(1, line, verify);
    }

    private static string VerifyTone(VerifyResult v) => v.Ok ? "good" : "crit";

    /// <summary>Under a half's bootloader version: what the boot line says, else whether it is recorded.</summary>
    private static string VersionSource(BootFacts? boot, HalfRecord? record)
    {
        if (boot != null) return boot.SignedEntry ? "signed" : "UF2";
        return record?.Bootloader != null ? "recorded" : "not recorded";
    }

    private static StackPanel Versions(KeyboardFacts f)
    {
        Grid Big(string side)
        {
            var record = f.Half(side);
            var boot = f.Boot is { } b && b.Keyboard == f.Name && b.Half == side ? b : null;
            var value = boot?.Version ?? (record?.Bootloader is { } k ? k.ToUpperInvariant() : "?");
            var sub = VersionSource(boot, record);
            var big = W.Head(value, 34);
            if (record?.Bootloader != BootloaderKind.Signed) big.Tok(TextBlock.ForegroundProperty, "mut");
            var text = W.Row(6, big, W.Hint(sub, 14));
            return W.Split(text, W.Lbl(side));
        }

        var latest = f.Packages.OrderByDescending(p => p.Modified).FirstOrDefault();
        return W.Column(10,
            W.Lbl("Bootloader versions"),
            Big("left"),
            Big("right"),
            W.Hint("Versions come from the firmware's boot line, seen in Debug (left half only). It is read from TIMER2.CC[0], which bad firmware could fake: the bond is the real proof."),
            W.Lbl("Where the alarm comes from"),
            W.Hint($"./hil stops with ALARM when a half recorded as signed in {HalfRecord.FileName} shows a UF2 drive. The app shows it here too when such a half is in its bootloader on USB now (Refresh), or its boot line says UF2."),
            W.Lbl("Builds"),
            W.Hint(latest != null ? $"Newest package: {latest.FileName}, {latest.Modified:yyyy-MM-dd HH:mm}." : "No packages yet."),
            CommandRow($"BLE=1 ./fw-nrf/build-uf2 {f.Name}"),
            W.Hint("Flash runs ./hil in the repo, the same as the commands above: it checks the half's serial, the bootloader kind and the tamper alarm."));
    }
}
