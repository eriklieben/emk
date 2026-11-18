namespace Kb.Ssd1306.Tests;

public class Ssd1306Tests
{
    // A typical single-transaction init (non-inverted), used for geometry tests.
    private static readonly byte[] Init128X32 =
    [
        0x00, 0xAE, 0xD5, 0x80, 0xA8, 0x1F, 0xD3, 0x00, 0x40, 0x8D, 0x14, 0x20, 0x00,
        0xA1, 0xC8, 0xDA, 0x02, 0x81, 0x8F, 0xD9, 0xF1, 0xDB, 0x40, 0xA4, 0xA6, 0xAF,
    ];

    // What ZMK's Zephyr driver sends for the Corne (same as INIT in display/src/ssd1306.rs).
    private static readonly byte[][] ZmkInit =
    [
        [0x00, 0xAE],
        [0x00, 0xD5, 0x80, 0xD9, 0x22, 0xDB, 0x20],
        [0x00, 0x40, 0xD3, 0x00, 0xDA, 0x02, 0xA8, 0x1F],
        [0x00, 0xA1, 0xC8],
        [0x00, 0x8D, 0x14, 0x33],
        [0x00, 0xA4, 0xA7],
        [0x00, 0x81, 0x80],
        [0x00, 0xAF],
    ];

    private static Ssd1306 InitializedDisplay()
    {
        var display = new Ssd1306();
        display.Write(Init128X32);
        return display;
    }

    /// <summary>Write a full 128×32 frame of zeros with one byte set to <paramref name="value"/>.</summary>
    private static void WriteFrame(Ssd1306 display, int page, int column, byte value)
    {
        display.Write([0x00, 0x20, 0x00, 0x21, 0, 127, 0x22, 0, 3]); // like ZMK: mode + window per write
        var data = new byte[1 + 128 * 4];
        data[0] = 0x40;
        data[1 + page * 128 + column] = value;
        display.Write(data);
    }

    [Test]
    public async Task Init_sequence_turns_display_on_in_horizontal_mode()
    {
        // Arrange
        var display = new Ssd1306();

        // Act
        display.Write(Init128X32);

        // Assert
        await Assert.That(display.IsLit).IsTrue();
        await Assert.That(display.Mode).IsEqualTo(Ssd1306.AddressingMode.Horizontal);
        await Assert.That(display.Multiplex).IsEqualTo(32);
        await Assert.That(display.SegmentRemap).IsTrue();
        await Assert.That(display.ComScanReversed).IsTrue();
        await Assert.That(display.Contrast).IsEqualTo(0x8F);
    }

    [Test]
    public async Task Frame_byte_appears_upright_with_remap_and_rotated_mount()
    {
        // Arrange
        var display = InitializedDisplay();

        // Act: page 1, column 3, bit 2 → RAM row 10
        WriteFrame(display, page: 1, column: 3, value: 0b0000_0100);

        // Assert
        await Assert.That(display.GetPixel(3, 10)).IsTrue();
        await Assert.That(display.GetPixel(124, 21)).IsFalse();
    }

    [Test]
    public async Task Without_remap_a_rotated_module_shows_the_image_upside_down()
    {
        // Arrange
        var display = InitializedDisplay();
        display.Write([0x00, 0xA0, 0xC0]); // undo segment and COM remap

        // Act
        WriteFrame(display, page: 1, column: 3, value: 0b0000_0100);

        // Assert
        await Assert.That(display.GetPixel(3, 10)).IsFalse();
        await Assert.That(display.GetPixel(124, 21)).IsTrue();
    }

    [Test]
    public async Task Display_stays_dark_without_charge_pump()
    {
        // Arrange
        var display = new Ssd1306();
        var initWithoutChargePump = Init128X32.Where((_, i) => i is not (9 or 10)).ToArray();

        // Act
        display.Write(initWithoutChargePump);

        // Assert
        await Assert.That(display.DisplayOn).IsTrue();
        await Assert.That(display.IsLit).IsFalse();
    }

    [Test]
    public async Task Inverse_mode_flips_pixels()
    {
        // Arrange
        var display = InitializedDisplay();
        WriteFrame(display, page: 0, column: 0, value: 0x01);

        // Act
        display.Write([0x00, 0xA7]);

        // Assert
        await Assert.That(display.GetPixel(0, 0)).IsFalse();
        await Assert.That(display.GetPixel(1, 0)).IsTrue();
    }

    [Test]
    public async Task Entire_display_on_lights_every_pixel()
    {
        // Arrange
        var display = InitializedDisplay();

        // Act
        display.Write([0x00, 0xA5]);

        // Assert
        await Assert.That(display.GetPixel(64, 16)).IsTrue();
    }

    [Test]
    public async Task Horizontal_mode_wraps_to_next_page_inside_column_window()
    {
        // Arrange
        var display = InitializedDisplay();
        display.Write([0x00, 0x21, 10, 11, 0x22, 2, 3]);

        // Act
        display.Write([0x40, 0xA1, 0xA2, 0xA3]);

        // Assert
        await Assert.That(display.Ram[2 * 128 + 10]).IsEqualTo(0xA1);
        await Assert.That(display.Ram[2 * 128 + 11]).IsEqualTo(0xA2);
        await Assert.That(display.Ram[3 * 128 + 10]).IsEqualTo(0xA3);
    }

