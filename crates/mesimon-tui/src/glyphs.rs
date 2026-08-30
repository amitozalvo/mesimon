//! Card and session glyphs (07 §4.2 owns the precedence; 06 §3.2/§4.2 own the
//! codepoints and registers). One aggregate glyph per card, never two;
//! `throttled` is deliberately absent from the card tier — it is a board
//! banner (M6), not a per-card flag (D14/D19).

use mesimon_core::board::{
    Confidence, ExitReason, Reason, SessionRecord, SessionState, StopReason,
};

/// Working-spinner frames. Braille dots on the unicode tier (one cell, Neutral
/// width), the classic bar on ascii. Cadence and frames are deliberately
/// hardcoded — configurability is deferred with the rest of the M6 polish.
const SPIN_UNICODE: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const SPIN_ASCII: &[char] = &['|', '/', '-', '\\'];

/// One spinner step per redraw-clock interval (see `App::spin_frame`).
pub(crate) const SPIN_STEP_MS: u64 = 100;

/// The animated working glyph for `frame` (any monotonically increasing
/// counter; wraps internally).
pub(crate) fn spinner(tier: Tier, frame: usize) -> char {
    let frames = if tier == Tier::Ascii { SPIN_ASCII } else { SPIN_UNICODE };
    frames[frame % frames.len()]
}

/// The plan-review mark: stacked lines read as a list of steps (U+2630,
/// EAW Neutral so it stays one cell, Emoji=No). Rides the same attention
/// register as `!` — the reason differs, the urgency does not.
fn plan_mark(tier: Tier) -> char {
    if tier == Tier::Ascii { '=' } else { '☰' }
}

/// Which colour family a glyph rides (06 §2.1: exactly three chromatic tokens;
/// everything else is the grey ramp).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Register {
    /// The one saturated colour — needs-you, persistent (L2/L3).
    Attn,
    /// Errored — never the accent.
    Err,
    /// Done-unseen — decays once seen (decay is M6).
    Calm,
    Grey,
}

/// Glyph tier: unicode is the default; ascii is forced under mono terminals
/// (06 §4.2 — `auto ≡ unicode`, nerd never auto-selected).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tier {
    Unicode,
    Ascii,
}

/// The card's aggregate state glyph, or None when nothing is abnormal —
/// a normal card starts its title at T[0] (07 §4.1).
///
/// Precedence (07 §4.2, D34.9 removes unclaimed, no dependency model yet):
/// requires_action > failed/exited{!=0} > idle{end_turn} unseen > running >
/// sleeping (all sessions) > unknown. Running is a deviation from 07 §4.1's
/// "normal card has no glyph": the ticking age alone read as ambiguous, so a
/// working card carries the grey spinner (author 2026-08-30). `spin` is the
/// redraw-clock frame; it only matters when the result is the working glyph.
pub(crate) fn card_glyph(
    sessions: &[&SessionRecord],
    tier: Tier,
    spin: usize,
) -> Option<(char, Register)> {
    if sessions.is_empty() {
        return None;
    }
    let usable = |s: &&&SessionRecord| {
        matches!(s.confidence, Confidence::High | Confidence::Medium)
    };
    // Plan approval gets its own mark, but only when it is the whole story —
    // any other pending reason (permission ranks above plan) keeps the bang.
    let mut has_attn = false;
    let mut all_plan = true;
    for s in sessions.iter().filter(usable) {
        if let SessionState::RequiresAction { reason } = &s.state {
            has_attn = true;
            all_plan &= matches!(reason, Reason::Plan);
        }
    }
    if has_attn {
        return Some((if all_plan { plan_mark(tier) } else { '!' }, Register::Attn));
    }
    if sessions.iter().any(|s| {
        matches!(
            s.state,
            SessionState::Failed { .. }
                | SessionState::Exited { reason: ExitReason::Crashed | ExitReason::Killed }
        )
    }) {
        return Some(('x', Register::Err));
    }
    if sessions
        .iter()
        .any(|s| matches!(s.state, SessionState::Idle { stop_reason: StopReason::EndTurn }))
    {
        return Some((if tier == Tier::Ascii { '+' } else { '✓' }, Register::Calm));
    }
    if sessions.iter().any(|s| matches!(s.state, SessionState::Running)) {
        return Some((spinner(tier, spin), Register::Grey));
    }
    if sessions.iter().all(|s| matches!(s.state, SessionState::Sleeping)) {
        return Some(('z', Register::Grey));
    }
    if sessions.iter().any(|s| matches!(s.state, SessionState::Unknown { .. })) {
        return Some(('?', Register::Grey));
    }
    None
}

