//! Behavior tests against the real Corne keymap, in Arrange / Act / Assert
//! form. Arrange drives the keyboard into the state under test and clears
//! the log, Act performs one user action, Assert checks what the host saw.
//!
//! Log format: `+A` / `-A` = usage pressed / released, `layer:nav`,
//! `caps_word:true`, or a system action like `BtClear`.

use kb_core::Direction::{Down, Left, Right, Up};
use kb_core::action::{MOUSE_PRECISION, kp, mkey, mkp, mmv, scroll};
use kb_core::keymaps::corne;
use kb_core::report::diff;
use kb_core::{Config, Engine, Output, Report, keys};

// Positions on the Corne (see keymaps/corne.rs).
const POS_Q: u8 = 1;
const POS_T: u8 = 5;
const POS_Y: u8 = 6;
const POS_ALT_TAB: u8 = 12;
const POS_A: u8 = 13;
const POS_W: u8 = 2;
const POS_E: u8 = 3;
const POS_R: u8 = 4;
const POS_S: u8 = 14;
const POS_D: u8 = 15;
const POS_F: u8 = 16;
const POS_G: u8 = 17;
const POS_J: u8 = 19;
const POS_K: u8 = 20;
const POS_Z: u8 = 25;
const POS_X: u8 = 26;
const POS_C: u8 = 27;
const POS_B: u8 = 29;
const POS_LOWER: u8 = 36;
const POS_BSPC: u8 = 37;
const POS_SPACE: u8 = 38;
const POS_DEL: u8 = 40;
const POS_RAISE: u8 = 41;

const NOTHING: [&str; 0] = [];

struct Kb {
    engine: Engine<'static, { corne::KEYS }>,
    now: u32,
    host: Report,
    log: Vec<String>,
}

impl Kb {
    fn new() -> Self {
        Self {
            engine: Engine::new(&corne::KEYMAP, Config::default()),
            now: 0,
            host: Report::default(),
            log: Vec::new(),
        }
    }

    fn at(&mut self, t: u32) -> &mut Self {
        assert!(t >= self.now, "time goes forward");
        self.now = t;
        self
    }

    fn down(&mut self, pos: u8) -> &mut Self {
        self.engine.press(pos, self.now);
        self.collect()
    }

    fn up(&mut self, pos: u8) -> &mut Self {
        self.engine.release(pos, self.now);
        self.collect()
    }

    /// Press and release `pos`, holding it for `ms`.
    fn tap(&mut self, pos: u8, ms: u32) -> &mut Self {
        self.down(pos);
        self.wait(ms);
        self.up(pos)
    }

    /// Let time pass, firing timeouts.
    fn wait(&mut self, ms: u32) -> &mut Self {
        self.now += ms;
        self.engine.tick(self.now);
        self.collect()
    }

    /// Forget everything logged so far (end of Arrange).
    fn clear(&mut self) -> &mut Self {
        self.log.clear();
        self
    }

    fn collect(&mut self) -> &mut Self {
        while let Some(out) = self.engine.poll() {
            match out {
                Output::Report(r) => {
                    let log = &mut self.log;
                    diff(&self.host, &r, |usage, pressed| {
                        let sign = if pressed { '+' } else { '-' };
                        log.push(format!("{sign}{}", keys::name(usage)));
                    });
                    self.host = r;
                }
                Output::Layer(l) => {
                    let name = corne::KEYMAP.layer_names[l as usize];
                    self.log.push(format!("layer:{name}"));
                }
                Output::CapsWord(on) => self.log.push(format!("caps_word:{on}")),
                Output::System(s) => self.log.push(format!("{s:?}")),
                Output::Mouse(m, down) => {
                    let sign = if down { '+' } else { '-' };
                    self.log.push(format!("{sign}{m:?}"));
                }
                Output::Pointer(on) => self.log.push(format!("pointer:{on}")),
            }
        }
        self
    }

    fn log(&self) -> &[String] {
        &self.log
    }
}

// ---- plain keys --------------------------------------------------------------

#[test]
fn tapping_a_key_sends_press_then_release() {
    // Arrange
    let mut kb = Kb::new();

    // Act
    kb.tap(POS_A, 30);

    // Assert
    assert_eq!(kb.log(), ["+A", "-A"]);
}

// ---- mod-tap: &mt LALT TAB ---------------------------------------------------

