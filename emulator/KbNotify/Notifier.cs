using System.ComponentModel;
using System.Net.Sockets;
using Kb.HostProtocol;

namespace KbNotify;

/// <summary>What kbnotify needs from the computer: its output, its platform, and the ways to
/// reach the keyboard. Program.cs passes the real ones; the tests pass fakes.</summary>
/// <param name="FindKeyboards">The keyboard's Raw HID devices, in the order they are tried.</param>
/// <param name="DeviceAtPath">Linux: the hidraw node given with --device.</param>
/// <param name="OpenDevice">Open a device for writing (may throw <see cref="UnauthorizedAccessException"/>).</param>
/// <param name="EmulatorSocketExists">Whether the emulator's socket is there.</param>
/// <param name="ConnectEmulator">Connect to the emulator (throws <see cref="SocketException"/> when it isn't running).</param>
internal sealed record NotifierEnvironment(
    TextWriter Out,
    TextWriter Error,
    bool IsWindows,
    Func<IReadOnlyList<RawHidDevice>> FindKeyboards,
    Func<string, RawHidDevice> DeviceAtPath,
    Func<RawHidDevice, IKeyboardLink> OpenDevice,
    Func<bool> EmulatorSocketExists,
    Func<Task<IKeyboardLink>> ConnectEmulator);

/// <summary>kbnotify itself: parse the command line, find the keyboard (or the emulator), send.</summary>
internal sealed class Notifier(NotifierEnvironment env)
{
    /// <summary>Run with <paramref name="args"/>; the exit code (0 done, 1 failed, 2 nothing to send).</summary>
    public async Task<int> RunAsync(IEnumerable<string> args)
    {
        var options = Options.Parse(args);
        if (options.Help)
        {
            await env.Out.WriteLineAsync(Options.Usage);
            return 0;
        }

        if (options.List) return await ListAsync();

        var text = string.Join(' ', options.Words);
        if (!options.Clear && text.Length == 0)
        {
            await env.Error.WriteLineAsync("nothing to send (try --help)");
            return 2;
        }

        IKeyboardLink link;
        try
        {
            link = await OpenAsync(options);
        }
        catch (Exception e)
        {
            // Top level of a command-line tool: every reason the keyboard can't be opened is one line on stderr.
            await env.Error.WriteLineAsync($"kbnotify: {e.Message}");
            return 1;
        }

        return await SendAsync(link, options.Clear, text);
    }

    private async Task<int> ListAsync()
    {
        var found = env.FindKeyboards();
        if (found.Count == 0) await env.Error.WriteLineAsync($"kbnotify: {NothingFound(withEmulator: false)}");
        foreach (var d in found) await env.Out.WriteLineAsync($"{d.Transport,-9} report {d.ReportId}  {d.Path}");
        return found.Count == 0 ? 1 : 0;
    }

    private async Task<int> SendAsync(IKeyboardLink link, bool clear, string text)
    {
        try
        {
            await using (link)
            {
                var packets = clear ? [NotificationPackets.Clear()] : NotificationPackets.Encode(text);
                foreach (var packet in packets) await link.SendAsync(packet);
                await env.Out.WriteLineAsync(clear
                    ? $"cleared on {link.Description}"
                    : $"sent {packets.Count} packet(s) to {link.Description}");
            }
            return 0;
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or Win32Exception)
        {
            await env.Error.WriteLineAsync($"kbnotify: sending to {link.Description} failed: {e.Message}");
            return 1;
        }
    }

    private async Task<IKeyboardLink> OpenAsync(Options options)
    {
        if (options.Device is { } device)
            return Open(env.IsWindows ? WindowsDevice(device) : env.DeviceAtPath(device));
        if (options.UseEmulator || env.EmulatorSocketExists())
        {
            try
            {
                return await env.ConnectEmulator();
            }
            catch (SocketException) when (!options.UseEmulator)
            {
                // stale socket: fall through to hardware
            }
        }
        var keyboards = env.FindKeyboards();
        if (keyboards.Count == 0) throw new InvalidOperationException(NothingFound());
        return Open(keyboards[0]);
    }

    private RawHidDevice WindowsDevice(string path) =>
        env.FindKeyboards().FirstOrDefault(d => string.Equals(d.Path, path, StringComparison.OrdinalIgnoreCase))
        ?? throw new InvalidOperationException($"{path} is not the keyboard's Raw HID collection (see --list)");

    private IKeyboardLink Open(RawHidDevice target)
    {
        try
        {
            return env.OpenDevice(target);
        }
        catch (UnauthorizedAccessException e)
        {
            throw new InvalidOperationException(env.IsWindows
                ? $"no access to {target.Path}"
                : $"no access to {target.Path}: add the udev rule from the README (or run with sudo once to check)", e);
        }
    }

    private string NothingFound(bool withEmulator = true) => (withEmulator ? "no emulator running and " : "") + (env.IsWindows
        ? "no keyboard with Raw HID found (is the firmware flashed? over Bluetooth: paired again since the update?)"
        : "no keyboard with Raw HID found (is the firmware flashed? can you read /sys/class/hidraw and write /dev/hidraw*? over Bluetooth: paired again since the update?)");
}
