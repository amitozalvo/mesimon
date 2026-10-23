//! Native Codex integration: protocol observation and lifecycle stay here.

pub mod discovery;
pub mod observation;
mod recovery;
pub use recovery::{
    recovery_endpoint_absent, recovery_launch_target, recovery_owner_absent,
    restore_unverified_cleanup, retain_unverified_cleanup, RecoveryLaunchTarget,
};
pub mod rpc;
pub mod runtime;

use mesimon_core::board::SessionState;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::{AgentAdapter, ExternalOwner, LaunchContext, LaunchSpec};
use crate::hook_settings::mesimon_bin;

pub struct Codex;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub session: uuid::Uuid,
    pub generation: u64,
    pub cwd: PathBuf,
    pub executable: String,
    pub upstream_socket: PathBuf,
    pub proxy_socket: PathBuf,
    pub snapshot_path: PathBuf,
    pub preview_path: PathBuf,
    pub resume: Option<String>,
    pub config_flags: Vec<String>,
    pub env: Vec<(String, String)>,
}

pub fn snapshot_path(paths: &crate::paths::Paths, id: uuid::Uuid) -> PathBuf {
    paths.rt_dir.join(format!("cdx-{}.json", &id.simple().to_string()[..16]))
}

pub fn preview_path(paths: &crate::paths::Paths, id: uuid::Uuid) -> PathBuf {
    paths.hooks_dir().join(format!("{id}.preview.json"))
}

/// Input readiness is independent of the configurable status line. Require
/// the native text cursor in the composer; menus hide it, and tmux copy mode
/// has no application cursor. Structured idle observation gates delivery too.
pub fn input_ready(screen: &mesimon_backend_tmux::InputScreen) -> bool {
    let Some((x, y)) = screen.cursor else { return false };
    let Some(above) = screen.lines.get(..=y) else { return false };
    let Some(prompt) = above.iter().rposition(|line| {
        line.strip_prefix('›').is_some_and(|text| text.is_empty() || text.starts_with(' '))
    }) else {
        return false;
    };
    // Wrapped/multiline input keeps the composer's two-column indentation.
    // Nothing below the cursor (including any status line) participates.
    x >= 2 && above[prompt + 1..].iter().all(|line| line.is_empty() || line.starts_with("  "))
}

/// An `input_ready` composer whose cursor has left the start of its prompt
/// line: it holds text. An empty box shows its placeholder in the same cells
/// a typed word would, so the cursor is the one sign (T-845): a steer's
/// Enter repeats only while the pasted words still stand.
pub fn composer_holds_text(screen: &mesimon_backend_tmux::InputScreen) -> bool {
    let Some((x, y)) = screen.cursor else { return false };
    input_ready(screen)
        && !(x == 2 && screen.lines.get(y).is_some_and(|line| line.starts_with('›')))
}

/// A Codex dialog stands in the box's place (T-907): the pane's last `›`
/// row is a numbered choice (`› 1. Yes, proceed (y)`), as an approval's, a
/// question's and the plan's lists draw their cursor. The last such row,
/// because the conversation above keeps every prompt the person sent, each
/// behind a `›` of its own.
pub fn dialog_shown(screen: &mesimon_backend_tmux::InputScreen) -> bool {
    let numbered = |row: &str| {
        row.trim_start()
            .split_once(". ")
            .is_some_and(|(n, _)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    };
    !input_ready(screen)
        && screen
            .lines
            .iter()
            .rev()
            .find_map(|line| line.trim_start().strip_prefix('›'))
            .is_some_and(numbered)
}

/// What words into a running Codex turn found in the pane (T-907).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SteerSight {
    Composer,
    Dialog,
    /// Neither, on every read: a redraw that outlasted them, or a screen
    /// no rule here knows.
    Unseen,
}

/// How many times a steer reads the pane, and how far apart (T-907).
pub const STEER_READS: usize = 3;
pub const STEER_READ_GAP_MS: u64 = 100;

/// Read the pane up to [`STEER_READS`] times (`read(i)`, the caller waiting
/// between) until it shows the box or a dialog. Codex hides its cursor
/// while it redraws a running turn's status line, so one read can land
/// mid-frame with the box intact (measured on 0.163: 1 read in 4,000 on a
/// quiet turn, more under a tool's streaming output) and refused the words
/// as a dialog no one could see. A dialog read stops at once; a failed read
/// is the pane's, not a frame's, and fails the whole.
pub fn steer_sight<E>(
    mut read: impl FnMut(usize) -> Result<mesimon_backend_tmux::InputScreen, E>,
) -> Result<SteerSight, E> {
    for i in 0..STEER_READS {
        let screen = read(i)?;
        if input_ready(&screen) {
            return Ok(SteerSight::Composer);
        }
        if dialog_shown(&screen) {
            return Ok(SteerSight::Dialog);
        }
    }
    Ok(SteerSight::Unseen)
}

/// Whether a steer's due Enter goes (T-872): the pane read at the key must
/// show the words still standing — never a dialog a running turn raised
/// since — and `live` is read only when the sampler's screen from after the
/// paste does too, or when none has landed, as on a machine whose samples
/// run slow: a missing sample never costs the Enter.
pub fn steer_presses(
    sampled: Option<&mesimon_backend_tmux::InputScreen>,
    live: impl FnOnce() -> Option<mesimon_backend_tmux::InputScreen>,
) -> bool {
    sampled.is_none_or(composer_holds_text) && live().is_some_and(|s| composer_holds_text(&s))
}

