//! Bluetooth to the computer: a HID-over-GATT keyboard and mouse with 10
//! profiles. One HID service, one report map: the keyboard is report ID 1
//! (input: modifiers, 0, six keys; output: LEDs), the mouse report ID 2
//! (`kb_core::mouse::MouseReport`), and Raw HID report ID 3 (vendor usage
//! page 0xFF60, a 32-byte output report: messages for the right screen,
//! `kb_core::host`; the computer shows it as a HID device of its own). A
//! computer paired before the mouse or Raw HID was added has the old report
//! map cached: pair it again once.
//!
//! A profile is a slot for one paired computer, stored in flash. Only the
//! selected profile is used:
//!
//! - **open** (no computer yet): the keyboard advertises as discoverable
//!   under its name, and the first computer to pair takes the profile;
//! - **taken**: it advertises without the discoverable flag, so only the
//!   computer already paired on it reconnects (by itself); anything else
//!   that connects is dropped.
//!
//! Switching profile drops the current computer and advertises for the new
//! one; clearing a profile forgets its computer and opens it again.

use core::cell::RefCell;

use core::cell::Cell;
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use embassy_sync::blocking_mutex::Mutex;
use kb_core::link::{PairReply, PairRequest};

use bt_hci::cmd::le::{LeConnUpdate, LeReadLocalSupportedFeatures};
use bt_hci::controller::{ControllerCmdAsync, ControllerCmdSync};
use embassy_futures::join::join;
use embassy_futures::select::{Either, Either3, select, select3};
use embassy_sync::blocking_mutex::raw::{CriticalSectionRawMutex, NoopRawMutex};
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Timer};
use embedded_storage_async::nor_flash::NorFlash;
use kb_core::link;
use trouble_host::prelude::*;

use crate::Timing;
use crate::left::LeftState;
use crate::split::Store;
use crate::storage::{self, ACTIVE_PROFILE, StoredProfile};

/// Number of Bluetooth profiles (as `kb_display::status::BT_PROFILES`).
pub const PROFILES: usize = 10;

/// The computer link's state that the rest of the left half's firmware sees.
pub struct HidState {
    /// Input reports for the computer: modifiers, 0, six key codes.
    pub reports: Channel<CriticalSectionRawMutex, [u8; 8], 16>,
    /// Mouse reports for the computer (`kb_core::mouse::MouseReport::bytes`).
    /// Motion is relative: the key loop only takes a report from the mouse
    /// when there is room, so none is dropped.
    pub mouse_reports: Channel<CriticalSectionRawMutex, [u8; MOUSE_REPORT_LEN], 4>,
    /// The mouse's feature report as the computer last wrote it: the
    /// Resolution Multipliers (`kb_core::mouse::resolution_from_feature`).
    /// 0 (whole detents) on each new connection.
    pub mouse_feature: AtomicU8,
    /// The selected profile (0-based).
    pub active: AtomicU8,
    /// Whether the selected profile's computer is connected (and encrypted).
    pub connected: AtomicBool,
    /// Whether the selected profile is open (ready to pair).
    pub open: AtomicBool,
    /// Signalled when the profile state above changes (or the split link's),
    /// so the key loop wakes and the screen follows.
    pub state_changed: Signal<CriticalSectionRawMutex, ()>,
    /// Battery level reported to the computer.
    pub battery: AtomicU8,
    /// The right half's battery level, reported as a second battery
    /// ("right"); 255 = not known yet.
    pub right_battery: AtomicU8,
    /// Signalled when either battery level changes.
    pub battery_changed: Signal<CriticalSectionRawMutex, ()>,
    /// Profile keys from the keymap.
    pub commands: Channel<CriticalSectionRawMutex, Command, 4>,
    /// What a pairing computer asks of the keyboard's user, if anything
    /// ([`HidState::pairing`]).
    pairing: Mutex<CriticalSectionRawMutex, Cell<Option<PairRequest>>>,
    /// The user's answer ([`HidState::reply_pairing`]).
    pair_reply: Signal<CriticalSectionRawMutex, PairReply>,
}

impl Default for HidState {
    fn default() -> Self {
        Self::new()
    }
}

impl HidState {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            reports: Channel::new(),
            mouse_reports: Channel::new(),
            mouse_feature: AtomicU8::new(0),
            active: AtomicU8::new(0),
            connected: AtomicBool::new(false),
            open: AtomicBool::new(true),
            state_changed: Signal::new(),
            battery: AtomicU8::new(100),
            right_battery: AtomicU8::new(255),
            battery_changed: Signal::new(),
            commands: Channel::new(),
            pairing: Mutex::new(Cell::new(None)),
            pair_reply: Signal::new(),
        }
    }

    /// What a computer that is pairing asks of the keyboard's user: type the
    /// code it shows (passkey entry), or compare one (numeric comparison).
    /// The key loop answers it with [`HidState::reply_pairing`]; meanwhile
    /// nothing typed goes to a computer (`kb_core::link::PairPrompt`).
    pub fn pairing(&self) -> Option<PairRequest> {
        self.pairing.lock(Cell::get)
    }

    /// Answer the [`HidState::pairing`] request (ignored when there is none).
    pub fn reply_pairing(&self, reply: PairReply) {
        if self.pairing().is_some() {
            self.pair_reply.signal(reply);
        }
    }

    fn ask(&self, request: PairRequest) {
        self.pair_reply.reset();
        self.pairing.lock(|c| c.set(Some(request)));
        self.state_changed.signal(());
    }

    fn stop_asking(&self) {
        if self.pairing.lock(core::cell::Cell::take).is_some() {
            self.state_changed.signal(());
        }
    }
}

