//! A clear from the computer (USB Raw HID in the firmware, delivered to the
//! left half's inbox) goes over the split link to the right half's screen.

mod common;
mod kb;

use blesim::Air;
use embassy_futures::join::join;
use embassy_time::Duration;
use kb::*;
use kb_ble::{LeftState, RightState};
use kb_core::host::{HostMessage, Text};

#[test]
fn clear_reaches_the_right_half() {
    // Arrange: the halves linked and a notification on the right screen (in the scenario).
    let air = Air::new();
    let left = LeftState::new(Box::leak(Box::new(kb_ble::log::LogRing::new())));
    let right = RightState::new(Box::leak(Box::new(kb_ble::log::LogRing::new())));
    let (mut left_flash, mut right_flash) = (RamFlash::new(), RamFlash::new());
    let split_server = kb_ble::split::split_server("right");
    let hid_server = kb_ble::hid::hid_server("left");
    let never = Stop::new();
    let shown = HostMessage::Notify(Text::new("Standup"));

    // Act: the computer clears it.
    let (linked, first) = run(
        join(
            left_half(&air, &left, &mut left_flash, &hid_server, &never),
            right_half(&air, &right, &mut right_flash, &split_server, &never),
        ),
        async {
            let linked = wait_until(Duration::from_secs(5), || {
                left_linked(&left) && right_linked(&right)
            })
            .await;
            left.inbox.deliver(shown);
            wait_until(Duration::from_secs(2), || right.notification().is_some()).await;
            let first = right.take_notification();
            left.inbox.deliver(HostMessage::Clear);
            wait_until(Duration::from_secs(2), || right.notification().is_some()).await;
            (linked, first)
        },
    );

    // Assert
    dump("left", left.log);
    dump("right", right.log);
    assert!(linked, "both halves linked");
    assert_eq!(first, Some(shown), "the notification came first");
    assert_eq!(right.notification(), Some(HostMessage::Clear));
}