/// The composer's footer names Plan mode (T-847): Codex 0.163 writes it at
/// the right of the line under the box once `/plan` took (`… · cwd… Plan
/// mode`). Only below the cursor, the box's own row excluded, so neither the
/// conversation nor the slash menu's `switch to Plan mode` reads as it. A
/// configured status line may leave it out; the runtime's
/// `collaboration_mode` is the other witness.
pub fn plan_mode_shown(screen: &mesimon_backend_tmux::InputScreen) -> bool {
    let Some((_, y)) = screen.cursor else { return false };
    input_ready(screen)
        && !composer_holds_text(screen)
        && screen.lines.iter().skip(y + 1).any(|line| line.trim_end().ends_with("Plan mode"))
}

/// The native implementation dialog is local UI, not an app-server request.
/// Require its choice list as well as the heading to reject conversational text.
pub fn plan_dialog(screen: &mesimon_backend_tmux::InputScreen) -> bool {
    let lines = &screen.lines;
    let Some(index) = lines.iter().rposition(|line| line.trim() == "Implement this plan?") else {
        return false;
    };
    let tail = &lines[index..];
    tail.iter().any(|line| line.contains("Yes, implement this plan"))
        && tail.iter().any(|line| line.contains("No, stay in Plan mode"))
        && !input_ready(screen)
}

pub fn startup_attention(
    screen: &mesimon_backend_tmux::InputScreen,
) -> Option<mesimon_core::board::Reason> {
    use mesimon_core::board::Reason;
    if input_ready(screen) {
        return None;
    }
    let text = screen.lines.join("\n");
    // Codex 0.163 changed the folder heading. This is a native startup
    // dialog, before any thread event can tell the board it needs a person.
    let folder_trust =
        screen.lines.iter().any(|line| line.trim_start().starts_with("Trust this folder?"))
            && text.contains("Trust and continue");
    if folder_trust || text.contains("Hooks need review") || text.contains("Do you trust") {
        Some(Reason::Trust)
    } else if text.contains("Sign in with ChatGPT") || text.contains("Sign in with an API key") {
        Some(Reason::Auth)
    } else {
        None
    }
}

fn policy_flags(column: &mesimon_core::board::ColumnSettings) -> Vec<String> {
    use mesimon_core::board::{CodexApproval, CodexSandbox};
    let mut flags = Vec::new();
    let sandbox = match column.codex_sandbox {
        CodexSandbox::Inherit => None,
        CodexSandbox::ReadOnly => Some("read-only"),
        CodexSandbox::WorkspaceWrite => Some("workspace-write"),
    };
    let approval = match column.codex_approval {
        CodexApproval::Inherit => None,
        CodexApproval::OnRequest => Some("on-request"),
        CodexApproval::Never => Some("never"),
    };
    for (key, value) in [("sandbox_mode", sandbox), ("approval_policy", approval)] {
        if let Some(value) = value {
            flags.extend(["-c".into(), format!("{key}=\"{value}\"")]);
        }
    }
    flags
}

/// The ticket's tier as Codex config (T-443): `model` and
/// `model_reasoning_effort`, the keys `~/.codex/config.toml` itself uses.
/// `-c` rather than `-m`, because `config_flags` reach both the app server
/// and the native client, and a resumed thread takes them too. The model is
/// `tier::check_model`'s whitelist, so it needs no TOML escaping.
fn tier_flags(tier: &mesimon_core::tier::Tier) -> Vec<String> {
    let mut flags = Vec::new();
    if tier.provider != mesimon_core::board::AgentProvider::Codex {
        return flags;
    }
    if let Some(model) = tier.model_arg() {
        flags.extend(["-c".into(), format!("model=\"{model}\"")]);
    }
    if let Some(effort) = tier.effort_arg() {
        flags.extend(["-c".into(), format!("model_reasoning_effort=\"{effort}\"")]);
    }
    flags
}

