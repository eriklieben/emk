//! The firmware's split link (`kb-ble`) on the air: a computer that pairs
//! with the bonded right half without bonding is dropped, and the left half
//! links afterwards as usual.

mod common;
mod kb;

use std::cell::Cell;

use blesim::Air;
use common::{RIGHT, Resources, computer, connect, runner};
use embassy_futures::join::join3;
use embassy_futures::select::select;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::Duration;
use kb::*;
use kb_ble::{LeftState, RightState};
use trouble_host::prelude::*;

const TEST: &str = "pc_pairing_without_bonding_is_dropped_and_the_left_half_still_links";

#[test]
fn pc_pairing_without_bonding_is_dropped_and_the_left_half_still_links() {
    if let Some(boot) = kb::boot() {
        return match boot.name.as_str() {
            "pair" => pair(boot),
            _ => intruder(boot),
        };
    }
    // Arrange: the halves paired.
    let boots = Boots::new(TEST);
    let paired = boots.run("pair");

    // Act: the right half starts alone, a PC pairs with it without bonding,
    // then the left half starts.
    let report = boots.run("intruder");

    // Assert
    assert_eq!(paired["linked"], "true", "the halves paired first");
    assert_eq!(report["pc_dropped"], "true", "the PC was disconnected");
    assert_eq!(report["dropped_by_verdict"], "true");
    assert_eq!(report["linked"], "true", "the left half linked afterwards");
    assert_eq!(report["pairings"], "1", "only the PC paired");
}

/// First start: the halves pair.
fn pair(mut boot: Boot) {
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
        embassy_futures::join::join(
            left_half(&air, &left, &mut left_flash, &hid_server, &never),
            right_half(&air, &right, &mut right_flash, &split_server, &never),
        ),
        wait_until(Duration::from_secs(5), || {
            left_linked(&left) && right_linked(&right)
        }),
    );
    boot.save_flash("left", &left_flash);
    boot.save_flash("right", &right_flash);
    boot.report("linked", linked);
}

/// Second start: the right half, a PC, then the left half.
fn intruder(mut boot: Boot) {
    let air = Air::new();
    let (mut left_flash, mut right_flash) = (boot.flash("left"), boot.flash("right"));
    let split_server = kb_ble::split::split_server("right");
    let hid_server = kb_ble::hid::hid_server("left");
    let (left, right) = (
        LeftState::new(Box::leak(Box::new(kb_ble::log::LogRing::new()))),
        RightState::new(Box::leak(Box::new(kb_ble::log::LogRing::new()))),
    );
    let never = Stop::new();
    let start_left = Signal::<CriticalSectionRawMutex, ()>::new();
    let mut pc_res = Resources::new();
    let pc = computer(air.controller("pc"), &mut pc_res);
    let mut central = pc.central();
    let pc_dropped = Cell::new(false);

    let linked = run(
        join3(
            async {
                start_left.wait().await;
                left_half(&air, &left, &mut left_flash, &hid_server, &never).await;
            },
            right_half(&air, &right, &mut right_flash, &split_server, &never),
            runner(&pc),
        ),
        async {
            // The PC: pair without bonding, until the right half drops it.
            let conn = connect(&mut central, Address::random(RIGHT)).await;
            conn.set_bondable(false).unwrap();
            conn.request_security().unwrap();
            let dropped = select(
                async {
                    loop {
                        match conn.next().await {
                            ConnectionEvent::Disconnected { .. } => break,
                            ConnectionEvent::PassKeyConfirm(_) => {
                                let _ = conn.pass_key_confirm();
                            }
                            _ => {}
                        }
                    }
                },
                // Before the right half's verify timeout would drop it.
                embassy_time::Timer::after(TIMING.split_verify / 2),
            )
            .await;
            pc_dropped.set(matches!(
                dropped,
                embassy_futures::select::Either::First(())
            ));
            drop(conn);
            start_left.signal(());
            wait_until(Duration::from_secs(5), || {
                left_linked(&left) && right_linked(&right)
            })
            .await
        },
    );

    dump("left", left.log);
    dump("right", right.log);
    boot.report("pc_dropped", pc_dropped.get());
    boot.report(
        "dropped_by_verdict",
        log_text(right.log)
            .contains("split: dropping a connection: paired, but not with the stored bond"),
    );
    boot.report("linked", linked);
    boot.report("pairings", air.pairing_requests());
}
