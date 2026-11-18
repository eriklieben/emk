//! Host stacks on the simulated air: finding, connecting, pairing,
//! re-encrypting with a bond, and GATT notifications, as between the two
//! keyboard halves (and a computer).

mod common;

use blesim::Air;
use bt_hci::param::LeConnRole;
use common::*;
use embassy_futures::join::join;
use embassy_futures::select::select;
use embassy_time::{Duration, Timer};
use trouble_host::prelude::*;

#[test]
fn central_finds_the_advertising_peripheral_and_connects() {
    // Arrange
    let air = Air::new();
    let mut left_res = Resources::new();
    let mut right_res = Resources::new();
    let left = half(air.controller("left"), LEFT, &mut left_res);
    let right = half(air.controller("right"), RIGHT, &mut right_res);
    let finder = Finder::default();
    let mut central = left.central();
    let mut peripheral = right.peripheral();

    // Act
    let ((found, left_peer, left_role), (right_peer, right_role)) = run(
        join(runner_with(&left, &finder), runner(&right)),
        join(
            async {
                let found = scan(&mut central, &finder).await;
                // Dropping the scan session only requests the end of the
                // scan: give the runner time to stop it (as the firmware does).
                Timer::after(Duration::from_millis(200)).await;
                let conn = connect(&mut central, found).await;
                (found, conn.peer_address(), conn.role())
            },
            async {
                let conn = advertise(&mut peripheral).await;
                (conn.peer_address(), conn.role())
            },
        ),
    );

    // Assert
    assert_eq!(found, Address::random(RIGHT));
    assert_eq!(left_peer, Address::random(RIGHT));
    assert_eq!(left_role, LeConnRole::Central);
    assert_eq!(right_peer, Address::random(LEFT));
    assert_eq!(right_role, LeConnRole::Peripheral);
}

#[test]
fn just_works_pairing_bonds_both_sides() {
    // Arrange: nothing to set up; pair_halves runs on a fresh air.

    // Act
    let [left_bond, right_bond] = pair_halves();

    // Assert
    assert!(left_bond.is_bonded);
    assert!(right_bond.is_bonded);
    assert_eq!(left_bond.ltk, right_bond.ltk);
    assert_eq!(left_bond.identity.addr, Address::random(RIGHT));
    assert_eq!(right_bond.identity.addr, Address::random(LEFT));
    assert_eq!(left_bond.security_level, SecurityLevel::Encrypted);
}

#[test]
fn reconnect_with_stored_bonds_encrypts_without_pairing() {
    // Arrange: bonds from an earlier pairing, loaded into fresh stacks (as
    // after a reboot).
    let [left_bond, right_bond] = pair_halves();
    let air = Air::new();
    let mut left_res = Resources::new();
    let mut right_res = Resources::new();
    let left = half(air.controller("left"), LEFT, &mut left_res);
    let right = half(air.controller("right"), RIGHT, &mut right_res);
    left.add_bond_information(left_bond).unwrap();
    right.add_bond_information(right_bond).unwrap();
    let mut central = left.central();
    let mut peripheral = right.peripheral();

    // Act
    let [left_saw, right_saw] = run(
        join(runner(&left), runner(&right)),
        both_secured(
            async {
                let conn = connect(&mut central, Address::random(RIGHT)).await;
                conn.set_bondable(false).unwrap();
                conn.request_security().unwrap();
                conn
            },
            async {
                let conn = advertise(&mut peripheral).await;
                conn.set_bondable(false).unwrap();
                conn
            },
        ),
    );

    // Assert
    assert_eq!(air.pairing_requests(), 0, "no new pairing");
    assert!(
        matches!(&right_saw.encrypted, Some(Some(bond)) if bond.identity.addr == Address::random(LEFT)),
        "right half saw {right_saw:?}"
    );
    // The central started the encryption itself (request_security), so trouble
    // reports it without the bond, and with a bond-less PairingComplete.
    assert!(left_saw.encrypted.is_some(), "left half saw {left_saw:?}");
    assert!(
        matches!(left_saw.pairing, Some(None)),
        "left half saw {left_saw:?}"
    );
}

