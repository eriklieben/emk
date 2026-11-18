//! The firmware's split link (`kb-ble`) on the air: the left half's key
//! presses reach the right half, whose screen rains for them too.

mod common;
mod kb;

use blesim::Air;
use embassy_futures::join::join;
use embassy_time::Duration;
use kb::*;
use kb_ble::{LeftState, RightState};

#[test]
fn left_halfs_key_presses_reach_the_right_half() {
    // Arrange
    let air = Air::new();
    let left = LeftState::new(Box::leak(Box::new(kb_ble::log::LogRing::new())));
    let right = RightState::new(Box::leak(Box::new(kb_ble::log::LogRing::new())));
    let (mut left_flash, mut right_flash) = (RamFlash::new(), RamFlash::new());
    let split_server = kb_ble::split::split_server("right");
    let hid_server = kb_ble::hid::hid_server("left");
    let never = Stop::new();

    // Act
    let (linked, arrived) = run(
        join(
            left_half(&air, &left, &mut left_flash, &hid_server, &never),
            right_half(&air, &right, &mut right_flash, &split_server, &never),
        ),
        async {
            let linked = wait_until(Duration::from_secs(5), || {
                left_linked(&left) && right_linked(&right)
            })
            .await;
            left.set_key_presses(41);
            left.set_key_presses(42);
            let arrived = wait_until(Duration::from_secs(1), || right.left_presses() == 42).await;
            (linked, arrived)
        },
    );

    // Assert
    dump("left", left.log);
    dump("right", right.log);
    assert!(linked, "both halves linked");
    assert!(arrived, "the right half has the left half's presses");
}
