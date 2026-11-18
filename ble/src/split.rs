//! The link between the halves.
//!
//! The right half advertises a custom "split" service and notifies a
//! bitmap of its held keys (Corne positions, bit n = position n) whenever it
//! changes. The characteristic needs an encrypted link, so nothing goes out
//! before the halves have paired. The left half finds the right half once
//! by that service, pairs with it (LE Secure Connections) and stores the
//! bond; after that it only connects to that one right half.
//!
//! Each notification is the whole state, so a lost link can't leave a key
//! stuck: on disconnect the left half releases everything the right half
//! held, and on reconnect the first notification brings it up to date.
//!
//! The other way, the left half writes its key presses (for the right
//! screen's rain) and messages from the computer (`crate::notify`): each
//! message as `kb_core::host` Raw HID reports, one write per chunk, which
//! the right half puts back together. One that arrives while the halves are
//! apart goes out when they link again (unless it is older than a
//! notification is shown).

use core::cell::Cell;
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU16, Ordering};

use embassy_futures::join::join;
use embassy_futures::select::{Either3, Either4, select, select3, select4};
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Timer};
use embedded_storage_async::nor_flash::NorFlash;
use kb_core::host::{self, HostMessage, Reassembler};
use kb_core::link::{SplitEvent, SplitVerdict, right_half_verdict};
use sequential_storage::cache::NoCache;
use sequential_storage::map::{MapConfig, MapStorage};
use trouble_host::prelude::*;

use crate::left::LeftState;
use crate::log::LogRing;
use crate::notify::Mailbox;
use crate::storage::{self, SPLIT_BOND};
use crate::{Params, Timing};

/// Split service and its key-bitmap characteristic (random 128-bit UUIDs).
const SPLIT_SERVICE: [u8; 16] = [
    0x01, 0x4e, 0x3a, 0x2b, 0x1d, 0x5c, 0x2f, 0x9e, 0x3c, 0x4a, 0x7d, 0x6b, 0x01, 0x00, 0x0e, 0x8f,
];

/// The right half's GATT server. Build it with [`split_server`].
#[gatt_server]
pub struct SplitServer {
    pub split: SplitService,
}

#[gatt_service(uuid = "8f0e0001-6b7d-4a3c-9e2f-5c1d2b3a4e01")]
pub struct SplitService {
    /// Held keys of the right half: bit n = Corne position n.
    #[characteristic(
        uuid = "8f0e0002-6b7d-4a3c-9e2f-5c1d2b3a4e01",
        read,
        notify,
        permissions(encrypted)
    )]
    pub keys: [u8; 8],
    /// The right half's battery level, percent.
    #[characteristic(
        uuid = "8f0e0003-6b7d-4a3c-9e2f-5c1d2b3a4e01",
        read,
        notify,
        value = 100,
        permissions(encrypted)
    )]
    pub battery: u8,
    /// The right half's log, in pieces: byte 0 = length, then that much text.
    /// The left half joins them into lines and adds them to its own log.
    #[characteristic(
        uuid = "8f0e0004-6b7d-4a3c-9e2f-5c1d2b3a4e01",
        notify,
        value = [0; LOG_PIECE],
        permissions(encrypted)
    )]
    pub log: [u8; LOG_PIECE],
    /// Key presses on the left half so far (u16, little endian, wrapping),
    /// written by the left half: the right half's screen rains for them too.
    #[characteristic(
        uuid = "8f0e0005-6b7d-4a3c-9e2f-5c1d2b3a4e01",
        write_without_response,
        value = [0; 2],
        permissions(encrypted)
    )]
    pub left_presses: [u8; 2],
    /// Messages from the computer for the right screen, written by the left
    /// half: `kb_core::host` Raw HID reports, one write per chunk.
    #[characteristic(
        uuid = "8f0e0006-6b7d-4a3c-9e2f-5c1d2b3a4e01",
        write_without_response,
        value = [0; host::REPORT_SIZE],
        permissions(encrypted)
    )]
    pub notification: [u8; host::REPORT_SIZE],
}

/// Size of a log notification: fits the 72-byte L2CAP packets we configure.
const LOG_PIECE: usize = 64;

/// The right half's GATT server, named `name`.
///
/// Only one can be built per program: trouble's `#[gatt_server]` keeps the
/// log characteristic's value (more than 20 bytes) in a `static`, and a
/// second server panics. A restarted right half reuses the first one.
///
/// # Panics
///
/// When trouble can't build the attribute table (more attributes than its
/// table holds), which no build of this firmware does.
#[must_use]
pub fn split_server(name: &str) -> SplitServer<'_> {
    SplitServer::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name,
        appearance: &appearance::human_interface_device::KEYBOARD,
    }))
    .unwrap_or_else(|_| panic!("gatt server"))
}

/// UUID of the split service's characteristic number `n` (keys 2, battery 3,
/// log 4, left presses 5, notification 6).
const fn split_uuid(n: u8) -> [u8; 16] {
    let mut uuid = SPLIT_SERVICE;
    uuid[12] = n;
    uuid
}

/// The state of the split link that the rest of a half's firmware sees.
pub struct SplitLink {
    /// Whether the halves are linked (shown on the screens).
    pub linked: AtomicBool,
    /// How far the link has come, shown at the bottom of the OLEDs (`link_detail`).
    stage: AtomicU8,
    /// Set at boot when the half's outermost top key is held: forget the bond
    /// with the other half, so the two can pair again.
    pub forget_bond: AtomicBool,
}

impl Default for SplitLink {
    fn default() -> Self {
        Self::new()
    }
}

impl SplitLink {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            linked: AtomicBool::new(false),
            stage: AtomicU8::new(0),
            forget_bond: AtomicBool::new(false),
        }
    }

    /// Name of the current link stage, for the screen.
    pub fn stage_name(&self) -> &'static str {
        STAGE_NAMES
            .get(self.stage.load(Ordering::Relaxed) as usize)
            .copied()
            .unwrap_or("?")
    }

    fn set_stage(&self, log: &LogRing, stage: Stage) {
        self.stage.store(stage as u8, Ordering::Relaxed);
        ring_log!(log, "split: {}", STAGE_NAMES[stage as usize]);
    }
}

