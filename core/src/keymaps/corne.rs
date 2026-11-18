//! Erik's Corne layout, translated by hand from
//! `zmk-config/main/config/corne.keymap`.
//!
//! Key positions (same numbering as ZMK `key-positions`):
//!
//! ```text
//!  0  1  2  3  4  5      6  7  8  9 10 11
//! 12 13 14 15 16 17     18 19 20 21 22 23
//! 24 25 26 27 28 29     30 31 32 33 34 35
//!          36 37 38     39 40 41
//! ```

use crate::action::Direction::{Down, Left, Right, Up};
use crate::action::MouseButton::{self, Middle};
use crate::action::NONE;
use crate::action::SystemAction::{BtClear, BtNext, BtPrev, RadioPower, ShowLink, StudioUnlock};
use crate::action::{
    Action, CAPS_WORD, MOUSE_PRECISION, TRANS, click, kp, mkey, mkp, mmv, mo, mt, scroll, sys,
};
use crate::engine::{Combo, ConditionalLayer, Keymap};
#[allow(
    clippy::wildcard_imports,
    reason = "the keymap is a table of key names"
)]
use crate::keys::*;

pub const KEYS: usize = 42;

const ___: Action = TRANS;

#[rustfmt::skip]
const DEFAULT: [Action; KEYS] = [
    kp(ESC),         kp(Q), kp(W), kp(E), kp(R),    kp(T),       kp(Y),   kp(U),         kp(I),    kp(O),     kp(P),    kp(BSLH),
    mt(LALT, TAB),   kp(A), kp(S), kp(D), kp(F),    kp(G),       kp(H),   kp(J),         kp(K),    kp(L),     kp(SEMI), kp(SQT),
    kp(LCTRL),       kp(Z), kp(X), kp(C), kp(V),    kp(B),       kp(N),   kp(M),         kp(FSLH), kp(COMMA), kp(DOT),  kp(RSHFT),
                                   mo(1), kp(BSPC), kp(SPACE),   kp(RET), kp(DEL),  mo(2),
];

#[rustfmt::skip]
const SYMBOLS: [Action; KEYS] = [
    kp(DOT), kp(EXCL), kp(AT), kp(HASH), kp(DLLR), kp(PRCNT),    kp(CARET), kp(AMPS),  kp(ASTRK), kp(LPAR), kp(RPAR), kp(PIPE),
    ___,     kp(N1),   kp(N2), kp(N3),   kp(N4),   kp(N5),       kp(MINUS), kp(EQUAL), kp(GRAVE), kp(LBRC), kp(RBRC), ___,
    ___,     kp(N6),   kp(N7), kp(N8),   kp(N9),   kp(N0),       kp(UNDER), kp(PLUS),  kp(TILDE), kp(LBKT), kp(RBKT), ___,
                                         ___,      ___,  ___,    ___,       ___,       ___,
];

/// Nav: F keys, arrows on HJKL with Home/End and pages under them, and the
/// mouse on the left hand, mirroring the arrows:
/// - S D F G move the pointer ◀ ▼ ▲ ▶ (as H J K L), Tab held = precise (slow);
/// - A middle click (always: A is empty on nav otherwise);
/// - once the pointer has moved since nav came up ("mouse mode", the MOUSE
///   bar on the left screen), W E R T scroll ◀ ▼ ▲ ▶ (left/right: horizontal
///   pan) and the Backspace / Space thumbs are left / right click; before
///   that they are F2-F5 and Backspace / Space (`mk:`).
#[rustfmt::skip]
const NAV: [Action; KEYS] = [
    ___,          kp(F1), mkey(scroll(Left), F2), mkey(scroll(Down), F3), mkey(scroll(Up), F4), mkey(scroll(Right), F5), kp(F6),   kp(F7),    kp(F8),    kp(F9),    kp(F10), kp(lg(L)),
    MOUSE_PRECISION, mkp(Middle),   mmv(Left),        mmv(Down),        mmv(Up),       mmv(Right),      kp(LEFT), kp(DOWN),  kp(UP),    kp(RIGHT), kp(F11), ___,
    ___,          sys(BtPrev),      sys(BtNext),      ___,              sys(ShowLink), sys(RadioPower), kp(HOME), kp(PG_DN), kp(PG_UP), kp(END),   kp(F12), sys(StudioUnlock),
                                    ___,              mkey(click(MouseButton::Left), BSPC), mkey(click(MouseButton::Right), SPACE),       ___,      ___,       ___,
];

const ___X: Action = NONE;

/// Window manager layer for Hyprland and erikwm: active while both symbols
/// and nav are held. Laid out for muscle memory from the other layers:
/// - go to workspace N: where digit N is on symbols (1-5 on A..G, 6-0 on Z..B);
/// - send the window to workspace N: where Shift+N is on symbols (!@#$% / ^&*());
/// - reorder: on nav's arrow keys H J K L (column left, window down/up, column right);
/// - previous/next workspace (→ = take the window along): below them, where Home..End are;
/// - Mod+W (cycle the column width, i.e. resize) on the Tab position.
#[rustfmt::skip]
const WM: [Action; KEYS] = [
    kp(lg(W)), kp(lg(ls(N1))), kp(lg(ls(N2))), kp(lg(ls(N3))), kp(lg(ls(N4))), kp(lg(ls(N5))),    kp(lg(ls(N6))),    kp(lg(ls(N7))),        kp(lg(ls(N8))),         kp(lg(ls(N9))),  kp(lg(ls(N0))), ___X,
    ___X, kp(lg(N1)),     kp(lg(N2)),     kp(lg(N3)),     kp(lg(N4)),     kp(lg(N5)),        kp(lg(COMMA)),     kp(lg(ls(J))),         kp(lg(ls(K))),          kp(lg(DOT)),     ___X,           ___X,
    ___X, kp(lg(N6)),     kp(lg(N7)),     kp(lg(N8)),     kp(lg(N9)),     kp(lg(N0)),        kp(lg(lc(LEFT))),  kp(lg(lc(ls(LEFT)))),  kp(lg(lc(ls(RIGHT)))),  kp(lg(lc(RIGHT))), ___X,         ___X,
                                      ___,            ___X,           ___X,              ___X,              ___X,                  ___,
];

