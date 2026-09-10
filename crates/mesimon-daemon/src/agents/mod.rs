//! Provider-owned launch configuration. Board policy supplies capabilities;
//! adapters translate them into the native agent's invocation.

pub mod claude;
pub mod codex;

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

pub trait AgentAdapter {
    fn capabilities(&self) -> AgentCapabilities;
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
    fn conversation_key(&self, record: &SessionRecord) -> Option<String>;
    fn history_missing(&self, record: &SessionRecord) -> bool;
    /// Human-readable owner identity, such as `pid 123`; no state mutation.
    fn external_owner(&self, record: &SessionRecord) -> Option<String>;
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
}
