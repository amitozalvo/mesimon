//! Provider-owned invocation, observation, history and recovery. Board policy
//! supplies capabilities; adapters return normalized evidence to its writer.

pub mod claude;
pub mod codex;
pub mod transcript;

use mesimon_core::board::{AgentTools, ColumnSettings, SessionKind, SessionRecord};
use std::path::Path;

use crate::paths::Paths;

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AgentPreview {
    pub text: Option<String>,
    pub activity: Option<AgentActivity>,
    pub reply_key: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AgentActivity {
    Tool(String),
    Thinking,
}

/// The observation transport is selected by the adapter. The daemon applies
/// only normalized evidence from either source to the shared state machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservationMode {
    Hooks,
    Structured,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResumePolicy {
    FreshWhenHistoryMissing,
    ExactOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentCapabilities {
    pub observation: ObservationMode,
    pub resume: ResumePolicy,
}

pub fn adapter(kind: SessionKind) -> Option<&'static dyn AgentAdapter> {
    match kind {
        SessionKind::Claude => Some(&claude::Claude),
        SessionKind::Codex => Some(&codex::Codex),
        SessionKind::Bash => None,
    }
}

/// Native history formats are interpreted only by their owning adapter.
pub fn read_preview(provider: SessionKind, path: &Path) -> Option<AgentPreview> {
    adapter(provider)?.preview(path)
}

/// A page of a session's transcript for Remote Control (T-626), each record
/// read by its provider's adapter.
pub fn read_transcript(
    provider: SessionKind,
    path: &Path,
    ask: transcript::Ask,
) -> Option<transcript::Page> {
    let adapter = adapter(provider)?;
    transcript::page(path, ask, &|at, v| adapter.rows(at, v)).ok()
}

/// A bounded normalized artifact, independent of either native history format.
pub fn read_preview_artifact(path: &Path) -> Option<AgentPreview> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path).ok()?.take(65537).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 65536 {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    // Native provider metadata is also JSON. Its unknown keys must not
    // deserialize as an empty normalized preview and mask the history reader.
    if !value.as_object()?.contains_key("text") {
        return None;
    }
    serde_json::from_value(value).ok()
}

pub struct LaunchContext<'a> {
    pub paths: &'a Paths,
    pub cwd: &'a std::path::Path,
    pub session: uuid::Uuid,
    pub tools: AgentTools,
    pub brief: bool,
    pub column: ColumnSettings,
    /// Plan mode for THIS launch (T-434): the composer's or the ask field's
    /// `^p`, or the crown's `plan`. Overrides the column's `claude_mode`
    /// once; a later wake reads the column again. Claude reads it; Codex
    /// has no launch flag for its plan mode and ignores it.
    pub plan: bool,
    /// The agent tier this launch runs on (T-443), already resolved for the
    /// session's provider (`tier::Book::launch`): its model and effort ride
    /// argv, a built-in passes nothing.
    pub tier: mesimon_core::tier::Tier,
    /// T-573's research seam: a Claude Code plugin folder (a mod of function
    /// hooks) loaded with `--plugin-dir` for this launch, `MESIMON_MOD_DIR`.
    /// Off by default and read by the Claude adapter alone; Codex has no
    /// mods. Nothing is written for it: the flag is the whole installation.
    pub mod_dir: Option<std::path::PathBuf>,
    /// The road this launch takes (T-574, T-577), decided by
    /// `Daemon::launch_road`: on the mod the board's tools are the mod's
    /// `$.tool.register`, so no `--mcp-config` and no `--allowedTools` ride
    /// argv. Claude only; Codex's launch is always `Hooks`.
    pub road: mesimon_core::road::Road,
}

pub struct LaunchSpec {
    pub argv: Vec<String>,
    pub generation: Option<u64>,
}

impl LaunchSpec {
    pub fn plain(argv: Vec<String>) -> Self {
        Self { argv, generation: None }
    }
}

#[derive(Default)]
pub struct HookObservation {
    pub signal: Option<mesimon_core::attention::Signal>,
    pub detail: Option<String>,
    pub plan: Option<String>,
    pub metadata_changed: bool,
}

/// The daemon schedules common evidence sources; adapters decide eligibility,
/// interpret native records and retain their own recovery cursors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryChannel {
    Startup,
    Activity,
    Status,
    Transcript,
}

pub enum RecoverySample {
    Startup { has_output: bool, title: Option<String> },
    Activity { last_output_ms: u64 },
    Status,
    Transcript,
}

pub struct RecoveryObservation {
    pub signal: mesimon_core::attention::Signal,
    pub preview: Option<String>,
    pub source: &'static str,
}

/// Provider-private transient state. No method may mutate board state; the
/// daemon authorizes every returned observation before applying it.
pub trait AgentRecovery: Send {
    fn needs_poll(
        &mut self,
        _record: &SessionRecord,
        _channel: RecoveryChannel,
        _now: u64,
    ) -> bool {
        false
    }

    fn poll(
        &mut self,
        _record: &SessionRecord,
        _sample: RecoverySample,
        _now: u64,
    ) -> Vec<RecoveryObservation> {
        Vec::new()
    }
}

struct NoPassiveRecovery;
impl AgentRecovery for NoPassiveRecovery {}

/// Another process holding a record's conversation (`resume_guard`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalOwner {
    /// The owning process when discovery found one; `None` when ownership
    /// could not be verified at all (a Codex record with no runtime of ours).
    pub pid: Option<i32>,
    /// The words the refusal carries, such as `pid 123`.
    pub label: String,
}

