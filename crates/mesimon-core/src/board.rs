use serde::{Deserialize, Serialize};

/// Session kinds v0.1 actually spawns. Notes are files, not sessions (D12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    Claude,
    Bash,
}

/// The full D15 state enum; the vocabulary is owned by 11 §11.7.1.
/// `Sleeping` has no producer until the sleep/reclaim milestone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum SessionState {
    Spawning,
    Running,
    RequiresAction {
        reason: Reason,
    },
    Idle {
        stop_reason: StopReason,
    },
    Sleeping,
    Exited {
        reason: ExitReason,
    },
    Failed {
        reason: FailReason,
    },
    Throttled,
    Unknown {
        #[serde(default)]
        reason: UnknownReason,
    },
}

impl SessionState {
    /// The record belongs to the ticket's working set — a `Sleeping` session is
    /// live-but-parked (it can be woken), only `Exited` is out.
    pub fn is_live(&self) -> bool {
        !matches!(self, SessionState::Exited { .. })
    }

    /// A pane exists (or should) for this record. `Sleeping` records have no
    /// process and no pane; every pane operation and pane-derived count keys
    /// off this, not `is_live`.
    pub fn has_pane(&self) -> bool {
        !matches!(self, SessionState::Exited { .. } | SessionState::Sleeping)
    }

    pub fn unknown() -> Self {
        SessionState::Unknown { reason: UnknownReason::NoSignal }
    }
}

/// Why a session needs a human. Rank order lives in `attention::rank`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    Permission,
    Secret,
    Question,
    Plan,
    Elicitation,
    Auth,
    QuotaResume,
    Trust,
    StartupModal,
    /// The resume-from-summary blocking dialog (09 §9): the session was
    /// resumed but is not yet accepting input. Shares rank 8 with
    /// `StartupModal` — an amendment to 11 §11.7.1, recorded in STALE-MAP.
    ResumeDialog,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    Interrupted,
    Unknown,
}

/// The reason, never the code (11 §11.7.1). `Killed` and `Dismissed` are
/// deliberate non-normative extras: `Killed` means mesimon's own kill ladder
/// ended a live session (the conversation survives — its corpse stays
/// resumable), `Dismissed` means the user x-ed an already-dead corpse off
/// the ticket rail (the only exit the rail hides).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitReason {
    UserQuit,
    Cleared,
    Resumed,
    LoggedOut,
    Crashed,
    Killed,
    Dismissed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailReason {
    Server,
    InvalidRequest,
    ModelNotFound,
    MaxOutputTokens,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownReason {
    SupervisorDead,
    AdapterGone,
    DaemonRestarted,
    #[default]
    NoSignal,
}

/// How trustworthy the source of the current state is (11 §11.5.4).
/// `Low`/`Stale` never get the saturated colour and never enter the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    #[default]
    High,
    Medium,
    Low,
    Stale,
}

/// Where a session record came from. Not named `origin` — that is D32c's word
/// for ticket/text provenance and carries trust semantics this does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    /// mesimon minted the UUID and spawned the process (`--session-id`).
    #[default]
    Spawned,
    /// A foreign session attached from the External drawer (19 §4). Badge
    /// word is "external"; hooks exist only after takeover-by-resume.
    Adopted,
}

