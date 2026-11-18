//! Maps keys on a normal (laptop) keyboard to Corne positions.
//!
//! The mapping is positional: the key where a Corne key would sit, not the
//! key with the same legend. The outer column becomes Tab / Caps Lock / Shift
//! and the six thumb keys sit around the space bar. Edit `POSITIONS` to suit
//! your keyboard.
//!
//! ```text
//! Corne          0  1  2  3  4  5      6  7  8  9 10 11
//!               12 13 14 15 16 17     18 19 20 21 22 23
//!               24 25 26 27 28 29     30 31 32 33 34 35
//!                        36 37 38     39 40 41
//!
//! Laptop       Tab  Q  W  E  R  T      Y  U  I  O  P  [
//!             Caps  A  S  D  F  G      H  J  K  L  ;  '
//!            Shift  Z  X  C  V  B      N  M  ,  .  /  RShift
//!                Super Alt Space      AltGr Menu RCtrl
//! ```

/// Linux input key codes (`linux/input-event-codes.h`).
mod code {
    pub const ESC: u16 = 1;
    pub const TAB: u16 = 15;
    pub const LEFTBRACE: u16 = 26;
    pub const CAPSLOCK: u16 = 58;
    pub const APOSTROPHE: u16 = 40;
    pub const LEFTSHIFT: u16 = 42;
    pub const RIGHTSHIFT: u16 = 54;
    pub const SEMICOLON: u16 = 39;
    pub const COMMA: u16 = 51;
    pub const DOT: u16 = 52;
    pub const SLASH: u16 = 53;
    pub const LEFTALT: u16 = 56;
    pub const SPACE: u16 = 57;
    pub const RIGHTCTRL: u16 = 97;
    pub const RIGHTALT: u16 = 100;
    pub const LEFTMETA: u16 = 125;
    pub const RIGHTMETA: u16 = 126;
    pub const COMPOSE: u16 = 127;
}

/// Physical key that stops the simulator (and releases the keyboard grab).
pub const QUIT_KEY: u16 = code::ESC;

#[rustfmt::skip]
const POSITIONS: &[(u16, u8)] = &[
    (code::TAB, 0), (16, 1), (17, 2), (18, 3), (19, 4), (20, 5),
    (21, 6), (22, 7), (23, 8), (24, 9), (25, 10), (code::LEFTBRACE, 11),

    (code::CAPSLOCK, 12), (30, 13), (31, 14), (32, 15), (33, 16), (34, 17),
    (35, 18), (36, 19), (37, 20), (38, 21), (code::SEMICOLON, 22), (code::APOSTROPHE, 23),

    (code::LEFTSHIFT, 24), (44, 25), (45, 26), (46, 27), (47, 28), (48, 29),
    (49, 30), (50, 31), (code::COMMA, 32), (code::DOT, 33), (code::SLASH, 34), (code::RIGHTSHIFT, 35),

    (code::LEFTMETA, 36), (code::LEFTALT, 37), (code::SPACE, 38),
    (code::RIGHTALT, 39), (code::COMPOSE, 40), (code::RIGHTMETA, 40), (code::RIGHTCTRL, 41),
];

pub fn position(linux_code: u16) -> Option<u8> {
    POSITIONS
        .iter()
        .find(|(c, _)| *c == linux_code)
        .map(|&(_, p)| p)
}