/// Each half uses its own subset of these.
#[derive(Clone, Copy)]
#[repr(u8)]
enum Stage {
    /// The state before the first stage (only ever stored as its number).
    #[allow(dead_code, reason = "never constructed: the initial stored value")]
    Start,
    /// Left: scanning for an unpaired right half.
    Scan,
    /// Left: connecting to the bonded right half.
    Wait,
    Found,
    /// Right: advertising.
    Adv,
    Conn,
    Pair,
    Subscribe,
    Linked,
    Disconnected,
    /// Left: the host side of the connect failed.
    ConnectError,
    PairError,
    DiscoverError,
    SubscribeError,
    AdvError,
    /// Left: the controller refused the connect.
    ConnectRefused,
    /// Left: pairing didn't finish in time.
    PairTimeout,
    /// Right: accepting the connection failed.
    AcceptError,
    /// Right: attaching the GATT server to the connection failed.
    ServerError,
}

const STAGE_NAMES: [&str; 19] = [
    "start", "scan", "wait", "found", "adv", "conn", "pair", "sub", "linked", "disc", "c-host",
    "p-err", "d-err", "s-err", "a-adv", "c-ctl", "p-tmo", "a-acc", "a-srv",
];

/// Erase the stored bond when the user asked for it at boot (`forget_bond`).
async fn forget_bond_if_asked<S: NorFlash>(
    link: &SplitLink,
    log: &LogRing,
    storage: &mut MapStorage<u8, S, NoCache>,
    buf: &mut [u8],
) {
    if link.forget_bond.load(Ordering::Relaxed) {
        ring_log!(log, "split: forgetting the bond with the other half");
        storage::store_bond(storage, buf, SPLIT_BOND, None, log).await;
    }
}

// ---- right half: peripheral ------------------------------------------------------

/// Everything the right half's firmware shares with its Bluetooth. The log
/// ring is borrowed for the same reason as in `LeftState`.
pub struct RightState<'a> {
    pub split: SplitLink,
    /// Its held keys, set by the key loop (`set_keys`).
    keys: Signal<CriticalSectionRawMutex, u64>,
    // No 64-bit atomics on the Cortex-M4: a critical-section cell instead.
    keys_now: Mutex<CriticalSectionRawMutex, Cell<u64>>,
    /// Its battery level, for the left half (`set_battery`).
    battery: AtomicU8,
    battery_changed: Signal<CriticalSectionRawMutex, ()>,
    /// Key presses on the left half, as it last wrote them.
    left_presses: AtomicU16,
    left_presses_changed: Signal<CriticalSectionRawMutex, ()>,
    /// The latest message from the computer, as the left half forwarded it.
    notification: Mailbox<HostMessage>,
    /// The right half's log; forwarded to the left half once linked.
    pub log: &'a LogRing,
}

impl<'a> RightState<'a> {
    pub const fn new(log: &'a LogRing) -> Self {
        Self {
            split: SplitLink::new(),
            keys: Signal::new(),
            keys_now: Mutex::new(Cell::new(0)),
            battery: AtomicU8::new(100),
            battery_changed: Signal::new(),
            left_presses: AtomicU16::new(0),
            left_presses_changed: Signal::new(),
            notification: Mailbox::new(),
            log,
        }
    }

    /// Key presses on the left half so far (wrapping), for the screen.
    pub fn left_presses(&self) -> u16 {
        self.left_presses.load(Ordering::Relaxed)
    }

    /// Wait until a key is pressed on the left half (to wake the right
    /// half's key loop and screen).
    pub async fn left_typed(&self) {
        self.left_presses_changed.wait().await;
    }

    /// The latest message from the computer, if one came since the last
    /// call (right half's key loop, for its screen).
    pub fn take_notification(&self) -> Option<HostMessage> {
        self.notification.take()
    }

    /// The latest message from the computer, left for the key loop.
    pub fn notification(&self) -> Option<HostMessage> {
        self.notification.peek()
    }

    /// Wait until a message from the computer arrives.
    pub async fn notification_arrived(&self) {
        self.notification.wait().await;
    }

    /// Report the right half's held keys (right half's key loop).
    pub fn set_keys(&self, bits: u64) {
        self.keys_now.lock(|c| c.set(bits));
        self.keys.signal(bits);
    }

    /// Report the right half's battery level (right half's battery task).
    pub fn set_battery(&self, pct: u8) {
        self.battery.store(pct, Ordering::Relaxed);
        self.battery_changed.signal(());
    }

    fn set_stage(&self, stage: Stage) {
        self.split.set_stage(self.log, stage);
    }
}

async fn ble_runner<C: Controller, P: PacketPool>(mut runner: Runner<'_, C, P>) {
    loop {
        if runner.run().await.is_err() {
            Timer::after(Duration::from_millis(100)).await;
        }
    }
}