#[test]
fn mod_tap_sends_nothing_while_undecided() {
    // Arrange
    let mut kb = Kb::new();

    // Act
    kb.down(POS_ALT_TAB).wait(199);

    // Assert
    assert_eq!(kb.log(), NOTHING);
}

#[test]
fn mod_tap_released_within_tapping_term_sends_tab() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_ALT_TAB);

    // Act
    kb.at(100).up(POS_ALT_TAB);

    // Assert
    assert_eq!(kb.log(), ["+TAB", "-TAB"]);
}

#[test]
fn mod_tap_held_for_tapping_term_sends_alt() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_ALT_TAB).wait(199);

    // Act
    kb.wait(1);

    // Assert
    assert_eq!(kb.log(), ["+LALT"]);
}

#[test]
fn releasing_held_mod_tap_releases_alt() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_ALT_TAB).wait(200).clear();

    // Act
    kb.at(500).up(POS_ALT_TAB);

    // Assert
    assert_eq!(kb.log(), ["-LALT"]);
}

#[test]
fn mod_tap_is_hold_preferred_when_another_key_is_pressed() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_ALT_TAB);

    // Act
    kb.at(30).down(POS_A);

    // Assert
    assert_eq!(kb.log(), ["+LALT", "+A"]);
}

// ---- thumb keys ------------------------------------------------------------------

#[test]
fn backspace_thumb_sends_backspace() {
    // Arrange
    let mut kb = Kb::new();

    // Act
    kb.tap(POS_BSPC, 300);

    // Assert: a plain key, even when held long
    assert_eq!(kb.log(), ["+BSPC", "-BSPC"]);
}

#[test]
fn delete_thumb_sends_delete() {
    // Arrange
    let mut kb = Kb::new();

    // Act
    kb.tap(POS_DEL, 300);

    // Assert
    assert_eq!(kb.log(), ["+DEL", "-DEL"]);
}

// ---- layers --------------------------------------------------------------------

#[test]
fn holding_lower_activates_symbols_layer() {
    // Arrange
    let mut kb = Kb::new();

    // Act
    kb.down(POS_LOWER);

    // Assert
    assert_eq!(kb.log(), ["layer:symbols"]);
}

#[test]
fn releasing_lower_returns_to_default_layer() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_LOWER).clear();

    // Act
    kb.up(POS_LOWER);

    // Assert
    assert_eq!(kb.log(), ["layer:default"]);
}

#[test]
fn symbols_layer_sends_digit() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_LOWER).clear();

    // Act
    kb.tap(POS_A, 20);

    // Assert
    assert_eq!(kb.log(), ["+N1", "-N1"]);
}

#[test]
fn shifted_symbol_is_sent_with_implicit_shift() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_LOWER).clear();

    // Act: `!` = Shift+1
    kb.tap(POS_Q, 20);

    // Assert
    assert_eq!(kb.log(), ["+LSHFT", "+N1", "-N1", "-LSHFT"]);
}

#[test]
fn transparent_key_falls_through_to_default_layer() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_LOWER).clear();

    // Act
    kb.tap(POS_ALT_TAB, 50);

    // Assert
    assert_eq!(kb.log(), ["+TAB", "-TAB"]);
}

#[test]
fn release_uses_the_layer_from_press_time() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_LOWER).down(POS_A).up(POS_LOWER).clear();

    // Act
    kb.up(POS_A);

    // Assert
    assert_eq!(kb.log(), ["-N1"]);
}

#[test]
fn nav_layer_d_position_moves_the_mouse_down() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_RAISE).clear();

    // Act
    kb.tap(POS_D, 20);

    // Assert: the pointer comes alive with the first move
    assert_eq!(kb.log(), ["pointer:true", "+Move(Down)", "-Move(Down)"]);
}

#[test]
fn nav_layer_sdfg_move_left_down_up_right() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_RAISE);

    // Act
    let actions: Vec<_> = [POS_S, POS_D, POS_F, POS_G]
        .iter()
        .map(|&p| kb.engine.binding(p))
        .collect();

    // Assert
    assert_eq!(actions, [mmv(Left), mmv(Down), mmv(Up), mmv(Right)]);
}

#[test]
fn nav_layer_wert_are_f2_to_f5_or_scroll_and_q_stays_f1() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_RAISE);

    // Act
    let actions: Vec<_> = [POS_Q, POS_W, POS_E, POS_R, POS_T]
        .iter()
        .map(|&p| kb.engine.binding(p))
        .collect();

    // Assert
    assert_eq!(
        actions,
        [
            kp(keys::F1),
            mkey(scroll(Left), keys::F2),
            mkey(scroll(Down), keys::F3),
            mkey(scroll(Up), keys::F4),
            mkey(scroll(Right), keys::F5)
        ]
    );
}

