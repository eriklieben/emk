//! The firmware's Bluetooth (`kb-ble`) is a keyboard and a mouse: a PC
//! finds the mouse's input report by its Report Reference (report ID 2,
//! input), gets mouse reports there, keyboard reports still arrive on
//! report ID 1, and writing the mouse's feature report (report ID 2,
//! feature) sets the Resolution Multipliers for smooth scrolling.

mod common;
mod kb;

use std::cell::Cell;
use std::sync::atomic::Ordering;

use blesim::{Air, SimController};
use common::{LEFT, Resources, computer, connect, runner, serve_computer};
use embassy_futures::join::join3;
use embassy_futures::select::select;
use embassy_time::Duration;
use kb::*;
use kb_ble::LeftState;
use kb_core::mouse::MouseReport;
use trouble_host::prelude::*;

const HID_SERVICE: u16 = 0x1812;
const HID_REPORT: u16 = 0x2a4d;
const REPORT_REFERENCE: u16 = 0x2908;

/// What the PC saw.
#[derive(Debug, Default)]
struct Seen {
    /// Report Reference of every input report characteristic, in order.
    references: std::cell::RefCell<Vec<[u8; 2]>>,
    mouse: Cell<Option<[u8; 7]>>,
    keyboard: Cell<Option<[u8; 8]>>,
}

/// The mouse input, keyboard input and mouse feature reports among the HID
/// service's characteristics `all`, found by their Report Reference as a PC
/// finds them. Every
/// Report Reference read goes into `seen`.
async fn find_reports<'c>(
    client: &GattClient<'_, SimController, DefaultPacketPool, 4>,
    all: &'c [Characteristic<[u8]>],
    seen: &Seen,
) -> [Option<&'c Characteristic<[u8]>>; 3] {
    let mut found = [None; 3];
    for c in all.iter().filter(|c| c.uuid == Uuid::new_short(HID_REPORT)) {
        let Ok(reference) = client
            .descriptor_by_uuid::<[u8], [u8]>(c, &Uuid::new_short(REPORT_REFERENCE))
            .await
        else {
            continue;
        };
        let mut value = [0u8; 2];
        client
            .read_handle(reference.handle(), &mut value)
            .await
            .expect("read the report reference");
        seen.references.borrow_mut().push(value);
        let slot = match value {
            [2, 1] => 0,
            [1, 1] => 1,
            [2, 3] => 2,
            _ => continue,
        };
        found[slot] = Some(c);
    }
    found
}

#[test]
fn pc_gets_mouse_reports_on_report_2_and_keys_on_report_1() {
    // Arrange
    let air = Air::new();
    let mut flash = RamFlash::new();
    let hid_server = kb_ble::hid::hid_server("left");
    let left = LeftState::new(Box::leak(Box::new(kb_ble::log::LogRing::new())));
    let never = Stop::new();
    let mut pc_res = Resources::new();
    let pc = computer(air.controller("pc"), &mut pc_res);
    let mut central = pc.central();
    let motion = MouseReport {
        buttons: 1,
        x: -300,
        y: 12,
        wheel: -1,
        pan: 0,
    };
    let keys = [0x02, 0, 0x04, 0, 0, 0, 0, 0];

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
            let events = async {
                serve_computer(&conn, &pc).await;
                // Dropped: the client's work decides how the test ends.
                core::future::pending::<()>().await;
            };
            let client = async {
                let client = GattClient::<SimController, DefaultPacketPool, 4>::new(&pc, &conn)
                    .await
                    .expect("gatt client");
                let work = async {
                    wait_until(Duration::from_secs(5), || {
                        left.hid.connected.load(Ordering::Relaxed)
                    })
                    .await;
                    let services = client
                        .services_by_uuid(&Uuid::new_short(HID_SERVICE))
                        .await
                        .expect("discover the HID service");
                    let service = services.first().expect("HID service").clone();
                    let all = client
                        .characteristics::<16>(&service)
                        .await
                        .expect("discover the characteristics");
                    let [mouse_input, keyboard_input, mouse_feature] =
                        find_reports(&client, &all, &seen).await;
                    let mouse_input = mouse_input.expect("a mouse input report");
                    let keyboard_input = keyboard_input.expect("a keyboard input report");
                    let mut mouse = client
                        .subscribe(mouse_input, false)
                        .await
                        .expect("subscribe");
                    left.hid.mouse_reports.send(motion.bytes()).await;
                    let data = mouse.next().await;
                    seen.mouse.set(data.as_ref().try_into().ok());
                    let mut keyboard = client
                        .subscribe(keyboard_input, false)
                        .await
                        .expect("subscribe");
                    left.hid.reports.send(keys).await;
                    let data = keyboard.next().await;
                    seen.keyboard.set(data.as_ref().try_into().ok());
                    let feature = mouse_feature.expect("a mouse feature report");
                    client
                        .write_handle(feature.handle, &[0x05])
                        .await
                        .expect("write the feature report");
                    wait_until(Duration::from_secs(1), || {
                        left.hid.mouse_feature.load(Ordering::Relaxed) == 0x05
                    })
                    .await;
                };
                select(client.task(), work).await
            };
            select(client, events).await;
            seen
        },
    );

    // Assert
    dump("left", left.log);
    assert_eq!(
        *seen.references.borrow(),
        [[1, 1], [2, 1], [2, 3], [1, 2], [3, 2]],
        "keyboard input, mouse input, mouse feature, keyboard LEDs, Raw HID output"
    );
    assert_eq!(
        kb_core::mouse::resolution_from_feature(left.hid.mouse_feature.load(Ordering::Relaxed)),
        (true, true),
        "both multipliers set"
    );
    assert_eq!(seen.mouse.get(), Some(motion.bytes()));
    assert_eq!(seen.keyboard.get(), Some(keys));
}
