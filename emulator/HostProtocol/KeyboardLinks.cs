using System.Net.Sockets;

namespace Kb.HostProtocol;

/// <summary>Where Raw HID packets go: a real keyboard or the emulator.</summary>
public interface IKeyboardLink : IAsyncDisposable
{
    string Description { get; }

    ValueTask SendAsync(ReadOnlyMemory<byte> report, CancellationToken cancellationToken = default);
}

/// <summary>
/// The Corne emulator, over a Unix domain socket. Each packet is sent as-is
/// (32 bytes).
/// </summary>
public sealed class EmulatorLink : IKeyboardLink
{
    private readonly Socket _socket;

    private EmulatorLink(Socket socket, string path)
    {
        _socket = socket;
        Description = $"emulator ({path})";
    }

    public static string DefaultSocketPath =>
        Path.Combine(Environment.GetEnvironmentVariable("XDG_RUNTIME_DIR") is { Length: > 0 } dir ? dir : Path.GetTempPath(),
            "kb-emulator.sock");

    public string Description { get; }

    public static async Task<EmulatorLink> ConnectAsync(string? path = null, CancellationToken cancellationToken = default)
    {
        path ??= DefaultSocketPath;
        var socket = new Socket(AddressFamily.Unix, SocketType.Stream, ProtocolType.Unspecified);
        try
        {
            await socket.ConnectAsync(new UnixDomainSocketEndPoint(path), cancellationToken);
            return new EmulatorLink(socket, path);
        }
        catch
        {
            socket.Dispose();
            throw;
        }
    }

    public async ValueTask SendAsync(ReadOnlyMemory<byte> report, CancellationToken cancellationToken = default)
    {
        if (report.Length != NotificationPackets.ReportSize)
            throw new ArgumentException($"report must be {NotificationPackets.ReportSize} bytes", nameof(report));
        await _socket.SendAsync(report, SocketFlags.None, cancellationToken);
    }

    public ValueTask DisposeAsync()
    {
        _socket.Dispose();
        return ValueTask.CompletedTask;
    }
}

/// <summary>How a keyboard's Raw HID collection is connected.</summary>
public enum HidTransport
{
    Usb,
    Bluetooth,
    Unknown,
}

/// <summary>A keyboard's Raw HID collection found on this computer.</summary>
/// <param name="Path">hidraw node on Linux, device interface path on Windows.</param>
/// <param name="ReportId">First byte of every write: 0 on USB, 3 over Bluetooth.</param>
/// <param name="OutputBytes">Bytes of the output report, without the report ID (32).</param>
public sealed record RawHidDevice(string Path, HidTransport Transport, byte ReportId, int OutputBytes)
{
    public string Description => Transport switch
    {
        HidTransport.Usb => $"keyboard over USB ({Path})",
        HidTransport.Bluetooth => $"keyboard over Bluetooth ({Path})",
        _ => $"keyboard ({Path})",
    };

    /// <summary>The bytes to write for <paramref name="report"/>: the report ID, then the report.</summary>
    public byte[] Frame(ReadOnlySpan<byte> report)
    {
        var buffer = new byte[1 + Math.Max(OutputBytes, report.Length)];
        buffer[0] = ReportId;
        report.CopyTo(buffer.AsSpan(1));
        return buffer;
    }
}

/// <summary>Picking the keyboard among the Raw HID devices found.</summary>
public static class RawHidDevices
{
    /// <summary>The keyboard's USB IDs (pid.codes test PID), on USB and in its Bluetooth PnP ID.</summary>
    public const ushort VendorId = 0x1209;
    public const ushort ProductId = 0x0001;

    /// <summary>USB first (the cable is the faster, surer way), then Bluetooth, then the rest.</summary>
    public static IReadOnlyList<RawHidDevice> Preferred(IEnumerable<RawHidDevice> devices) =>
        devices.OrderBy(d => d.Transport).ThenBy(d => d.Path, StringComparer.Ordinal).ToArray();
}

/// <summary>
/// A keyboard's Raw HID collection through Linux hidraw. hidraw expects the report number
/// first: 0 for the USB interface (no numbered reports), 3 for the collection in the
/// Bluetooth report map.
/// </summary>
public sealed class HidrawLink : IKeyboardLink
{
    private readonly FileStream _device;
    private readonly RawHidDevice _target;

