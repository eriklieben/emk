//! An in-memory Bluetooth LE "air" with fake HCI controllers, to run several
//! [`trouble_host`] stacks against each other in a normal `cargo test`: no
//! radio, no root, no `/dev/vhci`.
//!
//! Each [`Air::controller`] is an [`ExternalController`] over a
//! [`VirtualTransport`]: trouble talks plain HCI to it (commands, events, ACL
//! data, byte for byte as over a UART), and the air plays the link layer:
//!
//! - legacy advertising and passive/active scanning: advertising data (and
//!   the scan response, when scanning actively) reach every scanning
//!   controller as LE Advertising Reports, every [`ADV_INTERVAL`];
//! - connecting: an initiator (`LE Create Connection`, with or without the
//!   filter accept list) meets a connectable advertiser and both get an LE
//!   Connection Complete with their role; the advertiser stops advertising.
//!   As on Nordic's `SoftDevice` Controller, a controller that is still
//!   scanning refuses to connect (Command Disallowed);
//! - ACL data goes to the other end of the link as it is (the host does the
//!   L2CAP, ATT and SMP), and the sender gets Number Of Completed Packets;
//! - encryption: `LE Enable Encryption` from the central asks the peripheral's
//!   host for the key (LE Long Term Key Request); the link encrypts only when
//!   both hosts gave the same key, otherwise it drops with a MIC failure, as a
//!   real link would. The pairing itself runs in the hosts, over ACL; while
//!   the encryption starts, ACL data waits (as in the link layer's
//!   encryption start procedure) and goes out once the link is encrypted;
//! - disconnect, from either end, or [`Air::out_of_range`] for a lost link;
//!   [`Air::power_off`] for a device that restarts.
//!
//! Everything happens synchronously when a host writes a packet, so a test
//! runs the same way every time; only scanning has a clock (the
//! advertising interval).
//!
//! Set `BLESIM_TRACE=1` to print every HCI packet.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::future::{Future, poll_fn};
use std::pin::pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll, Wake, Waker};

use bt_hci::ReadHciError;
use bt_hci::cmd::Cmd;
use bt_hci::cmd::controller_baseband::{
    HostBufferSize, HostNumberOfCompletedPackets, Reset, SetControllerToHostFlowControl,
    SetEventMask, SetEventMaskPage2,
};
use bt_hci::cmd::info::{
    ReadBdAddr, ReadLocalSupportedCmds, ReadLocalSupportedFeatures, ReadLocalVersionInformation,
};
use bt_hci::cmd::le::{
    LeAddDeviceToFilterAcceptList, LeAddDeviceToResolvingList, LeClearFilterAcceptList,
    LeClearResolvingList, LeConnUpdate, LeCreateConn, LeCreateConnCancel, LeEnableEncryption,
    LeLongTermKeyRequestNegativeReply, LeLongTermKeyRequestReply, LeRand, LeReadBufferSize,
    LeReadFilterAcceptListSize, LeReadLocalSupportedFeatures, LeReadPhy,
    LeRemoteConnectionParameterRequestNegativeReply, LeRemoteConnectionParameterRequestReply,
    LeRemoveDeviceFromFilterAcceptList, LeRemoveDeviceFromResolvingList, LeSetAddrResolutionEnable,
    LeSetAdvData, LeSetAdvEnable, LeSetAdvParams, LeSetDataLength, LeSetEventMask, LeSetPhy,
    LeSetPrivacyMode, LeSetRandomAddr, LeSetResolvablePrivateAddrTimeout, LeSetScanEnable,
    LeSetScanParams, LeSetScanResponseData,
};
use bt_hci::cmd::link_control::Disconnect;
use bt_hci::cmd::status::ReadRssi;
use bt_hci::controller::ExternalController;
use bt_hci::transport::{PacketKind, PacketToController, PacketToHost, Transport};
use embassy_futures::select::{Either, select};
use embassy_time::{Duration, Timer};

/// How often an advertiser's packets reach the scanners.
pub const ADV_INTERVAL: Duration = Duration::from_millis(20);

/// A fake controller, ready for `trouble_host::new`.
pub type SimController = ExternalController<VirtualTransport, 8>;

/// The shared medium all fake controllers live on.
#[derive(Clone, Default)]
pub struct Air {
    state: Arc<Mutex<AirState>>,
}

impl Air {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A new controller on this air. `name` shows up in traces and picks the
    /// device in [`Air::out_of_range`]; the controller's public address is
    /// derived from its index.
    #[must_use]
    pub fn controller(&self, name: &'static str) -> SimController {
        let mut air = self.lock();
        let id = air.nodes.len();
        air.nodes.push(Node::new(name, id));
        ExternalController::new(VirtualTransport {
            air: self.clone(),
            id,
        })
    }

    /// The public address of the controller called `name`.
    #[must_use]
    pub fn public_address(&self, name: &str) -> [u8; 6] {
        let air = self.lock();
        air.nodes[air.node(name)].public
    }

    /// Whether the controller called `name` is advertising right now.
    #[must_use]
    pub fn is_advertising(&self, name: &str) -> bool {
        let air = self.lock();
        air.nodes[air.node(name)].adv_enabled
    }

