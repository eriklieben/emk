using System.Diagnostics;
using Avalonia.Headless;
using KbEmulator.Bench;

namespace KbEmulator.Tests;

/// <summary>Fake build-uf2 and ./hil scripts: never the real ones.</summary>
internal static class FakeScripts
{
    public static string Write(string body)
    {
        var dir = Path.Combine(Path.GetTempPath(), $"emk-fake-{Guid.NewGuid():N}");
        Directory.CreateDirectory(dir);
        var path = Path.Combine(dir, "script.sh");
        File.WriteAllText(path, "#!/bin/bash\n" + body + "\n");
        return path;
    }

    /// <summary>A run whose steps are these scripts.</summary>
    public static FlashJob Job(string side, string flash, string? build = null) =>
        new(Path.GetTempPath(), "kb_b", side, build != null, step =>
            new ProcessStartInfo("/bin/bash", [Write(step == FlashStep.Build ? build! : flash)]));
}

public class FlashJobTests
{
    private static (List<string> Lines, List<FlashState> States) Record(FlashJob job)
    {
        var lines = new List<string>();
        var states = new List<FlashState>();
        job.Line += l => { lock (lines) lines.Add(l); };
        job.StateChanged += (s, _) => { lock (states) states.Add(s); };
        return (lines, states);
    }

    [Test]
    public async Task Output_is_streamed_line_by_line()
    {
        // Arrange
        var job = FakeScripts.Job("right", "echo one; echo two >&2; echo three");
        var (lines, _) = Record(job);

        // Act
        await job.RunAsync();

        // Assert
        await Assert.That(lines).IsEquivalentTo(new[] { "one", "two", "three" });
    }

    [Test]
    public async Task Left_half_goes_through_restart_write_and_back()
    {
        // Arrange
        var job = FakeScripts.Job("left", """
            echo "asked the left half to restart into its bootloader"
            echo "bootloader: signed (serial 0123456789ABCDEF, port /dev/ttyACM1)"
            echo "flashed target/fw-nrf-left-ble-kb_b.zip"
            echo "keyboard back after 4s"
            """);
        var (_, states) = Record(job);

        // Act
        var final = await job.RunAsync();

        // Assert
        await Assert.That((final, job.Detail, string.Join(",", states)))
            .IsEqualTo((FlashState.Done, "keyboard back after 4s", "Restarting,Restarting,Writing,WaitingForKeyboard,Done"));
    }

    [Test]
    public async Task Right_half_asks_for_a_double_tap()
    {
        // Arrange
        var job = FakeScripts.Job("right", "echo 'waiting for the bootloader (double-tap reset on the right half)...'; sleep 0.2; exit 1");
        var details = new List<string>();
        job.StateChanged += (s, d) => { if (s == FlashState.WaitingForBootloader) lock (details) details.Add(d); };

        // Act
        await job.RunAsync();

        // Assert
        await Assert.That(details).IsEquivalentTo(new[] { "double-tap reset on the right half now" });
    }

    [Test]
    public async Task A_failing_script_fails_with_its_error_line()
    {
        // Arrange
        var job = FakeScripts.Job("right", "echo 'hil: no NICENANO drive appeared' >&2; exit 1");

        // Act
        var final = await job.RunAsync();

        // Assert
        await Assert.That((final, job.Detail)).IsEqualTo((FlashState.Failed, "hil: no NICENANO drive appeared"));
    }

    [Test]
    public async Task The_tamper_alarm_is_its_own_state()
    {
        // Arrange
        var job = FakeScripts.Job("left", "echo 'hil: ALARM: kb_b left (0123) had the signed bootloader and now shows a UF2 drive.' >&2; exit 1");

        // Act
        var final = await job.RunAsync();

        // Assert
        await Assert.That(final).IsEqualTo(FlashState.Alarm);
    }

    [Test]
    public async Task A_held_lock_means_busy()
    {
        // Arrange
        var job = FakeScripts.Job("left", "echo 'hil: another ./hil flash is running' >&2; exit 1");

        // Act
        var final = await job.RunAsync();

        // Assert
        await Assert.That(final).IsEqualTo(FlashState.Busy);
    }

    [Test]
    public async Task Cancel_stops_the_script()
    {
        // Arrange
        var job = FakeScripts.Job("right", "echo started; sleep 30; echo never");
        var started = new TaskCompletionSource();
        job.Line += l => { if (l == "started") started.TrySetResult(); };
        var run = job.RunAsync();
        await started.Task.WaitAsync(TimeSpan.FromSeconds(5));

        // Act
        job.Cancel();

        // Assert
        var final = await run.WaitAsync(TimeSpan.FromSeconds(5));
        await Assert.That(final).IsEqualTo(FlashState.Cancelled);
    }

    [Test]
    public async Task Build_runs_before_the_flash()
    {
        // Arrange
        var job = FakeScripts.Job("right", "echo flash", build: "echo build");
        var (lines, _) = Record(job);

        // Act
        await job.RunAsync();

        // Assert
        await Assert.That(lines).IsEquivalentTo(new[] { "build", "flash" });
    }

