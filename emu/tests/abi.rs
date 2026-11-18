//! The C ABI as the .NET app calls it: a handle from `kb_emu_new`, text out
//! through (buf, cap) returning the full length, null pointers answered with
//! the documented defaults.
//!
//! Integration tests on purpose: they link the same (non-test) build of the
//! library as the cdylib the app loads, so the coverage of the `kb_emu_*`
//! functions is counted (a `#[cfg(test)]` build of them doesn't match the
//! cdylib's).

use std::ptr::{null, null_mut};

use kb_core::host::{KIND_CLEAR, REPORT_SIZE, encode_notification};
use kb_emu::*;

/// An emulator, freed at the end of the test.
struct Handle(*mut Emu);

impl Handle {
    fn new() -> Self {
        Self(kb_emu_new())
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: from `kb_emu_new` in `new`, freed only here.
        unsafe { kb_emu_free(self.0) };
    }
}

const NAV: u8 = 41;
const SYMBOLS: u8 = 36;
const Q: u8 = 1;
const BT_PREV: u8 = 25;
const BT_NEXT: u8 = 26;
const SHOW_LINK: u8 = 28;
const RADIO_POWER: u8 = 29;
const STUDIO_UNLOCK: u8 = 35;

/// Text through a (buf, cap) function, with a buffer that fits.
fn text(f: impl FnOnce(*mut u8, usize) -> usize) -> String {
    let mut buf = [0u8; 512];
    let n = f(buf.as_mut_ptr(), buf.len());
    String::from_utf8(buf[..n].to_vec()).expect("UTF-8")
}

/// Every event waiting, as (kind, value).
///
/// # Safety
///
/// `e` is null or an emulator from `kb_emu_new` that is not freed yet.
unsafe fn events(e: *mut Emu, now: u32) -> Vec<(u8, u8)> {
    // SAFETY: `e` is null or live, as this function requires; the other
    // pointers are to local buffers, with their lengths.
    unsafe {
        let mut out = Vec::new();
        let mut ev = KbEvent {
            kind: 0,
            value: 0,
            mods: 0,
            reserved: 0,
            keys: [0; 32],
        };
        while kb_emu_poll(e, now, &raw mut ev) {
            out.push((ev.kind, ev.value));
        }
        out
    }
}

/// Hold nav and tap `pos` (a combo key acts when released alone).
///
/// # Safety
///
/// `e` is null or an emulator from `kb_emu_new` that is not freed yet.
unsafe fn tap_on_nav(e: *mut Emu, pos: u8, now: u32) {
    // SAFETY: `e` is null or live, as this function requires; the other
    // pointers are to local buffers, with their lengths.
    unsafe {
        kb_emu_press(e, NAV, now);
        kb_emu_press(e, pos, now + 10);
        kb_emu_release(e, pos, now + 20);
        kb_emu_release(e, NAV, now + 30);
    }
}

/// # Safety
///
/// `e` is null or an emulator from `kb_emu_new` that is not freed yet.
unsafe fn unlock_studio(e: *mut Emu, now: u32) {
    // SAFETY: `e` is null or live, as this function requires; the other
    // pointers are to local buffers, with their lengths.
    unsafe {
        tap_on_nav(e, STUDIO_UNLOCK, now);
        events(e, now + 30);
    }
}

/// # Safety
///
/// `e` is null or an emulator from `kb_emu_new` that is not freed yet.
unsafe fn studio_set(e: *mut Emu, layer: u8, pos: u8, enc: &str, now: u32) -> i32 {
    // SAFETY: `e` is null or live, as this function requires; the other
    // pointers are to local buffers, with their lengths.
    unsafe { kb_emu_studio_set(e, layer, pos, enc.as_ptr(), enc.len(), now) }
}

/// The I²C writes for a display, with a buffer that fits.
///
/// # Safety
///
/// `e` is null or an emulator from `kb_emu_new` that is not freed yet.
unsafe fn display(e: *mut Emu, half: u8, now: u32) -> Vec<u8> {
    // SAFETY: `e` is null or live, as this function requires; the other
    // pointers are to local buffers, with their lengths.
    unsafe {
        let needed = kb_emu_display(e, half, now, null_mut(), 0);
        let mut buf = vec![0u8; needed];
        let n = kb_emu_display(e, half, now, buf.as_mut_ptr(), buf.len());
        buf.truncate(n);
        buf
    }
}

// ---- null handles ------------------------------------------------------------

#[test]
fn null_handle_reads_as_the_documented_defaults() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = null_mut();

        // Act
        let got = (
            kb_emu_next_deadline(e),
            kb_emu_pairing(e),
            kb_emu_radio_dbm(e),
            kb_emu_power_state(e),
            kb_emu_profile(e),
            kb_emu_profile_open(e),
            kb_emu_profile_connected(e),
            kb_emu_peripheral_connected(e),
            kb_emu_transport(e),
            kb_emu_link_stage(e),
            kb_emu_simulate_pairing(e, 123_456, 0),
        );

        // Assert
        assert_eq!(got, (-1, false, 0, 0, 0, false, false, false, 1, 0, 0));
    }
}

