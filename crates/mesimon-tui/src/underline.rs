//! Keep an underline color's components inside one SGR parameter (T-909).
//!
//! Crossterm 0.29 writes `58;2;r;g;b`. Terminal.app treats an unsupported
//! semicolon-form 58 as separate SGR commands: component 46 paints a cyan
//! background, 41 a red one. Ratatui never requested those backgrounds,
//! so its diff cannot restore them when an animation finishes. The grouped
//! `58:2::r:g:b` / `58:5:n` groups the components for capable terminals.
//! Terminal.app also misparses that form, losing the existing text style
//! (confirmed by a second, non-default-color diagnostic). Omit custom
//! underline colors there: ordinary underlines inherit the text color.

use std::io::{self, Write};

/// Wrap only the renderer's output, not arbitrary agent or shell output.
/// `write_fmt` can split a command anywhere, including between ESC and [.
pub(crate) struct UnderlineColors<W> {
    inner: W,
    pending: Vec<u8>,
    colored: bool,
}

impl<W: Write> UnderlineColors<W> {
    pub(crate) fn new(inner: W, terminal_program: Option<&str>) -> Self {
        Self {
            inner,
            pending: Vec::with_capacity(24),
            // RGB text support does not imply support for SGR 58.
            colored: terminal_program != Some("Apple_Terminal"),
        }
    }

    fn emit(&mut self) -> io::Result<()> {
        // Only crossterm's two complete underline-color forms are changed.
        // Every other command, text byte and reset passes through exactly.
        if let Some(params) =
            self.pending.strip_prefix(b"\x1b[58;").and_then(|p| p.strip_suffix(b"m"))
        {
            let parts: Vec<_> = params.split(|b| *b == b';').collect();
            let components = |parts: &[&[u8]]| {
                parts.iter().all(|p| {
                    !p.is_empty()
                        && p.iter().all(u8::is_ascii_digit)
                        && std::str::from_utf8(p).ok().and_then(|p| p.parse::<u8>().ok()).is_some()
                })
            };
            let rgb = parts.len() == 4 && parts[0] == b"2" && components(&parts[1..]);
            let indexed = parts.len() == 2 && parts[0] == b"5" && components(&parts[1..]);
            if rgb || indexed {
                if !self.colored {
                    self.pending.clear();
                    return Ok(());
                }
                self.inner.write_all(if rgb { b"\x1b[58:2:" } else { b"\x1b[58:5" })?;
                for p in &parts[1..] {
                    self.inner.write_all(b":")?;
                    self.inner.write_all(p)?;
                }
                self.inner.write_all(b"m")?;
                self.pending.clear();
                return Ok(());
            }
        }
        self.inner.write_all(&self.pending)?;
        self.pending.clear();
        Ok(())
    }
}

impl<W: Write> Write for UnderlineColors<W> {
    fn write(&mut self, mut bytes: &[u8]) -> io::Result<usize> {
        let len = bytes.len();
        while !bytes.is_empty() {
            if self.pending.is_empty() {
                let n = bytes.iter().position(|b| *b == 0x1b).unwrap_or(bytes.len());
                self.inner.write_all(&bytes[..n])?;
                bytes = &bytes[n..];
                if bytes.is_empty() {
                    break;
                }
            }
            let b = bytes[0];
            bytes = &bytes[1..];
            // An interrupted escape must not hide the start of the next.
            if b == 0x1b && !self.pending.is_empty() {
                self.emit()?;
            }
            self.pending.push(b);
            const PREFIX: &[u8] = b"\x1b[58;";
            if self.pending.len() <= PREFIX.len() {
                if !PREFIX.starts_with(&self.pending) {
                    self.emit()?;
                }
            } else if (!b.is_ascii_digit() && b != b';') || self.pending.len() >= 32 {
                self.emit()?;
            }
        }
        Ok(len)
    }

