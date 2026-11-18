//! The decisions behind the Bluetooth links, without any radio code, so
//! they can be tested on the host: which connection the right half accepts,
//! what the computer profiles do on pairing and switching, and which output
//! gets the key reports. The firmware (`fw-nrf/src/ble`) feeds these with
//! what the Bluetooth stack reports and does what they answer.
//!
//! `P` is how a peer is identified (the firmware uses its identity address).

/// What the security layer reported on the right half's connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitEvent<P> {
    /// Pairing finished; `bond` is the new bond if both sides bonded.
    Paired { bond: Option<P> },
    /// The link is encrypted; `bond` is the stored bond it used, if any
    /// (`None` while a new pairing is finishing).
    Encrypted { bond: Option<P> },
}

/// What the right half does with its connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitVerdict<P> {
    /// It's the left half; store `new_bond` first if there is one.
    Accept { new_bond: Option<P> },
    /// Not the left half: disconnect.
    Reject,
    /// Not decided yet.
    Wait,
}

/// Whether a connection to the right half is its left half. With no bond
/// stored, the first device that pairs *and bonds* becomes the left half;
/// with one stored, only a link encrypted with exactly that bond is.
pub fn right_half_verdict<P: PartialEq>(
    stored: Option<&P>,
    event: SplitEvent<P>,
) -> SplitVerdict<P> {
    match (stored, event) {
        (None, SplitEvent::Paired { bond: Some(bond) }) => SplitVerdict::Accept {
            new_bond: Some(bond),
        },
        // A link encrypted with the stored bond. A reconnect like that can be
        // reported as a finished pairing too (when the right half asked for
        // security first, e.g. after the left half restarted while the right
        // half stayed on).
        (
            Some(stored),
            SplitEvent::Paired { bond: Some(bond) } | SplitEvent::Encrypted { bond: Some(bond) },
        ) if *stored == bond => SplitVerdict::Accept { new_bond: None },
        // Pairing without bonding, pairing anew while bonded, or a link
        // encrypted with another bond: someone else.
        (_, SplitEvent::Paired { .. } | SplitEvent::Encrypted { bond: Some(_) }) => {
            SplitVerdict::Reject
        }
        (_, SplitEvent::Encrypted { bond: None }) => SplitVerdict::Wait,
    }
}

/// How long the right half waits for a connection to prove it's the left half.
pub const SPLIT_VERIFY_MS: u32 = 10_000;

/// The computer profiles: one slot per paired computer, one selected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profiles<P, const N: usize> {
    slots: [Option<P>; N],
    active: usize,
}

/// What to do after a computer paired on the selected profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Paired<const N: usize> {
    /// Keep the connection (and store the bond in the selected slot).
    pub accept: bool,
    /// Slots that held the same computer before and are now open (forget them in flash).
    pub cleared: [bool; N],
}

impl<P: Copy + PartialEq, const N: usize> Profiles<P, N> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            slots: [None; N],
            active: 0,
        }
    }

    /// Restore from flash. An out-of-range `active` selects profile 0.
    pub fn restore(slots: [Option<P>; N], active: usize) -> Self {
        Self {
            slots,
            active: if active < N { active } else { 0 },
        }
    }

    pub fn active(&self) -> usize {
        self.active
    }

    pub fn slot(&self, i: usize) -> Option<P> {
        self.slots[i]
    }

    /// Whether the selected profile is open (ready to pair).
    pub fn is_open(&self) -> bool {
        self.slots[self.active].is_none()
    }

    pub fn next(&mut self) {
        self.active = (self.active + 1) % N;
    }

    pub fn prev(&mut self) {
        self.active = (self.active + N - 1) % N;
    }

    /// Select profile `i` (ignored when out of range).
    pub fn select(&mut self, i: usize) {
        if i < N {
            self.active = i;
        }
    }

    /// Forget the selected profile's computer; returns it (to remove its bond).
    pub fn clear(&mut self) -> Option<P> {
        self.slots[self.active].take()
    }

    /// A computer paired (with bonding) on the selected profile. Only an open
    /// profile takes it; if it was paired on another profile, it moves here.
    pub fn paired(&mut self, peer: P) -> Paired<N> {
        let mut cleared = [false; N];
        if !self.is_open() {
            return Paired {
                accept: false,
                cleared,
            };
        }
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if i != self.active && *slot == Some(peer) {
                *slot = None;
                cleared[i] = true;
            }
        }
        self.slots[self.active] = Some(peer);
        Paired {
            accept: true,
            cleared,
        }
    }

    /// A computer reconnected with its stored bond: only the selected
    /// profile's computer may stay.
    pub fn reconnected(&self, peer: P) -> bool {
        self.slots[self.active] == Some(peer)
    }
}

