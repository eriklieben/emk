//! HID usages with ZMK's key names, plus ZMK's modifier functions (`LS()`, `LC()`, ...).
#![allow(
    missing_docs,
    reason = "each key constant is the ZMK key name of its HID usage"
)]

use crate::action::Key;

/// Left Control in the HID modifier byte.
pub const MOD_LCTL: u8 = 0x01;
/// Left Shift in the HID modifier byte.
pub const MOD_LSFT: u8 = 0x02;
/// Left Alt in the HID modifier byte.
pub const MOD_LALT: u8 = 0x04;
/// Left GUI (Super) in the HID modifier byte.
pub const MOD_LGUI: u8 = 0x08;
/// Right Control in the HID modifier byte.
pub const MOD_RCTL: u8 = 0x10;
/// Right Shift in the HID modifier byte.
pub const MOD_RSFT: u8 = 0x20;
/// Right Alt (`AltGr`) in the HID modifier byte.
pub const MOD_RALT: u8 = 0x40;
/// Right GUI (Super) in the HID modifier byte.
pub const MOD_RGUI: u8 = 0x80;

/// `k` with Left Control held (ZMK's `LC()`).
#[must_use]
pub const fn lc(k: Key) -> Key {
    k.with_mods(MOD_LCTL)
}
/// `k` with Left Shift held (ZMK's `LS()`).
#[must_use]
pub const fn ls(k: Key) -> Key {
    k.with_mods(MOD_LSFT)
}
/// `k` with Left Alt held (ZMK's `LA()`).
#[must_use]
pub const fn la(k: Key) -> Key {
    k.with_mods(MOD_LALT)
}
/// `k` with Left GUI held (ZMK's `LG()`).
#[must_use]
pub const fn lg(k: Key) -> Key {
    k.with_mods(MOD_LGUI)
}
/// `k` with Right Control held (ZMK's `RC()`).
#[must_use]
pub const fn rc(k: Key) -> Key {
    k.with_mods(MOD_RCTL)
}
/// `k` with Right Shift held (ZMK's `RS()`).
#[must_use]
pub const fn rs(k: Key) -> Key {
    k.with_mods(MOD_RSFT)
}
/// `k` with Right Alt held (ZMK's `RA()`).
#[must_use]
pub const fn ra(k: Key) -> Key {
    k.with_mods(MOD_RALT)
}
/// `k` with Right GUI held (ZMK's `RG()`).
#[must_use]
pub const fn rg(k: Key) -> Key {
    k.with_mods(MOD_RGUI)
}

const fn k(code: u8) -> Key {
    Key::new(code)
}

// Letters
pub const A: Key = k(0x04);
pub const B: Key = k(0x05);
pub const C: Key = k(0x06);
pub const D: Key = k(0x07);
pub const E: Key = k(0x08);
pub const F: Key = k(0x09);
pub const G: Key = k(0x0A);
pub const H: Key = k(0x0B);
pub const I: Key = k(0x0C);
pub const J: Key = k(0x0D);
pub const K: Key = k(0x0E);
pub const L: Key = k(0x0F);
pub const M: Key = k(0x10);
pub const N: Key = k(0x11);
pub const O: Key = k(0x12);
pub const P: Key = k(0x13);
pub const Q: Key = k(0x14);
pub const R: Key = k(0x15);
pub const S: Key = k(0x16);
pub const T: Key = k(0x17);
pub const U: Key = k(0x18);
pub const V: Key = k(0x19);
pub const W: Key = k(0x1A);
pub const X: Key = k(0x1B);
pub const Y: Key = k(0x1C);
pub const Z: Key = k(0x1D);

// Number row
pub const N1: Key = k(0x1E);
pub const N2: Key = k(0x1F);
pub const N3: Key = k(0x20);
pub const N4: Key = k(0x21);
pub const N5: Key = k(0x22);
pub const N6: Key = k(0x23);
pub const N7: Key = k(0x24);
pub const N8: Key = k(0x25);
pub const N9: Key = k(0x26);
pub const N0: Key = k(0x27);

// Control and punctuation
pub const RET: Key = k(0x28);
pub const ESC: Key = k(0x29);
pub const BSPC: Key = k(0x2A);
pub const TAB: Key = k(0x2B);
pub const SPACE: Key = k(0x2C);
pub const MINUS: Key = k(0x2D);
pub const EQUAL: Key = k(0x2E);
pub const LBKT: Key = k(0x2F);
pub const RBKT: Key = k(0x30);
pub const BSLH: Key = k(0x31);
pub const SEMI: Key = k(0x33);
pub const SQT: Key = k(0x34);
pub const GRAVE: Key = k(0x35);
pub const COMMA: Key = k(0x36);
pub const DOT: Key = k(0x37);
pub const FSLH: Key = k(0x38);
pub const CAPS: Key = k(0x39);

