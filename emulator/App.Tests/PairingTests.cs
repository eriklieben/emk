namespace KbEmulator.Tests;

/// <summary>Pairing with a code typed on the keyboard, and the radio power key, through the Rust firmware library.</summary>
public class PairingTests
{
    private const int PosLower = 36, PosEnter = 39, PosNav = 41, PosRadio = 29;

    /// <summary>Positions of the digits on the symbols layer: 1..5, then 6..9, 0.</summary>
    private static int DigitPos(int d) => d switch
    {
        >= 1 and <= 5 => 12 + d,
        >= 6 and <= 9 => 19 + d,
        _ => 29,
    };

    private static void TypeCode(EmulatedKeyboard keyboard, int code)
    {
        keyboard.Press(PosLower);
        foreach (var c in code.ToString("000000", System.Globalization.CultureInfo.InvariantCulture))
        {
            keyboard.Press(DigitPos(c - '0'));
            keyboard.Release(DigitPos(c - '0'));
        }
        keyboard.Release(PosLower);
    }

    [Test]
    public async Task Typing_the_computers_code_and_enter_pairs()
    {
        // Arrange
        using var keyboard = new EmulatedKeyboard(() => 0);
        PairResult? result = null;
        keyboard.PairingEnded += r => result = r;
        var code = keyboard.SimulatePairing(305_917);
        TypeCode(keyboard, code);

        // Act
        keyboard.Press(PosEnter);

        // Assert
        await Assert.That(result).IsEqualTo(PairResult.Paired);
        await Assert.That(keyboard.Pairing).IsFalse();
    }

    [Test]
    public async Task Typed_code_does_not_reach_the_computer()
    {
        // Arrange
        using var keyboard = new EmulatedKeyboard(() => 0);
        keyboard.SimulatePairing(111_111);
        var typed = false;
        keyboard.ReportSent += (_, report) => typed |= report.Keys.Any(b => b != 0);

        // Act
        TypeCode(keyboard, 111_111);

        // Assert
        await Assert.That(typed).IsFalse();
    }

    [Test]
    public async Task Another_code_fails_the_pairing()
    {
        // Arrange
        using var keyboard = new EmulatedKeyboard(() => 0);
        PairResult? result = null;
        keyboard.PairingEnded += r => result = r;
        keyboard.SimulatePairing(111_111);
        TypeCode(keyboard, 222_222);

        // Act
        keyboard.Press(PosEnter);

        // Assert
        await Assert.That(result).IsEqualTo(PairResult.WrongCode);
    }

    [Test]
    public async Task Radio_key_steps_the_power_from_the_default()
    {
        // Arrange
        using var keyboard = new EmulatedKeyboard(() => 0);
        string? action = null;
        keyboard.SystemAction += a => action = a;
        keyboard.Press(PosNav);

        // Act
        keyboard.Press(PosRadio);

        // Assert
        await Assert.That(action).IsEqualTo("Radio power");
        await Assert.That(keyboard.RadioDbm).IsEqualTo(8);
    }

    [Test]
    public async Task Radio_key_is_labelled_radio()
    {
        // Arrange
        using var keyboard = new EmulatedKeyboard(() => 0);
        keyboard.Press(PosNav);

        // Act
        var label = keyboard.Label(PosRadio);

        // Assert
        await Assert.That(label).IsEqualTo("Radio");
    }
}