/// A session record the daemon persists. The UUID is minted by mesimon and
/// passed to the agent (`--session-id`) and to tmux (session name = sid16),
/// so identity is never discovered (D24).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: uuid::Uuid,
    pub kind: SessionKind,
    pub ticket: ulid::Ulid,
    /// Full argv, persisted and replayed on resume (D24) — with the identity
    /// flag (`--session-id`/`--resume`) rewritten to the current conversation.
    pub argv: Vec<String>,
    pub cwd: String,
    pub state: SessionState,
    /// Epoch ms when the session entered the attention set; None otherwise.
    /// Minted by the daemon only — orders the Tab queue.
    #[serde(default)]
    pub waiting_since: Option<u64>,
    #[serde(default)]
    pub state_changed_at: Option<u64>,
    /// From `SessionStart` — the only authoritative source (D24).
    #[serde(default)]
    pub transcript_path: Option<String>,
    /// Short excerpt for the card (e.g. the rendered API error string). ≤200 chars.
    #[serde(default)]
    pub detail: Option<String>,
    /// The session's self-declared name (OSC-0 pane title — Claude keeps its
    /// conversation summary there). None until the agent sets one; latched so
    /// a parked/paneless session keeps the last name it had.
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub confidence: Confidence,
    #[serde(default)]
    pub provenance: Provenance,
    /// The claude-side session id when it differs from `id` — set for adopted
    /// sessions, and relearned from the SessionStart transcript filename when
    /// an in-app /resume hands the pane a different conversation. Stays None
    /// while the pane hosts the record's own minted conversation
    /// (mesimon-spawned ones pass `--session-id id`, so the two coincide).
    #[serde(default)]
    pub claude_session_id: Option<uuid::Uuid>,
    /// Manual override: never sleep this session (D23 guard, third part).
    #[serde(default)]
    pub pinned_awake: bool,
    /// The ticket title was typed into this session's box and is still
    /// waiting for its Enter. Spike T-5 arm C (2026-08-31): an Enter sent in
    /// the same breath as the text is swallowed by Claude's paste detection,
    /// so the submit is deferred to the `SessionStart` frame — the earliest
    /// point the pane provably accepts a keystroke as a keystroke. Cleared
    /// the moment it is delivered; a session that never starts just keeps the
    /// prefill, which is the ordinary spawn's behaviour anyway.
    #[serde(default)]
    pub pending_submit: bool,
}

impl SessionRecord {
    pub fn new(
        id: uuid::Uuid,
        kind: SessionKind,
        ticket: ulid::Ulid,
        argv: Vec<String>,
        cwd: String,
        state: SessionState,
    ) -> Self {
        Self {
            id,
            kind,
            ticket,
            argv,
            cwd,
            state,
            waiting_since: None,
            state_changed_at: None,
            transcript_path: None,
            detail: None,
            title: None,
            confidence: Confidence::default(),
            provenance: Provenance::default(),
            claude_session_id: None,
            pinned_awake: false,
            pending_submit: false,
        }
    }

    /// The tmux session name for this record: first 16 hex chars of the UUID.
    pub fn sid16(&self) -> String {
        self.id.simple().to_string()[..16].to_string()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    /// Fractional index; columns sort by it.
    pub order: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ticket {
    /// Immutable identity; never appears in a path (D24).
    pub id: ulid::Ulid,
    /// Human-speakable display key, stable across moves, never encodes state (D24).
    pub short_key: String,
    pub title: String,
    pub column: String,
    /// Fractional index within the column.
    pub order: String,
    pub created_at: String,
    /// Per-ticket workspace strategy (M4 layering: the ticket field is the truth;
    /// a column policy only defaults NEW tickets, M5). `None` = inherit the board
    /// default. Must stay after the scalar fields (TOML serialize order).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceStrategy>,
    /// Tags, at most one per group (a group is an axis: kind, environment…).
    /// Must stay after every scalar — this serializes as `[[tags]]`, an array
    /// of tables, and a scalar after a table errors. Tables may follow tables,
    /// so it sits between `workspace` (a scalar) and `[archived]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<TagRef>,
    /// Archival is a field, not a directory move (13 §data-model) — the ticket
    /// keeps its column and order, so restore is exact. Must stay last: a TOML
    /// table; any scalar serialized after it errors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived: Option<Archived>,
}

/// What a TICKET wears: a pointer into the registry by (group, name).
///
/// Deliberately no colour here. Colour lives on the registry entry, so
/// recolouring a tag repaints every card at once instead of leaving 40
/// tickets holding a stale copy.
///
/// `group` is a plain `u8`, never an enum: an unknown enum variant from a
/// newer daemon fails the WHOLE `Response::Board` deserialize and the client
/// drops the line (the same reasoning that makes `Notice.kind` and
/// `WorktreeItem.status` `String`s).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagRef {
    pub name: String,
    /// The axis this tag belongs to — the digit that reaches it. 1–9, 0 = 10.
    pub group: u8,
}

/// A REGISTRY entry: the vocabulary, and what each name looks like.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag {
    pub name: String,
    pub group: u8,
    /// Index into the tag tint ramp, chosen by the user (Tab cycles it).
    /// `None` means "never chosen" and falls back to a hash of the name, so a
    /// tag has a stable colour from the moment it exists without anyone
    /// having to pick one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<u8>,
}

