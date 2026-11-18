//! The firmware's split link (`kb-ble`) on the air: after both halves
//! restart, they link again with the bond from flash, without pairing.

mod common;
mod kb;

use blesim::Air;
use embassy_futures::join::join;
use embassy_time::Duration;
use kb::*;
use kb_ble::{LeftState, RightState};

const TEST: &str = "restarted_halves_reconnect_with_their_stored_bond";

#[test]
fn restarted_halves_reconnect_with_their_stored_bond() {
    if let Some(boot) = kb::boot() {
        return start_both(boot);
    }
    // Arrange: the halves pair on their first start.
    let boots = Boots::new(TEST);
    let first = boots.run("first");

    // Act: both restart with what they stored.
    let second = boots.run("second");

    // Assert
    assert_eq!(first["linked"], "true", "linked on the first start");
    assert_eq!(first["pairings"], "1");
    assert_eq!(second["linked"], "true", "linked after the restart");
    assert_eq!(second["pairings"], "0", "no new pairing");
    assert_eq!(second["left_used_bond"], "true");
    assert_eq!(second["right_reconnected"], "true");
}

/// One start of both halves: run them until they are linked (or 5 s).
fn start_both(mut boot: Boot) {
    let air = Air::new();
    let (mut left_flash, mut right_flash) = (boot.flash("left"), boot.flash("right"));
    let split_server = kb_ble::split::split_server("right");
    let hid_server = kb_ble::hid::hid_server("left");
    let (left, right) = (
        LeftState::new(Box::leak(Box::new(kb_ble::log::LogRing::new()))),
        RightState::new(Box::leak(Box::new(kb_ble::log::LogRing::new()))),
    );
    let never = Stop::new();

    let linked = run(
        join(
            left_half(&air, &left, &mut left_flash, &hid_server, &never),
            right_half(&air, &right, &mut right_flash, &split_server, &never),
        ),
        wait_until(Duration::from_secs(5), || {
            left_linked(&left) && right_linked(&right)
        }),
    );

    dump("left", left.log);
    dump("right", right.log);
    boot.save_flash("left", &left_flash);
    boot.save_flash("right", &right_flash);
    boot.report("linked", linked);
    boot.report("pairings", air.pairing_requests());
    boot.report("left_used_bond", log_text(left.log).contains("split: wait"));
    boot.report(
        "right_reconnected",
        log_text(right.log).contains("split: left half reconnected"),
    );
}
