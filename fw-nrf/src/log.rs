//! Development log: timestamped lines in a RAM ring buffer, readable over a
//! USB serial port (CDC-ACM) next to the keyboard (`usb-log` feature, left
//! half). Lines written before the port is opened are kept (the last 4 KB),
//! so boot messages can be read afterwards: `cat /dev/ttyACM0`. Sending `s`
//! prints a status snapshot.

use kb_ble::LogRing;

/// The ring: the one the Bluetooth state borrows, so the Bluetooth code logs into it too.
#[cfg(feature = "ble")]
pub fn ring() -> &'static LogRing {
    &crate::ble::LOG
}

/// The ring, without Bluetooth.
#[cfg(not(feature = "ble"))]
pub fn ring() -> &'static LogRing {
    static RING: LogRing = LogRing::new();
    &RING
}

/// Key presses and releases from the log port (`p12` / `r12` / `k12`), for
/// test scripts; the key loop feeds them to the engine like real keys.
#[cfg(all(feature = "usb-log", not(all(feature = "ble", feature = "right"))))]
pub static INJECT: embassy_sync::channel::Channel<
    embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
    (u8, bool),
    16,
> = embassy_sync::channel::Channel::new();

/// Reports are traced but not sent (`m`), so tests don't type into the PC.
#[cfg(not(all(feature = "ble", feature = "right")))]
pub static MUTE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Whether every report sent to the computer is logged (`t` toggles it).
#[cfg(not(all(feature = "ble", feature = "right")))]
pub static TRACE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Write a log line: `dlog!("split: {}", stage)`.
#[macro_export]
macro_rules! dlog {
    ($($arg:tt)*) => {
        $crate::log::ring().write_line(format_args!($($arg)*))
    };
}
