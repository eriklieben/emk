//! Activity-based power states, like ZMK's `ZMK_IDLE_TIMEOUT` and
//! `ZMK_IDLE_SLEEP_TIMEOUT`:
//!
//! ```text
//! Active ──no activity for idle_ms──► Idle (OLEDs off, RGB off)
//! Idle   ──no activity for sleep_ms─► Sleep (deep sleep: radio off, VCC to OLEDs cut)
//! Idle   ──key──────────────────────► Active
//! Sleep  ──key──► reset (the chip wakes from System OFF by rebooting)
//! ```
//!
//! `sleep_ms` counts from the last activity, like ZMK.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerState {
    Active,
    Idle,
    Sleep,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PowerConfig {
    /// ZMK default: 30 s.
    pub idle_ms: u32,
    /// `None` disables deep sleep. ZMK default 15 min; Erik's config: 1 hour.
    pub sleep_ms: Option<u32>,
}

impl PowerConfig {
    /// The values in Erik's ZMK config (`ZMK_IDLE_TIMEOUT` default, `ZMK_IDLE_SLEEP_TIMEOUT=3600000`).
    pub const ZMK_CONFIG: Self = Self {
        idle_ms: 30_000,
        sleep_ms: Some(3_600_000),
    };
}

impl Default for PowerConfig {
    fn default() -> Self {
        Self::ZMK_CONFIG
    }
}

pub struct Power {
    config: PowerConfig,
    last_activity: u32,
    state: PowerState,
    sleep_allowed: bool,
}

impl Power {
    #[must_use]
    pub const fn new(config: PowerConfig, now: u32) -> Self {
        Self {
            config,
            last_activity: now,
            state: PowerState::Active,
            sleep_allowed: true,
        }
    }

    #[must_use]
    pub fn state(&self) -> PowerState {
        self.state
    }

    #[must_use]
    pub fn config(&self) -> PowerConfig {
        self.config
    }

    /// Change the timeouts; counts from the last activity.
    pub fn set_config(&mut self, config: PowerConfig, now: u32) -> Option<PowerState> {
        self.config = config;
        self.tick(now)
    }

    /// Allow or forbid deep sleep, e.g. forbid it while USB powers the board.
    /// Forbidden, the board stays idle (displays off) instead of sleeping.
    pub fn set_sleep_allowed(&mut self, allowed: bool) {
        self.sleep_allowed = allowed;
    }

    /// A key was pressed or released. Returns the new state if it changed.
    /// From `Sleep` the caller must reset the keyboard instead (see module docs).
    pub fn activity(&mut self, now: u32) -> Option<PowerState> {
        self.last_activity = now;
        self.set(PowerState::Active)
    }

    /// Advance time. Returns the new state if it changed.
    pub fn tick(&mut self, now: u32) -> Option<PowerState> {
        if self.state == PowerState::Sleep {
            return None;
        }
        let quiet = now.wrapping_sub(self.last_activity);
        let target = match self.config.sleep_ms {
            Some(sleep) if self.sleep_allowed && quiet >= sleep => PowerState::Sleep,
            _ if quiet >= self.config.idle_ms => PowerState::Idle,
            _ => PowerState::Active,
        };
        self.set(target)
    }

    /// When [`tick`](Self::tick) should next be called.
    #[must_use]
    pub fn next_deadline(&self) -> Option<u32> {
        match self.state {
            PowerState::Active => Some(self.last_activity.wrapping_add(self.config.idle_ms)),
            PowerState::Idle if self.sleep_allowed => self
                .config
                .sleep_ms
                .map(|s| self.last_activity.wrapping_add(s)),
            PowerState::Idle | PowerState::Sleep => None,
        }
    }

    fn set(&mut self, state: PowerState) -> Option<PowerState> {
        (state != self.state).then(|| {
            self.state = state;
            state
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAST: PowerConfig = PowerConfig {
        idle_ms: 100,
        sleep_ms: Some(1_000),
    };

    #[test]
    fn stays_active_before_idle_timeout() {
        // Arrange
        let mut p = Power::new(FAST, 0);

        // Act
        let change = p.tick(99);

        // Assert
        assert_eq!(change, None);
        assert_eq!(p.state(), PowerState::Active);
    }

    #[test]
    fn goes_idle_after_idle_timeout() {
        // Arrange
        let mut p = Power::new(FAST, 0);

        // Act
        let change = p.tick(100);

        // Assert
        assert_eq!(change, Some(PowerState::Idle));
    }

    #[test]
    fn goes_to_sleep_after_sleep_timeout() {
        // Arrange
        let mut p = Power::new(FAST, 0);
        p.tick(100);

        // Act
        let change = p.tick(1_000);

        // Assert
        assert_eq!(change, Some(PowerState::Sleep));
    }

    #[test]
    fn activity_wakes_from_idle() {
        // Arrange
        let mut p = Power::new(FAST, 0);
        p.tick(500);

        // Act
        let change = p.activity(600);

        // Assert
        assert_eq!(change, Some(PowerState::Active));
    }

    #[test]
    fn activity_restarts_the_timeouts() {
        // Arrange
        let mut p = Power::new(FAST, 0);
        p.activity(90);

        // Act
        let change = p.tick(150);

        // Assert
        assert_eq!(change, None);
    }

    #[test]
    fn forbidden_sleep_stays_idle() {
        // Arrange
        let mut p = Power::new(FAST, 0);
        p.set_sleep_allowed(false);
        p.tick(100);

        // Act
        let change = p.tick(5_000);

        // Assert
        assert_eq!(change, None);
        assert_eq!(p.state(), PowerState::Idle);
    }

    #[test]
    fn forbidden_sleep_has_no_sleep_deadline() {
        // Arrange
        let mut p = Power::new(FAST, 0);
        p.set_sleep_allowed(false);
        p.tick(100);

        // Act
        let deadline = p.next_deadline();

        // Assert
        assert_eq!(deadline, None);
    }

    #[test]
    fn sleep_can_be_disabled() {
        // Arrange
        let mut p = Power::new(
            PowerConfig {
                sleep_ms: None,
                ..FAST
            },
            0,
        );

        // Act
        let change = p.tick(1_000_000);

        // Assert
        assert_eq!(change, Some(PowerState::Idle));
    }

    #[test]
    fn next_deadline_is_idle_then_sleep_time() {
        // Arrange
        let mut p = Power::new(FAST, 0);
        let first = p.next_deadline();
        p.tick(100);

        // Act
        let second = p.next_deadline();

        // Assert
        assert_eq!((first, second), (Some(100), Some(1_000)));
    }
}
