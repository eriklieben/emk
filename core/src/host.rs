//! Messages from the computer to the keyboard, sent over a Raw HID
//! interface (vendor usage page 0xFF60, usage 0x61, 32-byte reports; the
//! same values QMK/VIA and zmk-raw-hid use), and forwarded to the right
//! half over the split link.
//!
//! Raw HID report (32 bytes):
//!
//! ```text
//! byte 0  kind         0x01 = notification chunk, 0x02 = clear notification
//! byte 1  message id   same for all chunks of one message
//! byte 2  chunk index  0-based
//! byte 3  chunk count  1..=5
//! byte 4  data length  bytes of text in this chunk (0..=27)
//! 5..32   text         UTF-8, zero padded
//! ```
//!
//! Every chunk but the last carries 27 bytes, so a message is at most
//! 5 × 27 = 135 bytes; text is limited to [`MAX_TEXT`] bytes. Control
//! characters are taken out on arrival ([`Text::from_host`]). There is no
//! reply: the keyboard only listens.
//!
//! Any program on a computer the keyboard is connected to (USB or a paired
//! Bluetooth profile) can send one; all it can do is show text on the right
//! half's screen.

pub const REPORT_SIZE: usize = 32;
pub const USAGE_PAGE: u16 = 0xFF60;
pub const USAGE: u8 = 0x61;
pub const MAX_TEXT: usize = 128;

/// How long a notification stays on the right screen (unless cleared).
pub const NOTIFICATION_MS: u32 = 15_000;

/// Report ID of the Raw HID collection in the Bluetooth report map (the
/// keyboard is 1, the mouse 2). The USB interface has no report IDs.
pub const BLE_REPORT_ID: u8 = 3;

/// Raw HID report descriptor of the USB interface (no report ID), as QMK's:
/// a 32-byte input report (usage 0x62, never sent) and a 32-byte output
/// report (usage 0x63, the messages).
#[rustfmt::skip]
pub const USB_REPORT_DESCRIPTOR: [u8; 34] = [
    0x06, 0x60, 0xFF, // Usage Page (Vendor 0xFF60)
    0x09, 0x61, // Usage (0x61)
    0xA1, 0x01, // Collection (Application)
    0x09, 0x62, //   Usage (0x62)
    0x15, 0x00, //   Logical Minimum (0)
    0x26, 0xFF, 0x00, //   Logical Maximum (255)
    0x95, REPORT_SIZE as u8, //   Report Count (32)
    0x75, 0x08, //   Report Size (8)
    0x81, 0x02, //   Input (Data, Variable, Absolute)
    0x09, 0x63, //   Usage (0x63)
    0x15, 0x00, //   Logical Minimum (0)
    0x26, 0xFF, 0x00, //   Logical Maximum (255)
    0x95, REPORT_SIZE as u8, //   Report Count (32)
    0x75, 0x08, //   Report Size (8)
    0x91, 0x02, //   Output (Data, Variable, Absolute)
    0xC0, // End Collection
];

/// The same collection in the Bluetooth report map: report ID
/// [`BLE_REPORT_ID`], output report only (a GATT input report would need a
/// characteristic and a subscription for nothing; hosts open the collection
/// without one).
#[rustfmt::skip]
pub const BLE_REPORT_DESCRIPTOR: [u8; 23] = [
    0x06, 0x60, 0xFF, // Usage Page (Vendor 0xFF60)
    0x09, 0x61, // Usage (0x61)
    0xA1, 0x01, // Collection (Application)
    0x85, BLE_REPORT_ID, //   Report ID (3)
    0x09, 0x63, //   Usage (0x63)
    0x15, 0x00, //   Logical Minimum (0)
    0x26, 0xFF, 0x00, //   Logical Maximum (255)
    0x95, REPORT_SIZE as u8, //   Report Count (32)
    0x75, 0x08, //   Report Size (8)
    0x91, 0x02, //   Output (Data, Variable, Absolute)
    0xC0, // End Collection
];

const HEADER: usize = 5;
pub const CHUNK_DATA: usize = REPORT_SIZE - HEADER;
pub const MAX_CHUNKS: usize = MAX_TEXT.div_ceil(CHUNK_DATA);

pub const KIND_NOTIFY: u8 = 0x01;
pub const KIND_CLEAR: u8 = 0x02;

