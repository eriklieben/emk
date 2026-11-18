//! The firmware's Bluetooth keyboard (`kb-ble`) on the air: the computer
//! sees two batteries, the left half's and the right half's.

mod common;
mod kb;

use std::sync::atomic::Ordering;

use blesim::{Air, SimController};
use common::{LEFT, Resources, computer, connect, runner, serve_computer};
use embassy_futures::join::join3;
use embassy_futures::select::select;
use embassy_time::Duration;
use kb::*;
use kb_ble::LeftState;
use trouble_host::prelude::*;

const BATTERY_SERVICE: u16 = 0x180f;
const BATTERY_LEVEL: u16 = 0x2a19;

#[test]
fn computer_sees_both_halves_batteries() {
    // Arrange
    let air = Air::new();
    let mut flash = RamFlash::new();
    let hid_server = kb_ble::hid::hid_server("left");
    let left = LeftState::new(Box::leak(Box::new(kb_ble::log::LogRing::new())));
    let never = Stop::new();
    let mut pc_res = Resources::new();
    let pc = computer(air.controller("pc"), &mut pc_res);
    let mut central = pc.central();

    // Act
    let levels = run(
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
                serve_computer(&conn, &pc).await;
                Vec::new()
            };
            let client = async {
                let client = GattClient::<SimController, DefaultPacketPool, 4>::new(&pc, &conn)
                    .await
                    .expect("gatt client");
                let read = async {
                    wait_until(Duration::from_secs(5), || {
                        left.hid.connected.load(Ordering::Relaxed)
                    })
                    .await;
                    // The right half reports 42%.
                    left.hid.right_battery.store(42, Ordering::Relaxed);
                    left.hid.battery_changed.signal(());
                    embassy_time::Timer::after(Duration::from_millis(100)).await;
                    let services = client
                        .services_by_uuid(&Uuid::new_short(BATTERY_SERVICE))
                        .await
                        .expect("discover the battery services");
                    let mut levels = Vec::new();
                    for service in &services {
                        let level = client
                            .characteristic_by_uuid::<u8>(service, &Uuid::new_short(BATTERY_LEVEL))
                            .await
                            .expect("battery level");
                        let mut pct = [0u8; 1];
                        client
                            .read_characteristic(&level, &mut pct)
                            .await
                            .expect("read");
                        levels.push(pct[0]);
                    }
                    levels
                };
                match select(client.task(), read).await {
                    embassy_futures::select::Either::Second(levels) => levels,
                    embassy_futures::select::Either::First(_) => Vec::new(),
                }
            };
            match select(client, events).await {
                embassy_futures::select::Either::First(levels)
                | embassy_futures::select::Either::Second(levels) => levels,
            }
        },
    );

    // Assert: the left half's (100%) and the right half's
    dump("left", left.log);
    assert_eq!(levels, [100, 42]);
}