impl<P: Copy + PartialEq, const N: usize> Default for Profiles<P, N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Where key reports go: the selected output, whatever is plugged in. A
/// USB cable alone (power, the log port) doesn't take the keys away from the
/// Bluetooth profile; the USB/BT key selects USB like another profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    Usb,
    Ble,
}

impl Output {
    #[must_use]
    pub fn toggled(self) -> Self {
        match self {
            Self::Usb => Self::Ble,
            Self::Ble => Self::Usb,
        }
    }
}

/// What the Profile ◀ / ▶ keys step through: the Bluetooth profiles and USB, in
/// one ring (BT 1 … BT n, USB, BT 1 …).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// Bluetooth profile, 0-based.
    Ble(usize),
    Usb,
}

impl Slot {
    /// The slot after this one, with `n` Bluetooth profiles.
    #[must_use]
    pub fn next(self, n: usize) -> Self {
        match self {
            Self::Ble(i) if i + 1 < n => Self::Ble(i + 1),
            Self::Ble(_) => Self::Usb,
            Self::Usb => Self::Ble(0),
        }
    }

    /// The slot before this one, with `n` Bluetooth profiles.
    #[must_use]
    pub fn prev(self, n: usize) -> Self {
        match self {
            Self::Ble(0) => Self::Usb,
            Self::Ble(i) => Self::Ble(i - 1),
            Self::Usb => Self::Ble(n.saturating_sub(1)),
        }
    }
}

/// How eagerly to look for a link: fast at first (a reconnect after a
/// restart or a profile switch is quick), then slow, so a half that's off or a
/// computer that's away doesn't drain the battery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pace {
    Fast,
    Slow,
}

/// How long to look fast before slowing down.
pub const FAST_PHASE_MS: u32 = 30_000;

/// The pace `elapsed_ms` after starting to look, and how long until it changes.
#[must_use]
pub fn pace(elapsed_ms: u32) -> (Pace, Option<u32>) {
    if elapsed_ms < FAST_PHASE_MS {
        (Pace::Fast, Some(FAST_PHASE_MS - elapsed_ms))
    } else {
        (Pace::Slow, None)
    }
}

/// Advertising interval to the computer, in microseconds (min, max): fast like
/// Apple's guideline (20-30 ms), slow at 1022.5 ms, one of its recommended values.
#[must_use]
pub fn advertising_interval_us(pace: Pace) -> (u32, u32) {
    match pace {
        Pace::Fast => (20_000, 30_000),
        Pace::Slow => (1_022_500, 1_022_500),
    }
}

/// Scan (interval, window) in microseconds while connecting to the right half:
/// receiving all the time at first, then 30 ms out of every second (3 %).
#[must_use]
pub fn split_scan_us(pace: Pace) -> (u32, u32) {
    match pace {
        Pace::Fast => (60_000, 60_000),
        Pace::Slow => (1_000_000, 30_000),
    }
}

/// The radio's transmit power levels the `RadioPower` key steps through, in
/// dBm, strongest first: +8 (the nRF52840's maximum, for range), +4, 0.
pub const RADIO_DBM: [i8; 3] = [8, 4, 0];

/// The level of a fresh keyboard (nothing stored yet): 0 dBm, enough for a
/// desk and easy on the battery.
pub const RADIO_DEFAULT_DBM: i8 = 0;