impl std::fmt::Display for ExternalOwner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

pub trait AgentAdapter {
    fn capabilities(&self) -> AgentCapabilities;
    /// Structured transports need no heuristic inference from quiet panes or
    /// history. Providers that support passive recovery explicitly opt in.
    fn recovery(&self) -> Box<dyn AgentRecovery> {
        Box::new(NoPassiveRecovery)
    }
    fn discover(
        &self,
        _roots: &[std::path::PathBuf],
        _known: &dyn Fn(&str) -> bool,
    ) -> Vec<mesimon_core::command::ExternalItem> {
        Vec::new()
    }
    /// Called only by the authorized daemon writer. Parsing may update
    /// provider identity/history metadata, never board policy or state.
    fn parse_hook(
        &self,
        _frame: &crate::ingest::HookFrame,
        _record: &mut SessionRecord,
    ) -> HookObservation {
        HookObservation::default()
    }
    fn preview(&self, path: &Path) -> Option<AgentPreview>;
    /// What one transcript record at byte offset `at` shows a phone (T-626):
    /// none, or its rows in order.
    fn rows(
        &self,
        at: u64,
        record: &serde_json::Value,
    ) -> Vec<mesimon_core::mesophon::TranscriptRow>;
    fn conversation_key(&self, record: &SessionRecord) -> Option<String>;
    fn history_missing(&self, record: &SessionRecord) -> bool;
    /// Who else holds this record's conversation right now; no state
    /// mutation. The pid is what lets `resume_guard` tell the daemon's OWN
    /// previous pane, still going down after a sleep's SIGTERM, from a
    /// process somewhere else (T-381).
    fn external_owner(&self, record: &SessionRecord) -> Option<ExternalOwner>;
    fn normalize_title(&self, title: &str) -> String;
    fn start(&self, context: &LaunchContext<'_>, identity: &str) -> Result<LaunchSpec, String>;
    fn resume(
        &self,
        context: &LaunchContext<'_>,
        record: &SessionRecord,
    ) -> Result<LaunchSpec, String>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesimon_core::board::SessionState;

    #[test]
    fn capabilities_and_conversation_identity_follow_the_session_provider() {
        let mut session = SessionRecord::new(
            uuid::Uuid::from_u128(1),
            SessionKind::Claude,
            ulid::Ulid(1),
            vec![],
            "/repo".into(),
            SessionState::Sleeping,
        );
        let claude = adapter(session.kind).unwrap();
        assert_eq!(
            claude.capabilities(),
            AgentCapabilities {
                observation: ObservationMode::Hooks,
                resume: ResumePolicy::FreshWhenHistoryMissing
            }
        );
        assert_eq!(claude.conversation_key(&session), Some(session.id.to_string()));
        session.claude_session_id = Some(uuid::Uuid::from_u128(2));
        assert_eq!(claude.conversation_key(&session), Some(uuid::Uuid::from_u128(2).to_string()));
        session.kind = SessionKind::Codex;
        session.codex_thread_id = Some("codex-exact-thread".into());
        let codex = adapter(session.kind).unwrap();
        assert_eq!(
            codex.capabilities(),
            AgentCapabilities {
                observation: ObservationMode::Structured,
                resume: ResumePolicy::ExactOnly
            }
        );
        assert_eq!(codex.conversation_key(&session).as_deref(), Some("codex-exact-thread"));
        assert!(!codex.history_missing(&session));
        assert!(adapter(SessionKind::Bash).is_none());
    }

    #[test]
    fn title_normalization_preserves_provider_specific_marks() {
        assert_eq!(
            adapter(SessionKind::Claude).unwrap().normalize_title("✳ fix parser"),
            "fix parser"
        );
        assert_eq!(adapter(SessionKind::Codex).unwrap().normalize_title("* working"), "* working");
        for provider in [SessionKind::Claude, SessionKind::Codex] {
            let title =
                adapter(provider).unwrap().normalize_title(&format!("\u{7}{}", "x".repeat(100)));
            assert_eq!(title, "x".repeat(80));
        }
    }

    #[test]
    fn structured_provider_never_infers_state_from_passive_recovery() {
        use mesimon_core::board::{Provenance, UnknownReason};
        let mut record = SessionRecord::new(
            uuid::Uuid::from_u128(1),
            SessionKind::Codex,
            ulid::Ulid(1),
            vec![],
            "/repo".into(),
            SessionState::Running,
        );
        // Even Claude-shaped legacy fields cannot opt Codex into inference.
        record.transcript_path = Some("/nonexistent/claude-history.jsonl".into());
        record.provenance = Provenance::Adopted;
        let mut recovery = adapter(record.kind).unwrap().recovery();
        for state in [
            SessionState::Running,
            SessionState::Spawning,
            SessionState::Unknown { reason: UnknownReason::DaemonRestarted },
        ] {
            record.state = state;
            for channel in [
                RecoveryChannel::Startup,
                RecoveryChannel::Activity,
                RecoveryChannel::Status,
                RecoveryChannel::Transcript,
            ] {
                assert!(!recovery.needs_poll(&record, channel, u64::MAX));
            }
            for sample in [
                RecoverySample::Startup { has_output: true, title: None },
                RecoverySample::Activity { last_output_ms: 0 },
                RecoverySample::Status,
                RecoverySample::Transcript,
            ] {
                assert!(recovery.poll(&record, sample, u64::MAX).is_empty());
            }
        }
    }
}
