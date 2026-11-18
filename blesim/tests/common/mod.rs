//! Shared pieces of the link tests: addresses, stacks, and the steps every
//! scenario goes through (advertise, scan, connect, pair).

#![allow(dead_code)] // each test file uses its own subset

use std::cell::Cell;
use std::future::Future;

use blesim::{SimController, block_on};
use embassy_futures::select::{Either, select};
use embassy_time::{Duration, Timer};
use trouble_host::prelude::*;

pub type HostStack<'d> = trouble_host::Stack<'d, SimController, DefaultPacketPool>;
pub type Resources = HostResources<DefaultPacketPool, 2, 6, 1, 4>;

/// Static random addresses of the two halves (two top bits set).
pub const LEFT: [u8; 6] = [0x01, 0x11, 0x11, 0x11, 0x11, 0xc1];
pub const RIGHT: [u8; 6] = [0x02, 0x22, 0x22, 0x22, 0x22, 0xc2];

/// The split service UUID (as in the firmware), little-endian.
pub const SPLIT_SERVICE: [u8; 16] = [
    0x01, 0x4e, 0x3a, 0x2b, 0x1d, 0x5c, 0x2f, 0x9e, 0x3c, 0x4a, 0x7d, 0x6b, 0x01, 0x00, 0x0e, 0x8f,
];

/// A keyboard-half stack: static random address, "Just Works" pairing.
pub fn half(controller: SimController, addr: [u8; 6], resources: &mut Resources) -> HostStack<'_> {
    trouble_host::new(controller, resources)
        .set_random_address(Address::random(addr))
        .set_io_capabilities(IoCapabilities::NoInputNoOutput)
        .build()
}

/// A computer's stack: its controller's public address, "Just Works" pairing.
pub fn computer(controller: SimController, resources: &mut Resources) -> HostStack<'_> {
    computer_with(controller, resources, IoCapabilities::NoInputNoOutput)
}

/// A computer's stack that pairs with `io` (`DisplayYesNo`: a screen with a
/// yes / no dialog, as Windows or `BlueZ`).
pub fn computer_with(
    controller: SimController,
    resources: &mut Resources,
    io: IoCapabilities,
) -> HostStack<'_> {
    trouble_host::new(controller, resources)
        .set_io_capabilities(io)
        .build()
}

/// Prints trouble-host's log (with the `log` feature) when `BLESIM_LOG` is set.
struct Logger;

impl log::Log for Logger {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        eprintln!("[{} {}] {}", record.level(), record.target(), record.args());
    }
    fn flush(&self) {}
}

pub fn init_log() {
    if std::env::var_os("BLESIM_LOG").is_some() {
        let _ = log::set_logger(&Logger);
        log::set_max_level(log::LevelFilter::Trace);
    }
}

/// Run `scenario` next to the given host runners, failing the test after 10 s
/// (simulated links are instant: anything slow is stuck).
pub fn run<R: Future, F: Future>(runners: R, scenario: F) -> F::Output {
    init_log();
    block_on(async {
        match select(
            select(runners, scenario),
            Timer::after(Duration::from_secs(10)),
        )
        .await
        {
            Either::First(Either::First(_)) => panic!("a host runner stopped"),
            Either::First(Either::Second(output)) => output,
            Either::Second(()) => panic!("scenario timed out"),
        }
    })
}

/// A host runner, forever; an error fails the test.
pub async fn runner(stack: &HostStack<'_>) {
    stack.runner().run().await.expect("host runner");
}

/// A host runner with an advertising-report handler, forever.
pub async fn runner_with<E: EventHandler>(stack: &HostStack<'_>, handler: &E) {
    stack
        .runner()
        .run_with_handler(handler)
        .await
        .expect("host runner");
}

/// Remembers the first advertiser with the split service (as the firmware's
/// `Finder`).
#[derive(Default)]
pub struct Finder {
    pub found: Cell<Option<Address>>,
}

impl EventHandler for Finder {
    fn on_adv_reports(&self, mut reports: LeAdvReportsIter<'_>) {
        while let Some(Ok(report)) = reports.next() {
            let has_service = report
                .data
                .windows(SPLIT_SERVICE.len())
                .any(|w| w == SPLIT_SERVICE);
            if self.found.get().is_none() && has_service {
                self.found.set(Some(Address {
                    kind: report.addr_kind,
                    addr: report.addr,
                }));
            }
        }
    }
}

/// Advertise the split service (name in the scan response) and accept one
/// connection, as the right half does.
pub async fn advertise<'d>(
    peripheral: &mut Peripheral<'d, SimController, DefaultPacketPool>,
) -> Connection<'d, DefaultPacketPool> {
    let mut adv = [0u8; 31];
    let len = AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::CompleteServiceUuids128(&[SPLIT_SERVICE]),
        ],
        &mut adv[..],
    )
    .unwrap();
    let mut scan = [0u8; 31];
    let scan_len =
        AdStructure::encode_slice(&[AdStructure::CompleteLocalName(b"right")], &mut scan[..])
            .unwrap();
    let advertiser = peripheral
        .advertise(
            &AdvertisementParameters::default(),
            Advertisement::ConnectableScannableUndirected {
                adv_data: &adv[..len],
                scan_data: &scan[..scan_len],
            },
        )
        .await
        .expect("advertise");
    advertiser.accept().await.expect("accept")
}