/// Per-session liveness glyph (06 §3.2) — the meta-strip dots, the accordion
/// rows, and the ticket-screen rail. Never blended with the card glyph (D28).
/// `spin` animates the working glyph, exactly as on the card.
pub(crate) fn session_glyph(state: &SessionState, tier: Tier, spin: usize) -> (char, Register) {
    let ascii = tier == Tier::Ascii;
    match state {
        SessionState::Spawning => (if ascii { '.' } else { '◦' }, Register::Grey),
        SessionState::Running => (spinner(tier, spin), Register::Grey),
        SessionState::RequiresAction { reason: Reason::Plan } => {
            (plan_mark(tier), Register::Attn)
        }
        SessionState::RequiresAction { .. } => ('!', Register::Attn),
        SessionState::Idle { stop_reason: StopReason::EndTurn } => {
            (if ascii { '+' } else { '✓' }, Register::Calm)
        }
        SessionState::Idle { .. } => (if ascii { '.' } else { '◦' }, Register::Grey),
        SessionState::Sleeping => ('z', Register::Grey),
        SessionState::Exited { reason: ExitReason::Crashed | ExitReason::Killed } => {
            ('x', Register::Err)
        }
        SessionState::Exited { .. } => (if ascii { '+' } else { '✓' }, Register::Grey),
        SessionState::Failed { .. } => ('x', Register::Err),
        SessionState::Throttled => ('~', Register::Grey),
        SessionState::Unknown { .. } => ('?', Register::Grey),
    }
}

/// The session-kind mark: `✻` is the mark Claude Code itself uses (U+273B,
/// Emoji=No, Neutral width — one cell), `$` for a shell. Ascii tier: `*`.
pub(crate) fn kind_mark(kind: mesimon_core::board::SessionKind, tier: Tier) -> char {
    use mesimon_core::board::SessionKind;
    match (kind, tier) {
        (SessionKind::Claude, Tier::Unicode) => '✻',
        (SessionKind::Claude, Tier::Ascii) => '*',
        (SessionKind::Bash, _) => '$',
    }
}

