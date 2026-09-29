//! The Remote Control dialog's scan-to-pair QR (T-497): the page a phone's
//! camera opens, with the pairing code in its fragment, drawn in half-block
//! cells. Two modules to a cell, the upper and the lower, so a module is
//! square in a terminal's 1:2 cell, and a light quiet zone on every side.
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Style};

/// The quiet zone, in modules. The standard asks four; two is what terminal
/// QR printers draw, and a phone camera reads it off a screen.
const QUIET: i32 = 2;

pub(crate) struct Qr(qrcodegen::QrCode);

impl Qr {
    /// The smallest symbol that holds `text`, its error correction raised as
    /// far as that size allows. `None` past what a QR code can hold.
    pub(crate) fn encode(text: &str) -> Option<Self> {
        qrcodegen::QrCode::encode_text(text, qrcodegen::QrCodeEcc::Low).ok().map(Self)
    }

    /// The columns and rows the code takes, quiet zone included.
    pub(crate) fn cells(&self) -> (u16, u16) {
        let side = self.0.size() + 2 * QUIET;
        (side as u16, ((side + 1) / 2) as u16)
    }

    /// Whether the module at `(x, y)` is dark, counted from the quiet zone's
    /// corner. Everything outside the symbol is light.
    fn dark(&self, x: i32, y: i32) -> bool {
        self.0.get_module(x - QUIET, y - QUIET)
    }

    /// Paint the code with its top-left cell at `(x, y)`: a cell whose two
    /// modules agree is a space on that ink, and one whose modules differ is
    /// `▀` in the upper module's ink on the lower's. A cell outside `buf`
    /// is skipped.
    pub(crate) fn paint(&self, buf: &mut Buffer, (x, y): (u16, u16), dark: Color, light: Color) {
        let ink = |d: bool| if d { dark } else { light };
        let (w, h) = self.cells();
        for r in 0..h {
            for c in 0..w {
                let (upper, lower) =
                    (self.dark(c as i32, 2 * r as i32), self.dark(c as i32, 2 * r as i32 + 1));
                let Some(cell) = buf.cell_mut((x + c, y + r)) else { continue };
                if upper == lower {
                    cell.set_char(' ').set_style(Style::default().bg(ink(upper)));
                } else {
                    cell.set_char('▀').set_style(Style::default().fg(ink(upper)).bg(ink(lower)));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    const LINK: &str = "https://remote.mesimon.dev/#pair=7K2M-QX4P-0B9D-RT6W-HN3C-5VJE-8FGA-1YSZ";

    /// The hosted page's link is a version-4 symbol: 33 modules and a quiet
    /// zone of two on each side, 37 columns by 19 rows.
    #[test]
    fn the_hosted_pairing_link_is_a_37_by_19_cell_code() {
        let qr = Qr::encode(LINK).expect("fits");
        assert_eq!(qr.0.version(), qrcodegen::Version::new(4));
        assert_eq!(qr.cells(), (37, 19));
    }

    /// Cells read back as the modules they were painted from: the quiet
    /// zone light, a finder pattern's dark corner where the standard puts it,
    /// and the extra half row below an odd side light.
    #[test]
    fn painted_cells_carry_the_modules_upper_over_lower() {
        let qr = Qr::encode(LINK).unwrap();
        let (w, h) = qr.cells();
        let (dark, light) = (Color::Rgb(0, 0, 0), Color::Rgb(255, 255, 255));
        let mut buf = Buffer::empty(Rect::new(0, 0, w + 2, h + 2));
        qr.paint(&mut buf, (1, 1), dark, light);
        let module = |mx: i32, my: i32| {
            let cell = &buf[(1 + mx as u16, 1 + (my / 2) as u16)];
            let upper = if cell.symbol() == "▀" { cell.fg } else { cell.bg };
            if my % 2 == 0 {
                upper == dark
            } else {
                cell.bg == dark
            }
        };
        for i in 0..w as i32 {
            for q in 0..QUIET {
                assert!(!module(i, q) && !module(q, i), "quiet zone at {i},{q}");
            }
        }
        // The top-left finder: a dark 7x7 ring, then a light ring, then the
        // dark 3x3 core.
        assert!(module(QUIET, QUIET) && module(QUIET + 6, QUIET + 6));
        assert!(!module(QUIET + 1, QUIET + 1) && module(QUIET + 3, QUIET + 3));
        for x in 0..w as i32 {
            assert!(!module(x, 2 * h as i32 - 1), "the half row past the side is light");
        }
        for y in 0..h {
            for x in 0..w {
                assert!(matches!(buf[(1 + x, 1 + y)].symbol(), " " | "▀"));
            }
        }
        assert_eq!(buf[(0, 0)].symbol(), " ", "nothing painted outside the code");
        assert_eq!(buf[(0, 0)].bg, Color::Reset);
    }
}