#[test]
fn nav_wert_type_f2_to_f5_before_the_mouse_moves() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_RAISE).clear();

    // Act
    kb.tap(POS_W, 20)
        .tap(POS_E, 20)
        .tap(POS_R, 20)
        .tap(POS_T, 20);

    // Assert
    assert_eq!(
        kb.log(),
        ["+F2", "-F2", "+F3", "-F3", "+F4", "-F4", "+F5", "-F5"]
    );
}

#[test]
fn nav_wert_scroll_once_the_mouse_moved() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_RAISE).tap(POS_S, 30).clear();

    // Act
    kb.tap(POS_W, 20)
        .tap(POS_E, 20)
        .tap(POS_R, 20)
        .tap(POS_T, 20);

    // Assert
    assert_eq!(
        kb.log(),
        [
            "+Scroll(Left)",
            "-Scroll(Left)",
            "+Scroll(Down)",
            "-Scroll(Down)",
            "+Scroll(Up)",
            "-Scroll(Up)",
            "+Scroll(Right)",
            "-Scroll(Right)"
        ]
    );
}

#[test]
fn nav_again_after_the_pointer_ended_types_f2() {
    // Arrange: moved, nav let go, nav again
    let mut kb = Kb::new();
    kb.down(POS_RAISE)
        .tap(POS_S, 30)
        .up(POS_RAISE)
        .down(POS_RAISE)
        .clear();

    // Act
    kb.tap(POS_W, 20);

    // Assert
    assert_eq!(kb.log(), ["+F2", "-F2"]);
}

#[test]
fn scroll_held_across_letting_nav_go_releases_as_a_scroll() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_RAISE)
        .tap(POS_S, 30)
        .down(POS_E)
        .up(POS_RAISE)
        .clear();

    // Act
    kb.up(POS_E);

    // Assert
    assert_eq!(kb.log(), ["-Scroll(Down)"]);
}

#[test]
fn nav_layer_tab_is_precision_and_a_is_middle_click() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_RAISE);

    // Act
    let actions = [kb.engine.binding(POS_ALT_TAB), kb.engine.binding(POS_A)];

    // Assert
    assert_eq!(
        actions,
        [MOUSE_PRECISION, mkp(kb_core::MouseButton::Middle)]
    );
}

#[test]
fn nav_thumbs_type_backspace_and_space_before_the_mouse_moves() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_RAISE).clear();

    // Act
    kb.tap(POS_BSPC, 20).tap(POS_SPACE, 20);

    // Assert
    assert_eq!(kb.log(), ["+BSPC", "-BSPC", "+SPACE", "-SPACE"]);
}

#[test]
fn nav_thumbs_click_once_the_mouse_moved() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_RAISE).tap(POS_G, 30).clear();

    // Act
    kb.tap(POS_BSPC, 20).tap(POS_SPACE, 20);

    // Assert
    assert_eq!(
        kb.log(),
        [
            "+Button(Left)",
            "-Button(Left)",
            "+Button(Right)",
            "-Button(Right)"
        ]
    );
}

#[test]
fn letting_nav_go_ends_the_pointer() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_RAISE).tap(POS_G, 30).clear();

    // Act
    kb.up(POS_RAISE);

    // Assert
    assert_eq!(kb.log(), ["pointer:false", "layer:default"]);
    assert!(!kb.engine.pointer_active());
}

#[test]
fn nav_again_after_the_pointer_ended_types_backspace() {
    // Arrange: moved, nav let go, nav again
    let mut kb = Kb::new();
    kb.down(POS_RAISE)
        .tap(POS_G, 30)
        .up(POS_RAISE)
        .down(POS_RAISE)
        .clear();

    // Act
    kb.tap(POS_BSPC, 20);

    // Assert
    assert_eq!(kb.log(), ["+BSPC", "-BSPC"]);
}

#[test]
fn click_held_across_letting_nav_go_releases_as_a_click() {
    // Arrange: click down, then nav let go
    let mut kb = Kb::new();
    kb.down(POS_RAISE)
        .tap(POS_G, 30)
        .down(POS_BSPC)
        .up(POS_RAISE)
        .clear();

    // Act
    kb.up(POS_BSPC);

    // Assert
    assert_eq!(kb.log(), ["-Button(Left)"]);
}