/// Advertise to the left half, accept it, and keep it up to date with the held keys.
pub async fn run_peripheral<C: Controller, S: NorFlash>(
    state: &RightState<'_>,
    controller: C,
    flash: &mut S,
    server: &SplitServer<'_>,
    params: Params,
) -> ! {
    let Params {
        address,
        storage: range,
        name,
        timing,
        // The right half stays at the controller's default power.
        set_tx_power: _,
    } = params;
    let mut resources: HostResources<DefaultPacketPool, 1, 3> = HostResources::new();
    let stack = trouble_host::new(controller, &mut resources)
        .set_random_address(address)
        // Two keyboard halves have no screen dialog or keypad for pairing:
        // declare that, so pairing is "Just Works" and needs no confirmation.
        .set_io_capabilities(IoCapabilities::NoInputNoOutput)
        .build();
    let mut storage = MapStorage::<u8, _, _>::new(flash, MapConfig::new(range), NoCache::new());
    let mut buf = [0u8; 128];
    forget_bond_if_asked(&state.split, state.log, &mut storage, &mut buf).await;
    if let Some(bond) = storage::load_bond(&mut storage, &mut buf, SPLIT_BOND).await {
        let _ = stack.add_bond_information(bond);
    }

    let runner = stack.runner();
    let mut peripheral = stack.peripheral();

    join(ble_runner(runner), async {
        loop {
            state.set_stage(Stage::Adv);
            let conn = match advertise(name, &mut peripheral, server).await {
                Ok(conn) => conn,
                Err(stage) => {
                    // Keep the error on screen long enough to read.
                    state.set_stage(stage);
                    Timer::after(Duration::from_secs(3)).await;
                    continue;
                }
            };
            state.set_stage(Stage::Conn);
            // Paired with the left half already: no new pairings, and only a
            // connection that encrypts with that stored bond stays (a PC that
            // pairs without bonding, say, is dropped).
            let stored = storage::load_bond(&mut storage, &mut buf, SPLIT_BOND)
                .await
                .map(|bond| bond.identity);
            let _ = conn.raw().set_bondable(stored.is_none());
            let verified = Cell::new(false);
            select(
                serve(
                    state,
                    server,
                    &conn,
                    &mut storage,
                    &mut buf,
                    stored,
                    &verified,
                ),
                select3(
                    notify_keys(state, server, &conn, &timing, &verified),
                    guard(state, &conn, &verified, &timing),
                    forward(state, server, &conn, &verified, &timing),
                ),
            )
            .await;
            // Not the left half: drop whatever the stack kept about it (a
            // pairing without bonding still leaves an entry), so repeated
            // attempts can't fill the bond slots.
            if !verified.get() {
                let peer = conn.raw().peer_identity();
                if stored.is_none_or(|left| left.addr != peer.addr) {
                    let _ = stack.remove_bond_information(peer);
                }
            }
            state.split.linked.store(false, Ordering::Relaxed);
            state.set_stage(Stage::Disconnected);
        }
    })
    .await;
    unreachable!()
}

async fn advertise<'d, 's, C: Controller>(
    name: &str,
    peripheral: &mut Peripheral<'d, C, DefaultPacketPool>,
    server: &'s SplitServer<'_>,
) -> Result<GattConnection<'d, 's, DefaultPacketPool>, Stage> {
    let mut adv = [0u8; 31];
    let len = AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::CompleteServiceUuids128(&[SPLIT_SERVICE]),
        ],
        &mut adv[..],
    )
    .map_err(|_| Stage::AdvError)?;
    let mut scan = [0u8; 31];
    let scan_len = AdStructure::encode_slice(
        &[AdStructure::CompleteLocalName(name.as_bytes())],
        &mut scan[..],
    )
    .map_err(|_| Stage::AdvError)?;
    let advertiser = peripheral
        .advertise(
            &AdvertisementParameters {
                tx_power: crate::TX_POWER,
                ..Default::default()
            },
            Advertisement::ConnectableScannableUndirected {
                adv_data: &adv[..len],
                scan_data: &scan[..scan_len],
            },
        )
        .await
        .map_err(|_| Stage::AdvError)?;
    let conn = advertiser.accept().await.map_err(|_| Stage::AcceptError)?;
    conn.with_attribute_server(server)
        .map_err(|_| Stage::ServerError)
}

/// Drop a connection that hasn't proven to be the left half in time.
async fn guard(
    state: &RightState<'_>,
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
    verified: &Cell<bool>,
    timing: &Timing,
) {
    Timer::after(timing.split_verify).await;
    if !verified.get() {
        ring_log!(
            state.log,
            "split: dropping a connection that is not the left half"
        );
        conn.raw().disconnect();
    }
    core::future::pending::<()>().await;
}

/// Handle GATT requests and pairing until the link drops; store a new bond.
async fn serve<S: NorFlash>(
    state: &RightState<'_>,
    server: &SplitServer<'_>,
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
    storage: &mut MapStorage<u8, S, NoCache>,
    buf: &mut [u8],
    stored: Option<Identity>,
    verified: &Cell<bool>,
) {
    // Messages from the computer, chunk by chunk (one per connection: a
    // message cut off by a lost link is dropped).
    let mut messages = Reassembler::new();
    loop {
        match conn.next().await {
            GattConnectionEvent::Disconnected { reason } => {
                ring_log!(state.log, "split: disconnected: {:?}", reason);
                return;
            }
            // Is it the left half? The rule is `kb_core::link::right_half_verdict`.
            GattConnectionEvent::PairingComplete { bond, .. } => {
                paired(state, conn, storage, buf, stored, verified, bond).await;
            }
            GattConnectionEvent::Encrypted { bond, .. } => {
                let event = SplitEvent::Encrypted {
                    bond: bond.as_ref().map(|b| b.identity.addr),
                };
                match right_half_verdict(stored.map(|id| id.addr).as_ref(), event) {
                    SplitVerdict::Accept { .. } => accept(state, verified, "reconnected"),
                    SplitVerdict::Reject => reject(state, conn, "encrypted with another bond"),
                    SplitVerdict::Wait => {}
                }
            }
            GattConnectionEvent::PairingFailed(e) => {
                ring_log!(state.log, "split: pairing failed: {:?}", e);
                state.set_stage(Stage::PairError);
            }
            GattConnectionEvent::PassKeyConfirm(_) => {
                let _ = conn.pass_key_confirm();
            }
            GattConnectionEvent::Gatt { event } => {
                if verified.get() {
                    take_write(state, server, &mut messages, &event);
                }
                if let Ok(reply) = event.accept() {
                    reply.send().await;
                }
            }
            _ => {}
        }
    }
}

/// The connection is the left half: the link is up.
fn accept(state: &RightState<'_>, verified: &Cell<bool>, why: &str) {
    ring_log!(state.log, "split: left half {}", why);
    verified.set(true);
    state.split.linked.store(true, Ordering::Relaxed);
    state.set_stage(Stage::Linked);
}

/// The connection is not the left half: drop it.
fn reject(state: &RightState<'_>, conn: &GattConnection<'_, '_, DefaultPacketPool>, why: &str) {
    ring_log!(state.log, "split: dropping a connection: {}", why);
    conn.raw().disconnect();
}

