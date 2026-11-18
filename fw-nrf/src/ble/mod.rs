//! Bluetooth: Nordic's `SoftDevice` Controller (`nrf-sdc`) on top of the
//! Multiprotocol Service Layer (`nrf-mpsl`), with the `trouble` host.
//!
//! The links themselves (split link, HID to the computer, bonds) are the
//! hardware-independent `kb_ble` crate; this module brings up the radio
//! (`radio`) and runs this half's `kb_ble` code on it, with the chip's
//! address, the bond storage region and the half's shared state.

pub mod radio;
pub mod storage;

use kb_ble::split::SplitLink;
use kb_ble::trouble_host::prelude::Address;
use kb_ble::{Params, Timing, TxTarget};

/// The left half's Bluetooth state, shared with the key loop and the log.
#[cfg(not(feature = "right"))]
pub static LEFT: kb_ble::LeftState<'static> = kb_ble::LeftState::new(&LOG);

/// The right half's Bluetooth state, shared with the key loop and the log.
#[cfg(feature = "right")]
pub static RIGHT: kb_ble::RightState<'static> = kb_ble::RightState::new(&LOG);

/// This half's log ring: a static of its own, all zeros at start (`.bss`).
pub static LOG: kb_ble::log::LogRing = kb_ble::log::LogRing::new();

const _: () = assert!(kb_ble::hid::PROFILES == kb_display::status::BT_PROFILES as usize);

/// This half's split link state.
pub fn link() -> &'static SplitLink {
    #[cfg(not(feature = "right"))]
    return &LEFT.split;
    #[cfg(feature = "right")]
    return &RIGHT.split;
}

/// A static random address from the chip's factory device address, so each
/// half keeps the same address and the bond stays valid.
pub fn own_address() -> Address {
    let ficr = embassy_nrf::pac::FICR;
    let low = ficr.deviceaddr(0).read();
    let high = ficr.deviceaddr(1).read();
    let mut addr = [0u8; 6];
    addr[..4].copy_from_slice(&low.to_le_bytes());
    addr[4..].copy_from_slice(&(high as u16).to_le_bytes());
    addr[5] |= 0xC0; // static random address: two top bits set
    Address::random(addr)
}

fn params(name: &'static str) -> Params {
    Params {
        address: own_address(),
        storage: storage::range(),
        name,
        timing: Timing::FIRMWARE,
        set_tx_power,
    }
}

/// The `SoftDevice` Controller's vendor command "Zephyr Write Tx Power Level"
/// (per role / connection), called directly: it applies at once, also to a
/// running advertiser or link, without going through the host stack (which
/// owns the controller). The executor runs one task at a time in thread
/// mode, so it never interrupts the host's own commands.
fn set_tx_power(target: TxTarget, dbm: i8) -> Result<i8, u8> {
    use nrf_sdc::raw;
    let (handle_type, handle) = match target {
        TxTarget::Advertising => (raw::SDC_HCI_VS_TX_POWER_HANDLE_TYPE_ADV, 0),
        TxTarget::ScanInit => (raw::SDC_HCI_VS_TX_POWER_HANDLE_TYPE_SCAN_INIT, 0),
        TxTarget::Connection(handle) => (raw::SDC_HCI_VS_TX_POWER_HANDLE_TYPE_CONN, handle),
    };
    let params = raw::sdc_hci_cmd_vs_zephyr_write_tx_power_t {
        handle_type: handle_type as u8,
        handle,
        tx_power_level: dbm,
    };
    let mut selected = raw::sdc_hci_cmd_vs_zephyr_write_tx_power_return_t {
        handle_type: 0,
        handle: 0,
        selected_tx_power: 0,
    };
    // SAFETY: both point to live structs of the layout the function expects.
    let status =
        unsafe { raw::sdc_hci_cmd_vs_zephyr_write_tx_power(&raw const params, &raw mut selected) };
    match status {
        0 => Ok(selected.selected_tx_power),
        e => Err(e),
    }
}

/// Right half: advertise to the left half and send it the held keys, forever.
#[cfg(feature = "right")]
pub async fn run_right(
    sdc: nrf_sdc::SoftdeviceController<'static>,
    flash: &mut nrf_sdc::mpsl::Flash<'static>,
    name: &'static str,
) -> ! {
    let server = kb_ble::split::split_server(name);
    kb_ble::split::run_peripheral(&RIGHT, sdc, flash, &server, params(name)).await
}

/// Left half: the link to the right half and to the computer, forever.
#[cfg(not(feature = "right"))]
#[allow(
    clippy::large_futures,
    reason = "no heap to box it; it lives in the calling task's static storage"
)]
pub async fn run_left(
    sdc: nrf_sdc::SoftdeviceController<'static>,
    flash: &mut nrf_sdc::mpsl::Flash<'static>,
    name: &'static str,
) -> ! {
    let server = kb_ble::hid::hid_server(name);
    // Studio's edits live in the same flash map as the bonds.
    LEFT.keymap.attach(&crate::STUDIO);
    kb_ble::left::run(&LEFT, sdc, flash, &server, params(name)).await
}
