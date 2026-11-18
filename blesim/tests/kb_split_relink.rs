//! The firmware's split link (`kb-ble`) on the air: the left half restarts
//! while the right half stays up.
//!
//! The right half's GATT server remembers that the bonded left half
//! subscribed to the keys, so its first key notification on the new
//! connection asks for security (SMP Security Request) before the link is
//! encrypted. The left half encrypts with the stored bond, and trouble then
//! reports that as `PairingComplete` with the stored bond on the right half.
//! `kb_core::link::right_half_verdict` used to reject that ("paired, but not
//! with the stored bond") in a loop, so the halves never linked again until
//! the right half restarted; a link lost without any restart (out of range)
//! went the same way. Now it accepts its own bond reported as a pairing.

mod common;
mod kb;

use blesim::Air;
use embassy_futures::join::join;
use embassy_time::{Duration, Timer};
use kb::*;
use kb_ble::{LeftState, RightState};

#[test]
fn left_half_relinks_after_restarting_while_the_right_half_stays_up() {
    // Arrange: linked halves.
    let air = Air::new();
    let (mut left_flash, mut right_flash) = (RamFlash::new(), RamFlash::new());
    let split_server = kb_ble::split::split_server("right");
    let hid_server = kb_ble::hid::hid_server("left");
    let (left, restarted_left, right) = (
        LeftState::new(Box::leak(Box::new(kb_ble::log::LogRing::new()))),
        LeftState::new(Box::leak(Box::new(kb_ble::log::LogRing::new()))),
        RightState::new(Box::leak(Box::new(kb_ble::log::LogRing::new()))),
    );
    let (stop_left, never) = (Stop::new(), Stop::new());

    // Act: the left half restarts (same flash, fresh state).
    let (first, second) = run(
        join(
            async {
                left_half(&air, &left, &mut left_flash, &hid_server, &stop_left).await;
                left_half(&air, &restarted_left, &mut left_flash, &hid_server, &never).await;
            },
            right_half(&air, &right, &mut right_flash, &split_server, &never),
        ),
        async {
            let first = wait_until(Duration::from_secs(5), || {
                left_linked(&left) && right_linked(&right)
            })
            .await;
            // Past the right half's first key notification with the subscription.
            Timer::after(Duration::from_millis(500)).await;
            stop_left.signal(());
            let second = wait_until(Duration::from_secs(5), || {
                left_linked(&restarted_left) && right_linked(&right)
            })
            .await;
            (first, second)
        },
    );

    // Assert
    dump("restarted left", restarted_left.log);
    dump("right", right.log);
    assert!(first, "linked first");
    assert!(second, "linked again after the left half restarted");
}
