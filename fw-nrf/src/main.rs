//! Corne firmware for the nice!nano v2 (nRF52840).
//!
//! ```text
//! left half, key loop (1 kHz while keys are active, asleep otherwise)
//!   matrix scan → debounce ─┐
//!   right half's keys ──────┴→ Engine → boot keyboard report ──► hid_task ──► USB
//!                                   └──→ Mouse (mouse keys) → mouse report ──► mouse_task ──► USB
//!   Power (idle / deep sleep) and Status ──► display_task ──► SSD1306 over I²C
//!   USB Raw HID (and Bluetooth report 3) ──► Reassembler ──► Inbox ──► Status.notification
//!                                                             └──► split link ──► right half
//! right half, with `ble`: matrix scan → debounce → held-key bitmap ──► split link ──► left half
//!   messages from the left half ──► Status.notification ──► display_task
//! ```
//!
//! Features: `left` / `right` pick the half; `usb-only` is the plain USB build
//! (the right half is then a standalone USB keyboard for testing); `ble` links
//! the halves over Bluetooth (see `ble/`). Pins come from ZMK's `corne` shield
//! and the nice!nano's Pro Micro mapping.
#![no_std]
#![no_main]
// The right half over Bluetooth has no USB keyboard or engine, so part of
// what the left half imports goes unused there.
#![cfg_attr(all(feature = "ble", feature = "right"), allow(unused_imports))]

#[macro_use]
mod log;
#[cfg(feature = "ble")]
mod ble;
mod diag;
#[cfg(all(feature = "usb-log", not(all(feature = "ble", feature = "right"))))]
mod logport;
mod matrix;
mod oled;
mod sleep;

use core::panic::PanicInfo;

use embassy_executor::Spawner;
use embassy_futures::select::select;
use embassy_nrf::config::{Config as NrfConfig, HfclkSource, LfclkSource};
use embassy_nrf::gpio::{Input, Level, Output, OutputDrive, Pull};
use embassy_nrf::usb::Driver;
#[cfg(not(feature = "ble"))]
use embassy_nrf::usb::vbus_detect::HardwareVbusDetect;
#[cfg(feature = "ble")]
use embassy_nrf::usb::vbus_detect::SoftwareVbusDetect;
use embassy_nrf::{Peri, bind_interrupts, peripherals, saadc, twim, usb};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Ticker, Timer};
use embassy_usb::class::hid::{self, HidReader, HidReaderWriter, HidWriter};
use embassy_usb::{Builder, UsbDevice};
use kb_core::battery;
use kb_core::debounce::Debouncer;
use kb_core::host::{self as host_msg, Shown};
use kb_core::keymaps::corne;
use kb_core::mouse::{Mouse, MouseReport, REPORT_LEN as MOUSE_REPORT_LEN};
use kb_core::power::{Power, PowerConfig, PowerState};
use kb_core::{Config, Engine, Output as EngineOutput, Report};
use kb_display::status::{Status, Transport, Wpm};
use static_cell::StaticCell;
use usbd_hid::descriptor::{KeyboardReport, SerializedDescriptor};

#[cfg(not(feature = "ble"))]
bind_interrupts!(struct Irqs {
    USBD => usb::InterruptHandler<peripherals::USBD>;
    CLOCK_POWER => usb::vbus_detect::InterruptHandler;
    TWISPI0 => twim::InterruptHandler<peripherals::TWISPI0>;
    SAADC => saadc::InterruptHandler;
});

// With Bluetooth, MPSL owns the radio, its timers and CLOCK_POWER; USB power
// is then polled (`vbus_task`) instead of taken from the CLOCK_POWER interrupt.
#[cfg(feature = "ble")]
bind_interrupts!(struct Irqs {
    USBD => usb::InterruptHandler<peripherals::USBD>;
    TWISPI0 => twim::InterruptHandler<peripherals::TWISPI0>;
    SAADC => saadc::InterruptHandler;
    RNG => embassy_nrf::rng::InterruptHandler<peripherals::RNG>;
    EGU0_SWI0 => nrf_sdc::mpsl::LowPrioInterruptHandler;
    CLOCK_POWER => nrf_sdc::mpsl::ClockInterruptHandler;
    RADIO => nrf_sdc::mpsl::HighPrioInterruptHandler;
    TIMER0 => nrf_sdc::mpsl::HighPrioInterruptHandler;
    RTC0 => nrf_sdc::mpsl::HighPrioInterruptHandler;
});

/// How the USB driver learns about USB power in this build.
#[cfg(not(feature = "ble"))]
type Vbus = HardwareVbusDetect;
#[cfg(all(feature = "ble", not(feature = "right")))]
type Vbus = &'static SoftwareVbusDetect;
#[cfg(not(all(feature = "ble", feature = "right")))]
type UsbDriver = Driver<'static, Vbus>;

#[cfg(feature = "ble")]
type BleFlash = nrf_sdc::mpsl::Flash<'static>;

/// Shown to the computer (USB product name and Bluetooth name). Set per
/// keyboard at build time: `KB_NAME=erik_kbd ./fw-nrf/build-uf2`.
const KEYBOARD_NAME: &str = match option_env!("KB_NAME") {
    Some(name) => name,
    None => "erik_kbd",
};

#[cfg(not(all(feature = "ble", feature = "right")))]
/// Boot-protocol keyboard reports: `[modifiers, 0, key × 6]`.
static REPORTS: Channel<CriticalSectionRawMutex, [u8; 8], 16> = Channel::new();

/// Studio: the keymap the engine types with, edited live over the log port
/// and (with Bluetooth) kept in flash by the left half's Bluetooth task.
/// Without Bluetooth there is no storage: edits last until a restart.
#[cfg(not(all(feature = "ble", feature = "right")))]
static STUDIO: kb_ble::keymap::SharedStudio<{ corne::KEYS }, { corne::LAYER_COUNT }> =
    kb_ble::keymap::SharedStudio::new(&corne::KEYMAP);

#[cfg(not(all(feature = "ble", feature = "right")))]
/// Mouse reports (`kb_core::mouse::MouseReport::bytes`) for the USB mouse.
/// Relative motion: the key loop only takes a report from the mouse when
/// there is room here, so nothing is dropped.
static MOUSE_REPORTS: Channel<CriticalSectionRawMutex, [u8; MOUSE_REPORT_LEN], 4> = Channel::new();

/// The USB mouse's feature report as the computer set it: the Resolution
/// Multipliers (smooth scrolling). 0 again after a USB reset.
#[cfg(not(all(feature = "ble", feature = "right")))]
static USB_MOUSE_FEATURE: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Answers the USB mouse's feature report requests (GET / `SET_REPORT`).
#[cfg(not(all(feature = "ble", feature = "right")))]
struct MouseRequests;