/// Short UTF-8 text with a fixed capacity, so it can live in `no_std` code.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Text {
    bytes: [u8; MAX_TEXT],
    len: u8,
}

impl Text {
    /// Copy `s`, cut at the last whole character that fits.
    #[must_use]
    pub fn new(s: &str) -> Self {
        let mut end = s.len().min(MAX_TEXT);
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        let mut bytes = [0; MAX_TEXT];
        bytes[..end].copy_from_slice(&s.as_bytes()[..end]);
        Self {
            bytes,
            len: end as u8,
        }
    }

    /// Longest valid UTF-8 prefix of `raw` (invalid bytes end the text).
    #[must_use]
    pub fn from_utf8_lossy_prefix(raw: &[u8]) -> Self {
        let raw = &raw[..raw.len().min(MAX_TEXT)];
        let valid = match core::str::from_utf8(raw) {
            Ok(s) => s,
            Err(e) => core::str::from_utf8(&raw[..e.valid_up_to()]).unwrap_or(""),
        };
        Self::new(valid)
    }

    /// Text that came from a computer: the longest valid UTF-8 prefix of
    /// `raw`, with tabs and line breaks turned into spaces and any other
    /// control character left out (the screen has no use for them, and they
    /// could be used to garble it).
    #[must_use]
    pub fn from_host(raw: &[u8]) -> Self {
        let text = Self::from_utf8_lossy_prefix(raw);
        let mut out = Self {
            bytes: [0; MAX_TEXT],
            len: 0,
        };
        let mut len = 0;
        for c in text.as_str().chars() {
            let c = match c {
                '\t' | '\n' | '\r' => ' ',
                c if c.is_control() => continue,
                c => c,
            };
            len += c.encode_utf8(&mut out.bytes[len..]).len();
        }
        out.len = len as u8;
        out
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("")
    }
}

impl core::fmt::Debug for Text {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Debug::fmt(self.as_str(), f)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostMessage {
    Notify(Text),
    Clear,
}

/// Split `text` into Raw HID reports (the host side; the .NET tool does the same).
pub fn encode_notification(text: &str, id: u8, mut report: impl FnMut(&[u8; REPORT_SIZE])) {
    let text = Text::new(text);
    let bytes = text.as_str().as_bytes();
    let count = bytes.len().div_ceil(CHUNK_DATA).max(1);
    for index in 0..count {
        let chunk = &bytes
            [(index * CHUNK_DATA).min(bytes.len())..((index + 1) * CHUNK_DATA).min(bytes.len())];
        let mut r = [0u8; REPORT_SIZE];
        r[..HEADER].copy_from_slice(&[
            KIND_NOTIFY,
            id,
            index as u8,
            count as u8,
            chunk.len() as u8,
        ]);
        r[HEADER..HEADER + chunk.len()].copy_from_slice(chunk);
        report(&r);
    }
}

/// Collects notification chunks until a whole message has arrived.
pub struct Reassembler {
    id: Option<u8>,
    count: u8,
    received: u8,
    lens: [u8; MAX_CHUNKS],
    buf: [u8; MAX_CHUNKS * CHUNK_DATA],
}

impl Default for Reassembler {
    fn default() -> Self {
        Self::new()
    }
}

impl Reassembler {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            id: None,
            count: 0,
            received: 0,
            lens: [0; MAX_CHUNKS],
            buf: [0; MAX_CHUNKS * CHUNK_DATA],
        }
    }

    /// Feed one Raw HID report. Returns a message once it is complete;
    /// malformed reports are ignored.
    pub fn feed(&mut self, report: &[u8]) -> Option<HostMessage> {
        let &[kind, id, index, count, len, ..] = report else {
            return None;
        };
        match kind {
            KIND_CLEAR => {
                self.id = None;
                return Some(HostMessage::Clear);
            }
            KIND_NOTIFY => {}
            _ => return None,
        }
        let (index, count, len) = (index as usize, count as usize, len as usize);
        let last = index + 1 == count;
        let valid = (1..=MAX_CHUNKS).contains(&count)
            && index < count
            && len <= CHUNK_DATA
            && (last || len == CHUNK_DATA)
            && report.len() >= HEADER + len;
        if !valid {
            return None;
        }
        if self.id != Some(id) || self.count as usize != count {
            // A new message starts; anything half-received is dropped.
            self.id = Some(id);
            self.count = count as u8;
            self.received = 0;
        }
        self.buf[index * CHUNK_DATA..index * CHUNK_DATA + len]
            .copy_from_slice(&report[HEADER..HEADER + len]);
        self.lens[index] = len as u8;
        self.received |= 1 << index;
        if self.received != (1u8 << count) - 1 {
            return None;
        }
        self.id = None;
        let total = (count - 1) * CHUNK_DATA + self.lens[count - 1] as usize;
        Some(HostMessage::Notify(Text::from_host(&self.buf[..total])))
    }
}

