//! The firmware's Bluetooth keyboard (`kb-ble`) on the air: a PC pairs on
//! the open profile, receives key reports, and is dropped when the profile
//! changes.

mod common;
mod kb;

use std::cell::Cell;
use std::sync::atomic::Ordering;

use blesim::{Air, SimController};
use common::{LEFT, Resources, computer, connect, runner};
use embassy_futures::join::join3;
use embassy_futures::select::select;
use embassy_time::{Duration, Timer};
use kb::*;
use kb_ble::{Command, LeftState};
use trouble_host::prelude::*;

/// HID service and its input report characteristic.
const HID_SERVICE: u16 = 0x1812;
const HID_REPORT: u16 = 0x2a4d;

const REPORT: [u8; 8] = [0x02, 0, 0x04, 0, 0, 0, 0, 0]; // shift + a

/// What the PC saw, and the profile state along the way.
#[derive(Debug, Default)]
struct Seen {
    taken: Cell<bool>,
    received: Cell<Option<[u8; 8]>>,
    dropped: Cell<bool>,
}

#[test]
fn pc_pairs_on_the_open_profile_gets_reports_and_is_dropped_on_bt_next() {
    // Arrange
    let air = Air::new();
    let mut flash = RamFlash::new();
    let hid_server = kb_ble::hid::hid_server("left");
    let left = LeftState::new(Box::leak(Box::new(kb_ble::log::LogRing::new())));
    let never = Stop::new();
    let mut pc_res = Resources::new();
    let pc = computer(air.controller("pc"), &mut pc_res);
    let mut central = pc.central();
    let open_at_start = left.hid.open.load(Ordering::Relaxed);

    // Act
    let seen = run(
        join3(
            left_half(&air, &left, &mut flash, &hid_server, &never),
            runner(&pc),
            async {},
        ),
        async {
            let seen = Seen::default();
            let conn = connect(&mut central, Address::random(LEFT)).await;
            conn.set_bondable(true).unwrap();
            conn.request_security().unwrap();
            let disconnected = Cell::new(false);
            let events = async {
                loop {
                    match conn.next().await {
                        ConnectionEvent::Disconnected { .. } => {
                            disconnected.set(true);
                            break;
                        }
                        ConnectionEvent::PassKeyConfirm(_) => {
                            let _ = conn.pass_key_confirm();
                        }
                        ConnectionEvent::RequestConnectionParams(req) => {
                            let _ = req.accept(None, &pc).await;
                        }
                        _ => {}
                    }
                }
                core::future::pending::<()>().await;
            };
            let client = async {
                let client = GattClient::<SimController, DefaultPacketPool, 4>::new(&pc, &conn)
                    .await
                    .expect("gatt client");
                let typing = async {
                    let secured = wait_until(Duration::from_secs(5), || {
                        left.hid.connected.load(Ordering::Relaxed)
                    })
                    .await;
                    seen.taken.set(
                        secured
                            && !left.hid.open.load(Ordering::Relaxed)
                            && left.hid.connected.load(Ordering::Relaxed),
                    );
                    let services = client
                        .services_by_uuid(&Uuid::new_short(HID_SERVICE))
                        .await
                        .expect("discover the HID service");
                    let service = services.first().expect("HID service").clone();
                    let input = client
                        .characteristic_by_uuid::<[u8; 8]>(&service, &Uuid::new_short(HID_REPORT))
                        .await
                        .expect("discover the input report");
                    let mut listener = client.subscribe(&input, false).await.expect("subscribe");
                    left.hid.reports.send(REPORT).await;
                    let data = listener.next().await;
                    seen.received.set(data.as_ref().try_into().ok());
                    left.hid.commands.send(Command::Next).await;
                    core::future::pending::<()>().await;
                };
                // The client's task ends when the link drops.
                select(client.task(), typing).await
            };
            select(client, events).await;
            seen.dropped.set(
                wait_until(Duration::from_secs(2), || {
                    disconnected.get() || !conn.is_connected()
                })
                .await,
            );
            // The keyboard settles on the next profile.
            Timer::after(Duration::from_millis(50)).await;
            seen
        },
    );

    // Assert
    dump("left", left.log);
    assert!(open_at_start, "profile 1 open at first");
    assert!(seen.taken.get(), "the PC took profile 1: {seen:?}");
    assert_eq!(
        seen.received.get(),
        Some(REPORT),
        "the report reached the PC"
    );
    assert!(seen.dropped.get(), "BtNext dropped the PC");
    assert_eq!(left.hid.active.load(Ordering::Relaxed), 1, "profile 2");
    assert!(left.hid.open.load(Ordering::Relaxed), "profile 2 is open");
    assert!(!left.hid.connected.load(Ordering::Relaxed));
    assert!(
        log_text(left.log).contains("host: paired without a code"),
        "a computer without a screen falls back to Just Works"
    );
}