#[cfg(not(all(feature = "ble", feature = "right")))]
impl hid::RequestHandler for MouseRequests {
    fn get_report(&mut self, id: hid::ReportId, buf: &mut [u8]) -> Option<usize> {
        match (id, buf.first_mut()) {
            (hid::ReportId::Feature(0), Some(b)) => {
                *b = USB_MOUSE_FEATURE.load(core::sync::atomic::Ordering::Relaxed);
                Some(1)
            }
            _ => None,
        }
    }

    fn set_report(&mut self, id: hid::ReportId, data: &[u8]) -> embassy_usb::control::OutResponse {
        match (id, data.first()) {
            (hid::ReportId::Feature(0), Some(&b)) => {
                USB_MOUSE_FEATURE.store(b, core::sync::atomic::Ordering::Relaxed);
                embassy_usb::control::OutResponse::Accepted
            }
            _ => embassy_usb::control::OutResponse::Rejected,
        }
    }
}

/// Raw HID output reports from the computer (`kb_core::host`), from the
/// interrupt OUT endpoint and from `SET_REPORT` requests alike.
#[cfg(not(all(feature = "ble", feature = "right")))]
static RAW_HID_REPORTS: Channel<CriticalSectionRawMutex, [u8; host_msg::REPORT_SIZE], 8> =
    Channel::new();

/// Takes the USB Raw HID interface's output reports (either pipe).
#[cfg(not(all(feature = "ble", feature = "right")))]
struct RawHidRequests;

#[cfg(not(all(feature = "ble", feature = "right")))]
impl hid::RequestHandler for RawHidRequests {
    fn set_report(&mut self, id: hid::ReportId, data: &[u8]) -> embassy_usb::control::OutResponse {
        match id {
            hid::ReportId::Out(0) => {
                let mut report = [0u8; host_msg::REPORT_SIZE];
                let n = data.len().min(report.len());
                report[..n].copy_from_slice(&data[..n]);
                // Full (nobody reading): the message is dropped, not the USB stack.
                let _ = RAW_HID_REPORTS.try_send(report);
                embassy_usb::control::OutResponse::Accepted
            }
            _ => embassy_usb::control::OutResponse::Rejected,
        }
    }
}

/// Messages from the computer without Bluetooth: only this half's screen.
#[cfg(not(feature = "ble"))]
static INBOX: kb_ble::notify::Inbox = kb_ble::notify::Inbox::new();

/// Where messages from the computer go: this half's status, and with
/// Bluetooth the right half too.
#[cfg(not(all(feature = "ble", feature = "right")))]
fn inbox() -> &'static kb_ble::notify::Inbox {
    #[cfg(feature = "ble")]
    return &ble::LEFT.inbox;
    #[cfg(not(feature = "ble"))]
    return &INBOX;
}

/// Whether a computer has set up the USB keyboard (not just power on the cable).
#[cfg(not(all(feature = "ble", feature = "right")))]
static USB_CONFIGURED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

#[cfg(not(all(feature = "ble", feature = "right")))]
struct UsbState;

#[cfg(not(all(feature = "ble", feature = "right")))]
impl embassy_usb::Handler for UsbState {
    fn reset(&mut self) {
        USB_CONFIGURED.store(false, core::sync::atomic::Ordering::Relaxed);
        USB_MOUSE_FEATURE.store(0, core::sync::atomic::Ordering::Relaxed);
    }
    fn configured(&mut self, configured: bool) {
        USB_CONFIGURED.store(configured, core::sync::atomic::Ordering::Relaxed);
    }
    fn enabled(&mut self, enabled: bool) {
        if !enabled {
            USB_CONFIGURED.store(false, core::sync::atomic::Ordering::Relaxed);
        }
    }
}

/// What the display should show.
#[derive(Clone, Copy, PartialEq)]
struct Screen {
    status: Status,
    power: PowerState,
}

static SCREEN: Signal<CriticalSectionRawMutex, Screen> = Signal::new();

/// The last screen sent, for the log's status command.
static SNAPSHOT: embassy_sync::blocking_mutex::Mutex<
    CriticalSectionRawMutex,
    core::cell::Cell<Option<Screen>>,
> = embassy_sync::blocking_mutex::Mutex::new(core::cell::Cell::new(None));

/// Send a screen to the display task (and keep it for the log's status).
fn publish(screen: Screen) {
    SNAPSHOT.lock(|c| c.set(Some(screen)));
    SCREEN.signal(screen);
}

/// Battery percentage of this half, measured every minute by `battery_task`.
static BATTERY: Signal<CriticalSectionRawMutex, u8> = Signal::new();

/// How often the battery is measured (ZMK's default report interval).
const BATTERY_INTERVAL: Duration = Duration::from_secs(60);

/// How often the matrix is scanned while keys are active (1 kHz).
const SCAN_INTERVAL: Duration = Duration::from_millis(1);

/// Debounce on both halves: an eager press (reported on the first closed
/// reading) and 5 ms for a release, so a bounce after the press can't
/// register as a release and a second press.
const PRESS_DEBOUNCE_MS: u32 = 0;
const RELEASE_DEBOUNCE_MS: u32 = 5;

/// The right half wakes at least this often (milliseconds) so its link icon
/// follows the link.
#[cfg(all(feature = "ble", feature = "right"))]
const LINK_ICON_REFRESH_MS: u32 = 1_000;

/// USB vendor and product ID: pid.codes' test PID.
#[cfg(not(all(feature = "ble", feature = "right")))]
const USB_VID: u16 = 0x1209;
#[cfg(not(all(feature = "ble", feature = "right")))]
const USB_PID: u16 = 0x0001;

