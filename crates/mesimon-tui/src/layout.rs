//! Board geometry (07 §2 owns; 06 §6.3's competing formula is superseded per
//! STALE-MAP §D33i, and the attention rail's RAIL=3 term is gone per D33k).
//! Pure arithmetic over cell counts — no ratatui, fully table-tested.
//!
//! Model: `[LPAD][slot][GUT][slot]…[RPAD]`, where a slot is either an
//! expanded column (width in [MIN_COL, MAX_COL]-ish, see clamp) or a 1-cell
//! spine. The cursor's column is always expanded; a contiguous window of
//! expanded columns slides to contain it, and collapsed columns pin to the
//! edge they fell off (07 §3.1).

pub(crate) const GUT: u16 = 1;
pub(crate) const LPAD: u16 = 1;
pub(crate) const RPAD: u16 = 1;
/// Below this width a column refuses to expand (07 §2.1: T = 23 + G, G = 0
/// under D33g, card frame = 3 → MIN_COL = 26).
pub(crate) const MIN_COL: u16 = 26;
/// Columns stop absorbing width past this (07 §2.3 clamp).
pub(crate) const MAX_COL: u16 = 40;
pub(crate) const SPINE: u16 = 1;
/// Hard floor (07 §2.4): below this the board renders a notice, not a layout.
pub(crate) const MIN_W: u16 = 60;
pub(crate) const MIN_H: u16 = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Slot {
    /// Expanded column: `x` is the accent-bar cell; `width` includes it.
    Expanded { x: u16, width: u16 },
    /// Collapsed 1-cell vertical spine.
    Spine { x: u16 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BoardGeometry {
    /// One slot per board column, in board order.
    pub slots: Vec<Slot>,
    /// Index of the first expanded column (the window's left edge).
    pub window: usize,
    /// Number of expanded columns.
    pub visible: usize,
}

/// How many columns fit expanded at width `w` out of `n_total`, and the
/// per-column widths. Returns (n_visible, widths for the visible columns).
fn fit(w: u16, n_total: usize) -> (usize, Vec<u16>) {
    debug_assert!(n_total > 0);
    for n_vis in (1..=n_total).rev() {
        let spines = (n_total - n_vis) as u16;
        let fixed = LPAD + RPAD + (n_vis as u16 - 1) * GUT + spines * (SPINE + GUT);
        let Some(avail) = w.checked_sub(fixed) else { continue };
        let base = avail / n_vis as u16;
        if base < MIN_COL && n_vis > 1 {
            continue;
        }
        // Clamp then hand out the remainder one cell at a time, leftmost first.
        let base = base.min(MAX_COL);
        let mut widths = vec![base; n_vis];
        let mut leftover = avail - base * n_vis as u16;
        for wd in widths.iter_mut() {
            if leftover == 0 || *wd >= MAX_COL {
                continue;
            }
            *wd += 1;
            leftover -= 1;
        }
        return (n_vis, widths);
    }
    (1, vec![w.saturating_sub(LPAD + RPAD).min(MAX_COL)])
}

/// Compute the board geometry. `window` persists across frames in App and is
/// slid here so the cursor's column is always expanded.
pub(crate) fn board_geometry(
    w: u16,
    n_total: usize,
    cursor_col: usize,
    window: &mut usize,
) -> BoardGeometry {
    if n_total == 0 {
        return BoardGeometry { slots: vec![], window: 0, visible: 0 };
    }
    let (visible, widths) = fit(w, n_total);
    // Slide the window to contain the cursor.
    *window = (*window).min(n_total - visible);
    if cursor_col < *window {
        *window = cursor_col;
    } else if cursor_col >= *window + visible {
        *window = cursor_col + 1 - visible;
    }

    let mut slots = Vec::with_capacity(n_total);
    let mut x = LPAD;
    for col in 0..n_total {
        if col < *window || col >= *window + visible {
            slots.push(Slot::Spine { x });
            x += SPINE + GUT;
        } else {
            let width = widths[col - *window];
            slots.push(Slot::Expanded { x, width });
            x += width + GUT;
        }
    }
    BoardGeometry { slots, window: *window, visible }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_board_at_120() {
        // D33i's published number: 4 columns at 120 → T floor 25.
        let mut win = 0;
        let g = board_geometry(120, 4, 0, &mut win);
        assert_eq!(g.visible, 4);
        let widths: Vec<u16> = g
            .slots
            .iter()
            .map(|s| match s {
                Slot::Expanded { width, .. } => *width,
                Slot::Spine { .. } => panic!("no spines at 120"),
            })
            .collect();
        assert_eq!(widths, vec![29, 29, 29, 28]); // T = 26,26,26,25
        // Total consumed = pads + gutters + widths == 120.
        let total: u16 = LPAD + RPAD + 3 * GUT + widths.iter().sum::<u16>();
        assert_eq!(total, 120);
    }

    #[test]
    fn default_board_at_100_grows_one_spine() {
        // 4×MIN_COL doesn't fit at 100 — the far column collapses.
        let mut win = 0;
        let g = board_geometry(100, 4, 0, &mut win);
        assert_eq!(g.visible, 3);
        assert!(matches!(g.slots[3], Slot::Spine { .. }));
        assert!(g.slots[..3].iter().all(|s| matches!(s, Slot::Expanded { .. })));
    }

    #[test]
    fn window_slides_to_cursor_and_spines_swap_edges() {
        let mut win = 0;
        let g = board_geometry(100, 4, 3, &mut win);
        assert_eq!(g.window, 1);
        assert!(matches!(g.slots[0], Slot::Spine { x } if x == LPAD));
        assert!(g.slots[1..].iter().all(|s| matches!(s, Slot::Expanded { .. })));
        // Sliding back left restores the original window.
        let g = board_geometry(100, 4, 0, &mut win);
        assert_eq!(g.window, 0);
        assert!(matches!(g.slots[3], Slot::Spine { .. }));
    }

    #[test]
    fn window_is_sticky_between_edges() {
        // Cursor inside the window does not move it.
        let mut win = 1;
        let g = board_geometry(100, 4, 2, &mut win);
        assert_eq!(g.window, 1);
        assert!(matches!(g.slots[0], Slot::Spine { .. }));
        assert!(matches!(g.slots[3], Slot::Spine { .. }) || g.visible == 3);
    }

    #[test]
    fn wide_terminals_clamp_column_width() {
        let mut win = 0;
        let g = board_geometry(300, 4, 0, &mut win);
        for s in &g.slots {
            if let Slot::Expanded { width, .. } = s {
                assert!(*width <= MAX_COL);
            }
        }
    }

    #[test]
    fn single_column_survives_any_width() {
        let mut win = 0;
        let g = board_geometry(60, 1, 0, &mut win);
        assert_eq!(g.visible, 1);
        assert!(matches!(g.slots[0], Slot::Expanded { .. }));
    }

    #[test]
    fn many_columns_at_the_floor() {
        // 8 columns at 60 wide: at least one expanded, spines for the rest,
        // everything within bounds.
        let mut win = 0;
        for cursor in 0..8 {
            let g = board_geometry(60, 8, cursor, &mut win);
            assert!(g.visible >= 1);
            assert!(matches!(g.slots[cursor], Slot::Expanded { .. }), "cursor col expanded");
            // No slot may run past the right pad.
            for s in &g.slots {
                let end = match s {
                    Slot::Expanded { x, width } => x + width,
                    Slot::Spine { x } => x + SPINE,
                };
                assert!(end <= 60 - RPAD);
            }
        }
    }

    #[test]
    fn empty_board() {
        let mut win = 0;
        let g = board_geometry(120, 0, 0, &mut win);
        assert!(g.slots.is_empty());
    }
}