/// Pairing completed: accept the left half (storing its new bond), drop a
/// stranger, or wait for more.
async fn paired<S: NorFlash>(
    state: &RightState<'_>,
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
    storage: &mut MapStorage<u8, S, NoCache>,
    buf: &mut [u8],
    stored: Option<Identity>,
    verified: &Cell<bool>,
    bond: Option<BondInformation>,
) {
    let event = SplitEvent::Paired {
        bond: bond.as_ref().map(|b| b.identity.addr),
    };
    match right_half_verdict(stored.map(|id| id.addr).as_ref(), event) {
        SplitVerdict::Accept { .. } => {
            if let Some(bond) = bond {
                storage::store_bond(storage, buf, SPLIT_BOND, Some(bond), state.log).await;
            }
            accept(state, verified, "paired");
        }
        SplitVerdict::Reject => reject(state, conn, "paired, but not with the stored bond"),
        SplitVerdict::Wait => {}
    }
}

/// What the verified left half writes: its key press count, and messages
/// from the computer (chunk by chunk, put together in `messages`).
fn take_write(
    state: &RightState<'_>,
    server: &SplitServer<'_>,
    messages: &mut Reassembler,
    event: &GattEvent<'_, '_, DefaultPacketPool>,
) {
    let GattEvent::Write(write) = event else {
        return;
    };
    if write.handle() == server.split.left_presses.handle {
        write.with_data(|_, data| {
            if let [lo, hi, ..] = *data {
                state
                    .left_presses
                    .store(u16::from_le_bytes([lo, hi]), Ordering::Relaxed);
                state.left_presses_changed.signal(());
            }
        });
    }
    if write.handle() == server.split.notification.handle {
        let message = write.with_data(|offset, data| {
            if offset == 0 {
                messages.feed(data)
            } else {
                None
            }
        });
        if let Some(message) = message {
            ring_log!(state.log, "split: message from the computer");
            state.notification.put(message);
        }
    }
}

/// Notify the held keys: the current state first, then every change.
async fn notify_keys(
    state: &RightState<'_>,
    server: &SplitServer<'_>,
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
    timing: &Timing,
    verified: &Cell<bool>,
) {
    // Not before the link is verified: a notification on an unencrypted link
    // makes trouble ask the left half for security first, and the reconnect
    // is then reported as a pairing without a bond, which can't be told
    // apart from a stranger pairing without bonding (so it was rejected, and
    // the halves never linked again after the left half restarted).
    while !verified.get() {
        Timer::after(Duration::from_millis(20)).await;
    }
    let keys = server.split.keys;
    let mut bits = state.keys_now.lock(core::cell::Cell::get);
    loop {
        // Stores the value (for the left half's first read) and notifies it;
        // the notification is skipped until the left half has subscribed.
        let _ = keys.notify(conn, &bits.to_le_bytes(), true).await;
        bits = select(state.keys.wait(), Timer::after(timing.notify_heartbeat))
            .await
            .into_value(bits);
    }
}

/// Once the left half is verified (and had a moment to subscribe): send it
/// the battery level and the log, from the oldest line still in the ring.
async fn forward(
    state: &RightState<'_>,
    server: &SplitServer<'_>,
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
    verified: &Cell<bool>,
    timing: &Timing,
) {
    while !verified.get() {
        Timer::after(Duration::from_millis(200)).await;
    }
    Timer::after(timing.right_forward_delay).await;
    let battery = server.split.battery;
    let log = server.split.log;
    let _ = battery
        .notify(conn, &state.battery.load(Ordering::Relaxed), false)
        .await;
    let mut pos = state.log.oldest();
    loop {
        let mut piece = [0u8; LOG_PIECE];
        let n = state.log.read(&mut pos, &mut piece[1..]);
        if n > 0 {
            piece[0] = n as u8;
            if log.notify(conn, &piece, false).await.is_err() {
                break;
            }
            continue;
        }
        if let embassy_futures::select::Either::Second(()) =
            select(state.log.wait_new(), state.battery_changed.wait()).await
        {
            let _ = battery
                .notify(conn, &state.battery.load(Ordering::Relaxed), false)
                .await;
        }
    }
    core::future::pending::<()>().await;
}

trait IntoValue {
    fn into_value(self, previous: u64) -> u64;
}

impl IntoValue for embassy_futures::select::Either<u64, ()> {
    fn into_value(self, previous: u64) -> u64 {
        match self {
            Self::First(bits) => bits,
            Self::Second(()) => previous,
        }
    }
}

// ---- left half: central ----------------------------------------------------------

/// Finds the right half's advertisement while scanning.
pub struct Finder {
    found: Cell<Option<Address>>,
}

impl Default for Finder {
    fn default() -> Self {
        Self::new()
    }
}

impl Finder {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            found: Cell::new(None),
        }
    }
}

