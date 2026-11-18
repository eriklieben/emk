//! A "Just Works" computer that pairs with the keyboard (`kb-ble`) on the
//! selected profile, and comes back later with its bond.

#![allow(dead_code)] // each test file uses its own subset

use std::sync::atomic::Ordering;

use embassy_futures::select::{Either, select};
use embassy_time::Duration;
use kb_ble::LeftState;
use trouble_host::prelude::*;

use crate::common::{HostStack, LEFT, connect, serve_computer};
use crate::kb::wait_until;

/// Connect to the left half and secure the link (pairing with bonding the
/// first time, encrypting with the stored bond after); `serve` runs the
/// computer's side of it. The connection, once the keyboard took it (or
/// the link dropped, or 5 s passed).
pub async fn connect_and_secure<'d>(
    central: &mut Central<'d, blesim::SimController, DefaultPacketPool>,
    pc: &HostStack<'_>,
    left: &LeftState<'_>,
) -> Connection<'d, DefaultPacketPool> {
    let conn = connect(central, Address::random(LEFT)).await;
    conn.set_bondable(true).unwrap();
    conn.request_security().unwrap();
    let taken = wait_until(Duration::from_secs(5), || {
        left.hid.connected.load(Ordering::Relaxed)
    });
    let _ = select(serve_computer(&conn, pc), taken).await;
    conn
}

/// Serve `conn` until it drops (true) or `within` passes (false).
pub async fn dropped_within(
    conn: &Connection<'_, DefaultPacketPool>,
    pc: &HostStack<'_>,
    within: Duration,
) -> bool {
    match select(serve_computer(conn, pc), embassy_time::Timer::after(within)).await {
        Either::First(()) => true,
        Either::Second(()) => !conn.is_connected(),
    }
}

/// Disconnect and wait until the keyboard saw it.
pub async fn leave(
    conn: Connection<'_, DefaultPacketPool>,
    pc: &HostStack<'_>,
    left: &LeftState<'_>,
) {
    conn.disconnect();
    let _ = dropped_within(&conn, pc, Duration::from_secs(1)).await;
    wait_until(Duration::from_secs(2), || {
        !left.hid.connected.load(Ordering::Relaxed)
    })
    .await;
}
