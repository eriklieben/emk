//! The firmware's Bluetooth keyboard (`kb-ble`) on the air: the previous
//! profile key wraps from the first profile to the last.

mod common;
mod kb;

use std::sync::atomic::Ordering;

use blesim::Air;
use embassy_time::Duration;
use kb::*;
use kb_ble::{Command, LeftState};

#[test]
fn previous_profile_of_the_first_is_the_last() {
    // Arrange
    let air = Air::new();
    let mut flash = RamFlash::new();
    let hid_server = kb_ble::hid::hid_server("left");
    let left = LeftState::new(Box::leak(Box::new(kb_ble::log::LogRing::new())));
    let never = Stop::new();

    // Act
    let switched = run(
        left_half(&air, &left, &mut flash, &hid_server, &never),
        async {
            left.hid.commands.send(Command::Prev).await;
            wait_until(Duration::from_secs(1), || {
                left.hid.active.load(Ordering::Relaxed) != 0
            })
            .await
        },
    );

    // Assert
    dump("left", left.log);
    assert!(switched);
    assert_eq!(
        left.hid.active.load(Ordering::Relaxed) as usize,
        kb_ble::hid::PROFILES - 1
    );
    assert!(log_text(left.log).contains("host: Prev -> profile 10"));
}