/// Start a task. Every task here is started once, at boot, and its pool has
/// room for exactly that one, so its token is always there: a missing one
/// is a bug in this file, caught at the first start.
pub(crate) fn start<S>(
    spawner: Spawner,
    task: Result<embassy_executor::SpawnToken<S>, embassy_executor::SpawnError>,
) {
    spawner.spawn(task.expect("each task is started once"));
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    diag::record_and_reset(info)
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    #[cfg(feature = "ble")]
    static FLASH: StaticCell<BleFlash> = StaticCell::new();
    // The Adafruit bootloader leaves its version in TIMER2.CC[0] (0x00MMmmpp)
    // right before it starts us; nothing has used TIMER2 since.
    let bootloader = embassy_nrf::pac::TIMER2.cc(0).read();
    let p = embassy_nrf::init(nrf_config());
    #[cfg(feature = "ble")]
    {
        use embassy_nrf::interrupt::{InterruptExt, Priority};
        embassy_nrf::interrupt::USBD.set_priority(Priority::P2);
        embassy_nrf::interrupt::TWISPI0.set_priority(Priority::P2);
        embassy_nrf::interrupt::SAADC.set_priority(Priority::P2);
    }

    // Instruction cache: flash wait states otherwise cost on every fetch.
    embassy_nrf::pac::NVMC
        .icachecnf()
        .write(|w| w.set_cacheen(true));

    // VCC to the OLED (nice!nano EXT_POWER, P0.13).
    let ext_power = Output::new(p.P0_13, Level::High, OutputDrive::Standard);
    start_display(spawner, p.TWISPI0, p.P0_17, p.P0_20);
    log_boot(bootloader);
    // Crashed last time? Show where, and stop here so it can be read.
    if let Some(crash) = diag::take_crash() {
        dlog!("previous run crashed: {}", crash.as_str());
        diag::MESSAGE.signal(("PANIC", crash));
        loop {
            Timer::after(Duration::from_secs(3600)).await;
        }
    }
    start_battery(spawner, p.SAADC);

    // Matrix: columns driven high one at a time, rows read with pull-downs (diodes col → row).
    // Same pins on both halves (Pro Micro D21 D20 D19 D18 D15 D14), in ZMK's
    // col-gpios order: the left half lists them outer → inner, the right half
    // (PCB flipped) inner → outer, i.e. reversed.
    #[cfg(not(feature = "right"))]
    let cols = [
        Output::new(p.P0_31, Level::Low, OutputDrive::Standard),
        Output::new(p.P0_29, Level::Low, OutputDrive::Standard),
        Output::new(p.P0_02, Level::Low, OutputDrive::Standard),
        Output::new(p.P1_15, Level::Low, OutputDrive::Standard),
        Output::new(p.P1_13, Level::Low, OutputDrive::Standard),
        Output::new(p.P1_11, Level::Low, OutputDrive::Standard),
    ];
    #[cfg(feature = "right")]
    let cols = [
        Output::new(p.P1_11, Level::Low, OutputDrive::Standard),
        Output::new(p.P1_13, Level::Low, OutputDrive::Standard),
        Output::new(p.P1_15, Level::Low, OutputDrive::Standard),
        Output::new(p.P0_02, Level::Low, OutputDrive::Standard),
        Output::new(p.P0_29, Level::Low, OutputDrive::Standard),
        Output::new(p.P0_31, Level::Low, OutputDrive::Standard),
    ];
    let rows = [
        Input::new(p.P0_22, Pull::Down),
        Input::new(p.P0_24, Pull::Down),
        Input::new(p.P1_00, Pull::Down),
        Input::new(p.P0_11, Pull::Down),
    ];

    // Outermost top key held while starting: forget the bond with the other half.
    #[cfg(feature = "ble")]
    let cols = {
        let mut cols = cols;
        let held = matrix::scan(&mut cols, &rows);
        let key = if cfg!(feature = "right") { 11 } else { 0 };
        if held[key] {
            ble::link()
                .forget_bond
                .store(true, core::sync::atomic::Ordering::Relaxed);
            diag::MESSAGE.signal(("PAIR", diag::Text::new("forgetting other half")));
            Timer::after(Duration::from_secs(2)).await;
        }
        cols
    };
    #[cfg(feature = "ble")]
    diag::stage("radio").await;
    #[cfg(feature = "ble")]
    let (radio, flash) = {
        use ble::radio::{self, Role};
        let role = if cfg!(feature = "right") {
            Role::Peripheral
        } else {
            Role::Central
        };
        let sdc_p = nrf_sdc::Peripherals::new(
            p.PPI_CH17, p.PPI_CH18, p.PPI_CH20, p.PPI_CH21, p.PPI_CH22, p.PPI_CH23, p.PPI_CH24,
            p.PPI_CH25, p.PPI_CH26, p.PPI_CH27, p.PPI_CH28, p.PPI_CH29,
        );
        let radio = radio::start(
            spawner,
            role,
            Irqs,
            p.RTC0,
            p.TIMER0,
            p.TEMP,
            p.RNG,
            (p.PPI_CH19, p.PPI_CH30, p.PPI_CH31),
            sdc_p,
        );
        let flash = FLASH.init(nrf_sdc::mpsl::Flash::take(radio.mpsl, p.NVMC));
        (radio, flash)
    };

    // Right half over Bluetooth: no USB keyboard here, the left half types.
    #[cfg(all(feature = "ble", feature = "right"))]
    {
        diag::stage("split").await;
        start(spawner, split_peripheral_task(radio.sdc, flash));
        run_split_right(cols, rows, ext_power).await;
    }

    #[cfg(not(all(feature = "ble", feature = "right")))]
    {
        #[cfg(feature = "ble")]
        let _hfclk = {
            diag::stage("split").await;
            start(spawner, split_central_task(radio.sdc, flash));
            // USB needs the 32 MHz crystal, which MPSL owns: keep it requested.
            diag::stage("hfclk").await;
            radio
                .mpsl
                .request_hfclk()
                .await
                .unwrap_or_else(|_| panic!("hfclk"))
        };
        #[cfg(not(feature = "ble"))]
        let vbus = HardwareVbusDetect::new(Irqs);
        #[cfg(feature = "ble")]
        let vbus = {
            static VBUS: StaticCell<SoftwareVbusDetect> = StaticCell::new();
            let vbus: &'static SoftwareVbusDetect =
                VBUS.init(SoftwareVbusDetect::new(false, false));
            start(spawner, vbus_task(vbus));
            vbus
        };
        diag::stage("usb").await;
        // The UF2 bootloader was a USB drive a moment ago and can hand over with
        // USB still set up; the first start after flashing then hung until the
        // next reset. Start from a clean USB peripheral and no USB power events.
        #[cfg(feature = "ble")]
        {
            let usbd = embassy_nrf::pac::USBD;
            usbd.usbpullup().write(|w| w.set_connect(false));
            usbd.enable().write(|w| w.set_enable(false));
            embassy_nrf::pac::POWER.intenclr().write(|w| {
                w.set_usbdetected(true);
                w.set_usbremoved(true);
                w.set_usbpwrrdy(true);
            });
        }
        start_usb(spawner, p.USBD, vbus);
        run_keyboard(cols, rows, ext_power).await;
    }
}

/// The chip's clocks and interrupt levels for this build.
fn nrf_config() -> NrfConfig {
    let mut config = NrfConfig::default();
    // LOCK=1 builds: UICR.APPROTECT = Enabled (written once, then a reset).
    // Only a full chip erase over SWD opens it again, and that erases the
    // bonds, the bootloader and the SoftDevice too (docs/signed-bootloader.md).
    // Without it Embassy keeps the port open (Debug::Allowed); that can't undo
    // an earlier lock, since UICR bits only go back to 1 with an erase.
    #[cfg(feature = "debug-lock")]
    {
        config.debug = embassy_nrf::config::Debug::Disallowed;
    }
    config.lfclk_source = LfclkSource::InternalRC;
    #[cfg(not(feature = "ble"))]
    {
        config.hfclk_source = HfclkSource::ExternalXtal; // USB needs the crystal
    }
    #[cfg(feature = "ble")]
    {
        // MPSL manages the clocks (USB gets the crystal through it, below), and
        // our interrupts stay below the radio's time-critical ones.
        config.hfclk_source = HfclkSource::Internal;
        config.gpiote_interrupt_priority = embassy_nrf::interrupt::Priority::P2;
        config.time_interrupt_priority = embassy_nrf::interrupt::Priority::P2;
    }
    config
}

