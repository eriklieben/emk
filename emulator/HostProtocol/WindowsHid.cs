namespace Kb.HostProtocol;

/// <summary>What Windows says about one HID collection (HidD_GetAttributes, HidP_GetCaps).</summary>
/// <param name="OutputReportByteLength">Including the report ID byte (33 for Raw HID).</param>
/// <param name="OutputReportId">Report ID of the collection's output report (HidP_GetValueCaps).</param>
public sealed record HidCollectionInfo(
    ushort VendorId,
    ushort ProductId,
    ushort UsagePage,
    ushort Usage,
    int OutputReportByteLength,
    byte OutputReportId);

/// <summary>An open HID collection to write output reports to.</summary>
public interface IHidWriter : IDisposable
{
    /// <summary>Write one output report, report ID first, <c>OutputReportByteLength</c> bytes.</summary>
    void Write(byte[] report);
}

/// <summary>The part of the Windows HID API kbnotify uses (SetupAPI + hid.dll), so finding the
/// keyboard can be tested without Windows.</summary>
public interface IWindowsHidApi
{
    /// <summary>Device interface paths of every HID collection present.</summary>
    IReadOnlyList<string> DevicePaths();

    /// <summary>The collection at <paramref name="path"/>, or null when it can't be asked.</summary>
    HidCollectionInfo? Query(string path);

    IHidWriter OpenForWrite(string path);
}

/// <summary>
/// The keyboard's Raw HID collection on Windows. Windows makes every top-level collection a
/// HID device of its own (the USB interface, and over Bluetooth the collection in the report
/// map, <c>…&amp;Col03</c>). Keyboards and mice are opened by Windows itself; a vendor
/// collection like this one can be opened and written by any program.
/// </summary>
public static class WindowsHid
{
    /// <summary>The service UUID in the device path of a Bluetooth LE HID collection.</summary>
    private const string HidOverGattService = "{00001812-0000-1000-8000-00805f9b34fb}";

    /// <summary>This keyboard's Raw HID collections, USB first.</summary>
    public static IReadOnlyList<RawHidDevice> Find(IWindowsHidApi api)
    {
        var found = new List<RawHidDevice>();
        foreach (var path in api.DevicePaths())
        {
            if (api.Query(path) is not { } info) continue;
            if (info.UsagePage != NotificationPackets.UsagePage || info.Usage != NotificationPackets.Usage) continue;
            if (info.VendorId != RawHidDevices.VendorId || info.ProductId != RawHidDevices.ProductId) continue;
            var outputBytes = info.OutputReportByteLength - 1;
            if (outputBytes < NotificationPackets.ReportSize) continue;
            found.Add(new RawHidDevice(path, TransportOf(path), info.OutputReportId, outputBytes));
        }
        return RawHidDevices.Preferred(found);
    }

    /// <summary>Bluetooth LE HID paths name the HID service; USB ones the USB IDs.</summary>
    public static HidTransport TransportOf(string path)
    {
        if (path.Contains(HidOverGattService, StringComparison.OrdinalIgnoreCase)) return HidTransport.Bluetooth;
        return path.Contains("vid_", StringComparison.OrdinalIgnoreCase) ? HidTransport.Usb : HidTransport.Unknown;
    }
}

/// <summary>A keyboard's Raw HID collection through the Windows HID driver.</summary>
public sealed class WindowsHidLink : IKeyboardLink
{
    private readonly IHidWriter _writer;
    private readonly RawHidDevice _target;

    public WindowsHidLink(IWindowsHidApi api, RawHidDevice target)
    {
        _target = target;
        _writer = api.OpenForWrite(target.Path);
        Description = target.Description;
    }

    public string Description { get; }

    public ValueTask SendAsync(ReadOnlyMemory<byte> report, CancellationToken cancellationToken = default)
    {
        _writer.Write(_target.Frame(report.Span));
        return ValueTask.CompletedTask;
    }

    public ValueTask DisposeAsync()
    {
        _writer.Dispose();
        return ValueTask.CompletedTask;
    }
}