#[test]
fn null_handle_gives_no_text() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = null_mut();

        // Act
        let lengths = [
            text(|b, c| kb_emu_label(e, Q, b, c)).len(),
            text(|b, c| kb_emu_layer_name(e, 0, b, c)).len(),
            text(|b, c| kb_emu_studio_binding(e, 0, 0, b, c)).len(),
            text(|b, c| kb_emu_studio_default_binding(e, 0, 0, b, c)).len(),
            text(|b, c| kb_emu_studio_layer(e, 0, b, c)).len(),
            text(|b, c| kb_emu_studio_combo(e, 0, b, c)).len(),
            text(|b, c| kb_emu_studio_default_combo(e, 0, b, c)).len(),
            text(|b, c| kb_emu_studio_next_line(e, 0, b, c)).len(),
            kb_emu_display(e, 0, 0, null_mut(), 0),
        ];

        // Assert
        assert_eq!(lengths, [0; 9]);
    }
}

#[test]
fn null_handle_studio_calls_are_bad_arguments() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let (e, combo) = (null_mut(), "- 1,2 * 50 0 kp:A");

        // Act
        let codes = [
            studio_set(e, 0, 0, "kp:A", 0),
            kb_emu_studio_check(e, 0, 0, "kp:A".as_ptr(), 4),
            kb_emu_studio_save(e, 0),
            kb_emu_studio_revert(e),
            kb_emu_studio_reset(e, 0),
            kb_emu_studio_set_storage_path(e, null(), 0),
            kb_emu_studio_combo_add(e, combo.as_ptr(), combo.len(), 0),
            kb_emu_studio_combo_set(e, 0, combo.as_ptr(), combo.len(), 0),
            kb_emu_studio_combo_remove(e, 0, 0),
            kb_emu_studio_combo_check(e, 255, combo.as_ptr(), combo.len()),
            kb_emu_studio_combo_check_remove(e, 0),
        ];

        // Assert
        assert_eq!(codes, [-1; 11]);
    }
}

#[test]
fn null_handle_setters_and_counts_do_nothing() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = null_mut();
        let mut mouse = KbMouse {
            x: 0,
            y: 0,
            wheel: 0,
            pan: 0,
            buttons: 0,
            pointer: 0,
            moving: 0,
            hires: 0,
        };

        // Act
        kb_emu_press(e, Q, 0);
        kb_emu_release(e, Q, 0);
        kb_emu_tick(e, 0);
        kb_emu_set_power_timeouts(e, 1, 2, 0);
        kb_emu_set_battery(e, 0, 50);
        kb_emu_set_half_power(e, 1, false, 0);
        kb_emu_reset_half(e, 0, 0);
        kb_emu_replay_boot(e);
        kb_emu_set_charging(e, 0, true);
        kb_emu_set_peripheral_connected(e, true);
        kb_emu_set_screen_style(e, 0);
        kb_emu_set_profile_connected(e, true);
        kb_emu_set_profile_open(e, false);
        kb_emu_set_profile(e, 3);
        kb_emu_set_transport(e, 0);
        kb_emu_set_link_stage(e, 1);
        kb_emu_mouse_set_resolution(e, true, true);
        kb_emu_mouse_reset_position(e);
        kb_emu_studio_lock(e, 0);
        kb_emu_free(e);
        let answered = (
            kb_emu_poll(e, 0, null_mut()),
            kb_emu_mouse_state(e, &raw mut mouse),
            kb_emu_raw_hid(e, [0u8; 32].as_ptr(), 32, 0),
            kb_emu_studio_dirty(e),
            kb_emu_studio_generation(e),
            kb_emu_studio_unlock_remaining_ms(e, 0),
            kb_emu_studio_combo_count(e),
            kb_emu_studio_default_combo_count(e),
            kb_emu_studio_command(e, "#studio get".as_ptr(), 11, 0),
        );

        // Assert
        assert_eq!(answered, (false, false, 0, false, 0, 0, 0, 0, false));
    }
}

// ---- keys and events ---------------------------------------------------------

#[test]
fn key_press_comes_out_as_a_report_with_its_usage() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        kb_emu_press(e.0, Q, 0);
        let mut ev = KbEvent {
            kind: 0,
            value: 0,
            mods: 0,
            reserved: 0,
            keys: [0; 32],
        };

        // Act
        let mut report = None;
        while kb_emu_poll(e.0, 0, &raw mut ev) {
            if ev.kind == 1 {
                report = Some(ev.keys);
            }
        }

        // Assert: Q is usage 0x14 (bit 4 of byte 2)
        let keys = report.expect("a report");
        assert_eq!(keys[0x14 / 8] & (1 << (0x14 % 8)), 1 << (0x14 % 8));
    }
}

#[test]
fn poll_into_a_null_event_reports_nothing() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        kb_emu_press(e.0, Q, 0);

        // Act
        let got = kb_emu_poll(e.0, 0, null_mut());

        // Assert
        assert!(!got);
    }
}

#[test]
fn layer_key_reports_the_layer() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        kb_emu_press(e.0, SYMBOLS, 0);

        // Act
        let got = events(e.0, 0);

        // Assert
        assert!(got.contains(&(2, 1)), "{got:?}");
    }
}