/// The session's lowercase state word for the ticket rail (06 §3.2/§3.3:
/// UPPERCASE ⟺ a human is required — those come from `reason_word`).
pub(crate) fn state_word(state: &SessionState) -> &'static str {
    match state {
        SessionState::Spawning => "spawning",
        SessionState::Running => "working",
        SessionState::RequiresAction { .. } => "NEEDS YOU",
        SessionState::Idle { stop_reason: StopReason::EndTurn } => "done",
        SessionState::Idle { .. } => "idle",
        SessionState::Sleeping => "sleeping",
        SessionState::Exited { reason: ExitReason::Crashed | ExitReason::Killed } => "FAILED",
        SessionState::Exited { .. } => "exited",
        SessionState::Failed { .. } => "FAILED",
        SessionState::Throttled => "throttled",
        SessionState::Unknown { .. } => "unavailable",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesimon_core::board::{FailReason, Reason, SessionKind, SessionRecord};

    fn rec(state: SessionState) -> SessionRecord {
        SessionRecord::new(
            uuid::Uuid::new_v4(),
            SessionKind::Claude,
            ulid::Ulid::new(),
            vec![],
            String::new(),
            state,
        )
    }

    #[test]
    fn running_card_shows_spinner() {
        let a = rec(SessionState::Running);
        let b = rec(SessionState::Spawning);
        assert_eq!(card_glyph(&[&a, &b], Tier::Unicode, 0), Some(('⠋', Register::Grey)));
        assert_eq!(card_glyph(&[&a], Tier::Ascii, 0), Some(('|', Register::Grey)));
        // The frame advances the glyph — that IS the animation.
        assert_ne!(card_glyph(&[&a], Tier::Unicode, 1), card_glyph(&[&a], Tier::Unicode, 0));
        // Spawning alone stays glyph-less: title at T[0] until work starts.
        assert_eq!(card_glyph(&[&b], Tier::Unicode, 0), None);
        assert_eq!(card_glyph(&[], Tier::Unicode, 0), None);
    }

    #[test]
    fn spinner_frames_wrap_and_stay_one_cell() {
        use unicode_width::UnicodeWidthChar;
        for tier in [Tier::Unicode, Tier::Ascii] {
            for f in 0..24 {
                let c = spinner(tier, f);
                assert_eq!(c.width(), Some(1), "{c:?} not one cell");
            }
            assert_eq!(spinner(tier, 0), spinner(tier, 20)); // 10- and 4-frame cycles
        }
    }

    #[test]
    fn attention_wins_over_everything() {
        let attn = rec(SessionState::RequiresAction { reason: Reason::Permission });
        let fail = rec(SessionState::Failed { reason: FailReason::Server });
        assert_eq!(card_glyph(&[&fail, &attn], Tier::Unicode, 0), Some(('!', Register::Attn)));
    }

    #[test]
    fn plan_gets_its_own_mark_unless_outranked() {
        use unicode_width::UnicodeWidthChar;
        let plan = rec(SessionState::RequiresAction { reason: Reason::Plan });
        let perm = rec(SessionState::RequiresAction { reason: Reason::Permission });
        assert_eq!(card_glyph(&[&plan], Tier::Unicode, 0), Some(('☰', Register::Attn)));
        assert_eq!(card_glyph(&[&plan], Tier::Ascii, 0), Some(('=', Register::Attn)));
        // A co-pending non-plan reason keeps the generic bang on the card.
        assert_eq!(card_glyph(&[&plan, &perm], Tier::Unicode, 0), Some(('!', Register::Attn)));
        assert_eq!(
            session_glyph(&plan.state, Tier::Unicode, 0),
            ('☰', Register::Attn)
        );
        assert_eq!(session_glyph(&perm.state, Tier::Unicode, 0).0, '!');
        assert_eq!('☰'.width(), Some(1));
    }

    #[test]
    fn low_confidence_attention_never_lights() {
        // 11 §11.5.4: Low/Stale never gets the saturated colour.
        let mut attn = rec(SessionState::RequiresAction { reason: Reason::Question });
        attn.confidence = Confidence::Low;
        assert_eq!(card_glyph(&[&attn], Tier::Unicode, 0), None);
    }

    #[test]
    fn failed_beats_done_beats_sleeping() {
        let fail = rec(SessionState::Failed { reason: FailReason::Server });
        let done = rec(SessionState::Idle { stop_reason: StopReason::EndTurn });
        let sleep = rec(SessionState::Sleeping);
        assert_eq!(
            card_glyph(&[&done, &fail], Tier::Unicode, 0),
            Some(('x', Register::Err))
        );
        assert_eq!(
            card_glyph(&[&sleep, &done], Tier::Unicode, 0),
            Some(('✓', Register::Calm))
        );
    }

    #[test]
    fn z_requires_all_sessions_sleeping() {
        let sleep = rec(SessionState::Sleeping);
        let run = rec(SessionState::Running);
        assert_eq!(card_glyph(&[&sleep], Tier::Unicode, 0), Some(('z', Register::Grey)));
        assert_eq!(card_glyph(&[&sleep, &run], Tier::Unicode, 0), Some(('⠋', Register::Grey)));
    }

    #[test]
    fn crashed_exit_is_err_clean_exit_is_not() {
        let crashed = rec(SessionState::Exited { reason: ExitReason::Crashed });
        let clean = rec(SessionState::Exited { reason: ExitReason::UserQuit });
        assert_eq!(card_glyph(&[&crashed], Tier::Unicode, 0), Some(('x', Register::Err)));
        assert_eq!(card_glyph(&[&clean], Tier::Unicode, 0), None);
    }

    #[test]
    fn ascii_tier_substitutes() {
        let done = rec(SessionState::Idle { stop_reason: StopReason::EndTurn });
        assert_eq!(card_glyph(&[&done], Tier::Ascii, 0), Some(('+', Register::Calm)));
        assert_eq!(session_glyph(&SessionState::Running, Tier::Ascii, 0).0, '|');
        assert_eq!(session_glyph(&SessionState::Running, Tier::Unicode, 0).0, '⠋');
    }

    #[test]
    fn uppercase_iff_human_required() {
        // 06 §3.3's case rule, spot-checked.
        assert_eq!(state_word(&SessionState::Running), "working");
        assert_eq!(
            state_word(&SessionState::RequiresAction { reason: Reason::Plan }),
            "NEEDS YOU"
        );
        assert_eq!(
            state_word(&SessionState::Failed { reason: FailReason::Server }),
            "FAILED"
        );
        assert_eq!(state_word(&SessionState::Sleeping), "sleeping");
    }
}
