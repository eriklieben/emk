namespace Kb.HostProtocol.Tests;

public class WindowsHidTests
{
    private const string UsbPath = @"\\?\hid#vid_1209&pid_0001&mi_04#8&2a1b3c4d&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}";
    private const string UsbKeyboardPath = @"\\?\hid#vid_1209&pid_0001&mi_00#8&1a1b3c4d&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}\kbd";
    private const string BluetoothPath = @"\\?\hid#{00001812-0000-1000-8000-00805f9b34fb}_dev_vid&021209_pid&0001_rev&0001_c1111111111c&col03#9&3a1b3c4d&0&0002#{4d1e55b2-f16f-11cf-88cb-001111000030}";
    private const string OtherPath = @"\\?\hid#vid_3434&pid_0361&mi_01#8&5a1b3c4d&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}";

    private static readonly HidCollectionInfo UsbRawHid = new(0x1209, 0x0001, 0xFF60, 0x61, 33, 0);
    private static readonly HidCollectionInfo BluetoothRawHid = new(0x1209, 0x0001, 0xFF60, 0x61, 33, 3);
    private static readonly HidCollectionInfo Keyboard = new(0x1209, 0x0001, 0x01, 0x06, 2, 0);

    [Test]
    public async Task Usb_collection_comes_before_bluetooth()
    {
        // Arrange
        var api = new FakeHid(
            (BluetoothPath, BluetoothRawHid),
            (UsbKeyboardPath, Keyboard),
            (UsbPath, UsbRawHid));

        // Act
        var found = WindowsHid.Find(api);

        // Assert
        await Assert.That(found.Select(d => (d.Path, d.Transport, d.ReportId)).ToArray()).IsEquivalentTo(new[]
        {
            (UsbPath, HidTransport.Usb, (byte)0),
            (BluetoothPath, HidTransport.Bluetooth, (byte)3),
        });
    }

    [Test]
    public async Task Bluetooth_collection_is_used_without_the_cable()
    {
        // Arrange
        var api = new FakeHid((UsbKeyboardPath, Keyboard), (BluetoothPath, BluetoothRawHid));

        // Act
        var found = WindowsHid.Find(api);

        // Assert
        await Assert.That(found.Single().ReportId).IsEqualTo((byte)3);
    }

    [Test]
    public async Task Other_vendors_raw_hid_is_left_alone()
    {
        // Arrange: a QMK keyboard declares the same usage page and usage
        var api = new FakeHid((OtherPath, UsbRawHid with { VendorId = 0x3434, ProductId = 0x0361 }));

        // Act
        var found = WindowsHid.Find(api);

        // Assert
        await Assert.That(found).IsEmpty();
    }

    [Test]
    public async Task Collection_that_cannot_be_asked_is_skipped()
    {
        // Arrange
        var api = new FakeHid((UsbPath, null));

        // Act
        var found = WindowsHid.Find(api);

        // Assert
        await Assert.That(found).IsEmpty();
    }

    [Test]
    public async Task Link_writes_the_whole_output_report_with_its_id_first()
    {
        // Arrange
        var api = new FakeHid((BluetoothPath, BluetoothRawHid));
        var target = WindowsHid.Find(api).Single();
        var packet = NotificationPackets.Encode("Hi", id: 1)[0];

        // Act
        await using (var link = new WindowsHidLink(api, target))
        {
            await link.SendAsync(packet);
        }

        // Assert: OutputReportByteLength (33) bytes
        await Assert.That(api.Written.Single()).IsEquivalentTo(new byte[] { 3 }.Concat(packet).ToArray());
    }

    [Test]
    public async Task Link_closes_the_device_when_done()
    {
        // Arrange
        var api = new FakeHid((UsbPath, UsbRawHid));
        var link = new WindowsHidLink(api, WindowsHid.Find(api).Single());

        // Act
        await link.DisposeAsync();

        // Assert
        await Assert.That(api.Closed).IsTrue();
    }

    /// <summary>Collections by path, and what was written to them.</summary>
    private sealed class FakeHid(params (string Path, HidCollectionInfo? Info)[] devices) : IWindowsHidApi
    {
        private readonly Dictionary<string, HidCollectionInfo?> _devices =
            devices.ToDictionary(d => d.Path, d => d.Info);

        public List<byte[]> Written { get; } = [];

        public bool Closed { get; private set; }

        public IReadOnlyList<string> DevicePaths() => _devices.Keys.ToArray();

        public HidCollectionInfo? Query(string path) => _devices.GetValueOrDefault(path);

        public IHidWriter OpenForWrite(string path) => new Writer(this);

        private sealed class Writer(FakeHid hid) : IHidWriter
        {
            public void Write(byte[] report) => hid.Written.Add(report);

            public void Dispose() => hid.Closed = true;
        }
    }
}
