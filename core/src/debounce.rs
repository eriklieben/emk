//! Per-key debouncing: a key changes state only after its raw reading has
//! been stable for `press_ms` / `release_ms` (ZMK's default is 5 ms each).

pub struct Debouncer<const N: usize> {
    stable: [bool; N],
    since: [Option<u32>; N],
    pub press_ms: u32,
    pub release_ms: u32,
}

impl<const N: usize> Debouncer<N> {
    #[must_use]
    pub const fn new(press_ms: u32, release_ms: u32) -> Self {
        Self {
            stable: [false; N],
            since: [None; N],
            press_ms,
            release_ms,
        }
    }

    /// No key is held and no change is pending: scanning can stop until a key
    /// press wakes the chip.
    pub fn is_idle(&self) -> bool {
        self.since.iter().all(Option::is_none) && !self.stable.iter().any(|&s| s)
    }

    /// Feed one matrix scan. Calls `on_change(position, pressed)` for every
    /// key whose debounced state changed.
    pub fn update(&mut self, raw: &[bool; N], now: u32, mut on_change: impl FnMut(u8, bool)) {
        for (i, &pressed) in raw.iter().enumerate() {
            if pressed == self.stable[i] {
                self.since[i] = None;
                continue;
            }
            let since = *self.since[i].get_or_insert(now);
            let needed = if pressed {
                self.press_ms
            } else {
                self.release_ms
            };
            if now.wrapping_sub(since) >= needed {
                self.stable[i] = pressed;
                self.since[i] = None;
                on_change(i as u8, pressed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Debouncer;

    fn scan(d: &mut Debouncer<2>, raw: [bool; 2], t: u32) -> Vec<(u8, bool)> {
        let mut out = Vec::new();
        d.update(&raw, t, |p, s| out.push((p, s)));
        out
    }

    #[test]
    fn press_is_not_reported_before_stable_period() {
        // Arrange
        let mut d = Debouncer::<2>::new(5, 5);
        scan(&mut d, [true, false], 0);

        // Act
        let changes = scan(&mut d, [true, false], 4);

        // Assert
        assert_eq!(changes, []);
    }

    #[test]
    fn press_is_reported_after_stable_period() {
        // Arrange
        let mut d = Debouncer::<2>::new(5, 5);
        scan(&mut d, [true, false], 0);

        // Act
        let changes = scan(&mut d, [true, false], 5);

        // Assert
        assert_eq!(changes, [(0, true)]);
    }

    #[test]
    fn stable_press_is_reported_only_once() {
        // Arrange
        let mut d = Debouncer::<2>::new(5, 5);
        scan(&mut d, [true, false], 0);
        scan(&mut d, [true, false], 5);

        // Act
        let changes = scan(&mut d, [true, false], 6);

        // Assert
        assert_eq!(changes, []);
    }

    #[test]
    fn bounce_restarts_the_timer() {
        // Arrange: pressed at 0, bounced open at 2, closed again at 3
        let mut d = Debouncer::<2>::new(5, 5);
        scan(&mut d, [true, false], 0);
        scan(&mut d, [false, false], 2);
        scan(&mut d, [true, false], 3);

        // Act
        let at_7 = scan(&mut d, [true, false], 7);
        let at_8 = scan(&mut d, [true, false], 8);

        // Assert
        assert_eq!(at_7, []);
        assert_eq!(at_8, [(0, true)]);
    }

    #[test]
    fn idle_before_any_key() {
        // Arrange
        let d = Debouncer::<2>::new(5, 5);

        // Act
        let idle = d.is_idle();

        // Assert
        assert!(idle);
    }

    #[test]
    fn not_idle_while_a_press_is_pending() {
        // Arrange
        let mut d = Debouncer::<2>::new(5, 5);

        // Act
        scan(&mut d, [true, false], 0);

        // Assert
        assert!(!d.is_idle());
    }

    #[test]
    fn not_idle_while_a_key_is_held() {
        // Arrange
        let mut d = Debouncer::<2>::new(0, 5);

        // Act
        scan(&mut d, [true, false], 0);

        // Assert
        assert!(!d.is_idle());
    }

    #[test]
    fn eager_press_is_reported_on_the_first_reading() {
        // Arrange
        let mut d = Debouncer::<2>::new(0, 5);

        // Act
        let changes = scan(&mut d, [true, false], 0);

        // Assert
        assert_eq!(changes, [(0, true)]);
    }

    #[test]
    fn release_waits_for_release_time() {
        // Arrange: no press debounce, 8 ms release debounce
        let mut d = Debouncer::<2>::new(0, 8);
        scan(&mut d, [false, true], 0);
        scan(&mut d, [false, false], 10);

        // Act
        let at_17 = scan(&mut d, [false, false], 17);
        let at_18 = scan(&mut d, [false, false], 18);

        // Assert
        assert_eq!(at_17, []);
        assert_eq!(at_18, [(1, false)]);
    }
}
