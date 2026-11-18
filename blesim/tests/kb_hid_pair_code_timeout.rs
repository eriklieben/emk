//! The firmware's Bluetooth keyboard (`kb-ble`) on the air: a pairing code
//! nobody types on the keyboard is rejected after the timeout.

mod common;
mod kb;
mod pairing;

use pairing::{Typed, pair_with_code};

#[test]
fn untyped_pairing_code_is_rejected_after_the_timeout() {
    // Arrange: a DisplayYesNo computer on the open profile (in `pair_with_code`),
    // the keyboard's 300 ms timeout (`kb::TIMING`).
    let typed = Typed::Nothing;

    // Act
    let outcome = pair_with_code(typed);

    // Assert
    assert!(
        outcome.asked.is_some(),
        "the keyboard asked for the code: {outcome:?}"
    );
    assert!(
        outcome.pc_failed || outcome.pc_dropped,
        "the computer's pairing failed: {outcome:?}"
    );
    assert_eq!(outcome.pc_level, None, "no pairing");
    assert!(!outcome.kb_connected);
    assert!(outcome.open_after, "the profile stays open");
    assert!(!outcome.prompt_after, "the prompt is gone");
    assert!(
        outcome
            .log
            .contains("host: pairing code not answered in time")
    );
}