/// Profile keys from the keymap (`&bt BT_NXT` and friends).
#[derive(Clone, Copy, Debug)]
pub enum Command {
    Next,
    Prev,
    /// Forget the selected profile's computer.
    Clear,
    /// Select profile `n` (0-based).
    Select(u8),
}

/// Bytes of a mouse report.
pub const MOUSE_REPORT_LEN: usize = kb_core::mouse::REPORT_LEN;

/// Report ID of the keyboard (input and LED output) in the report map.
pub const KEYBOARD_REPORT_ID: u8 = 1;
/// Report ID of the mouse.
pub const MOUSE_REPORT_ID: u8 = 2;
/// Report ID of Raw HID (messages from the computer, `kb_core::host`).
pub const RAW_HID_REPORT_ID: u8 = kb_core::host::BLE_REPORT_ID;
const RAW_HID_MAP: [u8; kb_core::host::BLE_REPORT_DESCRIPTOR.len()] =
    kb_core::host::BLE_REPORT_DESCRIPTOR;
/// Bytes of a Raw HID report.
pub const RAW_HID_REPORT_LEN: usize = kb_core::host::REPORT_SIZE;

/// Boot keyboard layout (modifiers, reserved byte, 5 LED outputs, six keys;
/// the same as the USB keyboard's reports) with report ID 1.
const KEYBOARD_MAP: [u8; 69] = [
    0x05,
    0x01,
    0x09,
    0x06,
    0xA1,
    0x01,
    0x85,
    KEYBOARD_REPORT_ID,
    0x05,
    0x07,
    0x19,
    0xE0,
    0x29,
    0xE7,
    0x15,
    0x00,
    0x25,
    0x01,
    0x75,
    0x01,
    0x95,
    0x08,
    0x81,
    0x02,
    0x15,
    0x00,
    0x26,
    0xFF,
    0x00,
    0x75,
    0x08,
    0x95,
    0x01,
    0x81,
    0x03,
    0x05,
    0x08,
    0x19,
    0x01,
    0x29,
    0x05,
    0x25,
    0x01,
    0x75,
    0x01,
    0x95,
    0x05,
    0x91,
    0x02,
    0x95,
    0x03,
    0x91,
    0x03,
    0x05,
    0x07,
    0x19,
    0x00,
    0x29,
    0xDD,
    0x26,
    0xFF,
    0x00,
    0x75,
    0x08,
    0x95,
    0x06,
    0x81,
    0x00,
    0xC0,
];

const MOUSE_MAP_LEN: usize = kb_core::mouse::REPORT_DESCRIPTOR.len() + 2;
const MOUSE_MAP: [u8; MOUSE_MAP_LEN] = kb_core::mouse::mouse_descriptor(Some(MOUSE_REPORT_ID));

/// Bytes of the report map.
pub const REPORT_MAP_LEN: usize = KEYBOARD_MAP.len() + MOUSE_MAP_LEN + RAW_HID_MAP.len();

/// The report map: the keyboard (ID 1), the mouse (ID 2), then Raw HID (ID 3).
pub static REPORT_MAP: [u8; REPORT_MAP_LEN] = {
    let mut out = [0u8; REPORT_MAP_LEN];
    let mut i = 0;
    while i < KEYBOARD_MAP.len() {
        out[i] = KEYBOARD_MAP[i];
        i += 1;
    }
    let mut j = 0;
    while j < MOUSE_MAP_LEN {
        out[i + j] = MOUSE_MAP[j];
        j += 1;
    }
    let mut k = 0;
    while k < RAW_HID_MAP.len() {
        out[i + j + k] = RAW_HID_MAP[k];
        k += 1;
    }
    out
};

#[gatt_server]
pub struct HidServer {
    pub hid: HidService,
    pub battery: BatteryService,
    pub right_battery: RightBatteryService,
    pub info: DeviceInfoService,
}

#[gatt_service(uuid = service::HUMAN_INTERFACE_DEVICE)]
pub struct HidService {
    /// HID 1.11, no country code, normally connectable.
    #[characteristic(uuid = "2a4a", read, value = [0x11, 0x01, 0x00, 0x02], permissions(encrypted))]
    pub hid_info: [u8; 4],
    #[characteristic(uuid = "2a4b", read, value = REPORT_MAP, permissions(encrypted))]
    pub report_map: [u8; REPORT_MAP_LEN],
    #[characteristic(uuid = "2a4c", write_without_response, permissions(encrypted))]
    pub control_point: u8,
    /// Report protocol.
    #[characteristic(
        uuid = "2a4e",
        read,
        write_without_response,
        value = 1,
        permissions(encrypted)
    )]
    pub protocol_mode: u8,
    /// Keyboard input report (report ID 1, input).
    #[descriptor(uuid = "2908", read = encrypted, value = [KEYBOARD_REPORT_ID, 1u8])]
    #[characteristic(uuid = "2a4d", read, notify, permissions(encrypted))]
    pub input: [u8; 8],
    /// Mouse input report (report ID 2, input).
    #[descriptor(uuid = "2908", read = encrypted, value = [MOUSE_REPORT_ID, 1u8])]
    #[characteristic(uuid = "2a4d", read, notify, permissions(encrypted))]
    pub mouse_input: [u8; MOUSE_REPORT_LEN],
    /// Mouse feature report (report ID 2, feature): the Resolution
    /// Multipliers the computer sets for smooth scrolling.
    #[descriptor(uuid = "2908", read = encrypted, value = [MOUSE_REPORT_ID, 3u8])]
    #[characteristic(uuid = "2a4d", read, write, permissions(encrypted))]
    pub mouse_feature: [u8; 1],
    /// Keyboard LED output report (report ID 1, output).
    #[descriptor(uuid = "2908", read = encrypted, value = [KEYBOARD_REPORT_ID, 2u8])]
    #[characteristic(
        uuid = "2a4d",
        read,
        write,
        write_without_response,
        permissions(encrypted)
    )]
    pub output: [u8; 1],
    /// Raw HID output report (report ID 3, output): messages for the right
    /// screen, one chunk per write (`kb_core::host`).
    #[descriptor(uuid = "2908", read = encrypted, value = [RAW_HID_REPORT_ID, 2u8])]
    #[characteristic(
        uuid = "2a4d",
        read,
        write,
        write_without_response,
        value = [0; RAW_HID_REPORT_LEN],
        permissions(encrypted)
    )]
    pub raw_hid: [u8; RAW_HID_REPORT_LEN],
}