/// Scan passively until `finder` saw the split service, as the left half does.
pub async fn scan(
    central: &mut Central<'_, SimController, DefaultPacketPool>,
    finder: &Finder,
) -> Address {
    finder.found.set(None);
    let mut scanner = Scanner::new(central);
    let _session = scanner
        .scan(&ScanConfig {
            active: false,
            ..Default::default()
        })
        .await
        .expect("scan");
    loop {
        if let Some(addr) = finder.found.get() {
            return addr;
        }
        Timer::after(Duration::from_millis(5)).await;
    }
}

/// Connect to `target` through the filter accept list.
pub async fn connect<'d>(
    central: &mut Central<'d, SimController, DefaultPacketPool>,
    target: Address,
) -> Connection<'d, DefaultPacketPool> {
    let config = ConnectConfig {
        connect_params: RequestedConnParams::default(),
        scan_config: ScanConfig {
            filter_accept_list: &[target],
            ..Default::default()
        },
    };
    central.connect(&config).await.expect("connect")
}

/// What one end saw while its link got secured.
#[derive(Debug, Default)]
#[allow(
    clippy::option_option,
    reason = "whether the event came, and the bond it carried"
)]
pub struct Secured {
    /// `PairingComplete`'s bond, if that event came. trouble's central sends
    /// one (without a bond) also when it only re-encrypted with a stored bond.
    pub pairing: Option<Option<BondInformation>>,
    /// `Encrypted`'s bond: `Some` when the link encrypted with a stored bond.
    pub encrypted: Option<Option<BondInformation>>,
}

/// The computer's side of a connection to the keyboard: confirm the
/// pairing ("Just Works") and accept parameter updates, until it drops.
pub async fn serve_computer(conn: &Connection<'_, DefaultPacketPool>, pc: &HostStack<'_>) {
    loop {
        match conn.next().await {
            ConnectionEvent::PassKeyConfirm(_) => {
                let _ = conn.pass_key_confirm();
            }
            ConnectionEvent::RequestConnectionParams(req) => {
                let _ = req.accept(None, pc).await;
            }
            ConnectionEvent::Disconnected { .. } => break,
            _ => {}
        }
    }
}

/// Wait until the link is encrypted, then collect what else arrives for a
/// moment (a pairing completes after the keys are exchanged, which is after
/// encryption). Answers "Just Works" confirmations.
pub async fn secured(conn: &Connection<'_, DefaultPacketPool>) -> Secured {
    let mut seen = Secured::default();
    let mut record = |event: ConnectionEvent| match event {
        ConnectionEvent::PairingComplete { bond, .. } => seen.pairing = Some(bond),
        ConnectionEvent::Encrypted { bond, .. } => seen.encrypted = Some(bond),
        ConnectionEvent::PairingFailed(e) => panic!("pairing failed: {e:?}"),
        ConnectionEvent::Disconnected { reason } => panic!("disconnected: {reason:?}"),
        ConnectionEvent::PassKeyConfirm(_) => conn.pass_key_confirm().expect("confirm"),
        _ => {}
    };
    loop {
        let event = conn.next().await;
        let encrypted = matches!(event, ConnectionEvent::Encrypted { .. });
        record(event);
        if encrypted {
            break;
        }
    }
    let _ = select(
        async {
            loop {
                record(conn.next().await);
            }
        },
        Timer::after(Duration::from_millis(100)),
    )
    .await;
    seen
}

/// The bond of a bonded pairing (panics on anything else).
pub fn paired_bond(secured: Secured) -> BondInformation {
    match secured.pairing {
        Some(Some(bond)) => bond,
        _ => panic!("expected a bonded pairing, got {secured:?}"),
    }
}

/// Pair the two halves once on a fresh air and return the bonds they store:
/// [left half's, right half's].
pub fn pair_halves() -> [BondInformation; 2] {
    let air = blesim::Air::new();
    let mut left_res = Resources::new();
    let mut right_res = Resources::new();
    let left = half(air.controller("left"), LEFT, &mut left_res);
    let right = half(air.controller("right"), RIGHT, &mut right_res);
    let mut central = left.central();
    let mut peripheral = right.peripheral();
    run(
        embassy_futures::join::join(runner(&left), runner(&right)),
        both_secured(
            async {
                let conn = connect(&mut central, Address::random(RIGHT)).await;
                conn.set_bondable(true).unwrap();
                conn.request_security().unwrap();
                conn
            },
            async {
                let conn = advertise(&mut peripheral).await;
                conn.set_bondable(true).unwrap();
                conn
            },
        ),
    )
    .map(paired_bond)
}

/// Bring up a connection on both ends (`central` and `peripheral` connect
/// and set up security), then wait until each end saw its link secured.
/// The connections stay up until both have: dropping one disconnects, which
/// would cut the other end's pairing short.
pub async fn both_secured<'d>(
    central: impl Future<Output = Connection<'d, DefaultPacketPool>>,
    peripheral: impl Future<Output = Connection<'d, DefaultPacketPool>>,
) -> [Secured; 2] {
    let (central, peripheral) = embassy_futures::join::join(central, peripheral).await;
    let (a, b) = embassy_futures::join::join(secured(&central), secured(&peripheral)).await;
    [a, b]
}