const COMBO_TIMEOUT_MS: u32 = 50;

/// Typing combos ignore presses that come within this time of the previous
/// key, so rolling over e.g. "xc" while typing never copies (ZMK
/// `require-prior-idle-ms`).
const COMBO_PRIOR_IDLE_MS: u32 = 150;

const fn combo(
    name: &'static str,
    keys: &'static [u8],
    action: Action,
    layers: &'static [u8],
) -> Combo<'static> {
    Combo {
        name,
        keys,
        action,
        layers,
        timeout_ms: COMBO_TIMEOUT_MS,
        require_prior_idle_ms: COMBO_PRIOR_IDLE_MS,
    }
}

pub static COMBOS: [Combo<'static>; 6] = [
    combo("cut", &[25, 26], kp(lc(X)), &[0]),
    combo("copy", &[26, 27], kp(lc(C)), &[0]),
    combo("paste", &[27, 28], kp(lc(V)), &[0]),
    combo("caps_word", &[5, 6], CAPS_WORD, &[0]),
    combo("escape", &[19, 20], kp(ESC), &[0]),
    // Both profile keys at once on the nav layer: forget the selected
    // profile's computer (Profile ◀ + Profile ▶).
    Combo {
        require_prior_idle_ms: 0,
        ..combo("reset_bluetooth", &[25, 26], sys(BtClear), &[2])
    },
];

/// Layers in the keymap.
pub const LAYER_COUNT: usize = 4;

pub static LAYERS: [[Action; KEYS]; LAYER_COUNT] = [DEFAULT, SYMBOLS, NAV, WM];

/// symbols + nav → wm (ZMK `conditional_layers`).
pub static CONDITIONAL_LAYERS: [ConditionalLayer<'static>; 1] = [ConditionalLayer {
    if_layers: &[1, 2],
    then_layer: 3,
}];

pub static KEYMAP: Keymap<'static, KEYS> = Keymap {
    layers: &LAYERS,
    layer_names: &["default", "symbols", "nav", "wm"],
    combos: &COMBOS,
    conditional_layers: &CONDITIONAL_LAYERS,
};

/// Which half a matrix belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// Key position for `row`, `col` of one half's 4 × 6 matrix, in ZMK's
/// `col-gpios` order: left column 0 is the outer pinky column, right column
/// 0 is the inner (Y) column. Row 3 only has the three thumb keys.
#[must_use]
pub const fn matrix_position(side: Side, row: usize, col: usize) -> Option<u8> {
    if col >= 6 {
        return None;
    }
    let pos = match (side, row) {
        (Side::Left, 0..=2) => row * 12 + col,
        (Side::Right, 0..=2) => row * 12 + 6 + col,
        (Side::Left, 3) if col >= 3 => 36 + col - 3,
        (Side::Right, 3) if col <= 2 => 39 + col,
        _ => return None,
    };
    Some(pos as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn left_outer_top_key_is_position_0() {
        // Arrange
        let (row, col) = (0, 0);

        // Act
        let pos = matrix_position(Side::Left, row, col);

        // Assert
        assert_eq!(pos, Some(0));
    }

    #[test]
    fn right_inner_top_key_is_position_6() {
        // Arrange
        let (row, col) = (0, 0);

        // Act
        let pos = matrix_position(Side::Right, row, col);

        // Assert
        assert_eq!(pos, Some(6));
    }

    #[test]
    fn thumb_rows_map_to_36_through_41() {
        // Arrange
        let left = [(3, 3), (3, 4), (3, 5)];
        let right = [(3, 0), (3, 1), (3, 2)];

        // Act
        let positions: Vec<_> = left
            .iter()
            .map(|&(r, c)| matrix_position(Side::Left, r, c))
            .chain(
                right
                    .iter()
                    .map(|&(r, c)| matrix_position(Side::Right, r, c)),
            )
            .collect();

        // Assert
        assert_eq!(positions, [36, 37, 38, 39, 40, 41].map(Some));
    }

    #[test]
    fn unused_thumb_row_columns_have_no_key() {
        // Arrange
        let (row, col) = (3, 0);

        // Act
        let pos = matrix_position(Side::Left, row, col);

        // Assert
        assert_eq!(pos, None);
    }

    #[test]
    fn every_position_is_used_exactly_once() {
        // Arrange
        let mut seen = [0u8; KEYS];

        // Act
        for side in [Side::Left, Side::Right] {
            for row in 0..4 {
                for col in 0..6 {
                    if let Some(pos) = matrix_position(side, row, col) {
                        seen[pos as usize] += 1;
                    }
                }
            }
        }

        // Assert
        assert!(seen.iter().all(|&n| n == 1));
    }
}
