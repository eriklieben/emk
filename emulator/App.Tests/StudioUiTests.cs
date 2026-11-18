using System.Diagnostics;
using Avalonia.Controls;
using Avalonia.Headless;
using Avalonia.Input;
using Avalonia.LogicalTree;
using KbEmulator.Bench;

namespace KbEmulator.Tests;

/// <summary>Studio's editor on the emulator target, used through its controls.</summary>
[NotInParallel("ui-session")]
public class StudioEditorTests
{
    private static HeadlessUnitTestSession Session => TestAppBuilder.Session;

    /// <summary>A Studio on a fresh emulator with its own storage file; unlocked when asked.</summary>
    private static Task<T> Run<T>(bool unlocked, Func<StudioView, EmulatedKeyboard, T> act) => Session.Dispatch(() =>
    {
        using var keyboard = new EmulatedKeyboard(() => 0);
        keyboard.SetStudioStorage(Path.Combine(Path.GetTempPath(), $"emk-studio-{Guid.NewGuid():N}", "keymap.txt"));
        var studio = new StudioView(keyboard);
        studio.Activate();
        studio.Pump();
        if (unlocked)
        {
            studio.UnlockButton.PerformClick();
            studio.Pump();
        }
        return act(studio, keyboard);
    }, CancellationToken.None);

    [Test]
    public async Task Unlock_button_unlocks_the_emulator()
    {
        // Arrange
        var unlocked = await Run(false, (s, _) =>
        {
            // Act
            s.UnlockButton.PerformClick();
            s.Pump();
            return s.Client.Unlocked;
        });

        // Assert
        await Assert.That(unlocked).IsTrue();
    }

    [Test]
    public async Task Apply_changes_the_key_and_its_legend()
    {
        // Arrange
        var (binding, label) = await Run(true, (s, k) =>
        {
            s.Select(0, 13);
            s.EditForTest(e => e.Mods.Add("LC"));

            // Act
            s.Apply();
            s.Pump();
            return (s.Client.Binding(0, 13), k.Label(13));
        });

        // Assert
        await Assert.That((binding, label)).IsEqualTo(("kp:LC(A)", StudioCodec.Label("kp:LC(A)")));
    }

    [Test]
    public async Task Apply_while_locked_shows_the_refusal()
    {
        // Arrange
        var message = await Run(false, (s, _) =>
        {
            s.Select(0, 13);
            s.EditForTest(e => e.Key = "B");

            // Act
            s.Apply();
            s.Pump();
            return s.Message;
        });

        // Assert
        await Assert.That(message).StartsWith("set err locked");
    }

    [Test]
    public async Task Check_shows_the_lockout_guard()
    {
        // Arrange
        var message = await Run(true, (s, _) =>
        {
            s.Select(0, 41);
            s.EditForTest(e =>
            {
                e.Kind = StudioKind.Key;
                e.Key = "A";
                e.Mods.Clear();
            });

            // Act
            s.Check();
            s.Pump();
            return s.Message;
        });

        // Assert
        await Assert.That(message).StartsWith("check err lockout");
    }

    [Test]
    public async Task Apply_of_a_lockout_is_refused_by_the_keyboard()
    {
        // Arrange
        var message = await Run(true, (s, _) =>
        {
            s.Select(2, 35);
            s.EditForTest(e => e.Kind = StudioKind.Transparent);

            // Act
            s.Apply();
            s.Pump();
            return s.Message;
        });

        // Assert
        await Assert.That(message).StartsWith("set err lockout");
    }

    [Test]
    public async Task Revert_key_to_default_puts_the_compiled_binding_back()
    {
        // Arrange
        var binding = await Run(true, (s, _) =>
        {
            s.Select(0, 13);
            s.EditForTest(e => e.Key = "B");
            s.Apply();
            s.Pump();

            // Act
            s.RevertKey();
            s.Pump();
            return s.Client.Binding(0, 13);
        });

        // Assert
        await Assert.That(binding).IsEqualTo("kp:A");
    }

    [Test]
    public async Task Save_to_flash_clears_the_unsaved_state()
    {
        // Arrange
        var dirty = await Run(true, (s, _) =>
        {
            s.Select(0, 13);
            s.EditForTest(e => e.Key = "B");
            s.Apply();
            s.Pump();

            // Act
            s.SaveButton.PerformClick();
            s.Pump();
            return s.Client.Dirty;
        });

        // Assert
        await Assert.That(dirty).IsFalse();
    }

