//! The firmware's Bluetooth keyboard (`kb-ble`) on the air: clearing the
//! profile drops its computer and forgets it, also after a restart.

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
fn clearing_a_taken_profile_drops_and_forgets_its_computer() {
    // Arrange: a computer paired on profile 1 (in the scenario).
    let air = Air::new();
    let mut flash = RamFlash::new();
    let hid_server = kb_ble::hid::hid_server("left");
    let left = LeftState::new(Box::leak(Box::new(kb_ble::log::LogRing::new())));
    let restarted = LeftState::new(Box::leak(Box::new(kb_ble::log::LogRing::new())));
    let never = Stop::new();
    let mut pc_res = Resources::new();
    let pc = computer(air.controller("pc"), &mut pc_res);
    let mut central = pc.central();

    // Act: the profile clear key, then a restart.
    let (taken, dropped) = run(
        join3(
            left_half(&air, &left, &mut flash, &hid_server, &never),
            runner(&pc),
            async {},
        ),
        async {
            let conn = pc::connect_and_secure(&mut central, &pc, &left).await;
            let taken = !left.hid.open.load(Ordering::Relaxed);
            left.hid.commands.send(Command::Clear).await;
            let dropped = pc::dropped_within(&conn, &pc, Duration::from_secs(2)).await;
            (taken, dropped)
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
    assert!(taken, "the computer took profile 1 first");
    assert!(dropped, "clearing dropped the computer");
    assert!(
        left.hid.open.load(Ordering::Relaxed),
        "profile 1 is open again"
    );
    assert!(log_text(left.log).contains("host: Clear -> profile 1"));
    assert!(
        log_text(restarted.log).contains("host: profile 1 (open)"),
        "the bond is gone from flash"
    );
}
