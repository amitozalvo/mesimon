//! Card and session glyphs (07 §4.2 owns the precedence; 06 §3.2/§4.2 own the
//! codepoints and registers). One aggregate glyph per card, never two;
//! `throttled` is deliberately absent from the card tier — it is a board
//! banner (M6), not a per-card flag (D14/D19).

use mesimon_core::board::{
    Confidence, ExitReason, Reason, SessionKind, SessionRecord, SessionState, StopReason,
};

/// Working-spinner frames. Braille dots on the unicode tier (one cell, Neutral
/// width), the classic bar on ascii. Cadence and frames are deliberately
/// hardcoded — configurability is deferred with the rest of the M6 polish.
const SPIN_UNICODE: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const SPIN_ASCII: &[char] = &['|', '/', '-', '\\'];

/// One spinner step per redraw-clock interval (see `App::spin_frame`).
pub(crate) const SPIN_STEP_MS: u64 = 100;

/// Waiting frames: a TWO-dot braille pair walking the ring, against the
/// working spinner's three-dot arc — two thirds of the ink at a quarter of
/// the speed. An uncertain session is not working and must not read as if it
/// were; it must still read as something (a single dot vanishes at dim2 —
/// author 2026-08-31, "no glyph at all"). Ascii tier drops a pair down the
/// cell instead, avoiding `.` (the idle mark) so no frame ever impersonates
/// a settled state.
const WAIT_UNICODE: &[char] = &['⠉', '⠘', '⠰', '⠤', '⠆', '⠃'];
const WAIT_ASCII: &[char] = &['"', ':', ','];

/// Redraw ticks per SLOW step: 4 × `SPIN_STEP_MS` = 400 ms a frame, four
/// times slower than the spinner. Waiting is not progress and launching is
/// not yet progress; neither should do more than barely move (D19's motion
/// ban bends for the spinner — it must not bend twice at the same speed).
/// Waiting and launching share this one cadence deliberately: the board has
/// a fast register and a slow one, and a third speed would be a third thing
/// moving.
const SLOW_STEP_TICKS: usize = 4;

/// The peek's activity mark: a bullet that BLINKS rather than spins. The row
/// beside it already names the step, so a second spinner would be two things
/// moving at one speed — D19's motion ban bends for the working spinner, and
/// it must not bend twice at the same cadence. So this holds its shape and
/// only changes weight, on a beat ten times slower than the spinner.
pub(crate) fn pulse(tier: Tier) -> char {
    // U+25CF, outside the 0x2500–0x259F structure range the L1 law bans, and
    // full-size on purpose: the small bullet reads as punctuation.
    if tier == Tier::Ascii {
        '*'
    } else {
        '●'
    }
}

/// Redraw ticks per half-blink: 10 × `SPIN_STEP_MS` = a one-second on/off
/// beat, slow enough that a whole test render lands inside the lit half.
const PULSE_STEP_TICKS: usize = 10;

/// Is the pulse in its lit half on `frame`? The unlit half drops one dim
/// tier — never the terminal's blink attribute, which is unreliable and
/// which nothing else on the board uses.
pub(crate) fn pulse_lit(frame: usize) -> bool {
    (frame / PULSE_STEP_TICKS) % 2 == 0
}

/// The animated working glyph for `frame` (any monotonically increasing
/// counter; wraps internally).
pub(crate) fn spinner(tier: Tier, frame: usize) -> char {
    let frames = if tier == Tier::Ascii { SPIN_ASCII } else { SPIN_UNICODE };
    frames[frame % frames.len()]
}

/// The waiting glyph for `frame` (same redraw-clock counter the spinner
/// rides; the divisor is what makes it slower).
pub(crate) fn waiting(tier: Tier, frame: usize) -> char {
    let frames = if tier == Tier::Ascii { WAIT_ASCII } else { WAIT_UNICODE };
    frames[(frame / SLOW_STEP_TICKS) % frames.len()]
}

/// The launching glyph: the WORKING arc at the slow cadence. Spawning is not
/// a different thing from working — it is working that has not started yet —
/// so a different shape would overstate the difference. The slowness is the
/// whole message: the same arc, not turning over yet.
///
/// Deliberately NOT disjoint from the spinner, where `waiting` must be and
/// is: `Unknown` means the daemon has lost track, and a still frame of that
/// must never read as progress. Launching resolves into working within
/// seconds and both mean the same thing to the reader — the agent is going,
/// leave it alone.
pub(crate) fn launching(tier: Tier, frame: usize) -> char {
    spinner(tier, frame / SLOW_STEP_TICKS)
}

/// Background-parked frames: a TWO-dot braille bar turning through the
/// CENTRE of the cell — `—`, `/`, `\` — against `waiting`'s two-dot pair
/// hugging the rim and the spinner's three-dot arc. Same ink as `waiting` by
/// necessity: one dot was measured invisible at dim2 (author 2026-08-31, "no
/// glyph at all"), which is why that glyph carries two, and this one may not
/// carry three without claiming the spinner's "work in flight here".
///
/// Ascii cannot borrow the same idea: `| / - \` are the spinner's frames and
/// a turning bar spelled in them IS the spinner. `~` is throttled, `. : , "`
/// are idle and waiting. So the ascii tier breathes instead of turning — a
/// ring swelling and shrinking, on the same clock.
const BG_UNICODE: &[char] = &['⠒', '⠌', '⠡'];
const BG_ASCII: &[char] = &['o', 'O'];

/// The background-parked glyph: the turn ended, but a task the agent
/// backgrounded is still running, so the work is not finished and the pane
/// has stopped painting. It rides the SLOW cadence — `waiting`'s and
/// `launching`'s — because something genuinely is in flight, just not in this
/// pane; a still mark would say the board had gone quiet when it had not.
/// No third speed is added: the board keeps one fast register and one slow.
pub(crate) fn background(tier: Tier, frame: usize) -> char {
    let frames = if tier == Tier::Ascii { BG_ASCII } else { BG_UNICODE };
    frames[(frame / SLOW_STEP_TICKS) % frames.len()]
}