    /// How many SMP Pairing Requests went over the air so far (a central
    /// starting a new pairing).
    #[must_use]
    pub fn pairing_requests(&self) -> usize {
        self.lock().pairing_requests
    }

    /// Number of links up right now.
    #[must_use]
    pub fn link_count(&self) -> usize {
        self.lock().links.len()
    }

    /// Drop every link of the controller called `name`, as if it went out of
    /// range: both ends see a supervision timeout.
    pub fn out_of_range(&self, name: &str) {
        let mut air = self.lock();
        let id = air.node(name);
        let handles: Vec<u16> = air
            .links
            .iter()
            .filter(|l| l.central == id || l.peripheral == id)
            .map(|l| l.handle)
            .collect();
        for handle in handles {
            air.drop_link(handle, STATUS_CONN_TIMEOUT, STATUS_CONN_TIMEOUT);
        }
    }

    /// Switch the controller called `name` off, as when its device restarts
    /// (its host stack is gone): its links drop (the other ends see a
    /// supervision timeout), it stops advertising, scanning and connecting,
    /// and the name is free for the controller of the restarted device.
    pub fn power_off(&self, name: &str) {
        self.out_of_range(name);
        let mut air = self.lock();
        let id = air.node(name);
        let node = &mut air.nodes[id];
        node.reset();
        node.rx.clear();
        node.off = true;
    }

    fn lock(&self) -> MutexGuard<'_, AirState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// The HCI transport between one host and its controller on the air.
pub struct VirtualTransport {
    air: Air,
    id: usize,
}

/// Errors of the virtual transport: only malformed packets.
#[derive(Debug)]
pub enum Error {
    Read(ReadHciError<Infallible>),
    Write,
}

impl From<ReadHciError<Infallible>> for Error {
    fn from(e: ReadHciError<Infallible>) -> Self {
        Self::Read(e)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for Error {}

impl embedded_io::Error for Error {
    fn kind(&self) -> embedded_io::ErrorKind {
        embedded_io::ErrorKind::Other
    }
}

impl embedded_io::ErrorType for VirtualTransport {
    type Error = Error;
}

impl VirtualTransport {
    async fn next_packet(&self) -> Packet {
        loop {
            let scanning = self.air.lock().nodes[self.id].scanning;
            let queued = poll_fn(|cx| {
                let mut air = self.air.lock();
                let node = &mut air.nodes[self.id];
                if let Some(packet) = node.rx.pop_front() {
                    Poll::Ready(packet)
                } else {
                    node.waker = Some(cx.waker().clone());
                    Poll::Pending
                }
            });
            if !scanning {
                return queued.await;
            }
            match select(queued, Timer::after(ADV_INTERVAL)).await {
                Either::First(packet) => return packet,
                Either::Second(()) => self.air.lock().advertise_to(self.id),
            }
        }
    }
}

impl Transport for VirtualTransport {
    async fn read<'a, P: PacketToHost<'a>>(&self, rx: &'a mut [u8]) -> Result<P, Self::Error> {
        let (kind, bytes) = self.next_packet().await;
        // Let the rest of the host run before it handles this packet, as with
        // a real controller, whose packets arrive one by one: a burst would
        // otherwise overrun the host's small internal queues.
        yield_now().await;
        let mut reader: &[u8] = &bytes;
        Ok(P::read_hci(kind, &mut reader, rx)?)
    }

    #[allow(
        clippy::unused_async_trait_impl,
        reason = "the trait's method is async; the air takes a packet at once"
    )]
    async fn write<P: PacketToController>(&self, tx: &P) -> Result<(), Self::Error> {
        let mut bytes = vec![0u8; tx.size()];
        tx.write_hci(&mut bytes[..]).map_err(|_| Error::Write)?;
        let mut air = self.air.lock();
        match P::KIND {
            PacketKind::Cmd => air.command(self.id, &bytes),
            PacketKind::AclData => air.acl(self.id, &bytes),
            _ => {}
        }
        Ok(())
    }
}

/// Return `Pending` once (waking at once), so other futures get a turn.
async fn yield_now() {
    let mut yielded = false;
    poll_fn(|cx| {
        if yielded {
            Poll::Ready(())
        } else {
            yielded = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    })
    .await;
}

/// Run a future to completion on this thread (a minimal executor, enough for
/// a test that joins a few host stacks; `embassy-time` wakes it for timers).
pub fn block_on<F: Future>(future: F) -> F::Output {
    struct Unpark(std::thread::Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
        std::thread::park();
    }
}

// ---- link layer -------------------------------------------------------------------

type Packet = (PacketKind, Vec<u8>);

const STATUS_OK: u8 = 0x00;
const STATUS_UNKNOWN_CMD: u8 = 0x01;
const STATUS_UNKNOWN_CONN: u8 = 0x02;
const STATUS_KEY_MISSING: u8 = 0x06;
const STATUS_CONN_TIMEOUT: u8 = 0x08;
const STATUS_DISALLOWED: u8 = 0x0C;
const STATUS_INVALID_PARAMS: u8 = 0x12;
const STATUS_LOCAL_HOST: u8 = 0x16;
const STATUS_MIC_FAILURE: u8 = 0x3D;

/// Advertising PDU types (LE Set Advertising Parameters).
const ADV_IND: u8 = 0x00;
const ADV_DIRECT_IND_HIGH: u8 = 0x01;
const ADV_SCAN_IND: u8 = 0x02;
const ADV_DIRECT_IND_LOW: u8 = 0x04;
const SCAN_RSP: u8 = 0x04;

const fn opcode<C: Cmd>() -> u16 {
    C::OPCODE.to_raw()
}

/// An address with its type: 0 public, 1 random.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Addr {
    kind: u8,
    addr: [u8; 6],
}

impl Addr {
    /// Identity address types (2, 3) are the same addresses as 0, 1 here: the
    /// air resolves no private addresses.
    fn new(kind: u8, addr: &[u8]) -> Self {
        Self {
            kind: kind & 1,
            addr: addr.try_into().unwrap_or_default(),
        }
    }
}

#[derive(Clone, Copy)]
struct Initiating {
    use_accept_list: bool,
    peer: Addr,
    own_kind: u8,
    interval: u16,
    latency: u16,
    timeout: u16,
}

/// One controller on the air.
#[allow(
    clippy::struct_excessive_bools,
    reason = "a controller's independent switches, as HCI sets them"
)]
struct Node {
    name: &'static str,
    public: [u8; 6],
    random: Option<[u8; 6]>,
    rx: VecDeque<Packet>,
    waker: Option<Waker>,
    rand: u64,
    accept_list: Vec<Addr>,
    resolving_list: Vec<Addr>,
    // Advertising.
    adv_type: u8,
    adv_own_kind: u8,
    adv_peer: Addr,
    adv_filter: u8,
    adv_data: Vec<u8>,
    scan_data: Vec<u8>,
    adv_enabled: bool,
    // Scanning.
    scan_active: bool,
    scan_own_kind: u8,
    scan_filter: u8,
    scanning: bool,
    filter_duplicates: bool,
    seen: Vec<Addr>,
    // Connecting.
    initiating: Option<Initiating>,
    /// Switched off (`Air::power_off`): out of the air for good.
    off: bool,
}

