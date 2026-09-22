//! Claude Code passive recovery: startup modals, status-file edges and bounded
//! transcript evidence. Only normalized observations leave this module.

use super::tail::{self, TailCursor};
use crate::agents::{AgentRecovery, RecoveryChannel, RecoveryObservation, RecoverySample};
use mesimon_core::adopt::{classify_tail_record, SessionsPidFile, TailEvent, TailTool};
use mesimon_core::attention::{self, Signal, TailHint};
use mesimon_core::board::{Provenance, Reason, SessionRecord, SessionState};
use std::path::PathBuf;

const TAIL_QUIET_MS: u64 = 45_000;
/// How often a held plan or question is re-read off the transcript tail
/// while its dialog is open. The machine's stale clock is fifteen minutes;
/// one 64 KiB read a minute per waiting session keeps it re-armed with
/// margin to spare (T-363).
const WAIT_AFFIRM_MS: u64 = 60_000;

#[derive(Default)]
pub(super) struct ClaudeRecovery {
    startup_stage: u8,
    cursor: Option<TailCursor>,
    status: Option<StatusProbe>,
    /// Last time a held plan/question was looked for on the tail; `None`
    /// since the cursor was minted, so the first poll looks at once.
    affirmed_at: Option<u64>,
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

fn observe_only(record: &SessionRecord) -> bool {
    record.provenance == Provenance::Adopted && record.argv.is_empty()
}

fn abort_only(record: &SessionRecord) -> bool {
    !observe_only(record)
        && (record.state == SessionState::Running || attention::is_attention(&record.state))
}

fn observation(signal: Signal, source: &'static str) -> RecoveryObservation {
    RecoveryObservation { signal, source, preview: None }
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
                !observe_only(record) && record.state == SessionState::Running
            }
            RecoveryChannel::Status => {
                // A claude in a ticket's shell (T-369) keeps a pid file like
                // any other; its idle/busy is worth Medium the same way.
                let eligible = (!observe_only(record) || record.host.is_some())
                    && (record.state == SessionState::Running
                        || record.state
                            == SessionState::RequiresAction { reason: Reason::Permission });
                if !eligible {
                    self.status = None;
                }
                eligible
            }
            RecoveryChannel::Transcript => {
                let eligible = record.transcript_path.is_some()
                    && ((observe_only(record) && record.state.is_live())
                        || (!observe_only(record)
                            && matches!(record.state, SessionState::Unknown { .. }))
                        || abort_only(record));
                if !eligible {
                    self.cursor = None;
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
                if now.saturating_sub(last_output_ms) < pane_quiet_ms() {
                    Vec::new()
                } else {
                    vec![observation(Signal::PaneQuiet, "activity")]
                }
            }
            RecoverySample::Status => self.status_observation(record, now).into_iter().collect(),
            RecoverySample::Transcript => self.transcript_observations(record, now),
        }
    }
}

impl ClaudeRecovery {
    fn status_observation(
        &mut self,
        record: &SessionRecord,
        now: u64,
    ) -> Option<RecoveryObservation> {
        let claude_id = record.claude_session_id.unwrap_or(record.id);
        let since = record.state_changed_at.unwrap_or(now);
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
        let abort_only = abort_only(record);
        let fresh = self.cursor.as_ref().is_none_or(|cursor| cursor.path != path);
        // Only an Unknown record may seed low-confidence state from resting
        // history. Owned live sessions recover explicit current-spell aborts.
        let backfill = if fresh && matches!(record.state, SessionState::Unknown { .. }) {
            resting_hint(&path, now)
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
        }
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
        if let SessionState::RequiresAction { reason } = &record.state {
            if let Some((tool, hint)) = pending_dialog(*reason) {
                if abort_only
                    && self.affirmed_at.is_none_or(|at| now.saturating_sub(at) >= WAIT_AFFIRM_MS)
                {
                    self.affirmed_at = Some(now);
                    if tail::last_event(&path) == Some(TailEvent::NeedsHuman { tool }) {
                        hints.push((hint, None));
                    }
                }
            }
        }
        for line in &lines {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else { continue };
            match classify_tail_record(&value) {
                TailEvent::Aborted => hints.push((TailHint::AbortedMidStream, None)),
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
        if !abort_only
            && hints.is_empty()
            && !cursor.tools.is_busy()
            && quiet >= TAIL_QUIET_MS
            && record.state == SessionState::Running
        {
            hints.push((TailHint::StaleQuiet, None));
        }
        hints
            .into_iter()
            .map(|(kind, preview)| RecoveryObservation {
                signal: Signal::TranscriptHint { kind },
                preview,
                source: "tail",
            })
            .collect()
    }
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
        // pane's own quiet, which a tool in flight keeps painting.
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
    use mesimon_core::board::{SessionKind, UnknownReason};
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
        // Answered: the tool result is the last word, and the wait is over.
        record.state = SessionState::RequiresAction { reason: Reason::Plan };
        history.append(serde_json::json!({"uuid":"answer", "type":"user", "message":{
            "content":[{"type":"tool_result","tool_use_id":"call1","content":"User has approved your plan."}]}}));
        assert!(recovery.poll(&record, RecoverySample::Transcript, 200_000).is_empty());
        // Nothing for a permission: the transcript cannot tell that dialog
        // from a running tool.
        record.state = SessionState::RequiresAction { reason: Reason::Permission };
        assert!(recovery.poll(&record, RecoverySample::Transcript, 300_000).is_empty());
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
}
