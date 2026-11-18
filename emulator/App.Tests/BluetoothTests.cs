namespace KbEmulator.Tests;

/// <summary>Bluetooth profiles, output and split link as the screens show them, through libkb_emu.so.</summary>
public class BluetoothTests
{
    private const int PosNav = 41, PosBtPrev = 25, PosBtNext = 26;
    private static readonly int[] ClearCombo = [PosBtPrev, PosBtNext];

    private static void TapOnNav(EmulatedKeyboard keyboard, int pos)
    {
        keyboard.Press(PosNav);
        keyboard.Press(pos);
        keyboard.Release(pos);
        keyboard.Release(PosNav);
    }

    [Test]
    public async Task Starts_on_the_first_profile_connected_over_bluetooth()
    {
        // Arrange
        using var keyboard = new EmulatedKeyboard();

        // Act
        var state = (keyboard.Profile, keyboard.ProfileOpen, keyboard.ProfileConnected, keyboard.Transport);

        // Assert
        await Assert.That(state).IsEqualTo((0, false, true, Transport.Bluetooth));
    }

    [Test]
    public async Task Bt_next_key_selects_the_next_profile_which_is_open()
    {
        // Arrange
        using var keyboard = new EmulatedKeyboard();

        // Act
        TapOnNav(keyboard, PosBtNext);

        // Assert
        await Assert.That(keyboard.Profile).IsEqualTo(1);
        await Assert.That(keyboard.ProfileOpen).IsTrue();
        await Assert.That(keyboard.ProfileConnected).IsFalse();
    }

    [Test]
    public async Task Bt_prev_key_from_the_first_profile_selects_usb()
    {
        // Arrange
        using var keyboard = new EmulatedKeyboard();

        // Act
        TapOnNav(keyboard, PosBtPrev);

        // Assert
        await Assert.That(keyboard.Transport).IsEqualTo(Transport.Usb);
    }

    [Test]
    public async Task Bt_prev_key_from_usb_selects_the_last_profile()
    {
        // Arrange
        using var keyboard = new EmulatedKeyboard();
        TapOnNav(keyboard, PosBtPrev); // USB

        // Act
        TapOnNav(keyboard, PosBtPrev);

        // Assert
        await Assert.That(keyboard.Profile).IsEqualTo(EmulatedKeyboard.ProfileCount - 1);
    }

    [Test]
    public async Task Bt_key_raises_bluetooth_changed()
    {
        // Arrange
        using var keyboard = new EmulatedKeyboard();
        var changes = 0;
        keyboard.BluetoothChanged += () => changes++;

        // Act
        TapOnNav(keyboard, PosBtNext);

        // Assert
        await Assert.That(changes).IsEqualTo(1);
    }

    [Test]
    public async Task Clear_combo_makes_the_profile_open_and_disconnected()
    {
        // Arrange
        using var keyboard = new EmulatedKeyboard();
        keyboard.Press(PosNav);
        foreach (var pos in ClearCombo[..^1]) keyboard.Press(pos);

        // Act
        keyboard.Press(ClearCombo[^1]);

        // Assert
        await Assert.That(keyboard.ProfileOpen).IsTrue();
        await Assert.That(keyboard.ProfileConnected).IsFalse();
    }

    [Test]
    public async Task Each_profile_keeps_its_own_pairing()
    {
        // Arrange
        using var keyboard = new EmulatedKeyboard();
        keyboard.Profile = 3;
        keyboard.SetProfileOpen(false);
        keyboard.Profile = 4;

        // Act
        keyboard.Profile = 3;

        // Assert
        await Assert.That(keyboard.ProfileOpen).IsFalse();
        await Assert.That(keyboard.ProfileConnected).IsFalse();
    }

    [Test]
    public async Task Connecting_an_open_profile_pairs_it()
    {
        // Arrange
        using var keyboard = new EmulatedKeyboard();
        keyboard.Profile = 2;

        // Act
        keyboard.SetProfileConnected(true);

        // Assert
        await Assert.That(keyboard.ProfileOpen).IsFalse();
        await Assert.That(keyboard.ProfileConnected).IsTrue();
    }

    [Test]
    public async Task Opening_the_connected_profile_disconnects_it()
    {
        // Arrange
        using var keyboard = new EmulatedKeyboard();

        // Act
        keyboard.SetProfileOpen(true);

        // Assert
        await Assert.That(keyboard.ProfileConnected).IsFalse();
    }

    [Test]
    public async Task Profile_out_of_range_is_clamped_to_the_last()
    {
        // Arrange
        using var keyboard = new EmulatedKeyboard();

        // Act
        keyboard.Profile = 99;

        // Assert
        await Assert.That(keyboard.Profile).IsEqualTo(EmulatedKeyboard.ProfileCount - 1);
    }

    [Test]
    public async Task Transport_can_be_set_to_usb()
    {
        // Arrange
        using var keyboard = new EmulatedKeyboard();

        // Act
        keyboard.Transport = Transport.Usb;

        // Assert
        await Assert.That(keyboard.Transport).IsEqualTo(Transport.Usb);
    }

    [Test]
    public async Task Right_half_can_be_unlinked()
    {
        // Arrange
        using var keyboard = new EmulatedKeyboard();

        // Act
        keyboard.SetPeripheralConnected(false);

        // Assert
        await Assert.That(keyboard.PeripheralConnected).IsFalse();
    }

    [Test]
    public async Task Link_stages_are_the_firmware_names_after_hidden()
    {
        // Arrange
        var stages = EmulatedKeyboard.LinkStages;

        // Act
        var (hidden, first, last) = (stages[0], stages[1], stages[^1]);

        // Assert
        await Assert.That(stages.Count).IsEqualTo(20);
        await Assert.That((hidden, first, last)).IsEqualTo(("", "start", "a-srv"));
    }

    [Test]
    public async Task Link_stage_is_kept_by_index()
    {
        // Arrange
        using var keyboard = new EmulatedKeyboard();
        var pair = EmulatedKeyboard.LinkStages.ToList().IndexOf("pair");

        // Act
        keyboard.LinkStage = pair;

        // Assert
        await Assert.That(keyboard.LinkStage).IsEqualTo(pair);
    }

    [Test]
    public async Task Showing_a_link_stage_redraws_the_left_screen()
    {
        // Arrange
        using var keyboard = new EmulatedKeyboard();
        keyboard.UpdateDisplays();
        var before = keyboard.Displays[0].Version;

        // Act
        keyboard.LinkStage = 3;
        keyboard.UpdateDisplays();

        // Assert
        await Assert.That(keyboard.Displays[0].Version).IsNotEqualTo(before);
    }

    [Test]
    public async Task Switching_to_an_open_profile_redraws_the_left_screen()
    {
        // Arrange
        using var keyboard = new EmulatedKeyboard();
        keyboard.UpdateDisplays();
        var before = keyboard.Displays[0].Version;

        // Act
        keyboard.Profile = 1;
        keyboard.UpdateDisplays();

        // Assert
        await Assert.That(keyboard.Displays[0].Version).IsNotEqualTo(before);
    }
}
