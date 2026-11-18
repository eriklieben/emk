//! The left half's Bluetooth: one host stack with two links at once, the
//! right half (we are its central) and the computer (we are its peripheral).

use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU16};

use embassy_futures::join::join5;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};
use embedded_storage_async::nor_flash::NorFlash;
use sequential_storage::cache::NoCache;
use sequential_storage::map::{MapConfig, MapStorage};
use trouble_host::prelude::*;

use crate::Params;
use crate::hid::{self, HidState};
use crate::keymap::{self, KeymapSlot};
use crate::log::LogRing;
use crate::notify::Inbox;
use crate::radio::{self, Radio};
use crate::split::{self, SplitLink};

/// Everything the left half's firmware shares with its Bluetooth.
///
/// The log ring is borrowed, not owned: this struct has a few non-zero
/// initial values, so as a static it gets a copy in flash for startup; the
/// 4 KB ring in its own (all-zero) static doesn't.
pub struct LeftState<'a> {
    pub split: SplitLink,
    /// Key changes from the right half, as (position, pressed).
    pub remote_events: Channel<CriticalSectionRawMutex, (u8, bool), 32>,
    /// The right half's battery level as it last reported it; 255 = unknown.
    pub right_battery: AtomicU8,
    /// The link to the computer.
    pub hid: HidState,
    /// The radio's transmit power (`RadioPower` key).
    pub radio: Radio,
    /// Key presses on this half so far (wrapping), sent to the right half
    /// for its screen ([`LeftState::set_key_presses`]).
    pub(crate) presses: AtomicU16,
    pub(crate) presses_changed: Signal<CriticalSectionRawMutex, ()>,
    /// Whether the keyboard rests (screens off): the split link slows down
    /// ([`LeftState::set_resting`]).
    pub(crate) resting: AtomicBool,
    pub(crate) resting_changed: Signal<CriticalSectionRawMutex, ()>,
    /// The left half's log (with the right half's lines, `R` in front).
    pub log: &'a LogRing,
    /// Studio's edited keymap, kept in flash once attached.
    pub keymap: KeymapSlot<'a>,
    /// Messages from the computer (USB or Bluetooth Raw HID), for this
    /// half's status and the right half's screen.
    pub inbox: Inbox,
}

impl<'a> LeftState<'a> {
    pub const fn new(log: &'a LogRing) -> Self {
        Self {
            split: SplitLink::new(),
            remote_events: Channel::new(),
            right_battery: AtomicU8::new(255),
            hid: HidState::new(),
            radio: Radio::new(),
            presses: AtomicU16::new(0),
            presses_changed: Signal::new(),
            resting: AtomicBool::new(false),
            resting_changed: Signal::new(),
            log,
            keymap: KeymapSlot::new(),
            inbox: Inbox::new(),
        }
    }

    /// Report whether the keyboard rests (left half's key loop, from its
    /// power state). Resting, the split link runs at a slower interval to
    /// save power; the next key brings it back to full speed.
    pub fn set_resting(&self, resting: bool) {
        use core::sync::atomic::Ordering;
        if self.resting.swap(resting, Ordering::Relaxed) != resting {
            self.resting_changed.signal(());
        }
    }

    /// Report the key presses so far (left half's key loop); the right half
    /// gets them for its digital rain.
    pub fn set_key_presses(&self, n: u16) {
        self.presses.store(n, core::sync::atomic::Ordering::Relaxed);
        self.presses_changed.signal(());
    }
}

/// Two links (right half + computer), two L2CAP channels each plus spare,
/// one advertising set, and bonds for the right half and 10 computers.
type Resources = HostResources<DefaultPacketPool, 2, 6, 1, 12>;

/// Run the left half's links forever; `server` is from [`hid::hid_server`].
pub async fn run<C, S>(
    state: &LeftState<'_>,
    controller: C,
    flash: &mut S,
    server: &hid::HidServer<'_>,
    params: Params,
) -> !
where
    C: Controller
        + bt_hci::controller::ControllerCmdSync<bt_hci::cmd::le::LeSetScanParams>
        + bt_hci::controller::ControllerCmdAsync<bt_hci::cmd::le::LeConnUpdate>
        + bt_hci::controller::ControllerCmdSync<bt_hci::cmd::le::LeReadLocalSupportedFeatures>,
    S: NorFlash,
{
    let Params {
        address,
        storage: range,
        name,
        timing,
        set_tx_power,
    } = params;
    let mut resources: Resources = HostResources::new();
    let stack = trouble_host::new(controller, &mut resources)
        .set_random_address(address)
        // A keyboard: a computer with a screen pairs with passkey entry (LE
        // Secure Connections, MITM protected): it shows a 6-digit code, the
        // user types it on this keyboard (the left screen prompts for it).
        // One without a screen falls back to "Just Works". The right half
        // declares no input or output, so the split link between the halves
        // stays "Just Works" (no prompt).
        .set_io_capabilities(IoCapabilities::KeyboardOnly)
        .build();
    let store: split::Store<_> = Mutex::new(MapStorage::new(
        flash,
        MapConfig::new(range),
        NoCache::new(),
    ));
    let finder = split::Finder::new();
    let mut runner = stack.runner();
    let mut central = stack.central();
    let mut peripheral = stack.peripheral();

    join5(
        async {
            loop {
                if let Err(e) = runner.run_with_handler(&finder).await {
                    ring_log!(state.log, "ble: host stopped: {:?}", e);
                    Timer::after(Duration::from_millis(100)).await;
                }
            }
        },
        split::run_central(state, &stack, &mut central, &finder, &store, &timing),
        hid::run(
            state,
            &stack,
            &mut peripheral,
            &store,
            server,
            name,
            &timing,
        ),
        radio::run(&state.radio, set_tx_power, &store, state.log),
        keymap::run(&state.keymap, &store, state.log),
    )
    .await;
    unreachable!()
}