    [Test]
    public async Task A_failed_build_never_flashes()
    {
        // Arrange
        var job = FakeScripts.Job("right", "echo flash", build: "echo 'build-uf2: fw.zip failed verification' >&2; exit 1");
        var (lines, _) = Record(job);

        // Act
        var final = await job.RunAsync();

        // Assert
        await Assert.That((final, lines.Contains("flash"))).IsEqualTo((FlashState.Failed, false));
    }

    [Test]
    public async Task Commands_are_the_repo_tools_with_the_keyboard_name()
    {
        // Arrange
        var job = new FlashJob("/repo", "kb_a", "right", buildFirst: true);

        // Act
        var commands = job.CommandLines();

        // Assert
        await Assert.That(commands).IsEquivalentTo(new[] { "BLE=1 SIDE=right ./fw-nrf/build-uf2 kb_a", "KB_NAME=kb_a ./hil flash right" });
    }
}

/// <summary>Flash mode's confirm-then-run flow, with fake scripts.</summary>
[NotInParallel("ui-session")] // timing tests must not wait behind renders on the one UI thread
public class FlashFlowTests
{
    private static HeadlessUnitTestSession Session => TestAppBuilder.Session;

    private static (FakeRepo Repo, FlashView View, List<string> Calls) NewView(string flashScript)
    {
        var repo = new FakeRepo().Halves("kb_b left 0123456789ABCDEF\nkb_b right FEDCBA9876543210\n");
        var calls = new List<string>();
        var sources = new DeviceSources { Repo = repo.Root, KeyPath = Path.Combine(repo.Root, "no.pem"), UsbRoot = repo.Root, LogPort = () => null };
        var view = new FlashView(sources)
        {
            JobFactory = (r, keyboard, side, build) =>
            {
                lock (calls) calls.Add($"job {keyboard} {side} build={build}");
                return new FlashJob(r, keyboard, side, build, _ => new ProcessStartInfo("/bin/bash", [FakeScripts.Write(flashScript)]));
            },
            BeforeFlash = side =>
            {
                lock (calls) calls.Add($"release {side}");
                return side == "left";
            },
        };
        view.AfterFlash = () => { lock (calls) calls.Add("resume"); };
        view.Refresh();
        return (repo, view, calls);
    }

    private static async Task WaitUntil(Func<bool> done)
    {
        var until = DateTime.UtcNow + TimeSpan.FromSeconds(10);
        while (!await Session.Dispatch(() =>
               {
                   Avalonia.Threading.Dispatcher.UIThread.RunJobs();
                   return done();
               }, CancellationToken.None))
        {
            if (DateTime.UtcNow > until) throw new TimeoutException();
            await Task.Delay(20);
        }
    }

    [Test]
    public async Task First_click_only_asks_to_confirm()
    {
        // Arrange
        var (repo, view, calls) = await Session.Dispatch(() => NewView("echo flashed"), CancellationToken.None);
        using var _ = repo;

        // Act
        var confirm = await Session.Dispatch(() =>
        {
            view.RequestFlash("left");
            return view.ConfirmSide;
        }, CancellationToken.None);

        // Assert
        await Assert.That((confirm, calls.Count)).IsEqualTo(("left", 0));
    }

    [Test]
    public async Task Confirming_runs_hil_and_hands_the_log_port_back()
    {
        // Arrange
        var (repo, view, calls) = await Session.Dispatch(() => NewView("echo 'flashed x.zip'; echo 'keyboard back after 2s'"), CancellationToken.None);
        using var _ = repo;
        await Session.Dispatch(() => view.RequestFlash("left"), CancellationToken.None);

        // Act
        await Session.Dispatch(() => view.ConfirmFlash(), CancellationToken.None);

        // Assert
        await WaitUntil(() => view.Job is { Running: false } && calls.Contains("resume"));
        var state = await Session.Dispatch(() => view.Job!.State, CancellationToken.None);
        await Assert.That((state, string.Join(", ", calls))).IsEqualTo((FlashState.Done, "job kb_b left build=False, release left, resume"));
    }

    [Test]
    public async Task No_second_flash_while_one_runs()
    {
        // Arrange
        var (repo, view, calls) = await Session.Dispatch(() => NewView("echo started; sleep 30"), CancellationToken.None);
        using var _ = repo;
        await Session.Dispatch(() =>
        {
            view.RequestFlash("right");
            view.ConfirmFlash();
        }, CancellationToken.None);

        // Act
        var second = await Session.Dispatch(() =>
        {
            view.RequestFlash("left");
            return view.ConfirmFlash();
        }, CancellationToken.None);

        // Assert
        await Session.Dispatch(view.CancelFlash, CancellationToken.None);
        await WaitUntil(() => view.Job is { Running: false });
        await Assert.That((second, calls.Count(c => c.StartsWith("job", StringComparison.Ordinal)))).IsEqualTo(((FlashJob?)null, 1));
    }