/// Presentation format of a battery level: uint8, percent, Bluetooth SIG
/// namespace, description "left" (0x010D) or "right" (0x010E), so the
/// computer can tell the halves apart.
const fn battery_format(description: u8) -> [u8; 7] {
    [0x04, 0x00, 0xAD, 0x27, 0x01, description, 0x01]
}

/// The left half's battery (the keyboard's, for the computer).
#[gatt_service(uuid = service::BATTERY)]
pub struct BatteryService {
    #[descriptor(uuid = "2904", read, value = battery_format(0x0D))]
    #[characteristic(uuid = characteristic::BATTERY_LEVEL, read, notify, value = 100)]
    pub level: u8,
}

/// The right half's battery, like ZMK's split battery proxy: a second
/// battery service, described as "right".
#[gatt_service(uuid = service::BATTERY)]
pub struct RightBatteryService {
    #[descriptor(uuid = "2904", read, value = battery_format(0x0E))]
    #[descriptor(uuid = "2901", read, value = "Right half")]
    #[characteristic(uuid = characteristic::BATTERY_LEVEL, read, notify, value = 100)]
    pub level: u8,
}

#[gatt_service(uuid = service::DEVICE_INFORMATION)]
pub struct DeviceInfoService {
    /// USB vendor ID source, VID 0x1209, PID 0x0001, version 1 (as on USB).
    #[characteristic(uuid = characteristic::PNP_ID, read, value = [0x02, 0x09, 0x12, 0x01, 0x00, 0x01, 0x00])]
    pub pnp_id: [u8; 7],
}

/// Which computer (if any) each profile belongs to, and the selected one.
/// The rules are `kb_core::link::Profiles`.
type Profiles = RefCell<link::Profiles<Identity, PROFILES>>;

fn publish(hid: &HidState, profiles: &Profiles) {
    let p = profiles.borrow();
    hid.active.store(p.active() as u8, Ordering::Relaxed);
    hid.open.store(p.is_open(), Ordering::Relaxed);
    hid.state_changed.signal(());
}

/// The keyboard's GATT server for the computer, named `name`.
///
/// Only one can be built per program: trouble's `#[gatt_server]` keeps the
/// report map (more than 20 bytes) in a `static`, and a second server
/// panics. A restarted left half reuses the first one.
///
/// # Panics
///
/// When trouble can't build the attribute table (more attributes than its
/// table holds), which no build of this firmware does.
#[must_use]
pub fn hid_server(name: &str) -> HidServer<'_> {
    HidServer::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name,
        appearance: &appearance::human_interface_device::KEYBOARD,
    }))
    .unwrap_or_else(|_| panic!("hid server"))
}

/// Load the profiles, then serve the selected one forever.
#[allow(clippy::too_many_arguments)]
pub async fn run<C, S: NorFlash>(
    state: &LeftState<'_>,
    stack: &Stack<'_, C, DefaultPacketPool>,
    peripheral: &mut Peripheral<'_, C, DefaultPacketPool>,
    store: &Store<S>,
    server: &HidServer<'_>,
    name: &'static str,
    timing: &Timing,
) -> !
where
    C: Controller
        + ControllerCmdAsync<LeConnUpdate>
        + ControllerCmdSync<LeReadLocalSupportedFeatures>,
{
    let profiles: Profiles = {
        let mut buf = [0u8; 128];
        let mut map = store.lock().await;
        let mut slots = [None; PROFILES];
        for (i, slot) in slots.iter_mut().enumerate() {
            if let Some(bond) =
                storage::load_bond(&mut map, &mut buf, storage::profile_bond(i)).await
            {
                *slot = Some(bond.identity);
                let _ = stack.add_bond_information(bond);
            }
        }
        let active = match map
            .fetch_item::<StoredProfile>(&mut buf, &ACTIVE_PROFILE)
            .await
        {
            Ok(Some(StoredProfile(p))) => p as usize,
            _ => 0,
        };
        RefCell::new(link::Profiles::restore(slots, active))
    };
    publish(&state.hid, &profiles);
    ring_log!(
        state.log,
        "host: profile {} ({})",
        profiles.borrow().active() + 1,
        if state.hid.open.load(Ordering::Relaxed) {
            "open"
        } else {
            "taken"
        }
    );

    let _ = &server.info; // read by the computer only
    let changed = Signal::<NoopRawMutex, ()>::new();

    join(
        commands(state, stack, store, &profiles, &changed),
        connections(
            state, stack, peripheral, store, server, &profiles, &changed, name, timing,
        ),
    )
    .await;
    unreachable!()
}