/// Owed frames (2026-09-04): mesimon will act on this ticket on its own
/// clock — a queued ask waiting for its checkout to go quiet, a merge or a
/// rebase ask the train will make when the board does. A TWO-dot pair on
/// the HALF-diagonals (`⠑ ⠢ ⠔ ⠊`: dots 1+5, 2+6, 3+5, 2+4), the six pairs the
/// spinner's arcs, `waiting`'s rim pairs and `background`'s centre bar all
/// leave unused, so no frame of it can impersonate any of them. Two dots for
/// the reason `waiting` has two: one is invisible at dim2. It rides the SLOW
/// cadence — nothing is happening here yet, and the board keeps one fast
/// register and one slow; a third speed would be a third thing moving. Ascii
/// alternates parentheses: a bracket says "held", and `( )` sit in no other
/// table. Grey, never the accent: the user owes nothing, mesimon does.
const QUEUED_UNICODE: &[char] = &['⠑', '⠢', '⠔', '⠊'];
const QUEUED_ASCII: &[char] = &['(', ')'];

/// The owed glyph for `frame` — see `QUEUED_UNICODE`.
pub(crate) fn queued(tier: Tier, frame: usize) -> char {
    let frames = if tier == Tier::Ascii { QUEUED_ASCII } else { QUEUED_UNICODE };
    frames[(frame / SLOW_STEP_TICKS) % frames.len()]
}

/// Lay the owed mark over a card's session glyph. It replaces NOTHING that
/// moves or shouts: a working spinner, a launching arc, a parked bar and a
/// waiting pair all say something truer about the card right now, and the
/// accent and the error mark outrank everything. It takes the STILL marks —
/// done, idle, asleep, interrupted — and an empty slot, which is where a
/// ticket waiting on somebody else's turn sits.
pub(crate) fn queued_over(
    glyph: Option<(char, Register)>,
    tier: Tier,
    frame: usize,
) -> Option<(char, Register)> {
    let owed = Some((queued(tier, frame), Register::Grey));
    match glyph {
        None => owed,
        Some((_, Register::Attn | Register::Err)) => glyph,
        Some((g, _)) if is_still_mark(g, tier) => owed,
        Some(_) => glyph,
    }
}

/// The marks that do not move: done (seen or not), idle, asleep, interrupted.
fn is_still_mark(g: char, tier: Tier) -> bool {
    if tier == Tier::Ascii {
        matches!(g, '+' | '.' | 'z' | ';')
    } else {
        matches!(g, '✓' | '✔' | '◦' | 'z' | '⊘')
    }
}

/// The plan-review mark: stacked lines read as a list of steps (U+2261
/// IDENTICAL TO — same Ambiguous-width class as the ✓ we already ship).
/// NOT U+2630 TRIGRAM FOR HEAVEN: Unicode 16 reclassified the trigrams
/// Neutral→Wide, so terminals with current tables render ☰ two cells while
/// unicode-width 0.2.0 (ours AND ratatui's) says one — every line after the
/// glyph shifts on screen and the diff cursor desyncs, leaving stale cells.
/// Rides the same attention register as `!` — the reason differs, the
/// urgency does not.
fn plan_mark(tier: Tier) -> char {
    if tier == Tier::Ascii {
        '='
    } else {
        '≡'
    }
}

/// The interrupted mark: the user pressed Esc on a turn and the agent sits
/// with the prompt handed back. Not done (no `✓`), not working, not lost
/// (`waiting` is for a state we cannot see) and not the plain `◦` of a
/// session nobody has asked anything yet. Before this the card carried NO
/// glyph for it — identical to a ticket nobody had ever opened (author
/// 2026-09-04: "it looks like no session exists there"). `⊘` U+2298
/// CIRCLED DIVISION SLASH: the halt sign — the one shape whose meaning
/// survives at one cell and at the grey register, and which collides with
/// none of `✓` done, `x` failed, `z` asleep, `◦` idle. EAW=Neutral, Emoji=No,
/// Mathematical Operators (every Menlo-family face). The intuitive marks are
/// all barred by the width law: `⏸`/`⏹` are emoji-presentation codepoints
/// and `‖`/`■` are Ambiguous, any of which can paint two cells. `¦` BROKEN
/// BAR shipped for an hour and was too thin to read (author: "doesn't read
/// nicely"). ASCII `;`, a sentence stopped short: `|` is a spinner frame,
/// `.` the idle mark, `-` archived, `"`/`:`/`,` the waiting frames.
pub(crate) fn interrupted(tier: Tier) -> char {
    if tier == Tier::Ascii {
        ';'
    } else {
        '⊘'
    }
}

/// The suggestion mark. NOT a chevron: `›` reads as "you are here" — every
/// terminal prompt has trained that — and a suggestion is the opposite, an
/// offer you have not taken. NOT `◊` either: a full-height diamond outline is
/// louder than the offer it introduces (author 2026-08-31, "a bit big"). `◦`
/// U+25E6 is a small mid-height ring, directionless, EAW=N, and 06 §4.1 scores
/// it 6/7 present.
///
/// It is 06 §4.2's `idle` mark reused, deliberately: that glyph
/// lives on cards, this one lives in the chrome, and the two never share a
/// region — no row ever shows both. The ASCII tier falls back to `*` rather
/// than §4.1's `.`, which is too faint to read as a mark of its own.
///
/// It appears in exactly two places, and that is the point: on the header's
/// suggestion chip and on the Esc-menu rows that chip stands in front of.
pub(crate) fn suggest_mark(tier: Tier) -> char {
    if tier == Tier::Ascii {
        '*'
    } else {
        '◦'
    }
}