/// How many tints the tag ramp offers. Mirrors `mesimon-tui`'s `theme::PIPS`;
/// core cannot see the theme, and the colour index is stored here, so the
/// modulus has to live on both sides. `tag_tints_agree` pins them together.
pub const TAG_TINTS: u8 = 6;

/// Most tags one axis may hold. A group is a small, readable set — past this
/// it is not an axis any more, it is a list, and the picker row stops fitting.
pub const MAX_TAGS_PER_GROUP: usize = 5;

/// The colour a name falls back to when nobody has picked one: stable across
/// machines and screenshots, never the order it was created in.
pub fn default_tint(name: &str) -> u8 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in name.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    (h % TAG_TINTS as u64) as u8
}

impl Tag {
    /// The tint to paint this tag with.
    pub fn tint(&self) -> u8 {
        self.color.unwrap_or_else(|| default_tint(&self.name)) % TAG_TINTS
    }
}

/// The longest a tag name may be, in bytes (13 §data-model). Enforced at the
/// daemon boundary by `sanitize_tag`, not here.
pub const TAG_MAX_BYTES: usize = 24;

/// Strip what a card row must never carry, then bound the length.
///
/// A tag name is user text rendered on a card row, so it runs the same
/// gauntlet as transcript peek text (`tui/src/peek.rs::sanitize`): control
/// chars, the drawn-structure range 0x2500–0x259F that the L1 law bans board-
/// wide, and the invisible width hazards — VS15/VS16 (U+FE0F turns a narrow
/// symbol into a two-cell emoji that `unicode-width` still counts as one),
/// ZWJ and the other zero-width format chars, and the combining keycap. A
/// terminal-vs-unicode-width disagreement on a card row shifts every later
/// cell one column right and strands a `selected_bg` cell past the card edge
/// that the diff never repaints.
///
/// Returns `None` for a name that is empty once sanitized — there is no such
/// thing as a blank tag.
pub fn sanitize_tag(raw: &str) -> Option<String> {
    let mut out = String::new();
    for ch in raw.chars() {
        let cp = ch as u32;
        let drop = ch.is_control()
            || (0x2500..=0x259F).contains(&cp)
            || matches!(cp, 0xFE0E | 0xFE0F | 0x200B..=0x200F | 0x2060..=0x206F | 0x20E3);
        if drop {
            continue;
        }
        if out.len() + ch.len_utf8() > TAG_MAX_BYTES {
            break;
        }
        out.push(ch);
    }
    let trimmed = out.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// The `[archived]` table on a ticket. Presence = off the board.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Archived {
    /// Same clock as `created_at` (`@<unix secs>`).
    pub at: String,
    /// Actor. v0.1 has no user@host plumbing — always "local" (STALE-MAP).
    #[serde(default)]
    pub by: String,
}

/// M4 (supersedes D25's column-only enum): how a ticket's sessions get a cwd.
/// The corpus's fourth value `none` is folded into "worktree, not yet provisioned" —
/// provisioning is lazy (first spawn), so an idea-card has no git identity anyway.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceStrategy {
    /// Own worktree at the state-dir root, branch `msmn/<KEY>-<slug>`.
    Worktree,
    /// cwd = the main checkout (pre-M4 behavior).
    SharedCheckout,
    /// Bound to a branch/worktree mesimon did not create; teardown keeps everything.
    AdoptExisting,
}

/// Layer-0 board default until column policies land (M5).
pub const DEFAULT_WORKSPACE: WorkspaceStrategy = WorkspaceStrategy::SharedCheckout;

impl Ticket {
    /// Layered resolution: ticket field, else the board default (column default is M5).
    pub fn workspace_strategy(&self) -> WorkspaceStrategy {
        self.workspace.unwrap_or(DEFAULT_WORKSPACE)
    }

    pub fn is_archived(&self) -> bool {
        self.archived.is_some()
    }

    /// This ticket's tag on axis `group`, if it wears one. At most one per
    /// group by construction — `set_tag` replaces rather than appends.
    pub fn tag_in(&self, group: u8) -> Option<&TagRef> {
        self.tags.iter().find(|t| t.group == group)
    }

