//! Deep sleep (nRF52840 System OFF) with wake-up on any key, like ZMK.

use embassy_nrf::gpio::{Input, Output};
use embassy_nrf::pac;

/// True when USB power (VBUS) is present.
pub fn usb_powered() -> bool {
    pac::POWER.usbregstatus().read().vbusdetect()
}

/// Cut OLED power, arm the rows to wake the chip, and power off.
/// Waking resets the chip, so this never returns.
pub fn deep_sleep(
    mut ext_power: Output<'static>,
    mut cols: [Output<'static>; 6],
    rows: [Input<'static>; 4],
) -> ! {
    ext_power.set_low();
    // With every column high, any key pulls its row high.
    for col in &mut cols {
        col.set_high();
    }
    for (port, pin) in [(0, 22), (0, 24), (1, 0), (0, 11)] {
        let gpio = if port == 0 { pac::P0 } else { pac::P1 };
        gpio.pin_cnf(pin)
            .modify(|w| w.set_sense(pac::gpio::vals::Sense::High));
    }
    // GPIO configuration is kept in System OFF; don't let the drivers reset it.
    core::mem::forget(ext_power);
    core::mem::forget(cols);
    core::mem::forget(rows);
    pac::POWER.systemoff().write(|w| w.set_systemoff(true));
    loop {
        cortex_m::asm::wfe();
    }
}
