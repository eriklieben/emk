using System.Net.Sockets;

namespace Kb.HostProtocol.Tests;

public class KeyboardLinksTests
{
    [Test]
    public async Task Descriptor_with_raw_hid_usage_is_recognized()
    {
        // Arrange
        var descriptor = Descriptors.Usb;

        // Act
        var found = HidrawLink.DeclaresRawHid(descriptor);

        // Assert
        await Assert.That(found).IsTrue();
    }

    [Test]
    public async Task Keyboard_descriptor_is_not_raw_hid()
    {
        // Arrange
        var descriptor = Descriptors.Keyboard;

        // Act
        var found = HidrawLink.DeclaresRawHid(descriptor);

        // Assert
        await Assert.That(found).IsFalse();
    }

    [Test]
    public async Task Finder_returns_matching_hidraw_devices()
    {
        // Arrange: fake /sys/class/hidraw with one Raw HID and one keyboard device
        var root = Directory.CreateTempSubdirectory("hidraw-test").FullName;
        WriteDevice(root, "hidraw0", Descriptors.Keyboard);
        WriteDevice(root, "hidraw3", Descriptors.Usb);

        // Act
        var devices = HidrawLink.Find(root).Select(d => d.Path).ToArray();

        // Assert
        await Assert.That(devices).IsEquivalentTo(new[] { "/dev/hidraw3" });
        Directory.Delete(root, recursive: true);
    }

    [Test]
    public async Task Finder_puts_usb_before_bluetooth()
    {
        // Arrange: the keyboard over Bluetooth (hidraw1) and on the cable (hidraw4)
        var root = Directory.CreateTempSubdirectory("hidraw-test").FullName;
        WriteDevice(root, "hidraw1", Descriptors.BluetoothMap, "HID_ID=0005:00001209:00000001");
        WriteDevice(root, "hidraw4", Descriptors.Usb, "HID_ID=0003:00001209:00000001");

        // Act
        var devices = HidrawLink.Find(root);

        // Assert
        await Assert.That(devices.Select(d => (d.Path, d.Transport, d.ReportId)).ToArray()).IsEquivalentTo(new[]
        {
            ("/dev/hidraw4", HidTransport.Usb, (byte)0),
            ("/dev/hidraw1", HidTransport.Bluetooth, (byte)3),
        });
        Directory.Delete(root, recursive: true);
    }

    [Test]
    public async Task Finder_skips_other_keyboards_with_raw_hid()
    {
        // Arrange: a QMK keyboard (same usage page) with other USB IDs
        var root = Directory.CreateTempSubdirectory("hidraw-test").FullName;
        WriteDevice(root, "hidraw2", Descriptors.Usb, "HID_ID=0003:00003434:00000361");

        // Act
        var devices = HidrawLink.Find(root);

        // Assert
        await Assert.That(devices).IsEmpty();
        Directory.Delete(root, recursive: true);
    }

    [Test]
    public async Task Device_given_by_path_gets_its_report_id_from_the_descriptor()
    {
        // Arrange
        var root = Directory.CreateTempSubdirectory("hidraw-test").FullName;
        WriteDevice(root, "hidraw7", Descriptors.BluetoothMap, "HID_ID=0005:00001209:00000001");

        // Act
        var device = HidrawLink.ForPath("/dev/hidraw7", root);

        // Assert
        await Assert.That(device.ReportId).IsEqualTo((byte)3);
        Directory.Delete(root, recursive: true);
    }

    [Test]
    public async Task Hidraw_link_writes_the_report_id_first()
    {
        // Arrange: a file standing in for the hidraw node
        var path = Path.Combine(Path.GetTempPath(), $"hidraw-{Guid.NewGuid():N}");
        await File.WriteAllBytesAsync(path, []);
        var packet = NotificationPackets.Encode("Hi", id: 2)[0];
        var target = new RawHidDevice(path, HidTransport.Bluetooth, 3, NotificationPackets.ReportSize);

        // Act
        await using (var link = new HidrawLink(target))
        {
            await link.SendAsync(packet);
        }

        // Assert
        var written = await File.ReadAllBytesAsync(path);
        await Assert.That(written).IsEquivalentTo(new byte[] { 3 }.Concat(packet).ToArray());
        File.Delete(path);
    }

    [Test]
    public async Task Emulator_link_sends_packets_unchanged()
    {
        // Arrange
        var path = Path.Combine(Path.GetTempPath(), $"kb-test-{Guid.NewGuid():N}.sock");
        using var server = new Socket(AddressFamily.Unix, SocketType.Stream, ProtocolType.Unspecified);
        server.Bind(new UnixDomainSocketEndPoint(path));
        server.Listen();
        var packet = NotificationPackets.Encode("Hi", id: 4)[0];
        var accept = server.AcceptAsync();

        // Act
        await using (var link = await EmulatorLink.ConnectAsync(path))
        {
            await link.SendAsync(packet);
        }

        // Assert
        using var client = await accept;
        var received = new byte[64];
        var n = await client.ReceiveAsync(received, SocketFlags.None);
        await Assert.That(received[..n]).IsEquivalentTo(packet);
        File.Delete(path);
    }

    private static void WriteDevice(string root, string name, byte[] descriptor, string? uevent = null)
    {
        var dir = Directory.CreateDirectory(Path.Combine(root, name, "device")).FullName;
        File.WriteAllBytes(Path.Combine(dir, "report_descriptor"), descriptor);
        if (uevent != null) File.WriteAllText(Path.Combine(dir, "uevent"), $"DRIVER=hid-generic\n{uevent}\n");
    }
}
