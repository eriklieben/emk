//! Key legends (`action_label`): what the emulator and Studio draw on a key.

use kb_core::action::{CAPS_WORD, NONE, TRANS, kp, sys};
use kb_core::keys::{
    BSLH, CAPS, COMMA, DOT, EQUAL, FSLH, GRAVE, INS, K, LBKT, LEFT, LGUI, MEH, MINUS, N0, N1, N2,
    N3, N4, N5, N6, N7, N8, N9, PSCRN, RALT, RBKT, RGUI, RIGHT, SEMI, SQT, lc, lg, ls,
};
use kb_core::{Action, SystemAction};
use kb_emu::action_label;

fn labels<const N: usize>(actions: [Action; N]) -> [String; N] {
    actions.map(action_label)
}

#[test]
fn shifted_number_row_shows_the_symbols() {
    // Arrange
    let keys = [N1, N2, N3, N4, N5, N6, N7, N8, N9, N0].map(|k| kp(ls(k)));

    // Act
    let got = labels(keys);

    // Assert
    assert_eq!(got, ["!", "@", "#", "$", "%", "^", "&", "*", "(", ")"]);
}

#[test]
fn shifted_punctuation_shows_the_symbols() {
    // Arrange
    let keys = [
        MINUS, EQUAL, LBKT, RBKT, BSLH, SEMI, SQT, GRAVE, COMMA, DOT, FSLH,
    ]
    .map(|k| kp(ls(k)));

    // Act
    let got = labels(keys);

    // Assert
    assert_eq!(
        got,
        ["_", "+", "{", "}", "|", ":", "\"", "~", "<", ">", "?"]
    );
}

#[test]
fn plain_punctuation_and_special_keys_have_short_names() {
    // Arrange
    let keys = [
        N0, MINUS, EQUAL, LBKT, RBKT, GRAVE, CAPS, PSCRN, INS, RALT, RGUI,
    ]
    .map(kp);

    // Act
    let got = labels(keys);

    // Assert
    assert_eq!(
        got,
        [
            "0", "-", "=", "[", "]", "`", "Caps", "PrtSc", "Ins", "AltGr", "Gui"
        ]
    );
}

#[test]
fn modifiers_on_a_key_are_shown_as_signs_in_front() {
    // Arrange
    let key = kp(lc(ls(LBKT)));

    // Act
    let got = action_label(key);

    // Assert: not a shifted symbol once Ctrl is in too
    assert_eq!(got, "⌃⇧[");
}

#[test]
fn meh_is_one_word() {
    // Arrange
    let key = kp(MEH);

    // Act
    let got = action_label(key);

    // Assert
    assert_eq!(got, "Meh");
}

#[test]
fn window_manager_moves_have_their_own_legends() {
    // Arrange
    let keys = [
        kp(lg(lc(RIGHT))),
        kp(lg(lc(ls(LEFT)))),
        kp(lg(lc(ls(RIGHT)))),
        kp(lg(DOT)),
        kp(lg(ls(K))),
    ];

    // Act
    let got = labels(keys);

    // Assert
    assert_eq!(got, ["WS ▶", "→ WS◀", "→ WS▶", "Col ▶", "Win ▲"]);
}

#[test]
fn gui_with_an_unknown_shortcut_is_a_plain_chord() {
    // Arrange: Super + Alt + 1 is no window manager action
    let key = kp(lg(kb_core::keys::la(N1)));

    // Act
    let got = action_label(key);

    // Assert
    assert_eq!(got, "⌥◆1");
}

#[test]
fn gui_on_its_own_is_the_modifier_name() {
    // Arrange
    let key = kp(LGUI);

    // Act
    let got = action_label(key);

    // Assert
    assert_eq!(got, "Gui");
}

#[test]
fn transparent_and_empty_keys_have_no_legend() {
    // Arrange
    let keys = [TRANS, NONE];

    // Act
    let got = labels(keys);

    // Assert
    assert_eq!(got, ["", ""]);
}

#[test]
fn caps_word_takes_two_lines() {
    // Arrange
    let key = CAPS_WORD;

    // Act
    let got = action_label(key);

    // Assert
    assert_eq!(got, "Caps\nWord");
}

#[test]
fn every_system_action_has_a_legend() {
    // Arrange
    let keys = SystemAction::ALL.map(sys);

    // Act
    let got = labels(keys);

    // Assert
    assert_eq!(
        got,
        [
            "Profile ◀",
            "Profile ▶",
            "Profile clear",
            "USB/BT",
            "Studio",
            "Link?",
            "Radio"
        ]
    );
}
