using Avalonia;
using Avalonia.Headless;
using Avalonia.Input;
using KbEmulator.Bench;

namespace KbEmulator.Tests;

/// <summary>A window on a hand-moved clock, for UI tests.</summary>
internal sealed class TestWindow
{
    public uint Now;

    public string Socket { get; } = Path.Combine(Path.GetTempPath(), $"kb-ui-{Guid.NewGuid():N}.sock");

    public TestWindow()
    {
        Window = new MainWindow(Socket, new HardwarePort(() => null), () => Now);
        Window.Show();
    }

    public MainWindow Window { get; }
    public EmulatorControls C => Window.Controls;
    public EmulatedKeyboard K => Window.Keyboard;

    public void Advance(uint ms)
    {
        for (var t = 0u; t < ms; t += 50)
        {
            Now += 50;
            for (var i = 0; i < 10; i++) Window.Tick();
        }
    }

    /// <summary>Run <paramref name="act"/> on the UI thread with a fresh window, then close it.</summary>
    public static Task<T> Run<T>(Func<TestWindow, T> act) => TestAppBuilder.Session.Dispatch(() =>
    {
        var w = new TestWindow();
        try
        {
            return act(w);
        }
        finally
        {
            w.Window.Close();
        }
    }, CancellationToken.None);
}

/// <summary>Every Emulator control, used as a user would, changes the emulated keyboard.</summary>
[NotInParallel("ui-session")]
public class EmulatorControlTests
{
    [Test]
    public async Task Dragging_the_left_gauge_sets_the_battery()
    {
        // Arrange
        var (text, slider) = await TestWindow.Run(w =>
        {
            // Act
            w.C.Gauges[0].Value = 40;
            return (w.C.BatteryText[0].Text, w.C.Sliders[0].Value);
        });

        // Assert
        await Assert.That((text, slider)).IsEqualTo(("40%", 40.0));
    }

    [Test]
    public async Task Moving_the_right_slider_moves_its_gauge()
    {
        // Arrange
        var gauge = await TestWindow.Run(w =>
        {
            // Act
            w.C.Sliders[1].Value = 70;
            return w.C.Gauges[1].Value;
        });

        // Assert
        await Assert.That(gauge).IsEqualTo(70);
    }

    [Test]
    public async Task Quick_power_preset_idles_after_five_seconds()
    {
        // Arrange
        var (power, pill) = await TestWindow.Run(w =>
        {
            w.C.PowerPreset.Choose(1);

            // Act
            w.Advance(6_000);
            return (w.K.Power, w.C.PowerHint.Text);
        });

        // Assert
        await Assert.That((power, pill)).IsEqualTo((PowerState.Idle, "Idle 5 s → OLEDs off\nSleep 20 s → Bluetooth off"));
    }

    [Test]
    public async Task Never_preset_stays_active()
    {
        // Arrange
        var power = await TestWindow.Run(w =>
        {
            w.C.PowerPreset.Choose(2);

            // Act
            w.Advance(60_000);
            return w.K.Power;
        });

        // Assert
        await Assert.That(power).IsEqualTo(PowerState.Active);
    }

    [Test]
    public async Task Replay_start_up_restarts_the_animation()
    {
        // Arrange
        var events = await TestWindow.Run(w =>
        {
            // Act
            w.C.ReplayStartUp.PerformClick();
            return w.Window.EventText;
        });

        // Assert
        await Assert.That(events).Contains("start-up replayed");
    }

    [Test]
    public async Task Clicking_profile_three_selects_it()
    {
        // Arrange
        var (profile, selected) = await TestWindow.Run(w =>
        {
            // Act
            w.C.Profiles[2].PerformClick();
            return (w.K.Profile, w.C.Profiles[2].Selected);
        });

        // Assert
        await Assert.That((profile, selected)).IsEqualTo((2, true));
    }

    [Test]
    public async Task The_usb_cell_switches_output_to_usb()
    {
        // Arrange
        var transport = await TestWindow.Run(w =>
        {
            // Act
            w.C.Profiles[^1].PerformClick();
            return w.K.Transport;
        });

        // Assert
        await Assert.That(transport).IsEqualTo(Transport.Usb);
    }

    [Test]
    public async Task Unticking_computer_connected_marks_the_profile_paired_only()
    {
        // Arrange
        var (connected, mark) = await TestWindow.Run(w =>
        {
            // Act
            w.C.Connected.IsChecked = false;
            return (w.K.ProfileConnected, w.C.Profiles[w.K.Profile].Mark);
        });

        // Assert
        await Assert.That((connected, mark)).IsEqualTo((false, "✗"));
    }

