namespace KbEmulator.Tests;

public class HostTextTests
{
    private const byte LeftCtrl = 0x01, LeftShift = 0x02;
    private const int UsageA = 0x04, UsageC = 0x06, Usage1 = 0x1E, UsageBackspace = 0x2A, UsageF20 = 0x6F;

    private static HidReport Report(byte mods, params int[] usages)
    {
        var keys = new byte[32];
        foreach (var u in usages) keys[u / 8] |= (byte)(1 << (u % 8));
        return new HidReport(mods, keys);
    }

    private static HostText NewHost() => new(usage => usage == UsageF20 ? "F20" : $"U{usage:X2}");

    [Test]
    public async Task Letter_is_typed_lowercase()
    {
        // Arrange
        var host = NewHost();

        // Act
        host.Apply(HidReport.Empty, Report(0, UsageA));

        // Assert
        await Assert.That(host.Text).IsEqualTo("a");
    }

    [Test]
    public async Task Shift_makes_letter_uppercase()
    {
        // Arrange
        var host = NewHost();

        // Act
        host.Apply(HidReport.Empty, Report(LeftShift, UsageA));

        // Assert
        await Assert.That(host.Text).IsEqualTo("A");
    }

    [Test]
    public async Task Shifted_digit_is_symbol()
    {
        // Arrange
        var host = NewHost();

        // Act
        host.Apply(HidReport.Empty, Report(LeftShift, Usage1));

        // Assert
        await Assert.That(host.Text).IsEqualTo("!");
    }

    [Test]
    public async Task Ctrl_combination_is_shown_as_shortcut_token()
    {
        // Arrange
        var host = NewHost();

        // Act
        host.Apply(HidReport.Empty, Report(LeftCtrl, UsageC));

        // Assert
        await Assert.That(host.Text).IsEqualTo("[⌃C]");
    }

    [Test]
    public async Task Held_key_is_typed_only_once()
    {
        // Arrange
        var host = NewHost();
        var held = Report(0, UsageA);
        host.Apply(HidReport.Empty, held);

        // Act
        host.Apply(held, Report(0, UsageA, UsageC));

        // Assert
        await Assert.That(host.Text).IsEqualTo("ac");
    }

    [Test]
    public async Task Backspace_removes_last_character()
    {
        // Arrange
        var host = NewHost();
        host.Apply(HidReport.Empty, Report(0, UsageA));

        // Act
        host.Apply(HidReport.Empty, Report(0, UsageBackspace));

        // Assert
        await Assert.That(host.Text).IsEqualTo("");
    }

    [Test]
    public async Task Non_printing_key_is_shown_by_name()
    {
        // Arrange
        var host = NewHost();

        // Act
        host.Apply(HidReport.Empty, Report(0, UsageF20));

        // Assert
        await Assert.That(host.Text).IsEqualTo("[F20]");
    }
}