#[test]
fn backspace_held_across_a_mouse_move_releases_as_backspace() {
    // Arrange: Backspace down (pointer not active yet), then a move
    let mut kb = Kb::new();
    kb.down(POS_RAISE).down(POS_BSPC).tap(POS_G, 30).clear();

    // Act
    kb.up(POS_BSPC);

    // Assert
    assert_eq!(kb.log(), ["-BSPC"]);
}

#[test]
fn held_backspace_stays_down_for_the_computer_to_repeat() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_RAISE).clear();

    // Act: held for a second
    kb.down(POS_BSPC).wait(1000);

    // Assert: pressed once, still held
    assert_eq!(kb.log(), ["+BSPC"]);
    assert!(kb.host.contains(keys::BSPC.code));
}

#[test]
fn precision_alone_does_not_wake_the_pointer() {
    // Arrange: the precision key on nav, no move
    let mut kb = Kb::new();
    kb.down(POS_RAISE).tap(POS_ALT_TAB, 20).clear();

    // Act
    kb.tap(POS_BSPC, 20);

    // Assert
    assert_eq!(kb.log(), ["+BSPC", "-BSPC"]);
}

#[test]
fn f3_on_nav_does_not_wake_the_pointer() {
    // Arrange: E on nav (F3 until the mouse moved)
    let mut kb = Kb::new();
    kb.down(POS_RAISE).tap(POS_E, 20).clear();

    // Act
    kb.tap(POS_SPACE, 20);

    // Assert
    assert_eq!(kb.log(), ["+SPACE", "-SPACE"]);
}

#[test]
fn middle_click_works_without_moving() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_RAISE).clear();

    // Act
    kb.tap(POS_A, 20);

    // Assert
    assert_eq!(kb.log(), ["+Button(Middle)", "-Button(Middle)"]);
}

#[test]
fn symbols_thumb_on_nav_still_reaches_wm() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_RAISE).clear();

    // Act
    kb.down(POS_LOWER);

    // Assert
    assert_eq!(kb.log(), ["layer:wm"]);
}

#[test]
fn nav_layer_b_position_steps_the_radio_power() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_RAISE).clear();

    // Act
    kb.tap(POS_B, 20);

    // Assert
    assert_eq!(kb.log(), ["RadioPower"]);
}

// ---- wm layer (symbols + nav) ---------------------------------------------------

const POS_H: u8 = 18;
const POS_N: u8 = 30;

/// Hold both layer keys: the conditional wm layer is active.
fn kb_on_wm_layer() -> Kb {
    let mut kb = Kb::new();
    kb.down(POS_LOWER).down(POS_RAISE).clear();
    kb
}

#[test]
fn holding_symbols_then_nav_activates_wm_layer() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_LOWER).clear();

    // Act
    kb.down(POS_RAISE);

    // Assert
    assert_eq!(kb.log(), ["layer:wm"]);
}

#[test]
fn holding_nav_then_symbols_activates_wm_layer() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_RAISE).clear();

    // Act
    kb.down(POS_LOWER);

    // Assert
    assert_eq!(kb.log(), ["layer:wm"]);
}

#[test]
fn releasing_one_layer_key_leaves_wm_layer() {
    // Arrange
    let mut kb = kb_on_wm_layer();

    // Act
    kb.up(POS_RAISE);

    // Assert
    assert_eq!(kb.log(), ["layer:symbols"]);
}

#[test]
fn wm_digit_position_goes_to_workspace() {
    // Arrange
    let mut kb = kb_on_wm_layer();

    // Act: A position is 1 on symbols, so workspace 1
    kb.tap(POS_A, 20);

    // Assert
    assert_eq!(kb.log(), ["+LGUI", "+N1", "-N1", "-LGUI"]);
}

#[test]
fn wm_digit_zero_position_goes_to_workspace_10() {
    // Arrange
    let mut kb = kb_on_wm_layer();

    // Act: B position is 0 on symbols
    kb.tap(29, 20);

    // Assert
    assert_eq!(kb.log(), ["+LGUI", "+N0", "-N0", "-LGUI"]);
}

#[test]
fn wm_shifted_digit_position_sends_window_to_workspace() {
    // Arrange
    let mut kb = kb_on_wm_layer();

    // Act: Q position is ! (Shift+1) on symbols, so move window to workspace 1
    kb.tap(POS_Q, 20);

    // Assert
    assert_eq!(
        kb.log(),
        ["+LSHFT", "+LGUI", "+N1", "-N1", "-LSHFT", "-LGUI"]
    );
}