/// Profile keys: switch or clear, persist, and restart the connection.
async fn commands<C: Controller, S: NorFlash>(
    state: &LeftState<'_>,
    stack: &Stack<'_, C, DefaultPacketPool>,
    store: &Store<S>,
    profiles: &Profiles,
    changed: &Signal<NoopRawMutex, ()>,
) -> ! {
    let mut buf = [0u8; 128];
    loop {
        let command = state.hid.commands.receive().await;
        match command {
            Command::Next => profiles.borrow_mut().next(),
            Command::Prev => profiles.borrow_mut().prev(),
            Command::Select(n) => profiles.borrow_mut().select(n as usize),
            Command::Clear => {
                let i = profiles.borrow().active();
                let forgotten = profiles.borrow_mut().clear();
                if let Some(identity) = forgotten {
                    let _ = stack.remove_bond_information(identity);
                }
                let mut map = store.lock().await;
                storage::store_bond(
                    &mut map,
                    &mut buf,
                    storage::profile_bond(i),
                    None,
                    state.log,
                )
                .await;
            }
        }
        let active = profiles.borrow().active();
        if !matches!(command, Command::Clear) {
            let mut map = store.lock().await;
            let _ = map
                .store_item(&mut buf, &ACTIVE_PROFILE, &StoredProfile(active as u8))
                .await;
        }
        publish(&state.hid, profiles);
        ring_log!(state.log, "host: {:?} -> profile {}", command, active + 1);
        changed.signal(());
    }
}

/// Advertise for the selected profile, serve its computer, repeat.
#[allow(clippy::too_many_arguments)]
async fn connections<C, S: NorFlash>(
    state: &LeftState<'_>,
    stack: &Stack<'_, C, DefaultPacketPool>,
    peripheral: &mut Peripheral<'_, C, DefaultPacketPool>,
    store: &Store<S>,
    server: &HidServer<'_>,
    profiles: &Profiles,
    changed: &Signal<NoopRawMutex, ()>,
    name: &str,
    timing: &Timing,
) -> !
where
    C: Controller
        + ControllerCmdAsync<LeConnUpdate>
        + ControllerCmdSync<LeReadLocalSupportedFeatures>,
{
    let hid = &state.hid;
    loop {
        hid.connected.store(false, Ordering::Relaxed);
        hid.state_changed.signal(());
        hid.reports.clear();
        hid.mouse_reports.clear();
        hid.mouse_feature.store(0, Ordering::Relaxed);
        changed.reset();
        let (i, open) = {
            let p = profiles.borrow();
            (p.active(), p.is_open())
        };
        // Fast for a while, then slow (`kb_core::link::pace`), restarting
        // the advertising when the pace changes.
        let started = Instant::now();
        let conn = loop {
            let (pace, change) = timing.pace(crate::elapsed_ms(started));
            let slow_down = async {
                match change {
                    Some(ms) => Timer::after(Duration::from_millis(u64::from(ms))).await,
                    None => core::future::pending().await,
                }
            };
            state.radio.advertising(state.log);
            let tx_power = crate::tx_power(state.radio.dbm());
            match select3(
                advertise(name, peripheral, server, open, pace, tx_power),
                changed.wait(),
                slow_down,
            )
            .await
            {
                Either3::First(Ok(conn)) => break Some(conn),
                Either3::First(Err(e)) => {
                    ring_log!(state.log, "host: advertising failed: {:?}", e);
                    Timer::after(Duration::from_secs(1)).await;
                }
                Either3::Second(()) => break None,
                Either3::Third(()) => ring_log!(state.log, "host: advertising slower now"),
            }
        };
        let Some(conn) = conn else { continue };
        ring_log!(
            state.log,
            "host: connection from {:?}",
            conn.raw().peer_address()
        );
        let _ = conn.raw().set_bondable(open);
        state.radio.connected(
            crate::radio::Link::Host,
            conn.raw().handle().raw(),
            state.log,
        );
        let outcome = select(
            serve(state, stack, &conn, server, store, profiles, i, timing),
            changed.wait(),
        )
        .await;
        if let Either::Second(()) = outcome {
            conn.raw().disconnect();
        }
        state.radio.disconnected(crate::radio::Link::Host);
        hid.stop_asking();
        hid.connected.store(false, Ordering::Relaxed);
        hid.state_changed.signal(());
        ring_log!(state.log, "host: connection closed");
    }
}

async fn advertise<'d, 's, C: Controller>(
    name: &str,
    peripheral: &mut Peripheral<'d, C, DefaultPacketPool>,
    server: &'s HidServer<'_>,
    open: bool,
    pace: link::Pace,
    tx_power: TxPower,
) -> Result<GattConnection<'d, 's, DefaultPacketPool>, BleHostError<C::Error>> {
    // Discoverable only while the profile is open; its own computer reconnects
    // without that flag.
    let flags = if open {
        LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED
    } else {
        BR_EDR_NOT_SUPPORTED
    };
    let appearance = appearance::human_interface_device::KEYBOARD.to_le_bytes();
    let mut adv = [0u8; 31];
    let len = AdStructure::encode_slice(
        &[
            AdStructure::Flags(flags),
            AdStructure::CompleteServiceUuids16(&[
                service::HUMAN_INTERFACE_DEVICE.to_le_bytes(),
                service::BATTERY.to_le_bytes(),
            ]),
            AdStructure::Unknown {
                ty: 0x19, // Appearance
                data: &appearance,
            },
            AdStructure::CompleteLocalName(name.as_bytes()),
        ],
        &mut adv[..],
    )?;
    let advertiser = peripheral
        .advertise(
            &{
                let (min, max) = link::advertising_interval_us(pace);
                AdvertisementParameters {
                    interval_min: Duration::from_micros(u64::from(min)),
                    interval_max: Duration::from_micros(u64::from(max)),
                    // Legacy advertising ignores it (the controller's
                    // per-role power applies, `crate::radio`); kept for an
                    // extended advertiser.
                    tx_power,
                    ..Default::default()
                }
            },
            Advertisement::ConnectableScannableUndirected {
                adv_data: &adv[..len],
                scan_data: &[],
            },
        )
        .await?;
    let conn = advertiser.accept().await?;
    conn.with_attribute_server(server)
        .map_err(BleHostError::BleHost)
}