impl EventHandler for Finder {
    fn on_adv_reports(&self, mut reports: LeAdvReportsIter<'_>) {
        while let Some(Ok(report)) = reports.next() {
            if self.found.get().is_none() && contains(report.data, &SPLIT_SERVICE) {
                self.found.set(Some(Address {
                    kind: report.addr_kind,
                    addr: report.addr,
                }));
            }
        }
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// The key-value store, shared by the split link and the computer profiles.
pub type Store<S> =
    embassy_sync::mutex::Mutex<embassy_sync::blocking_mutex::raw::NoopRawMutex, storage::Map<S>>;

/// Connect to the right half (pairing on first contact), subscribe to its
/// keys and turn bitmap changes into key events, forever. Runs on the left
/// half's stack next to the link to the computer (`left`).
pub async fn run_central<C, S>(
    state: &LeftState<'_>,
    stack: &Stack<'_, C, DefaultPacketPool>,
    central: &mut Central<'_, C, DefaultPacketPool>,
    finder: &Finder,
    store: &Store<S>,
    timing: &Timing,
) -> !
where
    C: Controller
        + bt_hci::controller::ControllerCmdSync<bt_hci::cmd::le::LeSetScanParams>
        + bt_hci::controller::ControllerCmdAsync<bt_hci::cmd::le::LeConnUpdate>
        + bt_hci::controller::ControllerCmdSync<bt_hci::cmd::le::LeReadLocalSupportedFeatures>,
    S: NorFlash,
{
    let mut buf = [0u8; 128];
    let mut peer = {
        let mut map = store.lock().await;
        forget_bond_if_asked(&state.split, state.log, &mut map, &mut buf).await;
        storage::load_bond(&mut map, &mut buf, SPLIT_BOND)
            .await
            .map(|bond| {
                let addr = bond.identity.addr;
                let _ = stack.add_bond_information(bond);
                addr
            })
    };
    loop {
        let target = if let Some(addr) = peer {
            state.set_stage(Stage::Wait);
            addr
        } else {
            state.set_stage(Stage::Scan);
            let addr = find_right_half(state, central, finder, timing).await;
            state.set_stage(Stage::Found);
            addr
        };
        let connected = connect_paced(state, central, target, timing).await;
        let conn = match connected {
            Ok(conn) => conn,
            Err(e) => {
                ring_log!(state.log, "split: connect failed: {:?}", e);
                // A command the controller refuses comes back as an HCI error
                // inside the host error; `Controller` is only for transport I/O.
                state.set_stage(match e {
                    BleHostError::BleHost(trouble_host::Error::Hci(_)) => Stage::ConnectRefused,
                    BleHostError::Controller(_) | BleHostError::BleHost(_) => Stage::ConnectError,
                });
                Timer::after(Duration::from_millis(500)).await;
                continue;
            }
        };
        state.set_stage(Stage::Conn);
        state
            .radio
            .connected(crate::radio::Link::Split, conn.handle().raw(), state.log);
        let bonded = peer.is_some();
        let _ = conn.set_bondable(!bonded);
        state.set_stage(Stage::Pair);
        if conn.request_security().is_err() {
            state.set_stage(Stage::PairError);
            state.radio.disconnected(crate::radio::Link::Split);
            continue;
        }
        // Wait for encryption; store the bond the first time.
        let pairing = wait_paired(state, stack, &conn, store, &mut buf, &mut peer);
        // Don't wait forever on a pairing that went nowhere: drop the
        // link and start over.
        let secure = match select(pairing, Timer::after(timing.pairing_timeout)).await {
            embassy_futures::select::Either::First(secure) => secure,
            embassy_futures::select::Either::Second(()) => {
                state.set_stage(Stage::PairTimeout);
                conn.disconnect();
                Timer::after(Duration::from_secs(2)).await;
                false
            }
        };
        if secure {
            state.set_stage(Stage::Subscribe);
            state.split.linked.store(true, Ordering::Relaxed);
            receive_keys(state, stack, &conn).await;
            state.split.linked.store(false, Ordering::Relaxed);
        }
        state.radio.disconnected(crate::radio::Link::Split);
    }
}

/// Connect to the right half at `target`. Listening all the time at first,
/// then a small part of the time (`kb_core::link::pace`): a right half
/// that's off doesn't drain the left.
async fn connect_paced<'s, C>(
    state: &LeftState<'_>,
    central: &mut Central<'s, C, DefaultPacketPool>,
    target: Address,
    timing: &Timing,
) -> Result<Connection<'s, DefaultPacketPool>, BleHostError<C::Error>>
where
    C: Controller + bt_hci::controller::ControllerCmdSync<bt_hci::cmd::le::LeSetScanParams>,
{
    let started = Instant::now();
    loop {
        let (pace, change) = timing.pace(crate::elapsed_ms(started));
        let (interval, window) = kb_core::link::split_scan_us(pace);
        let config = ConnectConfig {
            connect_params: split_link_params(),
            scan_config: ScanConfig {
                filter_accept_list: &[target],
                interval: Duration::from_micros(u64::from(interval)),
                window: Duration::from_micros(u64::from(window)),
                ..Default::default()
            },
        };
        let slow_down = async {
            match change {
                Some(ms) => Timer::after(Duration::from_millis(u64::from(ms))).await,
                None => core::future::pending().await,
            }
        };
        match select(central.connect(&config), slow_down).await {
            embassy_futures::select::Either::First(result) => return result,
            embassy_futures::select::Either::Second(()) => {
                ring_log!(state.log, "split: looking for the right half slower now");
            }
        }
    }
}

/// Wait until the link to the right half is encrypted, storing the bond (and
/// its address in `peer`) when it is a new one. Whether it got there.
async fn wait_paired<C, S: NorFlash>(
    state: &LeftState<'_>,
    stack: &Stack<'_, C, DefaultPacketPool>,
    conn: &Connection<'_, DefaultPacketPool>,
    store: &Store<S>,
    buf: &mut [u8],
    peer: &mut Option<Address>,
) -> bool
where
    C: Controller + bt_hci::controller::ControllerCmdAsync<bt_hci::cmd::le::LeConnUpdate>,
{
    loop {
        match conn.next().await {
            ConnectionEvent::PairingComplete { bond, .. } => {
                if let Some(bond) = bond {
                    let addr = bond.identity.addr;
                    let mut map = store.lock().await;
                    storage::store_bond(&mut map, buf, SPLIT_BOND, Some(bond), state.log).await;
                    *peer = Some(addr);
                }
                return true;
            }
            ConnectionEvent::PairingFailed(e) => {
                ring_log!(state.log, "split: pairing failed: {:?}", e);
                state.set_stage(Stage::PairError);
                return false;
            }
            ConnectionEvent::Disconnected { reason } => {
                ring_log!(state.log, "split: disconnected while pairing: {:?}", reason);
                state.set_stage(Stage::Disconnected);
                return false;
            }
            ConnectionEvent::RequestConnectionParams(req) => {
                let _ = req.accept(None, stack).await;
            }
            ConnectionEvent::PassKeyConfirm(_) => {
                let _ = conn.pass_key_confirm();
            }
            _ => {}
        }
    }
}

/// The split link like ZMK's: a 7.5 ms interval, so a key on the right half
/// waits at most that long, and peripheral latency 30, so the right half may
/// skip up to 30 idle intervals to save power but sends at once when needed.
/// (trouble's default is an 80 ms interval.)
fn split_link_params() -> RequestedConnParams {
    link_params(SPLIT_FAST_INTERVAL_US)
}

/// Link interval while typing, and while the keyboard rests (screens off):
/// the left half then wakes its radio six times less often. The first key
/// from the right half after a rest can take up to 45 ms; then it's fast again.
const SPLIT_FAST_INTERVAL_US: u64 = 7_500;
const SPLIT_RESTING_INTERVAL_US: u64 = 45_000;

/// Split link parameters with `interval_us`. The supervision timeout (4 s)
/// stays above (1 + latency) × interval × 2 for both intervals.
fn link_params(interval_us: u64) -> RequestedConnParams {
    RequestedConnParams {
        min_connection_interval: Duration::from_micros(interval_us),
        max_connection_interval: Duration::from_micros(interval_us),
        max_latency: 30,
        supervision_timeout: Duration::from_secs(4),
        ..Default::default()
    }
}

/// Scan (fast, then slow) until `finder` sees a right half; its address.
async fn find_right_half<C>(
    state: &LeftState<'_>,
    central: &mut Central<'_, C, DefaultPacketPool>,
    finder: &Finder,
    timing: &Timing,
) -> Address
where
    C: Controller + bt_hci::controller::ControllerCmdSync<bt_hci::cmd::le::LeSetScanParams>,
{
    let started = Instant::now();
    loop {
        // Fast, then slow, as for the bonded right half.
        let (pace, change) = timing.pace(crate::elapsed_ms(started));
        let (interval, window) = kb_core::link::split_scan_us(pace);
        let config = ScanConfig {
            active: false,
            interval: Duration::from_micros(u64::from(interval)),
            window: Duration::from_micros(u64::from(window)),
            ..Default::default()
        };
        let deadline = change.map(|ms| Instant::now() + Duration::from_millis(u64::from(ms)));
        finder.found.set(None);
        let found = {
            let mut scanner = Scanner::new(central);
            let Ok(_session) = scanner.scan(&config).await else {
                Timer::after(Duration::from_secs(1)).await;
                continue;
            };
            let found = loop {
                if let Some(addr) = finder.found.get() {
                    break Some(addr);
                }
                if deadline.is_some_and(|d| Instant::now() >= d) {
                    break None;
                }
                Timer::after(Duration::from_millis(50)).await;
            };
            let Some(found) = found else {
                ring_log!(state.log, "split: scanning slower now");
                continue;
            };
            found
            // Dropping the session only *requests* the end of the scan; the
            // runner sends it to the controller shortly after.
        };
        // Give the controller time to stop scanning: a connect while it still
        // scans is refused.
        Timer::after(Duration::from_millis(200)).await;
        return found;
    }
}

/// Subscribe to the right half's keys and forward changes until disconnect.
async fn receive_keys<C>(
    state: &LeftState<'_>,
    stack: &Stack<'_, C, DefaultPacketPool>,
    conn: &Connection<'_, DefaultPacketPool>,
) where
    C: Controller
        + bt_hci::controller::ControllerCmdAsync<bt_hci::cmd::le::LeConnUpdate>
        + bt_hci::controller::ControllerCmdSync<bt_hci::cmd::le::LeReadLocalSupportedFeatures>,
{
    let Ok(client) = GattClient::<C, DefaultPacketPool, 4>::new(stack, conn).await else {
        return;
    };
    let mut held = 0u64;
    let _ = select(client.task(), async {
        let Ok(services) = client
            .services_by_uuid(&Uuid::new_long(SPLIT_SERVICE))
            .await
        else {
            state.set_stage(Stage::DiscoverError);
            return;
        };
        let Some(service) = services.first().cloned() else {
            state.set_stage(Stage::DiscoverError);
            return;
        };
        let Ok(keys) = client
            .characteristic_by_uuid::<[u8; 8]>(&service, &Uuid::new_long(split_uuid(2)))
            .await
        else {
            state.set_stage(Stage::DiscoverError);
            return;
        };
        let Ok(mut listener) = client.subscribe(&keys, false).await else {
            state.set_stage(Stage::SubscribeError);
            return;
        };
        let Extras {
            battery,
            log,
            left_presses,
            notification,
        } = find_extras(&client, &service).await;
        let mut battery_updates = subscribe_if(&client, battery.as_ref()).await;
        let mut log_updates = subscribe_if(&client, log.as_ref()).await;
        state.set_stage(Stage::Linked);
        held = read_current(state, &client, &keys, battery.as_ref(), held).await;
        let mut line = RemoteLine::new();
        if let Some(c) = &left_presses {
            send_presses(state, &client, c).await;
        }
        // Each message gets its own id, so the right half can't mix chunks
        // of two.
        let mut message_id = 0u8;
        // One that came while the halves were apart.
        if let Some(c) = &notification {
            send_message(state, &client, c, &mut message_id).await;
        }
        // The link starts fast; slow it down while the keyboard rests.
        let mut resting = state.resting.load(Ordering::Relaxed);
        if resting {
            set_pace(state, stack, conn, true).await;
        }
        loop {
            let presses_changed = wait_if(left_presses.is_some(), state.presses_changed.wait());
            let message = wait_if(notification.is_some(), state.inbox.wait_for_split());
            let local = select3(presses_changed, state.resting_changed.wait(), message);
            match select4(
                listener.next(),
                next_of(&mut battery_updates),
                next_of(&mut log_updates),
                local,
            )
            .await
            {
                Either4::Fourth(Either3::First(())) => {
                    if let Some(c) = &left_presses {
                        send_presses(state, &client, c).await;
                    }
                }
                Either4::Fourth(Either3::Third(())) => {
                    if let Some(c) = &notification {
                        send_message(state, &client, c, &mut message_id).await;
                    }
                }
                Either4::Fourth(Either3::Second(())) => {
                    let now = state.resting.load(Ordering::Relaxed);
                    if now != resting {
                        resting = now;
                        set_pace(state, stack, conn, now).await;
                    }
                }
                Either4::First(data) => {
                    let mut bytes = [0u8; 8];
                    let n = data.as_ref().len().min(bytes.len());
                    bytes[..n].copy_from_slice(&data.as_ref()[..n]);
                    held = state.apply(held, u64::from_le_bytes(bytes));
                }
                Either4::Second(data) => {
                    if let Some(&pct) = data.as_ref().first() {
                        state.set_right_battery(pct);
                    }
                }
                Either4::Third(data) => line.feed_piece(state.log, data.as_ref()),
            }
        }
    })
    .await;
    // Link gone: release whatever the right half was holding.
    state.apply(held, 0);
}

/// The split service's optional characteristics: only where the right
/// half's firmware has them.
struct Extras {
    battery: Option<Characteristic<u8>>,
    log: Option<Characteristic<[u8; LOG_PIECE]>>,
    left_presses: Option<Characteristic<[u8; 2]>>,
    notification: Option<Characteristic<[u8; host::REPORT_SIZE]>>,
}

/// Find the split `service`'s optional characteristics.
async fn find_extras<C: Controller>(
    client: &GattClient<'_, C, DefaultPacketPool, 4>,
    service: &ServiceHandle,
) -> Extras {
    // Battery and log: only from a right half that has them.
    let battery = client
        .characteristic_by_uuid::<u8>(service, &Uuid::new_long(split_uuid(3)))
        .await
        .ok();
    let log = client
        .characteristic_by_uuid::<[u8; LOG_PIECE]>(service, &Uuid::new_long(split_uuid(4)))
        .await
        .ok();
    let left_presses = client
        .characteristic_by_uuid::<[u8; 2]>(service, &Uuid::new_long(split_uuid(5)))
        .await
        .ok();
    // Messages for the right screen: only to a right half that shows them.
    let notification = client
        .characteristic_by_uuid::<[u8; host::REPORT_SIZE]>(service, &Uuid::new_long(split_uuid(6)))
        .await
        .ok();
    Extras {
        battery,
        log,
        left_presses,
        notification,
    }
}

/// Send the right half the next message for its screen, if there is one,
/// in reports with the next `message_id`. Kept for the next link if it
/// doesn't get through.
async fn send_message<C: Controller>(
    state: &LeftState<'_>,
    client: &GattClient<'_, C, DefaultPacketPool, 4>,
    c: &Characteristic<[u8; host::REPORT_SIZE]>,
    message_id: &mut u8,
) {
    let Some(message) = state.inbox.take_for_split() else {
        return;
    };
    *message_id = message_id.wrapping_add(1);
    let mut reports = [[0u8; host::REPORT_SIZE]; host::MAX_CHUNKS];
    let mut n = 0;
    match &message {
        HostMessage::Notify(text) => {
            host::encode_notification(text.as_str(), *message_id, |r| {
                reports[n] = *r;
                n += 1;
            });
        }
        HostMessage::Clear => {
            reports[0][0] = host::KIND_CLEAR;
            n = 1;
        }
    }
    for report in &reports[..n] {
        if let Err(e) = client
            .write_characteristic_without_response(c, report)
            .await
        {
            ring_log!(state.log, "split: message not sent: {:?}", e);
            state.inbox.retry_for_split(message);
            return;
        }
    }
    ring_log!(state.log, "split: message sent to the right half");
}

/// Ask for the resting (slow) or the fast link interval.
async fn set_pace<C>(
    state: &LeftState<'_>,
    stack: &Stack<'_, C, DefaultPacketPool>,
    conn: &Connection<'_, DefaultPacketPool>,
    resting: bool,
) where
    C: Controller
        + bt_hci::controller::ControllerCmdAsync<bt_hci::cmd::le::LeConnUpdate>
        + bt_hci::controller::ControllerCmdSync<bt_hci::cmd::le::LeReadLocalSupportedFeatures>,
{
    let interval = if resting {
        SPLIT_RESTING_INTERVAL_US
    } else {
        SPLIT_FAST_INTERVAL_US
    };
    match conn
        .update_connection_params(stack, &link_params(interval))
        .await
    {
        Ok(()) => ring_log!(
            state.log,
            "split: link {} ({} us)",
            if resting { "resting" } else { "fast" },
            interval
        ),
        Err(e) => ring_log!(state.log, "split: link interval change failed: {:?}", e),
    }
}

/// The right half's held keys and battery level as they are now, read once
/// the link is up: the keys held now (`held` before).
async fn read_current<C: Controller>(
    state: &LeftState<'_>,
    client: &GattClient<'_, C, DefaultPacketPool, 4>,
    keys: &Characteristic<[u8; 8]>,
    battery: Option<&Characteristic<u8>>,
    held: u64,
) -> u64 {
    let mut held = held;
    let mut first = [0u8; 8];
    if client.read_characteristic(keys, &mut first).await.is_ok() {
        held = state.apply(held, u64::from_le_bytes(first));
    }
    if let Some(c) = battery {
        let mut pct = [0u8; 1];
        if client.read_characteristic(c, &mut pct).await.is_ok() {
            state.set_right_battery(pct[0]);
        }
    }
    held
}

/// Tell the right half how many keys the left half has had pressed (its
/// screen rains for them too).
async fn send_presses<C: Controller>(
    state: &LeftState<'_>,
    client: &GattClient<'_, C, DefaultPacketPool, 4>,
    c: &Characteristic<[u8; 2]>,
) {
    let n = state.presses.load(Ordering::Relaxed);
    let _ = client
        .write_characteristic_without_response(c, &n.to_le_bytes())
        .await;
}

/// Subscribe to `c`, if the right half has it (and the subscription works).
async fn subscribe_if<'c, C: Controller, T: AsGatt + ?Sized>(
    client: &'c GattClient<'_, C, DefaultPacketPool, 4>,
    c: Option<&Characteristic<T>>,
) -> Option<NotificationListener<'c, 512>> {
    match c {
        Some(c) => client.subscribe(c, false).await.ok(),
        None => None,
    }
}