    [Test]
    public async Task Revert_goes_back_to_what_was_saved()
    {
        // Arrange
        var binding = await Run(true, (s, _) =>
        {
            s.Select(0, 13);
            s.EditForTest(e => e.Key = "B");
            s.Apply();
            s.Pump();

            // Act
            s.RevertButton.PerformClick();
            s.Pump();
            return s.Client.Binding(0, 13);
        });

        // Assert
        await Assert.That(binding).IsEqualTo("kp:A");
    }

    [Test]
    public async Task Reset_all_needs_a_second_click()
    {
        // Arrange
        var (afterFirst, afterSecond) = await Run(true, (s, _) =>
        {
            s.Select(0, 13);
            s.EditForTest(e => e.Key = "B");
            s.Apply();
            s.Pump();
            s.ResetButton.PerformClick();
            s.Pump();
            var first = s.Client.Binding(0, 13);

            // Act
            s.ResetButton.PerformClick();
            s.Pump();
            return (first, s.Client.Binding(0, 13));
        });

        // Assert
        await Assert.That((afterFirst, afterSecond)).IsEqualTo(("kp:B", "kp:A"));
    }

    [Test]
    public async Task Esc_disarms_reset_all()
    {
        // Arrange
        var (closed, text) = await Run(true, (s, _) =>
        {
            s.ResetButton.PerformClick();

            // Act
            var c = s.Escape();
            return (c, s.ResetButton.Text);
        });

        // Assert
        await Assert.That((closed, text)).IsEqualTo((true, "Reset all to default…"));
    }

    [Test]
    public async Task Lock_locks_studio_again()
    {
        // Arrange
        var unlocked = await Run(true, (s, _) =>
        {
            // Act
            s.LockButton.PerformClick();
            s.Pump();
            return s.Client.Unlocked;
        });

        // Assert
        await Assert.That(unlocked).IsFalse();
    }

    [Test]
    public async Task Changed_keys_are_marked_on_the_keyboard()
    {
        // Arrange
        var changed = await Run(true, (s, _) =>
        {
            s.Select(0, 13);
            s.EditForTest(e => e.Key = "B");

            // Act
            s.Apply();
            s.Pump();
            return s.View.Changed.ToArray();
        });

        // Assert
        await Assert.That(changed.Select((c, i) => c ? i : -1).Where(i => i >= 0)).IsEquivalentTo(new[] { 13 });
    }

    [Test]
    public async Task Mod_tap_edit_encodes_hold_and_tap()
    {
        // Arrange
        var encoding = await Run(false, (s, _) =>
        {
            s.Select(0, 27);

            // Act
            s.EditForTest(e =>
            {
                e.Kind = StudioKind.ModTap;
                e.Hold = "LCTRL";
            });
            return s.Edit.Encode();
        });

        // Assert
        await Assert.That(encoding).IsEqualTo("mt:LCTRL/C");
    }

    [Test]
    public async Task Keyboard_target_without_a_keyboard_says_so()
    {
        // Arrange
        var support = await Session.Dispatch(() =>
        {
            using var keyboard = new EmulatedKeyboard(() => 0);
            var debug = new DebugView(_ => "base", 42, new HardwarePort(() => null));
            var studio = new StudioView(keyboard, debug);

            // Act
            studio.SelectTarget(1);
            var s = studio.Client.Support;
            debug.Dispose();
            return s;
        }, CancellationToken.None);

        // Assert
        await Assert.That(support).IsEqualTo(StudioSupport.NotConnected);
    }

    [Test]
    public async Task Selecting_a_layer_shows_its_legends()
    {
        // Arrange
        var keyText = await Run(false, (s, _) =>
        {
            // Act
            s.Select(1, 1);
            return s.KeyText;
        });

        // Assert
        await Assert.That(keyText).Contains("!");
    }
}

/// <summary>Flash's run panel used through its buttons, with fake scripts.</summary>
[NotInParallel("ui-session")]
public class FlashUiTests
{
    private static HeadlessUnitTestSession Session => TestAppBuilder.Session;

    private static FlashView View(FakeRepo repo, string script) => new(new DeviceSources
    {
        Repo = repo.Root, KeyPath = Path.Combine(repo.Root, "no.pem"), UsbRoot = repo.Root, LogPort = () => null,
    })
    {
        JobFactory = (r, k, side, b) => new FlashJob(r, k, side, b, _ => new ProcessStartInfo("/bin/bash", [FakeScripts.Write(script)])),
    };

