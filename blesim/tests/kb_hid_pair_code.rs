//! The firmware's Bluetooth keyboard (`kb-ble`) on the air: a computer with
//! a screen pairs with a code (passkey entry), typed on the keyboard.

mod common;
mod kb;
mod pairing;

use kb_core::link::PairRequest;
use pairing::{Typed, pair_with_code};
use trouble_host::prelude::SecurityLevel;

#[test]
fn computer_with_a_screen_pairs_with_the_code_typed_on_the_keyboard() {
    // Arrange: a DisplayYesNo computer on the open profile (in `pair_with_code`).
    let typed = Typed::TheCode;

    // Act
    let outcome = pair_with_code(typed);

    // Assert
    assert_eq!(outcome.asked, Some(PairRequest::TypeCode), "{outcome:?}");
    assert!(outcome.pc_code.is_some(), "the computer showed a code");
    assert_eq!(
        outcome.pc_level,
        Some(SecurityLevel::EncryptedAuthenticated),
        "MITM protected: {outcome:?}"
    );
    assert!(outcome.kb_connected, "the computer took the profile");
    assert!(!outcome.open_after, "the profile is taken");
    assert!(!outcome.prompt_after, "the prompt is gone");
    assert!(outcome.log.contains("host: paired with a code"));
}