impl Node {
    fn new(name: &'static str, id: usize) -> Self {
        let mut node = Self {
            name,
            public: [id as u8 + 1, 0x00, 0x00, 0xb1, 0xe5, 0x00],
            random: None,
            rx: VecDeque::new(),
            waker: None,
            rand: 0x5eed_0000 + id as u64,
            accept_list: Vec::new(),
            resolving_list: Vec::new(),
            adv_type: ADV_IND,
            adv_own_kind: 0,
            adv_peer: Addr::new(0, &[0; 6]),
            adv_filter: 0,
            adv_data: Vec::new(),
            scan_data: Vec::new(),
            adv_enabled: false,
            scan_active: false,
            scan_own_kind: 0,
            scan_filter: 0,
            scanning: false,
            filter_duplicates: false,
            seen: Vec::new(),
            initiating: None,
            off: false,
        };
        node.reset();
        node
    }

    /// HCI Reset: back to the power-on state (addresses, lists, procedures).
    fn reset(&mut self) {
        self.random = None;
        self.accept_list.clear();
        self.resolving_list.clear();
        self.adv_type = ADV_IND;
        self.adv_own_kind = 0;
        self.adv_filter = 0;
        self.adv_data.clear();
        self.scan_data.clear();
        self.adv_enabled = false;
        self.scan_active = false;
        self.scan_own_kind = 0;
        self.scan_filter = 0;
        self.scanning = false;
        self.seen.clear();
        self.initiating = None;
    }

    /// The device address used for an own-address type (RPAs are never
    /// generated: 2 and 3 fall back to the public or random address).
    fn own_address(&self, kind: u8) -> Addr {
        match (kind & 1, self.random) {
            (1, Some(random)) => Addr::new(1, &random),
            _ => Addr::new(0, &self.public),
        }
    }

