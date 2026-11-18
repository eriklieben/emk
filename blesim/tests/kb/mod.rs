//! Running the keyboard's real Bluetooth code (`kb-ble`) on the air: a RAM
//! flash like the nRF52840's, the halves as the firmware starts them (with
//! short timeouts), and ways to watch their state.
//!
//! Each half's GATT server can only be built once per program (see
//! `kb_ble::split::split_server` and `kb_ble::hid::hid_server`), and a reused
//! one remembers its subscriptions, which a restarted half wouldn't. So every
//! test is a test program of its own (`tests/kb_*.rs`), and a test that
//! restarts the keyboard runs each start ("boot") in a child process of its
//! own ([`Boots`]), with the flash kept in files in between.

#![allow(dead_code)] // each test file uses its own subset

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Command;

use blesim::{Air, block_on};
use embassy_futures::select::{Either, select};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Timer};
use embedded_storage_async::nor_flash::{
    ErrorType, MultiwriteNorFlash, NorFlash, NorFlashError, NorFlashErrorKind, ReadNorFlash,
};
use kb_ble::hid::HidServer;
use kb_ble::trouble_host::prelude::Address;
use kb_ble::{LeftState, Params, RightState, SplitServer, Timing, TxTarget};

use crate::common::{LEFT, RIGHT};

/// Size of the bond storage region (as `STORAGE` in fw-nrf's memory.x).
pub const STORAGE_SIZE: usize = 0x8000;

/// NOR flash in RAM, with the nRF52840's geometry: 4-byte writes, 4 KB
/// erases. Erasing sets every byte to 0xFF; writing can only clear bits.
pub struct RamFlash {
    bytes: Vec<u8>,
}

impl RamFlash {
    /// Erased flash.
    pub fn new() -> Self {
        Self {
            bytes: vec![0xFF; STORAGE_SIZE],
        }
    }

    /// The flash as saved in `path`, or erased flash if there is no such file.
    pub fn load(path: &Path) -> Self {
        match std::fs::read(path) {
            Ok(bytes) => {
                assert_eq!(bytes.len(), STORAGE_SIZE, "flash image {}", path.display());
                Self { bytes }
            }
            Err(_) => Self::new(),
        }
    }

    pub fn save(&self, path: &Path) {
        std::fs::write(path, &self.bytes).expect("save flash image");
    }
}

#[derive(Debug)]
pub enum FlashError {
    NotAligned,
    OutOfBounds,
}

impl NorFlashError for FlashError {
    fn kind(&self) -> NorFlashErrorKind {
        match self {
            Self::NotAligned => NorFlashErrorKind::NotAligned,
            Self::OutOfBounds => NorFlashErrorKind::OutOfBounds,
        }
    }
}

impl ErrorType for RamFlash {
    type Error = FlashError;
}

impl RamFlash {
    fn range(&self, offset: u32, len: usize) -> Result<std::ops::Range<usize>, FlashError> {
        let start = offset as usize;
        let end = start.checked_add(len).ok_or(FlashError::OutOfBounds)?;
        if end > self.bytes.len() {
            return Err(FlashError::OutOfBounds);
        }
        Ok(start..end)
    }
}

#[allow(
    clippy::unused_async_trait_impl,
    reason = "the trait's methods are async; RAM answers at once"
)]
impl ReadNorFlash for RamFlash {
    const READ_SIZE: usize = 1;

    async fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        let range = self.range(offset, bytes.len())?;
        bytes.copy_from_slice(&self.bytes[range]);
        Ok(())
    }

    fn capacity(&self) -> usize {
        self.bytes.len()
    }
}

#[allow(
    clippy::unused_async_trait_impl,
    reason = "the trait's methods are async; RAM answers at once"
)]
impl NorFlash for RamFlash {
    const WRITE_SIZE: usize = 4;
    const ERASE_SIZE: usize = 4096;

    async fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        let aligned = |at: u32| (at as usize).is_multiple_of(Self::ERASE_SIZE);
        if !aligned(from) || !aligned(to) || to < from {
            return Err(FlashError::NotAligned);
        }
        let range = self.range(from, (to - from) as usize)?;
        self.bytes[range].fill(0xFF);
        Ok(())
    }

    async fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        if !(offset as usize).is_multiple_of(Self::WRITE_SIZE)
            || !bytes.len().is_multiple_of(Self::WRITE_SIZE)
        {
            return Err(FlashError::NotAligned);
        }
        let range = self.range(offset, bytes.len())?;
        for (cell, &b) in self.bytes[range].iter_mut().zip(bytes) {
            *cell &= b;
        }
        Ok(())
    }
}

