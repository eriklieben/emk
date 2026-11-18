//! The firmware's Bluetooth keyboard (`kb-ble`) on the air: a wrong code
//! typed on the keyboard, and the computer doesn't pair.

mod common;
mod kb;
mod pairing;

use kb_core::link::PairRequest;
use pairing::{Typed, pair_with_code};

#[test]
fn wrong_code_typed_on_the_keyboard_fails_the_pairing() {
    // Arrange: a DisplayYesNo computer on the open profile (in `pair_with_code`).
    let typed = Typed::AnotherCode;

    // Act
    let outcome = pair_with_code(typed);

    // Assert
    assert_eq!(outcome.asked, Some(PairRequest::TypeCode), "{outcome:?}");
    assert!(
        outcome.pc_failed || outcome.pc_dropped,
        "the computer's pairing failed: {outcome:?}"
    );
    assert_eq!(outcome.pc_level, None, "no pairing");
    assert!(!outcome.kb_connected);
    assert!(outcome.open_after, "the profile stays open");
    assert!(!outcome.prompt_after, "the prompt is gone");
    assert!(outcome.log.contains("host: pairing failed"));
}