/// The level after `dbm` (`RadioPower`): +8 → +4 → 0 → +8. A level not in
/// [`RADIO_DBM`] counts as the default.
#[must_use]
pub fn next_radio_dbm(dbm: i8) -> i8 {
    let dbm = valid_radio_dbm(dbm);
    match RADIO_DBM.iter().position(|&d| d == dbm) {
        Some(i) => RADIO_DBM[(i + 1) % RADIO_DBM.len()],
        None => RADIO_DBM[0],
    }
}

/// `dbm` if it is one of the [`RADIO_DBM`] levels (say, from flash), else
/// the default.
#[must_use]
pub fn valid_radio_dbm(dbm: i8) -> i8 {
    if RADIO_DBM.contains(&dbm) {
        dbm
    } else {
        RADIO_DEFAULT_DBM
    }
}

/// What a computer pairing with the keyboard asks of the keyboard's user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PairRequest {
    /// Passkey entry: type the 6-digit code the computer shows, then Enter.
    TypeCode,
    /// Numeric comparison: Enter if the computer shows this code, Escape if not.
    Compare(u32),
}

/// The keyboard user's answer to a [`PairRequest`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PairReply {
    /// The code typed (passkey entry).
    Passkey(u32),
    /// The codes match (numeric comparison).
    Confirm,
    /// Escape: no pairing.
    Reject,
}

/// HID usages the pairing prompt reads.
pub const KEY_ENTER: u8 = 0x28;
pub const KEY_ESCAPE: u8 = 0x29;
pub const KEY_BACKSPACE: u8 = 0x2A;
pub const KEY_KP_ENTER: u8 = 0x58;

/// Digits of a pairing code.
pub const PASSKEY_DIGITS: usize = 6;

/// The digit a key types: 1..9, 0 on the top row (0x1E..0x27) and the keypad
/// (0x59..0x62).
#[must_use]
pub fn digit_of(code: u8) -> Option<u8> {
    match code {
        0x1E..=0x26 => Some(code - 0x1E + 1),
        0x27 | 0x62 => Some(0),
        0x59..=0x61 => Some(code - 0x59 + 1),
        _ => None,
    }
}

/// The keyboard's side of a pairing prompt, in the key loop: while a
/// [`PairRequest`] is up, nothing reaches any computer; the keys the user
/// presses answer it instead. Digits fill the code (Backspace takes one
/// back), Enter sends the 6 digits (or confirms a compared code), Escape
/// rejects. A key pressed during the prompt stays out of the reports until
/// it is released, also after the prompt is gone (the Enter that answered
/// it isn't typed into the computer).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PairPrompt {
    /// The engine's last report (keys as the user holds them).
    held: crate::Report,
    /// Keys pressed during a prompt, kept out until released.
    swallowed: crate::Report,
    /// Whether a prompt was up at the last report.
    active: bool,
    digits: [u8; PASSKEY_DIGITS],
    len: u8,
}