    [Test]
    public async Task Ticking_open_marks_the_profile_open()
    {
        // Arrange
        var (open, mark) = await TestWindow.Run(w =>
        {
            // Act
            w.C.Open.IsChecked = true;
            return (w.K.ProfileOpen, w.C.Profiles[w.K.Profile].Mark);
        });

        // Assert
        await Assert.That((open, mark)).IsEqualTo((true, "+"));
    }

    [Test]
    public async Task Unticking_right_half_linked_unlinks_it()
    {
        // Arrange
        var linked = await TestWindow.Run(w =>
        {
            // Act
            w.C.Linked.IsChecked = false;
            return w.K.PeripheralConnected;
        });

        // Assert
        await Assert.That(linked).IsFalse();
    }

    [Test]
    public async Task Output_segment_picks_usb()
    {
        // Arrange
        var transport = await TestWindow.Run(w =>
        {
            // Act
            w.C.Output.Choose(0);
            return w.K.Transport;
        });

        // Assert
        await Assert.That(transport).IsEqualTo(Transport.Usb);
    }

    [Test]
    public async Task Picking_a_link_stage_shows_it()
    {
        // Arrange
        var (stage, text) = await TestWindow.Run(w =>
        {
            // Act
            w.C.LinkStages[2]();
            return (w.K.LinkStage, w.C.LinkStage.Text);
        });

        // Assert
        await Assert.That((stage, text)).IsEqualTo((2, EmulatedKeyboard.LinkStages[2] + " ▾"));
    }

    [Test]
    public async Task Simulate_pairing_puts_the_prompt_on_the_screen()
    {
        // Arrange
        var (pairing, events) = await TestWindow.Run(w =>
        {
            // Act
            w.C.SimulatePairing.PerformClick();
            return (w.K.Pairing, w.Window.EventText);
        });

        // Assert
        await Assert.That((pairing, events.Contains("pairing: the computer shows"))).IsEqualTo((true, true));
    }

    [Test]
    public async Task Radio_segment_sets_four_dbm()
    {
        // Arrange
        var (dbm, readout) = await TestWindow.Run(w =>
        {
            // Act
            w.C.Radio.Choose(1);
            return (w.K.RadioDbm, w.C.RadioReadout.Text);
        });

        // Assert
        await Assert.That((dbm, readout)).IsEqualTo((4, "+4"));
    }

    [Test]
    public async Task Send_shows_the_message_on_the_right_half_and_empties_the_field()
    {
        // Arrange
        var (events, field) = await TestWindow.Run(w =>
        {
            // Act
            w.C.Message.Text = "Build passed";
            w.C.Send.PerformClick();
            return (w.Window.EventText, w.C.Message.Text);
        });

        // Assert
        await Assert.That((events.Contains("OS → notification shown on the right half"), field)).IsEqualTo((true, ""));
    }

    [Test]
    public async Task Enter_in_the_message_field_sends_it()
    {
        // Arrange
        var events = await TestWindow.Run(w =>
        {
            // Act
            w.C.Message.Text = "Hello";
            w.C.Message.RaiseEvent(new KeyEventArgs { RoutedEvent = InputElement.KeyDownEvent, Key = Key.Enter, Source = w.C.Message });
            return w.Window.EventText;
        });

        // Assert
        await Assert.That(events).Contains("notification shown");
    }

    [Test]
    public async Task Clear_takes_the_message_away()
    {
        // Arrange
        var events = await TestWindow.Run(w =>
        {
            // Act
            w.C.ClearMessage.PerformClick();
            return w.Window.EventText;
        });

        // Assert
        await Assert.That(events).Contains("OS → notification cleared");
    }

    [Test]
    public async Task Unticking_upright_screens_draws_zmks_screen()
    {
        // Arrange
        var differs = await TestWindow.Run(w =>
        {
            // Act
            w.Advance(3_000); // past the start-up animation
            var before = Pixels(w.K.Displays[0]);
            w.C.Upright.IsChecked = false;
            w.Advance(200);
            return before != Pixels(w.K.Displays[0]);
        });

        // Assert
        await Assert.That(differs).IsTrue();
    }