    [Test]
    public async Task Vertical_mode_advances_page_first()
    {
        // Arrange
        var display = InitializedDisplay();
        display.Write([0x00, 0x20, 0x01, 0x21, 0, 127, 0x22, 0, 1]);

        // Act
        display.Write([0x40, 0x11, 0x22, 0x33]);

        // Assert
        await Assert.That(display.Ram[0]).IsEqualTo(0x11);
        await Assert.That(display.Ram[128]).IsEqualTo(0x22);
        await Assert.That(display.Ram[1]).IsEqualTo(0x33);
    }

    [Test]
    public async Task Page_mode_uses_page_and_column_start_commands()
    {
        // Arrange
        var display = InitializedDisplay();
        display.Write([0x00, 0x20, 0x02]);

        // Act: page 2, column 0x25
        display.Write([0x00, 0xB2, 0x05, 0x12]);
        display.Write([0x40, 0x7E]);

        // Assert
        await Assert.That(display.Ram[2 * 128 + 0x25]).IsEqualTo(0x7E);
    }

    [Test]
    public async Task Continuation_bit_interleaves_commands_and_data()
    {
        // Arrange
        var display = InitializedDisplay();
        display.Write([0x00, 0x21, 0, 127, 0x22, 0, 3]);

        // Act: Co=1 command (A7 invert), then Co=0 data stream
        display.Write([0x80, 0xA7, 0x40, 0x55]);

        // Assert
        await Assert.That(display.Inverted).IsTrue();
        await Assert.That(display.Ram[0]).IsEqualTo(0x55);
    }

    [Test]
    public async Task Command_arguments_may_arrive_in_a_later_transaction()
    {
        // Arrange
        var display = InitializedDisplay();
        display.Write([0x00, 0x81]);

        // Act
        display.Write([0x00, 0x20]);

        // Assert
        await Assert.That(display.Contrast).IsEqualTo(0x20);
    }

    [Test]
    public async Task Scroll_setup_arguments_are_consumed()
    {
        // Arrange
        var display = InitializedDisplay();

        // Act: right scroll setup has 6 argument bytes, then A7
        display.Write([0x00, 0x26, 0x00, 0x00, 0x00, 0x03, 0x00, 0xFF, 0xA7]);

        // Assert
        await Assert.That(display.Inverted).IsTrue();
    }

    [Test]
    public async Task Version_changes_when_ram_is_written()
    {
        // Arrange
        var display = InitializedDisplay();
        var before = display.Version;

        // Act
        display.Write([0x40, 0xFF]);

        // Assert
        await Assert.That(display.Version).IsNotEqualTo(before);
    }

    [Test]
    public async Task Zmk_init_sequence_turns_display_on_inverted()
    {
        // Arrange
        var display = new Ssd1306();

        // Act
        foreach (var transaction in ZmkInit) display.Write(transaction);

        // Assert
        await Assert.That(display.IsLit).IsTrue();
        await Assert.That(display.Inverted).IsTrue();
        await Assert.That(display.Contrast).IsEqualTo((byte)0x80);
        await Assert.That(display.Multiplex).IsEqualTo(32);
    }

    [Test]
    public async Task Zmk_inverted_data_shows_cleared_bits_as_lit()
    {
        // Arrange: ZMK sends inverted bytes, so a lit pixel is a 0 bit
        var display = new Ssd1306();
        foreach (var transaction in ZmkInit) display.Write(transaction);

        // Act
        WriteFrame(display, page: 0, column: 0, value: 0xFE);

        // Assert
        await Assert.That(display.GetPixel(0, 0)).IsTrue();
        await Assert.That(display.GetPixel(0, 1)).IsFalse();
    }

    [Test]
    public async Task Power_cut_darkens_the_panel_and_clears_its_ram()
    {
        // Arrange
        var display = InitializedDisplay();
        WriteFrame(display, page: 0, column: 0, value: 0xFF);

        // Act
        display.Powered = false;

        // Assert
        await Assert.That(display.IsLit).IsFalse();
        await Assert.That(display.Ram[0]).IsEqualTo((byte)0);
    }

    [Test]
    public async Task Power_restored_needs_init_before_it_lights_up()
    {
        // Arrange
        var display = InitializedDisplay();
        display.Powered = false;

        // Act
        display.Powered = true;

        // Assert
        await Assert.That(display.IsLit).IsFalse();
        await Assert.That(display.DisplayOn).IsFalse();
    }

    [Test]
    public async Task Writes_are_ignored_while_unpowered()
    {
        // Arrange
        var display = new Ssd1306 { Powered = false };

        // Act
        display.Write(Init128X32);

        // Assert
        await Assert.That(display.DisplayOn).IsFalse();
    }
}
