namespace KbEmulator.Tests;

/// <summary>End-to-end through the Rust firmware library (libkb_emu.so).</summary>
public class EmulatedKeyboardTests
{
    private const int PosA = 13, PosLower = 36, PosDel = 40;
    private const int UsageA = 0x04;

    [Test]
    public async Task Pressing_a_key_sends_its_usage()
    {
        // Arrange
        using var keyboard = new EmulatedKeyboard();

        // Act
        keyboard.Press(PosA);

        // Assert
        await Assert.That(keyboard.LastReport.Contains(UsageA)).IsTrue();
    }

    [Test]
    public async Task Releasing_a_key_sends_empty_report()
    {
        // Arrange
        using var keyboard = new EmulatedKeyboard();
        keyboard.Press(PosA);

        // Act
        keyboard.Release(PosA);

        // Assert
        await Assert.That(keyboard.LastReport.Contains(UsageA)).IsFalse();
    }

    [Test]
    public async Task Holding_lower_raises_layer_changed()
    {
        // Arrange
        using var keyboard = new EmulatedKeyboard();
        int? layer = null;
        keyboard.LayerChanged += l => layer = l;

        // Act
        keyboard.Press(PosLower);

        // Assert
        await Assert.That(layer).IsEqualTo(1);
        await Assert.That(keyboard.LayerName(1)).IsEqualTo("symbols");
    }

    [Test]
    public async Task Delete_thumb_label_is_plain_delete()
    {
        // Arrange
        using var keyboard = new EmulatedKeyboard();

        // Act
        var label = keyboard.Label(PosDel);

        // Assert
        await Assert.That(label).IsEqualTo("Del");
    }

    [Test]
    public async Task Display_update_turns_on_both_oleds_and_draws_pixels()
    {
        // Arrange
        using var keyboard = new EmulatedKeyboard();

        // Act
        keyboard.UpdateDisplays();

        // Assert
        await Assert.That(keyboard.Displays.All(d => d.IsLit)).IsTrue();
        await Assert.That(keyboard.Displays.All(d => LitPixels(d).Contains(true))).IsTrue();
    }

    [Test]
    public async Task Layer_change_redraws_left_oled()
    {
        // Arrange
        using var keyboard = new EmulatedKeyboard();
        keyboard.UpdateDisplays();
        var before = keyboard.Displays[0].Version;
        keyboard.Press(PosLower);

        // Act
        keyboard.UpdateDisplays();

        // Assert
        await Assert.That(keyboard.Displays[0].Version).IsNotEqualTo(before);
    }

    private static IEnumerable<bool> LitPixels(Kb.Ssd1306.Ssd1306 d)
    {
        for (var y = 0; y < d.PanelHeight; y++)
            for (var x = 0; x < Kb.Ssd1306.Ssd1306.Columns; x++)
                yield return d.GetPixel(x, y);
    }
}
