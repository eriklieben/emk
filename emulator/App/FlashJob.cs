using System.Diagnostics;
using System.Text.RegularExpressions;

namespace KbEmulator;

/// <summary>Where a flash run is (from <c>./hil</c>'s and <c>build-uf2</c>'s output and exit codes).</summary>
public enum FlashState
{
    Idle,
    Building,
    /// <summary>Asked the left half (log port) to restart into its bootloader.</summary>
    Restarting,
    /// <summary>Waiting for the bootloader: double-tap reset on the half.</summary>
    WaitingForBootloader,
    /// <summary>The bootloader is there; the file is being written.</summary>
    Writing,
    /// <summary>Flashed; waiting for the left half to come back on USB.</summary>
    WaitingForKeyboard,
    Done,
    Failed,
    /// <summary>./hil stopped with the tamper alarm.</summary>
    Alarm,
    /// <summary>Another ./hil flash holds the lock.</summary>
    Busy,
    Cancelled,
}

public enum FlashStep
{
    Build,
    Flash,
}

/// <summary>
/// One run of the repo's own tools: optionally <c>BLE=1 [SIDE=right]
/// ./fw-nrf/build-uf2 &lt;name&gt;</c>, then <c>KB_NAME=&lt;name&gt; ./hil flash
/// &lt;side&gt;</c>. Nothing here talks to the keyboard itself: <c>./hil</c>
/// does the half and serial checks, the lock, UF2 or signed, and the restart.
/// Output lines and states are raised on a background thread.
/// </summary>
public sealed partial class FlashJob
{
    private readonly Func<FlashStep, ProcessStartInfo> _start;
    private readonly Lock _lock = new();
    private readonly Lock _stateLock = new();
    private Process? _process;
    private volatile bool _cancelled;

    /// <param name="start">How a step is started; default runs the repo's scripts (tests pass fake ones).</param>
    public FlashJob(string repo, string keyboard, string side, bool buildFirst, Func<FlashStep, ProcessStartInfo>? start = null)
    {
        Repo = repo;
        Keyboard = keyboard;
        Side = side;
        BuildFirst = buildFirst;
        _start = start ?? DefaultStart;
    }

    public string Repo { get; }
    public string Keyboard { get; }
    public string Side { get; }
    public bool BuildFirst { get; }

    public FlashState State { get; private set; } = FlashState.Idle;

    /// <summary>What the state means for the user ("double-tap reset on the right half", the error line…).</summary>
    public string Detail { get; private set; } = "";

    /// <summary>Started (or about to) and not finished: no other flash may start.</summary>
    public bool Running => !Finished;

    /// <summary>The run has ended (done, failed, cancelled…).</summary>
    public bool Finished { get; private set; }

    /// <summary>A line of output, stdout and stderr as they come.</summary>
    public event Action<string>? Line;

    public event Action<FlashState, string>? StateChanged;

    /// <summary>The command lines this run executes, for the confirm panel.</summary>
    public IReadOnlyList<string> CommandLines() =>
        (BuildFirst ? new[] { BuildCommand(Keyboard, Side) } : []).Append(FlashCommand(Keyboard, Side)).ToList();

    public static string BuildCommand(string keyboard, string side) =>
        $"BLE=1 {(side == "right" ? "SIDE=right " : "")}./fw-nrf/build-uf2 {keyboard}";

    public static string FlashCommand(string keyboard, string side) => $"KB_NAME={keyboard} ./hil flash {side}";

    private ProcessStartInfo DefaultStart(FlashStep step)
    {
        var psi = step == FlashStep.Build
            ? new ProcessStartInfo(Path.Combine(Repo, "fw-nrf", "build-uf2"), [Keyboard])
            : new ProcessStartInfo(Path.Combine(Repo, "hil"), ["flash", Side]);
        psi.WorkingDirectory = Repo;
        if (step == FlashStep.Build)
        {
            psi.Environment["BLE"] = "1";
            psi.Environment["SIDE"] = Side;
        }
        psi.Environment["KB_NAME"] = Keyboard;
        return psi;
    }

    /// <summary>Run to the end; the final state is returned (and raised).</summary>
    public async Task<FlashState> RunAsync()
    {
        try
        {
            return await RunSteps();
        }
        finally
        {
            Finished = true;
        }
    }

    private async Task<FlashState> RunSteps()
    {
        if (BuildFirst)
        {
            Set(FlashState.Building, $"building {Keyboard} {Side}");
            var (code, last) = await Step(FlashStep.Build);
            if (_cancelled) return Set(FlashState.Cancelled, "cancelled");
            if (code != 0) return Set(FlashState.Failed, last ?? $"build failed (exit code {code})");
        }
        Set(FlashState.Restarting, Side == "left" ? "restarting the left half into its bootloader" : $"starting ./hil flash {Side}");
        var (exit, error) = await Step(FlashStep.Flash);
        if (_cancelled) return Set(FlashState.Cancelled, "cancelled; the half may still be in its bootloader (press reset once)");
        if (State is FlashState.Alarm or FlashState.Busy) return State;
        if (exit == 0) return State == FlashState.Done ? State : Set(FlashState.Done, "flashed");
        return Set(FlashState.Failed, error ?? $"./hil exited with code {exit}");
    }