fn prepare(context: &LaunchContext<'_>, resume: Option<String>) -> Result<LaunchSpec, String> {
    if context.managed_git_worktree {
        // Codex 0.163.0's remote folder check needs a project config layer to
        // see the main checkout's existing trust. Even an empty .codex/ gives
        // its loader that layer; native trust resolution still decides whether
        // to enable it. Do this on resumes too, for older ticket worktrees.
        // One mkdir never edits existing files or follows a .codex symlink.
        // Best effort: failure leaves native onboarding to ask as before.
        let _ = std::fs::create_dir(context.cwd.join(".codex"));
    }
    let id = context.session;
    let generation = (uuid::Uuid::new_v4().as_u128() as u64).max(1);
    let stem = format!("cdx-{}-{:08x}", &id.simple().to_string()[..16], generation as u32);
    let mut config_flags = policy_flags(&context.column);
    config_flags.extend(tier_flags(&context.tier));
    // Codex's startup update prompt draws before the native client dials the
    // relay, so a launch that shows it times out, and its "Update now" runs
    // the installer in the pane under the live app-server, replacing the
    // binary both processes run (T-923). Codex updates outside mesimon.
    config_flags.extend(["-c".into(), "check_for_update_on_startup=false".into()]);
    // Stable definitions let native /hooks trust survive ticket changes.
    // Identity and guarded roots are invocation environment, not hook text.
    // Codex 0.153.4 preserves user hooks from both config.toml and hooks.json
    // beside these per-launch entries (bounded runtime probe).
    let quoted = mesimon_core::text::sh_quote(&mesimon_bin().display().to_string());
    let gate = serde_json::to_string(&format!("{quoted} gate --provider codex --from-env"))
        .map_err(|e| e.to_string())?;
    config_flags.extend(["-c".into(), format!("hooks.PreToolUse=[{{matcher=\"^apply_patch$\",hooks=[{{type=\"command\",command={gate},timeout=2}}]}}]")]);
    let brief = context.brief && context.tools != mesimon_core::board::AgentTools::Off;
    if brief {
        let command =
            serde_json::to_string(&format!("{quoted} agent-brief")).map_err(|e| e.to_string())?;
        config_flags.extend([
            "-c".into(),
            format!(
                "hooks.SessionStart=[{{hooks=[{{type=\"command\",command={command},timeout=2}}]}}]"
            ),
        ]);
    }
    if context.tools != mesimon_core::board::AgentTools::Off {
        let rendered =
            crate::hook_settings::mcp_config_json(context.paths, &mesimon_bin(), id, context.tools);
        let value: serde_json::Value =
            serde_json::from_str(&rendered).map_err(|e| e.to_string())?;
        let server = &value["mcpServers"]["mesimon"];
        config_flags.extend([
            "-c".into(),
            format!(
                "mcp_servers.mesimon={{command={},args={}}}",
                server["command"], server["args"],
            ),
        ]);
    }
    let config = RuntimeConfig {
        session: id,
        generation,
        cwd: context.cwd.to_path_buf(),
        executable: std::env::var("MESIMON_CODEX_BIN").unwrap_or_else(|_| "codex".into()),
        upstream_socket: context.paths.rt_dir.join(format!("{stem}-up.sock")),
        proxy_socket: context.paths.rt_dir.join(format!("{stem}-ui.sock")),
        snapshot_path: snapshot_path(context.paths, id),
        preview_path: preview_path(context.paths, id),
        resume,
        config_flags,
        env: vec![
            ("MESIMON_AGENT_BRIEF".into(), if brief { "1" } else { "0" }.into()),
            ("MESIMON_SESSION".into(), id.to_string()),
            ("MESIMON_GATE_BOARD".into(), context.paths.board_dir.display().to_string()),
            ("MESIMON_GATE_STATE".into(), context.paths.state_dir.display().to_string()),
            ("MESIMON_GATE_ALLOW".into(), context.paths.worktrees_root().display().to_string()),
            ("MESIMON_HOOK_SOCK".into(), context.paths.hook_sock().display().to_string()),
        ],
    };
    let path = context.paths.hooks_dir().join(format!("{id}.codex.json"));
    write_json(&path, &config).map_err(|e| format!("Codex runtime configuration: {e}"))?;
    Ok(LaunchSpec {
        argv: vec![
            std::env::var("MESIMON_CODEX_RUNTIME_BIN")
                .unwrap_or_else(|_| mesimon_bin().display().to_string()),
            "agent-runtime".into(),
            "--config".into(),
            path.display().to_string(),
        ],
        generation: Some(generation),
    })
}

impl AgentAdapter for Codex {
    fn discover(
        &self,
        roots: &[PathBuf],
        known: &dyn Fn(&str) -> bool,
    ) -> Vec<mesimon_core::command::ExternalItem> {
        discovery::scan(&discovery::codex_home(), roots, known)
    }

    fn capabilities(&self) -> super::AgentCapabilities {
        super::AgentCapabilities {
            observation: super::ObservationMode::Structured,
            resume: super::ResumePolicy::ExactOnly,
        }
    }

    fn preview(&self, path: &Path) -> Option<super::AgentPreview> {
        super::read_preview_artifact(path).or_else(|| discovery::read_preview(path))
    }

    fn rows(
        &self,
        at: u64,
        record: &serde_json::Value,
    ) -> Vec<mesimon_core::mesophon::TranscriptRow> {
        discovery::rows(at, record)
    }

    fn conversation_key(&self, record: &mesimon_core::board::SessionRecord) -> Option<String> {
        record.codex_thread_id.clone().filter(|id| !id.is_empty())
    }

    fn history_missing(&self, _record: &mesimon_core::board::SessionRecord) -> bool {
        // Native Codex owns exact-resume validation; never substitute a fresh conversation.
        false
    }

    fn external_owner(&self, record: &mesimon_core::board::SessionRecord) -> Option<ExternalOwner> {
        let identity = record.codex_thread_id.as_deref()?;
        let path =
            record.transcript_path.as_deref().map(Path::new).unwrap_or_else(|| Path::new(""));
        match discovery::external_owner(identity, Path::new(&record.cwd), path) {
            discovery::Ownership::Live(pid) => {
                Some(ExternalOwner { pid: Some(pid), label: format!("Codex process {pid}") })
            }
            discovery::Ownership::Unknown if record.argv.is_empty() => {
                Some(ExternalOwner { pid: None, label: "Codex ownership unverified".into() })
            }
            // An owned session has durable cleanup evidence from its former
            // runtime. Unknown unrelated servers do not erase that evidence;
            // a positively matched external writer still refuses above.
            _ => None,
        }
    }