#[test]
fn system_keys_report_their_numbers() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        for (i, pos) in [BT_PREV, BT_NEXT, SHOW_LINK, RADIO_POWER, STUDIO_UNLOCK]
            .into_iter()
            .enumerate()
        {
            tap_on_nav(e.0, pos, i as u32 * 100);
        }

        // Act
        let got = events(e.0, 500);

        // Assert
        let system: Vec<u8> = got
            .iter()
            .filter(|(kind, _)| *kind == 4)
            .map(|&(_, v)| v)
            .collect();
        assert_eq!(system, [0, 1, 5, 6, 4]);
    }
}

#[test]
fn caps_word_combo_reports_caps_word_on() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange: positions 5 and 6 together
        let e = Handle::new();
        kb_emu_press(e.0, 5, 0);
        kb_emu_press(e.0, 6, 5);
        kb_emu_release(e.0, 5, 30);
        kb_emu_release(e.0, 6, 35);

        // Act
        let got = events(e.0, 40);

        // Assert
        assert!(got.contains(&(3, 1)), "{got:?}");
    }
}

#[test]
fn radio_power_key_steps_the_level() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        tap_on_nav(e.0, RADIO_POWER, 0);
        events(e.0, 30);

        // Act
        let dbm = kb_emu_radio_dbm(e.0);

        // Assert
        assert_eq!(dbm, 8);
    }
}

#[test]
fn deadline_follows_a_pending_key_timer() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange: a mod-tap held (Tab / Alt) waits for its hold timeout
        let e = Handle::new();
        kb_emu_press(e.0, 12, 100);

        // Act
        let deadline = kb_emu_next_deadline(e.0);

        // Assert
        assert!(deadline > 100, "{deadline}");
    }
}

// ---- power -------------------------------------------------------------------

#[test]
fn short_idle_timeout_reports_idle_after_a_tick() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        kb_emu_set_power_timeouts(e.0, 1_000, 0, 0);
        events(e.0, 0);
        kb_emu_tick(e.0, 1_500);

        // Act
        let got = events(e.0, 1_500);

        // Assert
        assert!(got.contains(&(5, 1)), "{got:?}");
        assert_eq!(kb_emu_power_state(e.0), 1);
    }
}

#[test]
fn sleep_timeout_puts_the_keyboard_to_sleep() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        kb_emu_set_power_timeouts(e.0, 1_000, 2_000, 0);

        // Act
        kb_emu_tick(e.0, 3_000);

        // Assert
        assert_eq!(kb_emu_power_state(e.0), 2);
    }
}

#[test]
fn raw_hid_while_asleep_does_not_arrive() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        kb_emu_set_power_timeouts(e.0, 1_000, 2_000, 0);
        kb_emu_tick(e.0, 3_000);
        let mut clear = [0u8; REPORT_SIZE];
        clear[0] = KIND_CLEAR;

        // Act
        let result = kb_emu_raw_hid(e.0, clear.as_ptr(), clear.len(), 3_000);

        // Assert
        assert_eq!(result, RawHidResult::Asleep as u8);
    }
}

#[test]
fn key_in_deep_sleep_wakes_the_keyboard_and_is_lost() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        kb_emu_set_power_timeouts(e.0, 1_000, 2_000, 0);
        kb_emu_tick(e.0, 3_000);
        events(e.0, 3_000);
        kb_emu_press(e.0, Q, 3_100);

        // Act
        let got = events(e.0, 3_100);

        // Assert: awake (power event 0), no key report
        assert!(got.contains(&(5, 0)), "{got:?}");
        assert!(!got.iter().any(|(kind, _)| *kind == 1), "{got:?}");
        assert_eq!(kb_emu_power_state(e.0), 0);
    }
}

// ---- pairing -----------------------------------------------------------------

#[test]
fn simulated_pairing_keeps_six_digits_and_asks() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();

        // Act
        let code = kb_emu_simulate_pairing(e.0, 1_234_567, 0);

        // Assert
        assert_eq!(code, 234_567);
        assert!(kb_emu_pairing(e.0));
    }
}

#[test]
fn pairing_nobody_answers_times_out() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        kb_emu_simulate_pairing(e.0, 111_111, 0);
        kb_emu_tick(e.0, PAIR_TIMEOUT_MS);

        // Act
        let got = events(e.0, PAIR_TIMEOUT_MS);

        // Assert
        assert!(got.contains(&(6, PairResult::TimedOut as u8)), "{got:?}");
        assert!(!kb_emu_pairing(e.0));
    }
}

// ---- status ------------------------------------------------------------------

#[test]
fn profile_setters_show_on_the_getters() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        kb_emu_set_profile(e.0, 3);
        kb_emu_set_profile_open(e.0, false);

        // Act
        kb_emu_set_profile_connected(e.0, true);

        // Assert
        assert_eq!(
            (
                kb_emu_profile(e.0),
                kb_emu_profile_open(e.0),
                kb_emu_profile_connected(e.0)
            ),
            (3, false, true)
        );
        assert!(kb_emu_profile_count() > 3);
    }
}

#[test]
fn transport_round_trips() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        kb_emu_set_transport(e.0, 1);
        let ble = kb_emu_transport(e.0);

        // Act
        kb_emu_set_transport(e.0, 0);

        // Assert
        assert_eq!((ble, kb_emu_transport(e.0)), (1, 0));
    }
}