    fn next_rand(&mut self) -> u64 {
        // splitmix64: deterministic per controller, distinct between them.
        self.rand = self.rand.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.rand;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn connectable(&self) -> bool {
        self.adv_enabled
            && matches!(
                self.adv_type,
                ADV_IND | ADV_DIRECT_IND_HIGH | ADV_DIRECT_IND_LOW
            )
    }

    fn directed(&self) -> bool {
        matches!(self.adv_type, ADV_DIRECT_IND_HIGH | ADV_DIRECT_IND_LOW)
    }
}

struct Link {
    handle: u16,
    central: usize,
    peripheral: usize,
    encrypted: bool,
    /// The key the central started encryption with, until the peripheral's
    /// host answers the key request.
    pending_ltk: Option<[u8; 16]>,
    /// ACL data sent while the encryption starts, as (receiver, packet,
    /// sender): a link layer sends no data during the encryption start
    /// procedure, so it goes out once the link is encrypted.
    held: Vec<(usize, Packet, usize)>,
}

#[derive(Default)]
struct AirState {
    nodes: Vec<Node>,
    links: Vec<Link>,
    next_handle: u16,
    pairing_requests: usize,
}

fn trace_enabled() -> bool {
    std::env::var_os("BLESIM_TRACE").is_some_and(|v| v != "0")
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

impl AirState {
    fn node(&self, name: &str) -> usize {
        self.nodes
            .iter()
            .position(|n| n.name == name && !n.off)
            .unwrap_or_else(|| panic!("no controller called {name}"))
    }

    fn push(&mut self, id: usize, packet: Packet) {
        if trace_enabled() {
            eprintln!(
                "[blesim] {} <- {:?} {:02x?}",
                self.nodes[id].name, packet.0, packet.1
            );
        }
        let node = &mut self.nodes[id];
        node.rx.push_back(packet);
        if let Some(waker) = node.waker.take() {
            waker.wake();
        }
    }

    fn event(&mut self, id: usize, code: u8, params: &[u8]) {
        let mut bytes = vec![code, params.len() as u8];
        bytes.extend_from_slice(params);
        self.push(id, (PacketKind::Event, bytes));
    }

    fn le_event(&mut self, id: usize, subevent: u8, params: &[u8]) {
        let mut bytes = vec![subevent];
        bytes.extend_from_slice(params);
        self.event(id, 0x3e, &bytes);
    }

    fn complete(&mut self, id: usize, op: u16, status: u8, ret: &[u8]) {
        let mut params = vec![1];
        params.extend_from_slice(&op.to_le_bytes());
        params.push(status);
        params.extend_from_slice(ret);
        self.event(id, 0x0e, &params);
    }

    fn status(&mut self, id: usize, op: u16, status: u8) {
        let mut params = vec![status, 1];
        params.extend_from_slice(&op.to_le_bytes());
        self.event(id, 0x0f, &params);
    }

    fn disconnection_complete(&mut self, id: usize, handle: u16, reason: u8) {
        let mut params = vec![STATUS_OK];
        params.extend_from_slice(&handle.to_le_bytes());
        params.push(reason);
        self.event(id, 0x05, &params);
    }

    fn encryption_change(&mut self, id: usize, handle: u16, status: u8, enabled: bool) {
        let mut params = vec![status];
        params.extend_from_slice(&handle.to_le_bytes());
        params.push(u8::from(enabled));
        self.event(id, 0x08, &params);
    }

    fn key_refresh_complete(&mut self, id: usize, handle: u16) {
        let mut params = vec![STATUS_OK];
        params.extend_from_slice(&handle.to_le_bytes());
        self.event(id, 0x30, &params);
    }

    fn link(&self, id: usize, handle: u16) -> Option<usize> {
        self.links
            .iter()
            .position(|l| l.handle == handle && (l.central == id || l.peripheral == id))
    }

    /// `STATUS_OK` when `id` has link `handle`, else unknown connection.
    fn link_status(&self, id: usize, handle: u16) -> u8 {
        if self.link(id, handle).is_some() {
            STATUS_OK
        } else {
            STATUS_UNKNOWN_CONN
        }
    }

    fn drop_link(&mut self, handle: u16, central_reason: u8, peripheral_reason: u8) {
        if let Some(i) = self.links.iter().position(|l| l.handle == handle) {
            let link = self.links.remove(i);
            self.disconnection_complete(link.central, handle, central_reason);
            self.disconnection_complete(link.peripheral, handle, peripheral_reason);
        }
    }

    // ---- commands ---------------------------------------------------------------

    fn command(&mut self, id: usize, bytes: &[u8]) {
        let op = u16_at(bytes, 0);
        let p = &bytes[3..];
        if trace_enabled() {
            eprintln!("[blesim] {} -> cmd {op:04x} {p:02x?}", self.nodes[id].name);
        }
        let known = self.controller_command(id, op, p)
            || self.lists_command(id, op, p)
            || self.advertising_command(id, op, p)
            || self.connection_command(id, op, p)
            || self.encryption_command(id, op, p);
        if !known {
            if trace_enabled() {
                eprintln!("[blesim] unknown command {op:04x}");
            }
            self.complete(id, op, STATUS_UNKNOWN_CMD, &[]);
        }
    }

    /// Controller setup and information: reset, addresses, versions, features, buffers, random numbers. Whether `op` is one of them.
    fn controller_command(&mut self, id: usize, op: u16, p: &[u8]) -> bool {
        match op {
            // Configuration the air has no use for: acknowledge.
            o if o == opcode::<SetEventMask>()
                || o == opcode::<SetEventMaskPage2>()
                || o == opcode::<LeSetEventMask>()
                || o == opcode::<HostBufferSize>()
                || o == opcode::<SetControllerToHostFlowControl>()
                || o == opcode::<HostNumberOfCompletedPackets>()
                || o == opcode::<LeSetAddrResolutionEnable>()
                || o == opcode::<LeSetResolvablePrivateAddrTimeout>()
                || o == opcode::<LeSetPrivacyMode>()
                || o == opcode::<LeRemoteConnectionParameterRequestReply>()
                || o == opcode::<LeRemoteConnectionParameterRequestNegativeReply>() =>
            {
                self.complete(id, op, STATUS_OK, &[]);
            }
            o if o == opcode::<Reset>() => {
                let handles: Vec<u16> = self
                    .links
                    .iter()
                    .filter(|l| l.central == id || l.peripheral == id)
                    .map(|l| l.handle)
                    .collect();
                for handle in handles {
                    let i = self.link(id, handle).unwrap();
                    let link = self.links.remove(i);
                    let other = if link.central == id {
                        link.peripheral
                    } else {
                        link.central
                    };
                    self.disconnection_complete(other, handle, STATUS_CONN_TIMEOUT);
                }
                self.nodes[id].reset();
                self.complete(id, op, STATUS_OK, &[]);
            }
            o if o == opcode::<ReadBdAddr>() => {
                let addr = self.nodes[id].public;
                self.complete(id, op, STATUS_OK, &addr);
            }
            o if o == opcode::<ReadLocalVersionInformation>() => {
                // Bluetooth 5.4, no manufacturer (0xffff).
                self.complete(id, op, STATUS_OK, &[0x0d, 0, 0, 0x0d, 0xff, 0xff, 0, 0]);
            }
            o if o == opcode::<ReadLocalSupportedCmds>() => {
                self.complete(id, op, STATUS_OK, &[0xff; 64]);
            }
            o if o == opcode::<ReadLocalSupportedFeatures>() => {
                // Bit 38: LE supported (controller); bit 37: BR/EDR not supported.
                self.complete(id, op, STATUS_OK, &[0, 0, 0, 0, 0x60, 0, 0, 0]);
            }
            o if o == opcode::<LeReadLocalSupportedFeatures>() => {
                // Encryption, connection parameters request, extended reject,
                // peripheral-initiated features exchange, ping, data length
                // extension, privacy.
                self.complete(id, op, STATUS_OK, &[0x7f, 0, 0, 0, 0, 0, 0, 0]);
            }
            o if o == opcode::<LeReadBufferSize>() => {
                // 251-byte ACL packets, 8 of them.
                self.complete(id, op, STATUS_OK, &[251, 0, 8]);
            }
            o if o == opcode::<LeReadFilterAcceptListSize>() => {
                self.complete(id, op, STATUS_OK, &[8]);
            }
            o if o == opcode::<LeRand>() => {
                let value = self.nodes[id].next_rand();
                self.complete(id, op, STATUS_OK, &value.to_le_bytes());
            }
            o if o == opcode::<LeSetRandomAddr>() => {
                self.nodes[id].random = Some(p[..6].try_into().unwrap());
                self.complete(id, op, STATUS_OK, &[]);
            }
            _ => return false,
        }
        true
    }

    /// The filter accept list and the resolving list. Whether `op` is one of them.
    fn lists_command(&mut self, id: usize, op: u16, p: &[u8]) -> bool {
        match op {
            o if o == opcode::<LeClearFilterAcceptList>() => {
                self.nodes[id].accept_list.clear();
                self.complete(id, op, STATUS_OK, &[]);
            }
            o if o == opcode::<LeAddDeviceToFilterAcceptList>() => {
                let addr = Addr::new(p[0], &p[1..7]);
                let list = &mut self.nodes[id].accept_list;
                if !list.contains(&addr) {
                    list.push(addr);
                }
                self.complete(id, op, STATUS_OK, &[]);
            }
            o if o == opcode::<LeRemoveDeviceFromFilterAcceptList>() => {
                let addr = Addr::new(p[0], &p[1..7]);
                self.nodes[id].accept_list.retain(|a| *a != addr);
                self.complete(id, op, STATUS_OK, &[]);
            }
            // Kept for inspection only: the air resolves no private addresses.
            o if o == opcode::<LeClearResolvingList>() => {
                self.nodes[id].resolving_list.clear();
                self.complete(id, op, STATUS_OK, &[]);
            }
            o if o == opcode::<LeAddDeviceToResolvingList>() => {
                let addr = Addr::new(p[0], &p[1..7]);
                self.nodes[id].resolving_list.push(addr);
                self.complete(id, op, STATUS_OK, &[]);
            }
            o if o == opcode::<LeRemoveDeviceFromResolvingList>() => {
                let addr = Addr::new(p[0], &p[1..7]);
                self.nodes[id].resolving_list.retain(|a| *a != addr);
                self.complete(id, op, STATUS_OK, &[]);
            }
            _ => return false,
        }
        true
    }

    /// Advertising and scanning parameters, data and switches. Whether `op` is one of them.
    fn advertising_command(&mut self, id: usize, op: u16, p: &[u8]) -> bool {
        match op {
            o if o == opcode::<LeSetAdvParams>() => {
                if self.nodes[id].adv_enabled {
                    self.complete(id, op, STATUS_DISALLOWED, &[]);
                    return true;
                }
                let node = &mut self.nodes[id];
                node.adv_type = p[4];
                node.adv_own_kind = p[5];
                node.adv_peer = Addr::new(p[6], &p[7..13]);
                node.adv_filter = p[14];
                self.complete(id, op, STATUS_OK, &[]);
            }
            o if o == opcode::<LeSetAdvData>() => {
                let len = (p[0] as usize).min(31);
                self.nodes[id].adv_data = p[1..=len].to_vec();
                self.complete(id, op, STATUS_OK, &[]);
            }
            o if o == opcode::<LeSetScanResponseData>() => {
                let len = (p[0] as usize).min(31);
                self.nodes[id].scan_data = p[1..=len].to_vec();
                self.complete(id, op, STATUS_OK, &[]);
            }
            o if o == opcode::<LeSetAdvEnable>() => {
                let enable = p[0] != 0;
                let node = &mut self.nodes[id];
                if enable && node.adv_own_kind & 1 == 1 && node.random.is_none() {
                    self.complete(id, op, STATUS_INVALID_PARAMS, &[]);
                    return true;
                }
                node.adv_enabled = enable;
                self.complete(id, op, STATUS_OK, &[]);
                if enable {
                    self.try_connect();
                    let scanners: Vec<usize> = (0..self.nodes.len())
                        .filter(|&n| n != id && self.nodes[n].scanning)
                        .collect();
                    for scanner in scanners {
                        self.advertise_to(scanner);
                    }
                }
            }
            o if o == opcode::<LeSetScanParams>() => {
                if self.nodes[id].scanning {
                    self.complete(id, op, STATUS_DISALLOWED, &[]);
                    return true;
                }
                let node = &mut self.nodes[id];
                node.scan_active = p[0] == 1;
                node.scan_own_kind = p[5];
                node.scan_filter = p[6];
                self.complete(id, op, STATUS_OK, &[]);
            }
            o if o == opcode::<LeSetScanEnable>() => {
                let node = &mut self.nodes[id];
                let enable = p[0] != 0;
                if enable && !node.scanning {
                    node.seen.clear();
                }
                node.scanning = enable;
                node.filter_duplicates = p[1] != 0;
                self.complete(id, op, STATUS_OK, &[]);
                if enable {
                    self.advertise_to(id);
                }
            }
            _ => return false,
        }
        true
    }

    /// Connecting, disconnecting and the parameters of a link. Whether `op` is one of them.
    fn connection_command(&mut self, id: usize, op: u16, p: &[u8]) -> bool {
        match op {
            o if o == opcode::<LeCreateConn>() => {
                // Like Nordic's SoftDevice Controller: one procedure at a
                // time, so no connecting while still scanning.
                if self.nodes[id].initiating.is_some() || self.nodes[id].scanning {
                    self.status(id, op, STATUS_DISALLOWED);
                    return true;
                }
                self.nodes[id].initiating = Some(Initiating {
                    use_accept_list: p[4] != 0,
                    peer: Addr::new(p[5], &p[6..12]),
                    own_kind: p[12],
                    interval: u16_at(p, 15),
                    latency: u16_at(p, 17),
                    timeout: u16_at(p, 19),
                });
                self.status(id, op, STATUS_OK);
                self.try_connect();
            }
            o if o == opcode::<LeCreateConnCancel>() => {
                if self.nodes[id].initiating.take().is_none() {
                    self.complete(id, op, STATUS_DISALLOWED, &[]);
                    return true;
                }
                self.complete(id, op, STATUS_OK, &[]);
                // LE Connection Complete: status, then 17 bytes of nothing.
                let mut params = vec![STATUS_UNKNOWN_CONN];
                params.extend_from_slice(&[0; 17]);
                self.le_event(id, 0x01, &params);
            }
            o if o == opcode::<Disconnect>() => {
                let handle = u16_at(p, 0);
                let reason = p[2];
                let Some(i) = self.link(id, handle) else {
                    self.status(id, op, STATUS_UNKNOWN_CONN);
                    return true;
                };
                self.status(id, op, STATUS_OK);
                let link = self.links.remove(i);
                let other = if link.central == id {
                    link.peripheral
                } else {
                    link.central
                };
                self.disconnection_complete(id, handle, STATUS_LOCAL_HOST);
                self.disconnection_complete(other, handle, reason);
            }
            o if o == opcode::<LeConnUpdate>() => {
                let handle = u16_at(p, 0);
                let Some(i) = self.link(id, handle) else {
                    self.status(id, op, STATUS_UNKNOWN_CONN);
                    return true;
                };
                self.status(id, op, STATUS_OK);
                let link = &self.links[i];
                let (central, peripheral) = (link.central, link.peripheral);
                let mut params = vec![STATUS_OK];
                params.extend_from_slice(&handle.to_le_bytes());
                params.extend_from_slice(&p[4..6]); // interval: the maximum asked for
                params.extend_from_slice(&p[6..8]); // latency
                params.extend_from_slice(&p[8..10]); // supervision timeout
                self.le_event(central, 0x03, &params);
                self.le_event(peripheral, 0x03, &params);
            }
            o if o == opcode::<LeSetDataLength>() => {
                let status = self.link_status(id, u16_at(p, 0));
                self.complete(id, op, status, &p[0..2]);
            }
            o if o == opcode::<LeReadPhy>() => {
                let status = self.link_status(id, u16_at(p, 0));
                self.complete(id, op, status, &[p[0], p[1], 1, 1]);
            }
            o if o == opcode::<LeSetPhy>() => {
                let handle = u16_at(p, 0);
                if self.link(id, handle).is_none() {
                    self.status(id, op, STATUS_UNKNOWN_CONN);
                    return true;
                }
                self.status(id, op, STATUS_OK);
                // The air has one PHY: 1M.
                let mut params = vec![STATUS_OK];
                params.extend_from_slice(&handle.to_le_bytes());
                params.extend_from_slice(&[1, 1]);
                self.le_event(id, 0x0c, &params);
            }
            o if o == opcode::<ReadRssi>() => {
                let status = self.link_status(id, u16_at(p, 0));
                self.complete(id, op, status, &[p[0], p[1], (-40i8) as u8]);
            }
            _ => return false,
        }
        true
    }

    /// Starting encryption, and the peripheral's answer with its key. Whether `op` is one of them.
    fn encryption_command(&mut self, id: usize, op: u16, p: &[u8]) -> bool {
        match op {
            o if o == opcode::<LeEnableEncryption>() => {
                let handle = u16_at(p, 0);
                let Some(i) = self.link(id, handle) else {
                    self.status(id, op, STATUS_UNKNOWN_CONN);
                    return true;
                };
                if self.links[i].central != id {
                    self.status(id, op, STATUS_DISALLOWED);
                    return true;
                }
                self.status(id, op, STATUS_OK);
                self.links[i].pending_ltk = Some(p[12..28].try_into().unwrap());
                let peripheral = self.links[i].peripheral;
                let mut params = handle.to_le_bytes().to_vec();
                params.extend_from_slice(&p[2..10]); // random number
                params.extend_from_slice(&p[10..12]); // encrypted diversifier
                self.le_event(peripheral, 0x05, &params);
            }
            o if o == opcode::<LeLongTermKeyRequestReply>() => {
                let handle = u16_at(p, 0);
                let ltk: [u8; 16] = p[2..18].try_into().unwrap();
                let pending = self
                    .link(id, handle)
                    .filter(|&i| self.links[i].peripheral == id)
                    .and_then(|i| self.links[i].pending_ltk.take().map(|key| (i, key)));
                let Some((i, central_ltk)) = pending else {
                    self.complete(id, op, STATUS_DISALLOWED, &p[0..2]);
                    return true;
                };
                self.complete(id, op, STATUS_OK, &p[0..2]);
                let link = &mut self.links[i];
                let (central, refresh) = (link.central, link.encrypted);
                if central_ltk != ltk {
                    // Both ends use their own key: the first encrypted packet
                    // fails its integrity check and the link drops.
                    self.drop_link(handle, STATUS_MIC_FAILURE, STATUS_MIC_FAILURE);
                    return true;
                }
                link.encrypted = true;
                if refresh {
                    self.key_refresh_complete(central, handle);
                    self.key_refresh_complete(id, handle);
                } else {
                    self.encryption_change(central, handle, STATUS_OK, true);
                    self.encryption_change(id, handle, STATUS_OK, true);
                }
                self.release_held(i);
            }
            o if o == opcode::<LeLongTermKeyRequestNegativeReply>() => {
                let handle = u16_at(p, 0);
                let pending = self
                    .link(id, handle)
                    .filter(|&i| self.links[i].peripheral == id)
                    .filter(|&i| self.links[i].pending_ltk.take().is_some());
                let Some(i) = pending else {
                    self.complete(id, op, STATUS_DISALLOWED, &p[0..2]);
                    return true;
                };
                self.complete(id, op, STATUS_OK, &p[0..2]);
                let central = self.links[i].central;
                let encrypted = self.links[i].encrypted;
                self.encryption_change(central, handle, STATUS_KEY_MISSING, encrypted);
                self.release_held(i);
            }
            _ => return false,
        }
        true
    }

    // ---- advertising, scanning, connecting --------------------------------------

    /// One round of advertising packets from every advertiser to `scanner`.
    fn advertise_to(&mut self, scanner: usize) {
        if !self.nodes[scanner].scanning {
            return;
        }
        for adv in 0..self.nodes.len() {
            let a = &self.nodes[adv];
            if adv == scanner || !a.adv_enabled || a.directed() {
                continue;
            }
            let from = a.own_address(a.adv_own_kind);
            let (adv_type, adv_filter) = (a.adv_type, a.adv_filter);
            let (adv_data, scan_data) = (a.adv_data.clone(), a.scan_data.clone());
            let s = &mut self.nodes[scanner];
            if s.scan_filter & 1 == 1 && !s.accept_list.contains(&from) {
                continue;
            }
            if s.filter_duplicates {
                if s.seen.contains(&from) {
                    continue;
                }
                s.seen.push(from);
            }
            let scanner_addr = s.own_address(s.scan_own_kind);
            let scan_request = s.scan_active
                && matches!(adv_type, ADV_IND | ADV_SCAN_IND)
                && (adv_filter & 1 == 0 || self.nodes[adv].accept_list.contains(&scanner_addr));
            self.adv_report(scanner, adv_type, from, &adv_data);
            if scan_request {
                self.adv_report(scanner, SCAN_RSP, from, &scan_data);
            }
        }
    }

    fn adv_report(&mut self, scanner: usize, event_type: u8, from: Addr, data: &[u8]) {
        let mut params = vec![1, event_type, from.kind];
        params.extend_from_slice(&from.addr);
        params.push(data.len() as u8);
        params.extend_from_slice(data);
        params.push((-50i8) as u8);
        self.le_event(scanner, 0x02, &params);
    }

    /// Connect every initiator that has a matching connectable advertiser.
    fn try_connect(&mut self) {
        for central in 0..self.nodes.len() {
            let Some(init) = self.nodes[central].initiating else {
                continue;
            };
            let central_addr = self.nodes[central].own_address(init.own_kind);
            let found = (0..self.nodes.len()).find(|&peripheral| {
                let p = &self.nodes[peripheral];
                if peripheral == central || !p.connectable() {
                    return false;
                }
                let addr = p.own_address(p.adv_own_kind);
                let wanted = if init.use_accept_list {
                    self.nodes[central].accept_list.contains(&addr)
                } else {
                    init.peer == addr
                };
                let allowed = if p.directed() {
                    p.adv_peer == central_addr
                } else {
                    p.adv_filter & 2 == 0 || p.accept_list.contains(&central_addr)
                };
                wanted && allowed
            });
            if let Some(peripheral) = found {
                self.connect(central, peripheral, init, central_addr);
            }
        }
    }

    fn connect(&mut self, central: usize, peripheral: usize, init: Initiating, central_addr: Addr) {
        self.next_handle = self.next_handle % 0x0eff + 1;
        let handle = self.next_handle;
        let p = &mut self.nodes[peripheral];
        let peripheral_addr = p.own_address(p.adv_own_kind);
        p.adv_enabled = false;
        self.nodes[central].initiating = None;
        self.links.push(Link {
            handle,
            central,
            peripheral,
            encrypted: false,
            pending_ltk: None,
            held: Vec::new(),
        });
        for (id, role, peer) in [(central, 0, peripheral_addr), (peripheral, 1, central_addr)] {
            let mut params = vec![STATUS_OK];
            params.extend_from_slice(&handle.to_le_bytes());
            params.push(role);
            params.push(peer.kind);
            params.extend_from_slice(&peer.addr);
            params.extend_from_slice(&init.interval.to_le_bytes());
            params.extend_from_slice(&init.latency.to_le_bytes());
            params.extend_from_slice(&init.timeout.to_le_bytes());
            params.push(0); // central clock accuracy
            self.le_event(id, 0x01, &params);
        }
    }

    // ---- data -------------------------------------------------------------------

    /// The encryption start on link `i` is over: send the data held during it.
    fn release_held(&mut self, i: usize) {
        let held = std::mem::take(&mut self.links[i].held);
        let handle = self.links[i].handle;
        for (to, packet, from) in held {
            self.push(to, packet);
            self.completed_packet(from, handle);
        }
    }

    /// Number Of Completed Packets: one packet of `handle` left `id`'s buffer.
    fn completed_packet(&mut self, id: usize, handle: u16) {
        let mut params = vec![1];
        params.extend_from_slice(&handle.to_le_bytes());
        params.extend_from_slice(&1u16.to_le_bytes());
        self.event(id, 0x13, &params);
    }

    fn acl(&mut self, id: usize, bytes: &[u8]) {
        let header = u16_at(bytes, 0);
        let handle = header & 0x0fff;
        if trace_enabled() {
            eprintln!("[blesim] {} -> acl {:02x?}", self.nodes[id].name, bytes);
        }
        if let Some(i) = self.link(id, handle) {
            let link = &self.links[i];
            let other = if link.central == id {
                link.peripheral
            } else {
                link.central
            };
            // The receiver sees a start fragment as "first automatically flushable".
            let boundary = match (header >> 12) & 0b11 {
                0b00 => 0b10,
                b => b,
            };
            // A first fragment on the SMP channel (CID 6) with code 1.
            let smp_pairing_request = boundary != 0b01
                && bytes.len() >= 9
                && u16_at(bytes, 6) == 0x0006
                && bytes[8] == 0x01;
            if smp_pairing_request {
                self.pairing_requests += 1;
            }
            let mut forwarded = bytes.to_vec();
            forwarded[..2].copy_from_slice(&(handle | boundary << 12).to_le_bytes());
            if self.links[i].pending_ltk.is_some() {
                // The encryption is starting: no data until it's done.
                self.links[i]
                    .held
                    .push((other, (PacketKind::AclData, forwarded), id));
                return;
            }
            self.push(other, (PacketKind::AclData, forwarded));
        }
        // The packet left the controller's buffer (sent, or dropped with the link).
        self.completed_packet(id, handle);
    }
}
