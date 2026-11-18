//! The keyboard state sent to the host: modifiers plus every pressed key (NKRO).

/// Modifier bits plus a 256-bit set of pressed HID usages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub mods: u8,
    keys: [u8; 32],
}

impl Report {
    /// No keys, no modifiers.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            mods: 0,
            keys: [0; 32],
        }
    }

    /// Keep only the keys `other` holds too.
    pub fn retain_held_in(&mut self, other: &Self) {
        for (k, o) in self.keys.iter_mut().zip(other.keys) {
            *k &= o;
        }
    }

    /// Take out every key `other` holds.
    pub fn remove_keys_of(&mut self, other: &Self) {
        for (k, o) in self.keys.iter_mut().zip(other.keys) {
            *k &= !o;
        }
    }
    pub fn set(&mut self, code: u8, pressed: bool) {
        let (byte, bit) = ((code / 8) as usize, code % 8);
        if pressed {
            self.keys[byte] |= 1 << bit;
        } else {
            self.keys[byte] &= !(1 << bit);
        }
    }

    #[must_use]
    pub fn contains(&self, code: u8) -> bool {
        self.keys[(code / 8) as usize] & (1 << (code % 8)) != 0
    }

    /// Pressed usages in ascending order (modifiers are in `mods`, not here).
    /// Skips empty bytes, so a report with a few keys costs a few steps.
    pub fn keys(&self) -> impl Iterator<Item = u8> + '_ {
        self.keys
            .iter()
            .enumerate()
            .filter(|(_, b)| **b != 0)
            .flat_map(|(i, &b)| {
                (0..8u8)
                    .filter(move |bit| b & (1 << bit) != 0)
                    .map(move |bit| i as u8 * 8 + bit)
            })
    }

    /// NKRO bitmap as sent in a USB/BLE NKRO report.
    #[must_use]
    pub fn nkro_bitmap(&self) -> &[u8; 32] {
        &self.keys
    }

    /// The six key slots of a boot-protocol report. With more than six keys
    /// held every slot is `ErrorRollOver` (0x01), as the HID spec asks, instead
    /// of a misleading subset.
    #[must_use]
    pub fn boot_keys(&self) -> [u8; 6] {
        const ERROR_ROLL_OVER: u8 = 0x01;
        let mut out = [0; 6];
        let mut keys = self.keys();
        for slot in &mut out {
            match keys.next() {
                Some(code) => *slot = code,
                None => return out,
            }
        }
        if keys.next().is_some() {
            return [ERROR_ROLL_OVER; 6];
        }
        out
    }
}

/// Calls `f(usage, pressed)` for everything that changed from `prev` to `next`.
///
/// Modifiers are reported as usages 0xE0..=0xE7. Releases come first (keys,
/// then modifiers), then presses (modifiers, then keys), so a host that
/// applies the changes one by one never sees a key with the wrong modifiers:
/// `{Shift, 1}` → `{2}` releases Shift before pressing 2.
pub fn diff(prev: &Report, next: &Report, mut f: impl FnMut(u8, bool)) {
    for code in 0..=255u8 {
        if prev.contains(code) && !next.contains(code) {
            f(code, false);
        }
    }
    for bit in 0..8 {
        if prev.mods & (1 << bit) != 0 && next.mods & (1 << bit) == 0 {
            f(0xE0 + bit, false);
        }
    }
    for bit in 0..8 {
        if prev.mods & (1 << bit) == 0 && next.mods & (1 << bit) != 0 {
            f(0xE0 + bit, true);
        }
    }
    for code in 0..=255u8 {
        if !prev.contains(code) && next.contains(code) {
            f(code, true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_keys(codes: &[u8]) -> Report {
        let mut r = Report::default();
        for &c in codes {
            r.set(c, true);
        }
        r
    }

    #[test]
    fn keys_are_listed_in_ascending_order() {
        // Arrange
        let r = with_keys(&[0x52, 0x04, 0xE0 - 1]);

        // Act
        let keys: Vec<u8> = r.keys().collect();

        // Assert
        assert_eq!(keys, [0x04, 0x52, 0xDF]);
    }

    #[test]
    fn up_to_six_keys_fill_the_boot_report() {
        // Arrange
        let r = with_keys(&[4, 5, 6, 7, 8, 9]);

        // Act
        let boot = r.boot_keys();

        // Assert
        assert_eq!(boot, [4, 5, 6, 7, 8, 9]);
    }

    #[test]
    fn more_than_six_keys_report_error_roll_over() {
        // Arrange
        let r = with_keys(&[4, 5, 6, 7, 8, 9, 10]);

        // Act
        let boot = r.boot_keys();

        // Assert
        assert_eq!(boot, [0x01; 6]);
    }
}