    /// <summary>Stop the running script (and what it started).</summary>
    public void Cancel()
    {
        lock (_lock)
        {
            _cancelled = true;
            try
            {
                if (_process is { HasExited: false } p) p.Kill(entireProcessTree: true);
            }
            catch (InvalidOperationException)
            {
                // It exited between the check and the kill: nothing left to stop.
            }
        }
    }

    private async Task<(int Code, string? LastError)> Step(FlashStep step)
    {
        var psi = _start(step);
        psi.RedirectStandardOutput = true;
        psi.RedirectStandardError = true;
        psi.UseShellExecute = false;
        string? lastError = null;
        using var process = new Process { StartInfo = psi, EnableRaisingEvents = true };
        var exited = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        process.Exited += (_, _) => exited.TrySetResult();
        void OnLine(string? line, bool error)
        {
            if (line == null) return;
            if (error && line.Trim().Length > 0) lastError = line.Trim();
            Line?.Invoke(line);
            if (step == FlashStep.Flash) Interpret(line);
        }
        process.OutputDataReceived += (_, e) => OnLine(e.Data, false);
        process.ErrorDataReceived += (_, e) => OnLine(e.Data, true);
        lock (_lock)
        {
            if (_cancelled) return (-1, null);
            try
            {
                process.Start();
            }
            catch (Exception e) when (e is System.ComponentModel.Win32Exception or InvalidOperationException)
            {
                Line?.Invoke($"cannot start {psi.FileName}: {e.Message}");
                return (-1, $"cannot start {psi.FileName}: {e.Message}");
            }
            _process = process;
        }
        process.BeginOutputReadLine();
        process.BeginErrorReadLine();
        await exited.Task.ConfigureAwait(false);
        // The last output lines: wait for the pipes to close, but not for a
        // child that outlived the script (a cancel, a background job).
        if (!_cancelled)
        {
            using var drain = new CancellationTokenSource(TimeSpan.FromSeconds(2));
            try
            {
                await process.WaitForExitAsync(drain.Token).ConfigureAwait(false);
            }
            catch (OperationCanceledException)
            {
                // A child kept the pipes open: report what was read so far.
            }
        }
        lock (_lock) _process = null;
        return (process.ExitCode, lastError);
    }

    [GeneratedRegex(@"double-tap reset on the (\w+) half")]
    private static partial Regex DoubleTap();

    [GeneratedRegex(@"^bootloader: (\w+) \(serial ([^,]*), port ([^)]*)\)")]
    private static partial Regex BootloaderFound();

    [GeneratedRegex(@"keyboard back after (\d+)s")]
    private static partial Regex Back();

    /// <summary>Follow ./hil flash through its messages.</summary>
    private void Interpret(string line)
    {
        var text = line.StartsWith("hil: ", StringComparison.Ordinal) ? line[5..] : line;
        if (text.Contains("ALARM", StringComparison.Ordinal))
        {
            Set(FlashState.Alarm, text);
        }
        else if (text.Contains("another ./hil flash is running", StringComparison.Ordinal))
        {
            Set(FlashState.Busy, "another ./hil flash is running; wait for it");
        }
        else if (text.StartsWith("asked the left half to restart", StringComparison.Ordinal))
        {
            Set(FlashState.Restarting, "the left half restarts into its bootloader…");
        }
        else if (DoubleTap().Match(text) is { Success: true } tap)
        {
            Set(FlashState.WaitingForBootloader, $"double-tap reset on the {tap.Groups[1].Value} half now");
        }
        else if (BootloaderFound().Match(text) is { Success: true } found)
        {
            Set(FlashState.Writing, $"writing ({found.Groups[1].Value} bootloader, serial {found.Groups[2].Value})");
        }
        else if (text.StartsWith("flashed ", StringComparison.Ordinal))
        {
            if (Side == "left") Set(FlashState.WaitingForKeyboard, "flashed; waiting for the keyboard to come back on USB");
            else Set(FlashState.Done, text);
        }
        else if (Back().Match(text) is { Success: true } back)
        {
            Set(FlashState.Done, $"keyboard back after {back.Groups[1].Value}s");
        }
    }

    private FlashState Set(FlashState state, string detail)
    {
        lock (_stateLock)
        {
            State = state;
            Detail = detail;
            StateChanged?.Invoke(state, detail);
            return state;
        }
    }
}
