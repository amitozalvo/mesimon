//! The bounded owner-control surface. No paths, native argv, or local envelopes.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// An explicit one-shot human answer, never a policy or input rewrite.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    Allow,
    Deny,
}
impl PermissionDecision {
    pub fn hook_output(self) -> serde_json::Value {
        serde_json::json!({"hookSpecificOutput": {
            "hookEventName": "PermissionRequest", "decision": {"behavior": self}
        }})
    }
}

/// How long a paired phone may answer a permission dialog (T-632): as long
/// as the dialog stands, within a day. The hook set's `mesimon approve`
/// holds this long in one run; the daemon's wait ends here at the latest.
pub const PERMISSION_HOLD_SECS: u64 = 24 * 60 * 60;
/// One run of the mod's `mesimon approve`: a mod's process may live ten
/// minutes at most, so the mod runs it again and the daemon's wait passes
/// from one run to the next (`--renew`), the phone's card unchanged.
pub const PERMISSION_ROUND_SECS: u64 = 540;
/// `mesimon approve --renew` exits so when its round ran out with the
/// daemon still holding the dialog: the mod runs the next round. Any other
/// exit with no decision is the end of the hold.
pub const PERMISSION_RENEW_EXIT: i32 = 75;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Permission {
    pub request: String,
    pub tool: String,
    pub input: serde_json::Value,
    pub expires_at: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Info {
    pub enabled: bool,
    pub connected: bool,
    pub origin: String,
    pub code: Option<String>,
    pub error: Option<String>,
    pub devices: Vec<Device>,
    /// Whether the board keeps a sealed copy at the relay for its paired
    /// browsers to read while it is away (T-698). `None` from a daemon from
    /// before, which keeps none and offers no row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shelf: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Device {
    pub grant: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum LocalAction {
    Status,
    Enable,
    Disable,
    Pair,
    Revoke {
        grant: String,
    },
    /// Keep the sealed copy at the relay (T-698), or empty it and stop.
    Shelf {
        on: bool,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Snapshot,
    Foreground {
        ticket: Option<String>,
    },
    Dialog {
        ticket: String,
        session: String,
        request: String,
        response: DialogAnswer,
    },
    Permission {
        ticket: String,
        session: String,
        request: String,
        decision: PermissionDecision,
    },
    Preview {
        ticket: String,
        session: String,
    },
    /// A page of the agent's conversation, read from its transcript file
    /// (T-626): the newest page, the one ending at `before`, or with `after`
    /// and the `conversation` it belongs to, only what was written since.
    /// The pane's screen stays `Preview`, the page's raw view.
    Transcript {
        ticket: String,
        session: String,
        #[serde(default)]
        before: Option<u64>,
        #[serde(default)]
        after: Option<u64>,
        #[serde(default)]
        conversation: Option<String>,
        /// At most this many rows, and never more than `TRANSCRIPT_ROWS`.
        #[serde(default)]
        limit: Option<u16>,
    },
    Prompt {
        ticket: String,
        session: String,
        text: String,
        #[serde(default = "queue_by_default")]
        queued: bool,
        /// The composer's tier pick (T-643), the desk ask field's `^n`: a
        /// tier id from the board reply's `tiers`, applied to the ticket
        /// first. A pick the agent did not launch on relaunches it at its
        /// next idle with the words held, so the words queue whatever
        /// `queued` said. Sent only to a host that says `tiers`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tier: Option<String>,
    },
    SendNow {
        ticket: String,
        session: String,
    },
    TakeBack {
        ticket: String,
        session: String,
    },
    Status {
        command: u64,
    },
    /// File a ticket (T-497). It lands quietly: no agent starts, whatever
    /// the column says. The description becomes `notes[0]`; a tag must be
    /// one the board already has.
    Create {
        title: String,
        #[serde(default)]
        description: String,
        /// `None` lands it in the board's default column.
        #[serde(default)]
        column: Option<String>,
        #[serde(default)]
        tags: Vec<TagPick>,
        /// Pictures this browser uploaded for the description (T-670), each
        /// linked from it as a note links one, and kept only with the ticket.
        /// Sent only to a host that says `filed_pictures`: an older one
        /// refuses the field.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        uploads: Vec<String>,
    },
    /// Start an agent on a ticket that has none, or wake the one asleep
    /// on it (T-498, T-510): the board's Shift+Enter from a phone. The
    /// provider is the one the board's tiers give the ticket; `prompt` is
    /// the first turn's words, and blank it is the ticket's title and
    /// description on an empty seat, or a plain wake on a sleeping one.
    /// Answered `starting` (or `provisioning` while a worktree is cut),
    /// and the receipt turns `started` once the session runs.
    Start {
        ticket: String,
        #[serde(default)]
        prompt: Option<String>,
        /// The tier to start or wake it on (T-643), a tier id from the board
        /// reply's `tiers`: applied to the ticket first, as the desk's `^n`
        /// is before its Shift+Enter. Absent, the ticket's own. Sent only to
        /// a host that says `tiers`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tier: Option<String>,
    },
    /// Retitle a ticket (T-530): the board's rename from a phone. A blank
    /// title is refused; the host scrubs and caps it as it does the desk's.
    Rename {
        ticket: String,
        title: String,
    },
    /// Move a ticket to a column (T-530), before the ticket `before` names,
    /// or at the column's end without one. In its own column it is a
    /// reorder. The board's own gates apply: a column that needs the work
    /// merged refuses an unmerged ticket.
    Move {
        ticket: String,
        column: String,
        #[serde(default)]
        before: Option<String>,
    },
    /// Put one of the board's tags on a ticket, replacing the one it wore on
    /// that group, or with no `name` take the group's tag off (T-530). A tag
    /// the board does not have is refused: a phone never adds a word.
    Tag {
        ticket: String,
        group: u8,
        #[serde(default)]
        name: Option<String>,
    },
    /// Where the ticket's next agent works (T-642): its own worktree, or
    /// the shared checkout. The board's Shift+Tab from a phone, and refused
    /// as the desk refuses it once an agent runs or a worktree is cut.
    Workspace {
        ticket: String,
        worktree: bool,
    },
    /// A ticket's notes (T-532): every note's row, the description first,
    /// and the description's body, which the ticket page shows.
    Notes {
        ticket: String,
    },
    /// One note's whole body (T-532).
    Note {
        ticket: String,
        note: String,
    },
    /// Create (no `note`), replace, or with blank text delete one note
    /// (T-532), as the desk's note editor does. `rev` is the revision the
    /// browser opened: a note that moved on since is refused as stale.
    WriteNote {
        ticket: String,
        #[serde(default)]
        note: Option<String>,
        text: String,
        #[serde(default)]
        rev: Option<u64>,
        /// Pictures this browser uploaded for the note (T-629), each linked
        /// from `text` as the desk links one. They are kept only with it.
        /// Sent only to a host that says `pictures`: an older one refuses
        /// the field.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        uploads: Vec<String>,
    },
    /// One piece of a picture for a note on `ticket` (T-629), in order: no
    /// `upload` begins one at offset zero and the answer names it, and
    /// `complete` ends it, when the host checks it is a whole PNG. `data`
    /// is base64 of at most `PICTURE_CHUNK_BYTES`. An upload that waits ten
    /// minutes unsaved is let go. No `ticket` is a picture for the
    /// description of a ticket not filed yet (T-670), which only a host
    /// that says `filed_pictures` reads.
    Upload {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ticket: Option<String>,
        #[serde(default)]
        upload: Option<String>,
        offset: usize,
        data: String,
        #[serde(default)]
        complete: bool,
    },
    /// Point the ticket's awake agent at a note (T-532): the desk's second
    /// `^s`, mesimon's own sentence naming the note.
    TellAgent {
        ticket: String,
        note: String,
    },
}

/// A ticket a paired browser sealed for the host's mailbox (T-497): the
/// create op's fields and when it was written. Unknown fields are ignored,
/// not refused: a host may be older than the page that wrote the ticket.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MailTicket {
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub column: Option<String>,
    #[serde(default)]
    pub tags: Vec<TagPick>,
    /// When it was written, in the browser's clock; shown, never trusted.
    #[serde(default)]
    pub written_at: u64,
}

/// A note edit a paired browser sealed while its terminal was away (T-532):
/// `WriteNote`'s fields and when it was written. The letter says
/// `"kind": "note"`; a ticket's letter has no kind, as phase 3's page wrote it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MailNote {
    pub ticket: String,
    #[serde(default)]
    pub note: Option<String>,
    pub text: String,
    #[serde(default)]
    pub rev: Option<u64>,
    #[serde(default)]
    pub written_at: u64,
}

/// What a sealed letter carries.
#[derive(Clone, Debug)]
pub enum Letter {
    Ticket(MailTicket),
    Note(MailNote),
}

/// Read an opened letter: a note edit when it says so, else a ticket. An
/// older host reads a note's letter as a ticket with no title and refuses
/// it, so it never files one as the other.
pub fn read_letter(body: serde_json::Value) -> Option<Letter> {
    if body.get("kind").and_then(serde_json::Value::as_str) == Some("note") {
        serde_json::from_value(body).ok().map(Letter::Note)
    } else {
        serde_json::from_value(body).ok().map(Letter::Ticket)
    }
}

/// One note as the ticket page lists it (T-532).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteRow {
    pub id: String,
    /// The body's first line, as the desk names it.
    pub name: String,
    /// Who wrote it last, in the page's words: `you`, `agent`, a paired
    /// browser's name, or a teammate's.
    pub by: String,
    /// When, in milliseconds since the epoch, the host's clock.
    pub at: u64,
    pub rev: u64,
}