#[test]
fn wm_left_arrow_position_moves_column_left() {
    // Arrange
    let mut kb = kb_on_wm_layer();

    // Act: H position is ← on nav, so move column left (Mod + comma)
    kb.tap(POS_H, 20);

    // Assert
    assert_eq!(kb.log(), ["+LGUI", "+COMMA", "-COMMA", "-LGUI"]);
}

#[test]
fn wm_home_position_goes_to_previous_workspace() {
    // Arrange
    let mut kb = kb_on_wm_layer();

    // Act: N position is Home on nav
    kb.tap(POS_N, 20);

    // Assert
    assert_eq!(
        kb.log(),
        ["+LCTRL", "+LGUI", "+LEFT", "-LEFT", "-LCTRL", "-LGUI"]
    );
}

#[test]
fn wm_unused_key_sends_nothing() {
    // Arrange
    let mut kb = kb_on_wm_layer();

    // Act: the top-right key (Backspace position) is empty on the wm layer
    kb.tap(11, 20);

    // Assert
    assert_eq!(kb.log(), NOTHING);
}

#[test]
fn wm_tab_position_cycles_the_column_width() {
    // Arrange
    let mut kb = kb_on_wm_layer();

    // Act
    kb.tap(0, 20);

    // Assert: Mod+W
    assert_eq!(kb.log(), ["+LGUI", "+W", "-W", "-LGUI"]);
}

// ---- combos ----------------------------------------------------------------------

#[test]
fn combo_key_is_held_back_while_combo_is_possible() {
    // Arrange
    let mut kb = Kb::new();

    // Act
    kb.down(POS_J);

    // Assert
    assert_eq!(kb.log(), NOTHING);
}

#[test]
fn escape_combo_fires_on_second_key() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_J);

    // Act
    kb.at(20).down(POS_K);

    // Assert
    assert_eq!(kb.log(), ["+ESC"]);
}

#[test]
fn combo_keys_work_in_either_order() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_K);

    // Act
    kb.at(10).down(POS_J);

    // Assert
    assert_eq!(kb.log(), ["+ESC"]);
}

#[test]
fn combo_is_released_with_its_first_key() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_J).at(20).down(POS_K).clear();

    // Act
    kb.at(80).up(POS_J);

    // Assert
    assert_eq!(kb.log(), ["-ESC"]);
}

#[test]
fn releasing_the_last_combo_key_sends_nothing() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_J).at(20).down(POS_K).at(80).up(POS_J).clear();

    // Act
    kb.at(90).up(POS_K);

    // Assert
    assert_eq!(kb.log(), NOTHING);
}

#[test]
fn combo_key_tapped_alone_types_normally() {
    // Arrange
    let mut kb = Kb::new();

    // Act
    kb.tap(POS_J, 10);

    // Assert
    assert_eq!(kb.log(), ["+J", "-J"]);
}

#[test]
fn combo_key_is_still_held_back_just_before_timeout() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_J);

    // Act
    kb.wait(49);

    // Assert
    assert_eq!(kb.log(), NOTHING);
}

#[test]
fn combo_timeout_lets_the_key_through() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_J);

    // Act
    kb.wait(50);

    // Assert
    assert_eq!(kb.log(), ["+J"]);
}

#[test]
fn release_flushes_held_back_key_in_order() {
    // Arrange: J went through on timeout; after a pause K is held back as a new combo attempt
    let mut kb = Kb::new();
    kb.down(POS_J).wait(50).at(260).down(POS_K).clear();

    // Act
    kb.at(270).up(POS_J);

    // Assert
    assert_eq!(kb.log(), ["+K", "-J"]);
}

#[test]
fn slow_roll_over_combo_keys_types_both() {
    // Arrange
    let mut kb = Kb::new();

    // Act
    kb.down(POS_J)
        .at(60)
        .down(POS_K)
        .at(70)
        .up(POS_J)
        .at(80)
        .up(POS_K);

    // Assert
    assert_eq!(kb.log(), ["+J", "+K", "-J", "-K"]);
}

#[test]
fn combo_does_not_fire_while_typing() {
    // Arrange: A typed 100 ms before J, inside the 150 ms prior-idle window
    let mut kb = Kb::new();
    kb.tap(POS_A, 20).clear();

    // Act
    kb.at(100).down(POS_J).at(110).down(POS_K);

    // Assert: both keys typed straight away, no Esc
    assert_eq!(kb.log(), ["+J", "+K"]);
}