#[test]
fn peripheral_connected_round_trips() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();

        // Act
        kb_emu_set_peripheral_connected(e.0, false);

        // Assert
        assert!(!kb_emu_peripheral_connected(e.0));
    }
}

#[test]
fn link_stage_names_are_listed_and_unknown_ones_are_empty() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let count = kb_emu_link_stage_count();

        // Act
        let names: Vec<String> = (0..=count)
            .map(|i| text(|b, c| kb_emu_link_stage_name(i, b, c)))
            .collect();

        // Assert
        assert_eq!(names[0], "", "0 is hidden");
        assert!(names[1..count as usize].iter().all(|n| !n.is_empty()));
        assert_eq!(names[count as usize], "", "past the last");
    }
}

#[test]
fn battery_above_100_is_shown_as_full() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange: the right screen drawn at 100 %, then at 50 %
        let e = Handle::new();
        kb_emu_set_battery(e.0, 1, 100);
        let full = display(e.0, 1, 0);
        kb_emu_set_battery(e.0, 1, 50);
        display(e.0, 1, 0);

        // Act: half numbers past the right half are the right half
        kb_emu_set_battery(e.0, 9, 250);

        // Assert: the right screen draws as at 100 % again
        let redrawn = display(e.0, 1, 0);
        assert!(redrawn.len() >= 512);
        assert_eq!(redrawn[redrawn.len() - 512..], full[full.len() - 512..]);
    }
}

#[test]
fn screen_style_change_redraws_the_left_screen() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        display(e.0, 0, 0);
        let unchanged = display(e.0, 0, 0).len();

        // Act
        kb_emu_set_screen_style(e.0, 0);

        // Assert
        assert!(display(e.0, 0, 0).len() > unchanged);
    }
}

#[test]
fn charging_mark_redraws_the_screen() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        display(e.0, 0, 0);
        let unchanged = display(e.0, 0, 0).len();

        // Act
        kb_emu_set_charging(e.0, 0, true);

        // Assert
        assert!(display(e.0, 0, 0).len() > unchanged);
    }
}

#[test]
fn display_too_small_keeps_the_writes_for_the_next_call() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        let needed = kb_emu_display(e.0, 0, 0, null_mut(), 0);
        let mut small = vec![0u8; needed - 1];
        kb_emu_display(e.0, 0, 0, small.as_mut_ptr(), small.len());

        // Act
        let again = kb_emu_display(e.0, 0, 0, null_mut(), 0);

        // Assert
        assert_eq!(again, needed);
    }
}

#[test]
fn half_switched_off_has_no_keys() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange: Q is on the left half; position 6 (Y) is on the right
        let e = Handle::new();
        kb_emu_set_half_power(e.0, 1, false, 0);
        events(e.0, 0);
        kb_emu_press(e.0, 6, 10);

        // Act
        let got = events(e.0, 10);

        // Assert
        assert!(!got.iter().any(|(kind, _)| *kind == 1), "{got:?}");
        assert!(!kb_emu_peripheral_connected(e.0));
    }
}

#[test]
fn halves_link_again_after_a_reset() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        kb_emu_reset_half(e.0, 1, 0);
        let unlinked = kb_emu_peripheral_connected(e.0);

        // Act
        kb_emu_tick(e.0, 60_000);

        // Assert
        assert!(!unlinked);
        assert!(kb_emu_peripheral_connected(e.0));
    }
}

#[test]
fn replayed_boot_redraws_both_screens() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        display(e.0, 0, 0);
        display(e.0, 1, 0);
        let unchanged = display(e.0, 0, 10).len();

        // Act
        kb_emu_replay_boot(e.0);

        // Assert
        assert!(display(e.0, 0, 100).len() > unchanged);
    }
}

#[test]
fn layer_and_usage_names() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();

        // Act
        let names = [
            text(|b, c| kb_emu_layer_name(e.0, 1, b, c)),
            text(|b, c| kb_emu_layer_name(e.0, 9, b, c)),
            text(|b, c| kb_emu_usage_name(0x04, b, c)),
        ];

        // Assert
        assert_eq!(names, ["symbols", "?", "A"]);
    }
}

#[test]
fn label_into_a_short_buffer_returns_the_full_length() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange: the Tab / Alt mod-tap
        let e = Handle::new();
        let mut one = [0u8; 1];

        // Act
        let n = kb_emu_label(e.0, 12, one.as_mut_ptr(), one.len());

        // Assert: "Tab\nAlt", only the first byte copied
        assert_eq!((n, one[0]), ("Tab\nAlt".len(), b'T'));
    }
}

// ---- Raw HID -----------------------------------------------------------------

#[test]
fn raw_hid_notification_reaches_the_right_half() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        let mut reports = Vec::new();
        encode_notification("hi", 1, |r| reports.push(*r));

        // Act
        let results: Vec<u8> = reports
            .iter()
            .map(|r| kb_emu_raw_hid(e.0, r.as_ptr(), r.len(), 0))
            .collect();

        // Assert
        assert_eq!(results.last(), Some(&(RawHidResult::Delivered as u8)));
    }
}