/// Serve one computer until it disconnects: check it belongs to the profile,
/// store a new pairing, send key reports and battery level.
#[allow(clippy::too_many_arguments)]
async fn serve<C, S: NorFlash>(
    state: &LeftState<'_>,
    stack: &Stack<'_, C, DefaultPacketPool>,
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
    server: &HidServer<'_>,
    store: &Store<S>,
    profiles: &Profiles,
    i: usize,
    timing: &Timing,
) where
    C: Controller
        + ControllerCmdAsync<LeConnUpdate>
        + ControllerCmdSync<LeReadLocalSupportedFeatures>,
{
    let hid = &state.hid;
    let log = &state.log;
    let secure = Signal::<NoopRawMutex, ()>::new();
    // Whether this profile's computer is in (`secure` is consumed by the key
    // sender, so it can't tell later).
    let is_secure = core::cell::Cell::new(false);
    // What this profile's computer subscribed to, as last stored.
    let saved = core::cell::RefCell::new(None::<storage::StoredSubscriptions>);
    let events = async {
        let mut buf = [0u8; 128];
        // Raw HID chunks from this computer (`kb_core::host`).
        let mut messages = kb_core::host::Reassembler::new();
        // While a code is on the screen: until when it waits for an answer.
        let mut prompt: Option<Instant> = None;
        loop {
            let event = next_event(state, conn, &mut prompt).await;
            match event {
                // Passkey entry: the computer shows a code, the user types
                // it on the keyboard (the keyboard has no screen as far as
                // the computer knows, so this works when it is the computer's
                // only keyboard).
                GattConnectionEvent::PassKeyInput => {
                    prompt = Some(ask_user(state, PairRequest::TypeCode, timing));
                }
                // Numeric comparison (if a computer gets it anyway): the
                // computer shows the same 6 digits; Enter if they match.
                GattConnectionEvent::PassKeyConfirm(key) => {
                    let code = key.value() % 1_000_000;
                    prompt = Some(ask_user(state, PairRequest::Compare(code), timing));
                }
                GattConnectionEvent::PassKeyDisplay(key) => {
                    ring_log!(
                        log,
                        "host: pairing code {:06} (to type on the computer)",
                        key.value()
                    );
                }
                GattConnectionEvent::Disconnected { reason } => {
                    ring_log!(log, "host: disconnected: {:?}", reason);
                    return;
                }
                // A computer paired on this (open) profile.
                GattConnectionEvent::PairingComplete {
                    bond: Some(bond),
                    security_level,
                } => {
                    hid.stop_asking();
                    log_paired(log, security_level);
                    if !store_pairing(state, store, profiles, i, bond, &mut buf).await {
                        conn.raw().disconnect();
                        continue;
                    }
                    // A new computer: forget the previous one's subscriptions.
                    save_subscriptions(server, conn, store, &saved, i, log).await;
                    is_secure.set(true);
                    secure.signal(());
                }
                // A computer reconnected with its stored bond: only this profile's may stay.
                GattConnectionEvent::Encrypted {
                    bond: Some(bond), ..
                } => {
                    let ours = profiles.borrow().reconnected(bond.identity);
                    if ours {
                        ring_log!(log, "host: profile {} computer reconnected", i + 1);
                        restore_subscriptions(server, conn, store, &saved, i, log).await;
                        is_secure.set(true);
                        secure.signal(());
                    } else {
                        ring_log!(log, "host: another profile's computer; dropping");
                        conn.raw().disconnect();
                    }
                }
                GattConnectionEvent::PairingFailed(e) => {
                    prompt = None;
                    hid.stop_asking();
                    ring_log!(log, "host: pairing failed: {:?}", e);
                    conn.raw().disconnect();
                }
                GattConnectionEvent::BondLost => {
                    ring_log!(log, "host: the computer has no pairing with us (yet)");
                }
                GattConnectionEvent::Gatt { event } => {
                    let for_screen =
                        take_screen_message(state, server, &event, &mut messages, is_secure.get());
                    if !for_screen {
                        log_gatt(log, &event);
                        take_mouse_feature(state, server, &event);
                    }
                    let write = !for_screen && matches!(event, GattEvent::Write(_));
                    if let Ok(reply) = event.accept() {
                        reply.send().await;
                    }
                    // A subscription may have changed: keep it for after a restart.
                    if write && is_secure.get() {
                        save_subscriptions(server, conn, store, &saved, i, log).await;
                    }
                }
                _ => {}
            }
        }
    };
    let keys = send_reports(state, stack, conn, server, timing, &secure, &is_secure);
    select(events, keys).await;
}

/// Log how a computer paired: with a code or without.
fn log_paired(log: &crate::LogRing, security_level: SecurityLevel) {
    if security_level == SecurityLevel::EncryptedAuthenticated {
        ring_log!(log, "host: paired with a code (MITM protected)");
    } else {
        ring_log!(
            log,
            "host: paired without a code (the computer can't show or type one)"
        );
    }
}

/// Put pairing `request` up for the user (and in the log); until when it
/// waits for the answer.
fn ask_user(state: &LeftState<'_>, request: PairRequest, timing: &Timing) -> Instant {
    match request {
        PairRequest::TypeCode => ring_log!(
            state.log,
            "host: pairing with a code: type the computer's code, Enter"
        ),
        PairRequest::Compare(code) => ring_log!(
            state.log,
            "host: pairing by comparing {:06}: Enter if it matches",
            code
        ),
    }
    state.hid.ask(request);
    Instant::now() + timing.pair_timeout
}

