//! Claude Code passive recovery: startup modals, status-file edges and bounded
//! transcript evidence. Only normalized observations leave this module.

use super::tail::{self, TailCursor};
use crate::agents::{AgentRecovery, RecoveryChannel, RecoveryObservation, RecoverySample};
use mesimon_core::adopt::{classify_tail_record, SessionsPidFile, TailEvent, TailTool};
use mesimon_core::attention::{self, Signal, TailHint};
use mesimon_core::board::{
    FailReason, Reason, SessionRecord, SessionState, StopReason, UnknownReason,
};
use std::path::PathBuf;

const TAIL_QUIET_MS: u64 = 45_000;
/// How often a held plan or question is re-read off the transcript tail
/// while its dialog is open. The machine's stale clock is fifteen minutes;
/// one 64 KiB read a minute per waiting session keeps it re-armed with
/// margin to spare (T-363).
const WAIT_AFFIRM_MS: u64 = 60_000;
/// How often a native failed turn whose class the frame could not say is
/// re-read off the tail, and for how long (T-659). The race is the frame
/// beating Claude Code's write of the error row by milliseconds; a minute
/// of two-second reads covers a slow flush and then gives up on `unknown`.
const FAILURE_REREAD_MS: u64 = 2_000;
const FAILURE_REREAD_SPELL_MS: u64 = 60_000;
/// How often a background park is re-proved off Claude Code's session file
/// while it reads `busy` (T-724). The park clock is ten minutes; a minute
/// keeps it re-armed with margin and puts one decision a minute in the feed.
const PARK_AFFIRM_MS: u64 = 60_000;

#[derive(Default)]
pub(super) struct ClaudeRecovery {
    startup_stage: u8,
    cursor: Option<TailCursor>,
    status: Option<StatusProbe>,
    /// Last time a held plan/question was looked for on the tail; `None`
    /// since the cursor was minted, so the first poll looks at once.
    affirmed_at: Option<u64>,
    /// Whether this `Unknown` spell has been seeded from the resting tail.
    rested: bool,
    /// Last time a native `Failed { Unknown }` was re-read for its class.
    failure_read_at: Option<u64>,
    /// Last time a background park was proved by a `busy` session file.
    park_affirmed_at: Option<u64>,
    /// The plan or question call the tail last showed with no result after
    /// it, and its tool: what a departure is keyed on (T-957).
    dialog: Option<(TailTool, String)>,
    /// The held spell (`state_changed_at`) whose call was last looked for
    /// off the tail, for a cursor that came after the call.
    dialog_read: Option<Option<u64>>,
    /// The dialog whose result is the last word the cursor read, and the
    /// result's stamp: an interrupt read next is that dialog's Esc, in this
    /// read or the next.
    ended: Option<((TailTool, String), Option<u64>)>,
    /// The call the last departure was for, so a spell's first look back
    /// does not give it twice.
    departed: Option<String>,
}

/// The transcript's word for a held dialog: the tail event that says THIS
/// reason's tool is still pending. Only the two interaction tools have one —
/// a generic permission dialog leaves a tool call the transcript cannot tell
/// from a running tool.
fn pending_dialog(reason: Reason) -> Option<(TailTool, TailHint)> {
    match reason {
        Reason::Plan => Some((TailTool::ExitPlanMode, TailHint::ExitPlanMode)),
        Reason::Question => Some((TailTool::AskUserQuestion, TailHint::AskUserQuestion)),
        _ => None,
    }
}

fn abort_only(record: &SessionRecord) -> bool {
    !record.observe_only()
        && (record.state == SessionState::Running || attention::is_attention(&record.state))
}

/// A native failed turn the frame could not class (T-659): the mod relays
/// `unknown`, and the adapter's read of the tail at the frame found no error
/// row yet. The re-read below is the only road that corrects it.
fn parked(record: &SessionRecord) -> bool {
    !record.observe_only()
        && record.state == SessionState::Idle { stop_reason: StopReason::Background }
}

fn failure_unread(record: &SessionRecord) -> bool {
    record.native
        && record.state == SessionState::Failed { reason: FailReason::Unknown }
        && record.transcript_path.is_some()
}

/// `failure_unread` inside its re-read spell: the class may still turn out
/// a limit (`RateLimit` → QUOTA), so the seat takes no words yet (T-899
/// review): pasted, they would meet the limit T-808 holds them against.
/// The spell ended unclassed, or a class read, and it takes them as any
/// `Failed` does. The one clock both the re-read and the queue ask.
pub(crate) fn failure_being_read(record: &SessionRecord, now: u64) -> bool {
    failure_unread(record)
        && now.saturating_sub(record.state_changed_at.unwrap_or(now)) < FAILURE_REREAD_SPELL_MS
}

fn observation(signal: Signal, source: &'static str) -> RecoveryObservation {
    RecoveryObservation { signal, source, preview: None, dialog: None, at: None }
}

impl AgentRecovery for ClaudeRecovery {
    fn needs_poll(&mut self, record: &SessionRecord, channel: RecoveryChannel, now: u64) -> bool {
        match channel {
            RecoveryChannel::Startup => {
                if record.state != SessionState::Spawning {
                    return false;
                }
                let age = now.saturating_sub(record.state_changed_at.unwrap_or(now));
                if age >= 30_000 && self.startup_stage < 2 {
                    self.startup_stage = 2;
                    true
                } else if age >= 10_000 && self.startup_stage < 1 {
                    self.startup_stage = 1;
                    true
                } else {
                    false
                }
            }
            RecoveryChannel::Activity => {
                !record.observe_only() && record.state == SessionState::Running
            }
            RecoveryChannel::Status => {
                let eligible = parked(record)
                    || (!record.observe_only()
                        && (record.state == SessionState::Running
                            || record.state
                                == SessionState::RequiresAction { reason: Reason::Permission }));
                if !eligible {
                    self.status = None;
                }
                eligible
            }
            RecoveryChannel::Transcript => {
                if failure_unread(record) {
                    return failure_being_read(record, now)
                        && self
                            .failure_read_at
                            .is_none_or(|at| now.saturating_sub(at) >= FAILURE_REREAD_MS);
                }
                self.failure_read_at = None;
                let eligible = record.transcript_path.is_some()
                    && ((record.observe_only() && record.state.is_live())
                        || (!record.observe_only()
                            && matches!(record.state, SessionState::Unknown { .. }))
                        || abort_only(record));
                if !eligible {
                    self.cursor = None;
                    self.rested = false;
                    self.forget_dialog();
                }
                eligible
            }
        }
    }

    fn poll(
        &mut self,
        record: &SessionRecord,
        sample: RecoverySample,
        now: u64,
    ) -> Vec<RecoveryObservation> {
        match sample {
            RecoverySample::Startup { has_output: bytes, title } => {
                let osc0 = title.is_some_and(|t| t.contains("Claude") || t.contains('✳'));
                // The first probe only identifies a modal; missing output has
                // a longer deadline so a slow startup is not called missing.
                if self.startup_stage == 1 && !(bytes && !osc0) {
                    return Vec::new();
                }
                let resume = record.argv.iter().any(|arg| arg == "--resume");
                vec![observation(Signal::SpawnProbe { bytes, osc0, resume }, "probe")]
            }
            RecoverySample::Activity { last_output_ms } => {
                let quiet = pane_quiet_ms();
                if now.saturating_sub(last_output_ms) < quiet {
                    return Vec::new();
                }
                // A silent pane is not a dead turn while the transcript
                // says one is in flight (T-439): a live Claude Code pane
                // wrote no byte through a 63 s suite, its result and the
                // next call, so the suite read as an Esc and the card wore
                // "interrupted" until the tool's PostToolUse put it back.
                // One 64 KiB read a second, and only past the quiet
                // threshold on a Running pane.
                let in_flight = record
                    .transcript_path
                    .as_deref()
                    .is_some_and(|p| tail::turn_in_flight(std::path::Path::new(p), now, quiet));
                if in_flight {
                    return Vec::new();
                }
                // Nor while Claude Code's own session file calls the turn
                // busy (T-729): a model at high effort thought for two
                // minutes after a tool result, pane and transcript both
                // still, and the card read "interrupted" each time. An Esc
                // flips the file at the keypress, so only a turn that is
                // really over gets past this.
                if self.claude_busy(record, now) {
                    return Vec::new();
                }
                vec![observation(Signal::PaneQuiet, "activity")]
            }
            RecoverySample::Status => self.status_observation(record, now).into_iter().collect(),
            RecoverySample::Transcript => self.transcript_observations(record, now),
        }
    }
}

impl ClaudeRecovery {
    /// A new cursor, or a spell the transcript channel no longer reads.
    fn forget_dialog(&mut self) {
        self.dialog = None;
        self.dialog_read = None;
        self.ended = None;
        self.departed = None;
    }