    private static string Pixels(Kb.Ssd1306.Ssd1306 d)
    {
        var bits = new System.Text.StringBuilder();
        for (var y = 0; y < d.PanelHeight; y++)
        {
            for (var x = 0; x < Kb.Ssd1306.Ssd1306.Columns; x++) bits.Append(d.GetPixel(x, y) ? '1' : '0');
        }
        return bits.ToString();
    }

    [Test]
    public async Task Release_latched_keys_lets_go_of_them()
    {
        // Arrange
        var (latched, ctrl) = await TestWindow.Run(w =>
        {
            // Act
            w.Window.View.LatchForTest(24);
            w.C.ReleaseLatched.PerformClick();
            return (w.Window.View.Latched.Count, w.K.LastReport.Mods);
        });

        // Assert
        await Assert.That((latched, ctrl)).IsEqualTo((0, (byte)0));
    }

    [Test]
    public async Task Clear_output_empties_both_panes()
    {
        // Arrange
        var (host, events) = await TestWindow.Run(w =>
        {
            // Act
            w.Window.RoutePhysicalKey(PhysicalKey.A, true);
            w.Window.RoutePhysicalKey(PhysicalKey.A, false);
            w.C.ClearOutput.PerformClick();
            return (w.Window.HostText, w.Window.EventText);
        });

        // Assert
        await Assert.That((host, events)).IsEqualTo(("", ""));
    }

    [Test]
    public async Task Typing_on_this_pc_reaches_the_host_pane()
    {
        // Arrange
        var host = await TestWindow.Run(w =>
        {
            // Act
            w.Window.KeyPressQwerty(PhysicalKey.H, RawInputModifiers.None);
            w.Window.KeyReleaseQwerty(PhysicalKey.H, RawInputModifiers.None);
            w.Window.KeyPressQwerty(PhysicalKey.I, RawInputModifiers.None);
            w.Window.KeyReleaseQwerty(PhysicalKey.I, RawInputModifiers.None);
            return w.Window.HostText;
        });

        // Assert
        await Assert.That(host).IsEqualTo("hi");
    }

    [Test]
    public async Task Holding_the_symbols_key_shows_the_layer_chip()
    {
        // Arrange
        var layer = await TestWindow.Run(w =>
        {
            // Act
            w.Window.RoutePhysicalKey(PhysicalKey.MetaLeft, true);
            return w.Window.Chips.Layer;
        });

        // Assert
        await Assert.That(layer).IsEqualTo("symbols");
    }

    [Test]
    public async Task Held_chip_lists_the_report()
    {
        // Arrange
        var held = await TestWindow.Run(w =>
        {
            // Act
            w.Window.RoutePhysicalKey(PhysicalKey.A, true);
            return w.Window.Chips.Held;
        });

        // Assert
        await Assert.That(held).IsEqualTo(EmulatedKeyboard.UsageName(0x04));
    }

    [Test]
    public async Task Wpm_counts_recent_key_presses()
    {
        // Arrange
        var wpm = await TestWindow.Run(w =>
        {
            foreach (var key in new[] { PhysicalKey.A, PhysicalKey.S, PhysicalKey.D, PhysicalKey.F, PhysicalKey.G,
                                        PhysicalKey.H, PhysicalKey.J, PhysicalKey.K, PhysicalKey.L, PhysicalKey.Q })
            {
                w.Window.RoutePhysicalKey(key, true);
                w.Window.RoutePhysicalKey(key, false);
            }

            // Act: the chip refreshes every 100 ticks
            for (var i = 0; i < 100; i++) w.Window.Tick();
            return w.Window.Chips.Wpm;
        });

        // Assert: 10 presses in 10 s = 2 words in 10 s = 12 WPM
        await Assert.That(wpm).IsEqualTo("12");
    }
}

/// <summary>The power switch, USB port and reset button drawn on the keyboard.</summary>
[NotInParallel("ui-session")]
public class HardwareControlTests
{
    [Test]
    public async Task Switching_the_right_half_off_unlinks_it()
    {
        // Arrange
        var (linked, events) = await TestWindow.Run(w =>
        {
            // Act
            w.Window.View.FlipPower(1);
            w.Advance(100);
            return (w.K.PeripheralConnected, w.Window.EventText);
        });

        // Assert
        await Assert.That((linked, events.Contains("right half switched off"))).IsEqualTo((false, true));
    }