/// `wait`, or never when `cond` is false.
async fn wait_if<F: core::future::Future>(cond: bool, wait: F) -> F::Output {
    if cond {
        wait.await
    } else {
        core::future::pending().await
    }
}

/// The next notification of an optional subscription (never, without one).
async fn next_of<const MTU: usize>(
    listener: &mut Option<NotificationListener<'_, MTU>>,
) -> Notification<MTU> {
    match listener {
        Some(l) => l.next().await,
        None => core::future::pending().await,
    }
}

impl LeftState<'_> {
    fn set_stage(&self, stage: Stage) {
        self.split.set_stage(self.log, stage);
        // Wake the left half's key loop so the screen shows the new stage.
        self.hid.state_changed.signal(());
    }

    fn set_right_battery(&self, pct: u8) {
        self.right_battery.store(pct, Ordering::Relaxed);
        self.hid.state_changed.signal(());
        // And to the computer, as its second battery.
        self.hid.right_battery.store(pct, Ordering::Relaxed);
        self.hid.battery_changed.signal(());
    }

    /// Send a press/release for every position whose bit changed.
    fn apply(&self, previous: u64, now: u64) -> u64 {
        let mut changed = previous ^ now;
        while changed != 0 {
            let pos = changed.trailing_zeros() as u8;
            let _ = self.remote_events.try_send((pos, now & (1 << pos) != 0));
            changed &= changed - 1;
        }
        now
    }
}

