//! A message from the computer that arrives while the right half is away
//! (off, or out of range) goes to it once the halves link.

mod common;
mod kb;
mod raw_hid;

use blesim::Air;
use common::{Resources, computer, runner};
use embassy_futures::join::join4;
use embassy_time::{Duration, Timer};
use kb::*;
use kb_ble::{LeftState, RightState};
use kb_core::host::{HostMessage, Text};
use raw_hid::{Write, reports, send};

#[test]
fn message_sent_before_the_halves_link_reaches_the_right_half_after() {
    // Arrange
    let air = Air::new();
    let left = LeftState::new(Box::leak(Box::new(kb_ble::log::LogRing::new())));
    let right = RightState::new(Box::leak(Box::new(kb_ble::log::LogRing::new())));
    let (mut left_flash, mut right_flash) = (RamFlash::new(), RamFlash::new());
    let split_server = kb_ble::split::split_server("right");
    let hid_server = kb_ble::hid::hid_server("left");
    let never = Stop::new();
    let right_on = Stop::new();
    let mut pc_res = Resources::new();
    let pc = computer(air.controller("pc"), &mut pc_res);
    let mut central = pc.central();

    // Act
    let (sent, arrived) = run(
        join4(
            left_half(&air, &left, &mut left_flash, &hid_server, &never),
            async {
                // The right half is switched on after the message.
                right_on.wait().await;
                right_half(&air, &right, &mut right_flash, &split_server, &never).await;
            },
            runner(&pc),
            async {},
        ),
        async {
            let sent = send(
                &pc,
                &mut central,
                &left,
                &reports("Standup in 5", 1),
                Write::WithoutResponse,
                || left.inbox.screen.peek().is_some(),
            )
            .await;
            Timer::after(Duration::from_millis(100)).await;
            right_on.signal(());
            let arrived =
                wait_until(Duration::from_secs(10), || right.notification().is_some()).await;
            (sent, arrived)
        },
    );

    // Assert
    dump("left", left.log);
    dump("right", right.log);
    assert!(sent, "the computer wrote report 3");
    assert!(arrived, "the right half got it after linking");
    assert_eq!(
        right.notification(),
        Some(HostMessage::Notify(Text::new("Standup in 5")))
    );
}
