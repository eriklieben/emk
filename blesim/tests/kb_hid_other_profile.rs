//! The firmware's Bluetooth keyboard (`kb-ble`) on the air: a computer
//! paired on one profile that reconnects while another one is selected is
//! dropped; the selected profile stays open for its own computer.

mod common;
mod kb;
mod pc;

use std::sync::atomic::Ordering;

use blesim::Air;
use common::{Resources, computer, runner};
use embassy_futures::join::join3;
use embassy_time::Duration;
use kb::*;
use kb_ble::{Command, LeftState};

#[test]
fn computer_of_another_profile_is_dropped() {
    // Arrange: a computer paired on profile 1, then profile 2 selected (in the scenario).
    let air = Air::new();
    let mut flash = RamFlash::new();
    let hid_server = kb_ble::hid::hid_server("left");
    let left = LeftState::new(Box::leak(Box::new(kb_ble::log::LogRing::new())));
    let never = Stop::new();
    let mut pc_res = Resources::new();
    let pc = computer(air.controller("pc"), &mut pc_res);
    let mut central = pc.central();

    // Act: it connects again with its bond.
    let dropped = run(
        join3(
            left_half(&air, &left, &mut flash, &hid_server, &never),
            runner(&pc),
            async {},
        ),
        async {
            let conn = pc::connect_and_secure(&mut central, &pc, &left).await;
            left.hid.commands.send(Command::Next).await;
            pc::dropped_within(&conn, &pc, Duration::from_secs(2)).await;
            let conn = pc::connect_and_secure(&mut central, &pc, &left).await;
            pc::dropped_within(&conn, &pc, Duration::from_secs(2)).await
        },
    );

    // Assert
    dump("left", left.log);
    assert!(dropped, "the keyboard dropped it");
    assert!(log_text(left.log).contains("host: another profile's computer; dropping"));
    assert_eq!(left.hid.active.load(Ordering::Relaxed), 1, "profile 2");
    assert!(
        left.hid.open.load(Ordering::Relaxed),
        "profile 2 is still open"
    );
    assert!(!left.hid.connected.load(Ordering::Relaxed));
}