    fn start(&self, context: &LaunchContext<'_>, _identity: &str) -> Result<LaunchSpec, String> {
        prepare(context, None)
    }

    fn resume(
        &self,
        context: &LaunchContext<'_>,
        record: &mesimon_core::board::SessionRecord,
    ) -> Result<LaunchSpec, String> {
        let identity = record
            .codex_thread_id
            .clone()
            .filter(|id| !id.is_empty())
            .ok_or("Codex thread identity is unavailable; cannot resume this conversation")?;
        prepare(context, Some(identity))
    }
}

pub fn write_json(path: &Path, value: &impl Serialize) -> anyhow::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4().simple()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&serde_json::to_vec(value)?)?;
        std::fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// Missing legacy evidence never proves that no conversation was created.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchPhase {
    #[default]
    Unknown,
    BeforeSelection,
    SelectionPending,
    Selected,
}

/// The supervised runtime's current observation, independent of board state.
/// Only the daemon writer applies this evidence to sessions and automation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub session: uuid::Uuid,
    pub generation: u64,
    pub sequence: u64,
    pub heartbeat_ms: u64,
    pub thread_id: Option<String>,
    #[serde(default)]
    pub launch_phase: LaunchPhase,
    pub turn_id: Option<String>,
    pub state: SessionState,
    pub observation_hold: bool,
    pub history_path: Option<String>,
    /// Native foreground thread name, independent of terminal OSC titles.
    #[serde(default)]
    pub title: Option<String>,
    /// Complete provider plan item, never a stream delta or an approval claim.
    #[serde(default)]
    pub plan: Option<String>,
    #[serde(default)]
    pub plan_key: Option<String>,
    /// The server request that waits on a person, while it does (T-846): a
    /// question's words and options, or an approval's command, as the
    /// request carried them, keyed by its id; gone on
    /// `serverRequest/resolved`. The daemon projects it into the card
    /// Remote Control and the crown answer. Bounded, never a reply's text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<Ask>,
    /// Durable evidence that this generation and its owned server have stopped.
    #[serde(default)]
    pub stopped: bool,
    /// The app-server's latest `account/rateLimits/updated` params (T-327),
    /// verbatim and bounded, and when they arrived: the session's own quota
    /// report, which spares the daemon a probe while Codex is working.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limits: Option<serde_json::Value>,
    #[serde(default)]
    pub rate_limits_at_ms: u64,
    /// Why the hold stands, while it does (T-843): read off the file, never
    /// guessed at, when a seat stays `Running` after its turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold: Option<Hold>,
    /// How many times the ledger dropped stale bookkeeping after the server
    /// closed a turn and the idle audit found every thread quiet (T-843),
    /// and the last such release's reasons.
    #[serde(default)]
    pub releases: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_release: Option<String>,
    /// How many `turn/steer` requests on the foreground thread the server
    /// accepted (T-845): words typed into a running turn and taken by it.
    /// The daemon's ack for a steer it pasted is this count rising.
    #[serde(default)]
    pub steers: u64,
    /// The foreground thread's collaboration mode as the wire last said it
    /// (T-847): `plan` or `default`, from the server's
    /// `thread/settings/updated` and the native client's own
    /// `thread/settings/update` and `turn/start`. `None` until one of them
    /// names it: a thread starts in its default without saying so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collaboration_mode: Option<String>,
}

/// A server request that waits on a person, as the snapshot carries it
/// (T-846): the content the native dialog shows, so a phone or the crown
/// can answer the same dialog by its keys. Bounds: `Ask::QUESTIONS`
/// questions of `Ask::OPTIONS` options, `Ask::TEXT` chars a field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ask {
    /// The request's id, as `serverRequest/resolved` names it.
    pub id: String,
    #[serde(flatten)]
    pub kind: AskKind,
}