/// The notification a half shows: the latest message, until it is cleared
/// or [`NOTIFICATION_MS`] have passed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shown {
    text: Option<Text>,
    until: u32,
}

impl Shown {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            text: None,
            until: 0,
        }
    }

    /// Show (or clear) `message`, received at `now` (milliseconds).
    pub fn apply(&mut self, message: HostMessage, now: u32) {
        match message {
            HostMessage::Notify(text) => {
                self.text = Some(text);
                self.until = now.wrapping_add(NOTIFICATION_MS);
            }
            HostMessage::Clear => self.text = None,
        }
    }

    /// Take the notification down once its time is up.
    pub fn tick(&mut self, now: u32) {
        if self.text.is_some() && now.wrapping_sub(self.until) as i32 >= 0 {
            self.text = None;
        }
    }

    /// What is on the screen now.
    #[must_use]
    pub fn text(&self) -> Option<Text> {
        self.text
    }

    /// When [`tick`](Self::tick) takes it down, while one is shown.
    #[must_use]
    pub fn deadline(&self) -> Option<u32> {
        self.text.map(|_| self.until)
    }
}

/// Message from the left (central) half to the right (peripheral) half.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitMessage {
    Notify(Text),
    Clear,
}

/// Largest encoded [`SplitMessage`].
pub const SPLIT_MAX: usize = 2 + MAX_TEXT;