#[test]
fn combo_fires_after_a_pause_in_typing() {
    // Arrange: A typed 200 ms before J
    let mut kb = Kb::new();
    kb.tap(POS_A, 20).clear();

    // Act
    kb.at(200).down(POS_J).at(210).down(POS_K);

    // Assert
    assert_eq!(kb.log(), ["+ESC"]);
}

#[test]
fn layer_keys_do_not_count_as_typing_for_combos() {
    // Arrange: only a layer key was pressed just before (like ZMK, keycodes only)
    let mut kb = Kb::new();
    kb.down(POS_RAISE).at(10).up(POS_RAISE).clear();

    // Act
    kb.at(20).down(POS_J).at(30).down(POS_K);

    // Assert
    assert_eq!(kb.log(), ["+ESC"]);
}

#[test]
fn combos_are_ignored_on_other_layers() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_LOWER).clear();

    // Act
    kb.down(POS_J).at(10).down(POS_K);

    // Assert
    assert_eq!(kb.log(), ["+EQUAL", "+GRAVE"]);
}

#[test]
fn copy_combo_sends_ctrl_c() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_X);

    // Act
    kb.at(10).down(POS_C);

    // Assert
    assert_eq!(kb.log(), ["+LCTRL", "+C"]);
}

#[test]
fn releasing_copy_combo_releases_ctrl_c() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_X).at(10).down(POS_C).clear();

    // Act
    kb.at(40).up(POS_X);

    // Assert
    assert_eq!(kb.log(), ["-C", "-LCTRL"]);
}

#[test]
fn reset_bluetooth_combo_on_nav_layer() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_RAISE).clear();

    // Act: both profile keys (Profile ◀ on Z, Profile ▶ on X) within the combo timeout
    kb.at(100).down(POS_Z).at(105).down(POS_X);

    // Assert
    assert_eq!(kb.log(), ["BtClear"]);
}

// ---- caps word ---------------------------------------------------------------------

/// Turn caps word on with the T+Y combo and release both keys.
fn kb_with_caps_word() -> Kb {
    let mut kb = Kb::new();
    kb.down(POS_T)
        .at(5)
        .down(POS_Y)
        .at(30)
        .up(POS_T)
        .up(POS_Y)
        .at(100);
    kb.clear();
    kb
}

#[test]
fn caps_word_combo_turns_caps_word_on() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_T);

    // Act
    kb.at(5).down(POS_Y);

    // Assert
    assert_eq!(kb.log(), ["caps_word:true"]);
}

#[test]
fn caps_word_shifts_letters() {
    // Arrange
    let mut kb = kb_with_caps_word();

    // Act
    kb.tap(POS_A, 20);

    // Assert
    assert_eq!(kb.log(), ["+LSHFT", "+A", "-A", "-LSHFT"]);
}

#[test]
fn space_ends_caps_word() {
    // Arrange
    let mut kb = kb_with_caps_word();

    // Act
    kb.tap(POS_SPACE, 20);

    // Assert
    assert_eq!(kb.log(), ["caps_word:false", "+SPACE", "-SPACE"]);
}

#[test]
fn letters_are_lowercase_after_caps_word_ends() {
    // Arrange
    let mut kb = kb_with_caps_word();
    kb.tap(POS_SPACE, 20).clear();

    // Act
    kb.tap(POS_A, 20);

    // Assert
    assert_eq!(kb.log(), ["+A", "-A"]);
}

// ---- timing --------------------------------------------------------------------------

#[test]
fn no_deadline_when_idle() {
    // Arrange
    let kb = Kb::new();

    // Act
    let deadline = kb.engine.next_deadline();

    // Assert
    assert_eq!(deadline, None);
}

#[test]
fn deadline_is_end_of_tapping_term_while_mod_tap_is_undecided() {
    // Arrange
    let mut kb = Kb::new();
    kb.down(POS_ALT_TAB);

    // Act
    let deadline = kb.engine.next_deadline();

    // Assert
    assert_eq!(deadline, Some(200));
}

#[test]
fn deadline_is_combo_timeout_while_a_key_is_held_back() {
    // Arrange: J decides the mod-tap as hold, then waits for a combo
    let mut kb = Kb::new();
    kb.down(POS_ALT_TAB).at(10).down(POS_J);

    // Act
    let deadline = kb.engine.next_deadline();

    // Assert
    assert_eq!(deadline, Some(60));
}