impl Ask {
    pub const QUESTIONS: usize = 4;
    pub const OPTIONS: usize = 8;
    pub const TEXT: usize = 1000;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AskKind {
    /// `item/tool/requestUserInput`: its questions, in order. A secret
    /// question is carried as `secret` and never projected.
    Question { questions: Vec<AskQuestion> },
    /// A `requestApproval` of any kind, or Codex's one-tool MCP approval:
    /// `what` names the kind in a word (`command`, `edit`, `permissions`,
    /// `mcp_tool`), the rest is what the dialog shows.
    Approval {
        what: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        decisions: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskQuestion {
    pub question: String,
    #[serde(default)]
    pub header: String,
    #[serde(default)]
    pub options: Vec<AskOption>,
    #[serde(default)]
    pub secret: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskOption {
    pub label: String,
    #[serde(default)]
    pub description: String,
}

/// Why the runtime's ledger holds the seat, as the snapshot carries it
/// (T-843). A Codex session sat `Running` for an hour after Codex's own
/// rollout said `task_complete`, its phone reading "replying", and nothing on
/// disk said which of the ledger's six buckets held it. Counts and kinds only:
/// never an identity, never model text.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hold {
    /// The kinds of the items started and never completed, up to eight.
    #[serde(default)]
    pub items: Vec<String>,
    #[serde(default)]
    pub hooks: usize,
    /// Server requests that ask a person (an approval, a question).
    #[serde(default)]
    pub requests: usize,
    /// Server requests of a kind the ledger does not know, held as work.
    #[serde(default)]
    pub work_requests: usize,
    #[serde(default)]
    pub children: usize,
    #[serde(default)]
    pub flags: usize,
    #[serde(default)]
    pub late_work: bool,
    /// The capture is not continuous (a reconnect, a lost frame, capacity).
    #[serde(default)]
    pub discontinuous: bool,
    /// The server closed the turn (`turn/completed`) while the hold stayed.
    #[serde(default)]
    pub turn_closed: bool,
    /// Native requests still in flight; the audit waits for them.
    #[serde(default)]
    pub native_requests: usize,
    /// When this hold began, so a reader can tell a stall from a moment.
    #[serde(default)]
    pub since_ms: u64,
    /// When the turn closed under this hold; 0 while it runs.
    #[serde(default)]
    pub closed_since_ms: u64,
}

impl Hold {
    /// One line for the journal: the buckets that are not empty.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if !self.items.is_empty() {
            parts.push(format!("items {}", self.items.join(",")));
        }
        for (name, count) in [
            ("hooks", self.hooks),
            ("requests", self.requests),
            ("work requests", self.work_requests),
            ("children", self.children),
            ("flags", self.flags),
            ("native requests", self.native_requests),
        ] {
            if count > 0 {
                parts.push(format!("{name} {count}"));
            }
        }
        if self.late_work {
            parts.push("late work".into());
        }
        if self.discontinuous {
            parts.push("discontinuous".into());
        }
        if parts.is_empty() {
            "nothing pending".into()
        } else {
            parts.join(" ∙ ")
        }
    }
}

/// What a snapshot file looked like when it was last parsed: length, mtime
/// and inode. `write_json` renames a fresh file in, so a rewrite always
/// moves the inode, whatever the filesystem's mtime grain.
type Stamp = (u64, Option<std::time::SystemTime>, u64);

fn stamp_of(path: &Path) -> Option<Stamp> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok(), meta.ino()))
}

/// The snapshot file at `path`, or `None` where it is missing, oversized or
/// not a snapshot. Lost evidence is the daemon's to judge.
fn read_snapshot(path: &Path) -> Option<Snapshot> {
    use std::io::Read;
    let mut data = Vec::new();
    std::fs::File::open(path).ok()?.take(65537).read_to_end(&mut data).ok()?;
    if data.len() > 65536 {
        return None;
    }
    serde_json::from_slice(&data).ok()
}

/// The observer: one worker for the daemon's life (T-688), asked each tick
/// for the live runtimes' snapshot files and answering each ask through
/// `report`. A file is parsed again only when its `Stamp` moved; until then
/// the last parse is reported again, and the daemon judges it by the
/// heartbeat it carries, as it judged every re-parse before. The worker ends
/// with its asker or with the first report nobody takes.
pub fn spawn_observer(
    report: impl Fn(Vec<(uuid::Uuid, Option<Snapshot>)>) -> bool + Send + 'static,
) -> std::sync::mpsc::Sender<Vec<(uuid::Uuid, PathBuf)>> {
    let (ask, asks) = std::sync::mpsc::channel::<Vec<(uuid::Uuid, PathBuf)>>();
    std::thread::spawn(move || {
        let mut seen: std::collections::HashMap<PathBuf, (Stamp, Option<Snapshot>)> =
            std::collections::HashMap::new();
        while let Ok(paths) = asks.recv() {
            seen.retain(|p, _| paths.iter().any(|(_, q)| q == p));
            let snapshots = paths
                .into_iter()
                .map(|(id, path)| {
                    let snapshot = match (stamp_of(&path), seen.get(&path)) {
                        (Some(stamp), Some((was, kept))) if *was == stamp => kept.clone(),
                        (Some(stamp), _) => {
                            let fresh = read_snapshot(&path);
                            seen.insert(path, (stamp, fresh.clone()));
                            fresh
                        }
                        (None, _) => {
                            seen.remove(&path);
                            None
                        }
                    };
                    (id, snapshot)
                })
                .collect();
            if !report(snapshots) {
                break;
            }
        }
    });
    ask
}

/// Why a socket probe could not be made: a path no `sockaddr_un` holds,
/// or the socket or its nonblocking flag refused.
pub(crate) enum ProbeFail {
    Path,
    Socket(std::io::Error),
    NonBlock(std::io::Error),
}

