//! A computer sends a message for the right screen over Bluetooth: it
//! pairs with the left half, finds the Raw HID output report by its Report
//! Reference (report ID 3, output) and writes the `kb_core::host` chunks to
//! it, as the HID driver does for `kbnotify` (hidraw on Linux, `WriteFile` on
//! Windows).

#![allow(dead_code)] // each test file uses its own subset

use std::sync::atomic::Ordering;

use blesim::SimController;
use embassy_futures::select::{Either, select};
use embassy_time::Duration;
use kb_ble::LeftState;
use kb_core::host::REPORT_SIZE;
use trouble_host::prelude::*;

use crate::common::{HostStack, LEFT, connect, serve_computer};
use crate::kb::*;

const HID_SERVICE: u16 = 0x1812;
const HID_REPORT: u16 = 0x2a4d;
const REPORT_REFERENCE: u16 = 0x2908;

/// How the computer writes the output report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Write {
    /// Write Request (Windows).
    WithResponse,
    /// Write Command (`BlueZ`, when the characteristic allows it).
    WithoutResponse,
}

/// The report text as chunks (`kb_core::host`).
pub fn reports(text: &str, id: u8) -> Vec<[u8; REPORT_SIZE]> {
    let mut out = Vec::new();
    kb_core::host::encode_notification(text, id, |r| out.push(*r));
    out
}

/// Pair `pc` with the left half, write `reports` to the Raw HID output
/// report, and stay connected until `until` holds (at most 5 s). Whether
/// the report was found and every write went through.
pub async fn send(
    pc: &HostStack<'_>,
    central: &mut Central<'_, SimController, DefaultPacketPool>,
    left: &LeftState<'_>,
    reports: &[[u8; REPORT_SIZE]],
    write: Write,
    until: impl Fn() -> bool,
) -> bool {
    let conn = connect(central, Address::random(LEFT)).await;
    conn.set_bondable(true).unwrap();
    conn.request_security().unwrap();
    let events = async {
        serve_computer(&conn, pc).await;
        false
    };
    let client = async {
        let client = GattClient::<SimController, DefaultPacketPool, 4>::new(pc, &conn)
            .await
            .expect("gatt client");
        let work = async {
            wait_until(Duration::from_secs(5), || {
                left.hid.connected.load(Ordering::Relaxed)
            })
            .await;
            let Ok(services) = client.services_by_uuid(&Uuid::new_short(HID_SERVICE)).await else {
                return false;
            };
            let Some(service) = services.first().cloned() else {
                return false;
            };
            let Ok(all) = client.characteristics::<16>(&service).await else {
                return false;
            };
            let mut raw_hid = None;
            for c in all.iter().filter(|c| c.uuid == Uuid::new_short(HID_REPORT)) {
                let Ok(reference) = client
                    .descriptor_by_uuid::<[u8], [u8]>(c, &Uuid::new_short(REPORT_REFERENCE))
                    .await
                else {
                    continue;
                };
                let mut value = [0u8; 2];
                if client
                    .read_handle(reference.handle(), &mut value)
                    .await
                    .is_ok()
                    && value == [3, 2]
                {
                    raw_hid = Some(c.handle);
                }
            }
            let Some(handle) = raw_hid else {
                return false;
            };
            for report in reports {
                let sent = match write {
                    Write::WithResponse => client.write_handle(handle, report).await.is_ok(),
                    Write::WithoutResponse => client
                        .write_handle_without_response(handle, report)
                        .await
                        .is_ok(),
                };
                if !sent {
                    return false;
                }
            }
            wait_until(Duration::from_secs(5), &until).await;
            true
        };
        match select(client.task(), work).await {
            Either::First(_) => false,
            Either::Second(ok) => ok,
        }
    };
    match select(client, events).await {
        Either::First(ok) | Either::Second(ok) => ok,
    }
}