/// The next event from the computer. While a pairing prompt is up (until
/// `prompt`), the user's answer, or none in time, goes to the computer in
/// between.
async fn next_event<'c, 'v>(
    state: &LeftState<'_>,
    conn: &GattConnection<'c, 'v, DefaultPacketPool>,
    prompt: &mut Option<Instant>,
) -> GattConnectionEvent<'c, 'v, DefaultPacketPool> {
    let hid = &state.hid;
    loop {
        let Some(deadline) = *prompt else {
            return conn.next().await;
        };
        let answer = select(hid.pair_reply.wait(), Timer::at(deadline));
        match select(conn.next(), answer).await {
            Either::First(event) => return event,
            Either::Second(answer) => {
                *prompt = None;
                let asked = hid.pairing();
                hid.stop_asking();
                answer_pairing(conn, state.log, answer, asked);
            }
        }
    }
}

/// The user's `answer` to the pairing prompt `asked` (`Second`: none in
/// time), passed on to the computer.
fn answer_pairing(
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
    log: &crate::LogRing,
    answer: Either<PairReply, ()>,
    asked: Option<PairRequest>,
) {
    match (answer, asked) {
        (Either::First(PairReply::Passkey(code)), _) => {
            ring_log!(log, "host: pairing code typed");
            let _ = conn.raw().pass_key_input(code);
        }
        (Either::First(PairReply::Confirm), _) => {
            ring_log!(log, "host: pairing code confirmed");
            let _ = conn.raw().pass_key_confirm();
        }
        (Either::First(PairReply::Reject), asked) => {
            ring_log!(log, "host: pairing rejected on the keyboard");
            reject(conn, asked);
        }
        (Either::Second(()), asked) => {
            ring_log!(log, "host: pairing code not answered in time");
            reject(conn, asked);
        }
    }
}

/// A Raw HID write: a message for the screen, put together and passed on,
/// from this profile's computer only (`secure`). Not in the log byte by
/// byte (it's text the user may not want there). Whether `event` was one.
fn take_screen_message(
    state: &LeftState<'_>,
    server: &HidServer<'_>,
    event: &GattEvent<'_, '_, DefaultPacketPool>,
    messages: &mut kb_core::host::Reassembler,
    secure: bool,
) -> bool {
    let GattEvent::Write(w) = event else {
        return false;
    };
    if w.handle() != server.hid.raw_hid.handle {
        return false;
    }
    let message = w.with_data(|offset, data| match offset {
        0 if secure => messages.feed(data),
        _ => None,
    });
    if let Some(message) = message {
        ring_log!(state.log, "host: message for the screen (Bluetooth)");
        state.inbox.deliver(message);
    }
    true
}

/// A write of the mouse's feature report: the Resolution Multipliers
/// (smooth scrolling).
fn take_mouse_feature(
    state: &LeftState<'_>,
    server: &HidServer<'_>,
    event: &GattEvent<'_, '_, DefaultPacketPool>,
) {
    if let GattEvent::Write(w) = event
        && w.handle() == server.hid.mouse_feature.handle
    {
        let value = w.with_data(|_, data| data.first().copied());
        if let Some(value) = value {
            state.hid.mouse_feature.store(value, Ordering::Relaxed);
            ring_log!(state.log, "host: mouse resolution {:02x}", value);
        }
    }
}

/// What the computer asks for, in the log (finding out why a host doesn't
/// take the keys).
fn log_gatt(log: &crate::LogRing, event: &GattEvent<'_, '_, DefaultPacketPool>) {
    match event {
        GattEvent::Read(r) => ring_log!(log, "gatt: read {}", r.handle()),
        GattEvent::Write(w) => {
            w.with_data(|offset, data| {
                ring_log!(log, "gatt: write {} @{} {:02x?}", w.handle(), offset, data);
            });
        }
        GattEvent::Other(o) => ring_log!(log, "gatt: other {:?}", o),
        GattEvent::NotAllowed(n) => ring_log!(log, "gatt: not allowed {:?}", n),
    }
}

/// A computer paired on profile `i`: store its bond there (and clear the
/// profiles it was on before). Whether it may stay: `false` for a new
/// pairing on a profile another computer has.
async fn store_pairing<S: NorFlash>(
    state: &LeftState<'_>,
    store: &Store<S>,
    profiles: &Profiles,
    i: usize,
    bond: BondInformation,
    buf: &mut [u8],
) -> bool {
    let (hid, log) = (&state.hid, &state.log);
    let identity = bond.identity;
    let outcome = profiles.borrow_mut().paired(identity);
    if !outcome.accept {
        ring_log!(log, "host: new pairing on a taken profile; dropping");
        return false;
    }
    let mut map = store.lock().await;
    for j in (0..PROFILES).filter(|&j| outcome.cleared[j]) {
        storage::store_bond(&mut map, buf, storage::profile_bond(j), None, log).await;
    }
    storage::store_bond(&mut map, buf, storage::profile_bond(i), Some(bond), log).await;
    drop(map);
    hid.open.store(false, Ordering::Relaxed);
    hid.state_changed.signal(());
    ring_log!(log, "host: paired {:?} on profile {}", identity.addr, i + 1);
    true
}