/// The OLED over I²C (Pro Micro D2 = SDA P0.17, D3 = SCL P0.20).
fn start_display(
    spawner: Spawner,
    twi: Peri<'static, peripherals::TWISPI0>,
    sda: Peri<'static, peripherals::P0_17>,
    scl: Peri<'static, peripherals::P0_20>,
) {
    static TWIM_RAM: StaticCell<[u8; 16]> = StaticCell::new();
    let mut twim_config = twim::Config::default();
    twim_config.frequency = twim::Frequency::K400;
    let i2c = twim::Twim::new(twi, Irqs, sda, scl, twim_config, TWIM_RAM.init([0; 16]));
    start(spawner, oled::display_task(i2c));
}

/// The boot lines in the log: build, reset reason, bootloader version.
fn log_boot(bootloader: u32) {
    dlog!(
        "boot: {} {} {} (reset: {})",
        KEYBOARD_NAME,
        if cfg!(feature = "right") {
            "right"
        } else {
            "left"
        },
        if cfg!(feature = "ble") {
            "ble"
        } else {
            "usb-only"
        },
        reset_reason()
    );
    dlog!(
        "boot: bootloader {}.{}.{}, entry {}, debug port {}",
        bootloader >> 16,
        (bootloader >> 8) & 0xFF,
        bootloader & 0xFF,
        if cfg!(feature = "signed-boot") {
            "serial DFU (signed)"
        } else {
            "UF2"
        },
        if cfg!(feature = "debug-lock") {
            "locked"
        } else {
            "open"
        }
    );
}

/// The battery: VDDH/5 on the SAADC, set up like ZMK's battery-nrf-vddh
/// driver, measured by `battery_task`.
fn start_battery(spawner: Spawner, saadc_p: Peri<'static, peripherals::SAADC>) {
    let mut adc_config = saadc::Config::default();
    adc_config.resolution = saadc::Resolution::_12bit;
    adc_config.oversample = saadc::Oversample::Over4x;
    let mut channel = saadc::ChannelConfig::single_ended(saadc::VddhDiv5Input);
    channel.gain = saadc::Gain::Gain1_2;
    channel.reference = saadc::Reference::Internal;
    channel.time = saadc::Time::_40US;
    let adc = saadc::Saadc::new(saadc_p, Irqs, adc_config, [channel]);
    start(spawner, battery_task(adc));
}

#[cfg(not(all(feature = "ble", feature = "right")))]
/// The USB keyboard: device stack and HID writer tasks.
fn start_usb(spawner: Spawner, usbd: Peri<'static, peripherals::USBD>, vbus: Vbus) {
    static CONFIG_DESC: StaticCell<[u8; 256]> = StaticCell::new();
    static BOS_DESC: StaticCell<[u8; 256]> = StaticCell::new();
    static MSOS_DESC: StaticCell<[u8; 256]> = StaticCell::new();
    static CONTROL_BUF: StaticCell<[u8; 64]> = StaticCell::new();
    static HID_STATE: StaticCell<hid::State> = StaticCell::new();
    static MOUSE_STATE: StaticCell<hid::State> = StaticCell::new();
    static MOUSE_REQUESTS: StaticCell<MouseRequests> = StaticCell::new();
    static RAW_HID_STATE: StaticCell<hid::State> = StaticCell::new();
    static RAW_HID_REQUESTS: StaticCell<RawHidRequests> = StaticCell::new();
    static USB_STATE: StaticCell<UsbState> = StaticCell::new();
    let driver = Driver::new(usbd, Irqs, vbus);
    let mut usb_config = embassy_usb::Config::new(USB_VID, USB_PID);
    usb_config.manufacturer = Some("erik");
    usb_config.product = Some(KEYBOARD_NAME);
    usb_config.max_power = 100;
    usb_config.max_packet_size_0 = 64;
    // Keyboard + serial log port: a composite device with interface associations.
    #[cfg(feature = "usb-log")]
    {
        usb_config.device_class = 0xEF;
        usb_config.device_sub_class = 0x02;
        usb_config.device_protocol = 0x01;
        usb_config.composite_with_iads = true;
    }

    let mut builder = Builder::new(
        driver,
        usb_config,
        CONFIG_DESC.init([0; 256]),
        BOS_DESC.init([0; 256]),
        MSOS_DESC.init([0; 256]),
        CONTROL_BUF.init([0; 64]),
    );
    let hid_config = hid::Config {
        report_descriptor: KeyboardReport::desc(),
        request_handler: None,
        poll_ms: 1,
        max_packet_size: 8,
        hid_subclass: hid::HidSubclass::Boot,
        hid_boot_protocol: hid::HidBootProtocol::Keyboard,
    };
    let writer =
        HidWriter::<_, 8>::new(&mut builder, HID_STATE.init(hid::State::new()), hid_config);
    // The mouse: an interface of its own (no report IDs, so the keyboard's
    // boot interface stays exactly as it was).
    let mouse_config = hid::Config {
        report_descriptor: &kb_core::mouse::REPORT_DESCRIPTOR,
        request_handler: Some(MOUSE_REQUESTS.init(MouseRequests)),
        poll_ms: 1,
        max_packet_size: 8,
        hid_subclass: hid::HidSubclass::No,
        hid_boot_protocol: hid::HidBootProtocol::None,
    };
    let mouse = HidWriter::<_, MOUSE_REPORT_LEN>::new(
        &mut builder,
        MOUSE_STATE.init(hid::State::new()),
        mouse_config,
    );
    builder.handler(USB_STATE.init(UsbState));
    #[cfg(feature = "usb-log")]
    let log_port = {
        static CDC_STATE: StaticCell<embassy_usb::class::cdc_acm::State> = StaticCell::new();
        embassy_usb::class::cdc_acm::CdcAcmClass::new(
            &mut builder,
            CDC_STATE.init(embassy_usb::class::cdc_acm::State::new()),
            64,
        )
    };
    // Raw HID (messages for the screen, `kb_core::host`): an interface of its
    // own, as QMK's, after the others so their numbers stay (the log port
    // is interface 2 either way). 32-byte reports, no report IDs.
    let raw_hid_config = hid::Config {
        report_descriptor: &host_msg::USB_REPORT_DESCRIPTOR,
        request_handler: Some(RAW_HID_REQUESTS.init(RawHidRequests)),
        poll_ms: 10,
        max_packet_size: host_msg::REPORT_SIZE as u16,
        hid_subclass: hid::HidSubclass::No,
        hid_boot_protocol: hid::HidBootProtocol::None,
    };
    let (raw_hid, _never_sent) =
        HidReaderWriter::<_, { host_msg::REPORT_SIZE }, { host_msg::REPORT_SIZE }>::new(
            &mut builder,
            RAW_HID_STATE.init(hid::State::new()),
            raw_hid_config,
        )
        .split();
    let usb = builder.build();
    start(spawner, usb_task(usb));
    #[cfg(feature = "usb-log")]
    start(spawner, logport::log_task(log_port));
    start(spawner, hid_task(writer));
    start(spawner, mouse_task(mouse));
    start(spawner, raw_hid_out_task(raw_hid));
    start(spawner, raw_hid_task());
}