    /// Claude Code's own session file for this record, found once and
    /// re-looked-for every `STATUS_FILE_RETRY_MS` while it is missing.
    fn session_file(&mut self, record: &SessionRecord, now: u64) -> Option<SessionsPidFile> {
        let claude_id = record.claude_session_id.unwrap_or(record.id);
        let probe = self.status.get_or_insert(StatusProbe {
            path: None,
            looked_at: 0,
            near_idle: None,
            waiting_spell: None,
        });
        if probe.path.is_none() && now.saturating_sub(probe.looked_at) >= STATUS_FILE_RETRY_MS {
            probe.path = crate::census::status_file_for(&crate::census::claude_home(), claude_id);
            probe.looked_at = now;
        }
        let path = probe.path.as_ref()?;
        let Ok(text) = std::fs::read_to_string(path) else {
            probe.path = None;
            return None;
        };
        let pf = serde_json::from_str::<SessionsPidFile>(&text).ok()?;
        if pf.session_id != Some(claude_id) {
            probe.path = None;
            return None;
        }
        Some(pf)
    }

    /// Claude Code's own session file calls the turn busy. The one word
    /// for a model thinking past a minute after a tool result, which writes
    /// neither pane nor transcript while it does (T-729, T-731).
    fn claude_busy(&mut self, record: &SessionRecord, now: u64) -> bool {
        self.session_file(record, now).is_some_and(|pf| pf.status.as_deref() == Some("busy"))
    }

    fn status_observation(
        &mut self,
        record: &SessionRecord,
        now: u64,
    ) -> Option<RecoveryObservation> {
        let since = record.state_changed_at.unwrap_or(now);
        let pf = self.session_file(record, now)?;
        let probe = self.status.as_mut()?;
        // A park the hooks have gone quiet on is still earned while Claude
        // Code calls the session busy: a subagent deep in one reasoning step
        // sends nothing at all (T-724).
        if parked(record) {
            let due =
                self.park_affirmed_at.is_none_or(|at| now.saturating_sub(at) >= PARK_AFFIRM_MS);
            if pf.status.as_deref() != Some("busy") || !due {
                return None;
            }
            self.park_affirmed_at = Some(now);
            return Some(observation(Signal::StatusFileBusy, "status"));
        }
        if record.state != SessionState::Running {
            return probe
                .permission_resumed(pf.status.as_deref(), pf.status_updated_at, since)
                .then(|| observation(Signal::StatusFilePermissionResumed, "status"));
        }
        probe.waiting_spell = None;
        if !probe.confirms_idle(pf.status.as_deref(), pf.status_updated_at, since, now) {
            return None;
        }
        // Native idle precedes Stop on successful turns too; the transcript
        // distinguishes that completion from a recordless Esc interruption.
        let turn_done = record
            .transcript_path
            .as_deref()
            .is_some_and(|path| tail::turn_done_since(std::path::Path::new(path), since));
        Some(observation(Signal::StatusFileIdle { turn_done }, "status"))
    }

    fn transcript_observations(
        &mut self,
        record: &SessionRecord,
        now: u64,
    ) -> Vec<RecoveryObservation> {
        let Some(path) = record.transcript_path.as_deref().map(PathBuf::from) else {
            return Vec::new();
        };
        if failure_unread(record) {
            // A later spell mints its own cursor: the lines of this one are
            // the failure's, and nothing the abort-only class reads.
            self.cursor = None;
            self.failure_read_at = Some(now);
            return tail::api_error(&path)
                .map(|error| (super::hooks::failure_class(Some(&error.class)), error.text))
                .filter(|(class, _)| *class != attention::StopFailureClass::Unknown)
                .map(|(class, text)| RecoveryObservation {
                    signal: Signal::StopFailure { class },
                    preview: text.as_deref().and_then(super::hooks::failure_detail),
                    source: "tail",
                    dialog: None,
                    at: None,
                })
                .into_iter()
                .collect();
        }
        let abort_only = abort_only(record);
        let fresh = self.cursor.as_ref().is_none_or(|cursor| cursor.path != path);
        let unknown = matches!(record.state, SessionState::Unknown { .. });
        if fresh || !unknown {
            self.rested = false;
        }
        // Only an Unknown record may seed low-confidence state from resting
        // history, once per spell. Owned live sessions recover explicit
        // current-spell aborts. A spell begun on a living cursor is seeded
        // too: the stale clock demotes a held dialog the tail stopped
        // showing, and that cursor saw no new line to read (T-649: a card
        // wore `Unknown`'s turning glyph for good over a finished turn).
        // The restart's seed has one proof before the hint (T-663): the
        // turn the last daemon left open closed on the tail after it did.
        let mut closed = None;
        let backfill = if unknown && !self.rested {
            self.rested = true;
            if turn_closed_in_the_gap(record, &path) {
                closed = Some(observation(Signal::TranscriptTurnEnded, "tail"));
                None
            } else {
                // A quiet tail is no rest while Claude Code calls the turn
                // busy (T-731): a model thinking after a tool result across
                // a restart seeded `Idle` here, and the crown was told the
                // turn had finished. The record stays `Unknown` until a
                // hook or the next line says.
                resting_hint(&path, now)
                    .filter(|h| *h != TailHint::StaleQuiet || !self.claude_busy(record, now))
            }
        } else if fresh && abort_only {
            record
                .state_changed_at
                .filter(|since| tail::aborted_since(&path, *since))
                .map(|_| TailHint::AbortedMidStream)
        } else {
            None
        };
        if fresh {
            self.cursor = Some(TailCursor::at_end(path.clone(), now));
            self.affirmed_at = None;
            self.forget_dialog();
        }
        // The dialog the record holds, on a seat this adapter owns.
        let held = match &record.state {
            SessionState::RequiresAction { reason } if abort_only => {
                pending_dialog(*reason).map(|(tool, _)| tool)
            }
            _ => None,
        };
        let Some(cursor) = self.cursor.as_mut() else { return Vec::new() };
        let lines = cursor.poll(now);
        let quiet = now.saturating_sub(cursor.grew_at);
        let mut hints = Vec::new();
        hints.extend(backfill.map(|hint| (hint, None)));
        // A held plan or question is restated off the tail while its dialog
        // is open, so the machine's stale clock never fires on a wait that is
        // real (T-363: three plan cards on the simbly board went `?` fifteen
        // minutes in, with the agent still waiting). The clearing roads are
        // untouched — the answer's `PostToolUse`, the Esc's aborted record
        // below, the next prompt — and a tail that no longer shows the
        // dialog (a lost answer frame) affirms nothing, so the clock still
        // demotes that one.
        // A usage limit is restated the same way (T-802): the turn closed on
        // the limit's error row, and while that row is still the last word
        // nothing has run since — the person has not been back. The clock
        // still demotes a wait whose clearing frame was lost, since a prompt
        // that got through writes past the row.
        let mut limited = None;
        if record.state == (SessionState::RequiresAction { reason: Reason::QuotaResume })
            && abort_only
            && self.affirmed_at.is_none_or(|at| now.saturating_sub(at) >= WAIT_AFFIRM_MS)
        {
            self.affirmed_at = Some(now);
            limited = tail::api_error(&path)
                .filter(|error| {
                    super::hooks::failure_class(Some(&error.class))
                        == attention::StopFailureClass::RateLimit
                })
                .map(|_| {
                    observation(
                        Signal::StopFailure { class: attention::StopFailureClass::RateLimit },
                        "tail",
                    )
                });
        }
        if let SessionState::RequiresAction { reason } = &record.state {
            if let Some((tool, hint)) = pending_dialog(*reason) {
                if abort_only
                    && self.affirmed_at.is_none_or(|at| now.saturating_sub(at) >= WAIT_AFFIRM_MS)
                {
                    self.affirmed_at = Some(now);
                    let held = record
                        .state_changed_at
                        .is_some_and(|at| now.saturating_sub(at) >= WAIT_AFFIRM_MS);
                    match tail::last_event(&path) {
                        Some(TailEvent::NeedsHuman { tool: pending }) if pending == tool => {
                            hints.push((hint, None));
                        }
                        // A minute on, the turn's close is still the last
                        // word: the lead never called the dialog, whose call
                        // would be on the tail by now. A frame with no
                        // `agent_id` from a fork after the turn's `Stop`
                        // (T-649, mod road) held a finished ticket at
                        // QUESTION for fifteen minutes.
                        Some(TailEvent::TurnComplete) if held => {
                            hints.push((TailHint::TurnComplete, None));
                        }
                        _ => {}
                    }
                }
            }
        }
        // A dialog ends on the transcript when its own call's result is
        // written (T-957): a plan sent back with words, an Esc, an accept.
        // A person's refusal fires no hook (T-447), and neither the hook set
        // nor the classic mod sends the next ordinary tool's `PreToolUse`
        // that the machine's refusal road waits for, so the card wore
        // needs-you until the turn's end. A newer call after the result is a
        // dialog that stands again; an interrupt right after it is the Esc
        // that stopped the turn.
        let mut answered = None;
        let mut aborted_at = None;
        for line in &lines {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else { continue };
            let stamp = mesimon_core::adopt::record_ms(&value);
            if let Some((tool, id)) = tail::dialog_call(&value) {
                self.dialog = Some((tool, id.to_string()));
                self.ended = None;
                answered = None;
            } else if let Some(call) =
                self.dialog.take_if(|(_, id)| tail::carries_result_of(&value, id))
            {
                self.ended = Some((call.clone(), stamp));
                answered = Some((call, false, stamp));
            } else if tail::speaks(&value) {
                let ended = self.ended.take();
                if classify_tail_record(&value) == TailEvent::Aborted {
                    answered = ended.map(|(call, at)| (call, true, stamp.or(at)));
                }
            }
            // A person's prompt after an interrupt in the same read is a newer
            // turn (round 2): the interrupt ended the one before it, and says
            // nothing of this one, at High or Low.
            if mesimon_core::adopt::user_prompt(&value).is_some() {
                answered = answered.filter(|(_, interrupted, _)| !interrupted);
                hints.retain(|(kind, _)| *kind != TailHint::AbortedMidStream);
                aborted_at = None;
            }
            match classify_tail_record(&value) {
                TailEvent::Aborted => {
                    hints.push((TailHint::AbortedMidStream, None));
                    aborted_at = stamp;
                }
                _ if abort_only => {}
                TailEvent::AssistantText { text } => {
                    hints.push((TailHint::AssistantText, Some(crate::census::sanitize(&text))));
                }
                TailEvent::NeedsHuman { tool: TailTool::AskUserQuestion } => {
                    hints.push((TailHint::AskUserQuestion, None));
                }
                TailEvent::NeedsHuman { tool: TailTool::ExitPlanMode } => {
                    hints.push((TailHint::ExitPlanMode, None));
                }
                TailEvent::TurnComplete => hints.push((
                    TailHint::TurnComplete,
                    mesimon_core::adopt::assistant_text(&value).map(crate::census::sanitize),
                )),
                TailEvent::ToolInFlight => hints.push((
                    TailHint::ToolInFlight,
                    mesimon_core::adopt::assistant_text(&value).map(crate::census::sanitize),
                )),
                TailEvent::Latch | TailEvent::Other => {}
            }
        }
        // A cursor minted after the held dialog's call (a restart, a turn
        // resumed straight onto a plan) learns it off the tail, once a spell:
        // the call still open, or already ended (round 1: a refusal in the
        // restart's window, before this read), fenced by the spell, since
        // the tail holds every earlier dialog of the same kind too.
        if let Some(tool) = held
            .filter(|_| self.dialog.is_none() && self.dialog_read != Some(record.state_changed_at))
        {
            self.dialog_read = Some(record.state_changed_at);
            match tail::open_dialog(&path) {
                Some(open) => self.dialog = Some((open.tool, open.id)),
                None if answered.is_none() => {
                    let closed = record
                        .state_changed_at
                        .and_then(|since| tail::closed_dialog(&path, tool, since))
                        .filter(|c| self.departed.as_deref() != Some(c.id.as_str()));
                    if let Some(c) = closed {
                        if c.last {
                            self.ended = Some(((tool, c.id.clone()), c.at));
                        }
                        answered = Some(((tool, c.id), c.interrupted, c.at));
                    }
                }
                None => {}
            }
        }
        // The departure, given where no road sends the refusal road's frame.
        // An interrupt read a poll after the result finds the record on the
        // `Running` the departure left it for. The daemon drops it while
        // another dialog is projected for the seat, and an interrupt older
        // than the seat's last prompt (`at`).
        let left = answered
            .filter(|((tool, _), interrupted, _)| {
                held == Some(*tool) || (*interrupted && record.state == SessionState::Running)
            })
            .map(|((_, id), interrupted, at)| {
                self.departed = Some(id.clone());
                RecoveryObservation {
                    signal: Signal::DialogEnded { interrupted },
                    preview: None,
                    source: "dialog_result",
                    dialog: Some(id),
                    at: at.filter(|_| interrupted),
                }
            });
        if !abort_only
            && hints.is_empty()
            && !cursor.tools.is_busy()
            && quiet >= TAIL_QUIET_MS
            && record.state == SessionState::Running
            && !self.claude_busy(record, now)
        {
            hints.push((TailHint::StaleQuiet, None));
        }
        closed
            .into_iter()
            .chain(limited)
            .chain(left)
            .chain(hints.into_iter().map(|(kind, preview)| RecoveryObservation {
                signal: Signal::TranscriptHint { kind },
                preview,
                source: "tail",
                dialog: None,
                at: aborted_at.filter(|_| kind == TailHint::AbortedMidStream),
            }))
            .collect()
    }
}

