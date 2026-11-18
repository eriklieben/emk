using Kb.HostProtocol;

namespace KbEmulator.Tests;

/// <summary>Idle blanking and deep sleep, with a manual clock instead of real time.</summary>
public class PowerTests
{
    private const int PosA = 13;

    private sealed class Clock
    {
        public uint Now;
    }

    private static (EmulatedKeyboard Keyboard, Clock Clock) QuickKeyboard()
    {
        var clock = new Clock();
        var keyboard = new EmulatedKeyboard(() => clock.Now);
        keyboard.SetPowerTimeouts(TimeSpan.FromSeconds(5), TimeSpan.FromSeconds(20));
        keyboard.UpdateDisplays();
        return (keyboard, clock);
    }

    private static void Advance(EmulatedKeyboard keyboard, Clock clock, uint ms)
    {
        clock.Now += ms;
        keyboard.Tick();
        keyboard.UpdateDisplays();
    }

    [Test]
    public async Task Displays_show_the_sleep_picture_when_idle()
    {
        // Arrange
        var (keyboard, clock) = QuickKeyboard();
        using var _ = keyboard;

        // Act
        Advance(keyboard, clock, 5_000);

        // Assert
        await Assert.That(keyboard.Power).IsEqualTo(PowerState.Idle);
        await Assert.That(keyboard.Displays.All(d => d.IsLit)).IsTrue();
    }

    [Test]
    public async Task Displays_switch_off_after_the_sleep_picture()
    {
        // Arrange
        var (keyboard, clock) = QuickKeyboard();
        using var _ = keyboard;
        Advance(keyboard, clock, 5_000);

        // Act
        Advance(keyboard, clock, 3_000);

        // Assert
        await Assert.That(keyboard.Power).IsEqualTo(PowerState.Idle);
        await Assert.That(keyboard.Displays.Any(d => d.IsLit)).IsFalse();
    }

    [Test]
    public async Task Key_press_switches_idle_displays_back_on()
    {
        // Arrange
        var (keyboard, clock) = QuickKeyboard();
        using var _ = keyboard;
        Advance(keyboard, clock, 5_000);
        Advance(keyboard, clock, 3_000); // past the sleep picture: off

        // Act
        keyboard.Press(PosA);
        keyboard.UpdateDisplays();

        // Assert
        await Assert.That(keyboard.Displays.All(d => d.IsLit)).IsTrue();
    }

    [Test]
    public async Task Deep_sleep_cuts_oled_power()
    {
        // Arrange
        var (keyboard, clock) = QuickKeyboard();
        using var _ = keyboard;

        // Act
        Advance(keyboard, clock, 20_000);

        // Assert
        await Assert.That(keyboard.Power).IsEqualTo(PowerState.Sleep);
        await Assert.That(keyboard.Displays.Any(d => d.Powered)).IsFalse();
    }

    [Test]
    public async Task Key_that_wakes_from_deep_sleep_is_not_typed()
    {
        // Arrange
        var (keyboard, clock) = QuickKeyboard();
        using var _ = keyboard;
        Advance(keyboard, clock, 20_000);
        var reports = 0;
        keyboard.ReportSent += (_, _) => reports++;

        // Act
        keyboard.Press(PosA);

        // Assert
        await Assert.That(keyboard.Power).IsEqualTo(PowerState.Active);
        await Assert.That(reports).IsEqualTo(0);
    }

    [Test]
    public async Task Displays_come_back_after_waking_from_deep_sleep()
    {
        // Arrange
        var (keyboard, clock) = QuickKeyboard();
        using var _ = keyboard;
        Advance(keyboard, clock, 20_000);
        keyboard.Press(PosA);

        // Act
        keyboard.UpdateDisplays();

        // Assert
        await Assert.That(keyboard.Displays.All(d => d.IsLit)).IsTrue();
    }

    [Test]
    public async Task Notification_is_lost_during_deep_sleep()
    {
        // Arrange
        var (keyboard, clock) = QuickKeyboard();
        using var _ = keyboard;
        Advance(keyboard, clock, 20_000);

        // Act
        var result = keyboard.RawHid(NotificationPackets.Encode("Hi")[0]);

        // Assert
        await Assert.That(result).IsEqualTo(RawHidResult.Asleep);
    }
}