    [Test]
    [Arguments("light")]
    [Arguments("dark")]
    public async Task Confirm_panel_and_finished_run_render(string theme)
    {
        // Arrange
        var variant = theme == "light" ? Avalonia.Styling.ThemeVariant.Light : Avalonia.Styling.ThemeVariant.Dark;
        var (repo, view, _) = await Session.Dispatch(() => NewView("""
            echo 'waiting for the bootloader (double-tap reset on the right half)...'
            echo 'bootloader: uf2 (serial FEDCBA9876543210, port /dev/ttyACM1)'
            echo 'flashed target/fw-nrf-right-ble-kb_b.uf2'
            """), CancellationToken.None);
        using var _ = repo;
        var window = await Session.Dispatch(() =>
        {
            var w = new Avalonia.Controls.Window { Width = 1440, Height = 900, Content = view, RequestedThemeVariant = variant };
            w.Show();
            view.RequestFlash("left");
            Render(w, $"flash-confirm-{theme}.png");
            view.CancelConfirm();
            view.RequestFlash("right");
            view.ConfirmFlash();
            return w;
        }, CancellationToken.None);
        await WaitUntil(() => view.Job is { Running: false });

        // Act
        var path = await Session.Dispatch(() =>
        {
            var p = Render(window, $"flash-done-{theme}.png");
            window.Close();
            return p;
        }, CancellationToken.None);

        // Assert
        await Assert.That(new FileInfo(path).Length).IsGreaterThan(10_000);
    }

    private static string Render(Avalonia.Controls.Window window, string name)
    {
        Avalonia.Threading.Dispatcher.UIThread.RunJobs();
        var path = Path.Combine(AppContext.BaseDirectory, name);
        var size = new Avalonia.PixelSize((int)window.Bounds.Width, (int)window.Bounds.Height);
        using var bitmap = new Avalonia.Media.Imaging.RenderTargetBitmap(size);
        bitmap.Render(window);
        using (var file = File.Create(path)) bitmap.Save(file, Avalonia.Media.Imaging.PngBitmapEncoderOptions.Default);
        return path;
    }

    [Test]
    public async Task Build_first_is_passed_to_the_run()
    {
        // Arrange
        var (repo, view, calls) = await Session.Dispatch(() => NewView("echo flashed"), CancellationToken.None);
        using var _ = repo;
        await Session.Dispatch(() =>
        {
            view.RequestFlash("right");
            view.BuildFirst = true;
        }, CancellationToken.None);

        // Act
        await Session.Dispatch(() => view.ConfirmFlash(), CancellationToken.None);

        // Assert
        await WaitUntil(() => view.Job is { Running: false });
        await Assert.That(calls).Contains("job kb_b right build=True");
    }
}

/// <summary>Layout breakpoints.</summary>
[NotInParallel("ui-session")]
public class LayoutTests
{
    private static HeadlessUnitTestSession Session => TestAppBuilder.Session;

    [Test]
    [Arguments(1440, 0)]
    [Arguments(1280, 0)]
    [Arguments(1100, 3)]
    [Arguments(1000, 3)]
    [Arguments(900, 3)]
    [Arguments(760, 2)]
    [Arguments(720, 2)]
    public async Task Width_picks_the_rack_columns(double width, int expected)
    {
        // Arrange
        var w = width;

        // Act
        var columns = Responsive.NarrowColumns(w);

        // Assert
        await Assert.That(columns).IsEqualTo(expected);
    }

    [Test]
    public async Task A_narrow_window_narrows_every_mode()
    {
        // Arrange
        var result = await Session.Dispatch(() =>
        {
            var window = new MainWindow(Path.Combine(Path.GetTempPath(), $"kb-layout-{Guid.NewGuid():N}.sock"), new HardwarePort(() => null)) { Width = 760 };
            window.Show();

            // Act
            window.ApplyWidth(760);
            var r = (window.NarrowColumns, window.Debug.ReadoutColumns, window.Bar.Narrow, window.Flash.NarrowColumns);
            window.Close();
            return r;
        }, CancellationToken.None);

        // Assert
        await Assert.That(result).IsEqualTo((2, 2, true, 2));
    }

    [Test]
    public async Task Widening_again_restores_the_wide_layout()
    {
        // Arrange
        var result = await Session.Dispatch(() =>
        {
            var window = new MainWindow(Path.Combine(Path.GetTempPath(), $"kb-layout-{Guid.NewGuid():N}.sock"), new HardwarePort(() => null));
            window.Show();
            window.ApplyWidth(760);

            // Act
            window.ApplyWidth(1440);
            var r = (window.NarrowColumns, window.Debug.ReadoutColumns, window.Bar.Narrow);
            window.Close();
            return r;
        }, CancellationToken.None);

        // Assert
        await Assert.That(result).IsEqualTo((0, 4, false));
    }
}
