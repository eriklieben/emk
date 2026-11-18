//! The firmware's Bluetooth keyboard (`kb-ble`) on the air: a profile key
//! selects a profile, and the keyboard starts on it after a restart.

mod common;
mod kb;

use std::sync::atomic::Ordering;

use blesim::Air;
use embassy_time::Duration;
use kb::*;
use kb_ble::{Command, LeftState};

#[test]
fn selected_profile_is_kept_across_a_restart() {
    // Arrange
    let air = Air::new();
    let mut flash = RamFlash::new();
    let hid_server = kb_ble::hid::hid_server("left");
    let left = LeftState::new(Box::leak(Box::new(kb_ble::log::LogRing::new())));
    let restarted = LeftState::new(Box::leak(Box::new(kb_ble::log::LogRing::new())));
    let never = Stop::new();

    // Act: select profile 5, then restart the keyboard.
    run(
        left_half(&air, &left, &mut flash, &hid_server, &never),
        async {
            left.hid.commands.send(Command::Select(4)).await;
            wait_until(Duration::from_secs(1), || {
                log_text(left.log).contains("host: Select(4) -> profile 5")
            })
            .await;
        },
    );
    let air = Air::new();
    run(
        left_half(&air, &restarted, &mut flash, &hid_server, &never),
        wait_until(Duration::from_secs(1), || {
            log_text(restarted.log).contains("host: profile")
        }),
    );

    // Assert
    dump("left", left.log);
    dump("left after the restart", restarted.log);
    assert_eq!(left.hid.active.load(Ordering::Relaxed), 4, "profile 5");
    assert!(left.hid.open.load(Ordering::Relaxed), "nobody paired on it");
    assert_eq!(restarted.hid.active.load(Ordering::Relaxed), 4);
    assert!(log_text(restarted.log).contains("host: profile 5 (open)"));
}
