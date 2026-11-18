//! Tiny fixed-size string buffer, for formatting without an allocator.

use core::fmt;

pub struct StrBuf<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> StrBuf<N> {
    pub fn new() -> Self {
        Self {
            buf: [0; N],
            len: 0,
        }
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn char_count(&self) -> usize {
        self.as_str().chars().count()
    }

    pub fn as_str(&self) -> &str {
        // Only whole UTF-8 strs are ever appended.
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}

impl<const N: usize> fmt::Write for StrBuf<N> {
    /// Appends as much as fits; never splits a character.
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            let mut tmp = [0u8; 4];
            let bytes = c.encode_utf8(&mut tmp).as_bytes();
            if self.len + bytes.len() > N {
                return Ok(());
            }
            self.buf[self.len..self.len + bytes.len()].copy_from_slice(bytes);
            self.len += bytes.len();
        }
        Ok(())
    }
}

/// Word-wrap `s` into lines of at most `width` characters (`width` <= 16),
/// calling `line(index, text)` for at most `max_lines` lines. Words longer
/// than a line are broken. Returns the number of lines produced.
pub fn wrap(s: &str, width: usize, max_lines: usize, mut line: impl FnMut(usize, &str)) -> usize {
    use core::fmt::Write;
    let mut current = StrBuf::<64>::new();
    let mut lines = 0;
    let mut emit = |current: &mut StrBuf<64>, lines: &mut usize| {
        if *lines < max_lines {
            line(*lines, current.as_str());
        }
        *lines += 1;
        current.clear();
    };
    for word in s.split_whitespace() {
        let mut word = word;
        let needed = word.chars().count() + usize::from(!current.is_empty());
        if current.char_count() + needed > width && !current.is_empty() {
            emit(&mut current, &mut lines);
        }
        // Break words that are longer than a line.
        while word.chars().count() > width {
            let split = word
                .char_indices()
                .nth(width)
                .map_or(word.len(), |(i, _)| i);
            if !current.is_empty() {
                emit(&mut current, &mut lines);
            }
            let _ = current.write_str(&word[..split]);
            emit(&mut current, &mut lines);
            word = &word[split..];
        }
        if !word.is_empty() {
            if !current.is_empty() {
                let _ = current.write_char(' ');
            }
            let _ = current.write_str(word);
        }
    }
    if !current.is_empty() {
        emit(&mut current, &mut lines);
    }
    lines.min(max_lines)
}

#[cfg(test)]
mod tests {
    use super::wrap;

    fn wrapped(s: &str, width: usize, max: usize) -> Vec<String> {
        let mut out = Vec::new();
        wrap(s, width, max, |_, l| out.push(l.to_string()));
        out
    }

    #[test]
    fn wrap_breaks_between_words() {
        // Arrange
        let text = "Build ok on main";

        // Act
        let lines = wrapped(text, 5, 10);

        // Assert
        assert_eq!(lines, ["Build", "ok on", "main"]);
    }

    #[test]
    fn wrap_breaks_long_words() {
        // Arrange
        let text = "deployment";

        // Act
        let lines = wrapped(text, 5, 10);

        // Assert
        assert_eq!(lines, ["deplo", "yment"]);
    }

    #[test]
    fn wrap_stops_at_max_lines() {
        // Arrange
        let text = "a b c d e f g h";

        // Act
        let lines = wrapped(text, 1, 3);

        // Assert
        assert_eq!(lines, ["a", "b", "c"]);
    }
}
