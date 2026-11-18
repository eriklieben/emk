//! Bring up MPSL and the `SoftDevice` Controller.

use embassy_executor::Spawner;
use embassy_nrf::mode::Async;
use embassy_nrf::{Peri, peripherals, rng};
use nrf_sdc::mpsl::{self, MultiprotocolServiceLayer};
use nrf_sdc::{self as sdc, SoftdeviceController};
use static_cell::StaticCell;

/// L2CAP packet size and queue depth per link.
const L2CAP_MTU: u16 = 72;
const L2CAP_TXQ: u8 = 3;
const L2CAP_RXQ: u8 = 3;

pub struct Radio {
    pub mpsl: &'static MultiprotocolServiceLayer<'static>,
    pub sdc: SoftdeviceController<'static>,
}

#[embassy_executor::task]
async fn mpsl_task(mpsl: &'static MultiprotocolServiceLayer<'static>) -> ! {
    mpsl.run().await
}

/// Which roles this half's controller supports.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Right half: advertises and accepts the left half.
    Peripheral,
    /// Left half: scans for and connects to the right half, and advertises
    /// to and accepts the computer.
    Central,
}

/// Start MPSL (with the internal RC low-frequency clock, as the nice!nano has
/// no 32 kHz crystal fitted for us to rely on) and build the controller.
#[allow(clippy::too_many_arguments)]
pub fn start<I>(
    spawner: Spawner,
    role: Role,
    irqs: I,
    rtc0: Peri<'static, peripherals::RTC0>,
    timer0: Peri<'static, peripherals::TIMER0>,
    temp: Peri<'static, peripherals::TEMP>,
    rng_p: Peri<'static, peripherals::RNG>,
    mpsl_ppi: (
        Peri<'static, peripherals::PPI_CH19>,
        Peri<'static, peripherals::PPI_CH30>,
        Peri<'static, peripherals::PPI_CH31>,
    ),
    sdc_p: sdc::Peripherals<'static>,
) -> Radio
where
    I: Copy
        + embassy_nrf::interrupt::typelevel::Binding<
            embassy_nrf::interrupt::typelevel::RNG,
            rng::InterruptHandler<peripherals::RNG>,
        > + embassy_nrf::interrupt::typelevel::Binding<
            embassy_nrf::interrupt::typelevel::EGU0_SWI0,
            mpsl::LowPrioInterruptHandler,
        > + embassy_nrf::interrupt::typelevel::Binding<
            embassy_nrf::interrupt::typelevel::CLOCK_POWER,
            mpsl::ClockInterruptHandler,
        > + embassy_nrf::interrupt::typelevel::Binding<
            embassy_nrf::interrupt::typelevel::RADIO,
            mpsl::HighPrioInterruptHandler,
        > + embassy_nrf::interrupt::typelevel::Binding<
            embassy_nrf::interrupt::typelevel::TIMER0,
            mpsl::HighPrioInterruptHandler,
        > + embassy_nrf::interrupt::typelevel::Binding<
            embassy_nrf::interrupt::typelevel::RTC0,
            mpsl::HighPrioInterruptHandler,
        > + 'static,
{
    static MPSL: StaticCell<MultiprotocolServiceLayer> = StaticCell::new();
    static SESSION_MEM: StaticCell<mpsl::SessionMem<1>> = StaticCell::new();
    static RNG: StaticCell<rng::Rng<'static, Async>> = StaticCell::new();
    static SDC_MEM: StaticCell<sdc::Mem<16384>> = StaticCell::new();
    let mpsl_p = mpsl::Peripherals::new(rtc0, timer0, temp, mpsl_ppi.0, mpsl_ppi.1, mpsl_ppi.2);
    let lfclk_cfg = mpsl::raw::mpsl_clock_lfclk_cfg_t {
        source: mpsl::raw::MPSL_CLOCK_LF_SRC_RC as u8,
        rc_ctiv: mpsl::raw::MPSL_RECOMMENDED_RC_CTIV as u8,
        rc_temp_ctiv: mpsl::raw::MPSL_RECOMMENDED_RC_TEMP_CTIV as u8,
        accuracy_ppm: mpsl::raw::MPSL_DEFAULT_CLOCK_ACCURACY_PPM as u16,
        skip_wait_lfclk_started: mpsl::raw::MPSL_DEFAULT_SKIP_WAIT_LFCLK_STARTED != 0,
    };
    let mpsl = MPSL.init(
        MultiprotocolServiceLayer::with_timeslots(
            mpsl_p,
            irqs,
            lfclk_cfg,
            SESSION_MEM.init(mpsl::SessionMem::new()),
        )
        .unwrap_or_else(|_| panic!("mpsl")),
    );
    crate::start(spawner, mpsl_task(mpsl));

    let rng = RNG.init(rng::Rng::new(rng_p, irqs));
    let mem = SDC_MEM.init(sdc::Mem::new());

    let builder = sdc::Builder::new().unwrap_or_else(|_| panic!("sdc"));
    let builder = match role {
        Role::Peripheral => builder
            .support_adv()
            .support_peripheral()
            .peripheral_count(1),
        Role::Central => builder
            .support_scan()
            .support_central()
            .central_count(1)
            .and_then(|b| b.support_adv().support_peripheral().peripheral_count(1)),
    }
    .and_then(|b| b.buffer_cfg(L2CAP_MTU, L2CAP_MTU, L2CAP_TXQ, L2CAP_RXQ))
    // 0 dBm by default (`kb_ble::TX_POWER`); the left half then sets the
    // level it keeps in flash (RadioPower key: +8, +4, 0) on its roles and
    // links at runtime (`kb_ble::radio`).
    .and_then(|b| b.default_tx_power(kb_ble::TX_POWER as i8))
    .unwrap_or_else(|_| panic!("sdc config"));
    let sdc = builder
        .build(sdc_p, rng, mpsl, mem)
        .unwrap_or_else(|_| panic!("sdc build"));
    Radio { mpsl, sdc }
}