/// Once the link is `secure` for this profile's computer: ask for a short
/// connection interval, then send it the key, mouse and battery reports
/// until one doesn't go out.
async fn send_reports<C>(
    state: &LeftState<'_>,
    stack: &Stack<'_, C, DefaultPacketPool>,
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
    server: &HidServer<'_>,
    timing: &Timing,
    secure: &Signal<NoopRawMutex, ()>,
    is_secure: &core::cell::Cell<bool>,
) where
    C: Controller
        + ControllerCmdAsync<LeConnUpdate>
        + ControllerCmdSync<LeReadLocalSupportedFeatures>,
{
    let (hid, log) = (&state.hid, &state.log);
    // Nothing goes out before the link is encrypted for this profile's computer.
    // A code on the screen holds the timeout off (it has its own).
    let deadline = Instant::now() + timing.host_secure_timeout;
    loop {
        match select(secure.wait(), Timer::at(deadline)).await {
            Either::First(()) => break,
            Either::Second(()) if hid.pairing().is_some() => {
                Timer::after(Duration::from_millis(100)).await;
                if is_secure.get() {
                    break;
                }
            }
            Either::Second(()) => {
                ring_log!(log, "host: not secured in time; dropping");
                conn.raw().disconnect();
                core::future::pending::<()>().await;
            }
        }
    }
    // Ask the computer for a short interval, like ZMK (7.5-15 ms, latency 30);
    // it decides. Hosts usually start slower.
    let params = RequestedConnParams {
        min_connection_interval: Duration::from_micros(7_500),
        max_connection_interval: Duration::from_millis(15),
        max_latency: 30,
        supervision_timeout: Duration::from_secs(4),
        ..Default::default()
    };
    if let Err(e) = conn.raw().update_connection_params(stack, &params).await {
        ring_log!(log, "host: connection parameter request failed: {:?}", e);
    }
    hid.connected.store(true, Ordering::Relaxed);
    hid.state_changed.signal(());
    hid.reports.clear();
    hid.mouse_reports.clear();
    notify_batteries(server, hid, conn).await;
    loop {
        match select3(
            hid.reports.receive(),
            hid.mouse_reports.receive(),
            hid.battery_changed.wait(),
        )
        .await
        {
            Either3::First(report) => {
                if let Err(e) = server.hid.input.notify(conn, &report, false).await {
                    ring_log!(log, "host: report not sent: {:?}", e);
                    return;
                }
            }
            Either3::Second(report) => {
                if let Err(e) = server.hid.mouse_input.notify(conn, &report, false).await {
                    ring_log!(log, "host: mouse report not sent: {:?}", e);
                    return;
                }
            }
            Either3::Third(()) => {
                notify_batteries(server, hid, conn).await;
            }
        }
    }
}

/// Turn a pairing down: a compared code with a "no" (the computer hears the
/// pairing failed); a code to type can't be cancelled in trouble, so the
/// link drops.
fn reject(conn: &GattConnection<'_, '_, DefaultPacketPool>, asked: Option<PairRequest>) {
    match asked {
        Some(PairRequest::Compare(_)) => {
            let _ = conn.raw().pass_key_cancel();
        }
        _ => conn.raw().disconnect(),
    }
}

/// Tell the computer both battery levels (the right half's once known).
/// Stored for reads too: a notification only reaches a subscriber.
async fn notify_batteries(
    server: &HidServer<'_>,
    hid: &HidState,
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
) {
    let left = hid.battery.load(Ordering::Relaxed);
    let _ = server.battery.level.set(server, &left);
    let _ = server.battery.level.notify(conn, &left, false).await;
    let right = hid.right_battery.load(Ordering::Relaxed);
    if right <= 100 {
        let _ = server.right_battery.level.set(server, &right);
        let _ = server.right_battery.level.notify(conn, &right, false).await;
    }
}

/// Store what this connection's computer subscribed to under profile `i`,
/// if it changed since `saved`.
async fn save_subscriptions<S: NorFlash>(
    server: &HidServer<'_>,
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
    store: &Store<S>,
    saved: &core::cell::RefCell<Option<storage::StoredSubscriptions>>,
    i: usize,
    log: &crate::LogRing,
) {
    let Some(table) = server.get_client_att_table(conn.raw()) else {
        return;
    };
    let now = storage::StoredSubscriptions::new(table.raw());
    if saved.borrow().as_ref() == Some(&now) {
        return;
    }
    let mut buf = [0u8; 128];
    let mut map = store.lock().await;
    match map
        .store_item(&mut buf, &storage::profile_subscriptions(i), &now)
        .await
    {
        Ok(()) => {
            ring_log!(log, "host: subscriptions stored for profile {}", i + 1);
            *saved.borrow_mut() = Some(now);
        }
        Err(e) => ring_log!(log, "storage: subscriptions not stored: {:?}", e),
    }
}

