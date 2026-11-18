//! The firmware's split link (`kb-ble`) on the air: a left half that forgot
//! its bond can't link with a right half that still has it; once the right
//! half forgets too, the two pair again.

mod common;
mod kb;

use std::sync::atomic::Ordering;

use blesim::Air;
use embassy_futures::join::join;
use embassy_time::Duration;
use kb::*;
use kb_ble::{LeftState, RightState};

const TEST: &str = "halves_pair_again_only_once_both_forgot_their_bond";

#[test]
fn halves_pair_again_only_once_both_forgot_their_bond() {
    if let Some(boot) = kb::boot() {
        return start(boot);
    }
    // Arrange: the halves paired.
    let boots = Boots::new(TEST);
    let paired = boots.run("pair");
    let left_forgot = boots.run("left-forgets");

    // Act
    let both_forgot = boots.run("right-forgets");

    // Assert
    assert_eq!(paired["linked"], "true", "the halves paired first");
    assert_eq!(
        left_forgot["left_linked"], "false",
        "no link with one bond gone"
    );
    assert_eq!(left_forgot["right_linked"], "false");
    assert_eq!(left_forgot["right_rejected"], "true");
    assert_ne!(left_forgot["pairings"], "0", "the left half tried to pair");
    assert_eq!(both_forgot["linked"], "true", "linked once both forgot");
    assert_ne!(both_forgot["pairings"], "0", "by pairing anew");
}

/// One start of both halves; which one forgets its bond (the outermost top
/// key held at boot) depends on the boot.
fn start(mut boot: Boot) {
    let air = Air::new();
    let (mut left_flash, mut right_flash) = (boot.flash("left"), boot.flash("right"));
    let split_server = kb_ble::split::split_server("right");
    let hid_server = kb_ble::hid::hid_server("left");
    let (left, right) = (
        LeftState::new(Box::leak(Box::new(kb_ble::log::LogRing::new()))),
        RightState::new(Box::leak(Box::new(kb_ble::log::LogRing::new()))),
    );
    match boot.name.as_str() {
        "left-forgets" => left.split.forget_bond.store(true, Ordering::Relaxed),
        "right-forgets" => right.split.forget_bond.store(true, Ordering::Relaxed),
        _ => {}
    }
    let never = Stop::new();
    let both_linked = || left_linked(&left) && right_linked(&right);

    let (left_ever, right_ever) = run(
        join(
            left_half(&air, &left, &mut left_flash, &hid_server, &never),
            right_half(&air, &right, &mut right_flash, &split_server, &never),
        ),
        async {
            if boot.name == "left-forgets" {
                // Long enough for several attempts.
                let period = Duration::from_secs(2);
                let watch_left = ever_within(period, || left_linked(&left));
                let watch_right = ever_within(period, || right_linked(&right));
                join(watch_left, watch_right).await
            } else {
                let linked = wait_until(Duration::from_secs(5), both_linked).await;
                (linked, linked)
            }
        },
    );

    dump("left", left.log);
    dump("right", right.log);
    boot.save_flash("left", &left_flash);
    boot.save_flash("right", &right_flash);
    boot.report("linked", left_ever && right_ever);
    boot.report("left_linked", left_ever);
    boot.report("right_linked", right_ever);
    boot.report(
        "right_rejected",
        log_text(right.log).contains("split: dropping a connection"),
    );
    boot.report("pairings", air.pairing_requests());
}