impl NoteRow {
    pub fn of(meta: &crate::board::NoteMeta, by: String) -> Self {
        Self {
            id: meta.id.to_string(),
            name: meta.name.clone(),
            by,
            at: crate::board::stamp_secs(&meta.edited_at).unwrap_or(0) * 1000,
            rev: meta.rev,
        }
    }
}

/// A short digest of a ticket's notes, ids and revisions in order (T-532):
/// a page asks for them again only when it changes. Empty for no notes.
pub fn notes_stamp(notes: &[crate::board::NoteMeta]) -> String {
    if notes.is_empty() {
        return String::new();
    }
    // FNV-1a: stable across builds and platforms, which a page comparing
    // two answers needs, and no secret is kept by it.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for n in notes {
        for byte in n.id.to_bytes().into_iter().chain(n.rev.to_le_bytes()) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{hash:016x}")
}

/// A tag a new ticket wears, spelled as the board's vocabulary spells it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TagPick {
    pub group: u8,
    pub name: String,
}

/// One agent tier a phone may pick (T-643), as the desk's `^n` offers it:
/// `provider` is `claude` or `codex`, the word `Agent::provider` spells, so
/// a seat that holds an agent is offered its own provider's tiers alone.
/// `summary` is the launch in words (`Claude Code ∙ opus ∙ high`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TierOption {
    pub id: String,
    pub name: String,
    pub provider: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
}

/// One tag of the board's vocabulary, as the New ticket sheet offers it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagOption {
    pub group: u8,
    pub name: String,
    /// Index into the tag tint ramp (`board::TAG_TINTS`), the TUI's colour.
    pub tint: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub incarnation: String,
    pub id: u64,
    pub request: Request,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Ticket {
    /// The words queued for the ticket's agent.
    #[serde(default)]
    pub queued: Option<String>,
    /// Whose those words are and what they wait on (T-568). Beside `queued`,
    /// never in its place: an older page reads `queued` as the words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue: Option<Queue>,
    pub id: String,
    pub key: String,
    pub title: String,
    pub column: String,
    pub agent: Option<Agent>,
    /// The tags the ticket wears, each with the TUI's tint (T-497).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<TagOption>,
    /// A ticket a paired browser filed, once it was picked up at the desk
    /// (T-497): the browser that filed it turns its ticks teal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picked: Option<Picked>,
    /// How many notes the ticket has, the description included (T-532).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub notes: u32,
    /// `notes_stamp` of them: the page asks for the notes again only when
    /// it changes.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub noted: String,
    /// The ticket wears the crown (T-623): its agent edits every ticket.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub crown: bool,
    /// The crown's latest edit of this ticket, within the hour (T-623).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crowned: Option<Crowned>,
    /// Where the ticket's code lives (T-642), when the TUI would say so or
    /// the choice is still open; absent for a shared checkout that is
    /// settled, and on a board with no repository on the host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<Workspace>,
    /// The tier id the ticket's next launch runs on (T-643): its pick, or
    /// the board's default. Absent from an older host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,
    /// When the ticket was created (T-668): milliseconds since the epoch,
    /// the host's clock. Now lists a ticket created within the hour.
    /// Absent from an older host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<u64>,
}

/// A ticket's workspace as a phone reads it (T-642): the TUI card's
/// worktree mark and the ticket page's branch row. Words, never enums, so
/// a newer host's word does not fail an older page. No path: the branch
/// names the worktree.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    /// `worktree`, `shared` or `adopt`: the ticket's choice.
    pub kind: String,
    /// The choice may still change: no agent stands in the directory and
    /// no worktree is cut (the desk's `set_workspace` lock).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub open: bool,
    /// The worktree's branch, once it has one.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub branch: String,
    /// The card's state word: `planned` (asked for, not cut yet),
    /// `provisioning`, `error`, `evicted`, `conflict`, `merged`, `behind`,
    /// `ahead` or `clean`. Empty for a shared checkout.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub state: String,
    /// Commits on the branch the default branch lacks.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub ahead: u32,
    /// A provision's progress (`7/19`, `init script running`) or how the
    /// init script failed; never a stage's error text, which may name a
    /// path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// A ticket's workspace for a phone (T-642), from the desk's own facts:
/// its strategy, whether `set_workspace` would take a change, and the
/// worktree row the TUI draws. `None` where the TUI's card and page say
/// nothing and nothing can be chosen.
pub fn workspace(
    strategy: crate::board::WorkspaceStrategy,
    open: bool,
    wt: Option<&crate::command::WorktreeItem>,
) -> Option<Workspace> {
    use crate::board::WorkspaceStrategy as S;
    let kind = match strategy {
        S::Worktree => "worktree",
        S::SharedCheckout => "shared",
        S::AdoptExisting => "adopt",
    };
    let Some(w) = wt else {
        let planned = strategy == S::Worktree;
        return (planned || open).then(|| Workspace {
            kind: kind.into(),
            open,
            state: if planned { "planned".into() } else { String::new() },
            ..Workspace::default()
        });
    };
    // The card's order (`card::worktree_mark`): what is wrong first.
    let state = match w.status.as_str() {
        "queued" | "provisioning" => "provisioning",
        "error" => "error",
        "evicted" => "evicted",
        _ if w.conflict => "conflict",
        _ if w.merged => "merged",
        _ if w.needs_rebase => "behind",
        _ if w.ahead > 0 => "ahead",
        _ => "clean",
    };
    Some(Workspace {
        kind: kind.into(),
        open,
        branch: crate::text::scrub_text(&w.branch),
        state: state.into(),
        ahead: w.ahead,
        detail: if state == "error" {
            None
        } else {
            w.detail.as_deref().map(crate::text::scrub_text)
        },
    })
}

/// What the crown last did to a ticket (T-623), as the TUI's card says it
/// for a beat: `action` is the card's word (`moved`, `started`, `woke`…),
/// never an enum, so a newer host's word does not fail an older page; `by`
/// is the key of the ticket whose agent did it, or for `woke` the worker
/// whose news woke the crown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Crowned {
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    /// Milliseconds since the epoch, the host's clock.
    pub at: u64,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// A queued ask's facts as a phone reads them (T-568), the board's own row
/// (`command::Pending`) in its fields: whose words they are, whether they
/// wait on a person's send, and whose turn or answer they wait on.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Queue {
    /// The key of the crown ticket whose agent queued the words (T-413);
    /// absent for a person's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    /// The crown's words go by the queue like a person's (T-550); a crown's
    /// that does not send waits on a person's send.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sends: bool,
    /// Why the board held the words for a person's send: `agent asked`
    /// when the agent stopped on a question (T-420, T-565).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held: Option<String>,
    /// The keys whose turn the words wait for, the ticket's own among them
    /// while its own agent works (`Pending::waits_on`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waits: Vec<String>,
    /// The keys among those whose agent is on a question, which a person
    /// answers; for held words, the ticket's own while its agent asks
    /// (`Pending::asking`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub asking: Vec<String>,
}

/// Whose queued words a phone's prompt took the place of (T-568): `by` is
/// the crown ticket's key for its agent's words, `held` for a person's
/// words the board held on a question, and `you` for a person's own. A
/// word, never an enum, so a newer host's word does not fail an older page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Replaced {
    pub by: String,
}

/// How a phone's ticket was picked up, and when: `by` is `desk` (its page was
/// opened in the TUI) or `agent` (an agent started on it). A word, never an
/// enum, so a newer host's word does not fail an older browser's board.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Picked {
    pub by: String,
    /// Milliseconds since the epoch, the host's clock.
    pub at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Agent {
    #[serde(default)]
    pub permission: Option<Permission>,
    #[serde(default)]
    pub dialog: Option<Dialog>,
    pub session: String,
    pub provider: String,
    pub state: String,
    pub promptable: bool,
    /// When the agent entered `state`: milliseconds since the epoch, the
    /// host's clock (T-497).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
    /// The step a working turn is on, one line: a tool call, or `thinking`.
    /// Read from the transcript, as the TUI's card reads it. Live only: a
    /// browser never keeps it with the remembered board.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doing: Option<String>,
    /// The first line of the agent's latest reply. Live only, like `doing`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub said: Option<String>,
    /// The tier id this agent launched on (T-643). Beside a ticket `tier`
    /// that differs, the agent switches at its next idle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,
}

/// The page a pairing QR opens (T-497): the relay's browser origin with the
/// code in the fragment, which a browser never sends to the relay. The page
/// fills the code in and waits for Connect.
pub fn pair_link(origin: &str, code: &str) -> String {
    format!("{}/#pair={code}", origin.trim_end_matches('/'))
}

/// The longest line of an agent's words a phone is sent (T-497): one row.
pub const LINE_MAX_BYTES: usize = 200;

/// A reply's first line for a phone (T-497): the first line that says
/// something, without its heading, quote or bullet marker or its emphasis,
/// scrubbed where it leaves for another process and capped.
pub fn reply_line(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = line.trim_start_matches(['#', '>']).trim_start();
    // A bullet needs its space: `*emphasis*` is not one.
    let line = ["- ", "* ", "+ "].iter().find_map(|b| line.strip_prefix(b)).unwrap_or(line);
    step_line(&line.replace("**", "").replace('`', ""))
}

/// A step's words on one row (T-497): scrubbed and capped, as `reply_line`.
pub fn step_line(step: &str) -> Option<String> {
    let flat = crate::text::scrub_text(&step.replace(['\n', '\t'], " "));
    crate::text::nonblank(crate::text::cap_bytes(&flat, LINE_MAX_BYTES))
}

/// One row of a transcript page (T-626).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptRow {
    /// The byte offset of the record the row came from: its place in the
    /// file, the same for good. One record may give several rows.
    pub at: u64,
    pub kind: RowKind,
    pub text: String,
    /// When the record was written, epoch ms, where it says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowKind {
    /// The person's words.
    Prompt,
    /// The agent's words: a reply, or a progress note between tool calls.
    Reply,
    /// One tool call, on one line.
    Tool,
    /// What happened to the conversation: interrupted, compacted, cleared.
    Notice,
}

/// The most rows a transcript page carries (T-626).
pub const TRANSCRIPT_ROWS: usize = 200;
/// The longest row a phone is sent (T-626); a longer reply is cut and says so.
pub const TRANSCRIPT_ROW_MAX_BYTES: usize = 8 * 1024;
/// A page's rows, serialized, stay under this (T-626): a sealed answer over
/// 48 KiB is refused whole.
pub const TRANSCRIPT_PAGE_BYTES: usize = 32 * 1024;