/// How often the key loop sends the display a fresh status while awake.
#[cfg(not(all(feature = "ble", feature = "right")))]
const SCREEN_INTERVAL_MS: u32 = 50;

#[cfg(not(all(feature = "ble", feature = "right")))]
/// The keyboard: scans this half, merges in the right half's keys (with
/// `ble`), runs the engine and sends reports to USB.
async fn run_keyboard(
    mut cols: [Output<'static>; 6],
    mut rows: [Input<'static>; 4],
    ext_power: Output<'static>,
) -> ! {
    let mut kb = Keyboard::new();
    let mut ticker = Ticker::every(SCAN_INTERVAL);

    loop {
        ticker.next().await;
        let now = Instant::now().as_millis() as u32;

        let raw = matrix::scan(&mut cols, &rows);
        let mut activity = kb.take_keys(&raw, now);
        activity |= kb.take_host_messages(now);
        kb.follow_studio();
        kb.keys.engine.tick(now);

        // On USB power the board never deep-sleeps; it only goes idle (display off).
        kb.power.set_sleep_allowed(!sleep::usb_powered());
        activity |= kb.follow_output();
        let power_change = if activity {
            kb.power.activity(now)
        } else {
            kb.power.tick(now)
        };
        if let Some(state) = power_change {
            dlog!("power: {:?}", state);
        }
        if kb.power.state() == PowerState::Sleep {
            sleep::deep_sleep(ext_power, cols, rows);
        }

        while let Some(out) = kb.keys.engine.poll() {
            kb.handle(out, now);
        }
        kb.send_mouse_motion(now);

        // Screens off: the split link slows down too, until the next key.
        #[cfg(feature = "ble")]
        ble::LEFT.set_resting(kb.power.state() != PowerState::Active);
        if power_change.is_some() || now.wrapping_sub(kb.last_screen) >= SCREEN_INTERVAL_MS {
            kb.last_screen = now;
            kb.status.studio_unlocked = STUDIO.with(|s| {
                if s.expire(now) {
                    dlog!("studio: locked (idle)");
                }
                s.unlocked(now)
            });
            kb.publish(now);
        }

        // Nothing held, pending or timing out: stop scanning and let the chip
        // sleep until a key closes a row (or the next idle/sleep deadline, or
        // a key from the right half). The display gets the current state first,
        // or it would keep showing whatever it had 50 ms ago until the next key.
        if kb.is_idle() {
            kb.publish(now);
            let wait_ms = [kb.power.next_deadline(), kb.note.deadline()]
                .into_iter()
                .flatten()
                .map(|t| t.wrapping_sub(now))
                .min();
            #[cfg(feature = "ble")]
            let also = select(
                ble::LEFT.remote_events.ready_to_receive(),
                ble::LEFT.hid.state_changed.wait(),
            );
            #[cfg(not(feature = "ble"))]
            let also = core::future::pending::<()>();
            // So does a message from the computer.
            let also = select(also, inbox().screen.wait());
            // Keys from the log port wake it too.
            #[cfg(feature = "usb-log")]
            let also = select(also, log::INJECT.ready_to_receive());
            matrix::wait_for_key(&mut cols, &mut rows, wait_ms, also).await;
            ticker.reset();
        }
    }
}

/// The engine, fed by every source of keys.
#[cfg(not(all(feature = "ble", feature = "right")))]
struct Keys {
    engine: Engine<'static, { corne::KEYS }>,
    /// Keys held on both halves (for the log port's debug stream).
    #[cfg(feature = "usb-log")]
    held: u64,
}

#[cfg(not(all(feature = "ble", feature = "right")))]
impl Keys {
    /// Key `pos` went down or up at `now`.
    fn event(&mut self, pos: u8, down: bool, now: u32) {
        #[cfg(feature = "usb-log")]
        if down {
            self.held |= 1 << pos;
        } else {
            self.held &= !(1 << pos);
        }
        if down {
            self.engine.press(pos, now);
        } else {
            self.engine.release(pos, now);
        }
    }
}

/// Everything the key loop keeps between scans.
#[cfg(not(all(feature = "ble", feature = "right")))]
struct Keyboard {
    keys: Keys,
    /// Mouse keys: motion, buttons and wheel for the computer.
    mouse: Mouse,
    debounce: Debouncer<{ corne::KEYS }>,
    power: Power,
    wpm: Wpm,
    status: Status,
    /// The last report sent to the computer.
    host: Report,
    /// The latest message from the computer, while it is up.
    note: Shown,
    /// Studio's bindings as the engine has them.
    studio_generation: u32,
    /// When the display last got the status (milliseconds).
    last_screen: u32,
    /// Selected output, like a profile: Bluetooth (its profile) or USB,
    /// chosen with BT ◀ / ▶ (USB is a slot in that ring) or `OutToggle`.
    #[cfg(feature = "ble")]
    prefer: kb_core::link::Output,
    /// The engine's last report, before the pairing prompt takes keys out.
    #[cfg(feature = "ble")]
    engine_report: Report,
    /// A computer pairing with a code: what the user types answers it
    /// instead of reaching a computer.
    #[cfg(feature = "ble")]
    pair_prompt: kb_core::link::PairPrompt,
    #[cfg(feature = "ble")]
    pair_request: Option<kb_core::link::PairRequest>,
}

#[cfg(not(all(feature = "ble", feature = "right")))]
impl Keyboard {
    fn new() -> Self {
        Self {
            keys: Keys {
                engine: Engine::new(&corne::KEYMAP, Config::default()),
                #[cfg(feature = "usb-log")]
                held: 0,
            },
            mouse: Mouse::default(),
            debounce: Debouncer::new(PRESS_DEBOUNCE_MS, RELEASE_DEBOUNCE_MS),
            power: Power::new(PowerConfig::ZMK_CONFIG, 0),
            wpm: Wpm::new(),
            status: Status {
                transport: Transport::Usb,
                peripheral_connected: false,
                profile_connected: false,
                ..Status::default()
            },
            host: Report::default(),
            note: Shown::new(),
            studio_generation: STUDIO.generation().wrapping_sub(1),
            last_screen: 0,
            #[cfg(feature = "ble")]
            prefer: kb_core::link::Output::Ble,
            #[cfg(feature = "ble")]
            engine_report: Report::default(),
            #[cfg(feature = "ble")]
            pair_prompt: kb_core::link::PairPrompt::new(),
            #[cfg(feature = "ble")]
            pair_request: None,
        }
    }

    /// Feed the engine this half's debounced keys, the log port's injected
    /// ones and the right half's. Whether any key changed.
    fn take_keys(&mut self, raw: &[bool; corne::KEYS], now: u32) -> bool {
        let mut activity = false;
        self.debounce.update(raw, now, |pos, down| {
            activity = true;
            self.keys.event(pos, down, now);
        });
        #[cfg(feature = "usb-log")]
        while let Ok((pos, down)) = log::INJECT.try_receive() {
            dlog!("inject: {} {}", if down { "press" } else { "release" }, pos);
            activity = true;
            self.keys.event(pos, down, now);
        }
        #[cfg(feature = "ble")]
        while let Ok((pos, down)) = ble::LEFT.remote_events.try_receive() {
            activity = true;
            self.keys.event(pos, down, now);
        }
        #[cfg(feature = "usb-log")]
        logport::set_held(self.keys.held);
        activity
    }

    /// A message from the computer (it wakes the screens, so it can be
    /// read), and the current one's time running out. Whether one arrived.
    fn take_host_messages(&mut self, now: u32) -> bool {
        let mut activity = false;
        if let Some(message) = inbox().screen.take() {
            activity = matches!(message, host_msg::HostMessage::Notify(_));
            self.note.apply(message, now);
        }
        self.note.tick(now);
        self.status.notification = self.note.text();
        activity
    }

    /// Studio changed the keymap or the combos (an edit, a revert, flash
    /// loaded at start): from the next press on the engine types with them.
    /// A key or combo held now keeps what it was pressed as.
    fn follow_studio(&mut self) {
        if STUDIO.generation() != self.studio_generation {
            let engine = &mut self.keys.engine;
            self.studio_generation = STUDIO.with(|s| {
                engine.set_layers(s.layers());
                engine.set_combos(s.combos().defs());
                s.generation()
            });
        }
    }

    /// The output the reports go to, and the Bluetooth state on the screen.
    /// Whether a pairing prompt came or went (which wakes the screens).
    #[cfg(not(feature = "ble"))]
    fn follow_output(&mut self) -> bool {
        self.status.transport = Transport::Usb;
        false
    }

    /// The output the reports go to, and the Bluetooth state on the screen.
    /// Whether a pairing prompt came or went (which wakes the screens).
    #[cfg(feature = "ble")]
    fn follow_output(&mut self) -> bool {
        use core::sync::atomic::Ordering::Relaxed;
        // The selected output, like a profile: a cable for power or the log
        // port doesn't take the keys from Bluetooth.
        let output = match self.prefer {
            kb_core::link::Output::Usb => Transport::Usb,
            kb_core::link::Output::Ble => Transport::Ble,
        };
        let status = &mut self.status;
        if output != status.transport {
            // Release everything on the output we leave.
            send_report(&Report::default(), status.transport);
            self.mouse.reset();
            send_mouse(&MouseReport::default(), status.transport);
            dlog!("output: {:?}", output);
        }
        status.transport = output;
        status.profile = ble::LEFT.hid.active.load(Relaxed);
        status.profile_connected = ble::LEFT.hid.connected.load(Relaxed);
        status.profile_open = ble::LEFT.hid.open.load(Relaxed);
        let right = ble::LEFT.right_battery.load(Relaxed);
        if right <= 100 {
            status.battery[1] = right;
        }
        status.peripheral_connected = ble::link().linked.load(Relaxed);
        status.link_detail = ble::link().stage_name();
        status.radio_dbm = ble::LEFT.radio.dbm();
        status.radio_changes = ble::LEFT.radio.changes();
        // A pairing prompt came or went: the keys held now go in or out of
        // the reports, and the screens wake up for it.
        let request = ble::LEFT.hid.pairing();
        let changed = request != self.pair_request;
        if changed {
            self.pair_request = request;
            let (out, _) = self.pair_prompt.filter(request, &self.engine_report);
            if out != self.host {
                self.host = out;
                send_report(&out, self.status.transport);
            }
            dlog!("pairing prompt: {:?}", request);
        }
        self.status.pairing = kb_display::status::Pairing::of(request, &self.pair_prompt);
        changed
    }

    /// One thing the engine produced.
    fn handle(&mut self, out: EngineOutput, now: u32) {
        match out {
            EngineOutput::Report(r) => self.send_keys(r, now),
            EngineOutput::Layer(l) => self.status.layer = l,
            EngineOutput::CapsWord(on) => self.status.caps_word = on,
            EngineOutput::Mouse(action, true) => self.mouse.press(action, now),
            EngineOutput::Mouse(action, false) => self.mouse.release(action, now),
            EngineOutput::Pointer(on) => self.status.pointer_active = on,
            EngineOutput::System(s) => self.system(s, now),
        }
    }

    /// The engine's report `r`, through the pairing prompt (with Bluetooth),
    /// to the computer, counting the newly pressed keys.
    fn send_keys(&mut self, r: Report, now: u32) {
        #[cfg(feature = "ble")]
        let r = {
            self.engine_report = r;
            let (out, reply) = self.pair_prompt.filter(self.pair_request, &r);
            if let Some(reply) = reply {
                dlog!("pairing: {:?}", reply);
                ble::LEFT.hid.reply_pairing(reply);
            }
            self.status.pairing =
                kb_display::status::Pairing::of(self.pair_request, &self.pair_prompt);
            if out == self.host {
                return;
            }
            out
        };
        for code in r.keys() {
            if !self.host.contains(code) {
                self.wpm.key_pressed(now);
                self.status.key_presses = self.status.key_presses.wrapping_add(1);
                // The right half's screen rains for them too.
                #[cfg(feature = "ble")]
                ble::LEFT.set_key_presses(self.status.key_presses);
            }
        }
        self.host = r;
        self.status.mods = r.mods;
        send_report(&r, self.status.transport);
    }

    /// A system key without Bluetooth.
    #[cfg(not(feature = "ble"))]
    fn system(&mut self, s: kb_core::SystemAction, now: u32) {
        use kb_core::SystemAction;
        match s {
            // No radio on the USB-only build.
            SystemAction::RadioPower => {}
            SystemAction::StudioUnlock => studio_unlock(now),
            _ => self.status.apply(s),
        }
    }

    /// A system key with Bluetooth.
    #[cfg(feature = "ble")]
    fn system(&mut self, s: kb_core::SystemAction, now: u32) {
        use kb_ble::Command;
        use kb_core::SystemAction;
        match s {
            SystemAction::BtNext | SystemAction::BtPrev => {
                self.step_profile(s == SystemAction::BtNext);
            }
            // On the USB slot there is no computer to forget.
            SystemAction::BtClear => {
                if self.prefer == kb_core::link::Output::Ble {
                    let _ = ble::LEFT.hid.commands.try_send(Command::Clear);
                }
            }
            SystemAction::OutToggle => {
                self.prefer = self.prefer.toggled();
                dlog!("output preference: {:?}", self.prefer);
            }
            SystemAction::StudioUnlock => studio_unlock(now),
            SystemAction::ShowLink => self.status.apply(s),
            // The Bluetooth task steps, stores and applies it; the screen
            // follows `ble::LEFT.radio`.
            SystemAction::RadioPower => ble::LEFT.radio.step(),
        }
    }

    /// BT ◀ / ▶: step through BT 1-10 and USB as one ring.
    #[cfg(feature = "ble")]
    fn step_profile(&mut self, forward: bool) {
        use core::sync::atomic::Ordering::Relaxed;
        use kb_core::link::{Output, Slot};
        let n = kb_ble::hid::PROFILES;
        let here = match self.prefer {
            Output::Usb => Slot::Usb,
            Output::Ble => Slot::Ble(ble::LEFT.hid.active.load(Relaxed) as usize),
        };
        let there = if forward { here.next(n) } else { here.prev(n) };
        match there {
            Slot::Usb => self.prefer = Output::Usb,
            Slot::Ble(i) => {
                self.prefer = Output::Ble;
                let _ = ble::LEFT
                    .hid
                    .commands
                    .try_send(kb_ble::Command::Select(i as u8));
            }
        }
        dlog!("profile: {:?}", there);
    }

    /// Mouse motion up to now; a report when the output takes one (USB
    /// every ms there is motion, Bluetooth at its own pace).
    fn send_mouse_motion(&mut self, now: u32) {
        let to = self.status.transport;
        let (wheel_hires, pan_hires) = kb_core::mouse::resolution_from_feature(mouse_feature(to));
        self.mouse.set_resolution(wheel_hires, pan_hires);
        self.mouse.tick(now);
        let (queue_full, interval) = mouse_queue(to, &self.mouse);
        if !queue_full && let Some(r) = self.mouse.take_report(now, interval) {
            send_mouse(&r, to);
        }
    }

    /// Nothing held, pending or moving.
    fn is_idle(&self) -> bool {
        self.debounce.is_idle()
            && self.keys.engine.next_deadline().is_none()
            && !self.mouse.active()
    }

    /// Send the display the status as of `now`.
    fn publish(&mut self, now: u32) {
        self.wpm.advance(now);
        self.status.wpm = self.wpm.wpm();
        publish(Screen {
            status: self.status,
            power: self.power.state(),
        });
    }
}

/// The `StudioUnlock` key: Studio takes edits for the next few minutes.
#[cfg(not(all(feature = "ble", feature = "right")))]
fn studio_unlock(now: u32) {
    STUDIO.with(|s| s.unlock(now));
    dlog!("studio: unlocked");
    #[cfg(feature = "usb-log")]
    logport::poke();
}

/// Right half over Bluetooth: scan, debounce, and hand the held keys to the
/// split link. No engine here; the left half runs it.
#[cfg(all(feature = "ble", feature = "right"))]
async fn run_split_right(
    mut cols: [Output<'static>; 6],
    mut rows: [Input<'static>; 4],
    ext_power: Output<'static>,
) -> ! {
    use core::sync::atomic::Ordering;
    let mut debounce = Debouncer::<{ corne::KEYS }>::new(PRESS_DEBOUNCE_MS, RELEASE_DEBOUNCE_MS);
    let mut power = Power::new(PowerConfig::ZMK_CONFIG, 0);
    let mut status = Status {
        transport: Transport::Ble,
        ..Status::default()
    };
    let mut held = 0u64;
    // Key presses on this half; the left half's come over the link.
    let mut own_presses = 0u16;
    let mut last_left = ble::RIGHT.left_presses();
    // The latest message from the computer, while it is up.
    let mut note = Shown::new();
    let mut ticker = Ticker::every(SCAN_INTERVAL);
    loop {
        ticker.next().await;
        let now = Instant::now().as_millis() as u32;
        let raw = matrix::scan(&mut cols, &rows);
        let mut activity = false;
        debounce.update(&raw, now, |pos, down| {
            activity = true;
            if down {
                held |= 1 << pos;
                // Digital rain on this half's screen.
                own_presses = own_presses.wrapping_add(1);
            } else {
                held &= !(1 << pos);
            }
        });
        if activity {
            ble::RIGHT.set_keys(held);
        }

        power.set_sleep_allowed(!sleep::usb_powered());
        // Typing on the left half keeps this half awake too (its screen rains
        // for those keys).
        let left = ble::RIGHT.left_presses();
        let typed_left = left != last_left;
        last_left = left;
        // A message from the computer wakes the screen, so it can be read.
        if let Some(message) = ble::RIGHT.take_notification() {
            activity |= matches!(message, host_msg::HostMessage::Notify(_));
            note.apply(message, now);
        }
        note.tick(now);
        status.notification = note.text();
        if activity || typed_left {
            power.activity(now);
        } else {
            power.tick(now);
        }
        if power.state() == PowerState::Sleep {
            sleep::deep_sleep(ext_power, cols, rows);
        }
        status.peripheral_connected = ble::link().linked.load(Ordering::Relaxed);
        status.link_detail = ble::link().stage_name();
        status.key_presses = own_presses.wrapping_add(ble::RIGHT.left_presses());
        publish(Screen {
            status,
            power: power.state(),
        });

        if debounce.is_idle() {
            // Wake at least once a second so the link icon follows the link.
            let wait_ms = power
                .next_deadline()
                .map(|t| t.wrapping_sub(now).min(LINK_ICON_REFRESH_MS))
                .or(Some(LINK_ICON_REFRESH_MS));
            let also = select(ble::RIGHT.left_typed(), ble::RIGHT.notification_arrived());
            matrix::wait_for_key(&mut cols, &mut rows, wait_ms, also).await;
            ticker.reset();
        }
    }
}

#[cfg(all(feature = "ble", feature = "right"))]
#[embassy_executor::task]
async fn split_peripheral_task(
    sdc: nrf_sdc::SoftdeviceController<'static>,
    flash: &'static mut BleFlash,
) -> ! {
    ble::run_right(sdc, flash, KEYBOARD_NAME).await
}

#[cfg(all(feature = "ble", not(feature = "right")))]
#[embassy_executor::task]
#[allow(
    clippy::large_futures,
    reason = "no heap to box it; the task's future is allocated statically"
)]
async fn split_central_task(
    sdc: nrf_sdc::SoftdeviceController<'static>,
    flash: &'static mut BleFlash,
) -> ! {
    ble::run_left(sdc, flash, KEYBOARD_NAME).await
}

/// USB power, polled: with Bluetooth, MPSL owns the interrupt that would report it.
/// Only changes are reported, since reporting "detected" resets "ready".
#[cfg(all(feature = "ble", not(feature = "right")))]
#[embassy_executor::task]
async fn vbus_task(vbus: &'static SoftwareVbusDetect) -> ! {
    let mut detected = false;
    let mut ready = false;
    loop {
        let status = embassy_nrf::pac::POWER.usbregstatus().read();
        if status.vbusdetect() != detected {
            detected = status.vbusdetect();
            dlog!("usb power: {}", if detected { "on" } else { "off" });
            ready = false;
            vbus.detected(detected);
        }
        if detected && !ready && status.outputrdy() {
            ready = true;
            vbus.ready();
        }
        Timer::after(Duration::from_millis(100)).await;
    }
}

#[cfg(not(all(feature = "ble", feature = "right")))]
/// Queue a boot-protocol report for the HID task. Each report is the whole
/// keyboard state, so if the queue is full (the computer stopped reading, e.g.
/// while suspended) the old ones are dropped and only the newest is kept. That
/// way the last state, with every release, always arrives.
fn send_report(r: &Report, to: Transport) {
    let mut report = [0u8; 8];
    report[0] = r.mods;
    report[2..].copy_from_slice(&r.boot_keys());
    #[cfg(feature = "ble")]
    let queue = match to {
        Transport::Usb => &REPORTS,
        Transport::Ble => &ble::LEFT.hid.reports,
    };
    #[cfg(not(feature = "ble"))]
    let queue = {
        let _ = to;
        &REPORTS
    };
    #[cfg(feature = "usb-log")]
    logport::set_report(report);
    if log::TRACE.load(core::sync::atomic::Ordering::Relaxed) {
        dlog!("report {:?}: {:02x?}", to, report);
    }
    if log::MUTE.load(core::sync::atomic::Ordering::Relaxed) {
        return;
    }
    // Full (nobody reading): drop the backlog; the newest report is the state.
    if queue.try_send(report).is_err() {
        queue.clear();
        let _ = queue.try_send(report);
    }
}

#[cfg(not(all(feature = "ble", feature = "right")))]
/// The mouse feature report `to`'s computer set (the Resolution Multipliers).
fn mouse_feature(to: Transport) -> u8 {
    use core::sync::atomic::Ordering::Relaxed;
    match to {
        #[cfg(feature = "ble")]
        Transport::Ble => ble::LEFT.hid.mouse_feature.load(Relaxed),
        _ => USB_MOUSE_FEATURE.load(Relaxed),
    }
}

#[cfg(not(all(feature = "ble", feature = "right")))]
/// Whether the mouse queue for `to` is full, and the least time between
/// motion reports there.
fn mouse_queue(to: Transport, mouse: &Mouse) -> (bool, u32) {
    match to {
        #[cfg(feature = "ble")]
        Transport::Ble => (
            ble::LEFT.hid.mouse_reports.is_full(),
            mouse.config().ble_interval_ms,
        ),
        _ => {
            let _ = mouse;
            (MOUSE_REPORTS.is_full(), 0)
        }
    }
}

#[cfg(not(all(feature = "ble", feature = "right")))]
/// Queue a mouse report for `to` (traced and muted like the key reports).
/// Taken from the mouse only when the queue has room ([`mouse_queue`]):
/// mouse motion is relative, so a report may not be dropped.
fn send_mouse(r: &MouseReport, to: Transport) {
    let report = r.bytes();
    #[cfg(feature = "usb-log")]
    logport::set_mouse(report);
    if log::TRACE.load(core::sync::atomic::Ordering::Relaxed) {
        dlog!("mouse {:?}: {:02x?}", to, report);
    }
    if log::MUTE.load(core::sync::atomic::Ordering::Relaxed) {
        return;
    }
    #[cfg(feature = "ble")]
    let queue = match to {
        Transport::Usb => &MOUSE_REPORTS,
        Transport::Ble => &ble::LEFT.hid.mouse_reports,
    };
    #[cfg(not(feature = "ble"))]
    let queue = {
        let _ = to;
        &MOUSE_REPORTS
    };
    let _ = queue.try_send(report);
}

#[cfg(not(all(feature = "ble", feature = "right")))]
#[embassy_executor::task]
async fn mouse_task(mut writer: HidWriter<'static, UsbDriver, MOUSE_REPORT_LEN>) -> ! {
    loop {
        let report = MOUSE_REPORTS.receive().await;
        let _ = writer.write(&report).await; // dropped when USB is not configured
    }
}

/// The USB Raw HID interface's interrupt OUT pipe: its reports go the same
/// way as `SET_REPORT`'s ([`RawHidRequests`]).
#[cfg(not(all(feature = "ble", feature = "right")))]
#[embassy_executor::task]
async fn raw_hid_out_task(reader: HidReader<'static, UsbDriver, { host_msg::REPORT_SIZE }>) -> ! {
    reader.run(false, &mut RawHidRequests).await
}

/// Puts the computer's Raw HID reports together into messages for the
/// screens (`kb_core::host`).
#[cfg(not(all(feature = "ble", feature = "right")))]
#[embassy_executor::task]
async fn raw_hid_task() -> ! {
    let mut messages = host_msg::Reassembler::new();
    loop {
        let report = RAW_HID_REPORTS.receive().await;
        if let Some(message) = messages.feed(&report) {
            dlog!("host: message for the screen (USB)");
            inbox().deliver(message);
        }
    }
}

#[cfg(not(all(feature = "ble", feature = "right")))]
#[embassy_executor::task]
async fn usb_task(mut usb: UsbDevice<'static, UsbDriver>) -> ! {
    usb.run().await
}

#[cfg(not(all(feature = "ble", feature = "right")))]
#[embassy_executor::task]
async fn hid_task(mut writer: HidWriter<'static, UsbDriver, 8>) -> ! {
    loop {
        let report = REPORTS.receive().await;
        let _ = writer.write(&report).await; // dropped when USB is not configured
    }
}

/// Measures the battery at boot and every minute. A task of its own, so the
/// key loop can stay asleep while it runs.
#[embassy_executor::task]
async fn battery_task(mut adc: saadc::Saadc<'static, 1>) -> ! {
    adc.calibrate().await;
    loop {
        let mut sample = [0i16; 1];
        adc.sample(&mut sample).await;
        let mv = battery::vddh_millivolts(sample[0]);
        let pct = battery::lithium_ion_percent(mv);
        BATTERY.signal(pct);
        #[cfg(all(feature = "usb-log", not(all(feature = "ble", feature = "right"))))]
        logport::set_battery(pct);
        #[cfg(all(feature = "ble", feature = "right"))]
        ble::RIGHT.set_battery(pct);
        #[cfg(all(feature = "ble", not(feature = "right")))]
        {
            let hid = &ble::LEFT.hid;
            hid.battery
                .store(pct, core::sync::atomic::Ordering::Relaxed);
            hid.battery_changed.signal(());
        }
        Timer::after(BATTERY_INTERVAL).await;
    }
}

/// Why the chip last reset (POWER.RESETREAS, cleared after reading), for the
/// boot line in the log: a reset that nobody asked for shows up here.
fn reset_reason() -> &'static str {
    let reas = embassy_nrf::pac::POWER.resetreas();
    let bits = reas.read().0;
    reas.write_value(embassy_nrf::pac::power::regs::Resetreas(bits));
    match bits {
        0 => "power-on or brown-out",
        b if b & 1 != 0 => "reset pin",
        b if b & (1 << 1) != 0 => "watchdog",
        b if b & (1 << 2) != 0 => "software (crash or bootloader)",
        b if b & (1 << 3) != 0 => "CPU lockup",
        b if b & (1 << 16) != 0 => "woke from deep sleep (key)",
        b if b & (1 << 20) != 0 => "woke from deep sleep (USB)",
        _ => "other",
    }
}