    pub fn wears(&self, group: u8, name: &str) -> bool {
        self.tag_in(group).is_some_and(|t| t.name == name)
    }

    /// Set (or with `None`, clear) this ticket's tag on axis `group`.
    pub fn set_tag(&mut self, group: u8, name: Option<String>) {
        self.tags.retain(|t| t.group != group);
        if let Some(name) = name {
            self.tags.push(TagRef { name, group });
        }
        // Stable on disk and on the wire: a card's pips must not reorder
        // because an unrelated group changed.
        self.tags.sort_by_key(|t| t.group);
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Board {
    pub columns: Vec<Column>,
    pub tickets: Vec<Ticket>,
    pub sessions: Vec<SessionRecord>,
    /// Counter feeding short keys (T-1, T-2, …).
    pub next_key: u64,
    /// The tag registry: the vocabulary each axis offers, in the order it was
    /// created. Board-level and PERSISTED — a tag outlives the tickets that
    /// wear it, so untagging the last ticket does not silently retire the tag
    /// and a cycle keeps its shape.
    ///
    /// Nothing is seeded: the registry starts empty and grows the first time
    /// a name is typed. "Create on the fly" is about not having to set the
    /// board up before using it, NOT about deriving the list from the tickets.
    #[serde(default)]
    pub tags: Vec<Tag>,
}

/// D33i: the shipped default template.
pub const DEFAULT_COLUMNS: [&str; 4] = ["TODO", "IN PROGRESS", "REVIEW", "DONE"];

impl Board {
    pub fn with_default_columns() -> Self {
        let mut b = Board::default();
        let mut prev = String::new();
        for name in DEFAULT_COLUMNS {
            let order = crate::fracindex::between(&prev, "");
            b.columns.push(Column { name: name.into(), order: order.clone() });
            prev = order;
        }
        b
    }

    pub fn ticket(&self, id: ulid::Ulid) -> Option<&Ticket> {
        self.tickets.iter().find(|t| t.id == id)
    }

    pub fn ticket_mut(&mut self, id: ulid::Ulid) -> Option<&mut Ticket> {
        self.tickets.iter_mut().find(|t| t.id == id)
    }

    /// The vocabulary of axis `group`, in registry order — which is creation
    /// order, i.e. D31b's "stable order, config order, never by recency".
    ///
    /// Reading this off the registry rather than off the tickets is what
    /// makes the cycle well-behaved. An earlier cut derived it from whatever
    /// the tickets happened to wear, and that is a trap twice over: a tag
    /// vanished the moment its last wearer dropped it, and — worse — the
    /// ticket being cycled was itself part of the derivation, so taking a tag
    /// moved it in the list and the next press walked back to where it
    /// started. The cycle oscillated between two values and `none` was
    /// unreachable. A registry is independent of every ticket, so neither can
    /// happen.
    pub fn group_tags(&self, group: u8) -> Vec<&str> {
        self.tags.iter().filter(|t| t.group == group).map(|t| t.name.as_str()).collect()
    }

    /// The registry entries of axis `group`, in creation order.
    pub fn group_entries(&self, group: u8) -> Vec<&Tag> {
        self.tags.iter().filter(|t| t.group == group).collect()
    }

    /// Look a name up in the registry.
    pub fn tag_def(&self, group: u8, name: &str) -> Option<&Tag> {
        self.tags.iter().find(|t| t.group == group && t.name == name)
    }

    /// The tint a ticket's tag should be painted with. Falls back to the
    /// name's own hash for a reference whose registry entry has gone missing,
    /// so a card can never render a colourless band.
    pub fn tint_of(&self, t: &TagRef) -> u8 {
        self.tag_def(t.group, &t.name).map(|d| d.tint()).unwrap_or_else(|| default_tint(&t.name))
    }

    /// Add a name to axis `group`. `Err` says why not, so the caller can show
    /// it rather than failing silently.
    pub fn register_tag(&mut self, group: u8, name: &str) -> Result<(), String> {
        if self.tags.iter().any(|t| t.group == group && t.name == name) {
            return Err(format!("{name} is already in group {group}"));
        }
        if self.group_entries(group).len() >= MAX_TAGS_PER_GROUP {
            return Err(format!("group {group} is full ({MAX_TAGS_PER_GROUP} tags)"));
        }
        self.tags.push(Tag { name: name.to_string(), group, color: None });
        Ok(())
    }

    /// Rename a tag, carrying every ticket wearing it along. Returns the ids
    /// that changed so the caller writes only those files.
    pub fn rename_tag(
        &mut self,
        group: u8,
        from: &str,
        to: &str,
    ) -> Result<Vec<ulid::Ulid>, String> {
        if self.tag_def(group, from).is_none() {
            return Err(format!("no tag {from:?} in group {group}"));
        }
        if from != to && self.tag_def(group, to).is_some() {
            return Err(format!("{to} is already in group {group}"));
        }
        for t in self.tags.iter_mut() {
            if t.group == group && t.name == from {
                t.name = to.to_string();
            }
        }
        let mut touched = Vec::new();
        for t in self.tickets.iter_mut() {
            let mut hit = false;
            for r in t.tags.iter_mut() {
                if r.group == group && r.name == from {
                    r.name = to.to_string();
                    hit = true;
                }
            }
            if hit {
                touched.push(t.id);
            }
        }
        Ok(touched)
    }

    /// Set a tag's tint. `None` on the entry means "unchosen"; this always
    /// writes an explicit choice.
    pub fn set_tag_color(&mut self, group: u8, name: &str, color: u8) -> Result<(), String> {
        match self.tags.iter_mut().find(|t| t.group == group && t.name == name) {
            Some(t) => {
                t.color = Some(color % TAG_TINTS);
                Ok(())
            }
            None => Err(format!("no tag {name:?} in group {group}")),
        }
    }

    /// Remove a name from the registry AND from every ticket wearing it.
    /// Returns whether anything changed.
    ///
    /// Leaving a ticket wearing a retired tag would put a pip on the board
    /// that the cycle can never reach or clear, so the two must move together.
    pub fn forget_tag(&mut self, group: u8, name: &str) -> bool {
        let before = self.tags.len();
        self.tags.retain(|t| !(t.group == group && t.name == name));
        let mut changed = self.tags.len() != before;
        for t in &mut self.tickets {
            let n = t.tags.len();
            t.tags.retain(|tag| !(tag.group == group && tag.name == name));
            changed |= t.tags.len() != n;
        }
        changed
    }

    /// Tickets of one column, sorted by fractional order (ties by id for stability).
    /// Archived tickets stay in `tickets` (the ticket page needs them in the
    /// snapshot) but never surface here — this is the board's one chokepoint.
    pub fn column_tickets(&self, column: &str) -> Vec<&Ticket> {
        let mut v: Vec<&Ticket> =
            self.tickets.iter().filter(|t| t.column == column && !t.is_archived()).collect();
        v.sort_by(|a, b| a.order.cmp(&b.order).then(a.id.cmp(&b.id)));
        v
    }

    /// Archived tickets, newest archive first (`at` stamps sort as strings).
    pub fn archived_tickets(&self) -> Vec<&Ticket> {
        let mut v: Vec<&Ticket> = self.tickets.iter().filter(|t| t.is_archived()).collect();
        v.sort_by(|a, b| {
            let ka = a.archived.as_ref().map(|x| x.at.as_str()).unwrap_or("");
            let kb = b.archived.as_ref().map(|x| x.at.as_str()).unwrap_or("");
            kb.cmp(ka).then(a.id.cmp(&b.id))
        });
        v
    }

    /// Sessions of the ticket that hold (or should hold) a pane. The archive
    /// gate, its TUI advisory, and the header suggestion all share this — the
    /// suggestion never offers what the keystroke would refuse.
    pub fn ticket_awake_sessions(&self, id: ulid::Ulid) -> usize {
        self.sessions.iter().filter(|s| s.ticket == id && s.state.has_pane()).count()
    }

    pub fn sorted_columns(&self) -> Vec<&Column> {
        let mut v: Vec<&Column> = self.columns.iter().collect();
        v.sort_by(|a, b| a.order.cmp(&b.order));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An M1 sessions.json line must keep parsing after the M2 enum growth
    /// (store::load hard-fails on parse error — defaults are the migration).
    #[test]
    fn m1_session_record_parses() {
        let m1 = r#"{
            "id": "3f2b8c1e-9a4d-4e6f-8b1a-2c3d4e5f6a7b",
            "kind": "claude",
            "ticket": "01J8ZQ7VJ00000000000000000",
            "argv": ["claude"],
            "cwd": "/tmp",
            "state": { "state": "unknown" }
        }"#;
        let rec: SessionRecord = serde_json::from_str(m1).unwrap();
        assert_eq!(rec.state, SessionState::unknown());
        assert_eq!(rec.confidence, Confidence::High);
        assert!(rec.waiting_since.is_none());

        let m1_exited = r#"{
            "id": "3f2b8c1e-9a4d-4e6f-8b1a-2c3d4e5f6a7b",
            "kind": "bash",
            "ticket": "01J8ZQ7VJ00000000000000000",
            "argv": ["/bin/zsh"],
            "cwd": "/tmp",
            "state": { "state": "exited", "reason": "killed" }
        }"#;
        let rec: SessionRecord = serde_json::from_str(m1_exited).unwrap();
        assert_eq!(rec.state, SessionState::Exited { reason: ExitReason::Killed });
    }

    /// An M2 sessions.json line (pre-provenance) must keep parsing after the
    /// M3 field growth — defaults are the migration.
    #[test]
    fn m2_session_record_parses() {
        let m2 = r#"{
            "id": "3f2b8c1e-9a4d-4e6f-8b1a-2c3d4e5f6a7b",
            "kind": "claude",
            "ticket": "01J8ZQ7VJ00000000000000000",
            "argv": ["claude", "--settings", "/x.json", "--session-id", "3f2b8c1e-9a4d-4e6f-8b1a-2c3d4e5f6a7b"],
            "cwd": "/tmp",
            "state": { "state": "requires_action", "reason": "permission" },
            "waiting_since": 1724900000000,
            "state_changed_at": 1724900000000,
            "transcript_path": "/tmp/t.jsonl",
            "detail": "Bash",
            "confidence": "high"
        }"#;
        let rec: SessionRecord = serde_json::from_str(m2).unwrap();
        assert_eq!(rec.provenance, Provenance::Spawned);
        assert!(rec.claude_session_id.is_none());
        assert!(!rec.pinned_awake);
    }

