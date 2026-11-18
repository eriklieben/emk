//! The firmware's split link (`kb-ble`) on the air: while the keyboard
//! rests the left half slows the link down, keys still arrive, and typing
//! again makes it fast.

mod common;
mod kb;

use blesim::Air;
use embassy_futures::join::join;
use embassy_time::Duration;
use kb::*;
use kb_ble::{LeftState, RightState};

#[test]
fn resting_slows_the_link_and_keys_still_arrive() {
    // Arrange
    let air = Air::new();
    let left = LeftState::new(Box::leak(Box::new(kb_ble::log::LogRing::new())));
    let right = RightState::new(Box::leak(Box::new(kb_ble::log::LogRing::new())));
    let (mut left_flash, mut right_flash) = (RamFlash::new(), RamFlash::new());
    let split_server = kb_ble::split::split_server("right");
    let hid_server = kb_ble::hid::hid_server("left");
    let never = Stop::new();

    // Act
    let (linked, slowed, key, fast) = run(
        join(
            left_half(&air, &left, &mut left_flash, &hid_server, &never),
            right_half(&air, &right, &mut right_flash, &split_server, &never),
        ),
        async {
            let linked = wait_until(Duration::from_secs(5), || {
                left_linked(&left) && right_linked(&right)
            })
            .await;
            left.set_resting(true);
            let slowed = wait_until(Duration::from_secs(1), || {
                log_text(left.log).contains("split: link resting (45000 us)")
            })
            .await;
            right.set_keys(1 << 7);
            let key = left.remote_events.receive().await;
            left.set_resting(false);
            let fast = wait_until(Duration::from_secs(1), || {
                log_text(left.log).contains("split: link fast (7500 us)")
            })
            .await;
            (linked, slowed, key, fast)
        },
    );

    // Assert
    dump("left", left.log);
    assert!(linked, "both halves linked");
    assert!(slowed, "the link slowed down while resting");
    assert_eq!(key, (7, true));
    assert!(fast, "the link is fast again");
}