impl PairPrompt {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            held: crate::Report::new(),
            swallowed: crate::Report::new(),
            active: false,
            digits: [0; PASSKEY_DIGITS],
            len: 0,
        }
    }

    /// The digits typed so far for the current prompt.
    #[must_use]
    pub fn typed(&self) -> &[u8] {
        &self.digits[..self.len as usize]
    }

    /// Take the engine's `report` (or the last one again, when only the
    /// prompt changed) while `request` is up (`None`: no prompt). Returns the
    /// report for the computer and the user's answer, if one was given.
    pub fn filter(
        &mut self,
        request: Option<PairRequest>,
        report: &crate::Report,
    ) -> (crate::Report, Option<PairReply>) {
        if request.is_some() != self.active {
            self.active = request.is_some();
            self.len = 0;
        }
        let mut reply = None;
        for code in report.keys() {
            if self.held.contains(code) {
                continue;
            }
            let Some(request) = request else { continue };
            self.swallowed.set(code, true);
            if reply.is_none() {
                reply = self.key(request, code);
            }
        }
        self.swallowed.retain_held_in(report);
        self.held = *report;
        let out = if request.is_some() {
            crate::Report::default()
        } else {
            let mut out = *report;
            out.remove_keys_of(&self.swallowed);
            out
        };
        (out, reply)
    }

    /// One newly pressed key during `request`.
    fn key(&mut self, request: PairRequest, code: u8) -> Option<PairReply> {
        match (request, code) {
            (_, KEY_ESCAPE) => Some(PairReply::Reject),
            (PairRequest::Compare(_), KEY_ENTER | KEY_KP_ENTER) => Some(PairReply::Confirm),
            (PairRequest::Compare(_), _) => None,
            (PairRequest::TypeCode, KEY_ENTER | KEY_KP_ENTER) => {
                (self.len as usize == PASSKEY_DIGITS).then(|| {
                    PairReply::Passkey(self.typed().iter().fold(0, |n, &d| n * 10 + u32::from(d)))
                })
            }
            (PairRequest::TypeCode, KEY_BACKSPACE) => {
                self.len = self.len.saturating_sub(1);
                None
            }
            (PairRequest::TypeCode, code) => {
                if let Some(d) = digit_of(code)
                    && (self.len as usize) < PASSKEY_DIGITS
                {
                    self.digits[self.len as usize] = d;
                    self.len += 1;
                }
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEFT: u8 = 1;
    const PC: u8 = 2;
    const OTHER_PC: u8 = 3;

    // ---- right half -------------------------------------------------------------

    #[test]
    fn unbonded_right_half_accepts_the_first_bonding_pairing() {
        // Arrange
        let event = SplitEvent::Paired { bond: Some(LEFT) };

        // Act
        let verdict = right_half_verdict(None, event);

        // Assert
        assert_eq!(
            verdict,
            SplitVerdict::Accept {
                new_bond: Some(LEFT)
            }
        );
    }

    #[test]
    fn right_half_rejects_pairing_without_bonding() {
        // Arrange: what the PC did today
        let event = SplitEvent::Paired { bond: None };

        // Act
        let verdict = right_half_verdict(Some(&LEFT), event);

        // Assert
        assert_eq!(verdict, SplitVerdict::Reject);
    }

    #[test]
    fn unbonded_right_half_rejects_pairing_without_bonding() {
        // Arrange
        let event = SplitEvent::<u8>::Paired { bond: None };

        // Act
        let verdict = right_half_verdict(None, event);

        // Assert
        assert_eq!(verdict, SplitVerdict::Reject);
    }

    #[test]
    fn bonded_right_half_rejects_a_bond_with_someone_else() {
        // Arrange
        let event = SplitEvent::Paired { bond: Some(PC) };

        // Act
        let verdict = right_half_verdict(Some(&LEFT), event);

        // Assert
        assert_eq!(verdict, SplitVerdict::Reject);
    }

    #[test]
    fn bonded_right_half_accepts_its_bond_reported_as_pairing() {
        // Arrange: the left half restarted while the right half stayed on
        let event = SplitEvent::Paired { bond: Some(LEFT) };

        // Act
        let verdict = right_half_verdict(Some(&LEFT), event);

        // Assert
        assert_eq!(verdict, SplitVerdict::Accept { new_bond: None });
    }

    #[test]
    fn right_half_accepts_reconnect_with_its_bond() {
        // Arrange
        let event = SplitEvent::Encrypted { bond: Some(LEFT) };

        // Act
        let verdict = right_half_verdict(Some(&LEFT), event);

        // Assert
        assert_eq!(verdict, SplitVerdict::Accept { new_bond: None });
    }

    #[test]
    fn right_half_rejects_reconnect_with_another_bond() {
        // Arrange
        let event = SplitEvent::Encrypted { bond: Some(PC) };

        // Act
        let verdict = right_half_verdict(Some(&LEFT), event);

        // Assert
        assert_eq!(verdict, SplitVerdict::Reject);
    }

    #[test]
    fn right_half_waits_on_encryption_of_a_new_pairing() {
        // Arrange
        let event = SplitEvent::<u8>::Encrypted { bond: None };

        // Act
        let verdict = right_half_verdict(None, event);

        // Assert
        assert_eq!(verdict, SplitVerdict::Wait);
    }

    // ---- profiles ---------------------------------------------------------------

    #[test]
    fn new_profiles_start_open_on_the_first() {
        // Arrange / Act
        let profiles = Profiles::<u8, 10>::new();

        // Assert
        assert_eq!((profiles.active(), profiles.is_open()), (0, true));
    }

    #[test]
    fn prev_from_the_first_profile_wraps_to_the_last() {
        // Arrange
        let mut profiles = Profiles::<u8, 10>::new();

        // Act
        profiles.prev();

        // Assert
        assert_eq!(profiles.active(), 9);
    }

    #[test]
    fn next_from_the_last_profile_wraps_to_the_first() {
        // Arrange
        let mut profiles = Profiles::<u8, 10>::restore([None; 10], 9);

        // Act
        profiles.next();

        // Assert
        assert_eq!(profiles.active(), 0);
    }

    #[test]
    fn restore_ignores_an_out_of_range_profile() {
        // Arrange / Act
        let profiles = Profiles::<u8, 10>::restore([None; 10], 42);

        // Assert
        assert_eq!(profiles.active(), 0);
    }

    #[test]
    fn pairing_on_an_open_profile_takes_it() {
        // Arrange
        let mut profiles = Profiles::<u8, 10>::new();

        // Act
        let outcome = profiles.paired(PC);

        // Assert
        assert_eq!(
            (outcome.accept, profiles.slot(0), profiles.is_open()),
            (true, Some(PC), false)
        );
    }

    #[test]
    fn pairing_on_a_taken_profile_is_refused() {
        // Arrange
        let mut slots = [None; 10];
        slots[0] = Some(PC);
        let mut profiles = Profiles::restore(slots, 0);

        // Act
        let outcome = profiles.paired(OTHER_PC);

        // Assert
        assert_eq!((outcome.accept, profiles.slot(0)), (false, Some(PC)));
    }

    #[test]
    fn a_computer_paired_again_on_another_profile_moves_there() {
        // Arrange: PC on profile 1, pairing again from profile 3
        let mut slots = [None; 10];
        slots[0] = Some(PC);
        let mut profiles = Profiles::restore(slots, 2);

        // Act
        let outcome = profiles.paired(PC);

        // Assert
        assert_eq!(
            (outcome.cleared[0], profiles.slot(0), profiles.slot(2)),
            (true, None, Some(PC))
        );
    }

    #[test]
    fn clearing_a_profile_opens_it_and_returns_its_computer() {
        // Arrange
        let mut slots = [None; 10];
        slots[4] = Some(PC);
        let mut profiles = Profiles::restore(slots, 4);

        // Act
        let forgotten = profiles.clear();

        // Assert
        assert_eq!((forgotten, profiles.is_open()), (Some(PC), true));
    }

    #[test]
    fn only_the_selected_profiles_computer_may_reconnect() {
        // Arrange: PC on profile 1, OTHER_PC on profile 2, profile 2 selected
        let mut slots = [None; 10];
        slots[0] = Some(PC);
        slots[1] = Some(OTHER_PC);
        let profiles = Profiles::restore(slots, 1);

        // Act
        let allowed = [profiles.reconnected(PC), profiles.reconnected(OTHER_PC)];

        // Assert
        assert_eq!(allowed, [false, true]);
    }

    #[test]
    fn nobody_may_reconnect_on_an_open_profile() {
        // Arrange
        let profiles = Profiles::<u8, 10>::new();

        // Act
        let allowed = profiles.reconnected(PC);

        // Assert
        assert!(!allowed);
    }

    // ---- pace -------------------------------------------------------------------

    #[test]
    fn looks_fast_at_first_and_says_when_it_slows_down() {
        // Arrange / Act
        let got = pace(10_000);

        // Assert
        assert_eq!(got, (Pace::Fast, Some(20_000)));
    }

    #[test]
    fn looks_slow_after_the_fast_phase() {
        // Arrange / Act
        let got = pace(FAST_PHASE_MS);

        // Assert
        assert_eq!(got, (Pace::Slow, None));
    }

    #[test]
    fn slow_split_scan_listens_a_small_part_of_the_time() {
        // Arrange / Act
        let (interval, window) = split_scan_us(Pace::Slow);

        // Assert: at most 5 % duty cycle
        assert!(window * 20 <= interval);
    }

    #[test]
    fn fast_advertising_fits_the_ble_range() {
        // Arrange / Act
        let (min, max) = advertising_interval_us(Pace::Fast);

        // Assert: 20 ms is the shortest connectable advertising interval
        assert!(min >= 20_000 && min <= max);
    }

    // ---- profile ring -------------------------------------------------------------

    #[test]
    fn next_after_the_last_bluetooth_profile_is_usb() {
        // Arrange / Act
        let slot = Slot::Ble(9).next(10);

        // Assert
        assert_eq!(slot, Slot::Usb);
    }

    #[test]
    fn next_after_usb_is_the_first_profile() {
        // Arrange / Act
        let slot = Slot::Usb.next(10);

        // Assert
        assert_eq!(slot, Slot::Ble(0));
    }

    #[test]
    fn prev_before_the_first_profile_is_usb() {
        // Arrange / Act
        let slot = Slot::Ble(0).prev(10);

        // Assert
        assert_eq!(slot, Slot::Usb);
    }

    #[test]
    fn prev_before_usb_is_the_last_profile() {
        // Arrange / Act
        let slot = Slot::Usb.prev(10);

        // Assert
        assert_eq!(slot, Slot::Ble(9));
    }

    #[test]
    fn next_moves_between_bluetooth_profiles() {
        // Arrange / Act
        let slot = Slot::Ble(2).next(10);

        // Assert
        assert_eq!(slot, Slot::Ble(3));
    }

    // ---- radio power ------------------------------------------------------------

    #[test]
    fn radio_power_steps_down_from_the_strongest() {
        // Arrange
        let dbm = 8;

        // Act
        let next = next_radio_dbm(dbm);

        // Assert
        assert_eq!(next, 4);
    }

    #[test]
    fn radio_power_wraps_from_the_weakest_to_the_strongest() {
        // Arrange
        let dbm = 0;

        // Act
        let next = next_radio_dbm(dbm);

        // Assert
        assert_eq!(next, 8);
    }

    #[test]
    fn radio_power_steps_from_four_to_zero() {
        // Arrange
        let dbm = 4;

        // Act
        let next = next_radio_dbm(dbm);

        // Assert
        assert_eq!(next, 0);
    }

    #[test]
    fn stored_radio_power_outside_the_levels_is_the_default() {
        // Arrange
        let stored = -40;

        // Act
        let dbm = valid_radio_dbm(stored);

        // Assert
        assert_eq!(dbm, RADIO_DEFAULT_DBM);
    }

    // ---- pairing prompt ---------------------------------------------------------

    fn report_with(codes: &[u8]) -> crate::Report {
        let mut r = crate::Report::default();
        for &c in codes {
            r.set(c, true);
        }
        r
    }

    /// Tap each key in turn (press, release) while `request` is up; the last reply.
    fn type_keys(prompt: &mut PairPrompt, request: PairRequest, codes: &[u8]) -> Option<PairReply> {
        let mut reply = None;
        for &c in codes {
            reply = prompt.filter(Some(request), &report_with(&[c])).1.or(reply);
            prompt.filter(Some(request), &report_with(&[]));
        }
        reply
    }

    const N1: u8 = 0x1E;
    const N2: u8 = 0x1F;
    const N0: u8 = 0x27;
    const A: u8 = 0x04;

    #[test]
    fn top_row_and_keypad_keys_are_digits() {
        // Arrange
        let codes = [0x1E, 0x26, 0x27, 0x59, 0x61, 0x62, 0x04];

        // Act
        let digits = codes.map(digit_of);

        // Assert
        assert_eq!(
            digits,
            [Some(1), Some(9), Some(0), Some(1), Some(9), Some(0), None]
        );
    }

    #[test]
    fn six_digits_and_enter_send_the_code() {
        // Arrange
        let mut prompt = PairPrompt::new();
        type_keys(
            &mut prompt,
            PairRequest::TypeCode,
            &[N1, N2, N0, 0x59, 0x22, 0x27],
        );

        // Act
        let reply = type_keys(&mut prompt, PairRequest::TypeCode, &[KEY_ENTER]);

        // Assert
        assert_eq!(reply, Some(PairReply::Passkey(120_150)));
    }

    #[test]
    fn enter_before_six_digits_sends_nothing() {
        // Arrange
        let mut prompt = PairPrompt::new();
        type_keys(&mut prompt, PairRequest::TypeCode, &[N1, N2]);

        // Act
        let reply = type_keys(&mut prompt, PairRequest::TypeCode, &[KEY_ENTER]);

        // Assert
        assert_eq!(reply, None);
    }

    #[test]
    fn backspace_takes_the_last_digit_back() {
        // Arrange
        let mut prompt = PairPrompt::new();
        type_keys(&mut prompt, PairRequest::TypeCode, &[N1, N2]);

        // Act
        type_keys(&mut prompt, PairRequest::TypeCode, &[KEY_BACKSPACE]);

        // Assert
        assert_eq!(prompt.typed(), [1]);
    }

    #[test]
    fn a_seventh_digit_is_ignored() {
        // Arrange
        let mut prompt = PairPrompt::new();
        type_keys(&mut prompt, PairRequest::TypeCode, &[N1; 6]);

        // Act
        type_keys(&mut prompt, PairRequest::TypeCode, &[N2]);

        // Assert
        assert_eq!(prompt.typed(), [1; 6]);
    }

    #[test]
    fn escape_rejects_the_code() {
        // Arrange
        let mut prompt = PairPrompt::new();
        type_keys(&mut prompt, PairRequest::TypeCode, &[N1]);

        // Act
        let reply = type_keys(&mut prompt, PairRequest::TypeCode, &[KEY_ESCAPE]);

        // Assert
        assert_eq!(reply, Some(PairReply::Reject));
    }

    #[test]
    fn nothing_reaches_the_computer_during_a_prompt() {
        // Arrange
        let mut prompt = PairPrompt::new();
        let mut typing = report_with(&[A, N1]);
        typing.mods = 0x02;

        // Act
        let (out, _) = prompt.filter(Some(PairRequest::TypeCode), &typing);

        // Assert
        assert_eq!(out, crate::Report::default());
    }

    #[test]
    fn enter_confirms_a_compared_code() {
        // Arrange
        let mut prompt = PairPrompt::new();

        // Act
        let reply = type_keys(&mut prompt, PairRequest::Compare(123_456), &[KEY_ENTER]);

        // Assert
        assert_eq!(reply, Some(PairReply::Confirm));
    }

    #[test]
    fn the_answering_enter_stays_out_of_the_reports_until_released() {
        // Arrange: Enter sent the code; the prompt is gone, Enter still held.
        let mut prompt = PairPrompt::new();
        type_keys(&mut prompt, PairRequest::TypeCode, &[N1; 6]);
        prompt.filter(Some(PairRequest::TypeCode), &report_with(&[KEY_ENTER]));

        // Act
        let (out, _) = prompt.filter(None, &report_with(&[KEY_ENTER, A]));

        // Assert
        assert!(!out.contains(KEY_ENTER) && out.contains(A));
    }

    #[test]
    fn enter_is_typed_again_after_the_answering_press_is_released() {
        // Arrange
        let mut prompt = PairPrompt::new();
        prompt.filter(Some(PairRequest::Compare(1)), &report_with(&[KEY_ENTER]));
        prompt.filter(None, &report_with(&[]));

        // Act
        let (out, _) = prompt.filter(None, &report_with(&[KEY_ENTER]));

        // Assert
        assert!(out.contains(KEY_ENTER));
    }

    #[test]
    fn keys_pass_without_a_prompt() {
        // Arrange
        let mut prompt = PairPrompt::new();

        // Act
        let (out, reply) = prompt.filter(None, &report_with(&[KEY_ENTER, N1]));

        // Assert
        assert_eq!((out, reply), (report_with(&[KEY_ENTER, N1]), None));
    }

    #[test]
    fn a_new_prompt_starts_with_no_digits() {
        // Arrange
        let mut prompt = PairPrompt::new();
        type_keys(&mut prompt, PairRequest::TypeCode, &[N1, N2]);
        prompt.filter(None, &report_with(&[]));

        // Act
        prompt.filter(Some(PairRequest::TypeCode), &report_with(&[]));

        // Assert
        assert_eq!(prompt.typed(), []);
    }
}