    fn ticket(id: u128, column: &str, order: &str) -> Ticket {
        Ticket {
            id: ulid::Ulid(id),
            short_key: format!("T-{id}"),
            title: "t".into(),
            column: column.into(),
            order: order.into(),
            created_at: "@0".into(),
            workspace: None,
            tags: Vec::new(),
            archived: None,
        }
    }

    /// The registry is board-level and persisted: a group's vocabulary is
    /// what has been registered there, in creation order, independent of what
    /// any ticket currently wears. Nothing is seeded — a name enters by being
    /// used once — and nothing leaves except by an explicit retire.
    #[test]
    fn the_registry_outlives_its_wearers_and_holds_its_order() {
        let mut b = Board::with_default_columns();
        let mut id = 0u128;
        let mut mk = |tags: &[(u8, &str)]| {
            id += 1;
            let mut t = ticket(id, "TODO", "a");
            for (g, name) in tags {
                t.set_tag(*g, Some((*name).to_string()));
                let _ = b.register_tag(*g, name);
            }
            b.tickets.push(t);
        };
        mk(&[(1, "BUG"), (2, "DEV")]);
        mk(&[(1, "REGR")]);
        mk(&[(1, "BUG"), (2, "PRODUCTION")]);

        // Registry order is creation order (D31b's "config order").
        assert_eq!(b.group_tags(1), vec!["BUG", "REGR"]);
        assert_eq!(b.group_tags(2), vec!["DEV", "PRODUCTION"]);
        assert_eq!(b.group_tags(3), Vec::<&str>::new());
        assert!(b.register_tag(1, "BUG").is_err(), "registering twice is refused");

        // The vocabulary is independent of what the tickets wear. Derive it
        // from them instead and it breaks twice: a tag vanishes when its last
        // wearer drops it, and the ticket being cycled reorders its own list,
        // so the digit walks backwards and `none` becomes unreachable.
        let owned = |b: &Board| -> Vec<String> {
            b.group_tags(1).iter().map(|s| (*s).to_string()).collect()
        };
        let before = owned(&b);
        b.tickets[0].set_tag(1, Some("REGR".into()));
        assert_eq!(owned(&b), before, "wearing a tag must not reorder the cycle");
        b.tickets[0].set_tag(1, Some("BUG".into()));
        assert_eq!(owned(&b), before);

        // A tag OUTLIVES its wearers. This is the point of a registry: strip
        // it off every ticket and it is still in the cycle tomorrow.
        for t in b.tickets.iter_mut() {
            t.set_tag(1, None);
        }
        assert_eq!(owned(&b), before, "an unworn tag stays in the vocabulary");

        // Retiring is explicit, and takes the pips with it.
        b.tickets[0].set_tag(1, Some("BUG".into()));
        assert!(b.forget_tag(1, "BUG"));
        assert_eq!(b.group_tags(1), vec!["REGR"]);
        assert!(b.tickets[0].tag_in(1).is_none(), "a retired tag leaves no orphan pip");
        assert!(!b.forget_tag(1, "BUG"), "forgetting twice is a no-op");

        // Archiving must not renumber a cycle either.
        b.tickets[0].archived = Some(Archived { at: "@1".into(), by: "local".into() });
        assert_eq!(b.group_tags(1), vec!["REGR"]);
    }