/// The branch glyph `⎇` U+2387 and the two arrows the board's own checkout
/// (the header, T-124) and a ticket's worktree (the card's `⎇↑`/`⎇↓`) share.
/// One home so the two surfaces cannot drift: `↑` is "ahead, push/merge
/// due" and `↓` is "behind, pull/rebase due" wherever they appear. `↑`
/// U+2191 / `↓` U+2193 are EAW=N, one cell, outside the banned box range;
/// the ASCII tier reads `& ^ v`.
pub(crate) fn branch_mark(tier: Tier) -> char {
    if tier == Tier::Ascii {
        '&'
    } else {
        '⎇'
    }
}

/// The column header's one optional mark (T-117): this column DOES
/// something to a ticket — moves it on an edge, starts a claude, reaches
/// the merge train. `→` U+2192, one cell, outside the banned box range,
/// drawn in the header's quiet register and dropped first when the row is
/// tight; the ASCII tier reads `>`.
pub(crate) fn auto_mark(tier: Tier) -> char {
    if tier == Tier::Ascii {
        '>'
    } else {
        '→'
    }
}

pub(crate) fn ahead_mark(tier: Tier) -> char {
    if tier == Tier::Ascii {
        '^'
    } else {
        '↑'
    }
}

pub(crate) fn behind_mark(tier: Tier) -> char {
    if tier == Tier::Ascii {
        'v'
    } else {
        '↓'
    }
}

/// The done mark while its reply is UNREAD (T-173): `✔` U+2714, the heavy
/// check, against the thin `✓` U+2713 `card_glyph` gives a finished agent
/// once the cursor has been on the card. Same idea, thicker stroke — the
/// unread state is the same state, louder. EAW=N and one cell in
/// `unicode-width`; it does carry the Emoji property (text-default
/// presentation), which 07 §18 rule 2 refuses on principle — a terminal
/// that prefers emoji presentation draws it two cells wide and in colour.
/// The author chose it on iTerm2, where it is a narrow text glyph, over the
/// safer bold-`✓` (author 2026-09-04); if it misbehaves somewhere, that is
/// the fallback, and this is the one function to change. The ASCII tier
/// has no thicker `+`, so there the colour step alone says unread.
pub(crate) fn done_unread(tier: Tier) -> char {
    if tier == Tier::Ascii {
        '+'
    } else {
        '✔'
    }
}

/// Which colour family a glyph rides (06 §2.1: exactly three chromatic tokens;
/// everything else is the grey ramp).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Register {
    /// The one saturated colour — needs-you, persistent (L2/L3).
    Attn,
    /// Errored — never the accent.
    Err,
    /// Done-UNSEEN: `card.rs` demotes it to `Grey` once the cursor has been
    /// on the card for the reply it stands for (T-173, `App::spoke`). The
    /// rail keeps it calm — the ticket page is the looking.
    Calm,
    Grey,
    /// Parked — no process, nothing to watch. `dim3`, the de-emphasis floor
    /// (06 §4.2 specifies the sleeping mark there), one step under `Grey`:
    /// on `Grey` the `z` sat as loud as the idle ring and the working
    /// spinner beside it, while the card's own bar had already faded to
    /// its neutral resting level (`tags::bar_spans`). The glyph is the one element
    /// on a parked card that was not walking the ladder, and that — not the
    /// letter — is what read as low effort (author 2026-09-02). The letter
    /// stays: `⏾` U+23FE is present in 2 of 06 §4.1's seven faces and `☾`
    /// U+263E in 3, both under the `⚑` the doc rejected at 3/7.
    Dormant,
}

/// Glyph tier: unicode is the default; ascii is forced under mono terminals
/// (06 §4.2 — `auto ≡ unicode`, nerd never auto-selected).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tier {
    Unicode,
    Ascii,
}

/// Does this session's `Running` mean work is in flight? Only an agent's
/// does. A shell has no hook stream, so the daemon pins it at `Running` for
/// the whole life of its pane (D15: "a live pane is all running means") —
/// that is liveness, not activity, and the working spinner on it told the
/// board a shell sitting at its prompt was busy (dogfood 2026-09-01). The
/// spinner is the one place D19's motion ban bends; it may only bend for
/// something actually moving.
pub(crate) fn is_working(rec: &SessionRecord) -> bool {
    rec.kind == SessionKind::Claude && rec.state == SessionState::Running
}

/// Is this session still in its launch window — the pane opening, or the
/// composed prompt typed and not yet accepted?
///
/// `Spawning` is only the first half of it. Shift+Enter's Enter is DEFERRED
/// to the `SessionStart` frame (paste detection swallows one sent with the
/// text), and that same frame moves the record to `Idle{Unknown}` — a state
/// that rightly carries no glyph, because an idle agent is one waiting for
/// you. So the card went dark for the ~500 ms between the session starting
/// and `UserPromptSubmit` acking the prompt, mid-launch (dogfood 2026-09-01).
/// `pending_submit` is precisely what says that wait is OURS: the daemon is
/// still pressing Enter on a 500 ms cadence and the turn has not begun.
///
/// It mirrors the daemon's own `pressable` predicate (server.rs
/// `retry_pending_submits`) — the launch mark is shown exactly while the
/// daemon still expects the prompt to land — minus its `Running` arm, which
/// the spinner outranks here: once work is in flight the fast arc is the
/// truthful one. A stale flag cannot strand the mark: the daemon clears it on
/// the ack, on giving up, and on any state where the pane stopped being
/// pressable, and no state outside these two ever consults it.
pub(crate) fn is_launching(rec: &SessionRecord) -> bool {
    match &rec.state {
        SessionState::Spawning => true,
        // `EndTurn` is a turn that FINISHED, which is only reachable through
        // the ack that clears the flag — so it never rides this path live,
        // and a record reloaded holding a stale one keeps `done`, which is
        // both the truer word and the one the card already preferred.
        SessionState::Idle { stop_reason } => {
            rec.pending_submit && !matches!(stop_reason, StopReason::EndTurn)
        }
        _ => false,
    }
}