/// Joins the right half's log pieces into lines for the left half's log.
struct RemoteLine {
    buf: [u8; 160],
    len: usize,
}

impl RemoteLine {
    const fn new() -> Self {
        Self {
            buf: [0; 160],
            len: 0,
        }
    }

    /// One piece of the right half's log: a length byte, then that much text.
    fn feed_piece(&mut self, log: &LogRing, piece: &[u8]) {
        if let Some((&len, text)) = piece.split_first() {
            self.feed(log, &text[..(len as usize).min(text.len())]);
        }
    }

    fn feed(&mut self, log: &LogRing, text: &[u8]) {
        for &b in text {
            match b {
                b'\r' => {}
                b'\n' => {
                    let line = core::str::from_utf8(&self.buf[..self.len]).unwrap_or("?");
                    ring_log!(log, "R{}", line);
                    self.len = 0;
                }
                _ if self.len < self.buf.len() => {
                    self.buf[self.len] = b;
                    self.len += 1;
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_flash::{log_text, store};
    // The host's critical section, for the blocking mutexes.
    use critical_section as _;
    use embassy_futures::block_on;

    fn bond() -> BondInformation {
        BondInformation {
            ltk: LongTermKey(42),
            identity: Identity {
                addr: Address::random([2, 2, 2, 2, 2, 0xc2]),
                irk: None,
            },
            is_bonded: true,
            security_level: SecurityLevel::Encrypted,
        }
    }

    #[test]
    fn new_link_is_at_the_start_unlinked() {
        // Arrange / Act
        let link = SplitLink::default();

        // Assert
        assert_eq!(link.stage_name(), "start");
        assert!(!link.linked.load(Ordering::Relaxed));
    }

    #[test]
    fn stage_change_is_shown_and_logged() {
        // Arrange
        let (link, log) = (SplitLink::new(), LogRing::new());

        // Act
        link.set_stage(&log, Stage::PairTimeout);

        // Assert
        assert_eq!(link.stage_name(), "p-tmo");
        assert!(log_text(&log).ends_with(" split: p-tmo\r\n"));
    }

    #[test]
    fn unknown_stage_number_shows_a_question_mark() {
        // Arrange
        let link = SplitLink::new();
        link.stage.store(200, Ordering::Relaxed);

        // Act
        let name = link.stage_name();

        // Assert
        assert_eq!(name, "?");
    }

    #[test]
    fn every_stage_has_a_short_name_of_its_own() {
        // Arrange
        let names = STAGE_NAMES;

        // Act
        let longest = names.iter().map(|n| n.len()).max();

        // Assert
        assert!(longest <= Some(6), "they fit the bottom line of the OLED");
        for (i, n) in names.iter().enumerate() {
            assert!(!names[..i].contains(n), "{n} twice");
        }
    }

    #[test]
    fn bond_is_forgotten_when_asked_at_boot() {
        // Arrange
        let (link, log, store) = (SplitLink::new(), LogRing::new(), store());
        let mut buf = [0u8; 128];
        let mut map = block_on(store.lock());
        block_on(storage::store_bond(
            &mut map,
            &mut buf,
            SPLIT_BOND,
            Some(bond()),
            &log,
        ));
        link.forget_bond.store(true, Ordering::Relaxed);

        // Act
        block_on(forget_bond_if_asked(&link, &log, &mut map, &mut buf));

        // Assert
        assert!(block_on(storage::load_bond(&mut map, &mut buf, SPLIT_BOND)).is_none());
        assert!(log_text(&log).contains("split: forgetting the bond"));
    }

    #[test]
    fn bond_is_kept_when_not_asked() {
        // Arrange
        let (link, log, store) = (SplitLink::new(), LogRing::new(), store());
        let mut buf = [0u8; 128];
        let mut map = block_on(store.lock());
        block_on(storage::store_bond(
            &mut map,
            &mut buf,
            SPLIT_BOND,
            Some(bond()),
            &log,
        ));

        // Act
        block_on(forget_bond_if_asked(&link, &log, &mut map, &mut buf));

        // Assert
        assert!(block_on(storage::load_bond(&mut map, &mut buf, SPLIT_BOND)).is_some());
    }

    #[test]
    fn battery_level_wakes_the_forwarder() {
        // Arrange
        let log = LogRing::new();
        let right = RightState::new(&log);

        // Act
        right.set_battery(57);

        // Assert
        assert_eq!(right.battery.load(Ordering::Relaxed), 57);
        assert!(right.battery_changed.signaled());
    }

    #[test]
    fn held_keys_are_kept_for_the_heartbeat() {
        // Arrange
        let log = LogRing::new();
        let right = RightState::new(&log);

        // Act
        right.set_keys(0b1010);

        // Assert
        assert_eq!(right.keys_now.lock(Cell::get), 0b1010);
        assert!(right.keys.signaled());
    }

    #[test]
    fn right_half_starts_without_a_message() {
        // Arrange
        let log = LogRing::new();
        let right = RightState::new(&log);

        // Act
        let message = right.take_notification();

        // Assert
        assert_eq!(message, None);
        assert_eq!(right.left_presses(), 0);
    }

    #[test]
    fn finder_starts_without_an_address() {
        // Arrange / Act
        let finder = Finder::default();

        // Assert
        assert!(finder.found.get().is_none());
    }

    #[test]
    fn service_uuid_is_found_anywhere_in_the_advertisement() {
        // Arrange
        let mut adv = [0u8; 31];
        adv[5..21].copy_from_slice(&SPLIT_SERVICE);

        // Act
        let found = [
            contains(&adv, &SPLIT_SERVICE),
            contains(&adv[..20], &SPLIT_SERVICE),
        ];

        // Assert
        assert_eq!(found, [true, false]);
    }
}