    /// A group is a small readable set, not a list: past the cap the picker
    /// row stops fitting and the axis stops being an axis.
    #[test]
    fn a_group_fills_up() {
        let mut b = Board::with_default_columns();
        for i in 0..MAX_TAGS_PER_GROUP {
            assert!(b.register_tag(1, &format!("t{i}")).is_ok(), "{i}");
        }
        let err = b.register_tag(1, "one-too-many").expect_err("cap enforced");
        assert!(err.contains("full"), "{err}");
        // Other axes are unaffected, and a freed slot can be refilled.
        assert!(b.register_tag(2, "elsewhere").is_ok());
        assert!(b.forget_tag(1, "t0"));
        assert!(b.register_tag(1, "one-too-many").is_ok());
    }

    /// Colour is a registry property, so recolouring repaints every card at
    /// once. Unchosen falls back to a hash of the name — stable across
    /// machines, so a tag has a colour from the moment it exists.
    #[test]
    fn colour_lives_on_the_registry() {
        let mut b = Board::with_default_columns();
        b.register_tag(1, "BUG").expect("registered");
        let def = b.tag_def(1, "BUG").expect("in registry");
        assert_eq!(def.color, None);
        assert_eq!(def.tint(), default_tint("BUG"));
        assert!(def.tint() < TAG_TINTS);

        b.set_tag_color(1, "BUG", 3).expect("recoloured");
        assert_eq!(b.tag_def(1, "BUG").expect("still there").tint(), 3);
        // Out of range wraps rather than being refused.
        b.set_tag_color(1, "BUG", TAG_TINTS + 1).expect("wrapped");
        assert_eq!(b.tag_def(1, "BUG").expect("still there").tint(), 1);
        assert!(b.set_tag_color(1, "NOPE", 0).is_err());
    }