/// The card's aggregate state glyph, or None when nothing is abnormal —
/// a normal card starts its title at T[0] (07 §4.1).
///
/// Precedence (07 §4.2, D34.9 removes unclaimed, no dependency model yet):
/// requires_action > failed/exited{!=0} > idle{end_turn} unseen > running >
/// launching (`is_launching`) > sleeping (all sessions) > unknown. Running is a deviation from
/// 07 §4.1's "normal card has no glyph": the ticking age alone read as
/// ambiguous, so a working card carries the grey spinner (author 2026-08-30),
/// and spawning followed it for the same reason one press later. `spin` is
/// the redraw-clock frame; it matters for the working, launching and waiting
/// glyphs.
pub(crate) fn card_glyph(
    sessions: &[&SessionRecord],
    tier: Tier,
    spin: usize,
) -> Option<(char, Register)> {
    if sessions.is_empty() {
        return None;
    }
    let usable =
        |s: &&&SessionRecord| matches!(s.confidence, Confidence::High | Confidence::Medium);
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
            SessionState::Failed { .. } | SessionState::Exited { reason: ExitReason::Crashed }
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
    if sessions.iter().any(|s| is_working(s)) {
        return Some((spinner(tier, spin), Register::Grey));
    }
    // Spawning is the launch window, and until Shift+Enter nobody watched it:
    // every other spawn hands the focus straight to the pane, so the card had
    // nothing to say and 07 §4.1's "a normal card has no glyph" cost nothing.
    // The composer's Shift+Enter stays on the board on purpose — the card IS
    // how the user watches the work land — and for the seconds between the
    // press and the first hook the card was a title with no sign of life
    // (author 2026-09-01). It is the slow arc, never the spinner: nothing is
    // in flight yet.
    if sessions.iter().any(|s| is_launching(s)) {
        return Some((launching(tier, spin), Register::Grey));
    }
    // Parked on a backgrounded task: not working, not done, and not waiting on
    // the user. Before this the card fell through every arm and carried NO
    // glyph — identical to a ticket nobody had ever opened — for as long as
    // the task ran (dogfood 2026-09-01, T-128: two minutes on a build-poll).
    // Under the spinner deliberately: if any session on the ticket is really
    // working, that is the louder and truer thing to say.
    if sessions
        .iter()
        .any(|s| matches!(s.state, SessionState::Idle { stop_reason: StopReason::Background }))
    {
        return Some((background(tier, spin), Register::Grey));
    }
    // Interrupted by the user: a known fact, so above `waiting` (which says
    // we lost track) and under everything in flight or finished.
    if sessions
        .iter()
        .any(|s| matches!(s.state, SessionState::Idle { stop_reason: StopReason::Interrupted }))
    {
        return Some((interrupted(tier), Register::Grey));
    }
    if sessions.iter().all(|s| matches!(s.state, SessionState::Sleeping)) {
        return Some(('z', Register::Dormant));
    }
    if sessions.iter().any(|s| matches!(s.state, SessionState::Unknown { .. })) {
        return Some((waiting(tier, spin), Register::Grey));
    }
    None
}