/// The restart's one proof (T-663). Reconcile leaves a Claude whose turn the
/// last daemon saw open at `Unknown{DaemonRestarted}` with `state_changed_at`
/// untouched, so that stamp is still the moment the spell began; and the
/// transcript's last word dates the turn's close at or after it. A `Stop`
/// sent into the restart's window is lost, never queued (`mesimon hook`
/// fails open by design), and nothing later says it — the next edge is the
/// next prompt — so the seed's Low `TurnComplete` left two tickets in
/// IN PROGRESS with their turns over (the author's board, 2026-10-05). The
/// same `turn_done_since` the status-file probe trusts at Medium: an undated
/// close, or one older than the spell (the next turn's recordless Esc), is
/// the resting hint at Low as before, and a prompt or a tool result after
/// the close is an open turn and no close at all. Only the restart's own
/// `Unknown`: the stale clock's has a daemon listening, whose hooks are the
/// word for a turn it did not hear.
fn turn_closed_in_the_gap(record: &SessionRecord, path: &std::path::Path) -> bool {
    record.state == (SessionState::Unknown { reason: UnknownReason::DaemonRestarted })
        && record.state_changed_at.is_some_and(|since| tail::turn_done_since(path, since))
}
/// Pane silent past this while `Running` means the turn is no longer in
/// flight — the recordless Esc-interrupt catch (spike S-E: an interrupt fires
/// no hook, and one landing before the first assistant output writes nothing
/// to the transcript either; the pane byte stream is the only evidence left).
///
/// This is the FALLBACK, and the number is sized for a fallback. The 8 s it
/// started at rested on "a turn in flight repaints sub-second (spinner)",
/// which was true when measured and is not now: on Claude Code 2.1.257 a
/// working pane holds `#{window_activity}` still for 6–10 s as a matter of
/// course and for up to ~50 s while the model streams a large tool input (an
/// `Edit`/`Write` payload — nothing paints until the call is whole). Under
/// 8 s the probe fired four times in ninety seconds on one ticket, and each
/// verdict blanked the card until the next `PostToolUse` put the spinner
/// back (dogfood 2026-09-02, T-71; over the whole activity log 19 of the
/// probe's 40 verdicts were followed by a `PostToolUse` or `Stop`, i.e. by
/// the turn it had just declared dead). The primary catch — the transcript's
/// `[Request interrupted by user]` record, `poll_tails`' abort-only class —
/// lands in ~2 s regardless, and post-interrupt painting already held the
/// pane "active" for 60–80 s live (STALE-MAP, T-50), so the recordless case
/// was never a fast one. Sixty seconds clears every working silence
/// measured and costs that rare case a minute it was mostly paying anyway.
///
/// And silence alone no longer decides (T-439, 2026-09-23): a live Claude
/// Code 2.1.280 pane held `#{window_activity}` still through a 63 s test
/// suite, its result and the next call — Ink writes nothing while the frame
/// does not change, and what froze that frame is unmeasured — and the card
/// wore "interrupted" for 47 s of real work. The verdict is held while
/// `tail::turn_in_flight` finds a tool with no result or a reply stamped
/// inside the window; the recordless Esc leaves neither.
const PANE_QUIET_MS: u64 = 60_000;
/// A `status: idle` in Claude's session file counts only when stamped this
/// far after the Running spell began: the previous turn's `idle` write and
/// this turn's `UserPromptSubmit` hook can land in either order (a prompt
/// typed ahead is submitted the instant the turn ends), and a stale idle read
/// as this turn's would blank a card that just started working. That hazard
/// is milliseconds wide. An early Esc can also land inside that window;
/// such an idle must remain unchanged across a second probe before it counts.
const STATUS_IDLE_MARGIN_MS: u64 = 250;
/// How long a Running session with no session file goes between looks for
/// one (an older Claude Code writes none; a scan is ~40 small reads).
const STATUS_FILE_RETRY_MS: u64 = 30_000;

/// `probe_status_files`' memory of one Running session's session file.
struct StatusProbe {
    path: Option<std::path::PathBuf>,
    /// Epoch ms of the last failed search; 0 means never looked.
    looked_at: u64,
    near_idle: Option<(u64, u64)>,
    waiting_spell: Option<(u64, u64)>,
}

impl StatusProbe {
    fn permission_resumed(&mut self, status: Option<&str>, at: Option<u64>, since: u64) -> bool {
        if status == Some("waiting") {
            self.waiting_spell = at.map(|stamp| (since, stamp));
            return false;
        }
        let resumed = status == Some("busy")
            && self.waiting_spell.is_some_and(|(spell, waiting)| {
                spell == since && at.is_some_and(|stamp| stamp > since && stamp > waiting)
            });
        if status != Some("busy") {
            self.waiting_spell = None;
        }
        resumed
    }

