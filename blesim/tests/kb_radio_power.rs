//! The firmware's Bluetooth (`kb-ble`) on the air: the `RadioPower` key sets
//! the new level on advertising, scanning and the computer's link at once,
//! and the level comes back from flash after a restart.

mod common;
mod kb;

use std::sync::atomic::Ordering;

use blesim::Air;
use common::{LEFT, Resources, computer, connect, runner};
use embassy_futures::join::join3;
use embassy_futures::select::select;
use embassy_time::{Duration, Timer};
use kb::*;
use kb_ble::{LeftState, TxTarget};
use trouble_host::prelude::*;

#[test]
fn radio_power_key_applies_to_the_link_and_survives_a_restart() {
    // Arrange: a computer connected (Just Works) on the open profile.
    let air = Air::new();
    let mut flash = RamFlash::new();
    let hid_server = kb_ble::hid::hid_server("left");
    let left = LeftState::new(Box::leak(Box::new(kb_ble::log::LogRing::new())));
    let never = Stop::new();
    let mut pc_res = Resources::new();
    let pc = computer(air.controller("pc"), &mut pc_res);
    let mut central = pc.central();
    let at_boot = left.radio.dbm();

    // Act: RadioPower once, with the computer's link up.
    let handle = run(
        join3(
            left_half(&air, &left, &mut flash, &hid_server, &never),
            runner(&pc),
            async {},
        ),
        async {
            let conn = connect(&mut central, Address::random(LEFT)).await;
            conn.set_bondable(true).unwrap();
            conn.request_security().unwrap();
            let events = async {
                loop {
                    if let ConnectionEvent::RequestConnectionParams(req) = conn.next().await {
                        let _ = req.accept(None, &pc).await;
                    }
                }
            };
            let press = async {
                wait_until(Duration::from_secs(5), || {
                    left.hid.connected.load(Ordering::Relaxed)
                })
                .await;
                TX_POWER_SET.lock().unwrap().clear();
                left.radio.step();
                wait_until(Duration::from_secs(1), || left.radio.changes() == 1).await;
                // Time to store it.
                Timer::after(Duration::from_millis(50)).await;
                conn.handle().raw()
            };
            match select(events, press).await {
                embassy_futures::select::Either::First(()) => unreachable!(),
                embassy_futures::select::Either::Second(handle) => handle,
            }
        },
    );
    let set = TX_POWER_SET.lock().unwrap().clone();
    dump("left", left.log);
    // The keyboard restarts with what it stored.
    let restarted = LeftState::new(Box::leak(Box::new(kb_ble::log::LogRing::new())));
    let air = Air::new();
    let restored = run(
        left_half(&air, &restarted, &mut flash, &hid_server, &never),
        async {
            wait_until(Duration::from_secs(1), || {
                log_text(restarted.log).contains("radio: +8 dBm")
            })
            .await;
            restarted.radio.dbm()
        },
    );
    dump("left after the restart", restarted.log);

    // Assert
    assert_eq!(at_boot, 0, "a fresh keyboard starts at 0 dBm");
    assert_eq!(left.radio.dbm(), 8, "0 → +8 dBm");
    assert!(
        set.contains(&(TxTarget::Advertising, 8)),
        "advertising: {set:?}"
    );
    assert!(set.contains(&(TxTarget::ScanInit, 8)), "scanning: {set:?}");
    assert!(
        set.contains(&(TxTarget::Connection(handle), 8)),
        "the computer's link ({handle}): {set:?}"
    );
    assert!(log_text(left.log).contains("radio: now +8 dBm"));
    assert_eq!(restored, 8, "restored from flash");
}