#[test]
fn raw_hid_clear_clears_the_notification() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        encode_notification("hi", 1, |r| {
            kb_emu_raw_hid(e.0, r.as_ptr(), r.len(), 0);
        });
        let mut clear = [0u8; REPORT_SIZE];
        clear[0] = KIND_CLEAR;

        // Act
        let result = kb_emu_raw_hid(e.0, clear.as_ptr(), clear.len(), 10);

        // Assert
        assert_eq!(result, RawHidResult::Cleared as u8);
    }
}

#[test]
fn raw_hid_without_the_right_half_is_not_linked() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        kb_emu_set_peripheral_connected(e.0, false);
        let mut clear = [0u8; REPORT_SIZE];
        clear[0] = KIND_CLEAR;

        // Act
        let result = kb_emu_raw_hid(e.0, clear.as_ptr(), clear.len(), 0);

        // Assert
        assert_eq!(result, RawHidResult::NotLinked as u8);
    }
}

#[test]
fn raw_hid_null_report_is_pending() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();

        // Act
        let result = kb_emu_raw_hid(e.0, null(), 32, 0);

        // Assert
        assert_eq!(result, RawHidResult::Pending as u8);
    }
}

#[test]
fn notification_expires_after_its_time() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        encode_notification("hi", 1, |r| {
            kb_emu_raw_hid(e.0, r.as_ptr(), r.len(), 0);
        });
        let shown = (*e.0).status().notification.is_some();

        // Act
        kb_emu_tick(e.0, NOTIFICATION_MS);

        // Assert
        assert!(shown);
        assert!((*e.0).status().notification.is_none());
    }
}

#[test]
fn new_mouse_is_still_with_whole_detents() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        let mut m = KbMouse {
            x: 1,
            y: 1,
            wheel: 1,
            pan: 1,
            buttons: 1,
            pointer: 1,
            moving: 1,
            hires: 1,
        };

        // Act
        let filled = kb_emu_mouse_state(e.0, &raw mut m);

        // Assert
        assert!(filled);
        assert_eq!(
            (
                m.x, m.y, m.wheel, m.pan, m.buttons, m.pointer, m.moving, m.hires
            ),
            (0, 0, 0, 0, 0, 0, 0, 0)
        );
    }
}

#[test]
fn smooth_scrolling_shows_in_the_mouse_state() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        let mut m = KbMouse {
            x: 0,
            y: 0,
            wheel: 0,
            pan: 0,
            buttons: 0,
            pointer: 0,
            moving: 0,
            hires: 0,
        };

        // Act
        kb_emu_mouse_set_resolution(e.0, true, true);

        // Assert
        kb_emu_mouse_state(e.0, &raw mut m);
        assert_eq!(m.hires, 0b11);
    }
}

#[test]
fn mouse_state_into_null_is_refused() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();

        // Act
        let filled = kb_emu_mouse_state(e.0, null_mut());

        // Assert
        assert!(!filled);
    }
}

#[test]
fn reset_position_keeps_the_buttons() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        let mut m = KbMouse {
            x: 9,
            y: 9,
            wheel: 9,
            pan: 9,
            buttons: 9,
            pointer: 0,
            moving: 0,
            hires: 0,
        };

        // Act
        kb_emu_mouse_reset_position(e.0);

        // Assert
        kb_emu_mouse_state(e.0, &raw mut m);
        assert_eq!((m.x, m.y, m.wheel, m.pan, m.buttons), (0, 0, 0, 0, 0));
    }
}

// ---- Studio ------------------------------------------------------------------

#[test]
fn studio_set_while_locked_is_refused() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();

        // Act
        let code = studio_set(e.0, 0, Q, "kp:Z", 0);

        // Assert
        assert_eq!(code, 1);
    }
}

#[test]
fn studio_set_when_unlocked_changes_the_label_and_dirties() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        unlock_studio(e.0, 0);
        let generation = kb_emu_studio_generation(e.0);

        // Act
        let code = studio_set(e.0, 0, Q, "kp:Z", 100);

        // Assert
        assert_eq!(code, 0);
        assert!(kb_emu_studio_dirty(e.0));
        assert_ne!(kb_emu_studio_generation(e.0), generation);
        assert_eq!(text(|b, c| kb_emu_label(e.0, Q, b, c)), "Z");
        assert_eq!(text(|b, c| kb_emu_studio_binding(e.0, 0, Q, b, c)), "kp:Z");
        assert_eq!(
            text(|b, c| kb_emu_studio_default_binding(e.0, 0, Q, b, c)),
            "kp:Q"
        );
    }
}

#[test]
fn studio_save_keeps_the_edit_and_cleans() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        unlock_studio(e.0, 0);
        studio_set(e.0, 0, Q, "kp:Z", 100);

        // Act
        let code = kb_emu_studio_save(e.0, 100);

        // Assert
        assert_eq!(code, 0);
        assert!(!kb_emu_studio_dirty(e.0));
    }
}