#[test]
fn reconnect_with_a_forgotten_bond_does_not_encrypt() {
    // Arrange: the left half still has its bond, the right half forgot it.
    let [left_bond, _] = pair_halves();
    let air = Air::new();
    let mut left_res = Resources::new();
    let mut right_res = Resources::new();
    let left = half(air.controller("left"), LEFT, &mut left_res);
    let right = half(air.controller("right"), RIGHT, &mut right_res);
    left.add_bond_information(left_bond).unwrap();
    let mut central = left.central();
    let mut peripheral = right.peripheral();

    // Act
    let (left_events, right_events) = run(
        join(runner(&left), runner(&right)),
        join(
            async {
                let conn = connect(&mut central, Address::random(RIGHT)).await;
                conn.set_bondable(false).unwrap();
                conn.request_security().unwrap();
                events_until_disconnect(&conn).await
            },
            async {
                let conn = advertise(&mut peripheral).await;
                conn.set_bondable(false).unwrap();
                events_until_disconnect(&conn).await
            },
        ),
    );

    // Assert
    assert!(
        !left_events.iter().any(|e| e.starts_with("Encrypted")),
        "{left_events:?}"
    );
    assert!(
        !right_events.iter().any(|e| e.starts_with("Encrypted")),
        "{right_events:?}"
    );
    assert!(
        left_events.last().unwrap().starts_with("Disconnected"),
        "{left_events:?}"
    );
}

#[test]
fn encrypting_with_different_keys_drops_the_link() {
    // Arrange: both halves have a bond with each other, but with different
    // keys (as when one half paired again and the other kept the old bond).
    let [left_bond, mut right_bond] = pair_halves();
    right_bond.ltk = LongTermKey(right_bond.ltk.0 ^ 1);
    let air = Air::new();
    let mut left_res = Resources::new();
    let mut right_res = Resources::new();
    let left = half(air.controller("left"), LEFT, &mut left_res);
    let right = half(air.controller("right"), RIGHT, &mut right_res);
    left.add_bond_information(left_bond).unwrap();
    right.add_bond_information(right_bond).unwrap();
    let mut central = left.central();
    let mut peripheral = right.peripheral();

    // Act
    let (left_events, right_events) = run(
        join(runner(&left), runner(&right)),
        join(
            async {
                let conn = connect(&mut central, Address::random(RIGHT)).await;
                conn.set_bondable(false).unwrap();
                conn.request_security().unwrap();
                events_until_disconnect(&conn).await
            },
            async {
                let conn = advertise(&mut peripheral).await;
                conn.set_bondable(false).unwrap();
                events_until_disconnect(&conn).await
            },
        ),
    );

    // Assert
    let mic_failure = "Disconnected { reason: Connection Terminated due to MIC Failure }";
    assert_eq!(
        left_events.last().map(String::as_str),
        Some(mic_failure),
        "{left_events:?}"
    );
    assert_eq!(
        right_events.last().map(String::as_str),
        Some(mic_failure),
        "{right_events:?}"
    );
    assert_eq!(air.link_count(), 0);
}

#[test]
fn connecting_while_still_scanning_is_refused() {
    // Arrange
    let air = Air::new();
    let mut left_res = Resources::new();
    let mut right_res = Resources::new();
    let left = half(air.controller("left"), LEFT, &mut left_res);
    let right = half(air.controller("right"), RIGHT, &mut right_res);
    let finder = Finder::default();
    let mut central = left.central();
    let mut peripheral = right.peripheral();

    // Act
    let result = run(
        join(runner_with(&left, &finder), runner(&right)),
        select(
            async {
                // The session (not the scanner) keeps the scan going.
                let _session = Scanner::new(&mut central)
                    .scan(&ScanConfig::default())
                    .await
                    .expect("scan");
                let config = ConnectConfig {
                    connect_params: RequestedConnParams::default(),
                    scan_config: ScanConfig {
                        filter_accept_list: &[Address::random(RIGHT)],
                        ..Default::default()
                    },
                };
                central.connect(&config).await.map(|_| ())
            },
            async {
                advertise(&mut peripheral).await;
                core::future::pending::<()>().await;
            },
        ),
    );

    // Assert
    assert!(
        matches!(
            result,
            embassy_futures::select::Either::First(Err(BleHostError::BleHost(Error::Hci(
                bt_hci::param::Error::CMD_DISALLOWED
            ))))
        ),
        "{result:?}"
    );
}