    /// A rename carries every wearer along, and reports which files changed.
    #[test]
    fn renaming_carries_the_wearers() {
        let mut b = Board::with_default_columns();
        b.register_tag(1, "BUG").expect("registered");
        let mut t = ticket(1, "TODO", "a");
        t.set_tag(1, Some("BUG".into()));
        b.tickets.push(t);
        b.tickets.push(ticket(2, "TODO", "b"));

        let touched = b.rename_tag(1, "BUG", "DEFECT").expect("renamed");
        assert_eq!(touched, vec![ulid::Ulid(1)], "only the wearer's file changed");
        assert_eq!(b.group_tags(1), vec!["DEFECT"]);
        assert!(b.tickets[0].wears(1, "DEFECT"));
        assert!(b.tickets[1].tag_in(1).is_none());
        // A name already in the group, and a name that is not there at all.
        b.register_tag(1, "OTHER").expect("registered");
        assert!(b.rename_tag(1, "DEFECT", "OTHER").is_err());
        assert!(b.rename_tag(1, "GHOST", "X").is_err());
    }

    /// One tag per group: setting replaces, `None` clears, and the set stays
    /// sorted so a card's pips never reorder because another axis changed.
    #[test]
    fn set_tag_replaces_within_a_group() {
        let mut t = ticket(1, "TODO", "a");
        t.set_tag(2, Some("DEV".into()));
        t.set_tag(1, Some("BUG".into()));
        assert_eq!(t.tags.iter().map(|t| t.group).collect::<Vec<_>>(), vec![1, 2]);

        t.set_tag(1, Some("REGR".into()));
        assert_eq!(t.tags.len(), 2);
        assert_eq!(t.tag_in(1).map(|t| t.name.as_str()), Some("REGR"));

        t.set_tag(1, None);
        assert_eq!(t.tag_in(1), None);
        assert_eq!(t.tag_in(2).map(|t| t.name.as_str()), Some("DEV"));
    }

