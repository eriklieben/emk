//! Key matrix of one Corne half: 6 columns × 4 rows, diodes column → row.

use embassy_futures::select::{select, select_array};
use embassy_nrf::gpio::{Input, Output};
use embassy_time::Timer;
use kb_core::keymaps::corne::{self, Side, matrix_position};
use kb_display::frame::Half;

#[cfg(not(feature = "right"))]
const SIDE: Side = Side::Left;
#[cfg(feature = "right")]
const SIDE: Side = Side::Right;

/// Which half's OLED orientation and screen to use.
#[cfg(not(feature = "right"))]
pub const HALF: Half = Half::Left;
#[cfg(feature = "right")]
pub const HALF: Half = Half::Right;

/// Drive each column high in turn and read the rows.
pub fn scan(cols: &mut [Output<'static>; 6], rows: &[Input<'static>; 4]) -> [bool; corne::KEYS] {
    let mut raw = [false; corne::KEYS];
    for (c, col) in cols.iter_mut().enumerate() {
        col.set_high();
        // ~2 µs for the line to settle: asm::delay(n) loops n times at 3-5 cycles
        // each on the M4 (64 MHz), so 32 is 1.5-2.5 µs (docs/performance.md).
        cortex_m::asm::delay(32);
        for (r, row) in rows.iter().enumerate() {
            if let (true, Some(pos)) = (row.is_high(), matrix_position(SIDE, r, c)) {
                raw[pos as usize] = true;
            }
        }
        col.set_low();
    }
    raw
}

/// Sleep until any key is pressed, or `timeout_ms` passes (`None`: no timeout).
/// All columns are driven high, so any key pulls its row high; the rows wake
/// the chip through GPIOTE's port event, so nothing runs while waiting.
/// `also` ends the wait early too (e.g. a key event from the other half).
pub async fn wait_for_key(
    cols: &mut [Output<'static>; 6],
    rows: &mut [Input<'static>; 4],
    timeout_ms: Option<u32>,
    also: impl core::future::Future,
) {
    for col in cols.iter_mut() {
        col.set_high();
    }
    cortex_m::asm::delay(32);
    {
        let any_row = select_array(rows.each_mut().map(embassy_nrf::gpio::Input::wait_for_high));
        let timeout = async {
            match timeout_ms {
                Some(ms) => Timer::after_millis(u64::from(ms)).await,
                None => core::future::pending::<()>().await,
            }
        };
        select(select(any_row, timeout), also).await;
    }
    for col in cols.iter_mut() {
        col.set_low();
    }
}
