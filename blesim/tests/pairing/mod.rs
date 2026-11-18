//! A computer with a screen (`DisplayYesNo`, as Windows or `BlueZ`) pairs
//! with the keyboard: passkey entry, the computer shows a 6-digit code and
//! the keyboard's user types it (or a wrong one, or nothing) on the keyboard,
//! through the key loop's `PairPrompt`.

#![allow(dead_code)] // each test file uses its own subset

use std::cell::Cell;
use std::sync::atomic::Ordering;

use blesim::Air;
use embassy_futures::join::join3;
use embassy_futures::select::select;
use embassy_time::Duration;
use kb_ble::LeftState;
use kb_core::Report;
use kb_core::link::{KEY_ENTER, PairPrompt, PairRequest};
use trouble_host::prelude::*;

use crate::common::{LEFT, Resources, computer_with, connect, runner};
use crate::kb::*;

/// What both ends saw.
#[derive(Debug, Default)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent observations of one test"
)]
pub struct Outcome {
    /// The code the computer showed.
    pub pc_code: Option<u32>,
    /// What the keyboard asked its user (`HidState::pairing`).
    pub asked: Option<PairRequest>,
    /// The computer's pairing result: its security level, or failed.
    pub pc_level: Option<SecurityLevel>,
    pub pc_failed: bool,
    /// Whether the link to the computer dropped.
    pub pc_dropped: bool,
    /// Whether the keyboard took the computer (profile secured and taken).
    pub kb_connected: bool,
    /// Whether the profile is still open afterwards.
    pub open_after: bool,
    /// Whether the prompt is still up afterwards.
    pub prompt_after: bool,
    /// The keyboard's log.
    pub log: String,
}

/// What the keyboard's user types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Typed {
    /// The computer's code, then Enter.
    TheCode,
    /// Another code, then Enter.
    AnotherCode,
    /// Nothing.
    Nothing,
}

/// Tap the digits of `code` and Enter through `prompt`, as the key loop
/// gets them from the engine; the reply it gives.
fn type_code(prompt: &mut PairPrompt, code: u32) -> Option<kb_core::link::PairReply> {
    let mut keys: Vec<u8> = format!("{code:06}")
        .bytes()
        .map(|b| match b - b'0' {
            0 => 0x27,
            d => 0x1E + d - 1,
        })
        .collect();
    keys.push(KEY_ENTER);
    let mut reply = None;
    for key in keys {
        let mut down = Report::default();
        down.set(key, true);
        reply = prompt
            .filter(Some(PairRequest::TypeCode), &down)
            .1
            .or(reply);
        prompt.filter(Some(PairRequest::TypeCode), &Report::default());
    }
    reply
}

/// Pair a `DisplayYesNo` computer on the open profile; the keyboard's user
/// types `typed`.
pub fn pair_with_code(typed: Typed) -> Outcome {
    let air = Air::new();
    let mut flash = RamFlash::new();
    let hid_server = kb_ble::hid::hid_server("left");
    let left = LeftState::new(Box::leak(Box::new(kb_ble::log::LogRing::new())));
    let never = Stop::new();
    let mut pc_res = Resources::new();
    let pc = computer_with(
        air.controller("pc"),
        &mut pc_res,
        IoCapabilities::DisplayYesNo,
    );
    let mut central = pc.central();

    let mut outcome = run(
        join3(
            left_half(&air, &left, &mut flash, &hid_server, &never),
            runner(&pc),
            async {},
        ),
        async {
            let pc_code = Cell::new(None);
            let pc_level = Cell::new(None);
            let pc_failed = Cell::new(false);
            let pc_dropped = Cell::new(false);
            let conn = connect(&mut central, Address::random(LEFT)).await;
            conn.set_bondable(true).unwrap();
            conn.request_security().unwrap();
            let events = async {
                loop {
                    match conn.next().await {
                        // The computer shows the code to type on the keyboard.
                        ConnectionEvent::PassKeyDisplay(key) => pc_code.set(Some(key.value())),
                        ConnectionEvent::PairingComplete { security_level, .. } => {
                            pc_level.set(Some(security_level));
                        }
                        ConnectionEvent::PairingFailed(_) => pc_failed.set(true),
                        ConnectionEvent::RequestConnectionParams(req) => {
                            let _ = req.accept(None, &pc).await;
                        }
                        ConnectionEvent::Disconnected { .. } => {
                            pc_dropped.set(true);
                            break;
                        }
                        _ => {}
                    }
                }
                core::future::pending::<()>().await;
            };
            let keyboard = async {
                wait_until(Duration::from_secs(2), || {
                    left.hid.pairing().is_some() && pc_code.get().is_some()
                })
                .await;
                let asked = left.hid.pairing();
                let code = pc_code.get().unwrap_or(0);
                let mut prompt = PairPrompt::new();
                let reply = match typed {
                    Typed::TheCode => type_code(&mut prompt, code),
                    Typed::AnotherCode => type_code(&mut prompt, (code + 1) % 1_000_000),
                    Typed::Nothing => None,
                };
                if let Some(reply) = reply {
                    left.hid.reply_pairing(reply);
                }
                let connected = wait_until(Duration::from_secs(2), || {
                    left.hid.connected.load(Ordering::Relaxed)
                        || pc_failed.get()
                        || pc_dropped.get()
                })
                .await
                    && left.hid.connected.load(Ordering::Relaxed);
                // Let the ends settle (a rejected link drops).
                embassy_time::Timer::after(Duration::from_millis(100)).await;
                (asked, connected)
            };
            let (asked, kb_connected) = match select(events, keyboard).await {
                embassy_futures::select::Either::First(()) => (None, false),
                embassy_futures::select::Either::Second(seen) => seen,
            };
            Outcome {
                pc_code: pc_code.get(),
                asked,
                pc_level: pc_level.get(),
                pc_failed: pc_failed.get(),
                pc_dropped: pc_dropped.get(),
                kb_connected,
                ..Outcome::default()
            }
        },
    );
    dump("left", left.log);
    outcome.open_after = left.hid.open.load(Ordering::Relaxed);
    outcome.prompt_after = left.hid.pairing().is_some();
    outcome.log = log_text(left.log);
    outcome
}