// As MPSL's flash on the nRF52840: a word may be written again (clearing more bits).
impl MultiwriteNorFlash for RamFlash {}

/// Short timeouts, so a test takes seconds; links on the air are instant.
pub const TIMING: Timing = Timing {
    split_verify: Duration::from_millis(1_000),
    right_forward_delay: Duration::from_millis(100),
    notify_heartbeat: Duration::from_millis(100),
    pairing_timeout: Duration::from_millis(1_000),
    host_secure_timeout: Duration::from_millis(1_000),
    pair_timeout: Duration::from_millis(300),
    fast_phase_ms: 2_000,
};

fn params(addr: [u8; 6], name: &'static str) -> Params {
    Params {
        address: Address::random(addr),
        storage: 0..STORAGE_SIZE as u32,
        name,
        timing: TIMING,
        set_tx_power: record_tx_power,
    }
}

/// Every transmit power the halves set, in order (see [`record_tx_power`]).
pub static TX_POWER_SET: std::sync::Mutex<Vec<(TxTarget, i8)>> = std::sync::Mutex::new(Vec::new());

/// The controller's per-role power command, as the air has none: note it.
#[expect(
    clippy::unnecessary_wraps,
    reason = "a `SetTxPower`, which can fail on a real controller"
)]
fn record_tx_power(target: TxTarget, dbm: i8) -> Result<i8, u8> {
    TX_POWER_SET.lock().unwrap().push((target, dbm));
    Ok(dbm)
}

/// Stops a half (`Stop::signal`), as a power cut.
pub type Stop = Signal<CriticalSectionRawMutex, ()>;

/// The left half as the firmware runs it, on a new controller called "left",
/// until `stop`; then its controller is switched off.
pub async fn left_half(
    air: &Air,
    state: &LeftState<'_>,
    flash: &mut RamFlash,
    server: &HidServer<'_>,
    stop: &Stop,
) {
    let controller = air.controller("left");
    select(
        // Boxed: the left half's state machine is too big for the test's stack.
        Box::pin(kb_ble::left::run(
            state,
            controller,
            flash,
            server,
            params(LEFT, "left"),
        )),
        stop.wait(),
    )
    .await;
    air.power_off("left");
}

/// The right half as the firmware runs it, on a new controller called
/// "right", until `stop`; then its controller is switched off.
pub async fn right_half(
    air: &Air,
    state: &RightState<'_>,
    flash: &mut RamFlash,
    server: &SplitServer<'_>,
    stop: &Stop,
) {
    let controller = air.controller("right");
    select(
        kb_ble::split::run_peripheral(state, controller, flash, server, params(RIGHT, "right")),
        stop.wait(),
    )
    .await;
    air.power_off("right");
}

/// Run the halves (and whatever else) next to `scenario` until it finishes,
/// failing the test after 30 s.
pub fn run<R: Future, F: Future>(halves: R, scenario: F) -> F::Output {
    crate::common::init_log();
    block_on(async {
        match select(
            select(halves, scenario),
            Timer::after(Duration::from_secs(30)),
        )
        .await
        {
            Either::First(Either::First(_)) => panic!("the halves stopped before the scenario"),
            Either::First(Either::Second(output)) => output,
            Either::Second(()) => panic!("scenario timed out"),
        }
    })
}

/// Wait until `done` holds (checked every 5 ms), at most `within`; whether it did.
pub async fn wait_until(within: Duration, done: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + within;
    while !done() {
        if Instant::now() >= deadline {
            return false;
        }
        Timer::after(Duration::from_millis(5)).await;
    }
    true
}

/// Watch `cond` for `period` (every 5 ms) and say whether it ever held.
pub async fn ever_within(period: Duration, cond: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + period;
    let mut seen = false;
    while Instant::now() < deadline {
        seen |= cond();
        Timer::after(Duration::from_millis(5)).await;
    }
    seen
}

