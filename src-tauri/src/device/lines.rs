//! Device output → plain text lines (shared by the observer and the identity probe).
//!
//! Removes ANSI escape sequences (also when split across chunks), handles CR (a CR not
//! followed by LF overwrites the line, as `--More--` erasing does) and backspace, drops other
//! control characters and caps the current line.

const MAX_LINE: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Esc {
    None,
    /// After ESC.
    Start,
    /// CSI `ESC [` … final byte.
    Csi,
    /// OSC `ESC ]` … BEL or ST.
    Osc,
    OscEnd,
}

#[derive(Debug)]
pub struct LineAssembler {
    esc: Esc,
    cr: bool,
    line: String,
}

impl Default for LineAssembler {
    fn default() -> Self {
        Self::new()
    }
}

impl LineAssembler {
    pub fn new() -> Self {
        Self {
            esc: Esc::None,
            cr: false,
            line: String::new(),
        }
    }

    /// The incomplete last line.
    pub fn current(&self) -> &str {
        &self.line
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Feeds raw bytes; `on_line` gets every completed line.
    pub fn feed(&mut self, bytes: &[u8], mut on_line: impl FnMut(String)) {
        for ch in String::from_utf8_lossy(bytes).chars() {
            self.feed_char(ch, &mut on_line);
        }
    }

    fn feed_char(&mut self, ch: char, on_line: &mut impl FnMut(String)) {
        match self.esc {
            Esc::Start => {
                self.esc = match ch {
                    '[' => Esc::Csi,
                    ']' => Esc::Osc,
                    _ => Esc::None, // two-character escape
                };
                return;
            }
            Esc::Csi => {
                if ('@'..='~').contains(&ch) {
                    self.esc = Esc::None;
                }
                return;
            }
            Esc::Osc => {
                match ch {
                    '\x07' => self.esc = Esc::None,
                    '\x1b' => self.esc = Esc::OscEnd,
                    _ => {}
                }
                return;
            }
            Esc::OscEnd => {
                self.esc = if ch == '\\' { Esc::None } else { Esc::Osc };
                return;
            }
            Esc::None => {}
        }
        if self.cr {
            self.cr = false;
            if ch != '\n' {
                self.line.clear(); // carriage return without line feed: overwrite
            }
        }
        match ch {
            '\x1b' => self.esc = Esc::Start,
            '\n' => on_line(std::mem::take(&mut self.line)),
            '\r' => self.cr = true,
            '\x08' => {
                self.line.pop();
            }
            c if c.is_control() => {}
            c => {
                if self.line.len() < MAX_LINE {
                    self.line.push(c);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_escapes_and_applies_cr_and_backspace() {
        let mut a = LineAssembler::new();
        let mut lines = Vec::new();
        a.feed(
            b"\x1b[1mbold\x1b[0m\r\n --More-- \r     \rnext\x08t\r\n\x1b]0;title\x07x",
            |l| lines.push(l),
        );
        assert_eq!(lines, ["bold", "next"]);
        assert_eq!(a.current(), "x");
        a.feed(&vec![b'y'; 5000], |_| {});
        assert!(a.current().len() <= MAX_LINE);
    }
}
