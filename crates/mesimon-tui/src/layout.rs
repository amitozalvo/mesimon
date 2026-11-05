//! Board geometry (07 §2 owns; 06 §6.3's competing formula is superseded per
//! STALE-MAP §D33i, and the attention rail's RAIL=3 term is gone per D33k).
//! Pure arithmetic over cell counts — no ratatui, fully table-tested.
//!
//! Model: `[LPAD][slot][GUT][slot]…[RPAD]`, where a slot is either an
//! expanded column (width in [MIN_COL, its size's ceiling]) or a 1-cell
//! spine. The cursor's column is always expanded; a contiguous window of
//! expanded columns slides to contain it, and collapsed columns pin to the
//! edge they fell off (07 §3.1). A column PINNED collapsed (its `collapsed`
//! setting, T-117) is a spine at any width unless it holds the cursor or a
//! pending move's ghost (T-1133); the window runs over the live columns only.
//!
//! Widths (T-1135): every open column gets `MIN_COL`, and the slack above
//! that is shared by size — S none, M one share, L two, XL four — each
//! column stopping at its size's ceiling. All M is the equal split the
//! board had before sizes existed, cell for cell.

use mesimon_core::board::ColumnWidth;

pub(crate) const GUT: u16 = 1;
pub(crate) const LPAD: u16 = 1;
pub(crate) const RPAD: u16 = 1;
/// Below this width a column refuses to expand (07 §2.1: T = 23 + G, G = 0
/// under D33g, card frame = 3 → MIN_COL = 26).
pub(crate) const MIN_COL: u16 = 26;
/// An M column stops absorbing width past this (07 §2.3 clamp); the other
/// sizes have their own ceiling in `cap`.
pub(crate) const MAX_COL: u16 = 40;
pub(crate) const SPINE: u16 = 1;
/// A spine breathes: one extra cell on EACH side of it beyond the gutter,
/// so its letters never sit flush against the previous column's pad or the
/// next column's bar (user 2026-09-06, "collapsed columns should leave a gap
/// before next / previous column"). A spine slot costs `SPINE + GUT +
/// 2 * SPINE_GAP` cells.
pub(crate) const SPINE_GAP: u16 = 1;
/// Hard floor (07 §2.4): below this the board renders a notice, not a layout.
pub(crate) const MIN_W: u16 = 60;
pub(crate) const MIN_H: u16 = 20;

/// A size's share of the slack above the floor.
fn share(size: ColumnWidth) -> u16 {
    match size {
        ColumnWidth::S => 0,
        ColumnWidth::M => 1,
        ColumnWidth::L => 2,
        ColumnWidth::Xl => 4,
    }
}

/// A size's ceiling: what a wide terminal may stretch the column to.
fn cap(size: ColumnWidth) -> u16 {
    match size {
        ColumnWidth::S => MIN_COL,
        ColumnWidth::M => MAX_COL,
        ColumnWidth::L => 60,
        ColumnWidth::Xl => 90,
    }
}

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

/// Split `avail` cells among the open columns by size: the floor to each,
/// then the slack by share, a column that would pass its ceiling stopping
/// there and handing the rest on. The cells the floors of the floor
/// divisions leave go leftmost first. `None` when the floors do not fit —
/// bar one column alone, which takes what there is.
fn split(avail: u16, sizes: &[ColumnWidth]) -> Option<Vec<u16>> {
    let n = sizes.len();
    if n == 1 {
        return Some(vec![avail.min(cap(sizes[0]))]);
    }
    let floors = MIN_COL * n as u16;
    let mut pool = avail.checked_sub(floors)?;
    let mut widths = vec![MIN_COL; n];
    let mut capped: Vec<bool> = sizes.iter().map(|&s| share(s) == 0).collect();
    loop {
        let open: u32 =
            sizes.iter().zip(&capped).filter(|(_, &c)| !c).map(|(&s, _)| share(s) as u32).sum();
        if open == 0 {
            return Some(widths);
        }
        let slack = pool as u32;
        let want = move |s: ColumnWidth| MIN_COL + (slack * share(s) as u32 / open) as u16;
        let mut changed = false;
        for (i, &s) in sizes.iter().enumerate() {
            if !capped[i] && want(s) >= cap(s) {
                widths[i] = cap(s);
                pool -= cap(s) - MIN_COL;
                capped[i] = true;
                changed = true;
            }
        }
        if changed {
            continue;
        }
        for (i, &s) in sizes.iter().enumerate() {
            if !capped[i] {
                widths[i] = want(s);
            }
        }
        let mut leftover = avail - widths.iter().sum::<u16>();
        for (i, &s) in sizes.iter().enumerate() {
            if leftover == 0 {
                break;
            }
            if !capped[i] && widths[i] < cap(s) {
                widths[i] += 1;
                leftover -= 1;
            }
        }
        return Some(widths);
    }
}

