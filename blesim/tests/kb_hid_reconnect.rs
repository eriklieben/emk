//! The firmware's Bluetooth keyboard (`kb-ble`) on the air: a computer
//! paired on the profile comes back with its bond and gets its keys again,
//! with the subscriptions it had (Windows doesn't subscribe again).

mod common;
mod kb;
mod pc;

use std::cell::Cell;
use std::sync::atomic::Ordering;

use blesim::{Air, SimController};
use common::{Resources, computer, runner, serve_computer};
use embassy_futures::join::join3;
use embassy_futures::select::select;
use kb::*;
use kb_ble::LeftState;
use trouble_host::prelude::*;

const HID_SERVICE: u16 = 0x1812;
const HID_REPORT: u16 = 0x2a4d;
const REPORT: [u8; 8] = [0, 0, 0x05, 0, 0, 0, 0, 0]; // b

/// Subscribe to the keyboard's input report on `conn`, have the keyboard
/// send [`REPORT`], and return what arrived.
async fn report_through(
    pc: &common::HostStack<'_>,
    conn: &Connection<'_, DefaultPacketPool>,
    left: &LeftState<'_>,
) -> Option<[u8; 8]> {
    let client = GattClient::<SimController, DefaultPacketPool, 4>::new(pc, conn)
        .await
        .expect("gatt client");
    let received = Cell::new(None);
    let listen = async {
        let services = client
            .services_by_uuid(&Uuid::new_short(HID_SERVICE))
            .await
            .expect("HID service");
        let input = client
            .characteristic_by_uuid::<[u8; 8]>(&services[0], &Uuid::new_short(HID_REPORT))
            .await
            .expect("input report");
        let mut listener = client.subscribe(&input, false).await.expect("subscribe");
        left.hid.reports.send(REPORT).await;
        received.set(listener.next().await.as_ref().try_into().ok());
    };
    let _ = select(client.task(), select(listen, serve_computer(conn, pc))).await;
    received.get()
}

#[test]
fn paired_computer_reconnects_with_its_bond_and_gets_keys() {
    // Arrange: a computer paired on profile 1, subscribed, then away (in the scenario).
    let air = Air::new();
    let mut flash = RamFlash::new();
    let hid_server = kb_ble::hid::hid_server("left");
    let left = LeftState::new(Box::leak(Box::new(kb_ble::log::LogRing::new())));
    let never = Stop::new();
    let mut pc_res = Resources::new();
    let pc = computer(air.controller("pc"), &mut pc_res);
    let mut central = pc.central();

    // Act: it connects again.
    let (first, again) = run(
        join3(
            left_half(&air, &left, &mut flash, &hid_server, &never),
            runner(&pc),
            async {},
        ),
        async {
            let conn = pc::connect_and_secure(&mut central, &pc, &left).await;
            let first = report_through(&pc, &conn, &left).await;
            pc::leave(conn, &pc, &left).await;
            let conn = pc::connect_and_secure(&mut central, &pc, &left).await;
            let again = report_through(&pc, &conn, &left).await;
            (first, again)
        },
    );

    // Assert
    dump("left", left.log);
    let log = log_text(left.log);
    assert_eq!(first, Some(REPORT), "keys on the first connection");
    assert_eq!(again, Some(REPORT), "keys after reconnecting");
    assert!(
        log.contains("host: profile 1 computer reconnected"),
        "{log}"
    );
    assert!(
        log.contains("host: subscriptions restored for profile 1"),
        "{log}"
    );
    assert!(left.hid.connected.load(Ordering::Relaxed));
    assert_eq!(left.hid.active.load(Ordering::Relaxed), 0);
}
