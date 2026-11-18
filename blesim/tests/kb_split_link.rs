//! The firmware's split link (`kb-ble`) on the air: two fresh halves pair,
//! link, and the right half's keys reach the left half's key loop.

mod common;
mod kb;

use blesim::Air;
use embassy_futures::join::join;
use embassy_time::Duration;
use kb::*;
use kb_ble::{LeftState, RightState};

#[test]
fn fresh_halves_pair_link_and_carry_the_right_halfs_keys() {
    // Arrange
    let air = Air::new();
    let left = LeftState::new(Box::leak(Box::new(kb_ble::log::LogRing::new())));
    let right = RightState::new(Box::leak(Box::new(kb_ble::log::LogRing::new())));
    let (mut left_flash, mut right_flash) = (RamFlash::new(), RamFlash::new());
    let split_server = kb_ble::split::split_server("right");
    let hid_server = kb_ble::hid::hid_server("left");
    let never = Stop::new();

    // Act
    let (linked, events) = run(
        join(
            left_half(&air, &left, &mut left_flash, &hid_server, &never),
            right_half(&air, &right, &mut right_flash, &split_server, &never),
        ),
        async {
            let linked = wait_until(Duration::from_secs(5), || {
                left_linked(&left) && right_linked(&right)
            })
            .await;
            right.set_keys(1 << 3 | 1 << 20);
            let mut events = vec![
                left.remote_events.receive().await,
                left.remote_events.receive().await,
            ];
            right.set_keys(1 << 20);
            events.push(left.remote_events.receive().await);
            (linked, events)
        },
    );

    // Assert
    dump("left", left.log);
    dump("right", right.log);
    assert!(linked, "both halves linked");
    assert_eq!(events, [(3, true), (20, true), (3, false)]);
    assert_eq!(air.pairing_requests(), 1);
    assert!(log_text(right.log).contains("split: left half paired"));
}