impl TranscriptRow {
    /// A row's words as a phone is sent them (T-626): lines kept, controls
    /// and format hazards gone, cut at `TRANSCRIPT_ROW_MAX_BYTES`. `None`
    /// for words that say nothing.
    pub fn new(at: u64, kind: RowKind, text: &str, ms: Option<u64>) -> Option<Self> {
        let text = crate::text::scrub_lines(&text.replace('\t', "  "));
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let text = if text.len() > TRANSCRIPT_ROW_MAX_BYTES {
            format!("{}…", crate::text::cap_bytes(text, TRANSCRIPT_ROW_MAX_BYTES - 3).trim_end())
        } else {
            text.to_string()
        };
        Some(TranscriptRow { at, kind, text, ms })
    }
    /// A tool call's row: one line, as `step_line` draws a working step.
    pub fn tool(at: u64, label: &str, ms: Option<u64>) -> Option<Self> {
        Some(TranscriptRow { at, kind: RowKind::Tool, text: step_line(label)?, ms })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Reply {
    Ready {
        incarnation: String,
        next: u64,
        #[serde(default)]
        features: Vec<String>,
    },
    Board {
        title: String,
        columns: Vec<String>,
        tickets: Vec<Ticket>,
        /// Where a ticket lands when it names no column (T-279).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default_column: Option<String>,
        /// What each column is for, in the owner's words (T-467).
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        column_descriptions: BTreeMap<String, String>,
        /// The board's tag vocabulary: every tag a new ticket may wear.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        allowed_tags: Vec<TagOption>,
        /// The tiers a start or a prompt may pick (T-643), the desk's `^n`
        /// ring for an empty seat, in its order.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tiers: Vec<TierOption>,
        /// The tickets paired browsers filed that are archived now (T-665),
        /// newest archive first, each with no agent: Sent says what became of
        /// its ticket instead of losing it. Never in `tickets`, which an
        /// older page draws on the board. Absent from an older host.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        archived: Vec<Ticket>,
    },
    Preview {
        lines: Vec<String>,
        /// The pane's width in cells (T-506), so the browser draws the lines
        /// as the screen they came from. Absent from an older host.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cols: Option<u16>,
    },
    /// A page of a transcript (T-626). `rows` are the records in
    /// `[from, end)`, oldest first; the page before this one ends at
    /// `next_before`, absent when this page reaches the file's start. A
    /// page wholly before the file's end never changes, since the file is
    /// only appended to. `conversation` names the file without spelling
    /// its path, and changes when `/clear` or `/resume` starts a new one.
    Transcript {
        conversation: String,
        rows: Vec<TranscriptRow>,
        from: u64,
        end: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next_before: Option<u64>,
    },
    /// Where a command stands. A dialog answer (T-567) waits at
    /// `awaiting_delivery` until it settles as `answered` (the hook edge said
    /// the dialog took it), `input_sent` (keys went in and nothing confirmed
    /// them) or `unknown`, which names its `reason`.
    Delivery {
        status: String,
        /// Why a dialog answer is `unknown`: `label_not_found`,
        /// `label_wrapped`, `shape_unrecognised`, `deadline`, `state_changed`
        /// or `pane_unreachable`, and `; cursor moved` when keys had already
        /// gone in. Absent everywhere else, and from an older host.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        /// A prompt that took the place of queued words says whose (T-568).
        /// Absent everywhere else, and from an older host.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        replaced: Option<Replaced>,
    },
    Rejected {
        message: String,
    },
    TakenBack {
        text: String,
    },
    /// A filed ticket is on the board: its id, its key and where it landed.
    Created {
        ticket: String,
        key: String,
        column: String,
    },
    /// A rename, move or tag took (T-530); the board that follows shows it.
    Edited {
        ticket: String,
    },
    /// A ticket's notes (T-532), and the description's body when it has one.
    Notes {
        ticket: String,
        notes: Vec<NoteRow>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
    /// One note's body (T-532).
    Note {
        ticket: String,
        note: NoteRow,
        text: String,
    },
    /// A note was written (T-532): its id and revision now, or with no
    /// `note`, deleted.
    NoteWritten {
        ticket: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        #[serde(default)]
        rev: u64,
    },
    /// A picture piece was taken (T-629): the upload it belongs to.
    Uploaded {
        upload: String,
    },
    /// The note moved on since the browser opened it (T-532): nothing was
    /// written, and this is the note as it stands.
    NoteStale {
        ticket: String,
        note: NoteRow,
    },
    Awareness {
        ticket: String,
        awareness: Awareness,
        alert: bool,
    },
    Changed,
    Revoked,
}

impl Reply {
    /// A `Delivery` receipt with no reason.
    pub fn delivery(status: &str) -> Self {
        Reply::Delivery { status: status.into(), reason: None, replaced: None }
    }
}

/// What a phone's send is refused with while its agent waits on a person
/// (T-568): the sentence the board's own send gives (T-420), pointed at the
/// card the phone draws where it draws one it can answer.
pub fn answer_first(here: bool) -> &'static str {
    if here {
        "the agent is waiting on you ∙ answer it here first"
    } else {
        crate::command::ANSWER_IN_PANE_FIRST
    }
}

/// One thing a host leaves a paired browser to read while the host is away
/// (T-698), sealed to that browser alone: the board as its snapshot reads,
/// a ticket's notes, or the newest page of an agent's conversation. Each is
/// the answer the live channel gives, so the page draws it as it draws
/// those, marked as of `at`; nothing on it can be answered.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShelfItem {
    /// When the host wrote it, epoch ms.
    pub at: u64,
    #[serde(flatten)]
    pub held: Shelved,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Shelved {
    /// A `Board` answer, every agent unpromptable and with no dialog or
    /// permission to answer.
    Board { board: Reply },
    /// A ticket's `Notes` answer and a `Note` answer per body it holds.
    Notes { ticket: String, notes: Reply, bodies: Vec<Reply> },
    /// The newest `Transcript` page of the ticket's agent.
    Transcript { ticket: String, session: String, page: Reply },
}
impl Shelved {
    /// The slot's name (`control::shelf_slot`): one board, and one of each
    /// kind per ticket.
    pub fn name(&self) -> String {
        match self {
            Shelved::Board { .. } => "board".into(),
            Shelved::Notes { ticket, .. } => format!("notes:{ticket}"),
            Shelved::Transcript { ticket, .. } => format!("transcript:{ticket}"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Answer {
    pub id: u64,
    pub reply: Reply,
}

/// The most picture bytes one `Upload` carries (T-629). Sealed, a piece is
/// base64 and then hex, so 80 KiB is about 214 KiB on the wire, under the
/// control channel's 256 KiB frame.
pub const PICTURE_CHUNK_BYTES: usize = 80 * 1024;

fn queue_by_default() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_hosts_have_no_m2_features() {
        let Reply::Ready { features, .. } =
            serde_json::from_str::<Reply>(r#"{"result":"ready","incarnation":"old","next":1}"#)
                .unwrap()
        else {
            panic!("ready")
        };
        assert!(features.is_empty());
    }

    /// T-571: a batch answer names each question's answer by the same tag
    /// the single answer uses, and a field the host does not know is refused.
    #[test]
    fn a_batch_answer_carries_one_answer_per_question() {
        let Request::Dialog { response: DialogAnswer::Answers { answers }, .. } =
            serde_json::from_str(
                r#"{"op":"dialog","ticket":"t","session":"s","request":"r","response":
                {"answer":"answers","answers":[{"answer":"choice","index":1},
                {"answer":"choices","indices":[0,2]},{"answer":"text","text":"Mauve"}]}}"#,
            )
            .unwrap()
        else {
            panic!("answers")
        };
        assert_eq!(
            answers,
            [
                QuestionAnswer::Choice { index: 1 },
                QuestionAnswer::Choices { indices: vec![0, 2] },
                QuestionAnswer::Text { text: "Mauve".into() },
            ]
        );
        assert!(serde_json::from_str::<QuestionAnswer>(
            r#"{"answer":"choice","index":1,"indices":[1]}"#
        )
        .is_err());
        // The single answer an older browser sends still reads.
        assert!(matches!(
            serde_json::from_str::<DialogAnswer>(r#"{"answer":"choice","index":0}"#).unwrap(),
            DialogAnswer::Choice { index: 0 }
        ));
    }

    #[test]
    fn approval_is_a_one_shot_native_decision_without_rules_or_input_changes() {
        for (decision, word) in
            [(PermissionDecision::Allow, "allow"), (PermissionDecision::Deny, "deny")]
        {
            assert_eq!(
                decision.hook_output(),
                serde_json::json!({"hookSpecificOutput": {
                    "hookEventName": "PermissionRequest", "decision": {"behavior": word}
                }})
            );
        }
        assert!(serde_json::from_str::<PermissionDecision>("\"ask\"").is_err());
    }

    #[test]
    fn awareness_requires_real_completion_and_preserves_attention_ranks() {
        use crate::board::{Reason as R, SessionState as S, StopReason as E};
        assert_eq!(Phase::of(&S::Idle { stop_reason: E::EndTurn }), Phase::Completed);
        for stop_reason in [E::Interrupted, E::Unknown] {
            assert_eq!(Phase::of(&S::Idle { stop_reason }), Phase::Stale);
        }
        for reason in [R::Permission, R::Plan, R::Trust] {
            assert_eq!(Phase::of(&S::RequiresAction { reason }), Phase::WaitingForApproval);
        }
        for reason in [
            R::Question,
            R::Secret,
            R::Elicitation,
            R::Auth,
            R::QuotaResume,
            R::StartupModal,
            R::ResumeDialog,
        ] {
            assert_eq!(Phase::of(&S::RequiresAction { reason }), Phase::WaitingForInput);
        }
        assert_eq!(Phase::of(&S::Running), Phase::Running);
        assert_eq!(Phase::of(&S::Spawning), Phase::Starting);
        assert_eq!(Phase::of(&S::Sleeping), Phase::Stale);
    }

    /// A filed ticket names only a title; everything else takes the board's
    /// default, and a field the host does not know is refused, not dropped.
    #[test]
    fn a_filed_ticket_needs_a_title_and_defaults_the_rest() {
        let Request::Create { title, description, column, tags, uploads } =
            serde_json::from_str(r#"{"op":"create","title":"Fix it"}"#).unwrap()
        else {
            panic!("create")
        };
        assert_eq!(
            (title.as_str(), description.as_str(), column, tags),
            ("Fix it", "", None, vec![])
        );
        assert!(uploads.is_empty());
        let full = r#"{"op":"create","title":"t","description":"d","column":"TODO","tags":[{"group":1,"name":"BUG"}]}"#;
        let Request::Create { column, tags, .. } = serde_json::from_str(full).unwrap() else {
            panic!("create")
        };
        assert_eq!(column.as_deref(), Some("TODO"));
        assert_eq!(tags, vec![TagPick { group: 1, name: "BUG".into() }]);
        for bad in [
            r#"{"op":"create"}"#,
            r#"{"op":"create","title":"t","autorun":true}"#,
            r#"{"op":"create","title":"t","tags":[{"group":1,"name":"BUG","tint":3}]}"#,
        ] {
            assert!(serde_json::from_str::<Request>(bad).is_err(), "{bad}");
        }
    }

    /// A note write names its ticket and words; a fresh note has no id or
    /// revision, and a field the host does not know is refused.
    #[test]
    fn a_note_write_needs_a_ticket_and_text_and_defaults_the_rest() {
        let Request::WriteNote { ticket, note, text, rev, uploads } =
            serde_json::from_str(r#"{"op":"write_note","ticket":"01J","text":"hi"}"#).unwrap()
        else {
            panic!("write_note")
        };
        assert_eq!((ticket.as_str(), note, text.as_str(), rev), ("01J", None, "hi", None));
        assert!(uploads.is_empty());
        for bad in [
            r#"{"op":"write_note","ticket":"01J"}"#,
            r#"{"op":"write_note","text":"hi"}"#,
            r#"{"op":"write_note","ticket":"01J","text":"hi","force":true}"#,
            r#"{"op":"tell_agent","ticket":"01J"}"#,
        ] {
            assert!(serde_json::from_str::<Request>(bad).is_err(), "{bad}");
        }
    }

    /// A picture piece for a ticket not filed yet names no ticket, and is
    /// spelled without one (T-670); the filing that links it names it.
    #[test]
    fn a_picture_for_a_ticket_not_filed_yet_names_no_ticket() {
        let piece = r#"{"op":"upload","offset":0,"data":"AA=="}"#;
        let Request::Upload { ticket, upload, .. } = serde_json::from_str(piece).unwrap() else {
            panic!("upload")
        };
        assert_eq!((ticket, upload), (None, None));
        let filed = Request::Upload {
            ticket: None,
            upload: None,
            offset: 0,
            data: "AA==".into(),
            complete: false,
        };
        assert!(!serde_json::to_string(&filed).unwrap().contains("ticket"));
        let create = r#"{"op":"create","title":"t","description":"[Image #1](mesimon-attachment:01J)","uploads":["01J"]}"#;
        let Request::Create { uploads, .. } = serde_json::from_str(create).unwrap() else {
            panic!("create")
        };
        assert_eq!(uploads, vec!["01J".to_string()]);
    }

    /// A letter is a note edit only when it says so: phase 3's tickets have
    /// no kind, and a note's letter is never read as a ticket.
    #[test]
    fn a_letter_is_a_note_only_when_it_says_so() {
        let ticket = serde_json::json!({"title": "Fix it", "written_at": 5});
        assert!(matches!(read_letter(ticket), Some(Letter::Ticket(t)) if t.title == "Fix it"));
        let note = serde_json::json!({"kind": "note", "ticket": "01J", "note": "01K",
            "text": "new words", "rev": 3, "written_at": 5});
        let Some(Letter::Note(n)) = read_letter(note) else { panic!("note") };
        assert_eq!(
            (n.note.as_deref(), n.rev, n.text.as_str()),
            (Some("01K"), Some(3), "new words")
        );
        assert!(read_letter(serde_json::json!({"kind": "note", "text": "no ticket"})).is_none());
        // What an older host does with a note's letter: no title, no ticket.
        let body = serde_json::json!({"kind": "note", "ticket": "01J", "text": "x"});
        assert!(serde_json::from_value::<MailTicket>(body).is_err());
    }

    /// The stamp moves with any note's revision, an added or a removed one,
    /// and is empty with none, so a board without notes says nothing.
    #[test]
    fn the_notes_stamp_moves_with_every_write() {
        let meta = |rev| crate::board::NoteMeta {
            id: ulid::Ulid::from_parts(1, 1),
            name: "n".into(),
            rev,
            created_at: "@1".into(),
            created_by: "local".into(),
            edited_at: "@1790845550".into(),
            edited_by: "local".into(),
        };
        let one = notes_stamp(&[meta(1)]);
        assert_eq!(one.len(), 16);
        assert_eq!(one, notes_stamp(&[meta(1)]));
        assert_ne!(one, notes_stamp(&[meta(2)]));
        let mut other = meta(1);
        other.id = ulid::Ulid::from_parts(2, 2);
        assert_ne!(one, notes_stamp(&[meta(1), other]));
        assert_eq!(notes_stamp(&[]), "");
        let row = NoteRow::of(&meta(4), "you".into());
        assert_eq!((row.at, row.rev, row.by.as_str()), (1_790_845_550_000, 4, "you"));
    }

    /// The sheet's facts ride the board reply beside what an older browser
    /// already reads, and a board without them still parses.
    #[test]
    fn the_board_reply_carries_the_sheet_facts_only_when_there_are_any() {
        let bare = Reply::Board {
            title: "b".into(),
            columns: vec!["TODO".into()],
            tickets: vec![],
            default_column: None,
            column_descriptions: BTreeMap::new(),
            allowed_tags: vec![],
            tiers: vec![],
            archived: vec![],
        };
        let json = serde_json::to_value(&bare).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"result":"board","title":"b","columns":["TODO"],"tickets":[]})
        );
        let Reply::Board { default_column, allowed_tags, tiers, archived, .. } =
            serde_json::from_value(json).unwrap()
        else {
            panic!("board")
        };
        assert!(default_column.is_none() && allowed_tags.is_empty() && tiers.is_empty());
        assert!(archived.is_empty());
        let created = serde_json::to_value(Reply::Created {
            ticket: "01J".into(),
            key: "T-7".into(),
            column: "TODO".into(),
        })
        .unwrap();
        assert_eq!(
            created,
            serde_json::json!({"result":"created","ticket":"01J","key":"T-7","column":"TODO"})
        );
    }

    /// The projection's newer facts are absent unless there is something to
    /// say, so an older browser reads the same bytes as before; and a reply
    /// without them, from an older host, still parses.
    #[test]
    fn a_ticket_carries_tags_pickup_and_the_agent_s_step_only_when_known() {
        let bare = Ticket {
            queued: None,
            queue: None,
            id: "01J".into(),
            key: "T-1".into(),
            title: "t".into(),
            column: "TODO".into(),
            agent: Some(Agent {
                permission: None,
                dialog: None,
                session: "s".into(),
                provider: "claude".into(),
                state: "working".into(),
                promptable: true,
                since: None,
                doing: None,
                said: None,
                tier: None,
            }),
            tags: vec![],
            picked: None,
            notes: 0,
            noted: String::new(),
            crown: false,
            crowned: None,
            workspace: None,
            tier: None,
            created: None,
        };
        let json = serde_json::to_value(&bare).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"queued":null,"id":"01J","key":"T-1","title":"t","column":"TODO",
                "agent":{"permission":null,"dialog":null,"session":"s","provider":"claude",
                "state":"working","promptable":true}})
        );
        let full = Ticket {
            tags: vec![TagOption { group: 1, name: "BUG".into(), tint: 3 }],
            picked: Some(Picked { by: "desk".into(), at: 1_790_000_000_000 }),
            notes: 2,
            noted: "00ff00ff00ff00ff".into(),
            crown: true,
            crowned: Some(Crowned { action: "moved".into(), by: Some("T-9".into()), at: 1 }),
            workspace: Some(Workspace {
                kind: "worktree".into(),
                state: "ahead".into(),
                ahead: 3,
                ..Workspace::default()
            }),
            tier: Some("01K".into()),
            created: Some(1_789_000_000_000),
            agent: bare.agent.clone().map(|a| Agent {
                since: Some(1_790_000_000_000),
                doing: Some("Bash(cargo test)".into()),
                said: Some("Fixed.".into()),
                tier: Some("claude".into()),
                ..a
            }),
            ..bare
        };
        let back: Ticket = serde_json::from_value(serde_json::to_value(&full).unwrap()).unwrap();
        assert_eq!(back.tags, full.tags);
        assert_eq!(back.picked, full.picked);
        assert_eq!((back.notes, back.noted.as_str()), (2, "00ff00ff00ff00ff"));
        assert!(back.crown);
        assert_eq!(back.crowned, full.crowned);
        assert_eq!(back.workspace, full.workspace);
        assert_eq!(back.tier.as_deref(), Some("01K"));
        assert_eq!(back.created, Some(1_789_000_000_000));
        let agent = back.agent.unwrap();
        assert_eq!(agent.tier.as_deref(), Some("claude"));
        assert_eq!(
            (agent.since, agent.doing.as_deref(), agent.said.as_deref()),
            (Some(1_790_000_000_000), Some("Bash(cargo test)"), Some("Fixed."))
        );
    }

    /// A phone gets one plain row of a reply: markers, emphasis and hazards
    /// go, a long line is cut on a character, and silence stays silence.
    #[test]
    fn a_reply_reaches_a_phone_as_its_first_plain_line() {
        assert_eq!(
            reply_line("\n\n## Fixed, and three tests pass.\nmore").as_deref(),
            Some("Fixed, and three tests pass.")
        );
        assert_eq!(
            reply_line("- **Updated** `CHANGELOG.md`").as_deref(),
            Some("Updated CHANGELOG.md")
        );
        assert_eq!(reply_line("*emphasis* stays").as_deref(), Some("*emphasis* stays"));
        assert_eq!(reply_line("> quoted").as_deref(), Some("quoted"));
        assert_eq!(reply_line("a\u{202e}b\x07c").as_deref(), Some("abc"));
        assert_eq!(reply_line("  \n \n"), None);
        assert_eq!(reply_line("##"), None);
        let long = "é".repeat(150);
        let cut = reply_line(&long).unwrap();
        assert!(cut.len() <= LINE_MAX_BYTES && cut.chars().all(|c| c == 'é'));
        assert_eq!(step_line("Bash(cargo\ntest)").as_deref(), Some("Bash(cargo test)"));
    }

    /// The QR's page keeps the code off the wire: it rides the fragment.
    #[test]
    fn the_pairing_link_puts_the_code_in_the_fragment() {
        let code = "7K2M-QX4P-0B9D-RT6W-HN3C-5VJE-8FGA-1YSZ";
        for origin in ["https://remote.mesimon.dev", "https://remote.mesimon.dev/"] {
            assert_eq!(pair_link(origin, code), format!("https://remote.mesimon.dev/#pair={code}"));
        }
        assert_eq!(pair_link("http://localhost:8444", "C"), "http://localhost:8444/#pair=C");
    }

    /// A start names a ticket and, since T-510, the first turn's words;
    /// the provider and the mode are the host's, so a field that tries to
    /// pick one is refused. A page that sends no `prompt` (T-498's) still
    /// parses: the words are then the ticket's own.
    #[test]
    fn a_start_names_its_ticket_and_at_most_a_prompt() {
        let Request::Start { ticket, prompt, tier } =
            serde_json::from_str(r#"{"op":"start","ticket":"01J"}"#).unwrap()
        else {
            panic!("start")
        };
        assert_eq!(ticket, "01J");
        assert_eq!((prompt, tier), (None, None));
        // T-643: a tier the board offers may ride it.
        let Request::Start { tier, .. } =
            serde_json::from_str(r#"{"op":"start","ticket":"01J","tier":"coder"}"#).unwrap()
        else {
            panic!("start")
        };
        assert_eq!(tier.as_deref(), Some("coder"));
        let Request::Start { prompt, .. } =
            serde_json::from_str(r#"{"op":"start","ticket":"01J","prompt":"fix the test"}"#)
                .unwrap()
        else {
            panic!("start")
        };
        assert_eq!(prompt.as_deref(), Some("fix the test"));
        for bad in [
            r#"{"op":"start"}"#,
            r#"{"op":"start","ticket":"01J","provider":"codex"}"#,
            r#"{"op":"start","ticket":"01J","plan":true}"#,
        ] {
            assert!(serde_json::from_str::<Request>(bad).is_err(), "{bad}");
        }
    }

    /// The card edits (T-530) name the ticket and what changes, nothing
    /// else; a move without `before` lands at the column's end, and a tag
    /// without `name` takes the group's tag off.
    #[test]
    fn a_card_edit_names_its_ticket_and_only_what_changes() {
        let Request::Move { ticket, column, before } =
            serde_json::from_str(r#"{"op":"move","ticket":"01J","column":"DONE"}"#).unwrap()
        else {
            panic!("move")
        };
        assert_eq!((ticket.as_str(), column.as_str(), before), ("01J", "DONE", None));
        let Request::Move { before, .. } =
            serde_json::from_str(r#"{"op":"move","ticket":"01J","column":"DONE","before":"01K"}"#)
                .unwrap()
        else {
            panic!("move")
        };
        assert_eq!(before.as_deref(), Some("01K"));
        let Request::Tag { group, name, .. } =
            serde_json::from_str(r#"{"op":"tag","ticket":"01J","group":2}"#).unwrap()
        else {
            panic!("tag")
        };
        assert_eq!((group, name), (2, None));
        let Request::Rename { title, .. } =
            serde_json::from_str(r#"{"op":"rename","ticket":"01J","title":"Fix it"}"#).unwrap()
        else {
            panic!("rename")
        };
        assert_eq!(title, "Fix it");
        let Request::Workspace { worktree, .. } =
            serde_json::from_str(r#"{"op":"workspace","ticket":"01J","worktree":true}"#).unwrap()
        else {
            panic!("workspace")
        };
        assert!(worktree);
        for bad in [
            r#"{"op":"rename","ticket":"01J"}"#,
            r#"{"op":"move","ticket":"01J"}"#,
            r#"{"op":"move","ticket":"01J","column":"DONE","force":true}"#,
            r#"{"op":"tag","ticket":"01J","group":1,"name":"NEW","register":true}"#,
            r#"{"op":"workspace","ticket":"01J"}"#,
            r#"{"op":"workspace","ticket":"01J","worktree":"adopt"}"#,
        ] {
            assert!(serde_json::from_str::<Request>(bad).is_err(), "{bad}");
        }
        assert_eq!(
            serde_json::to_value(Reply::Edited { ticket: "01J".into() }).unwrap(),
            serde_json::json!({"result":"edited","ticket":"01J"})
        );
    }

    /// A phone reads a ticket's workspace as the TUI's card marks it
    /// (T-642): a settled shared checkout says nothing, an open one says it
    /// is open, a worktree asked for is `planned`, and a cut one carries its
    /// branch and the card's word, what is wrong first. A stage's error
    /// text, which may name a path, never leaves.
    #[test]
    fn a_ticket_s_workspace_reads_as_the_card_marks_it() {
        use crate::board::WorkspaceStrategy::{SharedCheckout, Worktree};
        let item = |status: &str| crate::command::WorktreeItem {
            ticket: ulid::Ulid::nil(),
            branch: "msmn/T-1-fix".into(),
            status: status.into(),
            merged: false,
            merged_in: String::new(),
            merged_oid: String::new(),
            conflict: false,
            ahead: 0,
            needs_rebase: false,
            detail: None,
            path: Some("/Users/me/state/worktrees/T-1-fix".into()),
            repos: Vec::new(),
        };
        assert_eq!(workspace(SharedCheckout, false, None), None);
        let open = workspace(SharedCheckout, true, None).unwrap();
        assert_eq!((open.kind.as_str(), open.open, open.state.as_str()), ("shared", true, ""));
        let planned = workspace(Worktree, true, None).unwrap();
        assert_eq!((planned.kind.as_str(), planned.state.as_str()), ("worktree", "planned"));
        assert_eq!(workspace(Worktree, false, None).unwrap().state, "planned");
        let word = |w: crate::command::WorktreeItem| workspace(Worktree, false, Some(&w)).unwrap();
        assert_eq!(word(item("queued")).state, "provisioning");
        let provisioning = word(crate::command::WorktreeItem {
            detail: Some("7/19".into()),
            ..item("provisioning")
        });
        assert_eq!(provisioning.detail.as_deref(), Some("7/19"));
        let failed = word(crate::command::WorktreeItem {
            detail: Some("add: fatal: '/Users/me/x' exists".into()),
            ..item("error")
        });
        assert_eq!((failed.state.as_str(), failed.detail), ("error", None));
        assert_eq!(word(item("evicted")).state, "evicted");
        let both = crate::command::WorktreeItem {
            conflict: true,
            merged: true,
            needs_rebase: true,
            ahead: 2,
            ..item("attached")
        };
        assert_eq!(word(both.clone()).state, "conflict");
        let both = crate::command::WorktreeItem { conflict: false, ..both };
        assert_eq!(word(both.clone()).state, "merged");
        let both = crate::command::WorktreeItem { merged: false, ..both };
        assert_eq!(word(both.clone()).state, "behind");
        let ahead = word(crate::command::WorktreeItem { needs_rebase: false, ..both });
        assert_eq!((ahead.state.as_str(), ahead.ahead), ("ahead", 2));
        let clean = word(item("attached"));
        assert_eq!((clean.state.as_str(), clean.branch.as_str()), ("clean", "msmn/T-1-fix"));
        let wire = serde_json::to_string(&clean).unwrap();
        assert!(!wire.contains("/Users/"), "{wire}");
        assert_eq!(
            serde_json::to_value(&clean).unwrap(),
            serde_json::json!({"kind":"worktree","branch":"msmn/T-1-fix","state":"clean"})
        );
    }

    /// A queued ask's facts ride beside its words (T-568): an older page
    /// reads `queued` as the words and never sees an object there, and a
    /// person's own ask with nothing ahead says nothing more.
    #[test]
    fn the_queue_s_facts_ride_beside_its_words() {
        let ticket = |queue| Ticket {
            queued: Some("next".into()),
            queue,
            id: "01J".into(),
            key: "T-3".into(),
            title: "t".into(),
            column: "TODO".into(),
            agent: None,
            tags: vec![],
            picked: None,
            notes: 0,
            noted: String::new(),
            crown: false,
            crowned: None,
            workspace: None,
            tier: None,
            created: None,
        };
        let json = serde_json::to_value(ticket(Some(Queue::default()))).unwrap();
        assert_eq!((&json["queued"], &json["queue"]), (&"next".into(), &serde_json::json!({})));
        assert!(serde_json::to_value(ticket(None)).unwrap().get("queue").is_none());
        let held = Queue {
            by: Some("T-411".into()),
            sends: false,
            held: Some("agent asked".into()),
            waits: vec![],
            asking: vec!["T-3".into()],
        };
        let json = serde_json::to_value(ticket(Some(held.clone()))).unwrap();
        assert_eq!(
            json["queue"],
            serde_json::json!({"by": "T-411", "held": "agent asked", "asking": ["T-3"]})
        );
        let back: Ticket = serde_json::from_value(json).unwrap();
        assert_eq!(back.queue, Some(held));
        // From a host before T-568: the words alone.
        let old: Ticket = serde_json::from_str(
            r#"{"queued":"next","id":"01J","key":"T-3","title":"t","column":"TODO","agent":null}"#,
        )
        .unwrap();
        assert!(old.queue.is_none());
    }

    /// A receipt says whose words a prompt replaced, and only then; an older
    /// host's receipt, without the field, still parses.
    #[test]
    fn a_receipt_names_whose_queued_words_it_replaced() {
        assert_eq!(
            serde_json::to_value(Reply::delivery("queued")).unwrap(),
            serde_json::json!({"result": "delivery", "status": "queued"})
        );
        let replaced = Reply::Delivery {
            status: "queued".into(),
            reason: None,
            replaced: Some(Replaced { by: "T-411".into() }),
        };
        assert_eq!(
            serde_json::to_value(&replaced).unwrap(),
            serde_json::json!({"result": "delivery", "status": "queued", "replaced": {"by": "T-411"}})
        );
        let Reply::Delivery { replaced, .. } =
            serde_json::from_str(r#"{"result":"delivery","status":"submitted"}"#).unwrap()
        else {
            panic!("delivery")
        };
        assert_eq!(replaced, None);
    }

    /// A refused send names where the answer goes: the phone's own card when
    /// it draws one it can answer, else the pane, in the board's words.
    #[test]
    fn a_send_at_a_dialog_is_refused_toward_the_answer() {
        assert_eq!(answer_first(true), "the agent is waiting on you ∙ answer it here first");
        assert_eq!(answer_first(false), crate::command::ANSWER_IN_PANE_FIRST);
        assert!(answer_first(false).ends_with("answer it in the pane first"));
    }

    #[test]
    fn phone_composers_queue_unless_they_explicitly_steer() {
        let request = r#"{"op":"prompt","ticket":"t","session":"s","text":"next"}"#;
        assert!(matches!(
            serde_json::from_str::<Request>(request).unwrap(),
            Request::Prompt { queued: true, .. }
        ));
        let request = r#"{"op":"prompt","ticket":"t","session":"s","text":"next","queued":false}"#;
        assert!(matches!(
            serde_json::from_str::<Request>(request).unwrap(),
            Request::Prompt { queued: false, .. }
        ));
        // T-643: a tier pick rides the words, and a prompt without one
        // serializes as an older host reads it.
        let request = r#"{"op":"prompt","ticket":"t","session":"s","text":"next","tier":"coder"}"#;
        assert!(matches!(
            serde_json::from_str::<Request>(request).unwrap(),
            Request::Prompt { tier: Some(t), .. } if t == "coder"
        ));
        let plain = Request::Prompt {
            ticket: "t".into(),
            session: "s".into(),
            text: "next".into(),
            queued: true,
            tier: None,
        };
        assert!(serde_json::to_value(plain).unwrap().get("tier").is_none());
    }
}

/// A small daemon-computed notification payload. No terminal or board contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Starting,
    Running,
    WaitingForApproval,
    WaitingForInput,
    Completed,
    Failed,
    Stale,
}
impl Phase {
    pub fn of(state: &crate::board::SessionState) -> Self {
        use crate::board::{Reason, SessionState as S, StopReason};
        match state {
            S::Spawning => Self::Starting,
            S::Running
            | S::Idle { stop_reason: StopReason::Background | StopReason::Monitoring } => {
                Self::Running
            }
            S::RequiresAction { reason: Reason::Permission | Reason::Plan | Reason::Trust } => {
                Self::WaitingForApproval
            }
            S::RequiresAction { .. } => Self::WaitingForInput,
            S::Idle { stop_reason: StopReason::EndTurn } => Self::Completed,
            S::Failed { .. } => Self::Failed,
            _ => Self::Stale,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Awareness {
    pub phase: Phase,
    pub headline: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(rename = "deepLink")]
    pub deep_link: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Dialog {
    pub request: String,
    #[serde(flatten)]
    pub content: DialogContent,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DialogContent {
    Questions { questions: Vec<Question> },
    Plan { markdown: String },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    pub question: String,
    pub header: String,
    pub options: Vec<QuestionOption>,
    #[serde(rename = "multiSelect", default)]
    pub multi_select: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionOption {
    pub label: String,
    pub description: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "answer", rename_all = "snake_case", deny_unknown_fields)]
pub enum DialogAnswer {
    Choice {
        index: usize,
    },
    Text {
        text: String,
    },
    Accept,
    Reject,
    /// One answer per question, in order (T-571): a batch, or a question
    /// that takes several choices. A browser sends it only to a host that
    /// advertised `dialog_multi`; an older host would drop the peer on it.
    Answers {
        answers: Vec<QuestionAnswer>,
    },
}
/// One question's answer inside `DialogAnswer::Answers` (T-571): an option
/// of a one-choice question, the options of a several-choice one, or words
/// typed in the question's own text row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "answer", rename_all = "snake_case", deny_unknown_fields)]
pub enum QuestionAnswer {
    Choice { index: usize },
    Choices { indices: Vec<usize> },
    Text { text: String },
}
