//! Messages from the computer (`kb_core::host`) on their way through the
//! halves: the left half takes them in (USB Raw HID in the firmware, the
//! Bluetooth Raw HID output report in [`crate::hid`]), shows them in its own
//! status and forwards them over the split link ([`crate::split`]) to the
//! right half, whose screen shows them.
//!
//! Only the latest message matters: a newer one replaces one that hasn't
//! gone out yet.

use core::cell::Cell;

use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant};
use kb_core::host::{HostMessage, NOTIFICATION_MS};

/// The latest value of something, and a wake-up when it changes.
pub struct Mailbox<T: Copy> {
    value: Mutex<CriticalSectionRawMutex, Cell<Option<T>>>,
    changed: Signal<CriticalSectionRawMutex, ()>,
}

impl<T: Copy> Default for Mailbox<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Copy> Mailbox<T> {
    pub const fn new() -> Self {
        Self {
            value: Mutex::new(Cell::new(None)),
            changed: Signal::new(),
        }
    }

    /// Leave `value` (replacing what was there) and wake the reader.
    pub fn put(&self, value: T) {
        self.value.lock(|c| c.set(Some(value)));
        self.changed.signal(());
    }

    /// Take what is there.
    pub fn take(&self) -> Option<T> {
        self.value.lock(Cell::take)
    }

    /// What is there, left in place.
    pub fn peek(&self) -> Option<T> {
        self.value.lock(Cell::get)
    }

    /// Put `value` back unless something newer arrived meanwhile.
    fn put_back(&self, value: T) {
        self.value.lock(|c| {
            if c.get().is_none() {
                c.set(Some(value));
            }
        });
    }

    /// Wait until something is put (returns at once if it was since the last wait).
    pub async fn wait(&self) {
        self.changed.wait().await;
    }
}

/// The left half's messages from the computer.
pub struct Inbox {
    /// For this half's own status (the key loop).
    pub screen: Mailbox<HostMessage>,
    /// For the right half, with when it arrived.
    split: Mailbox<(HostMessage, Instant)>,
}

impl Default for Inbox {
    fn default() -> Self {
        Self::new()
    }
}

impl Inbox {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            screen: Mailbox::new(),
            split: Mailbox::new(),
        }
    }

    /// A whole message arrived from the computer.
    pub fn deliver(&self, message: HostMessage) {
        self.screen.put(message);
        self.split.put((message, Instant::now()));
    }

    /// The message still to go to the right half, if any. One that waited
    /// longer than a notification is shown (the right half was away) is
    /// dropped; a clear always goes.
    pub(crate) fn take_for_split(&self) -> Option<HostMessage> {
        let (message, at) = self.split.take()?;
        let stale = at.elapsed() >= Duration::from_millis(u64::from(NOTIFICATION_MS));
        match message {
            HostMessage::Notify(_) if stale => None,
            _ => Some(message),
        }
    }

    /// A message that couldn't be sent (the link dropped): try again on the
    /// next link, unless a newer one is waiting.
    pub(crate) fn retry_for_split(&self, message: HostMessage) {
        self.split.put_back((message, Instant::now()));
    }

    /// Wait for a new message for the right half.
    pub(crate) async fn wait_for_split(&self) {
        self.split.wait().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kb_core::host::Text;

    #[test]
    fn newer_message_replaces_one_not_yet_sent() {
        // Arrange
        let inbox = Inbox::new();
        inbox.deliver(HostMessage::Notify(Text::new("old")));
        inbox.deliver(HostMessage::Notify(Text::new("new")));

        // Act
        let sent = inbox.take_for_split();

        // Assert
        assert_eq!(sent, Some(HostMessage::Notify(Text::new("new"))));
    }

    #[test]
    fn message_goes_to_the_right_half_once() {
        // Arrange
        let inbox = Inbox::new();
        inbox.deliver(HostMessage::Clear);
        inbox.take_for_split();

        // Act
        let again = inbox.take_for_split();

        // Assert
        assert_eq!(again, None);
    }

    #[test]
    fn retried_message_does_not_replace_a_newer_one() {
        // Arrange
        let inbox = Inbox::new();
        inbox.deliver(HostMessage::Notify(Text::new("newer")));

        // Act
        inbox.retry_for_split(HostMessage::Notify(Text::new("older")));

        // Assert
        assert_eq!(
            inbox.take_for_split(),
            Some(HostMessage::Notify(Text::new("newer")))
        );
    }

    #[test]
    fn screen_gets_the_message_too() {
        // Arrange
        let inbox = Inbox::new();

        // Act
        inbox.deliver(HostMessage::Notify(Text::new("hi")));

        // Assert
        assert_eq!(
            inbox.screen.take(),
            Some(HostMessage::Notify(Text::new("hi")))
        );
    }

    #[test]
    fn peek_leaves_the_message_for_the_reader() {
        // Arrange
        let inbox = Inbox::default();
        inbox.deliver(HostMessage::Clear);

        // Act
        let peeked = inbox.screen.peek();

        // Assert
        assert_eq!(peeked, Some(HostMessage::Clear));
        assert_eq!(inbox.screen.take(), Some(HostMessage::Clear));
    }

    #[test]
    fn waiting_reader_wakes_for_a_message_put_before() {
        // Arrange
        let mailbox = Mailbox::<u8>::default();
        mailbox.put(7);

        // Act
        embassy_futures::block_on(mailbox.wait());

        // Assert
        assert_eq!(mailbox.take(), Some(7));
    }

    #[test]
    fn fresh_clear_goes_to_the_right_half() {
        // Arrange
        let inbox = Inbox::new();
        inbox.deliver(HostMessage::Clear);

        // Act
        let sent = inbox.take_for_split();

        // Assert
        assert_eq!(sent, Some(HostMessage::Clear));
    }

    #[test]
    fn retried_message_goes_out_on_the_next_link() {
        // Arrange
        let inbox = Inbox::new();
        inbox.deliver(HostMessage::Notify(Text::new("hi")));
        let sent = inbox.take_for_split().unwrap();

        // Act
        inbox.retry_for_split(sent);

        // Assert
        assert_eq!(inbox.take_for_split(), Some(sent));
    }
}