    /// A tag name is user text on a card row, so it runs the same width
    /// gauntlet as peek text: a terminal-vs-unicode-width disagreement there
    /// strands a `selected_bg` cell past the card edge that the diff never
    /// repaints.
    #[test]
    fn sanitize_tag_strips_the_width_hazards() {
        assert_eq!(sanitize_tag("BUG"), Some("BUG".into()));
        assert_eq!(sanitize_tag("  spaced  "), Some("spaced".into()));
        // Nothing blank is a tag.
        assert_eq!(sanitize_tag(""), None);
        assert_eq!(sanitize_tag("   "), None);
        assert_eq!(sanitize_tag("\u{200b}"), None);
        // Control chars and the drawn-structure range the L1 law bans.
        assert_eq!(sanitize_tag("a\nb"), Some("ab".into()));
        assert_eq!(sanitize_tag("a\u{2500}b"), Some("ab".into()));
        assert_eq!(sanitize_tag("a\u{2588}b"), Some("ab".into()));
        // VS16 turns a narrow symbol into a two-cell emoji that
        // `unicode-width` still counts as one.
        assert_eq!(sanitize_tag("x\u{fe0f}"), Some("x".into()));
        assert_eq!(sanitize_tag("a\u{200d}b"), Some("ab".into()));
        // Bounded, and never split a char.
        let long = sanitize_tag(&"é".repeat(40)).expect("non-empty");
        assert!(long.len() <= TAG_MAX_BYTES, "{} bytes", long.len());
        assert!(long.chars().all(|c| c == 'é'));
    }

    /// Archived tickets stay in `tickets` but never surface on the board.
    #[test]
    fn column_tickets_excludes_archived() {
        let mut b = Board::with_default_columns();
        b.tickets.push(ticket(1, "DONE", "a"));
        let mut t = ticket(2, "DONE", "b");
        t.archived = Some(Archived { at: "@10".into(), by: "local".into() });
        b.tickets.push(t);
        let done: Vec<_> = b.column_tickets("DONE").iter().map(|t| t.id.0).collect();
        assert_eq!(done, vec![1]);
        let arch: Vec<_> = b.archived_tickets().iter().map(|t| t.id.0).collect();
        assert_eq!(arch, vec![2]);
    }

    /// Newest archive first; `column` survives so restore is exact.
    #[test]
    fn archived_tickets_newest_first() {
        let mut b = Board::with_default_columns();
        for (id, at) in [(1u128, "@100"), (2, "@300"), (3, "@200")] {
            let mut t = ticket(id, "REVIEW", "a");
            t.archived = Some(Archived { at: at.into(), by: "local".into() });
            b.tickets.push(t);
        }
        let ids: Vec<_> = b.archived_tickets().iter().map(|t| t.id.0).collect();
        assert_eq!(ids, vec![2, 3, 1]);
        assert_eq!(b.archived_tickets()[0].column, "REVIEW");
    }

    #[test]
    fn state_roundtrips() {
        for s in [
            SessionState::Spawning,
            SessionState::RequiresAction { reason: Reason::Permission },
            SessionState::RequiresAction { reason: Reason::ResumeDialog },
            SessionState::Idle { stop_reason: StopReason::EndTurn },
            SessionState::Sleeping,
            SessionState::Failed { reason: FailReason::Server },
            SessionState::Throttled,
            SessionState::Unknown { reason: UnknownReason::SupervisorDead },
        ] {
            let json = serde_json::to_string(&s).unwrap();
            let back: SessionState = serde_json::from_str(&json).unwrap();
            assert_eq!(s, back);
        }
    }
}