    private static async Task WaitUntil(Func<bool> done)
    {
        var until = DateTime.UtcNow + TimeSpan.FromSeconds(10);
        while (!await Session.Dispatch(() =>
               {
                   Avalonia.Threading.Dispatcher.UIThread.RunJobs();
                   return done();
               }, CancellationToken.None))
        {
            if (DateTime.UtcNow > until) throw new TimeoutException();
            await Task.Delay(20);
        }
    }

    [Test]
    public async Task Esc_closes_the_confirm_panel()
    {
        // Arrange
        using var repo = new FakeRepo().Halves("kb_b left 0123456789ABCDEF\n");
        var view = await Session.Dispatch(() =>
        {
            var v = View(repo, "true");
            v.Refresh();
            v.RequestFlash("left");
            return v;
        }, CancellationToken.None);

        // Act
        var (closed, confirm) = await Session.Dispatch(() => (view.Escape(), view.ConfirmSide), CancellationToken.None);

        // Assert
        await Assert.That((closed, confirm)).IsEqualTo((true, (string?)null));
    }

    [Test]
    public async Task Cancel_in_the_run_panel_stops_hil()
    {
        // Arrange
        using var repo = new FakeRepo().Halves("kb_b right FEDCBA9876543210\n");
        var view = await Session.Dispatch(() =>
        {
            var v = View(repo, "echo started; sleep 30");
            v.Refresh();
            v.RequestFlash("right");
            v.ConfirmFlash();
            return v;
        }, CancellationToken.None);
        await WaitUntil(() => view.JobLines.Contains("started"));

        // Act
        await Session.Dispatch(view.CancelFlash, CancellationToken.None);

        // Assert
        await WaitUntil(() => view.Job is { Running: false });
        var state = await Session.Dispatch(() => view.Job!.State, CancellationToken.None);
        await Assert.That(state).IsEqualTo(FlashState.Cancelled);
    }

    [Test]
    public async Task Close_dismisses_a_finished_run()
    {
        // Arrange
        using var repo = new FakeRepo().Halves("kb_b right FEDCBA9876543210\n");
        var view = await Session.Dispatch(() =>
        {
            var v = View(repo, "echo 'flashed x.uf2'");
            v.Refresh();
            v.RequestFlash("right");
            v.ConfirmFlash();
            return v;
        }, CancellationToken.None);
        await WaitUntil(() => view.Job is { Running: false });

        // Act
        var job = await Session.Dispatch(() =>
        {
            view.Dismiss();
            return view.Job;
        }, CancellationToken.None);

        // Assert
        await Assert.That(job).IsNull();
    }

    [Test]
    public async Task The_scripts_output_is_shown_in_the_panel()
    {
        // Arrange
        using var repo = new FakeRepo().Halves("kb_b right FEDCBA9876543210\n");
        var view = await Session.Dispatch(() =>
        {
            var v = View(repo, "echo 'hil: no NICENANO drive appeared' >&2; exit 1");
            v.Refresh();
            v.RequestFlash("right");
            return v;
        }, CancellationToken.None);

        // Act
        await Session.Dispatch(() => view.ConfirmFlash(), CancellationToken.None);

        // Assert
        await WaitUntil(() => view.Job is { Running: false });
        var (lines, detail) = await Session.Dispatch(() => (view.JobLines.ToList(), view.Job!.Detail), CancellationToken.None);
        await Assert.That((lines.Contains("hil: no NICENANO drive appeared"), detail)).IsEqualTo((true, "hil: no NICENANO drive appeared"));
    }
}

/// <summary>The Bench widgets on their own.</summary>
[NotInParallel("ui-session")]
public class WidgetTests
{
    private static HeadlessUnitTestSession Session => TestAppBuilder.Session;

    private static KeyEventArgs KeyDown(Control source, Key key) => new() { RoutedEvent = InputElement.KeyDownEvent, Key = key, Source = source };

    [Test]
    public async Task Enter_activates_a_button()
    {
        // Arrange
        var clicks = await Session.Dispatch(() =>
        {
            var n = 0;
            var button = new BenchButton("Go");
            button.Click += () => n++;

            // Act
            button.RaiseEvent(KeyDown(button, Key.Enter));
            return n;
        }, CancellationToken.None);

        // Assert
        await Assert.That(clicks).IsEqualTo(1);
    }