    fn flush(&mut self) -> io::Result<()> {
        // A real flush must not retain bytes. Crossterm flushes between
        // complete commands, never inside their write_fmt calls.
        self.emit()?;
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A legacy SGR reader that supports RGB backgrounds but ignores 58.
    /// Colons group a single parameter; semicolons begin another command.
    /// This models the misparse, not Terminal.app's rendering implementation.
    fn background_and_dim(bytes: &[u8]) -> ((u8, u8, u8), bool) {
        let text = std::str::from_utf8(bytes).unwrap();
        let mut bg = (0, 0, 0);
        let mut dim = false;
        for seq in text.split("\x1b[").skip(1) {
            let Some((params, _)) = seq.split_once('m') else { continue };
            let parts: Vec<_> = params.split(';').collect();
            let mut at = 0;
            while at < parts.len() {
                match parts[at] {
                    "0" => {
                        bg = (0, 0, 0);
                        dim = false;
                    }
                    "2" => dim = true,
                    "48" if parts.get(at + 1) == Some(&"2") => {
                        bg = (
                            parts[at + 2].parse().unwrap(),
                            parts[at + 3].parse().unwrap(),
                            parts[at + 4].parse().unwrap(),
                        );
                        at += 4;
                    }
                    // The two colors visible in the reported screenshot.
                    "41" => bg = (255, 0, 0),
                    "46" => bg = (0, 255, 255),
                    _ => {}
                }
                at += 1;
            }
        }
        (bg, dim)
    }

    #[test]
    fn an_ignored_underline_never_paints_a_background_or_dims_the_text() {
        for (component, polluted) in [(41, (255, 0, 0)), (46, (0, 255, 255))] {
            let before =
                format!("\x1b[0m\x1b[48;2;30;30;30m\x1b[58;2;{component};{component};{component}m");
            assert_eq!(
                background_and_dim(before.as_bytes()),
                (polluted, true),
                "reproduces the old stray background"
            );
            let mut out = UnderlineColors::new(Vec::new(), None);
            out.write_all(before.as_bytes()).unwrap();
            out.flush().unwrap();
            assert_eq!(background_and_dim(&out.inner), ((30, 30, 30), false));
        }
        // Indexed underline colors used to become blink (5) and a second
        // command too; a palette index must never become a background.
        let mut out = UnderlineColors::new(Vec::new(), None);
        out.write_all(b"\x1b[0m\x1b[48;2;30;30;30m\x1b[58;5;46m").unwrap();
        out.flush().unwrap();
        assert_eq!(background_and_dim(&out.inner), ((30, 30, 30), false));
    }

    #[test]
    fn only_underline_colors_are_grouped_at_every_write_boundary() {
        let before = "héllo\x1b[38;2;41;46;107m\x1b[48;5;42m\x1b[4m\x1b[58;2;41;46;107m✓\x1b[58;5;9m!\x1b[59m\x1b[0m\x1b[2;3H\x1b[?2026l";
        let after = "héllo\x1b[38;2;41;46;107m\x1b[48;5;42m\x1b[4m\x1b[58:2::41:46:107m✓\x1b[58:5:9m!\x1b[59m\x1b[0m\x1b[2;3H\x1b[?2026l";
        for split in 0..=before.len() {
            let mut out = UnderlineColors::new(Vec::new(), None);
            out.write_all(&before.as_bytes()[..split]).unwrap();
            out.write_all(&before.as_bytes()[split..]).unwrap();
            out.flush().unwrap();
            assert_eq!(out.inner, after.as_bytes(), "split {split}");
        }
        let mut out = UnderlineColors::new(Vec::new(), None);
        for b in before.bytes() {
            out.write_all(&[b]).unwrap();
        }
        out.flush().unwrap();
        assert_eq!(out.inner, after.as_bytes());
    }

    #[test]
    fn apple_terminal_omits_only_custom_underline_colors_at_every_write_boundary() {
        let before = b"\x1b[38;2;80;200;230m\x1b[48;2;65;40;75m\x1b[1m\x1b[4mBEFORE\x1b[58;2;41;46;107mAFTER\x1b[58;5;9m!\x1b[59m\x1b[24m\x1b[0m";
        let after = b"\x1b[38;2;80;200;230m\x1b[48;2;65;40;75m\x1b[1m\x1b[4mBEFOREAFTER!\x1b[59m\x1b[24m\x1b[0m";
        for split in 0..=before.len() {
            let mut out = UnderlineColors::new(Vec::new(), Some("Apple_Terminal"));
            out.write_all(&before[..split]).unwrap();
            out.write_all(&before[split..]).unwrap();
            out.flush().unwrap();
            assert_eq!(out.inner, after, "split {split}");
        }
        let mut out = UnderlineColors::new(Vec::new(), Some("Apple_Terminal"));
        for b in before {
            out.write_all(&[*b]).unwrap();
        }
        out.flush().unwrap();
        assert_eq!(out.inner, after);
        // The endpoint decides, not a global downgrade or a color profile.
        for program in [None, Some("iTerm.app"), Some("ghostty"), Some("tmux")] {
            let mut out = UnderlineColors::new(Vec::new(), program);
            out.write_all(b"\x1b[58;2;41;46;107m").unwrap();
            out.flush().unwrap();
            assert_eq!(out.inner, b"\x1b[58:2::41:46:107m", "{program:?}");
        }
    }

    #[test]
    fn other_forms_and_incomplete_commands_are_preserved() {
        for bytes in [
            "\x1b[58:2::41:46:107m",
            "\x1b[58;2;256;1;2m",
            "\x1b[58;2;1;2m",
            "\x1b[58;5;2;4m",
            "\x1b[58;5;m",
            "\x1b[58;1m",
            "\x1b[58;5;9H",
            "\x1b[58;2;00000000000000000000000000000000000000m",
            "\x1b[58;2;41",
        ] {
            let mut out = UnderlineColors::new(Vec::new(), None);
            out.write_all(bytes.as_bytes()).unwrap();
            out.flush().unwrap();
            assert_eq!(out.inner, bytes.as_bytes());
        }
        let mut out = UnderlineColors::new(Vec::new(), None);
        out.write_all(b"\x1b\x1b[58;5;41m").unwrap();
        out.flush().unwrap();
        assert_eq!(out.inner, b"\x1b\x1b[58:5:41m");
    }

    #[test]
    fn short_writes_are_completed_and_errors_propagate() {
        struct Short(Vec<u8>);
        impl Write for Short {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                let n = bytes.len().min(1);
                self.0.extend_from_slice(&bytes[..n]);
                Ok(n)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut out = UnderlineColors::new(Short(Vec::new()), None);
        out.write_all(b"\x1b[58;2;41;46;107mtext").unwrap();
        out.flush().unwrap();
        assert_eq!(out.inner.0, b"\x1b[58:2::41:46:107mtext");
        let mut storage = [0u8; 1];
        let mut out = UnderlineColors::new(storage.as_mut_slice(), None);
        assert!(out.write_all(b"\x1b[58;5;41m").is_err());
    }
}