    /// Near-boundary idle evidence is delayed, never discarded forever. A
    /// changed status/stamp cancels the confirmation; stale stamps never count.
    fn confirms_idle(
        &mut self,
        status: Option<&str>,
        at: Option<u64>,
        since: u64,
        now: u64,
    ) -> bool {
        let Some(at) = at.filter(|at| status == Some("idle") && *at > since) else {
            self.near_idle = None;
            return false;
        };
        if at >= since.saturating_add(STATUS_IDLE_MARGIN_MS) {
            self.near_idle = None;
            return true;
        }
        match self.near_idle {
            Some((stamp, first_seen)) if stamp == at => now.saturating_sub(first_seen) >= 2000,
            _ => {
                self.near_idle = Some((at, now));
                false
            }
        }
    }
}

/// Test seam only — e2e cannot spend a real minute per quiet verdict.
fn pane_quiet_ms() -> u64 {
    std::env::var("MESIMON_PANE_QUIET_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(PANE_QUIET_MS)
}

/// How an `Unknown` session's transcript rested → the hint that seeds its
/// recovered state (Low confidence). Quiet gating uses the file mtime: a
/// trailing assistant record on a long-quiet file is a turn that died, not
/// one in flight.
fn resting_hint(path: &std::path::Path, now: u64) -> Option<TailHint> {
    let quiet = std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(mesimon_core::clock::epoch_ms)
        .map(|ms| now.saturating_sub(ms))
        .unwrap_or(u64::MAX);
    match tail::last_event(path)? {
        TailEvent::TurnComplete => Some(TailHint::TurnComplete),
        TailEvent::Aborted => Some(TailHint::AbortedMidStream),
        TailEvent::NeedsHuman { tool: TailTool::AskUserQuestion } => {
            Some(TailHint::AskUserQuestion)
        }
        TailEvent::NeedsHuman { tool: TailTool::ExitPlanMode } => Some(TailHint::ExitPlanMode),
        TailEvent::AssistantText { .. } => {
            if quiet < TAIL_QUIET_MS {
                Some(TailHint::AssistantText)
            } else {
                Some(TailHint::StaleQuiet)
            }
        }
        // A trailing tool call: the tool is running, and the transcript is
        // STILL for as long as it does — the file's quiet says nothing here
        // (T-265: a reload during a 3.5-minute `cargo` call). Seed Running;
        // a turn that really died is `probe_activity`'s to catch, off the
        // pane's own quiet — and since T-439 that probe asks the tail the
        // same question first, because a live pane can go byte-silent for
        // over a minute while a tool runs.
        TailEvent::ToolInFlight => Some(TailHint::ToolInFlight),
        // A trailing user/attachment record: the turn may be in flight — say
        // nothing while the file is fresh, idle once it has clearly died.
        TailEvent::Other => {
            if quiet >= TAIL_QUIET_MS {
                Some(TailHint::StaleQuiet)
            } else {
                None
            }
        }
        TailEvent::Latch => None,
    }
}

#[cfg(test)]
mod status_probe_tests {
    use super::StatusProbe;

    #[test]
    fn permission_needs_observed_waiting_and_a_new_busy_stamp_in_same_spell() {
        let mut p = StatusProbe { path: None, looked_at: 0, near_idle: None, waiting_spell: None };
        assert!(!p.permission_resumed(Some("busy"), Some(1200), 1000));
        assert!(!p.permission_resumed(Some("waiting"), Some(1100), 1000));
        assert!(!p.permission_resumed(Some("busy"), Some(1000), 1000));
        assert!(p.permission_resumed(Some("busy"), Some(1200), 1000));
        assert!(!p.permission_resumed(Some("busy"), Some(1600), 1500));
    }

    #[test]
    fn early_idle_needs_confirmation_and_busy_cancels_it() {
        let mut p = StatusProbe { path: None, looked_at: 0, near_idle: None, waiting_spell: None };
        assert!(!p.confirms_idle(Some("idle"), Some(1120), 1000, 2000));
        assert!(!p.confirms_idle(Some("busy"), Some(1200), 1000, 4000));
        assert!(!p.confirms_idle(Some("idle"), Some(1120), 1000, 5000));
        assert!(!p.confirms_idle(Some("idle"), Some(1120), 1000, 6999));
        assert!(p.confirms_idle(Some("idle"), Some(1120), 1000, 7000));
        assert!(!p.confirms_idle(Some("idle"), Some(1120), 1500, 9000));
        assert!(p.confirms_idle(Some("idle"), Some(1800), 1500, 9000));
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    use mesimon_core::board::{Provenance, SessionKind, UnknownReason};
    use std::io::Write;

    fn record(state: SessionState) -> SessionRecord {
        let mut record = SessionRecord::new(
            uuid::Uuid::new_v4(),
            SessionKind::Claude,
            ulid::Ulid(1),
            vec!["claude".into(), "--resume".into()],
            "/repo".into(),
            state,
        );
        record.state_changed_at = Some(1000);
        record
    }

    struct History(PathBuf);
    impl History {
        fn new() -> Self {
            let path = std::env::temp_dir()
                .join(format!("mesimon-recovery-{}.jsonl", uuid::Uuid::new_v4()));
            std::fs::write(&path, "").unwrap();
            Self(path)
        }
        fn append(&self, value: serde_json::Value) {
            writeln!(std::fs::OpenOptions::new().append(true).open(&self.0).unwrap(), "{value}")
                .unwrap();
        }
    }
    impl Drop for History {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    /// T-724: a park whose hooks went quiet is re-proved off the session
    /// file while Claude Code calls it busy, once a minute, and only a park.
    #[test]
    fn a_busy_session_file_proves_a_park_once_a_minute() {
        let file = History::new();
        let mut parked = record(SessionState::Idle { stop_reason: StopReason::Background });
        let claude_id = parked.id;
        let write = |status: &str| {
            let body = serde_json::json!({
                "pid": 1,
                "sessionId": claude_id,
                "status": status,
                "statusUpdatedAt": 900,
            });
            std::fs::write(&file.0, body.to_string()).unwrap();
        };
        let mut recovery = ClaudeRecovery {
            status: Some(StatusProbe {
                path: Some(file.0.clone()),
                looked_at: 0,
                near_idle: None,
                waiting_spell: None,
            }),
            ..ClaudeRecovery::default()
        };
        let busy = |recovery: &mut ClaudeRecovery, record: &SessionRecord, now| {
            assert!(recovery.needs_poll(record, RecoveryChannel::Status, now));
            recovery
                .poll(record, RecoverySample::Status, now)
                .iter()
                .any(|o| matches!(o.signal, Signal::StatusFileBusy))
        };
        write("busy");
        assert!(busy(&mut recovery, &parked, 10_000), "the first look proves it");
        assert!(!busy(&mut recovery, &parked, 10_000 + PARK_AFFIRM_MS - 1), "a minute apart");
        assert!(busy(&mut recovery, &parked, 10_000 + PARK_AFFIRM_MS));
        write("idle");
        assert!(!busy(&mut recovery, &parked, 10_000 + 3 * PARK_AFFIRM_MS), "idle proves nothing");
        // A finished turn is never re-proved, whatever the file says.
        write("busy");
        parked.state = SessionState::Idle { stop_reason: StopReason::EndTurn };
        assert!(!recovery.needs_poll(
            &parked,
            RecoveryChannel::Status,
            10_000 + 5 * PARK_AFFIRM_MS
        ));
    }

    #[test]
    fn startup_distinguishes_slow_spawn_from_native_modal_and_resume() {
        let record = record(SessionState::Spawning);
        let mut recovery = ClaudeRecovery::default();
        assert!(!recovery.needs_poll(&record, RecoveryChannel::Startup, 10_999));
        assert!(recovery.needs_poll(&record, RecoveryChannel::Startup, 11_000));
        assert!(recovery
            .poll(&record, RecoverySample::Startup { has_output: false, title: None }, 11_000)
            .is_empty());
        assert!(!recovery.needs_poll(&record, RecoveryChannel::Startup, 20_000));
        assert!(recovery.needs_poll(&record, RecoveryChannel::Startup, 31_000));
        let observations = recovery.poll(
            &record,
            RecoverySample::Startup { has_output: false, title: None },
            31_000,
        );
        assert!(matches!(
            observations[0].signal,
            Signal::SpawnProbe { bytes: false, osc0: false, resume: true }
        ));
        assert!(!recovery.needs_poll(&record, RecoveryChannel::Startup, 90_000));
        let mut modal = ClaudeRecovery::default();
        assert!(modal.needs_poll(&record, RecoveryChannel::Startup, 11_000));
        assert!(matches!(
            modal.poll(
                &record,
                RecoverySample::Startup { has_output: true, title: Some("host".into()) },
                11_000
            )[0]
            .signal,
            Signal::SpawnProbe { bytes: true, osc0: false, resume: true }
        ));
        assert!(modal
            .poll(
                &record,
                RecoverySample::Startup { has_output: true, title: Some("✳ Claude".into()) },
                11_000
            )
            .is_empty());
    }

    #[test]
    fn transcript_recovery_keeps_tools_busy_and_owned_turns_abort_only() {
        let history = History::new();
        history.append(serde_json::json!({"uuid":"tool", "type":"assistant", "message":{
            "stop_reason":"tool_use", "content":[{"type":"tool_use","id":"call1","name":"Bash","input":{"command":"sleep 90"}}]}}));
        let mut record = record(SessionState::Unknown { reason: UnknownReason::DaemonRestarted });
        record.transcript_path = Some(history.0.display().to_string());
        let mut recovery = ClaudeRecovery::default();
        assert!(recovery.needs_poll(&record, RecoveryChannel::Transcript, 1000));
        let observations = recovery.poll(&record, RecoverySample::Transcript, 1000);
        assert!(matches!(
            observations[0].signal,
            Signal::TranscriptHint { kind: TailHint::ToolInFlight }
        ));
        record.state = SessionState::Running;
        record.provenance = Provenance::Adopted;
        record.argv.clear();
        assert!(recovery.needs_poll(&record, RecoveryChannel::Transcript, 100_000));
        assert!(
            recovery.poll(&record, RecoverySample::Transcript, 100_000).is_empty(),
            "quiet tool history cannot prove idle"
        );
        assert!(!recovery.needs_poll(&record, RecoveryChannel::Activity, 100_000));
        assert!(!recovery.needs_poll(&record, RecoveryChannel::Status, 100_000));
        record.argv.push("claude".into());
        history.append(serde_json::json!({"uuid":"reply", "type":"assistant", "message":{"stop_reason":"end_turn","content":[{"type":"text","text":"not an abort"}]}}));
        assert!(recovery.needs_poll(&record, RecoveryChannel::Transcript, 100_001));
        assert!(
            recovery.poll(&record, RecoverySample::Transcript, 100_001).is_empty(),
            "owned state stays hook-owned"
        );
        history.append(serde_json::json!({"uuid":"abort", "type":"user", "message":{"content":"[Request interrupted by user]"}}));
        let observations = recovery.poll(&record, RecoverySample::Transcript, 100_002);
        assert!(matches!(
            observations[0].signal,
            Signal::TranscriptHint { kind: TailHint::AbortedMidStream }
        ));
        assert!(observations[0].preview.is_none());
        record.state = SessionState::Sleeping;
        assert!(!recovery.needs_poll(&record, RecoveryChannel::Transcript, 100_003));
        assert!(recovery.cursor.is_none());
    }

    /// T-363: a plan card went `?` at fifteen minutes while the dialog was
    /// still open. While the record holds a plan or question and the tail's
    /// last event is that tool's pending call, the adapter restates the
    /// wait once a minute; once the transcript moves on it says nothing.
    #[test]
    fn a_held_dialog_is_restated_off_the_tail_once_a_minute() {
        let history = History::new();
        history.append(serde_json::json!({"uuid":"ask", "type":"assistant", "message":{
            "stop_reason":"tool_use", "content":[{"type":"tool_use","id":"call1","name":"ExitPlanMode","input":{}}]}}));
        // The uuid-less latch records Claude Code writes after the call.
        history.append(serde_json::json!({"type":"last-prompt", "lastPrompt":"plan it"}));
        let mut record = record(SessionState::RequiresAction { reason: Reason::Plan });
        record.transcript_path = Some(history.0.display().to_string());
        let mut recovery = ClaudeRecovery::default();
        assert!(recovery.needs_poll(&record, RecoveryChannel::Transcript, 1000));
        let first = recovery.poll(&record, RecoverySample::Transcript, 1000);
        assert!(matches!(
            first[..],
            [RecoveryObservation {
                signal: Signal::TranscriptHint { kind: TailHint::ExitPlanMode },
                ..
            }]
        ));
        assert!(
            recovery.poll(&record, RecoverySample::Transcript, 30_000).is_empty(),
            "rate-limited"
        );
        let again = recovery.poll(&record, RecoverySample::Transcript, 61_000);
        assert!(matches!(
            again[..],
            [RecoveryObservation {
                signal: Signal::TranscriptHint { kind: TailHint::ExitPlanMode },
                ..
            }]
        ));
        // A question's record does not affirm a plan.
        record.state = SessionState::RequiresAction { reason: Reason::Question };
        assert!(recovery.poll(&record, RecoverySample::Transcript, 130_000).is_empty());
        // Answered: the tool result is the last word, and the wait is over:
        // nothing restates it, and the call's own result is its departure
        // (T-957).
        record.state = SessionState::RequiresAction { reason: Reason::Plan };
        history.append(serde_json::json!({"uuid":"answer", "type":"user", "message":{
            "content":[{"type":"tool_result","tool_use_id":"call1","content":"User has approved your plan."}]}}));
        assert!(matches!(
            &recovery.poll(&record, RecoverySample::Transcript, 200_000)[..],
            [RecoveryObservation {
                signal: Signal::DialogEnded { interrupted: false },
                source: "dialog_result",
                dialog: Some(call),
                ..
            }] if call == "call1"
        ));
        // Nothing for a permission: the transcript cannot tell that dialog
        // from a running tool.
        record.state = SessionState::RequiresAction { reason: Reason::Permission };
        assert!(recovery.poll(&record, RecoverySample::Transcript, 300_000).is_empty());
    }

    /// T-957: a person's refusal of a plan or a question in the pane fires
    /// no hook, and neither the hook set nor the classic mod sends the next
    /// ordinary tool's `PreToolUse`. What Claude Code writes is the call's
    /// own result at the keypress (2.1.310, measured on the board's own
    /// transcripts): `is_error` with the person's words for a plan sent
    /// back, `is_error` and then an interrupt record for an Esc. That result
    /// is the dialog's departure; with the interrupt right after it, in the
    /// same read or the next, the Esc that stopped the turn. A sibling's
    /// result is not it (T-595), a newer call after it stands, and a cursor
    /// that came after the call learns it off the tail, open or ended.
    #[test]
    fn a_held_dialog_leaves_on_its_own_call_s_result() {
        let call = |id: &str, name: &str| {
            serde_json::json!({"uuid": format!("call-{id}"), "type":"assistant", "message":{
                "content":[{"type":"tool_use","id":id,"name":name,"input":{}}]}})
        };
        let at = |second: u32| format!("2026-10-24T16:10:{second:02}.000Z");
        let ms = |second: u32| mesimon_core::adopt::iso_ms(&at(second)).unwrap();
        let result = |id: &str, error: bool, text: &str, second: u32| {
            serde_json::json!({"uuid": format!("result-{id}"), "type":"user",
                "timestamp": at(second), "message":{"content":[{"type":"tool_result",
                    "tool_use_id":id,"is_error":error,"content":text}]}})
        };
        let abort = || {
            serde_json::json!({"uuid":"abort", "type":"user", "message":{
                "content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]}})
        };
        let refused = "The user doesn't want to proceed with this tool use. The tool use was \
                       rejected (eg. if it was a file edit, the new_string was NOT written to the \
                       file). To tell you how to proceed, the user said: Use echo instead.";
        let stop = "The user doesn't want to proceed with this tool use. The tool use was \
                    rejected (eg. if it was a file edit, the new_string was NOT written to the \
                    file). STOP what you are doing and wait for the user to tell you how to \
                    proceed.";
        let ended = |seen: &[RecoveryObservation], id: &str, esc: bool| {
            matches!(seen.first(), Some(RecoveryObservation {
                signal: Signal::DialogEnded { interrupted },
                source: "dialog_result",
                dialog: Some(call),
                ..
            }) if call == id && *interrupted == esc)
        };
        let left = |seen: &[RecoveryObservation], id: &str| ended(seen, id, false);
        let stays = |seen: &[RecoveryObservation]| seen.iter().all(|o| o.dialog.is_none());
        let plan = SessionState::RequiresAction { reason: Reason::Plan };
        let question = SessionState::RequiresAction { reason: Reason::Question };
        let history = History::new();
        let mut record = record(SessionState::Running);
        record.transcript_path = Some(history.0.display().to_string());
        let mut recovery = ClaudeRecovery::default();
        assert!(recovery.needs_poll(&record, RecoveryChannel::Transcript, 1000));
        assert!(stays(&recovery.poll(&record, RecoverySample::Transcript, 1000)));

        // A plan beside a sibling, sent back with words.
        history.append(call("s1", "Write"));
        history.append(call("plan1", "ExitPlanMode"));
        record.state = plan.clone();
        assert!(stays(&recovery.poll(&record, RecoverySample::Transcript, 2000)));
        history.append(result("s1", false, "File created successfully", 1));
        assert!(
            stays(&recovery.poll(&record, RecoverySample::Transcript, 3000)),
            "a sibling's result is not the plan's"
        );
        history.append(result("plan1", true, refused, 2));
        let seen = recovery.poll(&record, RecoverySample::Transcript, 4000);
        assert!(left(&seen, "plan1") && seen.len() == 1, "{seen:?}");
        assert!(stays(&recovery.poll(&record, RecoverySample::Transcript, 5000)), "once");

        // Replanned in the same batch: the new plan stands, then leaves.
        history.append(call("plan2", "ExitPlanMode"));
        assert!(stays(&recovery.poll(&record, RecoverySample::Transcript, 6000)));
        history.append(result("plan2", true, refused, 3));
        history.append(call("plan3", "ExitPlanMode"));
        assert!(stays(&recovery.poll(&record, RecoverySample::Transcript, 7000)));
        history.append(result("plan3", true, refused, 4));
        assert!(left(&recovery.poll(&record, RecoverySample::Transcript, 8000), "plan3"));

        // A question's Esc: one end, interrupted, then the Low interrupt hint.
        history.append(call("q1", "AskUserQuestion"));
        record.state = question.clone();
        assert!(stays(&recovery.poll(&record, RecoverySample::Transcript, 9000)));
        history.append(result("q1", true, stop, 5));
        history.append(abort());
        let seen = recovery.poll(&record, RecoverySample::Transcript, 10_000);
        assert!(ended(&seen, "q1", true), "{seen:?}");
        assert!(matches!(
            seen[1..],
            [RecoveryObservation {
                signal: Signal::TranscriptHint { kind: TailHint::AbortedMidStream },
                ..
            }]
        ));

        // A plan's Esc split across two reads: the departure, then the
        // interrupt on the `Running` it left for.
        history.append(call("plan5", "ExitPlanMode"));
        record.state = plan.clone();
        assert!(stays(&recovery.poll(&record, RecoverySample::Transcript, 10_500)));
        history.append(result("plan5", true, stop, 6));
        assert!(left(&recovery.poll(&record, RecoverySample::Transcript, 11_000), "plan5"));
        record.state = SessionState::Running;
        history.append(abort());
        assert!(ended(&recovery.poll(&record, RecoverySample::Transcript, 11_500), "plan5", true));
        // An interrupt after anything else is not the dialog's.
        history.append(call("plan6", "ExitPlanMode"));
        record.state = plan.clone();
        assert!(stays(&recovery.poll(&record, RecoverySample::Transcript, 11_600)));
        history.append(result("plan6", true, refused, 7));
        assert!(left(&recovery.poll(&record, RecoverySample::Transcript, 11_700), "plan6"));
        record.state = SessionState::Running;
        history.append(serde_json::json!({"uuid":"t", "type":"assistant", "message":{
            "content":[{"type":"text","text":"Replanning."}]}}));
        history.append(abort());
        assert!(stays(&recovery.poll(&record, RecoverySample::Transcript, 11_800)));

        // Round 2: an Esc and, at once, the person's next prompt in the same
        // read. The interrupt ended the turn before the prompt: no end at
        // High, no Low hint, whatever the record reads (here the prompt's
        // hook already put it back to running).
        history.append(call("plan11", "ExitPlanMode"));
        record.state = plan.clone();
        assert!(stays(&recovery.poll(&record, RecoverySample::Transcript, 11_850)));
        history.append(result("plan11", true, stop, 9));
        let mut stamped = abort();
        stamped["timestamp"] = serde_json::json!(at(9));
        history.append(stamped.clone());
        history.append(serde_json::json!({"uuid":"close", "type":"system",
            "subtype":"turn_duration", "durationMs": 1, "timestamp": at(9)}));
        history.append(serde_json::json!({"uuid":"next", "type":"user", "timestamp": at(11),
            "message":{"content":"Do it another way."}}));
        record.state = SessionState::Running;
        let seen = recovery.poll(&record, RecoverySample::Transcript, 11_900);
        assert!(stays(&seen), "{seen:?}");
        assert!(
            !seen.iter().any(|o| matches!(
                o.signal,
                Signal::TranscriptHint { kind: TailHint::AbortedMidStream }
            )),
            "{seen:?}"
        );
        // Without a prompt after it, the Esc's end and its hint carry the
        // interrupt's stamp, for the daemon to hold against a later prompt.
        history.append(call("plan12", "ExitPlanMode"));
        record.state = plan.clone();
        assert!(stays(&recovery.poll(&record, RecoverySample::Transcript, 11_950)));
        history.append(result("plan12", true, stop, 12));
        stamped["timestamp"] = serde_json::json!(at(13));
        history.append(stamped);
        let seen = recovery.poll(&record, RecoverySample::Transcript, 11_960);
        assert!(ended(&seen, "plan12", true), "{seen:?}");
        assert!(seen.iter().all(|o| o.at == Some(ms(13))), "{seen:?}");

        // Another dialog's result says nothing about the one held.
        history.append(call("q2", "AskUserQuestion"));
        record.state = plan.clone();
        history.append(result("q2", true, stop, 14));
        assert!(stays(&recovery.poll(&record, RecoverySample::Transcript, 12_000)));

        // A cursor minted after the call (a restart): learnt off the tail.
        let fresh = |record: &mut SessionRecord, history: &History, spell: u32| {
            record.transcript_path = Some(history.0.display().to_string());
            record.state_changed_at = Some(ms(spell));
            let mut recovery = ClaudeRecovery::default();
            assert!(recovery.needs_poll(record, RecoveryChannel::Transcript, 20_000));
            recovery
        };
        let history = History::new();
        history.append(call("plan4", "ExitPlanMode"));
        let mut recovery = fresh(&mut record, &history, 10);
        assert!(stays(&recovery.poll(&record, RecoverySample::Transcript, 20_000)));
        history.append(result("plan4", true, refused, 12));
        assert!(left(&recovery.poll(&record, RecoverySample::Transcript, 21_000), "plan4"));

        // Round 1: ended before the cursor came, in the spell: the refusal
        // with words, and an Esc, read on the first look, once.
        for (text, esc) in [(refused, false), (stop, true)] {
            let history = History::new();
            history.append(call("old", "ExitPlanMode"));
            history.append(result("old", true, refused, 5));
            history.append(call("plan7", "ExitPlanMode"));
            history.append(result("plan7", true, text, 12));
            if esc {
                history.append(abort());
            }
            let mut recovery = fresh(&mut record, &history, 10);
            let seen = recovery.poll(&record, RecoverySample::Transcript, 20_000);
            assert!(ended(&seen, "plan7", esc), "{seen:?}");
            assert!(stays(&recovery.poll(&record, RecoverySample::Transcript, 22_000)), "once");
            if !esc {
                // The interrupt after it, a read later, is still its Esc.
                record.state = SessionState::Running;
                history.append(abort());
                let seen = recovery.poll(&record, RecoverySample::Transcript, 23_000);
                assert!(ended(&seen, "plan7", true), "{seen:?}");
                record.state = plan.clone();
            }
        }
        // History never clears a newer wait: an earlier dialog's result,
        // stamped before this spell; the other tool's call last; an undated
        // result.
        let history = History::new();
        history.append(call("plan8", "ExitPlanMode"));
        history.append(result("plan8", true, refused, 5));
        let mut recovery = fresh(&mut record, &history, 10);
        assert!(stays(&recovery.poll(&record, RecoverySample::Transcript, 20_000)), "older");
        let history = History::new();
        history.append(call("plan9", "ExitPlanMode"));
        history.append(result("plan9", true, refused, 12));
        history.append(call("q3", "AskUserQuestion"));
        history.append(result("q3", true, stop, 13));
        let mut recovery = fresh(&mut record, &history, 10);
        assert!(stays(&recovery.poll(&record, RecoverySample::Transcript, 20_000)), "other tool");
        let history = History::new();
        history.append(call("plan10", "ExitPlanMode"));
        history.append(serde_json::json!({"uuid":"r", "type":"user", "message":{"content":[
            {"type":"tool_result","tool_use_id":"plan10","is_error":true,"content":refused}]}}));
        let mut recovery = fresh(&mut record, &history, 10);
        assert!(stays(&recovery.poll(&record, RecoverySample::Transcript, 20_000)), "undated");
    }

    /// T-802: a usage limit is restated off the tail once a minute while
    /// the limit's error row is the last word, so the stale clock never
    /// takes QUOTA off a card whose agent is still stopped on it; a prompt
    /// past the row, or another class of error, restates nothing.
    #[test]
    fn a_usage_limit_is_restated_off_the_tail_while_it_is_the_last_word() {
        let history = History::new();
        history.append(serde_json::json!({"uuid":"p", "type":"user", "message":{"content":"go"}}));
        history.append(super::tail::error_row("rate_limit", "You've reached your Fable limit."));
        let mut record = record(SessionState::RequiresAction { reason: Reason::QuotaResume });
        record.transcript_path = Some(history.0.display().to_string());
        let mut recovery = ClaudeRecovery::default();
        let limit = |read: &[RecoveryObservation]| {
            matches!(
                read,
                [RecoveryObservation {
                    signal: Signal::StopFailure { class: attention::StopFailureClass::RateLimit },
                    ..
                }]
            )
        };
        assert!(recovery.needs_poll(&record, RecoveryChannel::Transcript, 1000));
        assert!(limit(&recovery.poll(&record, RecoverySample::Transcript, 1000)));
        assert!(
            recovery.poll(&record, RecoverySample::Transcript, 30_000).is_empty(),
            "once a minute"
        );
        assert!(limit(&recovery.poll(&record, RecoverySample::Transcript, 61_000)));
        // Another wait's record is not restated as a limit.
        record.state = SessionState::RequiresAction { reason: Reason::Auth };
        assert!(recovery.poll(&record, RecoverySample::Transcript, 130_000).is_empty());
        // A prompt got through: the row is no longer the last word.
        record.state = SessionState::RequiresAction { reason: Reason::QuotaResume };
        history.append(serde_json::json!({"uuid":"q", "type":"user", "message":{"content":"on"}}));
        assert!(recovery.poll(&record, RecoverySample::Transcript, 200_000).is_empty());
    }

    /// T-649: a question frame came 2 s after the turn's `Stop`, from no
    /// call the lead's transcript holds, and the card held QUESTION for
    /// fifteen minutes and then `Unknown`'s turning glyph for good. A held
    /// dialog whose tail still ends on the turn's close a minute in reads
    /// as that close; an `Unknown` the stale clock reached on a living
    /// cursor is seeded from the resting tail, once.
    #[test]
    fn a_dialog_the_tail_never_shows_falls_back_to_the_turns_close() {
        let history = History::new();
        history.append(serde_json::json!({"uuid":"reply", "type":"assistant", "message":{
            "stop_reason":"end_turn", "content":[{"type":"text","text":"Committed."}]}}));
        history.append(
            serde_json::json!({"uuid":"hooks", "type":"system", "subtype":"stop_hook_summary"}),
        );
        history
            .append(serde_json::json!({"uuid":"end", "type":"system", "subtype":"turn_duration"}));
        let mut record = record(SessionState::RequiresAction { reason: Reason::Question });
        record.transcript_path = Some(history.0.display().to_string());
        let mut recovery = ClaudeRecovery::default();
        assert!(recovery.needs_poll(&record, RecoveryChannel::Transcript, 1000));
        assert!(
            recovery.poll(&record, RecoverySample::Transcript, 1000).is_empty(),
            "the call may not be written yet"
        );
        let closed = recovery.poll(&record, RecoverySample::Transcript, 61_000);
        assert!(matches!(
            closed[..],
            [RecoveryObservation {
                signal: Signal::TranscriptHint { kind: TailHint::TurnComplete },
                ..
            }]
        ));
        // The stale clock's demotion, on the cursor already minted.
        record.state = SessionState::Unknown { reason: UnknownReason::NoSignal };
        record.state_changed_at = Some(900_000);
        assert!(recovery.needs_poll(&record, RecoveryChannel::Transcript, 900_000));
        let seeded = recovery.poll(&record, RecoverySample::Transcript, 900_000);
        assert!(matches!(
            seeded[..],
            [RecoveryObservation {
                signal: Signal::TranscriptHint { kind: TailHint::TurnComplete },
                ..
            }]
        ));
        assert!(
            recovery.poll(&record, RecoverySample::Transcript, 901_000).is_empty(),
            "once per spell"
        );
        // A real question's call is on the tail and is restated, not closed.
        history.append(serde_json::json!({"uuid":"ask", "type":"assistant", "message":{
            "stop_reason":"tool_use", "content":[{"type":"tool_use","id":"q1","name":"AskUserQuestion","input":{}}]}}));
        record.state = SessionState::RequiresAction { reason: Reason::Question };
        record.state_changed_at = Some(1000);
        let held = recovery.poll(&record, RecoverySample::Transcript, 990_000);
        assert!(held
            .iter()
            .all(|o| !matches!(o.signal, Signal::TranscriptHint { kind: TailHint::TurnComplete })));
        assert!(held.iter().any(|o| matches!(
            o.signal,
            Signal::TranscriptHint { kind: TailHint::AskUserQuestion }
        )));
    }

    /// T-663: a turn that ended while no daemon was up. The record comes
    /// back `Unknown{DaemonRestarted}` over the spell the last daemon left
    /// it in, and the tail dates the turn's close after that spell began:
    /// the restart's seed is the turn's end, once per spell, never the Low
    /// hint. A spell begun after the close (the next turn's recordless Esc)
    /// and the stale clock's `Unknown` over the same tail get the hint as
    /// before; an undated close too; and a prompt after the close is an
    /// open turn, seeded from how it rests.
    #[test]
    fn a_turn_closed_after_the_spell_began_is_the_restarts_turn_end() {
        let at = |s: &str| mesimon_core::adopt::iso_ms(s).unwrap();
        let prompt = "2026-10-05T12:05:26.000Z"; // the train's rebase ask
        let close = "2026-10-05T12:06:10.000Z"; // inside the restart's window
        let history = History::new();
        history.append(serde_json::json!({"uuid":"p", "type":"user", "timestamp":prompt,
            "message":{"content":"Rebase your current branch"}}));
        history.append(serde_json::json!({"uuid":"r", "type":"assistant", "timestamp":close,
            "message":{"stop_reason":"end_turn", "content":[{"type":"text","text":"Rebased."}]}}));
        history.append(serde_json::json!({"uuid":"h", "type":"system",
            "subtype":"stop_hook_summary", "timestamp":"2026-10-05T12:06:10.400Z"}));
        history.append(serde_json::json!({"type":"last-prompt", "lastPrompt":"Rebase"}));
        let restarted = SessionState::Unknown { reason: UnknownReason::DaemonRestarted };
        let mut record = record(restarted.clone());
        record.transcript_path = Some(history.0.display().to_string());
        record.state_changed_at = Some(at(prompt) + 100); // Running since the prompt's ack
        let now = at(close) + 60_000;
        let mut recovery = ClaudeRecovery::default();
        assert!(recovery.needs_poll(&record, RecoveryChannel::Transcript, now));
        let seeded = recovery.poll(&record, RecoverySample::Transcript, now);
        assert!(
            matches!(
                seeded[..],
                [RecoveryObservation {
                    signal: Signal::TranscriptTurnEnded,
                    source: "tail",
                    preview: None,
                    dialog: None,
                    at: None
                }]
            ),
            "{seeded:?}"
        );
        assert!(
            recovery.poll(&record, RecoverySample::Transcript, now + 2_000).is_empty(),
            "once per spell"
        );
        let hint = |observations: &[RecoveryObservation]| {
            matches!(
                observations[..],
                [RecoveryObservation {
                    signal: Signal::TranscriptHint { kind: TailHint::TurnComplete },
                    ..
                }]
            )
        };
        // Running since after the close: that turn wrote nothing yet.
        let mut later = ClaudeRecovery::default();
        record.state_changed_at = Some(at(close) + 5_000);
        assert!(later.needs_poll(&record, RecoveryChannel::Transcript, now));
        assert!(hint(&later.poll(&record, RecoverySample::Transcript, now)));
        // The stale clock's Unknown over the same tail: a daemon was listening.
        let mut stale = ClaudeRecovery::default();
        record.state = SessionState::unknown();
        record.state_changed_at = Some(at(prompt) + 100);
        assert!(stale.needs_poll(&record, RecoveryChannel::Transcript, now));
        assert!(hint(&stale.poll(&record, RecoverySample::Transcript, now)));
        // A prompt typed after the close: the turn is open, nothing is proven,
        // and a fresh file says nothing of how it rests.
        history.append(serde_json::json!({"uuid":"p2", "type":"user",
            "timestamp":"2026-10-05T12:06:40.000Z", "message":{"content":"and push"}}));
        let mut open = ClaudeRecovery::default();
        record.state = restarted;
        assert!(open.needs_poll(&record, RecoveryChannel::Transcript, now));
        assert!(open.poll(&record, RecoverySample::Transcript, now).is_empty());
        // An undated close, the shape an older transcript rests in: the hint.
        let undated = History::new();
        undated.append(serde_json::json!({"uuid":"u1", "type":"assistant",
            "message":{"stop_reason":"end_turn", "content":[{"type":"text","text":"done"}]}}));
        undated
            .append(serde_json::json!({"uuid":"u2", "type":"system", "subtype":"turn_duration"}));
        record.transcript_path = Some(undated.0.display().to_string());
        let mut plain = ClaudeRecovery::default();
        assert!(plain.needs_poll(&record, RecoveryChannel::Transcript, now));
        assert!(hint(&plain.poll(&record, RecoverySample::Transcript, now)));
    }

    #[test]
    fn observed_reply_is_normalized_without_mutating_the_record() {
        let history = History::new();
        let mut record = record(SessionState::Running);
        record.provenance = Provenance::Adopted;
        record.argv.clear();
        record.transcript_path = Some(history.0.display().to_string());
        let mut recovery = ClaudeRecovery::default();
        assert!(recovery.needs_poll(&record, RecoveryChannel::Transcript, 1000));
        assert!(recovery.poll(&record, RecoverySample::Transcript, 1000).is_empty());
        history.append(serde_json::json!({"uuid":"reply", "type":"assistant", "message":{"stop_reason":"end_turn","content":[{"type":"text","text":"Read [result](https://example.test/result)"}]}}));
        let observations = recovery.poll(&record, RecoverySample::Transcript, 2000);
        assert_eq!(observations.len(), 1);
        assert!(matches!(
            observations[0].signal,
            Signal::TranscriptHint { kind: TailHint::TurnComplete }
        ));
        assert_eq!(
            observations[0].preview.as_deref(),
            Some("Read [result](https://example.test/result)")
        );
        assert_eq!(observations[0].source, "tail");
        assert!(record.detail.is_none());
        assert_eq!(record.state, SessionState::Running);
    }

    /// T-439: a pane silent past the threshold is still not quiet while the
    /// transcript shows a tool in flight; once the turn has closed the probe
    /// speaks again, and a record with no transcript (the stub e2es) is
    /// judged on silence alone as before.
    #[test]
    fn a_silent_pane_with_a_tool_in_flight_is_not_quiet() {
        let history = History::new();
        history.append(serde_json::json!({"uuid":"call", "type":"assistant",
            "timestamp":"2026-09-23T08:33:11.000Z", "message":{"stop_reason":"tool_use",
            "content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo nextest run"}}]}}));
        let mut record = record(SessionState::Running);
        record.transcript_path = Some(history.0.display().to_string());
        let mut recovery = ClaudeRecovery::default();
        let now = 1_790_152_451_076; // 08:34:11, sixty seconds after the pane's last byte
        let silent = || RecoverySample::Activity { last_output_ms: 1_790_152_391_000 };
        assert!(recovery.needs_poll(&record, RecoveryChannel::Activity, now));
        assert!(recovery.poll(&record, silent(), now).is_empty(), "tool in flight");
        // The turn closes, the pane stays silent: the fallback is back.
        history.append(serde_json::json!({"uuid":"res", "type":"user",
            "timestamp":"2026-09-23T08:34:59.000Z",
            "message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}}));
        history
            .append(serde_json::json!({"uuid":"end", "type":"system", "subtype":"turn_duration",
            "timestamp":"2026-09-23T08:35:00.000Z", "durationMs": 1}));
        let observations = recovery.poll(&record, silent(), now + 120_000);
        assert!(matches!(observations[0].signal, Signal::PaneQuiet));
        assert_eq!(observations[0].source, "activity");
        // No transcript to ask: silence decides, as it always did.
        record.transcript_path = None;
        let observations = recovery.poll(&record, silent(), now);
        assert!(matches!(observations[0].signal, Signal::PaneQuiet));
    }

    /// T-729: a model thinking past the threshold after a tool result leaves
    /// pane and transcript both still; the session file's `busy` holds the
    /// probe, and its `idle` lets it speak.
    #[test]
    fn a_silent_pane_claude_code_calls_busy_is_not_quiet() {
        let history = History::new();
        history.append(serde_json::json!({"uuid":"res", "type":"user",
            "timestamp":"2026-09-23T08:33:11.000Z",
            "message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}}));
        let file = History::new();
        let mut record = record(SessionState::Running);
        record.transcript_path = Some(history.0.display().to_string());
        let claude_id = record.id;
        let write = |status: &str| {
            let body = serde_json::json!({
                "pid": 1, "sessionId": claude_id, "status": status, "statusUpdatedAt": 900,
            });
            std::fs::write(&file.0, body.to_string()).unwrap();
        };
        let mut recovery = ClaudeRecovery {
            status: Some(StatusProbe {
                path: Some(file.0.clone()),
                looked_at: 0,
                near_idle: None,
                waiting_spell: None,
            }),
            ..ClaudeRecovery::default()
        };
        let now = 1_790_152_511_076; // two minutes past the tool result
        let silent = || RecoverySample::Activity { last_output_ms: 1_790_152_391_000 };
        write("busy");
        assert!(recovery.poll(&record, silent(), now).is_empty(), "Claude Code calls it busy");
        write("idle");
        let observations = recovery.poll(&record, silent(), now);
        assert!(matches!(observations[0].signal, Signal::PaneQuiet));
    }

    /// T-731: the same thinking across a restart. The restart's seed read a
    /// tool result quiet past the threshold as a turn gone idle, and the
    /// crown was told the turn had finished; while the session file reads
    /// `busy` the record stays `Unknown`, and an `idle` file seeds as before.
    #[test]
    fn a_quiet_tail_claude_code_calls_busy_seeds_no_idle_after_a_restart() {
        let history = History::new();
        history.append(serde_json::json!({"uuid":"res", "type":"user",
            "message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}}));
        let now = mesimon_core::clock::now_ms();
        let quiet = std::time::SystemTime::now() - std::time::Duration::from_secs(120);
        std::fs::File::options().write(true).open(&history.0).unwrap().set_modified(quiet).unwrap();
        let file = History::new();
        let mut record = record(SessionState::Unknown { reason: UnknownReason::DaemonRestarted });
        record.transcript_path = Some(history.0.display().to_string());
        record.state_changed_at = Some(now - 300_000);
        let claude_id = record.id;
        let write = |status: &str| {
            let body = serde_json::json!({
                "pid": 1, "sessionId": claude_id, "status": status, "statusUpdatedAt": 900,
            });
            std::fs::write(&file.0, body.to_string()).unwrap();
        };
        let fresh = || ClaudeRecovery {
            status: Some(StatusProbe {
                path: Some(file.0.clone()),
                looked_at: 0,
                near_idle: None,
                waiting_spell: None,
            }),
            ..ClaudeRecovery::default()
        };
        write("busy");
        let mut thinking = fresh();
        assert!(thinking.needs_poll(&record, RecoveryChannel::Transcript, now));
        let seeded = thinking.poll(&record, RecoverySample::Transcript, now);
        assert!(seeded.is_empty(), "Claude Code calls it busy: {seeded:?}");
        write("idle");
        let mut over = fresh();
        let seeded = over.poll(&record, RecoverySample::Transcript, now);
        assert!(
            matches!(
                seeded[..],
                [RecoveryObservation {
                    signal: Signal::TranscriptHint { kind: TailHint::StaleQuiet },
                    ..
                }]
            ),
            "{seeded:?}"
        );
    }

    /// T-659: a native failed turn whose frame beat the transcript's write
    /// stays `unknown` only until the error row lands; the re-read runs every
    /// two seconds for a minute, on a native record only.
    #[test]
    fn a_native_failure_takes_its_class_off_the_tail_once_written() {
        let history = History::new();
        history.append(serde_json::json!({"uuid":"p", "type":"user", "message":{"content":"go"}}));
        let mut record = record(SessionState::Failed { reason: FailReason::Unknown });
        record.transcript_path = Some(history.0.display().to_string());
        let mut recovery = ClaudeRecovery::default();
        assert!(
            !recovery.needs_poll(&record, RecoveryChannel::Transcript, 1000),
            "the hook set's road: its frame named the class"
        );
        record.native = true;
        assert!(recovery.needs_poll(&record, RecoveryChannel::Transcript, 1000));
        assert!(recovery.poll(&record, RecoverySample::Transcript, 1000).is_empty());
        assert!(!recovery.needs_poll(&record, RecoveryChannel::Transcript, 2000), "rate-limited");
        history.append(super::tail::error_row("rate_limit", "API Error: Rate limit reached"));
        assert!(recovery.needs_poll(&record, RecoveryChannel::Transcript, 3000));
        let read = recovery.poll(&record, RecoverySample::Transcript, 3000);
        assert!(matches!(
            read[..],
            [RecoveryObservation {
                signal: Signal::StopFailure { class: attention::StopFailureClass::RateLimit },
                ..
            }]
        ));
        assert_eq!(read[0].preview.as_deref(), Some("API Error: Rate limit reached"));
        // A minute on, `unknown` is the answer.
        assert!(!recovery.needs_poll(&record, RecoveryChannel::Transcript, 61_000));
        // A class the frame did say is never re-read.
        record.state = SessionState::Failed { reason: FailReason::Server };
        assert!(!recovery.needs_poll(&record, RecoveryChannel::Transcript, 4000));
    }

    /// T-899 review: a native `Failed{Unknown}` takes no words while the
    /// re-read may still class it a limit, and takes them once the spell
    /// ends or a class is read. The hook set's road named its class.
    #[test]
    fn a_native_failure_takes_words_only_once_its_class_is_read_or_given_up() {
        let mut record = record(SessionState::Failed { reason: FailReason::Unknown });
        record.transcript_path = Some("/tmp/t.jsonl".into());
        record.state_changed_at = Some(1_000);
        assert!(!failure_being_read(&record, 2_000), "the hook set's road");
        record.native = true;
        assert!(failure_being_read(&record, 1_000));
        assert!(failure_being_read(&record, 1_000 + FAILURE_REREAD_SPELL_MS - 1));
        assert!(!failure_being_read(&record, 1_000 + FAILURE_REREAD_SPELL_MS), "given up");
        record.state = SessionState::Failed { reason: FailReason::Server };
        assert!(!failure_being_read(&record, 2_000), "a class read");
    }
}