/// Per-session liveness glyph (06 §3.2) — the meta-strip dots, the accordion
/// rows, and the ticket-screen rail. Never blended with the card glyph (D28).
/// `spin` animates the working and waiting glyphs, exactly as on the card.
pub(crate) fn session_glyph(rec: &SessionRecord, tier: Tier, spin: usize) -> (char, Register) {
    let ascii = tier == Tier::Ascii;
    // A live shell is quiet, not working (`is_working`): it wears the idle
    // mark for as long as its pane lives, because pane death is the only
    // shell event there is.
    if rec.state == SessionState::Running && !is_working(rec) {
        return (if ascii { '.' } else { '◦' }, Register::Grey);
    }
    // The launch window, which outlives `Spawning`: the composed prompt is
    // typed but not yet accepted, so this `Idle` is not a settled state.
    if is_launching(rec) {
        return (launching(tier, spin), Register::Grey);
    }
    match &rec.state {
        SessionState::Spawning => (launching(tier, spin), Register::Grey),
        SessionState::Running => (spinner(tier, spin), Register::Grey),
        SessionState::RequiresAction { reason: Reason::Plan } => (plan_mark(tier), Register::Attn),
        SessionState::RequiresAction { .. } => ('!', Register::Attn),
        SessionState::Idle { stop_reason: StopReason::EndTurn } => {
            (if ascii { '+' } else { '✓' }, Register::Calm)
        }
        SessionState::Idle { stop_reason: StopReason::Background } => {
            (background(tier, spin), Register::Grey)
        }
        SessionState::Idle { stop_reason: StopReason::Interrupted } => {
            (interrupted(tier), Register::Grey)
        }
        SessionState::Idle { .. } => (if ascii { '.' } else { '◦' }, Register::Grey),
        SessionState::Sleeping => ('z', Register::Dormant),
        SessionState::Exited { reason: ExitReason::Crashed } => ('x', Register::Err),
        SessionState::Exited { .. } => (if ascii { '+' } else { '✓' }, Register::Grey),
        SessionState::Failed { .. } => ('x', Register::Err),
        SessionState::Throttled => ('~', Register::Grey),
        SessionState::Unknown { .. } => (waiting(tier, spin), Register::Grey),
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

/// A note row's mark on the ticket rail: three lines of text, and its
/// ASCII spelling.
pub(crate) fn note_mark(tier: Tier) -> char {
    match tier {
        Tier::Unicode => '≡',
        Tier::Ascii => '=',
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
        // Lowercase: nothing is required of the user. The turn is paused on
        // work the agent started, and it will resume itself.
        SessionState::Idle { stop_reason: StopReason::Background } => "background",
        // Lowercase too: the user stopped it and knows; the next prompt is
        // theirs to write when they choose.
        SessionState::Idle { stop_reason: StopReason::Interrupted } => "interrupted",
        SessionState::Idle { .. } => "idle",
        SessionState::Sleeping => "sleeping",
        // A deliberate kill is not a failure — the corpse stays resumable.
        SessionState::Exited { reason: ExitReason::Killed } => "killed",
        SessionState::Exited { reason: ExitReason::Crashed } => "FAILED",
        SessionState::Exited { .. } => "exited",
        SessionState::Failed { .. } => "FAILED",
        SessionState::Throttled => "throttled",
        SessionState::Unknown { .. } => "unavailable",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesimon_core::board::{FailReason, Reason, SessionKind, SessionRecord, UnknownReason};

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
        // A working session outranks a launching one on the same card: the
        // fast arc is the truthful one while anything is actually in flight.
        assert_eq!(
            card_glyph(&[&b, &a], Tier::Unicode, 0),
            Some((spinner(Tier::Unicode, 0), Register::Grey))
        );
        assert_eq!(card_glyph(&[], Tier::Unicode, 0), None);
    }

    /// Shift+Enter mints a ticket, spawns claude and STAYS on the board — the
    /// card is the only thing the user can watch. It used to show a title and
    /// nothing else until the first hook landed, so the launch window now
    /// carries the working arc at a quarter speed: the same shape as work,
    /// visibly not working yet.
    #[test]
    fn spawning_launches_slowly() {
        use unicode_width::UnicodeWidthChar;
        let spawning = rec(SessionState::Spawning);
        for tier in [Tier::Unicode, Tier::Ascii] {
            assert_eq!(
                card_glyph(&[&spawning], tier, 0),
                Some((launching(tier, 0), Register::Grey)),
                "a spawning card says nothing"
            );
            // The rail and the accordion dots say it too.
            assert_eq!(session_glyph(&spawning, tier, 0), (launching(tier, 0), Register::Grey));
            for f in 0..3 {
                assert_eq!(launching(tier, f), launching(tier, f + 1), "held for four ticks");
            }
            assert_ne!(launching(tier, 3), launching(tier, 4), "and then it steps");
            let cycle =
                if tier == Tier::Ascii { SPIN_ASCII } else { SPIN_UNICODE }.len() * SLOW_STEP_TICKS;
            assert_eq!(launching(tier, 0), launching(tier, cycle), "wraps cleanly");
            for f in 0..48 {
                assert_eq!(
                    launching(tier, f).width(),
                    Some(1),
                    "{:?} not one cell",
                    launching(tier, f)
                );
                // It is the WORKING arc, slowed — never a shape of its own,
                // and never one of the waiting frames, which mean the daemon
                // has lost track rather than not started yet.
                assert!((0..48).any(|w| spinner(tier, w) == launching(tier, f)));
                for w in 0..48 {
                    assert_ne!(
                        waiting(tier, w),
                        launching(tier, f),
                        "launching wore the waiting mark"
                    );
                }
            }
        }
    }

    /// The launch window does not end at `SessionStart`. That frame moves the
    /// record to `Idle{Unknown}` AND is when the deferred Enter is first
    /// pressed; the turn only begins at the `UserPromptSubmit` ack ~500 ms
    /// later. Between them the card went dark and then came back spinning
    /// (dogfood 2026-09-01, "for half a second it removed the animated
    /// glyph"). `pending_submit` is what keeps the mark lit across the seam.
    #[test]
    fn the_owed_enter_is_still_launching() {
        let mut composed = rec(SessionState::Idle { stop_reason: StopReason::Unknown });
        composed.pending_submit = true;
        // The same record without the owed Enter is an ordinary idle agent
        // waiting for YOU, and says nothing — that part must not change.
        let plain = rec(SessionState::Idle { stop_reason: StopReason::Unknown });
        for tier in [Tier::Unicode, Tier::Ascii] {
            assert!(is_launching(&composed));
            assert!(!is_launching(&plain));
            assert_eq!(
                card_glyph(&[&composed], tier, 0),
                Some((launching(tier, 0), Register::Grey)),
                "the card went dark mid-launch"
            );
            assert_eq!(card_glyph(&[&plain], tier, 0), None);
            assert_eq!(session_glyph(&composed, tier, 0), (launching(tier, 0), Register::Grey));
            // And the whole press-to-turn path is one unbroken mark: spawn,
            // session start, ack. Only the last frame changes what it says.
            let spawning = rec(SessionState::Spawning);
            let running = rec(SessionState::Running);
            assert_eq!(card_glyph(&[&spawning], tier, 0), card_glyph(&[&composed], tier, 0));
            assert_eq!(
                card_glyph(&[&running], tier, 0),
                Some((spinner(tier, 0), Register::Grey)),
                "the ack hands over to the working arc"
            );
        }
    }

    /// The flag is only ever read inside the launch window. A record that
    /// carries it into any other state — a restart reloads it from disk — is
    /// not launching, and must keep the mark its own state earned.
    #[test]
    fn a_stale_owed_enter_never_relabels_a_state() {
        for state in [
            SessionState::Unknown { reason: UnknownReason::DaemonRestarted },
            SessionState::Sleeping,
            SessionState::Exited { reason: ExitReason::UserQuit },
            SessionState::Failed { reason: FailReason::Server },
            SessionState::Idle { stop_reason: StopReason::EndTurn },
        ] {
            let mut owed = rec(state.clone());
            owed.pending_submit = true;
            let clean = rec(state.clone());
            assert_eq!(
                session_glyph(&owed, Tier::Unicode, 0),
                session_glyph(&clean, Tier::Unicode, 0),
                "{state:?} was relabelled by a stale owed Enter"
            );
            assert_eq!(
                card_glyph(&[&owed], Tier::Unicode, 0),
                card_glyph(&[&clean], Tier::Unicode, 0),
                "{state:?} was relabelled by a stale owed Enter"
            );
        }
    }

    /// Everything abnormal still outranks the launch window: a card with a
    /// spawning session and a pending question is a card that needs you.
    #[test]
    fn launching_yields_to_every_real_state() {
        let spawning = rec(SessionState::Spawning);
        let attn = rec(SessionState::RequiresAction { reason: Reason::Question });
        let done = rec(SessionState::Idle { stop_reason: StopReason::EndTurn });
        let fail = rec(SessionState::Failed { reason: FailReason::Server });
        assert_eq!(card_glyph(&[&spawning, &attn], Tier::Unicode, 0).unwrap().1, Register::Attn);
        assert_eq!(card_glyph(&[&spawning, &fail], Tier::Unicode, 0), Some(('x', Register::Err)));
        assert_eq!(card_glyph(&[&spawning, &done], Tier::Unicode, 0), Some(('✓', Register::Calm)));
        // But it outranks a parked session: `z` means nothing is happening.
        let sleep = rec(SessionState::Sleeping);
        assert_eq!(
            card_glyph(&[&sleep, &spawning], Tier::Unicode, 0),
            Some((launching(Tier::Unicode, 0), Register::Grey))
        );
    }

    /// A shell is `Running` from spawn to pane death (D15) — that is the
    /// pane being alive, never a command being in flight — so it must not
    /// wear the working spinner. Dogfood 2026-09-01: a shell idling at its
    /// prompt read as loading, forever.
    #[test]
    fn a_shell_never_spins() {
        let mut sh = rec(SessionState::Running);
        sh.kind = SessionKind::Bash;
        let agent = rec(SessionState::Running);
        assert!(!is_working(&sh));
        assert!(is_working(&agent));
        for tier in [Tier::Unicode, Tier::Ascii] {
            let idle = session_glyph(
                &rec(SessionState::Idle { stop_reason: StopReason::Unknown }),
                tier,
                0,
            );
            for f in 0..24 {
                let (g, reg) = session_glyph(&sh, tier, f);
                assert_ne!(g, spinner(tier, f), "the shell picked up the spinner");
                assert_eq!((g, reg), idle, "a live shell wears the idle mark, unmoving");
            }
            // And it carries no aggregate glyph of its own: nothing about an
            // open shell is abnormal, so the title starts at T[0].
            assert_eq!(card_glyph(&[&sh], tier, 0), None);
            // An agent on the same card still spins.
            assert_eq!(
                card_glyph(&[&sh, &agent], tier, 0),
                Some((spinner(tier, 0), Register::Grey))
            );
        }
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

    /// Uncertain is a WAITING state, not a `?`: one braille dot orbiting on a
    /// quarter of the spinner's cadence, so it never reads as work in
    /// progress. Deviates from 06 §4.2's `?` (STALE-MAP "Uncertain waits").
    #[test]
    fn unknown_waits_instead_of_asking() {
        use unicode_width::UnicodeWidthChar;
        let unk = rec(SessionState::Unknown { reason: UnknownReason::DaemonRestarted });
        for tier in [Tier::Unicode, Tier::Ascii] {
            let (g, reg) = session_glyph(&unk, tier, 0);
            assert_ne!(g, '?', "the question mark is retired");
            assert_eq!(reg, Register::Grey, "waiting never leaves the grey ramp");
            assert_eq!(card_glyph(&[&unk], tier, 0), Some((g, Register::Grey)));
            // A frame of waiting is never a frame of working: the two glyph
            // sets are disjoint, so no still frame is ambiguous.
            for f in 0..40 {
                assert_eq!(
                    waiting(tier, f).width(),
                    Some(1),
                    "{:?} not one cell",
                    waiting(tier, f)
                );
                for w in 0..40 {
                    assert_ne!(
                        waiting(tier, f),
                        spinner(tier, w),
                        "waiting frame collides with the spinner"
                    );
                }
            }
        }
    }

    /// A turn parked on a backgrounded task gets its own mark, and it must
    /// not be mistakable for any other: not the spinner (work in flight in
    /// THIS pane), not `waiting` (we have lost track), not the idle mark, and
    /// never the calm `✓` (which would say the ticket is ready to review).
    /// It rides the slow cadence — no third speed on the board.
    /// An Esc-interrupted turn wears a mark of its own: the card fell through
    /// every arm and drew NOTHING for it, the look of a ticket with no session
    /// (author 2026-09-04). Still, not done, not lost, not plain idle, one cell.
    #[test]
    fn an_interrupted_turn_has_its_own_still_mark() {
        use unicode_width::UnicodeWidthChar;
        let cut = rec(SessionState::Idle { stop_reason: StopReason::Interrupted });
        for tier in [Tier::Unicode, Tier::Ascii] {
            let (g, reg) = session_glyph(&cut, tier, 0);
            assert_eq!(reg, Register::Grey, "an interrupt asks nothing of the user");
            assert_eq!(card_glyph(&[&cut], tier, 0), Some((g, Register::Grey)));
            assert_eq!(g.width(), Some(1));
            assert_ne!(g, if tier == Tier::Ascii { '+' } else { '✓' }, "not done");
            assert_ne!(g, if tier == Tier::Ascii { '.' } else { '◦' }, "not plain idle");
            assert_ne!(g, 'z', "not asleep");
            for f in 0..40 {
                assert_ne!(g, spinner(tier, f), "not working");
                assert_ne!(g, waiting(tier, f), "not lost");
                assert_ne!(g, background(tier, f), "not parked");
                assert_eq!(session_glyph(&cut, tier, f).0, g, "still: it does not move");
            }
        }
        assert_eq!(state_word(&cut.state), "interrupted");
        // Under anything in flight or finished, over a lost session.
        let busy = rec(SessionState::Running);
        let done = rec(SessionState::Idle { stop_reason: StopReason::EndTurn });
        let lost = rec(SessionState::Unknown { reason: UnknownReason::DaemonRestarted });
        assert_eq!(
            card_glyph(&[&cut, &busy], Tier::Unicode, 0),
            Some((spinner(Tier::Unicode, 0), Register::Grey))
        );
        assert_eq!(card_glyph(&[&cut, &done], Tier::Unicode, 0), Some(('✓', Register::Calm)));
        assert_eq!(
            card_glyph(&[&lost, &cut], Tier::Unicode, 0),
            Some((interrupted(Tier::Unicode), Register::Grey))
        );
    }

    #[test]
    fn a_parked_turn_has_its_own_slow_mark() {
        use unicode_width::UnicodeWidthChar;
        let parked = rec(SessionState::Idle { stop_reason: StopReason::Background });
        for tier in [Tier::Unicode, Tier::Ascii] {
            let (g, reg) = session_glyph(&parked, tier, 0);
            assert_eq!(reg, Register::Grey, "a parked turn asks nothing of the user");
            assert_eq!(card_glyph(&[&parked], tier, 0), Some((g, Register::Grey)));
            assert_ne!(g, if tier == Tier::Ascii { '+' } else { '✓' }, "not done");
            assert_ne!(g, if tier == Tier::Ascii { '.' } else { '◦' }, "not plain idle");
            for f in 0..40 {
                let b = background(tier, f);
                assert_eq!(b.width(), Some(1), "{b:?} not one cell");
                for w in 0..40 {
                    assert_ne!(b, spinner(tier, w), "collides with the spinner");
                    assert_ne!(b, waiting(tier, w), "collides with waiting");
                }
            }
            // Slow cadence, and it wraps.
            for f in 0..3 {
                assert_eq!(background(tier, f), background(tier, f + 1), "held four ticks");
            }
            assert_ne!(background(tier, 3), background(tier, 4), "and then it steps");
            let frames = if tier == Tier::Ascii { BG_ASCII } else { BG_UNICODE };
            let cycle = frames.len() * SLOW_STEP_TICKS;
            assert_eq!(background(tier, 0), background(tier, cycle), "wraps cleanly");
        }
    }

    #[test]
    fn an_owed_ticket_has_its_own_slow_mark() {
        use unicode_width::UnicodeWidthChar;
        for tier in [Tier::Unicode, Tier::Ascii] {
            for f in 0..40 {
                let q = queued(tier, f);
                assert_eq!(q.width(), Some(1), "{q:?} not one cell");
                assert!(!(0x2500..=0x259F).contains(&(q as u32)), "{q:?} is in the range L1 bans");
                assert!(!is_still_mark(q, tier), "{q:?} impersonates a still mark");
                assert!(!matches!(q, 'x' | '!' | '~' | '=' | '*' | '-'), "{q:?} is another mark");
                for w in 0..40 {
                    assert_ne!(q, spinner(tier, w), "collides with the spinner");
                    assert_ne!(q, waiting(tier, w), "collides with waiting");
                    assert_ne!(q, background(tier, w), "collides with the parked bar");
                }
            }
            for f in 0..3 {
                assert_eq!(queued(tier, f), queued(tier, f + 1), "held four ticks");
            }
            assert_ne!(queued(tier, 3), queued(tier, 4), "and then it steps");
            let frames = if tier == Tier::Ascii { QUEUED_ASCII } else { QUEUED_UNICODE };
            let cycle = frames.len() * SLOW_STEP_TICKS;
            assert_eq!(queued(tier, 0), queued(tier, cycle), "wraps cleanly");
        }
    }

    /// The owed mark takes an empty slot and the still marks, and yields to
    /// everything that moves or shouts.
    #[test]
    fn queued_over_yields_to_moving_and_loud_marks() {
        for tier in [Tier::Unicode, Tier::Ascii] {
            let owed = Some((queued(tier, 7), Register::Grey));
            assert_eq!(queued_over(None, tier, 7), owed, "an empty slot");
            let done = rec(SessionState::Idle { stop_reason: StopReason::EndTurn });
            let idle = rec(SessionState::Idle { stop_reason: StopReason::Unknown });
            let asleep = rec(SessionState::Sleeping);
            let cut = rec(SessionState::Idle { stop_reason: StopReason::Interrupted });
            for still in [&done, &idle, &asleep, &cut] {
                let g = card_glyph(&[still], tier, 7);
                assert_eq!(queued_over(g, tier, 7), owed, "{:?} is still", still.state);
            }
            assert_eq!(
                queued_over(Some((done_unread(tier), Register::Calm)), tier, 7),
                owed,
                "the heavy check is still too"
            );
            let busy = rec(SessionState::Running);
            let parked = rec(SessionState::Idle { stop_reason: StopReason::Background });
            let lost = rec(SessionState::unknown());
            let mut launching = rec(SessionState::Spawning);
            launching.pending_submit = true;
            let attn = rec(SessionState::RequiresAction { reason: Reason::Permission });
            let crashed = rec(SessionState::Exited { reason: ExitReason::Crashed });
            for loud in [&busy, &parked, &lost, &launching, &attn, &crashed] {
                let g = card_glyph(&[loud], tier, 7);
                assert!(g.is_some());
                assert_eq!(queued_over(g, tier, 7), g, "{:?} keeps its mark", loud.state);
            }
        }
    }

    /// A really-working session on the same ticket outranks a parked one: the
    /// spinner is the louder and truer thing to say about that card.
    #[test]
    fn working_outranks_a_parked_turn_on_one_card() {
        let parked = rec(SessionState::Idle { stop_reason: StopReason::Background });
        let busy = rec(SessionState::Running);
        assert_eq!(
            card_glyph(&[&parked, &busy], Tier::Unicode, 0),
            Some((spinner(Tier::Unicode, 0), Register::Grey))
        );
    }

    /// Slower is the whole point: the waiting glyph holds for four redraw
    /// ticks (400 ms) where the spinner moves every one, and it still cycles.
    #[test]
    fn waiting_is_four_times_slower_than_working() {
        for tier in [Tier::Unicode, Tier::Ascii] {
            for f in 0..3 {
                assert_eq!(waiting(tier, f), waiting(tier, f + 1), "held for four ticks");
                assert_ne!(spinner(tier, f), spinner(tier, f + 1), "the spinner still steps");
            }
            assert_ne!(waiting(tier, 3), waiting(tier, 4), "and then it steps");
            let frames = if tier == Tier::Ascii { WAIT_ASCII } else { WAIT_UNICODE };
            let cycle = frames.len() * SLOW_STEP_TICKS;
            assert_eq!(waiting(tier, 0), waiting(tier, cycle), "wraps cleanly");
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
        assert_eq!(card_glyph(&[&plan], Tier::Unicode, 0), Some(('≡', Register::Attn)));
        assert_eq!(card_glyph(&[&plan], Tier::Ascii, 0), Some(('=', Register::Attn)));
        // A co-pending non-plan reason keeps the generic bang on the card.
        assert_eq!(card_glyph(&[&plan, &perm], Tier::Unicode, 0), Some(('!', Register::Attn)));
        assert_eq!(session_glyph(&plan, Tier::Unicode, 0), ('≡', Register::Attn));
        assert_eq!(session_glyph(&perm, Tier::Unicode, 0).0, '!');
        assert_eq!('≡'.width(), Some(1));
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
        assert_eq!(card_glyph(&[&done, &fail], Tier::Unicode, 0), Some(('x', Register::Err)));
        assert_eq!(card_glyph(&[&sleep, &done], Tier::Unicode, 0), Some(('✓', Register::Calm)));
    }

    #[test]
    fn z_requires_all_sessions_sleeping() {
        let sleep = rec(SessionState::Sleeping);
        let run = rec(SessionState::Running);
        assert_eq!(card_glyph(&[&sleep], Tier::Unicode, 0), Some(('z', Register::Dormant)));
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
        let run = rec(SessionState::Running);
        assert_eq!(session_glyph(&run, Tier::Ascii, 0).0, '|');
        assert_eq!(session_glyph(&run, Tier::Unicode, 0).0, '⠋');
        assert_eq!(queued(Tier::Ascii, 0), '(');
        assert_eq!(queued(Tier::Unicode, 0), '⠑');
    }

    /// The suggestion mark is its own thing at both tiers, and one cell wide
    /// wherever it lands — the chrome it rides is width-critical (06 §4.1).
    #[test]
    fn suggest_mark_is_one_cell_at_both_tiers() {
        use unicode_width::UnicodeWidthChar;
        assert_eq!(suggest_mark(Tier::Unicode), '◦');
        assert_eq!(suggest_mark(Tier::Ascii), '*');
        assert_eq!('◦'.width(), Some(1));
    }

    /// The branch glyph and both arrows are one cell at both tiers and stay
    /// out of the banned box range — the header row is width-critical.
    #[test]
    fn branch_marks_are_one_cell_at_both_tiers() {
        use unicode_width::UnicodeWidthChar;
        for tier in [Tier::Unicode, Tier::Ascii] {
            for c in [branch_mark(tier), ahead_mark(tier), behind_mark(tier), auto_mark(tier)] {
                assert_eq!(c.width(), Some(1), "{c:?}");
                assert!(!(0x2500..=0x259F).contains(&(c as u32)), "{c:?}");
            }
        }
        assert_eq!(
            (branch_mark(Tier::Unicode), ahead_mark(Tier::Unicode), behind_mark(Tier::Unicode)),
            ('⎇', '↑', '↓')
        );
        assert_eq!(
            (branch_mark(Tier::Ascii), ahead_mark(Tier::Ascii), behind_mark(Tier::Ascii)),
            ('&', '^', 'v')
        );
    }

    /// The unread done mark pins its literal and its width, and is not the
    /// read one: the pair is the whole signal at the glyph level.
    #[test]
    fn done_unread_is_the_heavy_check_one_cell_wide() {
        use unicode_width::UnicodeWidthChar;
        assert_eq!(done_unread(Tier::Unicode), '✔');
        assert_eq!(done_unread(Tier::Ascii), '+');
        assert_eq!('✔'.width(), Some(1));
        assert!(!(0x2500..=0x259F).contains(&('✔' as u32)));
        let done = SessionRecord::new(
            uuid::Uuid::from_u128(1),
            SessionKind::Claude,
            ulid::Ulid(1),
            vec!["claude".into()],
            "/repo".into(),
            SessionState::Idle { stop_reason: StopReason::EndTurn },
        );
        let (read, reg) = card_glyph(&[&done], Tier::Unicode, 0).expect("a done glyph");
        assert_eq!(reg, Register::Calm);
        assert_ne!(read, done_unread(Tier::Unicode), "read and unread differ in shape");
    }

    #[test]
    fn uppercase_iff_human_required() {
        // 06 §3.3's case rule, spot-checked.
        assert_eq!(state_word(&SessionState::Running), "working");
        assert_eq!(state_word(&SessionState::RequiresAction { reason: Reason::Plan }), "NEEDS YOU");
        assert_eq!(state_word(&SessionState::Failed { reason: FailReason::Server }), "FAILED");
        assert_eq!(state_word(&SessionState::Sleeping), "sleeping");
    }
}

/// The six cells a dialog's frame is drawn from. The ONE place the L1 law
/// admits box drawing (author 2026-09-03, T-158): a floating dialog's own
/// perimeter, and nothing else — `test_no_drawn_structure` checks every
/// box-drawing cell on screen against the frames the draw recorded. The
/// ascii tier (mono) spells the same frame with `+ - |`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FrameGlyphs {
    pub tl: char,
    pub tr: char,
    pub bl: char,
    pub br: char,
    pub h: char,
    pub v: char,
}

pub(crate) fn frame_set(tier: Tier) -> FrameGlyphs {
    match tier {
        Tier::Unicode => {
            FrameGlyphs { tl: '╭', tr: '╮', bl: '╰', br: '╯', h: '─', v: '│' }
        }
        Tier::Ascii => FrameGlyphs { tl: '+', tr: '+', bl: '+', br: '+', h: '-', v: '|' },
    }
}