/// Whether the left half's split link is fully up (subscribed to the keys).
pub fn left_linked(left: &LeftState<'_>) -> bool {
    left.split.stage_name() == "linked"
}

/// Whether the right half accepted the left half.
pub fn right_linked(right: &RightState<'_>) -> bool {
    right
        .split
        .linked
        .load(std::sync::atomic::Ordering::Relaxed)
}

/// Everything still in a log ring, as text.
pub fn log_text(ring: &kb_ble::LogRing) -> String {
    let mut pos = ring.oldest();
    let mut text = Vec::new();
    let mut chunk = [0u8; 256];
    loop {
        let n = ring.read(&mut pos, &mut chunk);
        if n == 0 {
            break;
        }
        text.extend_from_slice(&chunk[..n]);
    }
    String::from_utf8_lossy(&text).into_owned()
}

/// Print a half's log when a test fails (`--nocapture` shows it always).
pub fn dump(name: &str, ring: &kb_ble::LogRing) {
    eprintln!("---- {name} log ----\n{}", log_text(ring));
}

// ---- boots: one start of the keyboard per child process ----------------------------

const BOOT_VAR: &str = "KB_BLE_BOOT";
const DIR_VAR: &str = "KB_BLE_DIR";

/// Runs a test's boots, one after the other, each in a child process of this
/// test program (the same test, told which boot it is). The halves' flash
/// stays in files in a directory of its own, from one boot to the next.
pub struct Boots {
    test: &'static str,
    dir: PathBuf,
}

/// What a boot reported ([`Boot::report`]).
pub type Report = BTreeMap<String, String>;

impl Boots {
    /// `test` is the name of the `#[test]` function that runs [`boot`] first.
    pub fn new(test: &'static str) -> Self {
        let dir = std::env::temp_dir().join(format!("kb-ble-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("boot directory");
        Self { test, dir }
    }

    /// Run boot `name` in a child process; its report. A boot that fails
    /// fails the test, with its output.
    pub fn run(&self, name: &str) -> Report {
        let output = Command::new(std::env::current_exe().expect("test program"))
            .args([self.test, "--exact", "--nocapture", "--test-threads=1"])
            .env(BOOT_VAR, name)
            .env(DIR_VAR, &self.dir)
            .output()
            .expect("start the boot");
        let text = String::from_utf8_lossy(&output.stdout).into_owned()
            + &String::from_utf8_lossy(&output.stderr);
        // Shown when the test fails.
        eprintln!("==== boot {name} ====\n{text}");
        assert!(output.status.success(), "boot {name} failed");
        let report = std::fs::read_to_string(self.dir.join(format!("{name}.report")))
            .unwrap_or_else(|_| panic!("boot {name} wrote no report"));
        report
            .lines()
            .filter_map(|line| line.split_once('='))
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect()
    }
}

impl Drop for Boots {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// In a boot's child process: which boot this is.
pub fn boot() -> Option<Boot> {
    let name = std::env::var(BOOT_VAR).ok()?;
    let dir = PathBuf::from(std::env::var_os(DIR_VAR).expect("boot directory"));
    Some(Boot {
        name,
        dir,
        report: Report::new(),
    })
}

/// One start of the keyboard, in its own process.
pub struct Boot {
    pub name: String,
    dir: PathBuf,
    report: Report,
}

impl Boot {
    /// The flash of the half called `half` as the last boot left it.
    pub fn flash(&self, half: &str) -> RamFlash {
        RamFlash::load(&self.dir.join(format!("{half}.flash")))
    }

    /// Keep the flash of `half` for the next boot.
    pub fn save_flash(&self, half: &str, flash: &RamFlash) {
        flash.save(&self.dir.join(format!("{half}.flash")));
    }

    /// Tell the test something (`key=value`).
    pub fn report(&mut self, key: &str, value: impl std::fmt::Display) {
        self.report.insert(key.to_owned(), value.to_string());
    }
}

impl Drop for Boot {
    fn drop(&mut self) {
        let text = self.report.iter().fold(String::new(), |mut text, (k, v)| {
            let _ = writeln!(text, "{k}={v}");
            text
        });
        let _ = std::fs::write(self.dir.join(format!("{}.report", self.name)), text);
    }
}