    [Test]
    public async Task Plugging_usb_into_the_left_half_shows_it_charging()
    {
        // Arrange
        var (label, events) = await TestWindow.Run(w =>
        {
            // Act
            w.Window.View.PlugUsb(0);
            return (w.Window.View.IsUsbPlugged(0), w.Window.EventText);
        });

        // Assert
        await Assert.That((label, events.Contains("USB plugged into the left half"))).IsEqualTo((true, true));
    }

    [Test]
    public async Task Reset_restarts_the_half()
    {
        // Arrange
        var events = await TestWindow.Run(w =>
        {
            // Act
            w.Window.View.PressReset(0);
            return w.Window.EventText;
        });

        // Assert
        await Assert.That(events).Contains("left half reset");
    }

    [Test]
    public async Task Clicking_the_drawn_power_switch_flips_it()
    {
        // Arrange
        var on = await TestWindow.Run(w =>
        {
            // Act
            var view = w.Window.View;
            var at = view.TranslatePoint(CorneView.ControlCenters(0).Power, w.Window)!.Value;
            w.Window.MouseDown(at, MouseButton.Left);
            w.Window.MouseUp(at, MouseButton.Left);
            return view.IsOn(0);
        });

        // Assert
        await Assert.That(on).IsFalse();
    }

    [Test]
    public async Task Clicking_a_drawn_key_presses_it()
    {
        // Arrange
        var typed = await TestWindow.Run(w =>
        {
            // Act
            var view = w.Window.View;
            var center = view.KeyCenter(13);
            var at = view.TranslatePoint(center, w.Window)!.Value;
            w.Window.MouseDown(at, MouseButton.Left);
            var down = w.K.LastReport.Contains(0x04);
            w.Window.MouseUp(at, MouseButton.Left);
            return down;
        });

        // Assert
        await Assert.That(typed).IsTrue();
    }

    [Test]
    public async Task Right_clicking_a_drawn_key_latches_it()
    {
        // Arrange
        var latched = await TestWindow.Run(w =>
        {
            // Act
            var view = w.Window.View;
            var at = view.TranslatePoint(view.KeyCenter(24), w.Window)!.Value;
            w.Window.MouseDown(at, MouseButton.Right);
            w.Window.MouseUp(at, MouseButton.Right);
            return view.Latched.Contains(24);
        });

        // Assert
        await Assert.That(latched).IsTrue();
    }
}

/// <summary>The top bar's theme switch.</summary>
[NotInParallel("ui-session")]
public class ThemeSwitchTests
{
    [Test]
    public async Task Choosing_dark_in_the_top_bar_applies_and_saves_it()
    {
        // Arrange
        var (variant, saved) = await TestWindow.Run(w =>
        {
            var path = Environment.GetEnvironmentVariable("EMK_SETTINGS")!;

            // Act
            w.Window.Bar.ThemeSwitch.Choose(2);
            var v = Application.Current!.ActualThemeVariant;
            var s = AppSettings.Load(path).Theme;
            w.Window.Bar.ThemeSwitch.Choose(0); // back to System for the other tests
            return (v, s);
        });

        // Assert
        await Assert.That((variant.ToString(), saved)).IsEqualTo(("Dark", ThemePreference.Dark));
    }
}

/// <summary>kbnotify reaching the window's own socket.</summary>
[NotInParallel("ui-session")]
public class KbnotifyWindowTests
{
    [Test]
    public async Task A_kbnotify_message_shows_on_the_right_half()
    {
        // Arrange: the halves have linked
        var w = await TestAppBuilder.Session.Dispatch(() =>
        {
            var t = new TestWindow();
            t.Advance(3_000);
            return t;
        }, CancellationToken.None);

        // Act
        await using (var link = await Kb.HostProtocol.EmulatorLink.ConnectAsync(w.Socket))
        {
            foreach (var packet in Kb.HostProtocol.NotificationPackets.Encode("Build passed")) await link.SendAsync(packet);
        }

        // Assert
        var until = DateTime.UtcNow + TimeSpan.FromSeconds(10);
        string events;
        do
        {
            await Task.Delay(20);
            events = await TestAppBuilder.Session.Dispatch(() =>
            {
                Avalonia.Threading.Dispatcher.UIThread.RunJobs();
                w.Advance(50);
                return w.Window.EventText;
            }, CancellationToken.None);
        } while (!events.Contains("kbnotify →") && DateTime.UtcNow < until);
        await TestAppBuilder.Session.Dispatch(w.Window.Close, CancellationToken.None);
        await Assert.That(events).Contains("kbnotify → notification shown on the right half");
    }
}