/// Give a reconnecting computer back what it subscribed to before (it may
/// not subscribe again: Windows doesn't).
async fn restore_subscriptions<S: NorFlash>(
    server: &HidServer<'_>,
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
    store: &Store<S>,
    saved: &core::cell::RefCell<Option<storage::StoredSubscriptions>>,
    i: usize,
    log: &crate::LogRing,
) {
    let mut buf = [0u8; 128];
    let stored = {
        let mut map = store.lock().await;
        map.fetch_item::<storage::StoredSubscriptions>(&mut buf, &storage::profile_subscriptions(i))
            .await
    };
    if let Ok(Some(stored)) = stored {
        let mut raw = [0u8; 64];
        let len = stored.raw(&mut raw);
        if let Ok(view) = ClientAttTableView::try_from_raw(&raw[..len]) {
            server.set_client_att_table(conn.raw(), &view);
            ring_log!(log, "host: subscriptions restored for profile {}", i + 1);
        }
        *saved.borrow_mut() = Some(stored);
    }
    // A computer paired as a keyboard always wants the key and mouse
    // reports and the battery levels: count it subscribed even if nothing
    // (right) was stored, or a host that doesn't subscribe again (Windows)
    // gets no keys.
    if let Some(mut table) = server.get_client_att_table(conn.raw()) {
        let notify = [0x01, 0x00];
        for cccd in [
            server.hid.input.cccd_handle,
            server.hid.mouse_input.cccd_handle,
            server.battery.level.cccd_handle,
            server.right_battery.level.cccd_handle,
        ]
        .into_iter()
        .flatten()
        {
            let _ = table.write(cccd, 0, &notify);
        }
        server.set_client_att_table(conn.raw(), &table.view());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kb_core::mouse::input_report_len;

    #[test]
    fn report_map_declares_the_keyboard_as_report_1() {
        // Arrange / Act
        let len = input_report_len(&REPORT_MAP, KEYBOARD_REPORT_ID);

        // Assert: modifiers, reserved, six keys
        assert_eq!(len, 8);
    }

    #[test]
    fn report_map_declares_the_mouse_as_report_2() {
        // Arrange / Act
        let len = input_report_len(&REPORT_MAP, MOUSE_REPORT_ID);

        // Assert
        assert_eq!(len, MOUSE_REPORT_LEN);
    }

    #[test]
    fn report_map_has_no_reports_without_an_id() {
        // Arrange / Act
        let len = input_report_len(&REPORT_MAP, 0);

        // Assert
        assert_eq!(len, 0);
    }

    #[test]
    fn report_map_is_three_application_collections() {
        // Arrange / Act
        let opened = REPORT_MAP.windows(2).filter(|w| w == &[0xA1, 0x01]).count();

        // Assert: keyboard, mouse, Raw HID
        assert_eq!(opened, 3);
    }

    #[test]
    fn report_map_declares_raw_hid_as_output_report_3() {
        // Arrange / Act
        let len = kb_core::mouse::output_report_len(&REPORT_MAP, RAW_HID_REPORT_ID);

        // Assert
        assert_eq!(len, RAW_HID_REPORT_LEN);
    }

    #[test]
    fn report_map_ends_with_the_raw_hid_collection() {
        // Arrange: Usage Page (0xFF60), Usage (0x61), Collection, Report ID (3)
        let head = [
            0x06,
            0x60,
            0xFF,
            0x09,
            0x61,
            0xA1,
            0x01,
            0x85,
            RAW_HID_REPORT_ID,
        ];

        // Act
        let tail = &REPORT_MAP[KEYBOARD_MAP.len() + MOUSE_MAP_LEN..];

        // Assert
        assert!(tail.starts_with(&head));
        assert_eq!(REPORT_MAP[REPORT_MAP_LEN - 1], 0xC0);
    }

    #[test]
    fn report_map_has_no_raw_hid_input_report() {
        // Arrange / Act
        let len = input_report_len(&REPORT_MAP, RAW_HID_REPORT_ID);

        // Assert
        assert_eq!(len, 0);
    }

    // The host's critical section, for the blocking mutexes.
    use critical_section as _;

    #[test]
    fn new_state_is_profile_1_open_without_a_computer() {
        // Arrange / Act
        let hid = HidState::default();

        // Assert
        assert_eq!(hid.active.load(Ordering::Relaxed), 0);
        assert!(hid.open.load(Ordering::Relaxed));
        assert!(!hid.connected.load(Ordering::Relaxed));
        assert_eq!(
            hid.right_battery.load(Ordering::Relaxed),
            255,
            "not known yet"
        );
        assert_eq!(hid.pairing(), None);
    }

    #[test]
    fn answer_without_a_question_is_ignored() {
        // Arrange
        let hid = HidState::new();

        // Act
        hid.reply_pairing(PairReply::Confirm);

        // Assert
        assert!(!hid.pair_reply.signaled());
    }

    #[test]
    fn question_is_up_until_the_answer_is_passed_on() {
        // Arrange
        let hid = HidState::new();
        hid.ask(PairRequest::Compare(123_456));

        // Act
        hid.reply_pairing(PairReply::Confirm);

        // Assert
        assert_eq!(hid.pairing(), Some(PairRequest::Compare(123_456)));
        assert!(hid.pair_reply.signaled());
        assert!(hid.state_changed.signaled(), "the screen shows the prompt");
    }

    #[test]
    fn new_question_forgets_an_old_answer() {
        // Arrange
        let hid = HidState::new();
        hid.ask(PairRequest::TypeCode);
        hid.reply_pairing(PairReply::Reject);

        // Act
        hid.ask(PairRequest::TypeCode);

        // Assert
        assert!(!hid.pair_reply.signaled());
    }

    #[test]
    fn taking_the_question_down_wakes_the_screen() {
        // Arrange
        let hid = HidState::new();
        hid.ask(PairRequest::TypeCode);
        hid.state_changed.reset();

        // Act
        hid.stop_asking();

        // Assert
        assert_eq!(hid.pairing(), None);
        assert!(hid.state_changed.signaled());
    }

    #[test]
    fn no_question_to_take_down_leaves_the_screen_alone() {
        // Arrange
        let hid = HidState::new();

        // Act
        hid.stop_asking();

        // Assert
        assert!(!hid.state_changed.signaled());
    }

    #[test]
    fn profile_state_is_published_for_the_screen() {
        // Arrange
        let hid = HidState::new();
        let mut slots = [None; PROFILES];
        slots[4] = Some(Identity {
            addr: Address::random([1, 2, 3, 4, 5, 6]),
            irk: None,
        });
        let profiles: Profiles = RefCell::new(link::Profiles::restore(slots, 4));

        // Act
        publish(&hid, &profiles);

        // Assert
        assert_eq!(hid.active.load(Ordering::Relaxed), 4);
        assert!(!hid.open.load(Ordering::Relaxed), "profile 5 is taken");
        assert!(hid.state_changed.signaled());
    }

    #[test]
    fn batteries_are_described_as_left_and_right() {
        // Arrange
        let (left, right) = (0x0D, 0x0E);

        // Act
        let formats = [battery_format(left), battery_format(right)];

        // Assert: percentage (uint8), Bluetooth SIG namespace, the description last but one
        assert_eq!(formats[0][..5], [0x04, 0x00, 0xAD, 0x27, 0x01]);
        assert_eq!([formats[0][5], formats[1][5]], [0x0D, 0x0E]);
    }
}