// Function keys
pub const F1: Key = k(0x3A);
pub const F2: Key = k(0x3B);
pub const F3: Key = k(0x3C);
pub const F4: Key = k(0x3D);
pub const F5: Key = k(0x3E);
pub const F6: Key = k(0x3F);
pub const F7: Key = k(0x40);
pub const F8: Key = k(0x41);
pub const F9: Key = k(0x42);
pub const F10: Key = k(0x43);
pub const F11: Key = k(0x44);
pub const F12: Key = k(0x45);
pub const F13: Key = k(0x68);
pub const F14: Key = k(0x69);
pub const F15: Key = k(0x6A);
pub const F16: Key = k(0x6B);
pub const F17: Key = k(0x6C);
pub const F18: Key = k(0x6D);
pub const F19: Key = k(0x6E);
pub const F20: Key = k(0x6F);
pub const F21: Key = k(0x70);
pub const F22: Key = k(0x71);
pub const F23: Key = k(0x72);
pub const F24: Key = k(0x73);

// Navigation
pub const PSCRN: Key = k(0x46);
pub const SLCK: Key = k(0x47);
pub const PAUSE_BREAK: Key = k(0x48);
pub const INS: Key = k(0x49);
pub const HOME: Key = k(0x4A);
pub const PG_UP: Key = k(0x4B);
pub const DEL: Key = k(0x4C);
pub const END: Key = k(0x4D);
pub const PG_DN: Key = k(0x4E);
pub const RIGHT: Key = k(0x4F);
pub const LEFT: Key = k(0x50);
pub const DOWN: Key = k(0x51);
pub const UP: Key = k(0x52);

// Modifiers
pub const LCTRL: Key = k(0xE0);
pub const LSHFT: Key = k(0xE1);
pub const LALT: Key = k(0xE2);
pub const LGUI: Key = k(0xE3);
pub const RCTRL: Key = k(0xE4);
pub const RSHFT: Key = k(0xE5);
pub const RALT: Key = k(0xE6);
pub const RGUI: Key = k(0xE7);

/// Ctrl + Shift + Alt.
pub const MEH: Key = ls(lc(LALT));

// Shifted symbols (US layout)
pub const EXCL: Key = ls(N1);
pub const AT: Key = ls(N2);
pub const HASH: Key = ls(N3);
pub const DLLR: Key = ls(N4);
pub const PRCNT: Key = ls(N5);
pub const CARET: Key = ls(N6);
pub const AMPS: Key = ls(N7);
pub const ASTRK: Key = ls(N8);
pub const LPAR: Key = ls(N9);
pub const RPAR: Key = ls(N0);
pub const UNDER: Key = ls(MINUS);
pub const PLUS: Key = ls(EQUAL);
pub const LBRC: Key = ls(LBKT);
pub const RBRC: Key = ls(RBKT);
pub const PIPE: Key = ls(BSLH);
pub const TILDE: Key = ls(GRAVE);

/// Whether `code` is a letter, `A` to `Z`.
#[must_use]
pub const fn is_alpha(code: u8) -> bool {
    code >= 0x04 && code <= 0x1D
}

/// Whether `code` is a digit on the number row, `1` to `0`.
#[must_use]
pub const fn is_digit(code: u8) -> bool {
    code >= 0x1E && code <= 0x27
}

/// Short name of a HID usage, for logs and tests.
#[must_use]
pub fn name(code: u8) -> &'static str {
    const LETTERS: [&str; 26] = [
        "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R",
        "S", "T", "U", "V", "W", "X", "Y", "Z",
    ];
    const DIGITS: [&str; 10] = ["N1", "N2", "N3", "N4", "N5", "N6", "N7", "N8", "N9", "N0"];
    const FKEYS: [&str; 12] = [
        "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12",
    ];
    const FKEYS_13_24: [&str; 12] = [
        "F13", "F14", "F15", "F16", "F17", "F18", "F19", "F20", "F21", "F22", "F23", "F24",
    ];
    const MODS: [&str; 8] = [
        "LCTRL", "LSHFT", "LALT", "LGUI", "RCTRL", "RSHFT", "RALT", "RGUI",
    ];
    match code {
        0x04..=0x1D => LETTERS[(code - 0x04) as usize],
        0x1E..=0x27 => DIGITS[(code - 0x1E) as usize],
        0x28 => "RET",
        0x29 => "ESC",
        0x2A => "BSPC",
        0x2B => "TAB",
        0x2C => "SPACE",
        0x2D => "MINUS",
        0x2E => "EQUAL",
        0x2F => "LBKT",
        0x30 => "RBKT",
        0x31 => "BSLH",
        0x33 => "SEMI",
        0x34 => "SQT",
        0x35 => "GRAVE",
        0x36 => "COMMA",
        0x37 => "DOT",
        0x38 => "FSLH",
        0x39 => "CAPS",
        0x3A..=0x45 => FKEYS[(code - 0x3A) as usize],
        0x68..=0x73 => FKEYS_13_24[(code - 0x68) as usize],
        0x46 => "PSCRN",
        0x47 => "SLCK",
        0x48 => "PAUSE_BREAK",
        0x49 => "INS",
        0x4A => "HOME",
        0x4B => "PG_UP",
        0x4C => "DEL",
        0x4D => "END",
        0x4E => "PG_DN",
        0x4F => "RIGHT",
        0x50 => "LEFT",
        0x51 => "DOWN",
        0x52 => "UP",
        0xE0..=0xE7 => MODS[(code - 0xE0) as usize],
        _ => "?",
    }
}