    [Test]
    public async Task A_disabled_button_does_nothing()
    {
        // Arrange
        var clicks = await Session.Dispatch(() =>
        {
            var n = 0;
            var button = new BenchButton("Go") { Enabled = false };
            button.Click += () => n++;

            // Act
            button.PerformClick();
            return n;
        }, CancellationToken.None);

        // Assert
        await Assert.That(clicks).IsEqualTo(0);
    }

    [Test]
    public async Task A_latching_button_toggles()
    {
        // Arrange
        var latched = await Session.Dispatch(() =>
        {
            var button = new BenchButton("Mute", latching: true);

            // Act
            button.PerformClick();
            return button.IsLatched;
        }, CancellationToken.None);

        // Assert
        await Assert.That(latched).IsTrue();
    }

    [Test]
    public async Task Arrow_right_picks_the_next_segment()
    {
        // Arrange
        var picked = await Session.Dispatch(() =>
        {
            var segment = new Segmented(["A", "B", "C"]);
            var chosen = -1;
            segment.SelectionChanged += i => chosen = i;

            // Act
            segment.RaiseEvent(KeyDown(segment, Key.Right));
            return chosen;
        }, CancellationToken.None);

        // Assert
        await Assert.That(picked).IsEqualTo(1);
    }

    [Test]
    public async Task Setting_a_segment_in_code_raises_nothing()
    {
        // Arrange
        var raised = await Session.Dispatch(() =>
        {
            var segment = new Segmented(["A", "B"]);
            var n = 0;
            segment.SelectionChanged += _ => n++;

            // Act
            segment.SelectedIndex = 1;
            return n;
        }, CancellationToken.None);

        // Assert
        await Assert.That(raised).IsEqualTo(0);
    }

    [Test]
    public async Task Space_ticks_a_toggle()
    {
        // Arrange
        var on = await Session.Dispatch(() =>
        {
            var toggle = new Toggle("Follow");

            // Act
            toggle.RaiseEvent(KeyDown(toggle, Key.Space));
            return toggle.IsChecked;
        }, CancellationToken.None);

        // Assert
        await Assert.That(on).IsTrue();
    }

    [Test]
    public async Task Arrow_up_raises_a_gauge_by_ten()
    {
        // Arrange
        var value = await Session.Dispatch(() =>
        {
            var gauge = new Gauge { Value = 50 };

            // Act
            gauge.RaiseEvent(KeyDown(gauge, Key.Up));
            return gauge.Value;
        }, CancellationToken.None);

        // Assert
        await Assert.That(value).IsEqualTo(60);
    }

    [Test]
    [Arguments(LampState.Good, "lamp-good")]
    [Arguments(LampState.Warn, "lamp-warn")]
    [Arguments(LampState.Crit, "lamp-crit")]
    [Arguments(LampState.Accent, "acc")]
    [Arguments(LampState.Off, "lamp-off")]
    public async Task Lamps_use_the_semantic_colours(LampState state, string token)
    {
        // Arrange
        var lamp = await Session.Dispatch(() => new Lamp("x"), CancellationToken.None);

        // Act
        var used = await Session.Dispatch(() =>
        {
            lamp.State = state;
            return lamp.Token;
        }, CancellationToken.None);

        // Assert
        await Assert.That(used).IsEqualTo(token);
    }

    [Test]
    public async Task A_log_well_keeps_only_its_newest_lines()
    {
        // Arrange
        var well = await Session.Dispatch(() => new LogWell(maxLines: 10), CancellationToken.None);

        // Act
        var (count, first) = await Session.Dispatch(() =>
        {
            for (var i = 0; i < 25; i++) well.Add($"line {i}");
            return (well.Count, well.Text.Split('\n')[0]);
        }, CancellationToken.None);

        // Assert
        await Assert.That((count, first)).IsEqualTo((10, "line 15"));
    }

    [Test]
    public async Task Chip_panel_without_a_measurement_says_how_to_get_one()
    {
        // Arrange: no size report
        var text = await Session.Dispatch(() =>
        {
            // Act
            var panel = new ChipPanel(null);
            return string.Join(" ", panel.Children.OfType<TextBlock>().Select(t => t.Text));
        }, CancellationToken.None);

        // Assert
        await Assert.That(text).Contains("run ./size-report");
    }
}