#[test]
fn studio_save_to_an_unwritable_file_is_a_storage_error() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        // A "directory" that is a regular file: unwritable even for root (CI).
        let file = std::env::temp_dir().join(format!("emk-not-a-dir-{}", std::process::id()));
        std::fs::write(&file, b"").expect("temp file");
        let path = format!("{}/studio.txt", file.display());
        kb_emu_studio_set_storage_path(e.0, path.as_ptr(), path.len());
        unlock_studio(e.0, 0);
        studio_set(e.0, 0, Q, "kp:Z", 100);

        // Act
        let code = kb_emu_studio_save(e.0, 100);

        // Assert
        assert_eq!(code, 7);
        assert!(kb_emu_studio_dirty(e.0), "still dirty");
        std::fs::remove_file(file).ok();
    }
}

#[test]
fn studio_revert_drops_the_edit() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        unlock_studio(e.0, 0);
        studio_set(e.0, 0, Q, "kp:Z", 100);

        // Act
        let code = kb_emu_studio_revert(e.0);

        // Assert
        assert_eq!(code, 0);
        assert_eq!(text(|b, c| kb_emu_label(e.0, Q, b, c)), "Q");
    }
}

#[test]
fn studio_lock_locks_at_once() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        unlock_studio(e.0, 0);

        // Act
        kb_emu_studio_lock(e.0, 100);

        // Assert
        assert_eq!(kb_emu_studio_unlock_remaining_ms(e.0, 100), 0);
        assert_eq!(studio_set(e.0, 0, Q, "kp:Z", 100), 1);
    }
}

#[test]
fn studio_layer_lists_every_position() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();

        // Act
        let layer = text(|b, c| kb_emu_studio_layer(e.0, 0, b, c));

        // Assert
        assert_eq!(layer.split(' ').count(), KEYS);
        assert_eq!(layer.split(' ').nth(1), Some("kp:Q"));
        assert_eq!(kb_emu_studio_layer_count(), 4);
    }
}

#[test]
fn studio_layer_out_of_range_is_empty() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();

        // Act
        let n = kb_emu_studio_layer(e.0, 4, null_mut(), 0);

        // Assert
        assert_eq!(n, 0);
    }
}

#[test]
fn studio_storage_path_with_bad_utf8_is_refused() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();

        // Act
        let code = kb_emu_studio_set_storage_path(e.0, [0xFFu8].as_ptr(), 1);

        // Assert
        assert_eq!(code, -1);
    }
}

#[test]
fn studio_storage_path_without_a_file_yet_applies() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        let path = std::env::temp_dir().join(format!("emk-abi-{}.txt", std::process::id()));
        let path = path.to_str().unwrap();

        // Act
        let code = kb_emu_studio_set_storage_path(e.0, path.as_ptr(), path.len());

        // Assert
        assert_eq!(code, 0);
    }
}

#[test]
fn studio_command_that_is_not_studio_is_not_taken() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        let line = "#debug on";

        // Act
        let taken = kb_emu_studio_command(e.0, line.as_ptr(), line.len(), 0);

        // Assert
        assert!(!taken);
        assert_eq!(kb_emu_studio_next_line(e.0, 0, null_mut(), 0), 0);
    }
}

#[test]
fn studio_command_answer_waits_for_a_buffer_that_fits() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        let line = "#studio get";
        kb_emu_studio_command(e.0, line.as_ptr(), line.len(), 0);
        let needed = kb_emu_studio_next_line(e.0, 0, null_mut(), 0);

        // Act
        let mut buf = vec![0u8; kb_emu_studio_line_max() as usize];
        let n = kb_emu_studio_next_line(e.0, 0, buf.as_mut_ptr(), buf.len());

        // Assert
        assert_eq!(n, needed);
        assert!(buf[..n].ends_with(b"\r\n"));
    }
}

#[test]
fn studio_combos_start_as_the_compiled_ones() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();

        // Act
        let (live, compiled) = (
            kb_emu_studio_combo_count(e.0),
            kb_emu_studio_default_combo_count(e.0),
        );

        // Assert
        assert_eq!(live, compiled);
        assert_eq!(
            text(|b, c| kb_emu_studio_combo(e.0, 0, b, c)),
            text(|b, c| kb_emu_studio_default_combo(e.0, 0, b, c))
        );
        assert_eq!(text(|b, c| kb_emu_studio_combo(e.0, live, b, c)), "");
        assert_eq!(
            text(|b, c| kb_emu_studio_default_combo(e.0, compiled, b, c)),
            ""
        );
        assert!(kb_emu_studio_combo_max() >= live);
    }
}

#[test]
fn studio_combo_add_appends_it() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        unlock_studio(e.0, 0);
        let before = kb_emu_studio_combo_count(e.0);
        let combo = "undo 24,25 0 50 150 kp:LC(Z)";

        // Act
        let code = kb_emu_studio_combo_add(e.0, combo.as_ptr(), combo.len(), 100);

        // Assert
        assert_eq!(code, 0);
        assert_eq!(kb_emu_studio_combo_count(e.0), before + 1);
        assert!(text(|b, c| kb_emu_studio_combo(e.0, before, b, c)).starts_with("undo 24,25"));
    }
}