    public HidrawLink(RawHidDevice target)
    {
        _target = target;
        _device = new FileStream(target.Path, FileMode.Open, FileAccess.Write, FileShare.ReadWrite);
        Description = target.Description;
    }

    public string Description { get; }

    public async ValueTask SendAsync(ReadOnlyMemory<byte> report, CancellationToken cancellationToken = default)
    {
        await _device.WriteAsync(_target.Frame(report.Span), cancellationToken);
        await _device.FlushAsync(cancellationToken);
    }

    public ValueTask DisposeAsync() => _device.DisposeAsync();

    /// <summary>
    /// hidraw devices whose report descriptor has the Raw HID collection, and that are this
    /// keyboard (USB IDs 1209:0001) or don't say. USB first.
    /// </summary>
    public static IReadOnlyList<RawHidDevice> Find(string sysClassHidraw = "/sys/class/hidraw")
    {
        if (!Directory.Exists(sysClassHidraw)) return [];
        var found = new List<RawHidDevice>();
        foreach (var dir in Directory.EnumerateDirectories(sysClassHidraw))
        {
            if (Describe(dir) is { } device) found.Add(device);
        }
        return RawHidDevices.Preferred(found);
    }

    /// <summary>
    /// The hidraw node <paramref name="devicePath"/> (<c>/dev/hidrawN</c>) as found under
    /// <paramref name="sysClassHidraw"/>; without its descriptor, a USB-style device (report ID 0).
    /// </summary>
    public static RawHidDevice ForPath(string devicePath, string sysClassHidraw = "/sys/class/hidraw") =>
        Describe(Path.Combine(sysClassHidraw, Path.GetFileName(devicePath)), devicePath)
        ?? new RawHidDevice(devicePath, HidTransport.Unknown, 0, NotificationPackets.ReportSize);

    private static RawHidDevice? Describe(string sysDir, string? devicePath = null)
    {
        byte[] descriptor;
        try
        {
            descriptor = File.ReadAllBytes(Path.Combine(sysDir, "device", "report_descriptor"));
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            return null;
        }
        if (HidDescriptor.FindRawHid(descriptor) is not { } raw) return null;
        var (bus, vendor, product) = ReadHidId(Path.Combine(sysDir, "device", "uevent"));
        if (vendor is { } v && product is { } p && (v != RawHidDevices.VendorId || p != RawHidDevices.ProductId))
            return null; // another keyboard with Raw HID (QMK, ZMK): not ours to write to
        var transport = bus switch
        {
            0x03 => HidTransport.Usb,
            0x05 => HidTransport.Bluetooth,
            _ => HidTransport.Unknown,
        };
        return new RawHidDevice(devicePath ?? Path.Combine("/dev", Path.GetFileName(sysDir)), transport,
            raw.ReportId, raw.OutputBytes);
    }

    /// <summary><c>HID_ID=0003:00001209:00000001</c> in the device's uevent: bus, vendor, product.</summary>
    private static (int? Bus, ushort? Vendor, ushort? Product) ReadHidId(string uevent)
    {
        try
        {
            foreach (var line in File.ReadLines(uevent))
            {
                if (!line.StartsWith("HID_ID=", StringComparison.Ordinal)) continue;
                var parts = line["HID_ID=".Length..].Split(':');
                if (parts.Length == 3
                    && int.TryParse(parts[0], System.Globalization.NumberStyles.HexNumber, System.Globalization.CultureInfo.InvariantCulture, out var bus)
                    && uint.TryParse(parts[1], System.Globalization.NumberStyles.HexNumber, System.Globalization.CultureInfo.InvariantCulture, out var vendor)
                    && uint.TryParse(parts[2], System.Globalization.NumberStyles.HexNumber, System.Globalization.CultureInfo.InvariantCulture, out var product))
                    return (bus, (ushort)vendor, (ushort)product);
            }
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            // No readable uevent: the IDs stay unknown, the descriptor alone decides.
        }
        return (null, null, null);
    }

    /// <summary>True when the descriptor has the Raw HID collection (usage page 0xFF60, usage 0x61).</summary>
    public static bool DeclaresRawHid(ReadOnlySpan<byte> descriptor) => HidDescriptor.FindRawHid(descriptor) != null;
}
