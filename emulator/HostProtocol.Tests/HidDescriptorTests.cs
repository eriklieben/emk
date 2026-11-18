namespace Kb.HostProtocol.Tests;

public class HidDescriptorTests
{
    [Test]
    public async Task Usb_interface_has_no_report_id()
    {
        // Arrange
        var descriptor = Descriptors.Usb;

        // Act
        var raw = HidDescriptor.FindRawHid(descriptor);

        // Assert
        await Assert.That(raw).IsEqualTo(new RawHidCollection(0, 32));
    }

    [Test]
    public async Task Bluetooth_report_map_has_raw_hid_as_report_3()
    {
        // Arrange: the keyboard's report ID 1 and its LED output come first
        var descriptor = Descriptors.BluetoothMap;

        // Act
        var raw = HidDescriptor.FindRawHid(descriptor);

        // Assert
        await Assert.That(raw).IsEqualTo(new RawHidCollection(3, 32));
    }

    [Test]
    public async Task Keyboard_alone_has_no_raw_hid()
    {
        // Arrange
        var descriptor = Descriptors.Keyboard;

        // Act
        var raw = HidDescriptor.FindRawHid(descriptor);

        // Assert
        await Assert.That(raw).IsNull();
    }

    [Test]
    public async Task Usage_with_its_page_in_one_item_is_recognized()
    {
        // Arrange: Usage (0xFF600061) as a 4-byte local item instead of Usage Page + Usage
        byte[] descriptor =
        [
            0x0B, 0x61, 0x00, 0x60, 0xFF, 0xA1, 0x01, 0x85, 0x05,
            0x75, 0x08, 0x95, 0x20, 0x91, 0x02, 0xC0,
        ];

        // Act
        var raw = HidDescriptor.FindRawHid(descriptor);

        // Assert
        await Assert.That(raw).IsEqualTo(new RawHidCollection(5, 32));
    }

    [Test]
    public async Task Cut_off_descriptor_is_not_raw_hid()
    {
        // Arrange: the collection never ends
        var descriptor = Descriptors.Usb[..^1];

        // Act
        var raw = HidDescriptor.FindRawHid(descriptor);

        // Assert
        await Assert.That(raw).IsNull();
    }

    /// <summary>The Raw HID collection with an output report but no Report ID item of its own.</summary>
    private static readonly byte[] RawHidWithoutReportId =
    [
        0x06, 0x60, 0xFF, 0x09, 0x61, 0xA1, 0x01,
        0x09, 0x63, 0x15, 0x00, 0x26, 0xFF, 0x00, 0x95, 0x20, 0x75, 0x08, 0x91, 0x02,
        0xC0,
    ];

    [Test]
    public async Task Long_item_is_skipped()
    {
        // Arrange: a long item (size 2, tag 0x10) in front of the USB interface
        byte[] descriptor = [0xFE, 0x02, 0x10, 0xAA, 0xBB, .. Descriptors.Usb];

        // Act
        var raw = HidDescriptor.FindRawHid(descriptor);

        // Assert
        await Assert.That(raw).IsEqualTo(new RawHidCollection(0, 32));
    }

    [Test]
    public async Task Long_item_cut_off_ends_the_descriptor()
    {
        // Arrange
        byte[] descriptor = [0xFE];

        // Act
        var raw = HidDescriptor.FindRawHid(descriptor);

        // Assert
        await Assert.That(raw).IsNull();
    }

    [Test]
    public async Task Pop_restores_the_pushed_report_id()
    {
        // Arrange: Report ID 3, Push, Report ID 7, Pop
        byte[] descriptor = [0x85, 0x03, 0xA4, 0x85, 0x07, 0xB4, .. RawHidWithoutReportId];

        // Act
        var raw = HidDescriptor.FindRawHid(descriptor);

        // Assert
        await Assert.That(raw).IsEqualTo(new RawHidCollection(3, 32));
    }

    [Test]
    public async Task Pop_without_a_push_is_ignored()
    {
        // Arrange: Report ID 3, Pop
        byte[] descriptor = [0x85, 0x03, 0xB4, .. RawHidWithoutReportId];

        // Act
        var raw = HidDescriptor.FindRawHid(descriptor);

        // Assert
        await Assert.That(raw).IsEqualTo(new RawHidCollection(3, 32));
    }
}
