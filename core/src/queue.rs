//! Fixed-capacity FIFO, so the engine needs no allocator.

pub struct Queue<T: Copy, const C: usize> {
    buf: [Option<T>; C],
    head: usize,
    len: usize,
}

impl<T: Copy, const C: usize> Queue<T, C> {
    pub const fn new() -> Self {
        Self {
            buf: [None; C],
            head: 0,
            len: 0,
        }
    }

    /// Returns `false` (and drops the item) when full.
    pub fn push(&mut self, item: T) -> bool {
        if self.len == C {
            return false;
        }
        self.buf[(self.head + self.len) % C] = Some(item);
        self.len += 1;
        true
    }

    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        let item = self.buf[self.head].take();
        self.head = (self.head + 1) % C;
        self.len -= 1;
        item
    }
}

#[cfg(test)]
mod tests {
    use super::Queue;

    #[test]
    fn pop_returns_items_in_push_order() {
        // Arrange
        let mut q: Queue<u8, 4> = Queue::new();
        q.push(1);
        q.push(2);

        // Act
        let popped = [q.pop(), q.pop(), q.pop()];

        // Assert
        assert_eq!(popped, [Some(1), Some(2), None]);
    }

    #[test]
    fn push_into_full_queue_is_rejected() {
        // Arrange
        let mut q: Queue<u8, 2> = Queue::new();
        q.push(1);
        q.push(2);

        // Act
        let accepted = q.push(3);

        // Assert
        assert!(!accepted);
        assert_eq!(q.pop(), Some(1));
    }

    #[test]
    fn queue_keeps_order_across_wraparound() {
        // Arrange
        let mut q: Queue<u8, 2> = Queue::new();
        q.push(1);
        q.push(2);
        q.pop();

        // Act
        q.push(3);

        // Assert
        assert_eq!([q.pop(), q.pop()], [Some(2), Some(3)]);
    }
}
