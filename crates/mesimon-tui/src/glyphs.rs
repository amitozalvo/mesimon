//! Card and session glyphs (07 §4.2 owns the precedence; 06 §3.2/§4.2 own the
//! codepoints and registers). One aggregate glyph per card, never two;
//! `throttled` is deliberately absent from the card tier — it is a board
//! banner (M6), not a per-card flag (D14/D19).

use mesimon_core::board::{Confidence, ExitReason, SessionRecord, SessionState, StopReason};

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
/// requires_action > failed/exited{!=0} > idle{end_turn} unseen > sleeping
/// (all sessions) > unknown.
pub(crate) fn card_glyph(sessions: &[&SessionRecord], tier: Tier) -> Option<(char, Register)> {
    if sessions.is_empty() {
        return None;
    }
    let usable = |s: &&&SessionRecord| {
        matches!(s.confidence, Confidence::High | Confidence::Medium)
    };
    if sessions
        .iter()
        .filter(usable)
        .any(|s| matches!(s.state, SessionState::RequiresAction { .. }))
    {
        return Some(('!', Register::Attn));
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
pub(crate) fn session_glyph(state: &SessionState, tier: Tier) -> (char, Register) {
    let ascii = tier == Tier::Ascii;
    match state {
        SessionState::Spawning => (if ascii { '.' } else { '◦' }, Register::Grey),
        SessionState::Running => (if ascii { '>' } else { '▸' }, Register::Grey),
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
    fn normal_card_has_no_glyph() {
        let a = rec(SessionState::Running);
        let b = rec(SessionState::Spawning);
        assert_eq!(card_glyph(&[&a, &b], Tier::Unicode), None);
        assert_eq!(card_glyph(&[], Tier::Unicode), None);
    }

    #[test]
    fn attention_wins_over_everything() {
        let attn = rec(SessionState::RequiresAction { reason: Reason::Permission });
        let fail = rec(SessionState::Failed { reason: FailReason::Server });
        assert_eq!(card_glyph(&[&fail, &attn], Tier::Unicode), Some(('!', Register::Attn)));
    }

    #[test]
    fn low_confidence_attention_never_lights() {
        // 11 §11.5.4: Low/Stale never gets the saturated colour.
        let mut attn = rec(SessionState::RequiresAction { reason: Reason::Question });
        attn.confidence = Confidence::Low;
        assert_eq!(card_glyph(&[&attn], Tier::Unicode), None);
    }

    #[test]
    fn failed_beats_done_beats_sleeping() {
        let fail = rec(SessionState::Failed { reason: FailReason::Server });
        let done = rec(SessionState::Idle { stop_reason: StopReason::EndTurn });
        let sleep = rec(SessionState::Sleeping);
        assert_eq!(
            card_glyph(&[&done, &fail], Tier::Unicode),
            Some(('x', Register::Err))
        );
        assert_eq!(
            card_glyph(&[&sleep, &done], Tier::Unicode),
            Some(('✓', Register::Calm))
        );
    }

    #[test]
    fn z_requires_all_sessions_sleeping() {
        let sleep = rec(SessionState::Sleeping);
        let run = rec(SessionState::Running);
        assert_eq!(card_glyph(&[&sleep], Tier::Unicode), Some(('z', Register::Grey)));
        assert_eq!(card_glyph(&[&sleep, &run], Tier::Unicode), None);
    }

    #[test]
    fn crashed_exit_is_err_clean_exit_is_not() {
        let crashed = rec(SessionState::Exited { reason: ExitReason::Crashed });
        let clean = rec(SessionState::Exited { reason: ExitReason::UserQuit });
        assert_eq!(card_glyph(&[&crashed], Tier::Unicode), Some(('x', Register::Err)));
        assert_eq!(card_glyph(&[&clean], Tier::Unicode), None);
    }

    #[test]
    fn ascii_tier_substitutes() {
        let done = rec(SessionState::Idle { stop_reason: StopReason::EndTurn });
        assert_eq!(card_glyph(&[&done], Tier::Ascii), Some(('+', Register::Calm)));
        assert_eq!(session_glyph(&SessionState::Running, Tier::Ascii).0, '>');
        assert_eq!(session_glyph(&SessionState::Running, Tier::Unicode).0, '▸');
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
