//! Battery level from the nRF52840's VDDH pin, the same way ZMK does it
//! (`battery_nrf_vddh.c` + `lithium_ion_mv_to_pct` in `battery.c`).
//!
//! The SAADC samples the internal VDDH/5 input with gain 1/2 against the
//! 0.6 V internal reference at 12 bits, so full scale is 1.2 V at the ADC,
//! 6 V at VDDH. On the nice!nano VDDH is the battery (or the charger, while
//! USB is plugged in, which then reads close to 100%).

/// ADC full scale in mV at the input: 0.6 V reference / gain 1/2.
const FULL_SCALE_MV: u32 = 1_200;
const RESOLUTION: u32 = 4_096; // 12 bits
const VDDH_DIVIDER: u32 = 5;

/// Battery voltage in mV from a raw 12-bit VDDH/5 sample.
#[must_use]
pub fn vddh_millivolts(raw: i16) -> u16 {
    let raw = raw.max(0) as u32;
    (raw * FULL_SCALE_MV * VDDH_DIVIDER / RESOLUTION).min(u32::from(u16::MAX)) as u16
}

/// ZMK's linear Li-ion approximation: 4200 mV and up is 100%, 3450 mV and
/// below is 0%.
#[must_use]
pub fn lithium_ion_percent(millivolts: u16) -> u8 {
    match millivolts {
        4_200.. => 100,
        ..=3_450 => 0,
        mv => (u32::from(mv) * 2 / 15 - 459) as u8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_scale_sample_is_six_volts() {
        // Arrange
        let raw = 4_096;

        // Act
        let mv = vddh_millivolts(raw);

        // Assert
        assert_eq!(mv, 6_000);
    }

    #[test]
    fn typical_battery_sample_converts_to_millivolts() {
        // Arrange: 3.9 V battery → 0.78 V at VDDH/5 → 0.78 / 1.2 * 4096 ≈ 2662
        let raw = 2_662;

        // Act
        let mv = vddh_millivolts(raw);

        // Assert
        assert_eq!(mv, 3_899);
    }

    #[test]
    fn negative_noise_reads_as_zero() {
        // Arrange
        let raw = -3;

        // Act
        let mv = vddh_millivolts(raw);

        // Assert
        assert_eq!(mv, 0);
    }

    #[test]
    fn full_battery_is_100_percent() {
        // Arrange
        let mv = 4_250;

        // Act
        let pct = lithium_ion_percent(mv);

        // Assert
        assert_eq!(pct, 100);
    }

    #[test]
    fn empty_battery_is_0_percent() {
        // Arrange
        let mv = 3_400;

        // Act
        let pct = lithium_ion_percent(mv);

        // Assert
        assert_eq!(pct, 0);
    }

    #[test]
    fn mid_voltage_matches_zmk_formula() {
        // Arrange: ZMK: 3900 * 2 / 15 - 459 = 61
        let mv = 3_900;

        // Act
        let pct = lithium_ion_percent(mv);

        // Assert
        assert_eq!(pct, 61);
    }
}