#[test]
fn studio_combo_set_replaces_it() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        unlock_studio(e.0, 0);
        let combo = "copy 26,27 0 60 150 kp:LC(C)";

        // Act
        let code = kb_emu_studio_combo_set(e.0, 1, combo.as_ptr(), combo.len(), 100);

        // Assert
        assert_eq!(code, 0);
        assert!(text(|b, c| kb_emu_studio_combo(e.0, 1, b, c)).contains(" 60 "));
    }
}

#[test]
fn studio_combo_remove_moves_the_later_ones_down() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        unlock_studio(e.0, 0);
        let second = text(|b, c| kb_emu_studio_combo(e.0, 1, b, c));

        // Act
        let code = kb_emu_studio_combo_remove(e.0, 0, 100);

        // Assert
        assert_eq!(code, 0);
        assert_eq!(text(|b, c| kb_emu_studio_combo(e.0, 0, b, c)), second);
    }
}

#[test]
fn studio_combo_checks_change_nothing() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        let (good, bad) = ("- 1,2 * 50 0 kp:A", "- 1 * 50 0 kp:A");
        let count = kb_emu_studio_combo_count(e.0);

        // Act
        let codes = [
            kb_emu_studio_combo_check(e.0, 255, good.as_ptr(), good.len()),
            kb_emu_studio_combo_check(e.0, 0, bad.as_ptr(), bad.len()),
            kb_emu_studio_combo_check(e.0, 0, [0xFFu8].as_ptr(), 1),
            kb_emu_studio_combo_check_remove(e.0, 0),
            kb_emu_studio_combo_check_remove(e.0, 200),
        ];

        // Assert: ok, positions, bad text, ok, range (never "locked")
        assert_eq!(codes, [0, 8, -1, 0, 2]);
        assert_eq!(kb_emu_studio_combo_count(e.0), count);
    }
}

#[test]
fn studio_combo_with_bad_text_is_refused() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        unlock_studio(e.0, 0);

        // Act
        let codes = [
            kb_emu_studio_combo_add(e.0, [0xFFu8].as_ptr(), 1, 100),
            kb_emu_studio_combo_set(e.0, 0, [0xFFu8].as_ptr(), 1, 100),
            kb_emu_studio_combo_add(e.0, "only three words".as_ptr(), 16, 100),
        ];

        // Assert
        assert_eq!(codes, [-1, -1, 12]);
    }
}

#[test]
fn studio_reset_goes_back_to_the_compiled_keymap() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let e = Handle::new();
        unlock_studio(e.0, 0);
        studio_set(e.0, 0, Q, "kp:Z", 100);
        kb_emu_studio_save(e.0, 100);

        // Act
        let code = kb_emu_studio_reset(e.0, 200);

        // Assert
        assert_eq!(code, 0);
        assert_eq!(text(|b, c| kb_emu_label(e.0, Q, b, c)), "Q");
    }
}

#[test]
fn studio_names_for_pickers_end_with_nothing() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let past = 100_000;

        // Act
        let got = (
            text(|b, c| kb_emu_studio_key_name(0, b, c)),
            text(|b, c| kb_emu_studio_system_name(0, b, c)),
            kb_emu_studio_key_name(past, null_mut(), 0),
            kb_emu_studio_system_name(past, null_mut(), 0),
        );

        // Assert
        assert_eq!(got, ("A".to_owned(), "BtPrev".to_owned(), 0, 0));
    }
}

#[test]
fn studio_canonical_of_a_non_action_is_minus_one() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let enc = "kp:NOPE";

        // Act
        let n = kb_emu_studio_canonical(enc.as_ptr(), enc.len(), null_mut(), 0);

        // Assert
        assert_eq!(n, -1);
    }
}

#[test]
fn studio_label_of_a_non_action_is_empty() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let enc = "nonsense";

        // Act
        let n = kb_emu_studio_label(enc.as_ptr(), enc.len(), null_mut(), 0);

        // Assert
        assert_eq!(n, 0);
    }
}

// ---- the mirror (DEBUG tab) ----------------------------------------------------

/// A mirror, freed at the end of the test.
struct MirrorHandle(*mut mirror::Mirror);

impl MirrorHandle {
    fn new() -> Self {
        Self(mirror::kb_mirror_new())
    }
}

impl Drop for MirrorHandle {
    fn drop(&mut self) {
        // SAFETY: from `kb_mirror_new` in `new`, freed only here.
        unsafe { mirror::kb_mirror_free(self.0) };
    }
}

fn mirror_status(power: u8) -> mirror::MirrorStatus {
    mirror::MirrorStatus {
        battery_left: 80,
        battery_right: 70,
        linked: 1,
        transport: 0,
        power,
        ..mirror::MirrorStatus::default()
    }
}

/// # Safety
///
/// `m` is null or a mirror from `kb_mirror_new` that is not freed yet.
unsafe fn mirror_display(m: *mut mirror::Mirror, half: u8, now: u32) -> Vec<u8> {
    // SAFETY: `m` is null or live, as this function requires; the other
    // pointers are to local buffers, with their lengths.
    unsafe {
        let needed = mirror::kb_mirror_display(m, half, now, null_mut(), 0);
        let mut buf = vec![0u8; needed];
        let n = mirror::kb_mirror_display(m, half, now, buf.as_mut_ptr(), buf.len());
        buf.truncate(n);
        buf
    }
}