/// <summary>Keyboard navigation outside the Emulator (where keys go to the Corne).</summary>
[NotInParallel("ui-session")]
public class FocusTests
{
    [Test]
    public async Task Tab_moves_focus_between_controls_in_flash()
    {
        // Arrange
        var moved = await TestWindow.Run(w =>
        {
            w.Window.ShowMode(AppMode.Flash);
            Avalonia.Threading.Dispatcher.UIThread.RunJobs();
            w.Window.Bar.Modes.Focus(NavigationMethod.Tab);
            var before = w.Window.FocusManager?.GetFocusedElement();

            // Act
            w.Window.KeyPress(Key.Tab, RawInputModifiers.None, PhysicalKey.Tab, null);
            return before != w.Window.FocusManager?.GetFocusedElement();
        });

        // Assert
        await Assert.That(moved).IsTrue();
    }

    [Test]
    public async Task Tab_in_the_emulator_is_the_corne_key()
    {
        // Arrange
        var typed = await TestWindow.Run(w =>
        {
            // Act
            w.Window.KeyPress(Key.Tab, RawInputModifiers.None, PhysicalKey.Tab, null);
            return w.K.LastReport.Contains(0x2B) || w.Window.View.PressedHere.Contains(0);
        });

        // Assert
        await Assert.That(typed).IsTrue();
    }

    [Test]
    public async Task Enter_in_the_emulator_does_not_fire_a_focused_button()
    {
        // Arrange
        var events = await TestWindow.Run(w =>
        {
            w.C.ReplayStartUp.Focus();

            // Act
            w.Window.KeyPress(Key.Enter, RawInputModifiers.None, PhysicalKey.Enter, null);
            return w.Window.EventText;
        });

        // Assert
        await Assert.That(events).DoesNotContain("start-up replayed");
    }

    [Test]
    public async Task Enter_on_a_focused_button_in_flash_fires_it()
    {
        // Arrange
        var confirm = await TestWindow.Run(w =>
        {
            w.Window.ShowMode(AppMode.Flash);
            Avalonia.Threading.Dispatcher.UIThread.RunJobs();
            var button = w.Window.Flash.GetLogicalDescendants().OfType<BenchButton>().First(b => b.Text == "Flash left half");
            button.Focus(NavigationMethod.Tab);

            // Act
            w.Window.KeyPress(Key.Enter, RawInputModifiers.None, PhysicalKey.Enter, null);
            return w.Window.Flash.ConfirmSide;
        });

        // Assert
        await Assert.That(confirm).IsEqualTo("left");
    }

    [Test]
    [Arguments("light")]
    [Arguments("dark")]
    public async Task Keyboard_focus_shows_a_ring(string theme)
    {
        // Arrange
        var path = Path.Combine(AppContext.BaseDirectory, $"main-window-focus-{theme}.png");
        var focused = await TestWindow.Run(w =>
        {
            Avalonia.Application.Current!.RequestedThemeVariant = theme == "light" ? Avalonia.Styling.ThemeVariant.Light : Avalonia.Styling.ThemeVariant.Dark;
            w.Window.ShowMode(AppMode.Security);
            Avalonia.Threading.Dispatcher.UIThread.RunJobs();
            var open = w.Window.Security.GetLogicalDescendants().OfType<BenchButton>().First(b => b.Text == "Open Flash");

            // Act
            open.Focus(NavigationMethod.Tab);
            Avalonia.Threading.Dispatcher.UIThread.RunJobs();
            var size = new Avalonia.PixelSize((int)w.Window.Bounds.Width, (int)w.Window.Bounds.Height);
            using var bitmap = new Avalonia.Media.Imaging.RenderTargetBitmap(size);
            bitmap.Render(w.Window);
            using (var file = File.Create(path)) bitmap.Save(file, Avalonia.Media.Imaging.PngBitmapEncoderOptions.Default);
            Avalonia.Application.Current.RequestedThemeVariant = Avalonia.Styling.ThemeVariant.Default;
            return open.BoxShadow.Count > 0 && Math.Abs(open.BoxShadow[0].Spread - 2) < 0.001;
        });

        // Assert
        await Assert.That(focused).IsTrue();
    }

    [Test]
    public async Task Esc_in_the_window_closes_flashs_confirm_panel()
    {
        // Arrange
        var confirm = await TestWindow.Run(w =>
        {
            w.Window.ShowMode(AppMode.Flash);
            w.Window.Flash.RequestFlash("left");

            // Act
            w.Window.KeyPress(Key.Escape, RawInputModifiers.None, PhysicalKey.Escape, null);
            return w.Window.Flash.ConfirmSide;
        });

        // Assert
        await Assert.That(confirm).IsNull();
    }
}