/// A nonblocking connect to the Unix socket at `path`, sending nothing: the
/// inner result is the connect's, so a full accept backlog is never waited
/// on. Both of Codex's liveness checks ask through this one `unsafe` block.
pub(crate) fn connect_probe(path: &Path) -> Result<std::io::Result<()>, ProbeFail> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    let bytes = path.as_os_str().as_bytes();
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.contains(&0) || bytes.len() >= address.sun_path.len() {
        return Err(ProbeFail::Path);
    }
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    #[cfg(target_os = "macos")]
    {
        address.sun_len = std::mem::size_of_val(&address) as u8;
    }
    for (target, source) in address.sun_path.iter_mut().zip(bytes) {
        *target = *source as libc::c_char;
    }
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err(ProbeFail::Socket(std::io::Error::last_os_error()));
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) } < 0 {
        return Err(ProbeFail::NonBlock(std::io::Error::last_os_error()));
    }
    let result = unsafe {
        libc::connect(
            fd.as_raw_fd(),
            (&address as *const libc::sockaddr_un).cast(),
            std::mem::size_of_val(&address) as libc::socklen_t,
        )
    };
    Ok(if result == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) })
}

#[cfg(test)]
mod input_tests {
    use super::*;
    use mesimon_backend_tmux::InputScreen;

    fn screen(text: &str, cursor: Option<(usize, usize)>) -> InputScreen {
        InputScreen { lines: text.lines().map(str::to_string).collect(), cursor }
    }

    #[test]
    fn input_cursor_is_independent_of_every_status_line() {
        for footer in [
            "",
            "? for shortcuts",
            "100% context left",
            "my arbitrary status",
            "mesimon · gpt-6-astra high · Context 0% used · weekly 42% left",
            "gpt-6-astra high · /repo · main · Ready",
            "› custom status",
            "Hooks need review",
        ] {
            assert!(
                input_ready(&screen(
                    &format!("› Ask Codex to do anything\n{footer}"),
                    Some((2, 0))
                )),
                "{footer}"
            );
        }
        assert!(input_ready(&screen("› multiline\n  continuation\n\n  last line", Some((11, 3)))));
    }

    #[test]
    fn input_requires_the_active_cursor_inside_the_composer() {
        assert!(!input_ready(&screen("› text\n? for shortcuts", None)));
        assert!(!input_ready(&screen("› text", Some((0, 0)))));
        assert!(!input_ready(&screen("› text\nloading", Some((7, 1)))));
        assert!(!input_ready(&screen("› text", Some((2, 5)))));
        assert!(!input_ready(&screen("loading", Some((2, 0)))));
        // A stale composer above a different text field is not its cursor.
        assert!(!input_ready(&screen("› old prompt\nSearch sessions\n  query", Some((7, 2)))));
        for modal in
            ["Hooks need review", "Do you trust", "Select a model", "An unknown future dialog"]
        {
            assert!(!input_ready(&screen(&format!("› text\n{modal}\n› 1. Yes"), None)));
        }
    }

    /// Mid-turn the composer sits under the turn's status line and is still
    /// a box (T-845); the cursor alone says whether it holds words.
    #[test]
    fn a_running_turns_composer_is_a_box_and_its_cursor_says_whether_it_holds_text() {
        let working =
            "• Working (12s • esc to interrupt)\n\n› Ask Codex to do anything\n\n  ? for shortcuts";
        let empty = screen(working, Some((2, 2)));
        assert!(input_ready(&empty));
        assert!(!composer_holds_text(&empty));
        let typed = screen(
            "• Working (12s • esc to interrupt)\n\n› also check the docs\n\n  ? for shortcuts",
            Some((21, 2)),
        );
        assert!(input_ready(&typed) && composer_holds_text(&typed));
        let wrapped = screen("› a long steer that\n  wraps", Some((2, 1)));
        assert!(composer_holds_text(&wrapped));
        let dialog = screen("Allow command?\n› 1. Yes\n  2. No", None);
        assert!(!input_ready(&dialog) && !composer_holds_text(&dialog));
    }

    /// A steer's Enter (T-872): the live read decides, the sampler's screen
    /// only spares it. No sample after the paste (a slow sampler) still
    /// presses into a box holding the words; a sample showing the box empty
    /// reads nothing live; a dialog at the key is never pressed into.
    #[test]
    fn a_steer_presses_on_the_live_box_and_a_missing_sample_never_costs_the_enter() {
        let held = || screen("› also check the docs", Some((21, 0)));
        let empty = || screen("› Ask Codex to do anything", Some((2, 0)));
        let dialog = || screen("Allow command?\n› 1. Yes\n  2. No", None);
        let reads = std::cell::Cell::new(0);
        let live = |s: InputScreen| {
            let reads = &reads;
            move || {
                reads.set(reads.get() + 1);
                Some(s)
            }
        };
        assert!(steer_presses(None, live(held())), "no sample landed: the live box decides");
        assert!(steer_presses(Some(&held()), live(held())));
        assert!(!steer_presses(None, live(dialog())), "a dialog raised since is never answered");
        assert!(!steer_presses(Some(&held()), live(dialog())));
        assert!(!steer_presses(None, || None), "a pane that cannot be read is not pressed");
        assert_eq!(reads.get(), 4);
        assert!(!steer_presses(Some(&empty()), live(held())), "the words went");
        assert_eq!(reads.get(), 4, "a fresh sample of an empty box forks nothing");
    }

    /// T-904's Codex seat on 2026-10-22, 24 minutes into a tool call: the
    /// box intact under the turn's status line. Its cursor is the frame's.
    const WORKING: &str = "\
• Ran python3 -B ci/rig.py --only R3
  └ == R3 running

• Working (24m 13s • esc to interrupt)
  └ Tip: Use /keymap to configure keyboard shortcuts.


› Ask Codex to do anything

