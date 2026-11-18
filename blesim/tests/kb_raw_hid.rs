//! A message from the computer over Bluetooth (`kbnotify`): the computer
//! writes the Raw HID output report (report ID 3) on the left half, the
//! left half puts the chunks together, shows it in its own status and
//! forwards it over the split link to the right half's screen.

mod common;
mod kb;
mod raw_hid;

use blesim::Air;
use common::{Resources, computer, runner};
use embassy_futures::join::join4;
use kb::*;
use kb_ble::{LeftState, RightState};
use kb_core::host::{HostMessage, Text};
use raw_hid::{Write, reports, send};

#[test]
fn message_written_over_bluetooth_reaches_both_halves() {
    // Arrange
    let air = Air::new();
    let left = LeftState::new(Box::leak(Box::new(kb_ble::log::LogRing::new())));
    let right = RightState::new(Box::leak(Box::new(kb_ble::log::LogRing::new())));
    let (mut left_flash, mut right_flash) = (RamFlash::new(), RamFlash::new());
    let split_server = kb_ble::split::split_server("right");
    let hid_server = kb_ble::hid::hid_server("left");
    let never = Stop::new();
    let mut pc_res = Resources::new();
    let pc = computer(air.controller("pc"), &mut pc_res);
    let mut central = pc.central();
    let text = "Build passed on main: 42 tests, 0 failures, deploy queued";
    let expected = Some(HostMessage::Notify(Text::new(text)));

    // Act
    let (linked, sent) = run(
        join4(
            left_half(&air, &left, &mut left_flash, &hid_server, &never),
            right_half(&air, &right, &mut right_flash, &split_server, &never),
            runner(&pc),
            async {},
        ),
        async {
            let linked = wait_until(embassy_time::Duration::from_secs(5), || {
                left_linked(&left) && right_linked(&right)
            })
            .await;
            let sent = send(
                &pc,
                &mut central,
                &left,
                &reports(text, 7),
                Write::WithResponse,
                || right.notification().is_some(),
            )
            .await;
            (linked, sent)
        },
    );

    // Assert
    dump("left", left.log);
    dump("right", right.log);
    assert!(linked, "both halves linked");
    assert!(sent, "the computer found report 3 and wrote it");
    assert_eq!(left.inbox.screen.peek(), expected, "the left half has it");
    assert_eq!(right.notification(), expected, "the right half has it");
}