/// Compute the board geometry. `window` persists across frames in App and is
/// slid here so the cursor's column is always expanded. `pinned[col]` says
/// the column is a spine unless it holds the cursor (T-117); `window` is an
/// index into the LIVE columns — the unpinned ones plus the cursor's — and
/// the clamp absorbs a pin set that changed under it.
///
/// `target` is a pending move's column (T-1133, user: "open collapsed column
/// when a ticket is pending move into there"): it is expanded beside the
/// cursor's for as long as the ghost stands in it, pinned or squeezed out.
/// It is always the cursor's neighbour, so one slide more keeps both; when
/// only one column fits, the ghost's wins — it is where the eye has to be.
///
/// `sizes[col]` is the column's width word (T-1135), M where absent; the
/// caller puts XL on the cursor's column when the cursor-grow switch is on.
/// A column falls off the window when the floors no longer fit, whatever
/// its size: a size moves slack, never the fold.
pub(crate) fn board_geometry(
    w: u16,
    n_total: usize,
    cursor_col: usize,
    target: Option<usize>,
    pinned: &[bool],
    sizes: &[ColumnWidth],
    window: &mut usize,
) -> BoardGeometry {
    if n_total == 0 {
        return BoardGeometry { slots: vec![], window: 0, visible: 0 };
    }
    let live: Vec<usize> = (0..n_total)
        .filter(|&c| {
            c == cursor_col || Some(c) == target || !pinned.get(c).copied().unwrap_or(false)
        })
        .collect();
    let size_of = |c: usize| sizes.get(c).copied().unwrap_or_default();
    let at = |col: usize| live.iter().position(|&c| c == col);
    let spines_off = (n_total - live.len()) as u16;
    // The widest window that fits: for each count, the slide that holds the
    // cursor and then the ghost, and the split over exactly those columns.
    let mut fitted = None;
    for n_vis in (1..=live.len()).rev() {
        let mut win = (*window).min(live.len() - n_vis);
        for i in [at(cursor_col).or(Some(0)), target.and_then(at)].into_iter().flatten() {
            if i < win {
                win = i;
            } else if i >= win + n_vis {
                win = i + 1 - n_vis;
            }
        }
        let spines = (live.len() - n_vis) as u16 + spines_off;
        let fixed = LPAD + RPAD + (n_vis as u16 - 1) * GUT + spines * (SPINE + GUT + 2 * SPINE_GAP);
        let Some(avail) = w.checked_sub(fixed) else { continue };
        let vis: Vec<ColumnWidth> = live[win..win + n_vis].iter().map(|&c| size_of(c)).collect();
        if let Some(widths) = split(avail, &vis) {
            fitted = Some((win, widths));
            break;
        }
    }
    let (win, widths) = fitted.unwrap_or_else(|| {
        // Nothing fits even alone: one column takes what there is.
        let i = at(cursor_col).unwrap_or(0);
        (i, vec![w.saturating_sub(LPAD + RPAD).min(cap(size_of(live[i])))])
    });
    *window = win;
    let visible = widths.len();

    let mut slots = Vec::with_capacity(n_total);
    let mut x = LPAD;
    for col in 0..n_total {
        let shown = at(col).filter(|&i| i >= win && i < win + visible);
        match shown {
            None => {
                x += SPINE_GAP;
                slots.push(Slot::Spine { x });
                x += SPINE + SPINE_GAP + GUT;
            }
            Some(i) => {
                let width = widths[i - win];
                slots.push(Slot::Expanded { x, width });
                x += width + GUT;
            }
        }
    }
    BoardGeometry { slots, window: win, visible }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_board_at_120() {
        // D33i's published number: 4 columns at 120 → T floor 25.
        let mut win = 0;
        let g = board_geometry(120, 4, 0, None, &[], &[], &mut win);
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
        let g = board_geometry(100, 4, 0, None, &[], &[], &mut win);
        assert_eq!(g.visible, 3);
        assert!(matches!(g.slots[3], Slot::Spine { .. }));
        assert!(g.slots[..3].iter().all(|s| matches!(s, Slot::Expanded { .. })));
    }

    #[test]
    fn window_slides_to_cursor_and_spines_swap_edges() {
        let mut win = 0;
        let g = board_geometry(100, 4, 3, None, &[], &[], &mut win);
        assert_eq!(g.window, 1);
        assert!(matches!(g.slots[0], Slot::Spine { x } if x == LPAD + SPINE_GAP));
        assert!(g.slots[1..].iter().all(|s| matches!(s, Slot::Expanded { .. })));
        // Sliding back left restores the original window.
        let g = board_geometry(100, 4, 0, None, &[], &[], &mut win);
        assert_eq!(g.window, 0);
        assert!(matches!(g.slots[3], Slot::Spine { .. }));
    }

    #[test]
    fn window_is_sticky_between_edges() {
        // Cursor inside the window does not move it.
        let mut win = 1;
        let g = board_geometry(100, 4, 2, None, &[], &[], &mut win);
        assert_eq!(g.window, 1);
        assert!(matches!(g.slots[0], Slot::Spine { .. }));
        assert!(matches!(g.slots[3], Slot::Spine { .. }) || g.visible == 3);
    }

    #[test]
    fn wide_terminals_clamp_column_width() {
        let mut win = 0;
        let g = board_geometry(300, 4, 0, None, &[], &[], &mut win);
        for s in &g.slots {
            if let Slot::Expanded { width, .. } = s {
                assert!(*width <= MAX_COL);
            }
        }
    }

    #[test]
    fn single_column_survives_any_width() {
        let mut win = 0;
        let g = board_geometry(60, 1, 0, None, &[], &[], &mut win);
        assert_eq!(g.visible, 1);
        assert!(matches!(g.slots[0], Slot::Expanded { .. }));
    }

    #[test]
    fn many_columns_at_the_floor() {
        // 8 columns at 60 wide: at least one expanded, spines for the rest,
        // everything within bounds.
        let mut win = 0;
        for cursor in 0..8 {
            let g = board_geometry(60, 8, cursor, None, &[], &[], &mut win);
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
        let g = board_geometry(120, 0, 0, None, &[], &[], &mut win);
        assert!(g.slots.is_empty());
    }

    /// A spine has a cell of air on each side: the previous column's pad and
    /// the next column's bar never touch its letters.
    #[test]
    fn a_spine_breathes_on_both_sides() {
        let mut win = 0;
        let pinned = [false, true, false];
        let g = board_geometry(120, 3, 0, None, &pinned, &[], &mut win);
        let (
            Slot::Expanded { x: x0, width: w0 },
            Slot::Spine { x: sx },
            Slot::Expanded { x: x2, .. },
        ) = (g.slots[0].clone(), g.slots[1].clone(), g.slots[2].clone())
        else {
            panic!("{:?}", g.slots);
        };
        assert_eq!(sx - (x0 + w0), GUT + SPINE_GAP, "air before the spine");
        assert_eq!(x2 - (sx + SPINE), GUT + SPINE_GAP, "air after it");
    }

    /// A pinned column (T-117) is a spine at any width — unless the cursor
    /// is in it, which expands it for the visit.
    #[test]
    fn pinned_column_is_a_spine_at_any_width() {
        let mut win = 0;
        let pinned = [false, false, false, true];
        let g = board_geometry(120, 4, 0, None, &pinned, &[], &mut win);
        assert_eq!(g.visible, 3);
        assert!(matches!(g.slots[3], Slot::Spine { .. }));
        assert!(g.slots[..3].iter().all(|s| matches!(s, Slot::Expanded { .. })));
        let g = board_geometry(120, 4, 3, None, &pinned, &[], &mut win);
        assert!(matches!(g.slots[3], Slot::Expanded { .. }), "the cursor expands it");
        assert_eq!(g.visible, 4);
        // A pinned spine off the window's edge still costs its cell: no slot
        // runs past the right pad.
        let mut win = 0;
        let pinned = [true, true, true, true, true, false, false, false];
        for cursor in 0..8 {
            let g = board_geometry(60, 8, cursor, None, &pinned, &[], &mut win);
            assert!(matches!(g.slots[cursor], Slot::Expanded { .. }), "cursor col expanded");
            for s in &g.slots {
                let end = match s {
                    Slot::Expanded { x, width } => x + width,
                    Slot::Spine { x } => x + SPINE,
                };
                assert!(end <= 60 - RPAD);
            }
        }
    }

    /// The sizes (T-1135): all M is the old equal split cell for cell; a
    /// size shares the slack above the floors and never folds a column
    /// that would not have folded; a ceiling hands the rest on; and the
    /// cursor's grown column moves one edge when it steps to its neighbour.
    #[test]
    fn sizes_share_the_slack_above_the_floor() {
        use ColumnWidth::*;
        let widths = |g: &BoardGeometry| -> Vec<u16> {
            g.slots
                .iter()
                .filter_map(|s| match s {
                    Slot::Expanded { width, .. } => Some(*width),
                    Slot::Spine { .. } => None,
                })
                .collect()
        };
        let mut win = 0;
        let g = board_geometry(120, 4, 0, None, &[], &[M, M, M, M], &mut win);
        assert_eq!(widths(&g), vec![29, 29, 29, 28], "all M is the old split");
        // 120: 115 cells, 104 of floors, 11 of slack over 6 shares.
        let g = board_geometry(120, 4, 0, None, &[], &[M, Xl, M, S], &mut win);
        assert_eq!(widths(&g), vec![28, 34, 27, 26]);
        // 100 folds one column at all M; a size folds no other.
        let g = board_geometry(100, 4, 0, None, &[], &[M, Xl, M, S], &mut win);
        assert_eq!(g.visible, 3, "{:?}", g.slots);
        assert!(matches!(g.slots[3], Slot::Spine { .. }));
        // Wide: each size stops at its ceiling and the rest is left over.
        let mut win = 0;
        let g = board_geometry(300, 4, 0, None, &[], &[S, M, L, Xl], &mut win);
        assert_eq!(widths(&g), vec![26, 40, 60, 90]);
        // The grown column: TODO and DONE keep their x across the step.
        let x_of = |g: &BoardGeometry, c: usize| match g.slots[c] {
            Slot::Expanded { x, .. } => x,
            Slot::Spine { x } => x,
        };
        let mut win = 0;
        let a = board_geometry(120, 4, 1, None, &[], &[M, Xl, M, M], &mut win);
        let b = board_geometry(120, 4, 2, None, &[], &[M, M, Xl, M], &mut win);
        assert_eq!(widths(&a), vec![28, 33, 27, 27]);
        assert_eq!(widths(&b), vec![28, 28, 32, 27], "the leftover cells stay leftmost");
        assert_eq!(x_of(&a, 0), x_of(&b, 0));
        assert_eq!(x_of(&a, 3), x_of(&b, 3));
    }

    /// A pending move's column opens while the ghost stands in it (T-1133):
    /// a pinned column for the visit, a squeezed-out one by sliding the
    /// window — and the cursor's column stays open beside it.
    #[test]
    fn a_pending_moves_column_is_open() {
        let expanded = |g: &BoardGeometry, c: usize| matches!(g.slots[c], Slot::Expanded { .. });
        // Pinned: DONE is a spine until a ghost enters it.
        let pinned = [false, false, false, true];
        let mut win = 0;
        let g = board_geometry(120, 4, 2, Some(3), &pinned, &[], &mut win);
        assert!(expanded(&g, 3) && expanded(&g, 2), "{:?}", g.slots);
        // The drop or the cancel folds it again.
        let g = board_geometry(120, 4, 2, None, &pinned, &[], &mut win);
        assert!(!expanded(&g, 3), "{:?}", g.slots);
        // Squeezed out at 100: the window slides one to take the ghost in.
        let mut win = 0;
        let g = board_geometry(100, 4, 2, Some(3), &[], &[], &mut win);
        assert!(expanded(&g, 3) && expanded(&g, 2), "{:?}", g.slots);
        assert_eq!(g.window, 1);
        // Leftward the same, across a pinned column and the window's edge.
        let mut win = 1;
        let g = board_geometry(100, 4, 1, Some(0), &[true, false, false, false], &[], &mut win);
        assert!(expanded(&g, 0) && expanded(&g, 1), "{:?}", g.slots);
        // One column fits: the ghost's is the one shown.
        let mut win = 0;
        let g = board_geometry(60, 8, 4, Some(5), &[], &[], &mut win);
        assert_eq!(g.visible, 1);
        assert!(expanded(&g, 5), "{:?}", g.slots);
    }
}