#[test]
fn going_out_of_range_drops_the_link_on_both_ends() {
    // Arrange
    let air = Air::new();
    let mut left_res = Resources::new();
    let mut right_res = Resources::new();
    let left = half(air.controller("left"), LEFT, &mut left_res);
    let right = half(air.controller("right"), RIGHT, &mut right_res);
    let mut central = left.central();
    let mut peripheral = right.peripheral();

    // Act
    let (left_events, right_events) = run(
        join(runner(&left), runner(&right)),
        join(
            async {
                let conn = connect(&mut central, Address::random(RIGHT)).await;
                air.out_of_range("right");
                events_until_disconnect(&conn).await
            },
            async {
                let conn = advertise(&mut peripheral).await;
                events_until_disconnect(&conn).await
            },
        ),
    );

    // Assert
    let timeout = "Disconnected { reason: Connection Timeout }";
    assert_eq!(
        left_events.last().map(String::as_str),
        Some(timeout),
        "{left_events:?}"
    );
    assert_eq!(
        right_events.last().map(String::as_str),
        Some(timeout),
        "{right_events:?}"
    );
}

/// Every connection event (by name) until the link drops, or until 2 s pass.
async fn events_until_disconnect(conn: &Connection<'_, DefaultPacketPool>) -> Vec<String> {
    let mut seen = Vec::new();
    let collect = async {
        loop {
            let event = conn.next().await;
            let name = format!("{event:?}");
            let done = matches!(event, ConnectionEvent::Disconnected { .. });
            seen.push(name);
            if done {
                return;
            }
        }
    };
    select(collect, Timer::after(Duration::from_secs(2))).await;
    seen
}

#[test]
fn a_computer_pairs_with_the_bonded_peripheral_without_bonding() {
    // Arrange: the right half is bonded with the left half; a computer
    // connects to it and pairs without bonding.
    let [_, right_bond] = pair_halves();
    let air = Air::new();
    let mut right_res = Resources::new();
    let mut pc_res = Resources::new();
    let right = half(air.controller("right"), RIGHT, &mut right_res);
    let pc = computer(air.controller("pc"), &mut pc_res);
    right.add_bond_information(right_bond.clone()).unwrap();
    let pc_addr = Address {
        kind: AddrKind::PUBLIC,
        addr: BdAddr::new(air.public_address("pc")),
    };
    let mut central = pc.central();
    let mut peripheral = right.peripheral();

    // Act
    let right_peer = std::cell::Cell::new(None);
    let [pc_saw, right_saw] = run(
        join(runner(&pc), runner(&right)),
        both_secured(
            async {
                let conn = connect(&mut central, Address::random(RIGHT)).await;
                conn.set_bondable(false).unwrap();
                conn.request_security().unwrap();
                conn
            },
            async {
                let conn = advertise(&mut peripheral).await;
                // The firmware's right half: no new bonds once bonded.
                conn.set_bondable(false).unwrap();
                right_peer.set(Some(conn.peer_address()));
                conn
            },
        ),
    );

    // Assert
    assert_eq!(right_peer.get(), Some(pc_addr));
    assert_eq!(air.pairing_requests(), 1);
    assert!(
        matches!(pc_saw.pairing, Some(None)),
        "computer saw {pc_saw:?}"
    );
    assert!(
        matches!(right_saw.pairing, Some(None)),
        "right half saw {right_saw:?}"
    );
    // The left half's bond is kept; trouble also keeps the computer's key for
    // the session, marked as not bonded.
    let bonds = right.with_bond_information(<[trouble_host::BondInformation]>::to_vec);
    assert_eq!(bonds.len(), 2, "{bonds:?}");
    assert_eq!(bonds[0], right_bond);
    assert_eq!(bonds[1].identity.addr, pc_addr);
    assert!(!bonds[1].is_bonded);
}