  T-904-allow-agents-to-interact-with · GPT-6-Astra high · Context 51% used · weekly 96% left
  ? for shortcuts";

    /// Codex 0.163.0's command approval as captured on 2026-10-17 (T-846).
    const APPROVAL: &str = "\
  Would you like to run the following command?

  Environment: local

  Reason: May I write this disposable fixture file?

  $ printf MESIMON_REFUSED_WRITE >
  /private/tmp/msmn-e2e-codex-runtime-apj3xgcw/approval-refused.txt


› 1. Yes, proceed (y)
  2. Yes, and don't ask again for commands that start with `printf
     MESIMON_REFUSED_WRITE > /private/tmp/msmn-e2e-codex-runtime-apj3xgcw/
     approval-refused.txt` (p)
  3. No, and tell Codex what to do differently (esc)

  Press enter to confirm or esc to cancel";

    /// A send-now into a working turn (T-907): a frame read mid-redraw (the
    /// cursor hidden, as measured, or parked on the status line) is neither
    /// the box nor a dialog, and the next read finds the box; a real
    /// approval refuses on its first read, in the dialog's words.
    #[test]
    fn a_redraw_never_reads_as_a_dialog_and_a_real_one_refuses_the_steer() {
        let composer = screen(WORKING, Some((2, 7)));
        assert!(input_ready(&composer) && !dialog_shown(&composer));
        for frame in [screen(WORKING, None), screen(WORKING, Some((36, 3)))] {
            assert!(!input_ready(&frame), "{:?}", frame.cursor);
            assert!(!dialog_shown(&frame), "a redraw is no dialog: {:?}", frame.cursor);
        }
        let approval = screen(APPROVAL, None);
        assert!(!input_ready(&approval) && dialog_shown(&approval));
        let question =
            screen("  Choose fixture color?\n\n  › 1. Blue  Use blue.\n    2. Green", None);
        assert!(dialog_shown(&question));
        // A prompt the person once sent, `› 1. …` in the conversation, is
        // above the box: the box is the last `›` row.
        let history = screen(&format!("› 1. Do the first step\n{WORKING}"), None);
        assert!(!dialog_shown(&history));

        let reads = |frames: Vec<InputScreen>| {
            let mut frames = frames.into_iter();
            let taken = std::cell::Cell::new(0);
            let sight = steer_sight(|i| {
                taken.set(i + 1);
                frames.next().ok_or("no pane")
            });
            (sight, taken.get())
        };
        let redraw = || screen(WORKING, None);
        assert_eq!(
            reads(vec![redraw(), screen(WORKING, Some((2, 7)))]),
            (Ok(SteerSight::Composer), 2),
            "the second read finds the box"
        );
        assert_eq!(reads(vec![redraw(), redraw(), redraw()]), (Ok(SteerSight::Unseen), 3));
        assert_eq!(reads(vec![screen(APPROVAL, None)]), (Ok(SteerSight::Dialog), 1));
        assert_eq!(reads(vec![redraw()]), (Err("no pane"), 2), "a pane that cannot be read");
    }

    #[test]
    fn startup_trust_recognizes_current_and_older_native_dialogs() {
        use mesimon_core::board::Reason;
        for dialog in [
            "/repo/worktree\n  Trust this folder?\n› 1. Trust and continue\n  2. Quit",
            "  Trust this folder? Codex can read, edit, and run files here, subject to\n  your permission settings.\n› 1. Trust and continue\n  2. Quit",
            "Do you trust the contents of this directory?\n› 1. Yes, proceed",
            "Hooks need review\n1. Review hooks\n2. Trust all and continue",
        ] {
            assert_eq!(startup_attention(&screen(dialog, None)), Some(Reason::Trust));
            let composer = format!("{dialog}\n› ");
            assert_eq!(
                startup_attention(&screen(&composer, Some((2, composer.lines().count() - 1)))),
                None,
                "a live composer wins over historical dialog text"
            );
        }
        for text in [
            "The question was: Trust this folder?",
            "Trust this folder?\nAn explanation without the native choices",
            "Select a model\n› 1. Continue",
        ] {
            assert_eq!(startup_attention(&screen(text, None)), None);
        }
        assert_eq!(startup_attention(&screen("Sign in with ChatGPT", None)), Some(Reason::Auth));
    }

    #[test]
    fn plan_mode_is_read_off_the_footer_and_nowhere_else() {
        // Codex 0.163's own lines, after `/plan` took and before.
        let on = "› Ask Codex to do anything\n\n  GPT-6.1-Sol medium · /tmp/probe Plan mode\n  ← for agents · ? for shortcuts";
        assert!(plan_mode_shown(&screen(on, Some((2, 0)))));
        let off = "› Ask Codex to do anything\n\n  GPT-6.1-Sol default · /tmp/probe\n  ← for agents · ? for shortcuts";
        assert!(!plan_mode_shown(&screen(off, Some((2, 0)))));
        // The conversation above the box, and the slash menu in it, say nothing.
        let above = "• Model changed to gpt-6.1-sol medium for Plan mode\n› /plan  switch to Plan mode\n› Ask Codex to do anything\n\n  GPT-6.1-Sol default · /tmp/probe";
        assert!(!plan_mode_shown(&screen(above, Some((2, 2)))));
        // Words still in the box: the slash command has not gone yet.
        assert!(!plan_mode_shown(&screen("› /plan\n\n  GPT · /tmp Plan mode", Some((7, 0)))));
        // A dialog hides the cursor: nothing is read.
        assert!(!plan_mode_shown(&screen(on, None)));
    }

    #[test]
    fn plan_modal_is_distinct_from_plan_text_and_native_composer() {
        let modal = screen("Implement this plan?\n› 1. Yes, implement this plan\n  2. Yes, clear context and implement\n  3. No, stay in Plan mode\nPress enter to confirm or esc to go back", None);
        assert!(plan_dialog(&modal));
        assert!(!input_ready(&modal));
        let composer = screen("Implement this plan?\n› 1. Yes, implement this plan\n  3. No, stay in Plan mode\n› Continue planning", Some((2, 3)));
        assert!(!plan_dialog(&composer));
        assert!(input_ready(&composer));
        assert!(!plan_dialog(&screen("The answer is: Implement this plan?", None)));
    }
}

#[cfg(test)]
mod policy_tests {
    use super::*;
    use mesimon_core::board::{ClaudeMode, CodexApproval, CodexSandbox, ColumnSettings};

    #[test]
    fn inherited_codex_policy_emits_nothing_even_with_claude_plan_mode() {
        let column = ColumnSettings { claude_mode: ClaudeMode::Plan, ..Default::default() };
        assert!(policy_flags(&column).is_empty());
    }

    #[test]
    fn explicit_policies_use_native_config_and_preserve_independence() {
        let mut column = ColumnSettings {
            codex_sandbox: CodexSandbox::ReadOnly,
            codex_approval: CodexApproval::OnRequest,
            ..Default::default()
        };
        assert_eq!(
            policy_flags(&column),
            ["-c", "sandbox_mode=\"read-only\"", "-c", "approval_policy=\"on-request\""]
        );
        column.codex_sandbox = CodexSandbox::WorkspaceWrite;
        column.codex_approval = CodexApproval::Never;
        assert_eq!(
            policy_flags(&column),
            ["-c", "sandbox_mode=\"workspace-write\"", "-c", "approval_policy=\"never\""]
        );
        column.codex_sandbox = CodexSandbox::Inherit;
        assert_eq!(policy_flags(&column), ["-c", "approval_policy=\"never\""]);
    }

    /// T-443: a Codex tier is two config keys; a Claude tier, and a
    /// built-in, are none.
    #[test]
    fn a_codex_tier_is_model_and_reasoning_effort_config() {
        use mesimon_core::board::AgentProvider;
        use mesimon_core::tier::{Effort, Tier};
        let reviewer = Tier {
            id: "A".into(),
            name: "reviewer".into(),
            provider: AgentProvider::Codex,
            model: "gpt-6-astra".into(),
            effort: Effort::High,
            description: String::new(),
        };
        assert_eq!(
            tier_flags(&reviewer),
            ["-c", "model=\"gpt-6-astra\"", "-c", "model_reasoning_effort=\"high\""]
        );
        assert!(tier_flags(&Tier::builtin(AgentProvider::Codex)).is_empty());
        let claude = Tier { provider: AgentProvider::ClaudeCode, ..reviewer };
        assert!(tier_flags(&claude).is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(sequence: u64) -> Snapshot {
        Snapshot {
            session: uuid::Uuid::nil(),
            generation: 1,
            sequence,
            heartbeat_ms: 1_000 * sequence,
            thread_id: None,
            launch_phase: LaunchPhase::default(),
            turn_id: None,
            state: SessionState::Idle { stop_reason: mesimon_core::board::StopReason::Unknown },
            observation_hold: false,
            history_path: None,
            title: None,
            plan: None,
            plan_key: None,
            request: None,
            stopped: false,
            rate_limits: None,
            rate_limits_at_ms: 0,
            hold: None,
            releases: 0,
            last_release: None,
            steers: 0,
            collaboration_mode: None,
        }
    }

    /// T-688: an unchanged file is reported from the last parse, a rewrite
    /// is read again, and a file that went is `None`.
    #[test]
    fn observer_reparses_only_a_moved_snapshot() {
        let dir = std::env::temp_dir().join(format!("msmn-cdx-observer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cdx.json");
        let id = uuid::Uuid::new_v4();
        write_json(&path, &snapshot(1)).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let ask = spawn_observer(move |s| tx.send(s).is_ok());
        let round = || {
            ask.send(vec![(id, path.clone())]).unwrap();
            rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap().remove(0).1
        };
        assert_eq!(round().map(|s| s.sequence), Some(1));
        // Same bytes, same stamp: the parse is kept.
        assert_eq!(round().map(|s| s.sequence), Some(1));
        // A rewrite of the same length moves the inode: read again.
        write_json(&path, &snapshot(2)).unwrap();
        assert_eq!(round().map(|s| s.sequence), Some(2));
        std::fs::remove_file(&path).unwrap();
        assert_eq!(round(), None);
        write_json(&path, &snapshot(3)).unwrap();
        assert_eq!(round().map(|s| s.sequence), Some(3));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