#[test]
fn mirror_null_handle_does_nothing() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let m = null_mut();
        let status = mirror_status(0);

        // Act
        mirror::kb_mirror_set_status(m, &raw const status, null(), 0);
        mirror::kb_mirror_set_frame(m, 0, null());
        mirror::kb_mirror_free(m);
        let n = mirror::kb_mirror_display(m, 0, 0, null_mut(), 0);

        // Assert
        assert_eq!(n, 0);
    }
}

#[test]
fn mirror_status_with_its_stage_shows_on_the_right_screen() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let m = MirrorHandle::new();
        let status = mirror_status(0);
        let stage = "linked";

        // Act
        mirror::kb_mirror_set_status(m.0, &raw const status, stage.as_ptr(), stage.len());

        // Assert
        let st = (*m.0).status();
        assert_eq!(
            (st.link_detail, st.battery, st.transport),
            ("linked", [80, 70], kb_display::status::Transport::Usb)
        );
    }
}

#[test]
fn mirror_null_status_is_ignored() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let m = MirrorHandle::new();
        let status = mirror_status(0);
        mirror::kb_mirror_set_status(m.0, &raw const status, null(), 0);

        // Act
        mirror::kb_mirror_set_status(m.0, null(), null(), 0);

        // Assert
        assert_eq!((*m.0).status().battery, [80, 70]);
    }
}

#[test]
fn mirror_frame_is_sent_as_the_hardware_drew_it() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let m = MirrorHandle::new();
        let frame = kb_display::frame::Frame {
            buf: [0xA5u8; kb_core::debug::FRAME_BYTES],
        };
        mirror::kb_mirror_set_frame(m.0, 0, frame.buf.as_ptr());

        // Act
        let writes = mirror_display(m.0, 0, 0);

        // Assert: the frame's data transaction comes last
        assert!(writes.ends_with(&kb_display::ssd1306::frame_data(&frame)));
    }
}

#[test]
fn mirror_display_too_small_keeps_the_writes() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let m = MirrorHandle::new();
        let needed = mirror::kb_mirror_display(m.0, 1, 0, null_mut(), 0);
        let mut small = vec![0u8; needed - 1];
        mirror::kb_mirror_display(m.0, 1, 0, small.as_mut_ptr(), small.len());

        // Act
        let again = mirror::kb_mirror_display(m.0, 1, 0, null_mut(), 0);

        // Assert
        assert_eq!(again, needed);
    }
}

#[test]
fn mirror_idle_draws_the_sleep_art() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange: the right screen lit, then the keyboard idles
        let m = MirrorHandle::new();
        let active = mirror_status(0);
        mirror::kb_mirror_set_status(m.0, &raw const active, null(), 0);
        mirror_display(m.0, 1, 0);
        let idle = mirror_status(1);
        mirror::kb_mirror_set_status(m.0, &raw const idle, null(), 0);
        mirror_display(m.0, 1, 100);

        // Act
        let art = mirror_display(m.0, 1, 100 + kb_display::anim::SLEEP_ART_MS / 2);

        // Assert: a new frame
        assert!(art.len() > kb_core::debug::FRAME_BYTES, "{}", art.len());
    }
}

#[test]
fn mirror_idle_goes_dark_after_the_sleep_art() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let m = MirrorHandle::new();
        let active = mirror_status(0);
        mirror::kb_mirror_set_status(m.0, &raw const active, null(), 0);
        mirror_display(m.0, 1, 0);
        let idle = mirror_status(1);
        mirror::kb_mirror_set_status(m.0, &raw const idle, null(), 0);
        mirror_display(m.0, 1, 100);

        // Act
        let after = mirror_display(m.0, 1, 100 + kb_display::anim::SLEEP_ART_MS);

        // Assert: display off, one short write
        assert!(!after.is_empty() && after.len() < 16, "{after:?}");
    }
}

#[test]
fn mirror_comes_back_on_after_going_dark() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange
        let m = MirrorHandle::new();
        let active = mirror_status(0);
        mirror::kb_mirror_set_status(m.0, &raw const active, null(), 0);
        mirror_display(m.0, 1, 0);
        let asleep = mirror_status(2);
        mirror::kb_mirror_set_status(m.0, &raw const asleep, null(), 0);
        mirror_display(m.0, 1, 10);
        mirror::kb_mirror_set_status(m.0, &raw const active, null(), 0);

        // Act
        let back = mirror_display(m.0, 1, 20);

        // Assert: display on, no second init, and the unchanged frame isn't resent
        let mut on = Vec::new();
        kb_display::ssd1306::display_on(|w| {
            on.extend_from_slice(&(w.len() as u16).to_le_bytes());
            on.extend_from_slice(w);
        });
        assert_eq!(back, on);
    }
}

#[test]
fn mirror_label_shows_the_layer_s_binding() {
    // SAFETY: every handle is null or a live one of this test (freed when it
    // drops), and every buffer is valid for the length passed with it.
    unsafe {
        // Arrange: Q on symbols is `!`
        let (layer, pos) = (1, Q);

        // Act
        let label = text(|b, c| mirror::kb_mirror_label(layer, pos, b, c));

        // Assert
        assert_eq!(label, "!");
    }
}