impl SplitMessage {
    /// Encode as `[kind, length, text...]`; returns the number of bytes used.
    pub fn encode(&self, out: &mut [u8; SPLIT_MAX]) -> usize {
        match self {
            Self::Clear => {
                out[..2].copy_from_slice(&[KIND_CLEAR, 0]);
                2
            }
            Self::Notify(text) => {
                let bytes = text.as_str().as_bytes();
                out[0] = KIND_NOTIFY;
                out[1] = bytes.len() as u8;
                out[2..2 + bytes.len()].copy_from_slice(bytes);
                2 + bytes.len()
            }
        }
    }

    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        match bytes {
            [KIND_CLEAR, ..] => Some(Self::Clear),
            [KIND_NOTIFY, len, rest @ ..] if rest.len() >= *len as usize => {
                Some(Self::Notify(Text::from_host(&rest[..*len as usize])))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reports(text: &str, id: u8) -> Vec<[u8; REPORT_SIZE]> {
        let mut out = Vec::new();
        encode_notification(text, id, |r| out.push(*r));
        out
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "compared with what `feed` returns"
    )]
    fn notify(text: &str) -> Option<HostMessage> {
        Some(HostMessage::Notify(Text::new(text)))
    }

    #[test]
    fn short_text_is_one_report_with_known_bytes() {
        // Arrange: same golden bytes as the .NET HostProtocol tests

        // Act
        let r = reports("Hello", 7);

        // Assert
        assert_eq!(r.len(), 1);
        assert_eq!(
            r[0][..10],
            [0x01, 0x07, 0x00, 0x01, 0x05, b'H', b'e', b'l', b'l', b'o']
        );
        assert!(r[0][10..].iter().all(|&b| b == 0));
    }

    #[test]
    fn long_text_is_split_into_full_chunks() {
        // Arrange
        let text = "x".repeat(60);

        // Act
        let r = reports(&text, 1);

        // Assert: 27 + 27 + 6
        assert_eq!(
            r.iter().map(|c| (c[2], c[3], c[4])).collect::<Vec<_>>(),
            [(0, 3, 27), (1, 3, 27), (2, 3, 6)]
        );
    }

    #[test]
    fn text_longer_than_max_is_cut_at_a_character_boundary() {
        // Arrange
        let text = "é".repeat(100); // 200 bytes

        // Act
        let t = Text::new(&text);

        // Assert
        assert_eq!(t.as_str().len(), MAX_TEXT);
        assert_eq!(t.as_str().chars().count(), 64);
    }

    #[test]
    fn single_report_completes_a_message() {
        // Arrange
        let mut r = Reassembler::new();
        let packet = reports("Build passed", 3)[0];

        // Act
        let msg = r.feed(&packet);

        // Assert
        assert_eq!(msg, notify("Build passed"));
    }

    #[test]
    fn chunks_arriving_out_of_order_are_reassembled() {
        // Arrange
        let text = "Meeting in 5 minutes with the platform team";
        let mut packets = reports(text, 9);
        packets.reverse();
        let mut r = Reassembler::new();
        r.feed(&packets[0]);

        // Act
        let msg = r.feed(&packets[1]);

        // Assert
        assert_eq!(msg, notify(text));
    }

    #[test]
    fn incomplete_message_returns_nothing() {
        // Arrange
        let packets = reports(&"y".repeat(40), 2);
        let mut r = Reassembler::new();

        // Act
        let msg = r.feed(&packets[0]);

        // Assert
        assert_eq!(msg, None);
    }

    #[test]
    fn new_message_id_drops_the_half_received_one() {
        // Arrange
        let mut r = Reassembler::new();
        r.feed(&reports(&"a".repeat(40), 1)[0]);

        // Act
        let msg = r.feed(&reports("b", 2)[0]);

        // Assert
        assert_eq!(msg, notify("b"));
    }

    #[test]
    fn clear_report_clears() {
        // Arrange
        let mut r = Reassembler::new();
        let mut packet = [0u8; REPORT_SIZE];
        packet[0] = KIND_CLEAR;

        // Act
        let msg = r.feed(&packet);

        // Assert
        assert_eq!(msg, Some(HostMessage::Clear));
    }

    #[test]
    fn malformed_report_is_ignored() {
        // Arrange: claims 3 chunks but chunk 0 is short
        let mut r = Reassembler::new();
        let mut packet = [0u8; REPORT_SIZE];
        packet[..5].copy_from_slice(&[KIND_NOTIFY, 1, 0, 3, 4]);

        // Act
        let msg = r.feed(&packet);

        // Assert
        assert_eq!(msg, None);
    }

    #[test]
    fn control_characters_are_taken_out() {
        // Arrange
        let raw = "Build\tpassed\n\x1b[2Jok\x07".as_bytes();

        // Act
        let text = Text::from_host(raw);

        // Assert: tab and newline become spaces, escape and bell go
        assert_eq!(text.as_str(), "Build passed [2Jok");
    }

    #[test]
    fn notification_with_control_characters_arrives_clean() {
        // Arrange
        let mut r = Reassembler::new();
        let packet = reports("line one\nline two", 5)[0];

        // Act
        let msg = r.feed(&packet);

        // Assert
        assert_eq!(msg, notify("line one line two"));
    }

    #[test]
    fn character_split_across_chunks_is_reassembled() {
        // Arrange: "é" (2 bytes) straddles the end of the first 27-byte chunk
        let text = format!("{}é tail", "a".repeat(26));
        let packets = reports(&text, 6);
        let mut r = Reassembler::new();
        r.feed(&packets[0]);

        // Act
        let msg = r.feed(&packets[1]);

        // Assert
        assert_eq!(packets.len(), 2);
        assert_eq!(msg, notify(&text));
    }

    #[test]
    fn invalid_utf8_ends_the_text() {
        // Arrange
        let mut r = Reassembler::new();
        let mut packet = [0u8; REPORT_SIZE];
        packet[..5].copy_from_slice(&[KIND_NOTIFY, 1, 0, 1, 4]);
        packet[5..9].copy_from_slice(&[b'o', b'k', 0xFF, b'x']);

        // Act
        let msg = r.feed(&packet);

        // Assert
        assert_eq!(msg, notify("ok"));
    }

    #[test]
    fn repeated_chunk_does_not_complete_a_message() {
        // Arrange
        let packets = reports(&"z".repeat(40), 3);
        let mut r = Reassembler::new();
        r.feed(&packets[0]);

        // Act
        let msg = r.feed(&packets[0]);

        // Assert
        assert_eq!(msg, None);
    }

    #[test]
    fn chunk_index_past_the_count_is_ignored() {
        // Arrange
        let mut r = Reassembler::new();
        let mut packet = [0u8; REPORT_SIZE];
        packet[..6].copy_from_slice(&[KIND_NOTIFY, 1, 2, 2, 1, b'x']);

        // Act
        let msg = r.feed(&packet);

        // Assert
        assert_eq!(msg, None);
    }

    #[test]
    fn more_chunks_than_fit_are_ignored() {
        // Arrange: 6 chunks would be 162 bytes, more than MAX_TEXT
        let mut r = Reassembler::new();
        let mut packet = [0u8; REPORT_SIZE];
        packet[..6].copy_from_slice(&[KIND_NOTIFY, 1, 5, 6, 1, b'x']);

        // Act
        let msg = r.feed(&packet);

        // Assert
        assert_eq!(msg, None);
    }

    #[test]
    fn short_report_is_ignored() {
        // Arrange
        let mut r = Reassembler::new();

        // Act
        let msg = r.feed(&[KIND_NOTIFY, 1, 0]);

        // Assert
        assert_eq!(msg, None);
    }

    #[test]
    fn empty_notification_is_one_report() {
        // Arrange
        let mut r = Reassembler::new();
        let packet = reports("", 8)[0];

        // Act
        let msg = r.feed(&packet);

        // Assert
        assert_eq!(msg, notify(""));
    }

    #[test]
    fn longest_message_fills_five_chunks() {
        // Arrange
        let text = "m".repeat(MAX_TEXT);
        let mut r = Reassembler::new();
        let packets = reports(&text, 9);
        for p in &packets[..4] {
            r.feed(p);
        }

        // Act
        let msg = r.feed(&packets[4]);

        // Assert
        assert_eq!(packets.len(), MAX_CHUNKS);
        assert_eq!(msg, notify(&text));
    }

    #[test]
    fn shown_notification_goes_after_its_time() {
        // Arrange
        let mut shown = Shown::new();
        shown.apply(HostMessage::Notify(Text::new("hi")), 1_000);

        // Act
        shown.tick(1_000 + NOTIFICATION_MS);

        // Assert
        assert_eq!(shown.text(), None);
    }

    #[test]
    fn shown_notification_stays_until_its_time() {
        // Arrange
        let mut shown = Shown::new();
        shown.apply(HostMessage::Notify(Text::new("hi")), u32::MAX - 10);

        // Act: the clock wraps in between
        shown.tick((u32::MAX - 10).wrapping_add(NOTIFICATION_MS - 1));

        // Assert
        assert_eq!(shown.text(), Some(Text::new("hi")));
    }

    #[test]
    fn clear_takes_the_notification_down() {
        // Arrange
        let mut shown = Shown::new();
        shown.apply(HostMessage::Notify(Text::new("hi")), 0);

        // Act
        shown.apply(HostMessage::Clear, 10);

        // Assert
        assert_eq!(shown.text(), None);
        assert_eq!(shown.deadline(), None);
    }

    #[test]
    fn usb_descriptor_has_32_byte_reports_without_an_id() {
        // Arrange / Act
        let out = crate::mouse::output_report_len(&USB_REPORT_DESCRIPTOR, 0);
        let input = crate::mouse::input_report_len(&USB_REPORT_DESCRIPTOR, 0);

        // Assert
        assert_eq!((out, input), (REPORT_SIZE, REPORT_SIZE));
    }

    #[test]
    fn ble_descriptor_has_a_32_byte_output_report_3() {
        // Arrange / Act
        let out = crate::mouse::output_report_len(&BLE_REPORT_DESCRIPTOR, BLE_REPORT_ID);

        // Assert
        assert_eq!(out, REPORT_SIZE);
    }

    #[test]
    fn descriptors_start_with_the_raw_hid_usage() {
        // Arrange: what kbnotify looks for (Usage Page 0xFF60, Usage 0x61)
        let usage = [0x06, 0x60, 0xFF, 0x09, 0x61];

        // Act
        let starts = (
            USB_REPORT_DESCRIPTOR.starts_with(&usage),
            BLE_REPORT_DESCRIPTOR.starts_with(&usage),
        );

        // Assert
        assert_eq!(starts, (true, true));
    }

    #[test]
    fn split_message_round_trips() {
        // Arrange
        let msg = SplitMessage::Notify(Text::new("CI green ✓"));
        let mut buf = [0u8; SPLIT_MAX];
        let len = msg.encode(&mut buf);

        // Act
        let decoded = SplitMessage::decode(&buf[..len]);

        // Assert
        assert_eq!(decoded, Some(msg));
    }
}
