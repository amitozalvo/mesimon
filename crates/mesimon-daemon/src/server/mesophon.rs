//! Owner control is authenticated and applied here, on the board's single writer.
use super::*;
use crate::team::control_io::{self, Event as NetEvent};
use mesimon_core::mesophon::{self as api, Answer, Info, LocalAction, Reply};
use mesimon_team::{
    control::{self, Channel, Wire},
    crypto::{BoardId, DeviceId, DevicePublic, ObjectId},
    hex,
    invite::InviteCode,
};
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashSet},
    sync::mpsc::SyncSender,
};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Grant {
    pub(super) id: BoardId,
    pub(super) device: DeviceId,
    pub(super) public: DevicePublic,
    name: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Stored {
    pub(super) schema: u32,
    pub(super) board: BoardId,
    host: DeviceId,
    pub(super) grants: Vec<Grant>,
    /// The envelopes filed most recently, newest last (T-497). The ticket's
    /// own `envelope` makes a second delivery file nothing; this answers one
    /// whose ticket has since been deleted, so a replayed envelope cannot
    /// bring it back. An older build drops it on its next save, harmlessly.
    #[serde(default)]
    filed: Vec<Filed>,
    /// The note letters answered most recently, newest last (T-532), with
    /// their answers: a replayed letter is answered again and writes nothing.
    #[serde(default)]
    noted: Vec<NoteFiled>,
    /// The person turned the relay's copy off (T-698). Written only with
    /// `schema: 2`, which an older build bars rather than drop this and
    /// leave a copy on a later upgrade.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) shelf_off: bool,
}
/// A board disabled before the relay heard it had no devices left (T-698):
/// until the relay acknowledges an empty list for it, its browsers could
/// still read their shelves there, so the host keeps connecting to say so.
/// Kept in `mesophon-retired.json` across restarts; a board enabled again
/// meanwhile takes this id back, and its first publish says the same.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub(super) struct Retired {
    board: BoardId,
    host: DeviceId,
}
#[derive(Clone, Serialize, Deserialize)]
struct NoteFiled {
    envelope: ObjectId,
    /// Kept as JSON, read back when asked: an answer a later build cannot
    /// read is unknown, never a state file that does not open.
    answer: serde_json::Value,
}
#[derive(Clone, Serialize, Deserialize)]
struct Filed {
    envelope: ObjectId,
    ticket: String,
    key: String,
    column: String,
}
/// Where a retirement the relay has not heard waits (`Retired`).
const RETIRED_FILE: &str = "mesophon-retired.json";
/// How many filed envelopes `Stored::filed` remembers.
const FILED_KEEP: usize = 256;
/// Past this many bytes a board goes to the browser without its agents'
/// words, well inside the 48 KiB an answer may carry.
const BOARD_WORDS_BUDGET: usize = 40 * 1024;

/// A reply's serialized size, counted and never kept (T-688): the board
/// answer was serialized four or five times to be measured, per phone, up
/// to once a second. `None` is a reply that does not serialize.
fn measure(reply: &Reply) -> Option<usize> {
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0 += buf.len();
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0);
    serde_json::to_writer(&mut count, reply).ok().map(|()| count.0)
}
/// How many archived tickets a phone filed ride the board (T-665): as many
/// as Sent keeps (`KEEP` in `web/mesophon/sent.js`).
const ARCHIVED_SENT: usize = 50;
struct Peer {
    grant: BoardId,
    device: DeviceId,
    channel: Channel,
    subscribed: bool,
    foreground: Option<(String, Instant)>,
    floor: u64,
    ceiling: u64,
    high: u64,
}
/// An answer on its way down a pane (T-567): a dialog's, walked one key a
/// pass and confirmed by the hook edge, or a Codex approval's (T-846),
/// walked to its `Yes` or `No` row and confirmed by the request leaving the
/// runtime's snapshot.
struct DialogDelivery {
    by: Deliverer,
    ticket: ulid::Ulid,
    request: String,
    answer: DeliveryAnswer,
    deadline: Instant,
    next: Instant,
    steps: u8,
    /// The most keys this answer may take (`walk_budget`).
    max_steps: u8,
    walk: Walk,
    /// Set once the last key is in (T-567): until then the answer waits for
    /// the hook edge, and past it the keys were sent and not confirmed.
    confirm: Option<Instant>,
    /// The `answer` frame the answer went down the session's mod in
    /// (T-576), in place of every key: confirmed by the mod's own report
    /// alone, since a dialog the mod closed fires no `PostToolUse`, and one
    /// that does fire was the person's.
    by_mod: Option<String>,
}
/// What a delivery answers (T-873).
enum DeliveryAnswer {
    /// A question, a plan or a trust dialog the hook stream or a Codex
    /// snapshot projected into `dialogs`.
    Dialog(api::DialogAnswer),
    /// A Codex approval waiting in `permissions`, a phone's alone.
    Approval { allow: bool },
    /// The crown's plan sent back with words (T-943): the plan dialog's
    /// `Tell Claude what to change` row, its empty field pasted into and
    /// Enter pressed, walked only while the seat holds the pane it held at
    /// the call (`pane`, the record's key then) and no tmux client is in it.
    Revise { text: String, pane: Option<String> },
}
/// The request a delivery answers while it still stands, with its answer:
/// what `dialog_step_for` walks.
#[derive(Clone, Copy)]
enum Standing<'a> {
    Dialog(&'a api::Dialog, &'a api::DialogAnswer),
    Approval(&'a api::Permission, bool),
    /// A plan dialog the crown sends back, with its words (T-943).
    Revise(&'a api::Dialog, &'a str),
}
/// Whose answer a dialog delivery carries, and where its receipt goes.
enum Deliverer {
    /// A paired phone's: the receipt is its command's (T-567).
    Phone { grant: BoardId, device: DeviceId, command: u64 },
    /// The board's own `w` on a trust dialog (T-844): a person at the
    /// terminal, whose receipt is the card's state and a feed line.
    Board,
    /// The crown's `answer_agent` (T-569). The shim's call waits on `reply`
    /// until the delivery settles; `answer` is the label or the text, for
    /// the feed and the card; `prior` is the turn's mark before the answer
    /// took it, given back when the keys went in and the dialog stood.
    Crown {
        crown: ulid::Ulid,
        session: uuid::Uuid,
        answer: String,
        reply: Option<Sender<ClientReply>>,
        prior: Option<Option<TurnAsk>>,
    },
}
impl Deliverer {
    fn principal(&self) -> Principal {
        match self {
            Deliverer::Phone { grant, device, .. } => {
                Principal::Paired { device: device.to_hex(), grant: grant.to_hex() }
            }
            Deliverer::Crown { session, .. } => Principal::Agent { session: *session },
            Deliverer::Board => Principal::Local,
        }
    }
}
/// How long an answer's last key waits for the hook edge (T-567).
const DIALOG_CONFIRM: Duration = Duration::from_secs(5);
/// How long a plan sent back waits for its dialog to leave after the Enter
/// (T-943): the replanning model's first tool call, the hook road's edge,
/// came 7 s after it on 2.1.310. Inside the shim's 75 s; never retried.
const PLAN_CHANGE_CONFIRM: Duration = Duration::from_secs(30);
/// The longest a batch's keys may walk (T-571): `walk_budget`'s ceiling,
/// and with `DIALOG_CONFIRM` inside the shim's wait for `answer_agent`
/// (`ANSWER_WAIT_SECS` in the `mesimon` crate's `mcp.rs`).
const DIALOG_WALK_MAX: Duration = Duration::from_secs(60);
/// The longest text an answer types into a dialog: Remote Control's cap,
/// which `dialog_plan` enforces for every answer.
const DIALOG_TEXT_MAX: usize = 1000;
/// How much of the crown's answer the card's line carries (T-569), inside
/// `SessionRecord::detail`'s 200.
const CROWN_ANSWER_LINE_BYTES: usize = 160;
/// How a hook frame ended a projected dialog (T-567).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DialogEdge {
    /// The dialog's own tool finished: it took an answer.
    Answered,
    /// The session's mod returned the board's answer in the dialog's place
    /// (T-576): its `answered` report, never a `PostToolUse` of the engine's.
    ModAnswered,
    /// The next tool started with no answer taken, the refusal road (T-447),
    /// or the mod saw the person refuse the dialog (T-576).
    Dismissed,
}
struct PermissionWait {
    projection: api::Permission,
    road: PermissionRoad,
    /// When it was offered; a renewal keeps it.
    offered: Instant,
    deadline: Instant,
    /// Only devices paired when the prompt was offered may answer it, from
    /// any connection: a phone that slept and came back is the same phone
    /// (T-632).
    grants: Vec<BoardId>,
}
/// How a permission's answer reaches its agent.
enum PermissionRoad {
    /// Claude's `PermissionRequest` hook: `mesimon approve` holds its stdout
    /// open on `stream`, and the decision is written there.
    Hook {
        stream: UnixStream,
        /// The mod's `mesimon approve --renew`: its run ends every round,
        /// and the next run takes the wait over (`PERMISSION_RENEW_GRACE`).
        renews: bool,
        /// When a renewing run's stream closed.
        closed: Option<Instant>,
    },
    /// A Codex approval (T-846), projected off the runtime's snapshot: the
    /// answer walks the pane's dialog to its `Yes` or `No` row and presses
    /// Enter, and `serverRequest/resolved` on the ledger confirms it.
    Codex,
}
/// How long a renewing wait outlives its run for the next one to take it
/// over (T-632): the mod starts it at once, so this is slack for a loaded
/// machine.
const PERMISSION_RENEW_GRACE: Duration = Duration::from_secs(5);
/// How old a wait must be before the session's own next tool call says its
/// dialog is gone (T-632). The mod relays the dialog's own `PreToolUse` in
/// order behind earlier frames, so it may land after the wait began.
const PERMISSION_PASSED_AFTER: Duration = Duration::from_secs(5);
struct Invite {
    code: InviteCode,
    expires: Instant,
}
impl Invite {
    fn accepts(&self, public: &DevicePublic, proof: &str, now: Instant) -> bool {
        now < self.expires
            && hex::decode::<32>(proof).is_some_and(|p| self.code.verifies_proof(public, &p))
    }
}
/// A start a paired phone asked for (T-498), followed until its session
/// runs: `control_follow_starts` turns the command's receipt from
/// `provisioning` or `starting` into `started`, or a refusal.
struct StartWait {
    grant: BoardId,
    device: DeviceId,
    command: u64,
    /// The record the spawn made; `None` while the worktree is being cut.
    session: Option<uuid::Uuid>,
}
/// What `control_cancel_kept` took, to be put back whole (T-958).
pub(super) struct Cancelled {
    id: uuid::Uuid,
    pending: Pending,
    /// Each receipt it overwrote, with what was there before.
    receipts: Vec<(BoardId, u64, Option<Reply>)>,
    owed: Option<Owed>,
    codex_due: Option<u64>,
    pending_submit: bool,
}

#[derive(Clone)]
struct Pending {
    grant: BoardId,
    device: DeviceId,
    command: u64,
    ticket: ulid::Ulid,
    pasted: bool,
    send_now_receipt: Option<(BoardId, DeviceId, u64)>,
}

pub(super) struct Control {
    pub(super) tx: Sender<Msg>,
    pub(super) jobs: Option<SyncSender<Wire>>,
    generation: u64,
    pub(super) stored: Option<Stored>,
    /// A disabled board the relay has not yet heard is empty (T-698).
    retired: Option<Retired>,
    barred: bool,
    pub(super) keys: Option<mesimon_team::crypto::DeviceKeys>,
    identity: Option<String>,
    invite: Option<Invite>,
    peers: HashMap<String, Peer>,
    high: HashMap<BoardId, u64>,
    receipts: HashMap<BoardId, BTreeMap<u64, Reply>>,
    pending: HashMap<uuid::Uuid, Pending>,
    /// Starts on their way, by ticket (T-498).
    starts: HashMap<ulid::Ulid, StartWait>,
    permissions: HashMap<uuid::Uuid, PermissionWait>,
    /// The request each Codex seat's snapshot last carried (T-846), by
    /// its id: a change is the old request's resolution, the edge a Codex
    /// dialog's answer is confirmed by; `plan:` prefixed for a plan.
    codex_asks: HashMap<uuid::Uuid, String>,
    phases: HashMap<ulid::Ulid, api::Phase>,
    suppress_awareness: bool,
    dialogs: HashMap<uuid::Uuid, api::Dialog>,
    dialog_deliveries: HashMap<uuid::Uuid, DialogDelivery>,
    /// The last dialog a hook edge ended on each session, and how (T-567).
    dialog_edges: HashMap<uuid::Uuid, (String, DialogEdge)>,
    /// The `RequiresAction` spell each seat's dialog was last looked for in
    /// (T-933), by its `state_changed_at`: once per spell, or for a record
    /// that names its call, again as its transcript grows (T-957 round 9).
    /// `GIVEN` for the length: the hook stream gave the spell its dialog.
    dialogs_read: HashMap<uuid::Uuid, (u64, u64)>,
    incarnation: ObjectId,
    origin: String,
    pub(super) online: bool,
    /// This relay keeps mail for the host while it is away (T-497).
    mail: bool,
    /// This relay keeps a shelf for each paired browser (T-698).
    pub(super) shelf_relay: bool,
    /// What this host left there, and what goes next.
    pub(super) shelf: super::shelf::Shelf,
    error: Option<String>,
    retry: Instant,
    dirty: bool,
    /// Each transcript's words as a phone sees them, keyed by path (T-497).
    words: RefCell<HashMap<String, Words>>,
    /// Transcript pages being read off the writer thread (T-626).
    reading: usize,
}
/// The most transcript pages read at once (T-626): a phone asks one at a
/// time, so this is several phones, and a further ask is refused.
const READING_MAX: usize = 4;
/// An agent's step and latest reply line, read from its transcript, and the
/// file's length and mtime when read: a browser asks for the board every two
/// seconds, and a transcript is re-read only when it changed.
struct Words {
    len: u64,
    mtime_ms: u64,
    doing: Option<String>,
    said: Option<String>,
    /// The read both came from; the next one resumes there (T-755).
    read: crate::agents::PreviewRead,
}
impl Control {
    pub(super) fn new(tx: Sender<Msg>) -> Self {
        Self {
            tx,
            jobs: None,
            generation: 0,
            retired: None,
            stored: None,
            barred: false,
            keys: None,
            identity: None,
            invite: None,
            peers: HashMap::new(),
            high: HashMap::new(),
            receipts: HashMap::new(),
            pending: HashMap::new(),
            starts: HashMap::new(),
            permissions: HashMap::new(),
            codex_asks: HashMap::new(),
            phases: HashMap::new(),
            suppress_awareness: false,
            dialogs: HashMap::new(),
            dialog_deliveries: HashMap::new(),
            dialog_edges: HashMap::new(),
            dialogs_read: HashMap::new(),
            incarnation: ObjectId::random(),
            origin: String::new(),
            online: false,
            mail: false,
            shelf_relay: false,
            shelf: Default::default(),
            error: None,
            retry: Instant::now(),
            dirty: false,
            words: RefCell::new(HashMap::new()),
            reading: 0,
        }
    }
    pub(super) fn send(&mut self, wire: Wire) {
        if self.jobs.as_ref().is_some_and(|tx| tx.try_send(wire).is_err()) {
            self.jobs = None;
            self.online = false;
            self.peers.clear();
            self.error = Some("connection interrupted".into());
        }
    }
    fn answer(&mut self, peer: &str, id: u64, reply: Reply) {
        let bytes = measure(&reply);
        self.answer_measured(peer, id, reply, bytes);
    }
    /// `answer` for a reply its builder measured (T-688): the board answer
    /// is measured as it is trimmed to fit, and not again here.
    fn answer_measured(&mut self, peer: &str, id: u64, reply: Reply, bytes: Option<usize>) {
        if let (Some(p), Some(keys)) = (self.peers.get_mut(peer), self.keys.as_ref()) {
            let reply = if bytes.is_none_or(|n| n > 48 * 1024) {
                Reply::Rejected { message: "response exceeds the browser preview limit".into() }
            } else {
                reply
            };
            let Ok(value) = serde_json::to_value(Answer { id, reply }) else { return };
            if let Ok(record) = p.channel.seal(keys, value) {
                self.send(Wire::Packet { peer: peer.into(), record });
            }
        }
    }
    fn remember(&mut self, grant: BoardId, id: u64, reply: Reply) {
        // A start on its way keeps its receipt (T-498): it is the oldest
        // id, and a browser asking every two seconds passes 128 in a minute.
        let held: Vec<u64> =
            self.starts.values().filter(|w| w.grant == grant).map(|w| w.command).collect();
        let receipts = self.receipts.entry(grant).or_default();
        receipts.insert(id, reply);
        while receipts.len() > 128 {
            let Some(oldest) = receipts.keys().copied().find(|k| !held.contains(k)) else { break };
            receipts.remove(&oldest);
        }
    }
}
impl Daemon {
    pub(super) fn control_start(&mut self) {
        self.control.retired = std::fs::read(self.paths.state_dir.join(RETIRED_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        match std::fs::read(self.paths.state_dir.join("mesophon.json")) {
            Ok(bytes) => match serde_json::from_slice::<Stored>(&bytes) {
                Ok(s)
                    if (s.schema == 1 || (s.schema == 2 && s.shelf_off))
                        && s.grants.len() <= 32
                        && s.grants.iter().all(|g| g.device == g.public.id()) =>
                {
                    self.control.stored = Some(s)
                }
                _ => {
                    self.control.barred = true;
                    self.control.error =
                        Some("Mesophon state is unreadable or newer than this build".into());
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                self.control.barred = true;
                self.control.error = Some("could not read Mesophon state".into());
            }
        }
    }
    /// Record, or with `None` clear, the board the relay must hear is empty.
    fn control_retire(&mut self, retired: Option<Retired>) -> anyhow::Result<()> {
        let path = self.paths.state_dir.join(RETIRED_FILE);
        match &retired {
            Some(r) => store::write_atomic(&path, &serde_json::to_string(r)?, 0o600)?,
            None => match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            },
        }
        self.control.retired = retired;
        Ok(())
    }
    pub(super) fn control_save(&self, s: &Stored) -> anyhow::Result<()> {
        store::write_atomic(
            &self.paths.state_dir.join("mesophon.json"),
            &serde_json::to_string(s)?,
            0o600,
        )
    }
    pub(super) fn control_info(&self) -> Info {
        Info {
            enabled: self.control.stored.is_some() || self.control.barred,
            connected: self.control.online,
            origin: self.control.origin.clone(),
            code: self
                .control
                .invite
                .as_ref()
                .filter(|i| Instant::now() < i.expires)
                .map(|i| i.code.encode()),
            error: self.control.error.clone(),
            devices: self
                .control
                .stored
                .as_ref()
                .map(|s| {
                    s.grants
                        .iter()
                        .map(|g| api::Device { grant: g.id.to_hex(), name: g.name.clone() })
                        .collect()
                })
                .unwrap_or_default(),
            shelf: self.control.stored.as_ref().map(|s| !s.shelf_off),
        }
    }
    pub(super) fn control_local(&mut self, action: LocalAction) -> Response {
        let fail = |m: &str| Response::Err { message: m.into() };
        if self.control.barred && !matches!(action, LocalAction::Disable) {
            return Response::Mesophon { info: self.control_info() };
        }
        match action {
            LocalAction::Status => return Response::Mesophon { info: self.control_info() },
            LocalAction::Enable => {
                let Some(device) = self.team.device.as_ref().filter(|d| d.credential.is_some())
                else {
                    return fail("sign in to a relay first");
                };
                let Some(keys) = device.keys() else {
                    return fail("device identity unavailable");
                };
                if self.control.stored.is_none() {
                    // A retirement the relay has not heard yet is finished by
                    // this board's first publish, which lists no device.
                    let board = self
                        .control
                        .retired
                        .filter(|r| r.host == keys.id())
                        .map_or_else(BoardId::random, |r| r.board);
                    let s = Stored {
                        schema: 1,
                        board,
                        host: keys.id(),
                        grants: Vec::new(),
                        filed: Vec::new(),
                        noted: Vec::new(),
                        shelf_off: false,
                    };
                    if self.control_save(&s).is_err() {
                        return fail("could not save Mesophon state");
                    }
                    self.control.stored = Some(s);
                }
                self.control.retry = Instant::now();
                self.control_tick();
            }
            LocalAction::Disable => {
                // The relay forgets this board's browsers, their shelves
                // included, only once it hears the board has none (T-698):
                // until it says so, the board stays retired here.
                if let Some(s) = &self.control.stored {
                    let retired = Retired { board: s.board, host: s.host };
                    if self.control_retire(Some(retired)).is_err() {
                        return fail("could not disable Mesophon");
                    }
                }
                match std::fs::remove_file(self.paths.state_dir.join("mesophon.json")) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => return fail("could not disable Mesophon"),
                }
                self.control_revoke_all();
                // A board with no devices is one the relay keeps nothing
                // for: its browsers' shelves go with the list (T-698).
                if let Some(board) = self.control.stored.as_ref().map(|s| s.board) {
                    self.control.send(Wire::Host {
                        board,
                        devices: Vec::new(),
                        invites: Vec::new(),
                    });
                }
                self.control.shelf.forget();
                self.control.stored = None;
                self.control.barred = false;
                self.control.high.clear();
                self.control.receipts.clear();
                self.control.starts.clear();
                self.control.error = None;
                self.control.origin.clear();
                self.control.invite = None;
                self.control.jobs = None;
                self.control.online = false;
                self.control.mail = false;
                self.control.shelf_relay = false;
                self.control.generation += 1;
                // The retirement's own connection, at the next tick.
                self.control.retry = Instant::now();
            }
            LocalAction::Shelf { on } => {
                if let Err(message) = self.shelf_set(on) {
                    return fail(message);
                }
            }
            LocalAction::Pair => {
                if !self.control.online {
                    return fail("connect Mesophon to a compatible relay first");
                }
                if self.control.stored.as_ref().is_none_or(|s| s.grants.len() >= 32) {
                    return fail("paired-device limit reached");
                }
                let Some(keys) = self.control.keys.as_ref() else {
                    return fail("identity unavailable");
                };
                self.control.invite = Some(Invite {
                    code: InviteCode::mint(&keys.public()),
                    expires: Instant::now() + Duration::from_secs(600),
                });
                self.control_publish();
            }
            LocalAction::Revoke { grant } => {
                let Some(mut s) = self.control.stored.clone() else {
                    return fail("Mesophon is disabled");
                };
                let Some(id) = BoardId::parse(&grant) else {
                    return fail("invalid grant");
                };
                s.grants.retain(|g| g.id != id);
                if self.control_save(&s).is_err() {
                    return fail("could not save revocation");
                }
                self.control.stored = Some(s);
                self.control_revoke(id);
                self.control.high.remove(&id);
                self.control.receipts.remove(&id);
                self.control_publish();
            }
        }
        self.broadcast();
        Response::Mesophon { info: self.control_info() }
    }
    fn control_publish(&mut self) {
        if let Some(s) = &self.control.stored {
            let wire = Wire::Host {
                board: s.board,
                devices: s.grants.iter().map(|g| g.device).collect(),
                invites: self
                    .control
                    .invite
                    .as_ref()
                    .map(|i| vec![hex::encode(&i.code.secret_hash())])
                    .unwrap_or_default(),
            };
            self.control.send(wire);
        }
    }
    pub(super) fn control_changed(&mut self) {
        self.control.dirty = true;
        self.control.shelf.dirty = true;
        self.control_awareness();
    }
    pub(super) fn control_tick(&mut self) {
        self.control_expire_permissions();
        self.control_recover_dialogs();
        self.control_deliver_dialogs();
        if self.control.stored.is_none() && self.control.retired.is_none() {
            return;
        }
        let pending: Vec<_> = self.control.pending.keys().copied().collect();
        for id in pending {
            self.control_delivery_allowed(id);
        }
        self.control_follow_starts();
        let identity = self
            .team
            .device
            .as_ref()
            .and_then(|d| d.credential.as_ref())
            .map(|c| hex::encode(&c.hash()));
        if identity != self.control.identity {
            self.control_revoke_all();
            self.control.jobs = None;
            self.control.online = false;
            self.control.generation += 1;
            self.control.identity = identity.clone();
            self.control.invite = None;
        }
        if identity.is_none() {
            return;
        }
        if self.control.invite.as_ref().is_some_and(|i| Instant::now() >= i.expires) {
            self.control.invite = None;
            self.control_publish();
        }
        if self.control.jobs.is_none() && Instant::now() >= self.control.retry {
            let Some(device) = self.team.device.clone() else {
                return;
            };
            let Some(keys) = device.keys() else {
                return;
            };
            // A retirement another identity made is not this one's to finish:
            // the relay takes a board's list from its host alone.
            if self.control.stored.is_none()
                && self.control.retired.is_some_and(|r| r.host != keys.id())
            {
                let _ = self.control_retire(None);
                return;
            }
            if self.control.stored.as_ref().is_some_and(|s| s.host != keys.id()) {
                self.control.error = Some(
                    "this board was enabled by a different identity; disable and pair again".into(),
                );
                return;
            }
            self.control.keys = Some(keys);
            self.control.generation += 1;
            let generation = self.control.generation;
            let tx = self.control.tx.clone();
            self.control.jobs = Some(control_io::spawn(device, move |e| {
                let (ack, done) = std::sync::mpsc::channel();
                tx.send(Msg::Control(generation, e, ack)).is_ok()
                    && done.recv_timeout(Duration::from_secs(5)).is_ok()
            }));
            self.control.retry = Instant::now() + Duration::from_secs(5);
        }
        if self.control.dirty && self.ticks.is_multiple_of(4) {
            self.control.dirty = false;
            let peers: Vec<_> = self
                .control
                .peers
                .iter()
                .filter(|(_, p)| p.subscribed)
                .map(|(id, _)| id.clone())
                .collect();
            for peer in peers {
                self.control.answer(&peer, 0, Reply::Changed);
            }
        }
        self.shelf_tick();
    }
    pub(super) fn on_control(&mut self, generation: u64, event: NetEvent) {
        if generation != self.control.generation {
            return;
        }
        // Disabled with a retirement owed (T-698): the connection exists to
        // publish the board with no devices, and ends once the relay took it.
        if self.control.stored.is_none() {
            let Some(r) = self.control.retired else { return };
            match event {
                NetEvent::Online(..) => {
                    self.control.send(Wire::Host {
                        board: r.board,
                        devices: Vec::new(),
                        invites: Vec::new(),
                    });
                }
                NetEvent::Frame(Wire::Published) => {
                    if self.control_retire(None).is_ok() {
                        self.control.jobs = None;
                        self.control.generation += 1;
                    }
                }
                NetEvent::Offline => {
                    self.control.jobs = None;
                    self.control.retry = Instant::now() + Duration::from_secs(5);
                }
                NetEvent::Frame(_) => {}
            }
            return;
        }
        let announce = matches!(
            &event,
            NetEvent::Online(..)
                | NetEvent::Offline
                | NetEvent::Frame(Wire::Peer { .. } | Wire::Gone { .. } | Wire::Error { .. })
        );
        match event {
            NetEvent::Online(origin, mail, shelf) => {
                self.control.origin = origin;
                self.control.online = true;
                self.control.mail = mail;
                self.control.shelf_relay = shelf;
                self.control.shelf.forget();
                self.control.error = None;
                self.control_publish();
                // Turned off while away (T-698): the relay may still hold
                // what was there, so every connection empties it again.
                if shelf {
                    if let Some(s) = self.control.stored.as_ref().filter(|s| s.shelf_off) {
                        let empty: Vec<Wire> = s
                            .grants
                            .iter()
                            .map(|g| Wire::Shelve {
                                device: g.device,
                                keep: Vec::new(),
                                item: None,
                            })
                            .collect();
                        for wire in empty {
                            self.control.send(wire);
                        }
                    }
                }
                // After the publish, which is what makes this the host.
                if let Some(board) = self.control.stored.as_ref().map(|s| s.board).filter(|_| mail)
                {
                    self.control.send(Wire::Collect { board });
                }
            }
            NetEvent::Frame(Wire::Mail { items }) => self.control_mail(items),
            NetEvent::Offline => {
                self.control.online = false;
                self.control.mail = false;
                self.control.shelf_relay = false;
                self.control.shelf.forget();
                self.control.jobs = None;
                self.control.peers.clear();
                self.control.error = Some("relay unavailable or does not support Mesophon".into());
                self.control.retry = Instant::now() + Duration::from_secs(5);
            }
            NetEvent::Frame(Wire::Peer { peer, device, name, public, proof, challenge }) => {
                self.control_peer(peer, device, name, public, proof, challenge)
            }
            NetEvent::Frame(Wire::Packet { peer, record }) => {
                let value =
                    self.control.peers.get_mut(&peer).and_then(|p| p.channel.open(&record).ok());
                if let Some(command) =
                    value.and_then(|v| serde_json::from_value::<api::Command>(v).ok())
                {
                    self.control_command(&peer, command);
                } else {
                    self.control.peers.remove(&peer);
                    self.control.send(Wire::Close { peer });
                }
            }
            NetEvent::Frame(Wire::Gone { peer }) => {
                self.control.peers.remove(&peer);
            }
            // The board's list reached the relay: a retirement this board
            // took back is finished (T-698).
            NetEvent::Frame(Wire::Published) => {
                let board = self.control.stored.as_ref().map(|s| s.board);
                if self.control.retired.is_some_and(|r| Some(r.board) == board) {
                    let _ = self.control_retire(None);
                }
            }
            // The relay refused the phone's mail for this Mac's lapsed grant
            // (T-522): the same edge as a refused board write, so a Mac that
            // writes no shared board still renews by itself.
            NetEvent::Frame(Wire::Error { code }) if code == control::LAPSED => {
                self.team_on_lapse()
            }
            NetEvent::Frame(_) => {}
        }
        self.control_expire_permissions();
        if announce {
            self.broadcast();
        }
    }
    fn control_peer(
        &mut self,
        peer: String,
        device: DeviceId,
        name: String,
        public: DevicePublic,
        proof: Option<String>,
        challenge: ObjectId,
    ) {
        let grant = self.control_accept(device, name, public, proof);
        let Some(grant) = grant else {
            self.control.send(Wire::Close { peer });
            return;
        };
        let (Some(s), Some(keys)) = (&self.control.stored, &self.control.keys) else {
            return;
        };
        let next = self.control.high.get(&grant.id).copied().unwrap_or(0) + 1;
        let ceiling = next + 1_000_000;
        self.control.high.insert(grant.id, ceiling);
        let ready = serde_json::to_value(Answer {
            id: 0,
            reply: Reply::Ready {
                incarnation: self.control.incarnation.to_hex(),
                next,
                features: [
                    "permission",
                    "dialog",
                    "awareness",
                    "create",
                    "start",
                    // The card edits (T-530).
                    "rename",
                    "move",
                    "tag",
                    // Where a ticket's next agent works (T-642).
                    "workspace",
                    // A ticket into the archive and back (T-737).
                    "archive",
                    // Notes read and written from the ticket page (T-532).
                    "notes",
                    // A batch or several-choice question answered whole,
                    // `DialogAnswer::Answers` (T-571).
                    "dialog_multi",
                    // Pictures in notes, `Upload` and `WriteNote`'s
                    // `uploads` (T-629).
                    "pictures",
                    // Pictures in a filed ticket's description, `Create`'s
                    // `uploads` and an `Upload` with no ticket (T-670).
                    "filed_pictures",
                    // Pictures with the words to an agent, `Prompt`'s and
                    // `Start`'s `uploads` (T-853).
                    "ask_pictures",
                    // The conversation read from its transcript (T-626).
                    "transcript",
                    // A tier picked for a start or a prompt (T-643).
                    "tiers",
                    // Tickets for an away host wait at the relay (T-497).
                    if self.control.mail { "mailbox" } else { "" },
                    // Words for an agent wait there too (T-881).
                    if self.control.mail { "mail_prompts" } else { "" },
                ]
                .into_iter()
                .filter(|f| !f.is_empty())
                .map(String::from)
                .collect(),
            },
        })
        .unwrap_or_default();
        if let Ok((channel, welcome)) = Channel::host(
            s.board,
            grant.id,
            self.control.incarnation,
            public,
            keys,
            ready,
            challenge,
        ) {
            self.control.peers.insert(
                peer.clone(),
                Peer {
                    grant: grant.id,
                    device,
                    channel,
                    subscribed: false,
                    foreground: None,
                    floor: next,
                    ceiling,
                    high: next - 1,
                },
            );
            self.control.send(Wire::Welcome { peer, welcome: Box::new(welcome) });
        }
    }
    fn control_accept(
        &mut self,
        device: DeviceId,
        name: String,
        public: DevicePublic,
        proof: Option<String>,
    ) -> Option<Grant> {
        if device != public.id() || self.control.peers.len() >= 32 {
            return None;
        }
        let mut s = self.control.stored.clone()?;
        if let Some(proof) = proof {
            let invite = self.control.invite.as_ref()?;
            if !invite.accepts(&public, &proof, Instant::now()) || s.grants.len() >= 32 {
                return None;
            }
            let g = Grant {
                id: BoardId::random(),
                device,
                public,
                name: mesimon_core::text::scrub_text(&name).chars().take(64).collect(),
            };
            s.grants.push(g.clone());
            if self.control_save(&s).is_err() {
                return None;
            }
            self.control.stored = Some(s);
            self.control.invite = None;
            self.control_publish();
            Some(g)
        } else {
            s.grants.iter().find(|g| g.device == device && g.public == public).cloned()
        }
    }
    fn control_command(&mut self, peer: &str, command: api::Command) {
        let Some(p) = self.control.peers.get(peer) else {
            return;
        };
        let (grant, device) = (p.grant, p.device);
        if !self.control_granted(grant, device) {
            self.control.answer(peer, command.id, Reply::Revoked);
            return;
        }
        if command.incarnation != self.control.incarnation.to_hex()
            || command.id == 0
            || command.id > 9_007_199_254_740_000
        {
            self.control.answer(
                peer,
                command.id,
                Reply::Rejected { message: "stale or invalid command".into() },
            );
            return;
        }
        if command.id < p.floor || command.id > p.ceiling {
            self.control.answer(
                peer,
                command.id,
                Reply::Rejected { message: "command belongs to another connection".into() },
            );
            return;
        }
        if command.id <= p.high {
            let reply = self
                .control
                .receipts
                .get(&grant)
                .and_then(|r| r.get(&command.id))
                .cloned()
                .unwrap_or(Reply::delivery("unknown"));
            self.control.answer(peer, command.id, reply);
            return;
        }
        if let Some(p) = self.control.peers.get_mut(peer) {
            p.high = command.id;
        }
        let by = Principal::Paired { device: device.to_hex(), grant: grant.to_hex() };
        // The board answer comes measured (T-688); every other is measured
        // once on its way out.
        let mut measured: Option<Option<usize>> = None;
        let reply = match command.request {
            api::Request::Foreground { ticket } => {
                if authorize(&by, &Action::Read, &Resource::Board).denied() {
                    return self.control.answer(peer, command.id, Reply::Revoked);
                }
                if ticket.as_ref().is_some_and(|id| {
                    ulid::Ulid::from_string(id).ok().and_then(|id| self.board.ticket(id)).is_none()
                }) {
                    Reply::Rejected { message: "ticket unavailable".into() }
                } else {
                    if let Some(p) = self.control.peers.get_mut(peer) {
                        p.foreground = ticket.map(|t| (t, Instant::now()));
                    }
                    Reply::delivery("observed")
                }
            }
            api::Request::Dialog { ticket, session, request, response } => self
                .control_dialog_answer(
                    &by, grant, device, command.id, &ticket, &session, &request, response,
                ),
            api::Request::Permission { ticket, session, request, decision } => self
                .control_permission_answer(
                    &by, peer, command.id, &ticket, &session, &request, decision,
                ),
            api::Request::Snapshot => {
                if let Some(p) = self.control.peers.get_mut(peer) {
                    p.subscribed = true;
                }
                if authorize(&by, &Action::Read, &Resource::Board).denied() {
                    Reply::Revoked
                } else {
                    let (reply, bytes) = self.control_board();
                    measured = Some(bytes);
                    reply
                }
            }
            api::Request::Preview { ticket, session } => {
                self.control_preview(&by, &ticket, &session)
            }
            api::Request::Transcript { ticket, session, before, after, conversation, limit } => {
                let ask = crate::agents::transcript::Ask {
                    before,
                    after,
                    limit: limit.map(usize::from),
                    ..Default::default()
                };
                // A page is read off this thread and answered when it lands
                // (`control_transcript_read`); a refusal is answered here.
                match self.control_transcript(
                    &by,
                    (peer, grant, command.id),
                    (&ticket, &session),
                    ask,
                    conversation,
                ) {
                    Some(reply) => reply,
                    None => return,
                }
            }
            api::Request::Prompt { ticket, session, text, queued, tier, uploads } => {
                let send = |d: &mut Self, text: String| {
                    d.control_prompt(
                        &by,
                        (grant, device, command.id),
                        (&ticket, &session),
                        text,
                        queued,
                        tier,
                    )
                };
                self.control_pictured(grant, &ticket, text, &uploads, send)
            }
            api::Request::SendNow { ticket, session } => {
                self.control_queue_action(&by, &ticket, &session, Some((grant, device, command.id)))
            }
            api::Request::TakeBack { ticket, session } => {
                self.control_queue_action(&by, &ticket, &session, None)
            }
            api::Request::Status { command } => self
                .control
                .receipts
                .get(&grant)
                .and_then(|r| r.get(&command))
                .cloned()
                .unwrap_or(Reply::delivery("unknown")),
            api::Request::Create { title, description, column, tags, uploads } => {
                let pictures = (grant, uploads);
                self.control_create_pictured(&by, title, description, column, &tags, pictures)
            }
            api::Request::Start { ticket, prompt, tier, uploads } => {
                let start = |d: &mut Self, text: String| {
                    let prompt = Some(text).filter(|t| !t.is_empty());
                    d.control_start_agent(&by, (grant, device, command.id), &ticket, prompt, tier)
                };
                let text = prompt.unwrap_or_default();
                self.control_pictured(grant, &ticket, text, &uploads, start)
            }
            api::Request::Rename { ticket, title } => self.control_rename(&by, &ticket, &title),
            api::Request::Move { ticket, column, before } => {
                self.control_move(&by, &ticket, &column, before.as_deref())
            }
            api::Request::Tag { ticket, group, name } => {
                self.control_tag(&by, &ticket, group, name)
            }
            api::Request::Workspace { ticket, worktree } => {
                self.control_workspace(&by, &ticket, worktree)
            }
            api::Request::Archive { ticket, restore } => {
                self.control_archive(&by, &ticket, restore)
            }
            api::Request::Notes { ticket } => self.control_notes(&by, &ticket),
            api::Request::Note { ticket, note } => self.control_note(&by, &ticket, &note),
            api::Request::WriteNote { ticket, note, text, rev, uploads } => {
                if uploads.is_empty() {
                    self.control_write_note(&by, &ticket, note.as_deref(), text, rev)
                } else {
                    let pictures = (grant, uploads);
                    self.control_write_pictured(&by, &ticket, note.as_deref(), text, rev, pictures)
                }
            }
            api::Request::Upload { ticket, upload, offset, data, complete } => {
                let ticket = ticket.as_deref();
                self.control_upload(&by, grant, ticket, upload.as_deref(), offset, &data, complete)
            }
            api::Request::TellAgent { ticket, note } => {
                self.control_tell_agent(&by, &ticket, &note)
            }
        };
        self.control.remember(grant, command.id, reply.clone());
        let bytes = measured.unwrap_or_else(|| measure(&reply));
        self.control.answer_measured(peer, command.id, reply, bytes);
    }
    pub(super) fn control_observe_dialog(&mut self, id: uuid::Uuid, frame: &HookFrame) {
        if frame.payload.get("agent_id").is_some() {
            return;
        }
        // The hook edge is the only receipt that a dialog took an answer
        // (T-567), and the dialog is gone once it comes.
        let edge = self
            .control
            .dialogs
            .get(&id)
            .and_then(|d| dialog_edge(frame, d).map(|edge| (d.request.clone(), edge)));
        if let Some(edge) = edge {
            self.control.dialogs.remove(&id);
            self.control.dialog_edges.insert(id, edge);
        }
        if self.control.dialogs.get(&id).is_some_and(|d| dialog_dropped(frame, d)) {
            self.control.dialogs.remove(&id);
        }
        if !matches!(frame.event.as_str(), "PreToolUse" | "PermissionRequest") {
            return;
        }
        let Some(tool) = frame.payload["tool_name"].as_str() else { return };
        let content = dialog_content(tool, &frame.payload["tool_input"]);
        // An id-less `PermissionRequest` repeating the dialog projected for
        // the seat, the same tool's same content: it keeps that dialog's
        // identity, its request below and its named call here (round 11).
        let repeat = frame.event == "PermissionRequest"
            && frame.payload["tool_use_id"].is_null()
            && content
                .as_ref()
                .is_some_and(|c| self.control.dialogs.get(&id).is_some_and(|d| d.content == *c));
        // The call the record is held on (T-957 rounds 9, 10), observed from
        // the hook whether or not its content projects: a dialog too big to
        // draw still names its call, and a replacement that gives no id
        // names none, never the call before it; only a repeat keeps it.
        if matches!(tool, "ExitPlanMode" | "AskUserQuestion") {
            let call =
                frame.payload["tool_use_id"].as_str().filter(|s| s.len() <= 256).map(String::from);
            // The prompts heard when it was named (round 4): an Esc read for
            // it after another is heard ended an earlier turn.
            if let Some(call) = &call {
                if self.dialog_heard.get(&id).is_none_or(|(named, _)| named != call) {
                    let heard = self.prompts_heard.get(&id).copied().unwrap_or(0);
                    self.dialog_heard.insert(id, (call.clone(), heard));
                }
            }
            if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
                if !repeat && rec.dialog_call != call {
                    rec.dialog_call = call;
                    self.persist_sessions();
                }
            }
        }
        let Some(content) = content else { return };
        // PermissionRequest may repeat the same PreToolUse payload. Keep its
        // identity stable while a browser is selecting that exact dialog.
        if repeat {
            return;
        }
        let request = frame.payload["tool_use_id"]
            .as_str()
            .filter(|s| s.len() <= 256)
            .map(str::to_string)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        if self
            .control
            .dialogs
            .get(&id)
            .is_some_and(|d| d.request == request && d.content == content)
        {
            return;
        }
        // A new dialog on a record already stopped on that kind of dialog
        // has no state edge to wake the crown with (T-943): a plan sent
        // back is replanned and stopped on again inside the attention
        // settle, so the record never left `RequiresAction{Plan}`. This
        // runs before the frame reaches the machine, so a record anywhere
        // else gets its edge and its wake there, and no wake twice.
        let reason = match content {
            api::DialogContent::Plan { .. } => Some(Reason::Plan),
            api::DialogContent::Questions { .. } => Some(Reason::Question),
            api::DialogContent::Trust { .. } => None,
        };
        let restop = reason
            .and_then(|reason| {
                let stopped = SessionState::RequiresAction { reason };
                self.board.sessions.iter().find(|s| s.id == id && s.state == stopped)
            })
            .and_then(|s| {
                super::crownwake::stop_cause(self.board.crown_mode, s.kind, &s.state)
                    .map(|cause| (s.ticket, cause))
            });
        // A dialog projected with no call id of its own is noted under the
        // request it was given (round 4).
        if self.dialog_heard.get(&id).is_none_or(|(named, _)| *named != request) {
            let heard = self.prompts_heard.get(&id).copied().unwrap_or(0);
            self.dialog_heard.insert(id, (request.clone(), heard));
        }
        self.control.dialogs.insert(id, api::Dialog { request, content });
        self.control_changed();
        if let Some((ticket, cause)) = restop {
            self.note_crown_wake(ticket, cause, None, None);
        }
    }

    /// A plan sent back whose dialog the window's screen read saw gone
    /// leaves needs-you with it (T-955). No frame says so on the hook set or
    /// the classic mod: the plan's rejection fires no hook (T-447) and
    /// neither road sends an ordinary tool's `PreToolUse`. So the record's
    /// machine is given the refusal road's signal, `ToolStarted`, as a
    /// synthetic departure of the dialog, never as a tool start anyone saw
    /// (source `plan_sent_back`). The machine's leave settle stands, so a
    /// replan inside it keeps the plan. Only while no dialog is projected
    /// for the seat and its record still stands on a plan.
    fn sent_back_left(&mut self, id: uuid::Uuid) {
        use mesimon_core::board::Reason;
        let planned = SessionState::RequiresAction { reason: Reason::Plan };
        if self.control.dialogs.contains_key(&id)
            || !self.board.sessions.iter().any(|s| s.id == id && s.state == planned)
        {
            return;
        }
        let signal = Signal::ToolStarted { nested: false };
        let Some(change) = self.observe_signal(id, &signal, now_ms(), "plan_sent_back") else {
            return;
        };
        if self.apply_change(id, &change, None, Some("plan_sent_back")) {
            self.persist_and_notify();
        }
    }

    /// A Claude seat stopped on a plan or a question with no dialog to show
    /// for it (T-933): a restart empties `dialogs`, which only the hook
    /// stream filled, and the record's `RequiresAction` outlives it, so the
    /// crown read `needs_you` with no request and its `accept_plan` was
    /// refused. The transcript still holds the call with its `tool_use` id,
    /// the request the hook stream names, and no result after it: that is
    /// the dialog, rebuilt within the same bounds. One read per spell, and
    /// none for a spell the hook stream already gave a dialog, so an answer
    /// that took one is never resurrected; a call the transcript shows
    /// answered or gone rebuilds nothing.
    fn control_recover_dialogs(&mut self) {
        const GIVEN: u64 = u64::MAX;
        use mesimon_core::board::Reason;
        let board = &self.board;
        self.control.dialogs_read.retain(|id, _| board.sessions.iter().any(|s| s.id == *id));
        let due: Vec<_> = self
            .board
            .sessions
            .iter()
            .filter(|s| s.kind == SessionKind::Claude)
            .filter_map(|s| {
                let SessionState::RequiresAction {
                    reason: reason @ (Reason::Plan | Reason::Question),
                } = s.state
                else {
                    return None;
                };
                let spell = s.state_changed_at.unwrap_or_default();
                let read = self.control.dialogs_read.get(&s.id);
                if read == Some(&(spell, GIVEN)) {
                    return None;
                }
                // A record that names its call (T-957 round 9) is read again
                // as its transcript grows, until that call's line is there:
                // a restart can come before the line does.
                let grown = s
                    .dialog_call
                    .as_ref()
                    .and(s.transcript_path.as_deref())
                    .and_then(|p| std::fs::metadata(p).ok())
                    .map_or(0, |m| m.len());
                let key = (spell, grown);
                (read != Some(&key))
                    .then(|| (s.id, reason, key, s.transcript_path.clone(), s.dialog_call.clone()))
            })
            .collect();
        for (id, reason, key, path, named) in due {
            if self.control.dialogs.contains_key(&id) {
                self.control.dialogs_read.insert(id, (key.0, GIVEN));
                continue;
            }
            self.control.dialogs_read.insert(id, key);
            let Some(open) = path
                .and_then(|p| crate::agents::claude::tail::open_dialog(std::path::Path::new(&p)))
            else {
                continue;
            };
            // Never another call than the one the record names.
            if named.as_ref().is_some_and(|named| *named != open.id) {
                continue;
            }
            use mesimon_core::adopt::TailTool;
            let tool = match (open.tool, reason) {
                (TailTool::ExitPlanMode, Reason::Plan) => "ExitPlanMode",
                (TailTool::AskUserQuestion, Reason::Question) => "AskUserQuestion",
                _ => continue,
            };
            if open.id.len() > 256 {
                continue;
            }
            let Some(content) = dialog_content(tool, &open.input) else { continue };
            self.control.dialogs.insert(id, api::Dialog { request: open.id, content });
            self.control_changed();
        }
    }

    /// The question dialog a session stopped on, for the crown's
    /// `get_ticket` (T-566): `(request, questions)` off the projection
    /// Remote Control draws, which `control_observe_dialog` keeps from every
    /// hook frame whether or not a phone is paired. `None` where the hook
    /// stream carried no question dialog for the session.
    pub(super) fn control_questions(
        &self,
        session: uuid::Uuid,
    ) -> Option<(&str, &[api::Question])> {
        let dialog = self.control.dialogs.get(&session)?;
        match &dialog.content {
            api::DialogContent::Questions { questions } => Some((&dialog.request, questions)),
            api::DialogContent::Plan { .. } | api::DialogContent::Trust { .. } => None,
        }
    }

    /// The request of the dialog projected for `session`, any kind.
    pub(super) fn control_dialog_request(&self, session: uuid::Uuid) -> Option<&str> {
        self.control.dialogs.get(&session).map(|d| d.request.as_str())
    }

    /// The plan a stopped agent's dialog shows (T-582), as the hook stream
    /// carried it: its request and markdown.
    pub(super) fn control_plan(&self, session: uuid::Uuid) -> Option<(&str, &str)> {
        let dialog = self.control.dialogs.get(&session)?;
        match &dialog.content {
            api::DialogContent::Plan { markdown } => Some((&dialog.request, markdown)),
            api::DialogContent::Questions { .. } | api::DialogContent::Trust { .. } => None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn control_dialog_answer(
        &mut self,
        by: &Principal,
        grant: BoardId,
        device: DeviceId,
        command: u64,
        ticket: &str,
        session: &str,
        request: &str,
        response: api::DialogAnswer,
    ) -> Reply {
        let Some(id) = self.control_target(ticket, session) else {
            return Reply::Rejected { message: "session changed".into() };
        };
        if authorize(by, &Action::PromptExisting, &Resource::Session { id }).denied() {
            return Reply::Revoked;
        }
        // A person's answer wins a race with the crown's (T-569): a crown
        // answer still walking its keys gives way, and the person's walks
        // from wherever the cursor now is, the screen read at every step.
        if self.control.dialog_deliveries.get(&id).is_some_and(|d| {
            matches!(d.by, Deliverer::Crown { .. }) && d.confirm.is_none() && d.request == request
        }) {
            if let Some(crown) = self.control.dialog_deliveries.remove(&id) {
                let reason = if crown.steps > 0 {
                    "a_person_answered; cursor moved"
                } else {
                    "a_person_answered"
                };
                let screen = self.dialog_screen(id);
                self.control_settle_dialog(id, crown, "unknown", Some(reason.into()), &screen);
            }
        }
        if self.control.dialog_deliveries.contains_key(&id) || self.control_input_in_flight(id) {
            return Reply::Rejected { message: "input is already pending".into() };
        }
        let Some(dialog) = self.control.dialogs.get(&id).filter(|d| d.request == request) else {
            return Reply::Rejected { message: "dialog changed; check the pane".into() };
        };
        let codex = codex_shapes(self.session_kind(id), dialog);
        if dialog_plan(dialog, &response).is_none() || (codex && !codex_takes(dialog, &response)) {
            return Reply::Rejected {
                message: "this dialog shape is not verified; answer in the pane".into(),
            };
        }
        let (max_steps, walk) = walk_budget(dialog);
        let Ok(ticket) = ulid::Ulid::from_string(ticket) else {
            return Reply::Rejected { message: "invalid ticket".into() };
        };
        self.control.dialog_deliveries.insert(
            id,
            DialogDelivery {
                by: Deliverer::Phone { grant, device, command },
                ticket,
                request: request.into(),
                answer: DeliveryAnswer::Dialog(response),
                deadline: Instant::now() + walk,
                next: Instant::now(),
                steps: 0,
                max_steps,
                walk: Walk::default(),
                confirm: None,
                by_mod: None,
            },
        );
        Reply::delivery("awaiting_delivery")
    }

    /// Walk each queued dialog answer one key per pass (T-567), and a Codex
    /// approval's to its `Yes, proceed` or `No, …` row (T-846). The last key
    /// waits for the hook edge, or for the approval to leave the snapshot:
    /// `answered` only when it comes, `input_sent` past `DIALOG_CONFIRM`, and
    /// `unknown` with its reason when no key could be chosen. Keys already in
    /// are never undone.
    fn control_deliver_dialogs(&mut self) {
        use mesimon_backend_tmux::DialogKey;
        let board = &self.board;
        self.control.dialog_edges.retain(|id, _| board.sessions.iter().any(|s| s.id == *id));
        self.crown_answer_lines.retain(|id, _| board.sessions.iter().any(|s| s.id == *id));
        let ready: Vec<_> = self
            .control
            .dialog_deliveries
            .iter()
            .filter(|(_, p)| Instant::now() >= p.next)
            .map(|(id, _)| *id)
            .collect();
        for id in ready {
            let Some(mut pending) = self.control.dialog_deliveries.remove(&id) else { continue };
            if let Some(until) = pending.confirm {
                if let DeliveryAnswer::Approval { .. } = pending.answer {
                    if self.delivery_standing(id, &pending).is_none() {
                        self.control_settle_dialog(id, pending, "answered", None, "");
                    } else if Instant::now() >= until {
                        self.control_settle_dialog(id, pending, "input_sent", None, "");
                    } else {
                        pending.next = Instant::now() + Duration::from_millis(250);
                        self.control.dialog_deliveries.insert(id, pending);
                    }
                    continue;
                }
                // A plan sent back (T-943) claims only that its Enter went
                // and the dialog left: any edge that ends it the refusal way
                // (the next tool call, the mod's `declined`), the projection
                // dropped, or at the window's end a screen without it. An
                // Esc or an error leaves it alike, so nothing says the words
                // were read; an accept that ended it is not this answer.
                if let DeliveryAnswer::Revise { .. } = pending.answer {
                    let edge = self.control_dialog_edge(id, &pending.request);
                    let standing = self.delivery_standing(id, &pending).is_some();
                    // What the window's end saw, if it looked: `Some(None)`
                    // is a capture that failed, no observation at all.
                    let mut seen: Option<Option<String>> = None;
                    let due = Instant::now() >= until;
                    let settled = revise_settled(edge, standing, due, || {
                        let read = self.dialog_capture(id);
                        seen = Some(read.clone());
                        read
                    });
                    match settled {
                        Some((status, reason)) => {
                            let reason = reason.map(str::to_string);
                            // Only a screen read that found the composer
                            // shows the dialog gone (T-955); any other read
                            // keeps the projection, as a failed one does.
                            let left = status == "input_sent" && matches!(seen, Some(Some(_)));
                            let screen = match &seen {
                                Some(read) if left => read.as_deref(),
                                Some(_) => None,
                                None => Some(""),
                            };
                            self.control_settle_seen(id, pending, status, reason, screen);
                            if left {
                                self.sent_back_left(id);
                            }
                        }
                        None => {
                            pending.next = Instant::now() + Duration::from_millis(250);
                            self.control.dialog_deliveries.insert(id, pending);
                        }
                    }
                    continue;
                }
                let edge = self.control_dialog_edge(id, &pending.request);
                let reject =
                    matches!(pending.answer, DeliveryAnswer::Dialog(api::DialogAnswer::Reject));
                // An answer the mod carries (T-576) is confirmed by the mod's
                // own report alone: the engine's `PostToolUse`, or a refusal,
                // is the person's answer winning the race in the pane.
                if pending.by_mod.is_some() {
                    match edge {
                        Some(DialogEdge::ModAnswered) => {
                            self.control_settle_dialog(id, pending, "answered", None, "")
                        }
                        Some(DialogEdge::Answered | DialogEdge::Dismissed) => {
                            let reason = Some("a_person_answered".into());
                            self.control_settle_dialog(id, pending, "unknown", reason, "")
                        }
                        None if Instant::now() >= until => {
                            self.control_settle_dialog(id, pending, "input_sent", None, "")
                        }
                        None => {
                            pending.next = Instant::now() + Duration::from_millis(100);
                            self.control.dialog_deliveries.insert(id, pending);
                        }
                    }
                    continue;
                }
                // A trust dialog's leave has no hook of its own on Codex, and
                // on Claude `SessionStart` may follow the composer by a beat
                // (T-844): the seat moving off the dialog is the answer taken.
                let trust_taken = self.trust_dialog(id).is_some() && !self.dialog_waiting(id);
                match edge {
                    Some(DialogEdge::Answered | DialogEdge::ModAnswered) => {
                        self.control_settle_dialog(id, pending, "answered", None, "")
                    }
                    None if trust_taken => {
                        self.control_settle_dialog(id, pending, "answered", None, "")
                    }
                    Some(DialogEdge::Dismissed) if reject => {
                        self.control_settle_dialog(id, pending, "answered", None, "")
                    }
                    Some(DialogEdge::Dismissed) => {
                        self.control_settle_dialog(id, pending, "input_sent", None, "")
                    }
                    None if Instant::now() >= until => {
                        let screen = self.dialog_screen(id);
                        self.control_settle_dialog(id, pending, "input_sent", None, &screen);
                    }
                    None => {
                        pending.next = Instant::now() + Duration::from_millis(250);
                        self.control.dialog_deliveries.insert(id, pending);
                    }
                }
                continue;
            }
            // Why a plan sent back may not take its next key (T-943).
            let mut held = "state_changed";
            let allowed = match &pending.by {
                Deliverer::Phone { grant, device, .. } => {
                    let by = pending.by.principal();
                    let (action, waiting) = match pending.answer {
                        DeliveryAnswer::Dialog(_) | DeliveryAnswer::Revise { .. } => {
                            (Action::PromptExisting, self.dialog_waiting(id))
                        }
                        DeliveryAnswer::Approval { .. } => {
                            (Action::ApproveExisting, self.approval_waiting(id))
                        }
                    };
                    self.control_granted(*grant, *device)
                        && !authorize(&by, &action, &Resource::Session { id }).denied()
                        && self.control_target(&pending.ticket.to_string(), &id.to_string())
                            == Some(id)
                        && waiting
                }
                Deliverer::Crown { crown, session, .. } => match &pending.answer {
                    DeliveryAnswer::Revise { pane, .. } => {
                        match self.crown_revise_held(*crown, *session, pending.ticket, id, pane) {
                            Some(why) => {
                                held = why;
                                false
                            }
                            None => true,
                        }
                    }
                    _ => self.crown_answer_allowed(*crown, *session, pending.ticket, id),
                },
                Deliverer::Board => self.dialog_waiting(id),
            };
            let standing = self.delivery_standing(id, &pending);
            // A session whose mod speaks `answer` takes the whole answer in
            // one frame (T-576): no screen is read, no key is sent, a batch
            // or a several-choice question is answered whole, and the mod
            // returns it in the dialog's place. A refusal stays the Escape
            // key: the mod has no refusal of its own that puts no words of
            // ours in front of the model.
            let mod_answers = standing
                .filter(|_| allowed && Instant::now() < pending.deadline)
                .filter(|_| self.mod_speaks(id, "answer"))
                .and_then(|s| match s {
                    Standing::Dialog(d, response) => Some((d, dialog_plan(d, response))),
                    Standing::Approval(..) | Standing::Revise(..) => None,
                })
                .and_then(|(d, plan)| match (&d.content, plan) {
                    (api::DialogContent::Questions { questions }, Some(Plan::Single(a))) => {
                        Some(mod_answers(questions, std::slice::from_ref(&a)))
                    }
                    (api::DialogContent::Questions { questions }, Some(Plan::Batch(a))) => {
                        Some(mod_answers(questions, &a))
                    }
                    _ => None,
                });
            if let Some(answers) = mod_answers {
                let tool_use_id = pending.request.clone();
                let frame = self.mod_enqueue(id, ModCommand::Answer { tool_use_id, answers });
                pending.by_mod = Some(frame);
                self.feed.board(
                    pending.by.principal().actor(),
                    "answer_by_mod",
                    Some(pending.ticket),
                );
                pending.steps += 1;
                pending.confirm = Some(Instant::now() + DIALOG_CONFIRM);
                pending.next = Instant::now() + Duration::from_millis(100);
                if let Deliverer::Crown { crown, prior, .. } = &mut pending.by {
                    *prior = Some(self.turn_asks.get(&pending.ticket).copied());
                    self.mark_turn(pending.ticket, TurnAsk::Crown(*crown));
                }
                self.control.dialog_deliveries.insert(id, pending);
                continue;
            }
            // A plan sent back tells a capture that failed from an empty
            // screen (T-943): it is no sign of the dialog, gone or there.
            let capture = self.dialog_capture(id);
            let unseen =
                capture.is_none() && matches!(pending.answer, DeliveryAnswer::Revise { .. });
            let screen = capture.unwrap_or_default();
            let kind = self.session_kind(id);
            let sid = id.simple().to_string()[..16].to_string();
            // The crown's keys never go under a person's hands (T-958): a
            // tmux client on the pane, read fresh before each key, ends the
            // walk where it stands, as a plan sent back's does (T-943,
            // `crown_revise_held`, which has read it already).
            let crown_keys = matches!(
                (&pending.by, &pending.answer),
                (Deliverer::Crown { .. }, DeliveryAnswer::Dialog(_))
            );
            let step = match standing {
                None => Err("state_changed"),
                Some(_) if !allowed => Err(held),
                Some(_)
                    if Instant::now() >= pending.deadline || pending.steps >= pending.max_steps =>
                {
                    Err("deadline")
                }
                Some(_) if unseen => Err("pane_unreachable"),
                Some(_) if crown_keys && self.person_at_pane(&sid) => Err(A_PERSON_ATTACHED),
                Some(s) => dialog_step_for(kind, s, &screen, &pending.walk).map_err(Miss::word),
            };
            // A plan sent back expects the cursor where its arrow puts it
            // (T-943): a cursor anywhere else on the next screen is a
            // person's hand, and stops the walk.
            if let (
                DeliveryAnswer::Revise { .. },
                Ok(arrow @ (DialogStep::Up | DialogStep::Down)),
            ) = (&pending.answer, &step)
            {
                pending.walk.at = plan_cursor(&screen).map(|at| match arrow {
                    DialogStep::Up => at.saturating_sub(1),
                    _ => at + 1,
                });
            }
            // `Ok(true)` is the last key: Enter, or Escape for a refusal. A
            // tick is remembered for one key, so the next screen must show it.
            pending.walk.toggled = None;
            let sent = step.and_then(|step| {
                match step {
                    DialogStep::Up => self.backend.dialog_key(&sid, DialogKey::Up).map(|_| false),
                    DialogStep::Down => {
                        self.backend.dialog_key(&sid, DialogKey::Down).map(|_| false)
                    }
                    DialogStep::Toggle { question, row } => {
                        self.backend.dialog_key(&sid, DialogKey::Space).map(|_| {
                            pending.walk.toggled = Some((question, row));
                            false
                        })
                    }
                    DialogStep::Reject => {
                        self.backend.dialog_key(&sid, DialogKey::Escape).map(|_| true)
                    }
                    DialogStep::Next => self.backend.send_enter(&sid).map(|_| false),
                    DialogStep::Submit => self.backend.send_enter(&sid).map(|_| true),
                    DialogStep::Paste { question, text } => {
                        self.backend.paste_input(&sid, &text).map(|_| {
                            pending.walk.pasted.push(question);
                            false
                        })
                    }
                    // No key: the screen has not drawn the last one yet.
                    DialogStep::Wait => Ok(false),
                }
                .map_err(|_| "pane_unreachable")
            });
            match sent {
                Ok(last) => {
                    pending.steps += 1;
                    let pause = if last { 250 } else { 350 };
                    pending.next = Instant::now() + Duration::from_millis(pause);
                    if last {
                        let window = match pending.answer {
                            DeliveryAnswer::Revise { .. } => PLAN_CHANGE_CONFIRM,
                            _ => DIALOG_CONFIRM,
                        };
                        pending.confirm = Some(Instant::now() + window);
                        // The crown's answer is the turn's from here (T-569):
                        // its end wakes the crown with `answered your ask`,
                        // as the turn that took a sent ask does (T-469).
                        if let Deliverer::Crown { crown, prior, .. } = &mut pending.by {
                            *prior = Some(self.turn_asks.get(&pending.ticket).copied());
                            self.mark_turn(pending.ticket, TurnAsk::Crown(*crown));
                        }
                    }
                    self.control.dialog_deliveries.insert(id, pending);
                }
                Err(reason) => {
                    let reason = walk_reason(reason, pending.steps);
                    let screen = (!unseen).then_some(screen.as_str());
                    self.control_settle_seen(id, pending, "unknown", Some(reason), screen);
                }
            }
        }
    }

    /// The mod's report on a held question (T-576), its outcome the reason.
    /// `answered`: the mod returned the board's answer and the engine fired
    /// no `PostToolUse` for it, so the report is ingested as that frame, of
    /// the mod's road (the shadow pairs nothing with it): the card leaves
    /// needs-you on it and the answer's delivery is confirmed by it.
    /// `declined`: the person refused the native dialog, which no hook says
    /// (the trap T-447 measured), so it is the dialog's dismissal here.
    /// `nothing_held`: an answer came after the person's own; the delivery
    /// settles on the person's edge.
    pub(super) fn on_mod_answer(&mut self, id: uuid::Uuid, frame: HookFrame) {
        let request = frame.payload["tool_use_id"].as_str().unwrap_or("").to_string();
        match frame.reason.as_deref() {
            // Ingested as the `PostToolUse` the engine never fired, keeping
            // its reason: on the mod road every frame comes by the mod
            // (T-577), and the reason is what tells this one from the
            // engine's own (`dialog_edge`).
            Some("answered") => self.on_hook(HookFrame { event: "PostToolUse".into(), ..frame }),
            Some("declined") => {
                if self.control.dialogs.get(&id).is_some_and(|d| d.request == request) {
                    self.control.dialogs.remove(&id);
                    self.control.dialog_edges.insert(id, (request, DialogEdge::Dismissed));
                    self.control_changed();
                }
            }
            other => self.journal.line(&format!(
                "mod answer for session {id} ({request}): {}",
                other.unwrap_or("no outcome")
            )),
        }
    }

    /// The hook edge that ended `request` on `session`, if one has (T-567):
    /// the confirmation an answer's keys wait for, whoever sent them.
    pub(super) fn control_dialog_edge(
        &self,
        session: uuid::Uuid,
        request: &str,
    ) -> Option<DialogEdge> {
        self.control.dialog_edges.get(&session).filter(|(r, _)| r == request).map(|(_, e)| *e)
    }

    /// The request `pending` answers on `id`, while it stands (T-873): the
    /// dialog projected for it, or the Codex approval's wait.
    fn delivery_standing<'a>(
        &'a self,
        id: uuid::Uuid,
        pending: &'a DialogDelivery,
    ) -> Option<Standing<'a>> {
        match &pending.answer {
            DeliveryAnswer::Dialog(response) => self
                .control
                .dialogs
                .get(&id)
                .filter(|d| d.request == pending.request)
                .map(|d| Standing::Dialog(d, response)),
            DeliveryAnswer::Approval { allow } => self
                .control
                .permissions
                .get(&id)
                .filter(|p| p.codex() && p.projection.request == pending.request)
                .map(|p| Standing::Approval(&p.projection, *allow)),
            DeliveryAnswer::Revise { text, .. } => self
                .control
                .dialogs
                .get(&id)
                .filter(|d| d.request == pending.request)
                .filter(|d| matches!(d.content, api::DialogContent::Plan { .. }))
                .map(|d| Standing::Revise(d, text)),
        }
    }

    /// The harness behind a session's pane; a record gone reads as Claude's,
    /// whose shapes refuse any other screen.
    pub(super) fn session_kind(&self, id: uuid::Uuid) -> SessionKind {
        self.board.sessions.iter().find(|s| s.id == id).map_or(SessionKind::Claude, |s| s.kind)
    }

    /// Whether the session still waits on a question, a plan or its folder
    /// trust dialog (T-844).
    fn dialog_waiting(&self, id: uuid::Uuid) -> bool {
        use mesimon_core::board::Reason;
        self.board.sessions.iter().any(|s| {
            s.id == id
                && matches!(
                    s.state,
                    SessionState::RequiresAction {
                        reason: Reason::Question | Reason::Plan | Reason::Trust
                    }
                )
        })
    }

    /// Whether the session still waits on a permission (T-846).
    fn approval_waiting(&self, id: uuid::Uuid) -> bool {
        self.board.sessions.iter().any(|s| {
            s.id == id && s.state == SessionState::RequiresAction { reason: Reason::Permission }
        })
    }

    /// The pane's whole visible screen: a dialog is the question, up to
    /// eight options with their descriptions, and the footer (T-567).
    fn dialog_screen(&self, id: uuid::Uuid) -> String {
        self.dialog_capture(id).unwrap_or_default()
    }

    /// `dialog_screen`, with a capture that failed said as `None` (T-943):
    /// an empty string is no observation that the dialog is gone.
    fn dialog_capture(&self, id: uuid::Uuid) -> Option<String> {
        match self.pane_tail(id, MAX_PANE_TAIL_LINES) {
            Response::PaneTail { lines, .. } => Some(lines.join("\n")),
            _ => None,
        }
    }

    /// A dialog answer's last word (T-567): its receipt and feed line, and
    /// the projection kept while the session still waits and `screen` still
    /// shows the dialog, so the card still offers the question. A Codex
    /// approval's wait is dropped once the keys went in, whatever confirmed
    /// them; one no key carried stays for the next answer (T-846).
    fn control_settle_dialog(
        &mut self,
        id: uuid::Uuid,
        pending: DialogDelivery,
        status: &str,
        reason: Option<String>,
        screen: &str,
    ) {
        self.control_settle_seen(id, pending, status, reason, Some(screen));
    }

    /// `control_settle_dialog` with what the screen showed, `None` when no
    /// capture succeeded (T-943): a plan sent back whose last read failed
    /// keeps the projection it never saw leave, while its record still waits
    /// on that dialog, so `get_ticket` still carries the request and the plan
    /// and the next accept or revision is not refused as a changed plan.
    fn control_settle_seen(
        &mut self,
        id: uuid::Uuid,
        pending: DialogDelivery,
        status: &str,
        reason: Option<String>,
        screen: Option<&str>,
    ) {
        let waiting = self.dialog_waiting(id);
        let kind = self.session_kind(id);
        let mut standing = false;
        let mut trust = false;
        if let Some(dialog) = self.control.dialogs.get(&id).filter(|d| d.request == pending.request)
        {
            trust = matches!(dialog.content, api::DialogContent::Trust { .. });
            // A plan sent back may leave its words in the field (T-943),
            // where the empty row's label no longer shows.
            let drawn = match (&pending.answer, screen) {
                (DeliveryAnswer::Revise { .. }, None) => true,
                (DeliveryAnswer::Revise { .. }, Some(screen)) => plan_dialog_drawn(screen),
                (_, screen) => dialog_shape_for(kind, dialog, screen.unwrap_or_default()),
            };
            if waiting && drawn {
                standing = true;
            } else {
                self.control.dialogs.remove(&id);
            }
        }
        // A trust accept that sent no key says so on the card (T-844): the
        // board's `w` has no receipt of its own, and the phone's card reads
        // the same line in its awareness.
        if trust && waiting && status == "unknown" {
            if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
                rec.detail = Some(crate::trust_dialog::NOT_ACCEPTED.into());
            }
        }
        let by = pending.by.principal();
        match pending.by {
            Deliverer::Phone { grant, command, .. } => {
                let event = match pending.answer {
                    DeliveryAnswer::Dialog(_) | DeliveryAnswer::Revise { .. } => {
                        "mesophon_dialog_answer"
                    }
                    DeliveryAnswer::Approval { .. } => {
                        if status != "unknown" {
                            self.control.permissions.remove(&id);
                        }
                        "mesophon_permission_answer"
                    }
                };
                self.control.remember(
                    grant,
                    command,
                    Reply::Delivery { status: status.into(), reason, replaced: None },
                );
                self.feed.board_outcome(by.actor(), event, Some(pending.ticket), status);
            }
            Deliverer::Board => {
                self.feed.board_outcome(by.actor(), "trust_accept", Some(pending.ticket), status);
            }
            Deliverer::Crown { crown, session, answer, reply, prior, .. } => {
                let revise = matches!(pending.answer, DeliveryAnswer::Revise { .. });
                self.crown_answer_settled(
                    id,
                    pending.ticket,
                    crown,
                    &answer,
                    status,
                    standing,
                    prior,
                    revise,
                );
                let cmd = if revise { "accept_plan" } else { "answer_agent" };
                self.feed.board_answer(
                    by.actor(),
                    cmd,
                    Some(pending.ticket),
                    status,
                    &answer,
                    reason.as_deref(),
                );
                let key = self.board.ticket(pending.ticket).map(|t| t.short_key.clone());
                let seen = Some(self.seen_token(pending.ticket));
                // A plan sent back answers as the accept does (T-943): the
                // crown called `accept_plan`.
                let response = match revise {
                    true => Response::AgentPlanAccepted {
                        key: key.unwrap_or_default(),
                        outcome: status.into(),
                        reason,
                        seen,
                    },
                    false => Response::AgentAnswered {
                        key: key.unwrap_or_default(),
                        outcome: status.into(),
                        reason,
                        answer,
                        seen,
                    },
                };
                if let Some(reply) = reply {
                    // Mid-tick, before the tick's own flush (T-582): the
                    // feed line the receipt speaks for is on disk first.
                    let _ = self.feed.flush();
                    // The reply the crown reads is this one (T-930).
                    let (response, told) = self.with_crown_rules(session, response);
                    let _ = reply.send(ClientReply { response, delivered: None, told });
                }
            }
        }
        self.control_changed();
    }

    /// The crown's `answer_agent` (T-569), past the gates `handle_agent`
    /// runs first (the crown, the crown's own ticket, the stamp, `authorize`):
    /// the board's switch, the agent's provenance, the stop, the dialog, its
    /// shape and the answer, each refused in words, and then Remote
    /// Control's road with the crown's name on it. `answer` is the one
    /// question's `index` or `text`, or one answer per question (T-571).
    /// `Ok` is the session whose dialog the answer walks; the receipt waits
    /// for the delivery to settle (`control_park_reply`).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn crown_dialog_answer(
        &mut self,
        crown: ulid::Ulid,
        session: uuid::Uuid,
        target: ulid::Ulid,
        key: &str,
        request: &str,
        answer: CrownAnswer,
    ) -> std::result::Result<uuid::Uuid, String> {
        if !self.board.crown_mode.answers() {
            return Err(format!(
                "this board leaves {key}'s question to a person (Settings → Agents → Crown mode is \
                 supervised); raise_hand on the crown's own ticket reaches the person"
            ));
        }
        let Some(rec) = self.board.live_agent(target) else {
            return Err(format!("{key} has no agent, so no question to answer"));
        };
        // T-539's line, as `crown_ask_hold` draws it: the one who started an
        // agent is the one who answers it.
        if rec.started_by.is_none() {
            return Err(format!(
                "a person started {key}'s agent, and the one who started it answers it, in the \
                 pane or from Remote Control"
            ));
        }
        let id = rec.id;
        match rec.state {
            SessionState::RequiresAction { reason: Reason::Question } => {}
            // T-582: a plan has its own road.
            SessionState::RequiresAction { reason: Reason::Plan } => {
                return Err(format!(
                    "{key}'s agent is stopped on a plan, not a question; accept_plan accepts a \
                     plan, after get_ticket shows it (needs_you)"
                ));
            }
            SessionState::RequiresAction { reason } => {
                let word = agent_reason_word(reason);
                return Err(format!(
                    "{key}'s agent is stopped on a {word}, not a question; a {word} is never the \
                     crown's to answer: a person answers it, in the pane or from Remote Control"
                ));
            }
            ref other => {
                return Err(format!(
                    "{key}'s agent is not stopped on a question (it is {}); answer_agent answers \
                     the question get_ticket shows in needs_you",
                    agent_state_word(other)
                ));
            }
        }
        // A person's answer always wins: a projection that moved on, or one
        // a hook edge already ended, refuses the crown's.
        let Some(dialog) = self.control.dialogs.get(&id).filter(|d| d.request == request) else {
            return Err("dialog changed; read get_ticket again".into());
        };
        let api::DialogContent::Questions { questions } = &dialog.content else {
            return Err(format!("{key}'s dialog is a plan; accept_plan accepts a plan"));
        };
        let answers = crown_answers(key, questions, answer)?;
        let answer = answer_words(questions, &answers);
        let response = api::DialogAnswer::Answers { answers };
        if dialog_plan(dialog, &response).is_none() {
            return Err(format!(
                "{key}'s dialog is not a shape the board answers; a person answers it"
            ));
        }
        if self.session_kind(id) == SessionKind::Codex && !codex_takes(dialog, &response) {
            return Err(format!(
                "{key}'s agent is a codex, whose dialog takes one choice of one question from \
                 the board (index); words, or several answers, are a person's, in the pane"
            ));
        }
        let (max_steps, walk) = walk_budget(dialog);
        // Queued words wait behind the answer and do not hold it (T-568).
        if self.control.dialog_deliveries.contains_key(&id) || self.control_input_in_flight(id) {
            return Err(format!("an answer for {key}'s question is already on its way"));
        }
        self.control.dialog_deliveries.insert(
            id,
            DialogDelivery {
                by: Deliverer::Crown { crown, session, answer, reply: None, prior: None },
                ticket: target,
                request: request.into(),
                answer: DeliveryAnswer::Dialog(response),
                deadline: Instant::now() + walk,
                next: Instant::now(),
                steps: 0,
                max_steps,
                walk: Walk::default(),
                confirm: None,
                by_mod: None,
            },
        );
        Ok(id)
    }

    /// The writer's reply for the `answer_agent` call that queued the answer
    /// for `id` (T-569): kept with the delivery, which answers it when it
    /// settles. Handed back when there is no such delivery, to be sent now.
    pub(super) fn control_park_reply(
        &mut self,
        id: uuid::Uuid,
        reply: Sender<ClientReply>,
    ) -> Option<Sender<ClientReply>> {
        match self.control.dialog_deliveries.get_mut(&id).map(|d| &mut d.by) {
            Some(Deliverer::Crown { reply: slot @ None, .. }) => {
                *slot = Some(reply);
                None
            }
            _ => Some(reply),
        }
    }

    /// The crown's plan sent back with words (T-943), past the accept's
    /// gates (`crown_plan_check`): a Claude agent's alone, the words as they
    /// came or refused, and the walk queued on Remote Control's road with the
    /// crown's name on it, its seat's pane kept for every key's check. `Ok`
    /// is the session whose dialog it walks; the receipt waits for it to
    /// settle (`control_park_reply`).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn crown_plan_revise(
        &mut self,
        crown: ulid::Ulid,
        session: uuid::Uuid,
        target: ulid::Ulid,
        key: &str,
        request: &str,
        text: String,
    ) -> std::result::Result<uuid::Uuid, String> {
        let id = self.crown_plan_check(target, key, request)?;
        let Some(rec) = self.board.sessions.iter().find(|s| s.id == id) else {
            return Err(format!("{key} has no agent, so no plan to send back"));
        };
        if rec.kind != SessionKind::Claude {
            return Err(format!(
                "{key}'s agent is a codex, whose plan the board does not send back with words; \
                 a person does, in the pane"
            ));
        }
        let pane = rec.pane_key.clone();
        if let Some(why) = dialog_text_refusal("change", &text) {
            return Err(why);
        }
        if self.control_input_in_flight(id) {
            return Err(format!("words for {key}'s agent are on their way; its plan waits"));
        }
        let (max_steps, walk) = ONE_WALK;
        self.control.dialog_deliveries.insert(
            id,
            DialogDelivery {
                by: Deliverer::Crown {
                    crown,
                    session,
                    answer: text.clone(),
                    reply: None,
                    prior: None,
                },
                ticket: target,
                request: request.into(),
                answer: DeliveryAnswer::Revise { text, pane },
                deadline: Instant::now() + walk,
                next: Instant::now(),
                steps: 0,
                max_steps,
                walk: Walk::default(),
                confirm: None,
                by_mod: None,
            },
        );
        Ok(id)
    }

    /// Whether a dialog answer is on its way down `id`'s pane, anyone's.
    pub(super) fn dialog_delivery_pending(&self, id: uuid::Uuid) -> bool {
        self.control.dialog_deliveries.contains_key(&id)
    }

    /// Why the crown's plan sent back may not take its next key (T-943),
    /// `None` when it may: what allowed it at the call (`crown_plan_allowed`'s
    /// list), the same pane, and fresh, at the key, no tmux client in it. A
    /// person there may be typing, and one the server cannot list counts as
    /// there.
    fn crown_revise_held(
        &self,
        crown: ulid::Ulid,
        session: uuid::Uuid,
        ticket: ulid::Ulid,
        id: uuid::Uuid,
        pane: &Option<String>,
    ) -> Option<&'static str> {
        let by = Principal::Agent { session };
        let rec = self.board.live_agent(ticket).filter(|rec| {
            rec.id == id
                && rec.started_by.is_some()
                && rec.kind == SessionKind::Claude
                && rec.state.has_pane()
                && rec.state == SessionState::RequiresAction { reason: Reason::Plan }
                && &rec.pane_key == pane
        });
        let Some(rec) = rec.filter(|_| {
            self.board.is_crowned(crown)
                && self.board.crown_mode.answers()
                && !authorize(&by, &Action::Mutate, &Resource::Ticket { id: ticket }).denied()
        }) else {
            return Some("state_changed");
        };
        if self.person_at_pane(&rec.sid16()) {
            return Some(A_PERSON_ATTACHED);
        }
        None
    }

    /// Whether the crown's answer may take its next key (T-569): what
    /// allowed it at the call still holds — the crown on its ticket, the
    /// board's switch, an agent the crown started on that ticket at a
    /// question, and the crown's `Mutate` on it. Anything else ends the walk
    /// as `state_changed`, keys already in left where they are.
    fn crown_answer_allowed(
        &self,
        crown: ulid::Ulid,
        session: uuid::Uuid,
        ticket: ulid::Ulid,
        id: uuid::Uuid,
    ) -> bool {
        let by = Principal::Agent { session };
        self.board.is_crowned(crown)
            && self.board.crown_mode.answers()
            && self.board.live_agent(ticket).is_some_and(|rec| {
                rec.id == id
                    && rec.started_by.is_some()
                    && rec.state == SessionState::RequiresAction { reason: Reason::Question }
            })
            && !authorize(&by, &Action::Mutate, &Resource::Ticket { id: ticket }).denied()
    }

    /// Everyone sees the crown's answer (T-569), once its keys went in and
    /// the dialog did not stay standing: `♛ answered` on the card, and the
    /// card's line `answered by T-411: <answer>` until the next state edge
    /// (`crown_answer_lines`). Keys that went in with the dialog still up
    /// give the turn its mark back. An answer no key carried shows nothing:
    /// its feed line is the record. A plan sent back (`revise`, T-943) reads
    /// `♛ sent plan back` and `plan sent back by T-411: <words>`.
    #[allow(clippy::too_many_arguments)]
    fn crown_answer_settled(
        &mut self,
        id: uuid::Uuid,
        ticket: ulid::Ulid,
        crown: ulid::Ulid,
        answer: &str,
        status: &str,
        standing: bool,
        prior: Option<Option<TurnAsk>>,
        revise: bool,
    ) {
        // The mod's answer lost the race to the person's own (T-576): the
        // turn is theirs, so it gets its mark back, as keys a dialog never
        // took do.
        if status == "unknown" {
            if let Some(prior) = prior {
                match prior {
                    Some(ask) => self.turn_asks.insert(ticket, ask),
                    None => self.turn_asks.remove(&ticket),
                };
            }
            return;
        }
        if status == "input_sent" && standing {
            if let Some(prior) = prior {
                match prior {
                    Some(ask) => self.turn_asks.insert(ticket, ask),
                    None => self.turn_asks.remove(&ticket),
                };
            }
            return;
        }
        let by = self.board.ticket(crown).map(|t| t.short_key.clone()).unwrap_or_default();
        let shown = mesimon_core::text::cap_bytes(answer, CROWN_ANSWER_LINE_BYTES);
        let tail = if shown.len() < answer.len() { "…" } else { "" };
        let (head, touch) = match revise {
            true => ("plan sent back by", "sent plan back"),
            false => ("answered by", "answered"),
        };
        let line = format!("{head} {by}: {shown}{tail}");
        if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
            rec.detail = Some(line.clone());
            self.crown_answer_lines.insert(id, line);
            self.persist_sessions();
        }
        self.crown_touched(crown, ticket, touch);
        self.crown_said(ticket, answer);
        self.broadcast();
    }

    pub(super) fn control_prompt_edge(&mut self, prompt: bool) {
        self.control.suppress_awareness = prompt;
    }
    fn control_awareness(&mut self) {
        let Some(stored) = &self.control.stored else { return };
        let board = stored.board.to_hex();
        // Each ticket's most urgent agent, the first of its rank as
        // `min_by_key` keeps it: one pass over the records, not one per
        // ticket (T-840).
        let mut urgent: HashMap<ulid::Ulid, &SessionRecord> = HashMap::new();
        for s in self.board.sessions.iter().filter(|s| s.kind.is_agent()) {
            let held = urgent.entry(s.ticket).or_insert(s);
            if attention::rank(&s.state) < attention::rank(&held.state) {
                *held = s;
            }
        }
        let mut changes = Vec::new();
        for ticket in self.board.tickets.iter().filter(|t| !t.is_archived()) {
            let Some(&session) = urgent.get(&ticket.id) else {
                continue;
            };
            let phase = api::Phase::of(&session.state);
            let previous = self.control.phases.insert(ticket.id, phase);
            // Initial observation is a baseline, never a completion alert.
            // A prompt send does not alter the attention state: the old terminal
            // phase is already cached, so that edge cannot announce Done.
            if self.control.suppress_awareness || previous.is_none() || previous == Some(phase) {
                continue;
            }
            changes.push((
                ticket.id.to_string(),
                api::Awareness {
                    phase,
                    headline: format!(
                        "{} · {}",
                        ticket.short_key,
                        ticket.title.chars().take(120).collect::<String>()
                    ),
                    detail: session.detail.clone(),
                    deep_link: format!("#board={board}&ticket={}", ticket.id),
                },
            ));
        }
        let peers: Vec<_> = self
            .control
            .peers
            .iter()
            .filter(|(_, p)| p.subscribed)
            .map(|(id, p)| (id.clone(), p.foreground.clone()))
            .collect();
        for (ticket, awareness) in changes {
            let visible = peers.iter().any(|(_, foreground)| {
                foreground.as_ref().is_some_and(|(id, stamp)| {
                    id == &ticket && stamp.elapsed() < Duration::from_secs(15)
                })
            });
            for (peer, _) in &peers {
                let alert = !visible
                    && matches!(
                        awareness.phase,
                        api::Phase::WaitingForApproval
                            | api::Phase::WaitingForInput
                            | api::Phase::Completed
                            | api::Phase::Failed
                    );
                self.control.answer(
                    peer,
                    0,
                    Reply::Awareness {
                        ticket: ticket.clone(),
                        awareness: awareness.clone(),
                        alert,
                    },
                );
            }
        }
    }

    pub(super) fn control_permission_wait(&mut self, frame: HookFrame, stream: UnixStream) {
        let Some(id) = self.resolve_session(&frame.session) else { return };
        let by = Principal::Automation { rule: "permission_hook".into() };
        if authorize(&by, &Action::Mutate, &Resource::Session { id }).denied()
            || self.control.stored.is_none()
            || !self.control.online
        {
            return;
        }
        let Some(rec) = self.board.sessions.iter().find(|s| s.id == id) else { return };
        if rec.kind != SessionKind::Claude
            || self.control_target(&rec.ticket.to_string(), &id.to_string()) != Some(id)
            || frame.payload["session_id"].as_str()
                != Some(rec.claude_session_id.unwrap_or(rec.id).to_string().as_str())
            || frame.payload["hook_event_name"] != "PermissionRequest"
            || frame.payload.get("agent_id").is_some()
        {
            return;
        }
        let Some(tool) =
            frame.payload["tool_name"].as_str().filter(|t| !t.is_empty() && t.len() <= 256)
        else {
            return;
        };
        if matches!(tool, "AskUserQuestion" | "ExitPlanMode") {
            return;
        }
        let input = &frame.payload["tool_input"];
        if !input.is_object()
            || serde_json::to_vec(&frame.payload).map_or(true, |b| b.len() > 16 * 1024)
        {
            return;
        }
        let grants: Vec<_> = self
            .control
            .stored
            .iter()
            .flat_map(|s| &s.grants)
            .filter(|g| self.control_granted(g.id, g.device))
            .map(|g| g.id)
            .collect();
        if grants.is_empty() {
            return;
        }
        // Closing this stream gives the deciding process empty stdout. Never
        // hold the writer for a phone: all waiting is state plus a deadline.
        let _ = stream.set_nonblocking(true);
        let renews = frame.reason.as_deref() == Some("renew");
        // The mod's next round of the same dialog takes the wait over, its
        // request and deadline unchanged, so a phone mid-tap still lands.
        // Any other request is a newer dialog and replaces the old wait,
        // whose run then prints nothing.
        if let Some(old) = self.control.permissions.remove(&id) {
            if renews && takes_over(&old, tool, input) {
                let road = PermissionRoad::Hook { stream, renews, closed: None };
                let wait = PermissionWait { road, grants, ..old };
                self.control.permissions.insert(id, wait);
                return;
            }
        }
        let hold = Duration::from_secs(mesimon_core::mesophon::PERMISSION_HOLD_SECS);
        self.control.permissions.insert(
            id,
            PermissionWait {
                projection: api::Permission {
                    request: uuid::Uuid::new_v4().to_string(),
                    tool: tool.into(),
                    input: input.clone(),
                    expires_at: now_ms() + hold.as_millis() as u64,
                },
                road: PermissionRoad::Hook { stream, renews, closed: None },
                offered: Instant::now(),
                deadline: Instant::now() + hold,
                grants,
            },
        );
        self.control_changed();
    }

    /// The hook edge that ends a session's permission wait, if `frame` is
    /// one: `releases_permission`'s, or the session's own next tool call
    /// once the wait is old enough that it cannot be the dialog's own
    /// (T-632): the person refused the dialog in the pane, which fires no
    /// hook.
    pub(super) fn control_release_permission(&mut self, id: uuid::Uuid, frame: &HookFrame) {
        let passed = self.control.permissions.get(&id).is_some_and(|p| {
            frame.event == "PreToolUse"
                && frame.payload.get("agent_id").is_none()
                && p.offered.elapsed() >= PERMISSION_PASSED_AFTER
        });
        if passed || super::releases_permission(frame) {
            self.control_cancel_permission(id);
        }
    }

    pub(super) fn control_cancel_permission(&mut self, id: uuid::Uuid) {
        if self.control.permissions.remove(&id).is_some() {
            self.control_changed();
        }
    }

    fn control_expire_permissions(&mut self) {
        let now = Instant::now();
        for p in self.control.permissions.values_mut() {
            p.note_closed(now);
        }
        let stored = self.control.stored.as_ref();
        let granted = |grant: BoardId| {
            stored
                .into_iter()
                .flat_map(|s| &s.grants)
                .any(|g| g.id == grant && self.control_granted(g.id, g.device))
        };
        let expired: Vec<_> = self
            .control
            .permissions
            .iter()
            .filter(|(id, p)| {
                p.lapsed(now, granted)
                    || !self.board.sessions.iter().any(|s| {
                        s.id == **id
                            && self.control_target(&s.ticket.to_string(), &s.id.to_string())
                                == Some(**id)
                    })
            })
            .map(|(id, _)| *id)
            .collect();
        for id in expired {
            self.control_cancel_permission(id);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn control_permission_answer(
        &mut self,
        by: &Principal,
        peer: &str,
        command: u64,
        ticket: &str,
        session: &str,
        request: &str,
        decision: api::PermissionDecision,
    ) -> Reply {
        let Some(id) = self.control_target(ticket, session) else {
            return Reply::Rejected { message: "session changed".into() };
        };
        if authorize(by, &Action::ApproveExisting, &Resource::Session { id }).denied() {
            return Reply::Revoked;
        }
        self.control_expire_permissions();
        let grant = self.control.peers.get(peer).map(|p| p.grant);
        if !self.control.permissions.get(&id).is_some_and(|p| {
            p.projection.request == request
                && !matches!(p.road, PermissionRoad::Hook { closed: Some(_), .. })
                && grant.is_some_and(|g| p.grants.contains(&g))
        }) {
            return Reply::Rejected {
                message: "permission expired or was already answered; check the pane".into(),
            };
        }
        if self.control.permissions.get(&id).is_some_and(PermissionWait::codex) {
            // A Codex approval (T-846): the answer walks the pane's dialog,
            // as a question's does, and the receipt waits for the request
            // to leave the runtime's snapshot. The wait stays until then, so
            // no second phone starts a walk of its own.
            let (Ok(ticket), Some(grant)) = (ulid::Ulid::from_string(ticket), grant) else {
                return Reply::Rejected { message: "invalid ticket".into() };
            };
            if self.control.dialog_deliveries.contains_key(&id) || self.control_input_in_flight(id)
            {
                return Reply::Rejected { message: "input is already pending".into() };
            }
            let Some(device) = self.control.peers.get(peer).map(|p| p.device) else {
                return Reply::Rejected { message: "session changed".into() };
            };
            let (max_steps, walk) = ONE_WALK;
            self.control.dialog_deliveries.insert(
                id,
                DialogDelivery {
                    by: Deliverer::Phone { grant, device, command },
                    ticket,
                    request: request.into(),
                    answer: DeliveryAnswer::Approval {
                        allow: decision == api::PermissionDecision::Allow,
                    },
                    deadline: Instant::now() + walk,
                    next: Instant::now(),
                    steps: 0,
                    max_steps,
                    walk: Walk::default(),
                    confirm: None,
                    by_mod: None,
                },
            );
            return Reply::delivery("awaiting_delivery");
        }
        let Some(mut pending) = self.control.permissions.remove(&id) else {
            return Reply::Rejected { message: "permission is no longer pending".into() };
        };
        // No second browser can answer after this point. Successful transport
        // is not proof Claude executed the tool; report only decision delivery.
        // A Codex wait returned above, so the road here is the hook's.
        let PermissionRoad::Hook { stream, .. } = &mut pending.road else {
            return Reply::Rejected { message: "permission is no longer pending".into() };
        };
        let sent = serde_json::to_vec(&decision)
            .ok()
            .is_some_and(|bytes| stream.write_all(&bytes).is_ok());
        self.feed.board(
            by.actor(),
            "mesophon_permission_answer",
            ulid::Ulid::from_string(ticket).ok(),
        );
        self.control_changed();
        Reply::delivery(if sent { "decision_sent" } else { "unknown" })
    }

    /// The request a Codex seat's snapshot carries, projected (T-846): a
    /// question into `dialogs`, as the hook stream's `AskUserQuestion` is,
    /// an approval into `permissions` where a phone is paired to answer it,
    /// and the plan a seat stopped on into `dialogs` as a `Plan`. The
    /// request's id changing is its resolution: the projection goes and
    /// the dialog's edge reads `Answered`, which is what a walked answer's
    /// receipt waits for. A secret question, a form and a seat not stopped
    /// on its request project nothing.
    pub(super) fn control_codex_ask(
        &mut self,
        id: uuid::Uuid,
        ask: Option<&crate::agents::codex::Ask>,
        plan: Option<(&str, &str)>,
    ) {
        use crate::agents::codex::AskKind;
        let Some(rec) = self.board.sessions.iter().find(|s| s.id == id) else { return };
        let (state, ticket) = (rec.state.clone(), rec.ticket);
        let plan = plan.filter(|_| state == SessionState::RequiresAction { reason: Reason::Plan });
        let current = match (ask, plan) {
            (Some(ask), _) => Some(ask.id.clone()),
            (None, Some((key, _))) => Some(format!("plan:{key}")),
            (None, None) => None,
        };
        let was =
            self.control.codex_asks.get(&id).filter(|w| Some(*w) != current.as_ref()).cloned();
        if let Some(was) = was {
            if self.control.dialogs.get(&id).is_some_and(|d| d.request == was) {
                self.control.dialogs.remove(&id);
                self.control.dialog_edges.insert(id, (was.clone(), DialogEdge::Answered));
                self.control_changed();
            }
            if self
                .control
                .permissions
                .get(&id)
                .is_some_and(|p| p.codex() && p.projection.request == was)
            {
                self.control.permissions.remove(&id);
                self.control_changed();
            }
            self.control.codex_asks.remove(&id);
        }
        let Some(current) = current else { return };
        if self.control.codex_asks.get(&id) == Some(&current) {
            return;
        }
        match (ask.map(|a| &a.kind), plan) {
            (Some(AskKind::Question { questions }), _)
                if state == SessionState::RequiresAction { reason: Reason::Question } =>
            {
                let Some(content) = Self::codex_questions(questions) else { return };
                let dialog = api::Dialog { request: current.clone(), content };
                self.control.dialogs.insert(id, dialog);
                self.control.codex_asks.insert(id, current);
                self.control_changed();
            }
            (Some(AskKind::Approval { what, command, cwd, reason, .. }), _)
                if state == SessionState::RequiresAction { reason: Reason::Permission } =>
            {
                let by = Principal::Automation { rule: "codex_observer".into() };
                if authorize(&by, &Action::Mutate, &Resource::Session { id }).denied()
                    || self.control.stored.is_none()
                    || !self.control.online
                    || self.control_target(&ticket.to_string(), &id.to_string()) != Some(id)
                {
                    return;
                }
                let grants: Vec<_> = self
                    .control
                    .stored
                    .iter()
                    .flat_map(|s| &s.grants)
                    .filter(|g| self.control_granted(g.id, g.device))
                    .map(|g| g.id)
                    .collect();
                if grants.is_empty() {
                    return;
                }
                let mut input = serde_json::Map::new();
                for (key, value) in [("command", command), ("cwd", cwd), ("reason", reason)] {
                    if let Some(value) = value {
                        input.insert(key.into(), serde_json::Value::String(value.clone()));
                    }
                }
                let hold = Duration::from_secs(mesimon_core::mesophon::PERMISSION_HOLD_SECS);
                self.control.permissions.insert(
                    id,
                    PermissionWait {
                        projection: api::Permission {
                            request: current.clone(),
                            tool: what.clone(),
                            input: serde_json::Value::Object(input),
                            expires_at: now_ms() + hold.as_millis() as u64,
                        },
                        road: PermissionRoad::Codex,
                        offered: Instant::now(),
                        deadline: Instant::now() + hold,
                        grants,
                    },
                );
                self.control.codex_asks.insert(id, current);
                self.control_changed();
            }
            (None, Some((_, markdown))) => {
                let content = api::DialogContent::Plan { markdown: markdown.into() };
                self.control.dialogs.insert(id, api::Dialog { request: current.clone(), content });
                self.control.codex_asks.insert(id, current);
                self.control_changed();
            }
            _ => {}
        }
    }

    /// A Codex question's content as the phone's dialog (T-846), within the
    /// bounds `control_observe_dialog` keeps for Claude's: one to four
    /// questions, each with one to eight options; a secret question, or one
    /// with no options (free words in Codex's notes row, unmeasured), draws
    /// no card. The `header` is Codex's, or the question's first word.
    fn codex_questions(
        questions: &[crate::agents::codex::AskQuestion],
    ) -> Option<api::DialogContent> {
        if questions.is_empty()
            || questions.len() > 4
            || questions.iter().any(|q| q.secret || q.options.is_empty() || q.options.len() > 8)
        {
            return None;
        }
        let questions = questions
            .iter()
            .map(|q| api::Question {
                question: q.question.clone(),
                header: if q.header.is_empty() {
                    q.question.split_whitespace().next().unwrap_or_default().to_string()
                } else {
                    q.header.clone()
                },
                options: q
                    .options
                    .iter()
                    .map(|o| api::QuestionOption {
                        label: o.label.clone(),
                        description: o.description.clone(),
                    })
                    .collect(),
                multi_select: false,
            })
            .collect();
        Some(api::DialogContent::Questions { questions })
    }

    /// A ticket filed live from the owner's phone, with the pictures its
    /// description links (T-670): the uploads are kept only with the ticket
    /// they were sent for, and a refusal lets them go, so a retry uploads
    /// afresh, as a refused pictured note write does.
    fn control_create_pictured(
        &mut self,
        by: &Principal,
        title: String,
        description: String,
        column: Option<String>,
        tags: &[api::TagPick],
        (grant, uploads): (BoardId, Vec<String>),
    ) -> Reply {
        self.grant_pictures(grant, &uploads, |d, owner, ids| {
            d.control_create(by, title, description, column, tags, Via::Live(owner, ids))
        })
    }

    /// The pictures a phone's command names (T-629, T-670, T-853): this
    /// grant's uploads by id, handed to `body`, and let go again when the
    /// body refuses, so a retry uploads afresh. An id that is no upload
    /// refuses before the body runs.
    fn grant_pictures(
        &mut self,
        grant: BoardId,
        uploads: &[String],
        body: impl FnOnce(&mut Self, &crate::attachments::Owner, &[ulid::Ulid]) -> Reply,
    ) -> Reply {
        let owner = crate::attachments::Owner::Grant(grant.to_hex());
        let ids: Vec<ulid::Ulid> =
            uploads.iter().filter_map(|u| ulid::Ulid::from_string(u).ok()).collect();
        let reply = if ids.len() == uploads.len() {
            body(self, &owner, &ids)
        } else {
            Reply::Rejected { message: "unknown picture".into() }
        };
        if matches!(reply, Reply::Rejected { .. } | Reply::Revoked) {
            self.uploads.discard(&owner, &ids);
        }
        reply
    }

    /// A ticket filed from the owner's phone (T-497): the composer's mint,
    /// landing quietly. A paired device starts nothing, so nothing here
    /// spawns, provisions or prompts.
    fn control_create(
        &mut self,
        by: &Principal,
        title: String,
        description: String,
        column: Option<String>,
        tags: &[api::TagPick],
        via: Via<'_>,
    ) -> Reply {
        let reject = |message: String| Reply::Rejected { message };
        let Some(column) = column.or_else(|| self.board.landing_column()) else {
            return reject("no column on the board takes new tickets".into());
        };
        if self.board.column(&column).is_none() {
            return reject(format!("no such column: {column}"));
        }
        if let Decision::Deny { reason } =
            authorize(by, &Action::FileTicket, &Resource::Column { name: column.clone() })
        {
            return reject(format!("denied: {reason}"));
        }
        if let Some(message) = self.team_viewer_refusal() {
            return reject(message);
        }
        let tags = match filed_tags(&self.board, tags) {
            Ok(tags) => tags,
            Err(message) => return reject(message),
        };
        // Refused whole past the limit, never cut; then scrubbed here, at
        // the boundary, because the browser's textarea is not one.
        if let Some(message) = mesimon_core::board::note_size_error(&description) {
            return reject(message);
        }
        let note = mesimon_core::board::sanitize_note(&description);
        let (envelope, uploads) = match via {
            Via::Mail(envelope) => (Some(envelope.to_hex()), &[][..]),
            Via::Live(_, ids) => (None, ids),
        };
        let images = match via {
            Via::Live(owner, ids) if !ids.is_empty() => {
                if note.trim().is_empty() {
                    return reject("pictures need a description".into());
                }
                match self.uploads.prepare(owner, ids, &note) {
                    Ok(images) => images,
                    Err(e) => return reject(format!("could not save pictures: {e:#}")),
                }
            }
            _ => Vec::new(),
        };
        let mint = Mint {
            column,
            title,
            workspace: None,
            from: None,
            tags,
            note: Some((note, images)),
            tier: None,
            envelope,
        };
        match self.mint_full(by, mint) {
            Ok(id) => {
                self.uploads.committed(uploads);
                self.feed.board(by.actor(), "mesophon_create_ticket", Some(id));
                let (key, column) = self
                    .board
                    .ticket(id)
                    .map(|t| (t.short_key.clone(), t.column.clone()))
                    .unwrap_or_default();
                Reply::Created { ticket: id.to_string(), key, column }
            }
            Err(message) => reject(message),
        }
    }

    /// An agent started from the owner's phone (T-498): the board's
    /// Shift+Enter on a ticket whose seat is empty or asleep — as the
    /// paired device. `prompt` is the first turn's words (T-510), and
    /// blank it is what the desk's blank Enter is: on an empty seat the
    /// provider its tiers give it starts on the title and description; on
    /// a sleeping one the agent wakes with nothing to say, since a wake
    /// with no words is `WakeSession`, never a paste of nothing. Refused
    /// while the seat is awake, stopping or starting; answered `starting`,
    /// or `provisioning` while a worktree is cut, and
    /// `control_follow_starts` turns the receipt `started` once it runs.
    fn control_start_agent(
        &mut self,
        by: &Principal,
        (grant, device, command): (BoardId, DeviceId, u64),
        ticket: &str,
        prompt: Option<String>,
        tier: Option<String>,
    ) -> Reply {
        let reject = |message: &str| Reply::Rejected { message: message.into() };
        let Some(id) = ulid::Ulid::from_string(ticket)
            .ok()
            .filter(|id| self.board.ticket(*id).is_some_and(|t| !t.is_archived()))
        else {
            return reject("ticket unavailable");
        };
        if let Decision::Deny { reason } =
            authorize(by, &Action::StartAgent, &Resource::Ticket { id })
        {
            return reject(&format!("denied: {reason}"));
        }
        let policy = self.board.ticket(id).map(|t| t.effective_execution_policy());
        if policy.is_some_and(|p| mesimon_core::authorize::authorize_execution(by, p).denied()) {
            return reject(
                "this ticket came from outside the board: start its agent at your terminal",
            );
        }
        if let Some(message) = self.team_viewer_refusal() {
            return reject(&message);
        }
        if let Some(message) = seat_refusal(&self.board, id) {
            return reject(message);
        }
        if self.control.starts.contains_key(&id)
            || self.pending_spawns.iter().any(|s| s.ticket == id && s.kind.is_agent())
            || self.pending_resumes.iter().any(|r| r.ticket == id)
        {
            return reject("an agent is already starting on this ticket");
        }
        // A start queued at the terminal behind a busy checkout is this
        // same start, already asked for; a second would lose its words.
        if self.queued.iter().any(|q| q.ticket == id) {
            return reject("a prompt is queued for this ticket at your terminal");
        }
        // The pick lands on the ticket before the seat is read, as the desk's
        // `^n` before its Shift+Enter (T-643): an empty seat starts on it and
        // a parked one wakes on it. A sleeping Codex seat refuses a Claude
        // tier in `apply_ticket_tier`'s words.
        if let Some(pick) = tier {
            if let Err(message) = self.apply_ticket_tier(id, Some(pick)) {
                return reject(&message);
            }
            self.persist_sessions();
        }
        // The words cross the same boundary a desk prompt does: scrubbed,
        // blank-is-none and refused past the cap (`sanitize_prompt`).
        let words = match prompt.as_deref().map(mesimon_core::command::sanitize_prompt) {
            Some(Err(message)) => return reject(&message),
            Some(Ok(words)) => words,
            None => None,
        };
        let asleep = self
            .board
            .live_agent(id)
            .filter(|s| matches!(s.state, SessionState::Sleeping))
            .map(|s| s.id);
        let answer = match (asleep, words) {
            (Some(_), Some(text)) => self.prompt_sleeping(id, text, false, super::Ack::PROMPT),
            (Some(sid), None) => self.resume_session_in(sid, false, false),
            (None, words) => {
                let kind = self.tier_book().start_provider(id).session_kind();
                self.spawn_session(id, kind, true, words, None, false)
            }
        };
        let (status, session) = match answer {
            Response::Spawned { id, .. } => ("starting", Some(id)),
            Response::Provisioning => ("provisioning", None),
            Response::Err { message } => return reject(&message),
            other => return reject(&format!("unexpected spawn answer: {other:?}")),
        };
        self.feed.board(by.actor(), "mesophon_start_agent", Some(id));
        self.control.starts.insert(id, StartWait { grant, device, command, session });
        Reply::delivery(status)
    }

    /// Follow each phone's start to its end (T-498): a receipt turns
    /// `started` once the session left `Spawning` and took its first prompt,
    /// and a refusal if it died first or its worktree could not be cut.
    /// The browser reads it with `status`, as it reads a prompt's.
    fn control_follow_starts(&mut self) {
        let tickets: Vec<_> = self.control.starts.keys().copied().collect();
        for ticket in tickets {
            let Some(wait) = self.control.starts.get(&ticket) else { continue };
            let (grant, device, command) = (wait.grant, wait.device, wait.command);
            if !self.control_granted(grant, device) {
                self.control.starts.remove(&ticket);
                continue;
            }
            let session = wait.session.or_else(|| self.board.live_agent(ticket).map(|s| s.id));
            let provisioning =
                self.pending_spawns.iter().any(|s| s.ticket == ticket && s.kind.is_agent());
            let cut_failed = self.worktrees.get(&ticket).and_then(|b| match &b.status {
                BindingStatus::Error { message, .. } => Some(message.clone()),
                _ => None,
            });
            let delivery = |status: &str| Reply::delivery(status);
            let (reply, done) = match start_progress(&self.board, session, provisioning, cut_failed)
            {
                Progress::Provisioning => (delivery("provisioning"), false),
                Progress::Starting => (delivery("starting"), false),
                Progress::Started => (delivery("started"), true),
                Progress::Failed(message) => (Reply::Rejected { message }, true),
            };
            self.control.remember(grant, command, reply);
            if done {
                self.control.starts.remove(&ticket);
            } else if let Some(wait) = self.control.starts.get_mut(&ticket) {
                wait.session = session;
            }
        }
    }

    /// A batch from the relay's mailbox (T-497): every envelope answered,
    /// then the next batch asked for, until the relay sends an empty one.
    /// A ticket on the board and not archived, by the id a browser sent.
    fn control_ticket(&self, ticket: &str) -> Option<ulid::Ulid> {
        ulid::Ulid::from_string(ticket)
            .ok()
            .filter(|id| self.board.ticket(*id).is_some_and(|t| !t.is_archived()))
    }

    /// The opening every card edit from a phone shares: the ticket on the
    /// board and not archived, the action authorized, and no viewer's seat.
    /// `Err` is the rejection's words.
    fn control_gate(
        &self,
        by: &Principal,
        ticket: &str,
        action: &Action,
    ) -> std::result::Result<ulid::Ulid, String> {
        let Some(id) = self.control_ticket(ticket) else {
            return Err("ticket unavailable".into());
        };
        if let Decision::Deny { reason } = authorize(by, action, &Resource::Ticket { id }) {
            return Err(format!("denied: {reason}"));
        }
        if let Some(message) = self.team_viewer_refusal() {
            return Err(message);
        }
        Ok(id)
    }

    /// A phone's note write as answered: fed, and the note's new revision.
    fn control_note_written(
        &mut self,
        by: &Principal,
        id: ulid::Ulid,
        ticket: &str,
        written: Response,
    ) -> Reply {
        let reject = |message: &str| Reply::Rejected { message: message.into() };
        match written {
            Response::NoteWritten { note } => {
                self.feed.board(by.actor(), "mesophon_write_note", Some(id));
                let rev = note
                    .and_then(|n| self.board.ticket(id).and_then(|t| t.note(n)))
                    .map_or(0, |m| m.rev);
                Reply::NoteWritten { ticket: ticket.into(), note: note.map(|n| n.to_string()), rev }
            }
            Response::Err { message } => reject(&message),
            other => reject(&format!("unexpected note answer: {other:?}")),
        }
    }

    /// A new title from the owner's phone (T-530): the desk's rename,
    /// scrubbed and capped the same way, and refused blank. The same title
    /// again writes nothing.
    fn control_rename(&mut self, by: &Principal, ticket: &str, title: &str) -> Reply {
        let reject = |message: &str| Reply::Rejected { message: message.into() };
        let id = match self.control_gate(by, ticket, &Action::RenameTicket) {
            Ok(id) => id,
            Err(message) => return reject(&message),
        };
        let Some(t) = self.board.ticket(id) else { return reject("ticket unavailable") };
        match phone_title(t, title) {
            Err(message) => return reject(message),
            Ok(None) => {}
            Ok(Some(title)) => {
                if let Response::Err { message } = self.with_ticket(id, |t| t.title = title) {
                    return reject(&message);
                }
                self.feed.board(by.actor(), "mesophon_rename_ticket", Some(id));
            }
        }
        Reply::Edited { ticket: id.to_string() }
    }

    /// A move from the owner's phone (T-530): `place_ticket`, as a person,
    /// so every gate a move at the desk meets applies, and a move inside
    /// the ticket's own column is a reorder.
    fn control_move(
        &mut self,
        by: &Principal,
        ticket: &str,
        column: &str,
        before: Option<&str>,
    ) -> Reply {
        let reject = |message: &str| Reply::Rejected { message: message.into() };
        let Some(id) = self.control_ticket(ticket) else {
            return reject("ticket unavailable");
        };
        let before = match before.map(ulid::Ulid::from_string) {
            None => None,
            Some(Ok(b)) => Some(b),
            Some(Err(_)) => return reject("ticket unavailable"),
        };
        if let Some(message) = self.team_viewer_refusal() {
            return reject(&message);
        }
        match self.place_ticket(
            id,
            column,
            Position::Before(before),
            by,
            None,
            "mesophon_move_ticket",
        ) {
            Ok(_) => Reply::Edited { ticket: id.to_string() },
            Err(message) => reject(&message),
        }
    }

    /// A tag from the owner's phone (T-530): one the board already has, on
    /// its group, replacing the group's other one; with no name, the
    /// group's tag comes off. A phone never adds to the vocabulary, as when
    /// it files a ticket. Wearing what is worn writes nothing.
    fn control_tag(
        &mut self,
        by: &Principal,
        ticket: &str,
        group: u8,
        name: Option<String>,
    ) -> Reply {
        let reject = |message: &str| Reply::Rejected { message: message.into() };
        let id = match self.control_gate(by, ticket, &Action::TagTicket) {
            Ok(id) => id,
            Err(message) => return reject(&message),
        };
        let Some(t) = self.board.ticket(id) else { return reject("ticket unavailable") };
        match phone_tag(&self.board, t, group, name.as_deref()) {
            Err(message) => return reject(&message),
            Ok(false) => {}
            Ok(true) => {
                if let Response::Err { message } = self.with_ticket(id, |t| t.set_tag(group, name))
                {
                    return reject(&message);
                }
                self.feed.board(by.actor(), "mesophon_tag_ticket", Some(id));
            }
        }
        Reply::Edited { ticket: id.to_string() }
    }

    /// Where a ticket's next agent works, from the owner's phone (T-642):
    /// the desk's Shift+Tab, `set_workspace` and its lock, so a ticket with
    /// an agent in its directory or a worktree already cut refuses in the
    /// desk's words. Choosing what is chosen writes nothing.
    fn control_workspace(&mut self, by: &Principal, ticket: &str, worktree: bool) -> Reply {
        let reject = |message: &str| Reply::Rejected { message: message.into() };
        let id = match self.control_gate(by, ticket, &Action::ChooseWorkspace) {
            Ok(id) => id,
            Err(message) => return reject(&message),
        };
        if self.team_content_only() {
            return reject("this board has no repository on this machine");
        }
        // Barred bindings refuse before the no-op reads, as the desk's
        // `SetWorkspace` does (T-683): `set_workspace` says so itself.
        if self.worktrees_barred {
            return reject(&self.barred_message("worktrees"));
        }
        let Some(t) = self.board.ticket(id) else { return reject("ticket unavailable") };
        if (t.workspace_strategy() == WorkspaceStrategy::Worktree) == worktree {
            return Reply::Edited { ticket: id.to_string() };
        }
        // The desk's own pick (`App::set_ticket_workspace`): a worktree, or
        // back to the board's default.
        let workspace = worktree.then_some(WorkspaceStrategy::Worktree);
        if let Response::Err { message } = self.set_workspace(id, workspace) {
            return reject(&message);
        }
        self.feed.board(by.actor(), "mesophon_set_workspace", Some(id));
        Reply::Edited { ticket: id.to_string() }
    }

    /// A ticket into the archive from the owner's phone (T-737), the desk's
    /// `a a`: `archive_ticket` as the person, so a ticket with a session
    /// awake, or an unticked box where its column asks for them all,
    /// refuses in the desk's words, and a landed worktree is reclaimed as
    /// at the desk. With `restore`, the desk's restore: back to the column
    /// it left. Asking for what already is writes nothing.
    fn control_archive(&mut self, by: &Principal, ticket: &str, restore: bool) -> Reply {
        let reject = |message: &str| Reply::Rejected { message: message.into() };
        let gone = || reject("ticket unavailable");
        let Ok(id) = ulid::Ulid::from_string(ticket) else { return gone() };
        let Some(archived) = self.board.ticket(id).map(|t| t.is_archived()) else {
            return gone();
        };
        if let Decision::Deny { reason } =
            authorize(by, &Action::ArchiveTicket, &Resource::Ticket { id })
        {
            return reject(&format!("denied: {reason}"));
        }
        if let Some(message) = self.team_viewer_refusal() {
            return reject(&message);
        }
        if archived != restore {
            return Reply::Edited { ticket: id.to_string() };
        }
        let (answer, rule) = if restore {
            (self.unarchive_ticket(id), "mesophon_unarchive_ticket")
        } else {
            (self.archive_ticket(id, by), "mesophon_archive_ticket")
        };
        if let Response::Err { message } = answer {
            return reject(&message);
        }
        self.feed.board(by.actor(), rule, Some(id));
        Reply::Edited { ticket: id.to_string() }
    }

    /// A ticket's workspace as the phone's board reads it (T-642): the TUI
    /// card's mark and whether Shift+Tab would take a change, none on a
    /// board with no repository here.
    fn control_workspace_of(&self, t: &Ticket) -> Option<api::Workspace> {
        if self.team_content_only() {
            return None;
        }
        let binding = self.worktrees.get(&t.id);
        let open = binding.is_none()
            && !self.board.sessions.iter().any(|s| s.ticket == t.id && s.state.has_pane());
        let item = binding.map(|b| self.worktree_item(&t.id, b));
        api::workspace(t.workspace_strategy(), open, item.as_ref())
    }

    fn control_mail(&mut self, items: Vec<control::MailItem>) {
        let Some(board) = self.control.stored.as_ref().map(|s| s.board) else { return };
        let more = !items.is_empty();
        for item in items {
            let answer = self.control_file_mail(board, item);
            self.control.send(answer);
        }
        if more {
            self.control.send(Wire::Collect { board });
        }
    }

    /// One envelope: opened with the grant it claims, filed at most once, and
    /// answered by a receipt only that grant's browser can open. A sender
    /// with no grant, or a letter that does not open, is dropped unanswered.
    fn control_file_mail(&mut self, board: BoardId, item: control::MailItem) -> Wire {
        let control::MailItem { device, envelope } = item;
        let id = envelope.id;
        let discard = Wire::Discard { device, id };
        let Some(grant) = self
            .control
            .stored
            .as_ref()
            .and_then(|s| s.grants.iter().find(|g| g.device == device))
            .cloned()
        else {
            return discard;
        };
        if !self.control_granted(grant.id, device) {
            return discard;
        }
        let Some(body) = self.control.keys.as_ref().and_then(|keys| {
            control::open_mail(board, grant.id, &envelope, keys, &grant.public).ok()
        }) else {
            return discard;
        };
        let reply = self.control_filed(&grant, id, body);
        let receipt = serde_json::to_value(&reply).ok().and_then(|body| {
            let keys = self.control.keys.as_ref()?;
            control::seal_receipt(board, grant.id, id, keys, &grant.public, body).ok()
        });
        match receipt {
            Some(receipt) => Wire::Collected { device, receipt: Box::new(receipt) },
            None => discard,
        }
    }

    /// File one opened envelope, or give the answer it already had. Written
    /// while its browser was away, it lands where it can: a column the board
    /// has since lost becomes the default one, and a tag it lost is dropped.
    fn control_filed(&mut self, grant: &Grant, id: ObjectId, body: serde_json::Value) -> Reply {
        let letter = api::read_letter(body);
        if let Some(api::Letter::Note(note)) = letter {
            return self.control_filed_note(grant, id, note);
        }
        if let Some(api::Letter::Prompt(prompt)) = letter {
            return self.control_filed_prompt(grant, id, prompt);
        }
        let hex = id.to_hex();
        if let Some(t) = self.board.tickets.iter().find(|t| t.envelope.as_deref() == Some(&hex)) {
            return Reply::Created {
                ticket: t.id.to_string(),
                key: t.short_key.clone(),
                column: t.column.clone(),
            };
        }
        if let Some(f) =
            self.control.stored.as_ref().and_then(|s| s.filed.iter().find(|f| f.envelope == id))
        {
            return Reply::Created {
                ticket: f.ticket.clone(),
                key: f.key.clone(),
                column: f.column.clone(),
            };
        }
        let Some(api::Letter::Ticket(ticket)) = letter else {
            return Reply::Rejected { message: "the ticket did not read".into() };
        };
        let column = ticket.column.filter(|c| self.board.column(c).is_some());
        let tags: Vec<_> = ticket
            .tags
            .into_iter()
            .filter(|t| self.board.tag_def(t.group, &t.name).is_some())
            .collect();
        let by = Principal::Paired { device: grant.device.to_hex(), grant: grant.id.to_hex() };
        let reply = self.control_create(
            &by,
            ticket.title,
            ticket.description,
            column,
            &tags,
            Via::Mail(id),
        );
        if let Reply::Created { ticket, key, column } = &reply {
            if let Some(mut s) = self.control.stored.clone() {
                s.filed.push(Filed {
                    envelope: id,
                    ticket: ticket.clone(),
                    key: key.clone(),
                    column: column.clone(),
                });
                let excess = s.filed.len().saturating_sub(FILED_KEEP);
                s.filed.drain(..excess);
                // The ticket itself already holds the envelope: a failed
                // save here costs only the replay guard, never a duplicate.
                let _ = self.control_save(&s);
                self.control.stored = Some(s);
            }
        }
        reply
    }

    // ---- notes (T-532) ----------------------------------------------------

    /// A note edit written while the terminal was away, applied once. A
    /// replayed letter gets the answer it had; an edit whose words the note
    /// already holds was this letter, written before a crash cut its answer.
    fn control_filed_note(&mut self, grant: &Grant, id: ObjectId, note: api::MailNote) -> Reply {
        if let Some(answer) = self.control_answered(id) {
            return answer;
        }
        let by = Principal::Paired { device: grant.device.to_hex(), grant: grant.id.to_hex() };
        let already = self.control_note_body(&note.ticket, note.note.as_deref()).and_then(
            |(ticket, meta, text)| {
                (text == mesimon_core::board::sanitize_note(&note.text)).then(|| {
                    Reply::NoteWritten {
                        ticket: ticket.to_string(),
                        note: Some(meta.id.to_string()),
                        rev: meta.rev,
                    }
                })
            },
        );
        let answer = already.unwrap_or_else(|| {
            self.control_write_note(&by, &note.ticket, note.note.as_deref(), note.text, note.rev)
        });
        // A failed save costs the replay guard only: an edit is answered
        // again by its words, a delete by the note being gone.
        self.control_keep_answer(id, &answer);
        answer
    }

    /// The answer a note's or a prompt's letter already had, if any.
    fn control_answered(&self, id: ObjectId) -> Option<Reply> {
        let f = self.control.stored.as_ref()?.noted.iter().find(|f| f.envelope == id)?;
        Some(serde_json::from_value(f.answer.clone()).unwrap_or(Reply::delivery("unknown")))
    }

    /// Keep a letter's answer in the ring a replay is answered from.
    fn control_keep_answer(&mut self, id: ObjectId, answer: &Reply) {
        let Some(mut s) = self.control.stored.clone() else { return };
        let kept = serde_json::to_value(answer).unwrap_or_default();
        s.noted.push(NoteFiled { envelope: id, answer: kept });
        let excess = s.noted.len().saturating_sub(FILED_KEEP);
        s.noted.drain(..excess);
        let _ = self.control_save(&s);
        self.control.stored = Some(s);
    }

    // ---- prompts by mail (T-881) ------------------------------------------

    /// Words a phone wrote for a ticket's agent while the terminal was away:
    /// queued once, as the phone's Queue queues them, on the agent the
    /// ticket has now. A replayed letter gets the answer it had. The answer
    /// is kept before the queue is drained, so the words reach the pane
    /// only once the ring says so; a crash before that replays the letter,
    /// and a pane ask lives in memory alone, so nothing went twice.
    fn control_filed_prompt(
        &mut self,
        grant: &Grant,
        id: ObjectId,
        prompt: api::MailPrompt,
    ) -> Reply {
        if let Some(answer) = self.control_answered(id) {
            return answer;
        }
        let by = Principal::Paired { device: grant.device.to_hex(), grant: grant.id.to_hex() };
        let target = (prompt.ticket.as_str(), prompt.session.as_deref());
        let answer = self.control_mail_prompt(&by, target, prompt.text);
        self.control_keep_answer(id, &answer);
        if let Reply::Delivery { .. } = answer {
            self.drain_queue();
            self.broadcast();
        }
        answer
    }

    /// The gates and the park for one mailed prompt, the words not yet sent.
    fn control_mail_prompt(
        &mut self,
        by: &Principal,
        (ticket, session): (&str, Option<&str>),
        text: String,
    ) -> Reply {
        let reject = |message: &str| Reply::Rejected { message: message.into() };
        let Some(ticket) = self.control_ticket(ticket) else {
            return reject("ticket unavailable");
        };
        let id = match mail_prompt_seat(&self.board, ticket, session) {
            Ok(id) => id,
            Err(message) => return reject(message),
        };
        if authorize(by, &Action::PromptExisting, &Resource::Session { id }).denied() {
            return Reply::Revoked;
        }
        if let Some(message) = self.team_viewer_refusal() {
            return reject(&message);
        }
        let text = match mesimon_core::command::sanitize_prompt(&text) {
            Ok(Some(text)) => text,
            Ok(None) => return reject("nothing to send"),
            Err(message) => return reject(&message),
        };
        // As the live Queue: words already going into the pane are not
        // replaced by a letter.
        if self.parked(id)
            || self.steering(id)
            || self.board.sessions.iter().any(|s| s.id == id && s.pending_submit)
        {
            return reject("a prompt is already going in to this agent");
        }
        let replaced = self.queued.iter().find(|q| q.ticket == ticket).map(|q| api::Replaced {
            by: q
                .by
                .and_then(|c| self.board.ticket(c))
                .map(|c| c.short_key.clone())
                .or_else(|| q.held.map(|_| "held".into()))
                .unwrap_or_else(|| "you".into()),
        });
        self.forget_queued(ticket, "queued_ask_replaced", "mesophon", "person");
        if let Err(message) =
            self.park_ask(ticket, QueuedSeat::Pane(id), text, super::Asker::Person, false, false)
        {
            return reject(&message);
        }
        self.feed.board(by.actor(), "mesophon_prompt_mail", Some(ticket));
        Reply::Delivery { status: "queued".into(), reason: None, replaced }
    }

    /// One note's metadata and body as they stand, or `None`.
    fn control_note_body(
        &self,
        ticket: &str,
        note: Option<&str>,
    ) -> Option<(ulid::Ulid, mesimon_core::board::NoteMeta, String)> {
        let id = self.control_ticket(ticket)?;
        let note = ulid::Ulid::from_string(note?).ok()?;
        match self.read_note(id, note) {
            Response::Note { text, meta } => {
                Some((id, meta, mesimon_core::board::sanitize_note(&text)))
            }
            _ => None,
        }
    }

    /// Who wrote a note last, in the page's words: `you` at the desk, `agent`,
    /// a paired browser by the name it paired with, a teammate by theirs.
    pub(super) fn control_author(&self, meta: &mesimon_core::board::NoteMeta) -> String {
        let by = if meta.edited_by.is_empty() { &meta.created_by } else { &meta.edited_by };
        if by.starts_with("agent:") {
            mesimon_core::keymap::AGENT_WORD.into()
        } else if let Some(device) = by.strip_prefix("device:") {
            self.control
                .stored
                .as_ref()
                .and_then(|s| s.grants.iter().find(|g| g.device.to_hex() == device))
                .map_or_else(|| "phone".into(), |g| g.name.clone())
        } else if let Some(name) = by.strip_prefix("member:") {
            name.into()
        } else if by.starts_with("automation:") {
            "mesimon".into()
        } else {
            "you".into()
        }
    }

    pub(super) fn control_notes(&self, by: &Principal, ticket: &str) -> Reply {
        let Some(id) = self.control_ticket(ticket) else {
            return Reply::Rejected { message: "ticket unavailable".into() };
        };
        if authorize(by, &Action::Read, &Resource::Ticket { id }).denied() {
            return Reply::Revoked;
        }
        let Some(t) = self.board.ticket(id) else {
            return Reply::Rejected { message: "ticket unavailable".into() };
        };
        let notes: Vec<_> =
            t.notes.iter().map(|n| api::NoteRow::of(n, self.control_author(n))).collect();
        let description = t.description().and_then(|meta| match self.read_note(id, meta.id) {
            Response::Note { text, .. } => Some(mesimon_core::board::sanitize_note(&text)),
            _ => None,
        });
        let reply = Reply::Notes { ticket: ticket.into(), notes, description };
        // A ticket with very many notes goes without the description's
        // body, which the page then asks for alone, as any other note.
        if measure(&reply).is_some_and(|n| n > BOARD_WORDS_BUDGET) {
            if let Reply::Notes { ticket, notes, .. } = reply {
                return Reply::Notes { ticket, notes, description: None };
            }
        }
        reply
    }

    pub(super) fn control_note(&self, by: &Principal, ticket: &str, note: &str) -> Reply {
        let Some(id) = self.control_ticket(ticket) else {
            return Reply::Rejected { message: "ticket unavailable".into() };
        };
        if authorize(by, &Action::Read, &Resource::Ticket { id }).denied() {
            return Reply::Revoked;
        }
        match self.control_note_body(ticket, Some(note)) {
            Some((_, meta, text)) => Reply::Note {
                ticket: ticket.into(),
                note: api::NoteRow::of(&meta, self.control_author(&meta)),
                text,
            },
            None => Reply::Rejected { message: "the note is gone".into() },
        }
    }

    /// The desk's note editor from a phone (T-532), with one rule more: an
    /// edit names the revision it was opened at, and a note that moved on
    /// since is answered with itself and left alone. The description stays:
    /// emptying it would make the next note the description.
    fn control_write_note(
        &mut self,
        by: &Principal,
        ticket: &str,
        note: Option<&str>,
        text: String,
        rev: Option<u64>,
    ) -> Reply {
        let reject = |message: &str| Reply::Rejected { message: message.into() };
        let id = match self.control_gate(by, ticket, &Action::Annotate) {
            Ok(id) => id,
            Err(message) => return reject(&message),
        };
        let Some(t) = self.board.ticket(id) else { return reject("ticket unavailable") };
        let note = match note_gate(t, note, text.trim().is_empty(), rev) {
            Ok(note) => note,
            Err(gate) => return gate.reply(ticket, |meta| self.control_author(meta)),
        };
        let written = self.write_note(id, note, text, None, by);
        self.control_note_written(by, id, ticket, written)
    }

    /// One piece of a picture from a phone (T-629), staged under its grant
    /// the way the desk's is staged under its connection, so a reconnect
    /// mid-upload carries on. Asked as the note write it is for, or with no
    /// ticket as the filing whose description it is for (T-670); either way
    /// the gate that keeps it is the write that links it.
    #[allow(clippy::too_many_arguments)]
    fn control_upload(
        &mut self,
        by: &Principal,
        grant: BoardId,
        ticket: Option<&str>,
        upload: Option<&str>,
        offset: usize,
        data: &str,
        complete: bool,
    ) -> Reply {
        let reject = |message: &str| Reply::Rejected { message: message.into() };
        let decision = match ticket {
            Some(ticket) => {
                let Some(id) = self.control_ticket(ticket) else {
                    return reject("ticket unavailable");
                };
                authorize(by, &Action::Annotate, &Resource::Ticket { id })
            }
            None => {
                let Some(name) = self.board.landing_column() else {
                    return reject("no column on the board takes new tickets");
                };
                authorize(by, &Action::FileTicket, &Resource::Column { name })
            }
        };
        if let Decision::Deny { reason } = decision {
            return reject(&format!("denied: {reason}"));
        }
        if let Some(message) = self.team_viewer_refusal() {
            return reject(&message);
        }
        if data.len() > api::PICTURE_CHUNK_BYTES.div_ceil(3) * 4 {
            return reject("picture piece too large");
        }
        let upload = match upload.map(ulid::Ulid::from_string).transpose() {
            Ok(upload) => upload,
            Err(_) => return reject("unknown picture"),
        };
        let owner = crate::attachments::Owner::Grant(grant.to_hex());
        match self.uploads.chunk(&owner, upload, offset, data, complete) {
            Ok(upload) => Reply::Uploaded { upload: upload.to_string() },
            Err(e) => reject(&format!("{e:#}")),
        }
    }

    /// Words to an agent with pictures this phone uploaded (T-853): the
    /// pictures are kept with the ticket before `send` puts the words in
    /// front of it, and a refusal lets them go, uploads included, so a
    /// retry uploads afresh. With none, `send` alone.
    fn control_pictured(
        &mut self,
        grant: BoardId,
        ticket: &str,
        text: String,
        uploads: &[String],
        send: impl FnOnce(&mut Self, String) -> Reply,
    ) -> Reply {
        if uploads.is_empty() {
            return send(self, text);
        }
        self.grant_pictures(grant, uploads, |d, owner, ids| {
            let Ok(id) = ulid::Ulid::from_string(ticket) else {
                return Reply::Rejected { message: "invalid ticket".into() };
            };
            let refused = |r: &Reply| matches!(r, Reply::Rejected { .. } | Reply::Revoked);
            match d.with_pictures(owner, id, text, ids, send, refused) {
                Ok(reply) => reply,
                Err(message) => Reply::Rejected { message },
            }
        })
    }

    /// A note write that links pictures this phone uploaded (T-629): the
    /// same gates as `control_write_note`, then the desk's own save, which
    /// keeps the pictures only once the note that links them is written.
    /// A refusal lets the uploads go, and a retry uploads afresh; a stale
    /// answer keeps them for Save mine, inside the ten-minute idle rule.
    fn control_write_pictured(
        &mut self,
        by: &Principal,
        ticket: &str,
        note: Option<&str>,
        text: String,
        rev: Option<u64>,
        (grant, uploads): (BoardId, Vec<String>),
    ) -> Reply {
        self.grant_pictures(grant, &uploads, |d, owner, ids| {
            d.control_write_pictured_inner(by, ticket, note, text, rev, owner, ids)
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn control_write_pictured_inner(
        &mut self,
        by: &Principal,
        ticket: &str,
        note: Option<&str>,
        text: String,
        rev: Option<u64>,
        owner: &crate::attachments::Owner,
        ids: &[ulid::Ulid],
    ) -> Reply {
        let reject = |message: &str| Reply::Rejected { message: message.into() };
        let id = match self.control_gate(by, ticket, &Action::Annotate) {
            Ok(id) => id,
            Err(message) => return reject(&message),
        };
        if text.trim().is_empty() {
            return reject("a note with pictures needs words");
        }
        if let Some(message) = mesimon_core::board::note_size_error(&text) {
            return reject(&message);
        }
        let Some(t) = self.board.ticket(id) else { return reject("ticket unavailable") };
        let note = match note_gate(t, note, false, rev) {
            Ok(note) => note,
            Err(gate) => return gate.reply(ticket, |meta| self.control_author(meta)),
        };
        let text = mesimon_core::board::sanitize_note(&text);
        let written = self.save_note_with_attachments(owner, id, note, text, ids.to_vec(), by);
        self.control_note_written(by, id, ticket, written)
    }

    /// The desk's second `^s` (T-532): mesimon's own sentence, naming the
    /// note, pasted to the ticket's awake agent. A prompt into a session the
    /// phone may already send, so it is authorized as one.
    fn control_tell_agent(&mut self, by: &Principal, ticket: &str, note: &str) -> Reply {
        let reject = |message: &str| Reply::Rejected { message: message.into() };
        let Some(id) = self.control_ticket(ticket) else {
            return reject("ticket unavailable");
        };
        let Some(note) = ulid::Ulid::from_string(note)
            .ok()
            .filter(|n| self.board.ticket(id).is_some_and(|t| t.note(*n).is_some()))
        else {
            return reject("the note is gone");
        };
        let Some(session) = self.prompt_target(id) else {
            return reject("no agent is awake on this ticket");
        };
        if authorize(by, &Action::PromptExisting, &Resource::Session { id: session }).denied() {
            return Reply::Revoked;
        }
        match self.note_to_agent(id, note) {
            Response::Ok => {
                self.feed.board(by.actor(), "mesophon_tell_agent", Some(id));
                Reply::delivery("submitted")
            }
            Response::Err { message } => reject(&message),
            other => reject(&format!("unexpected answer: {other:?}")),
        }
    }

    pub(super) fn control_board(&self) -> (Reply, Option<usize>) {
        let columns = self.board.sorted_columns();
        let mut allowed_tags: Vec<_> = self
            .board
            .tags
            .iter()
            .map(|t| api::TagOption { group: t.group, name: t.name.clone(), tint: t.tint() })
            .collect();
        allowed_tags.sort_by_key(|t| t.group);
        let book = self.tier_book();
        let tiers = projected_tiers(&book);
        let order = self.queue_order();
        let mut reply = Reply::Board {
            tiers,
            title: self
                .paths
                .repo_root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            columns: columns.iter().map(|c| c.name.clone()).collect(),
            default_column: self.board.landing_column(),
            column_descriptions: columns
                .iter()
                .filter_map(|c| Some((c.name.clone(), c.settings.description.clone()?)))
                .collect(),
            allowed_tags,
            // Board order, as the TUI draws it: a reorder moves a column's
            // `order`, not its place in the list.
            tickets: columns
                .iter()
                .flat_map(|c| self.board.column_tickets(&c.name))
                .map(|t| {
                    let mut row = self.control_card(t, &book);
                    let q = self.queued.iter().find(|q| q.ticket == t.id);
                    row.queued = q.map(|q| q.text.clone());
                    row.queue = q.map(|q| self.control_queue(q, &order));
                    row.agent = self.board.live_agent(t.id).map(|s| {
                        let (doing, said) = self.control_words(s);
                        self.control_agent(t, s, doing, said)
                    });
                    row.shell =
                        projected_shell(&self.board, &self.terminals, &self.foregrounds, t.id);
                    row
                })
                .collect(),
            // A phone's own tickets, once archived (T-665): no agent, no
            // queue, since an archived ticket has neither on the board.
            archived: sent_archived(&self.board).map(|t| self.control_card(t, &book)).collect(),
        };
        // A transcript no live agent reads any more leaves the cache.
        let live: HashSet<&str> = self.board.sessions.iter().filter_map(preview_path).collect();
        self.control.words.borrow_mut().retain(|path, _| live.contains(path.as_str()));
        // The board must fit one answer (`Control::answer`'s cap): the
        // agents' words are what a crowded board goes without first, then
        // the archived tickets Sent reads. Measured once, and again only
        // after a trim (T-688): this answers every phone up to once a second.
        let mut bytes = measure(&reply);
        let over = |bytes: Option<usize>| bytes.is_some_and(|n| n > BOARD_WORDS_BUDGET);
        if over(bytes) {
            if let Reply::Board { tickets, .. } = &mut reply {
                for agent in tickets.iter_mut().filter_map(|t| t.agent.as_mut()) {
                    (agent.doing, agent.said) = (None, None);
                }
            }
            bytes = measure(&reply);
        }
        if over(bytes) {
            if let Reply::Board { archived, .. } = &mut reply {
                archived.clear();
            }
            bytes = measure(&reply);
        }
        (reply, bytes)
    }

    /// A ticket as a phone's card reads it, without its agent or its queued
    /// words, which only a ticket on the board has.
    fn control_card(&self, t: &Ticket, book: &mesimon_core::tier::Book<'_>) -> api::Ticket {
        api::Ticket {
            id: t.id.to_string(),
            queued: None,
            queue: None,
            key: t.short_key.clone(),
            title: t.title.clone(),
            column: t.column.clone(),
            tags: projected_tags(&self.board, t),
            picked: projected_pickup(t),
            notes: u32::try_from(t.notes.len()).unwrap_or(u32::MAX),
            noted: api::notes_stamp(&t.notes),
            crown: self.board.is_crowned(t.id),
            tier: Some(book.of_ticket(t.id).id),
            created: mesimon_core::board::stamp_secs(&t.created_at).map(|s| s * 1000),
            crowned: projected_crown_touch(
                &self.board,
                self.crown_touches.get(&t.id),
                mesimon_core::clock::now_ms(),
            ),
            workspace: self.control_workspace_of(t),
            agent: None,
            shell: None,
        }
    }

    /// One agent as a phone sees it: its state word and since when, its
    /// step and latest reply line, and a dialog or permission waiting on it.
    fn control_agent(
        &self,
        t: &Ticket,
        s: &SessionRecord,
        doing: Option<String>,
        said: Option<String>,
    ) -> api::Agent {
        api::Agent {
            tier: Some(Self::launched_tier(s)),
            since: s.state_changed_at,
            doing,
            said,
            dialog: self.control_dialog_of(s).map(phone_dialog),
            trust: self.control_trust_of(s),
            permission: self.control_permission_of(s.id).cloned(),
            session: s.id.to_string(),
            provider: if s.kind == SessionKind::Codex { "codex" } else { "claude" }.into(),
            state: match s.state {
                SessionState::Spawning => "starting",
                SessionState::Running => "working",
                SessionState::RequiresAction { .. } => "needs attention",
                SessionState::Idle { .. } => "idle",
                SessionState::Sleeping => "sleeping",
                SessionState::Exited { .. } => "exited",
                SessionState::Failed { .. } => "failed",
                SessionState::Throttled => "rate limited",
                SessionState::Unknown { .. } => "unknown",
            }
            .into(),
            promptable: self.control_target(&t.id.to_string(), &s.id.to_string()).is_some(),
        }
    }

    /// The dialog a phone draws for this agent: the hook stream's projection,
    /// while the agent is stopped on a question or a plan.
    fn control_dialog_of(&self, s: &SessionRecord) -> Option<&api::Dialog> {
        use mesimon_core::board::Reason;
        self.control.dialogs.get(&s.id).filter(|_| {
            matches!(
                s.state,
                SessionState::RequiresAction { reason: Reason::Question | Reason::Plan }
            )
        })
    }

    /// The folder trust dialog a phone offers to accept (T-844), while the
    /// agent still stands on it: its own field on the agent, never a
    /// `dialog` kind a browser from before would not know.
    fn control_trust_of(&self, s: &SessionRecord) -> Option<api::Trust> {
        use mesimon_core::board::Reason;
        if s.state != (SessionState::RequiresAction { reason: Reason::Trust }) {
            return None;
        }
        let dialog = self.control.dialogs.get(&s.id)?;
        match &dialog.content {
            api::DialogContent::Trust { folder } => {
                Some(api::Trust { request: dialog.request.clone(), folder: folder.clone() })
            }
            _ => None,
        }
    }

    /// The trust dialog projected on `id`'s seat (T-844), if one stands.
    fn trust_dialog(&self, id: uuid::Uuid) -> Option<&api::Dialog> {
        self.control
            .dialogs
            .get(&id)
            .filter(|d| matches!(d.content, api::DialogContent::Trust { .. }))
    }

    /// A folder trust dialog read off `id`'s pane (T-844), projected for the
    /// phone's card and the board's `w` while it stands, once per dialog:
    /// a request minted here, since no hook names one. `true` when new.
    pub(super) fn control_offer_trust(&mut self, id: uuid::Uuid, folder: Option<String>) -> bool {
        if self.trust_dialog(id).is_some() {
            return false;
        }
        let request = format!("trust-{}", uuid::Uuid::new_v4().simple());
        self.control
            .dialogs
            .insert(id, api::Dialog { request, content: api::DialogContent::Trust { folder } });
        self.control_changed();
        true
    }

    /// The trust dialog left `id`'s pane with no hook to say so (T-844):
    /// the projection goes. A delivery still walking it settles on its own
    /// (`state_changed`, or `answered` once the seat moved on).
    pub(super) fn control_drop_trust(&mut self, id: uuid::Uuid) -> bool {
        let trust = self.trust_dialog(id).is_some();
        if trust {
            self.control.dialogs.remove(&id);
            self.control_changed();
        }
        trust
    }

    /// Whether an accept is still walking `id`'s dialog (T-844): its
    /// projection stays until the delivery settles on it.
    pub(super) fn control_trust_walking(&self, id: uuid::Uuid) -> bool {
        self.control.dialog_deliveries.contains_key(&id)
    }

    /// The board's `w` (T-844): the trust dialog `ticket`'s agent stands on,
    /// accepted by the walk a phone's card asks for, with a feed line of its
    /// own and the card's state for a receipt. Refuses in words, and the
    /// dialog stays up for the person.
    pub(super) fn accept_trust(&mut self, ticket: ulid::Ulid) -> Response {
        use mesimon_core::board::Reason;
        let Some(rec) = self.board.pane_target(ticket) else {
            return Response::Err { message: "no live agent session on this ticket".into() };
        };
        if rec.state != (SessionState::RequiresAction { reason: Reason::Trust }) {
            return Response::Err { message: "no trust dialog waiting".into() };
        }
        let id = rec.id;
        let Some(dialog) = self.trust_dialog(id) else {
            return Response::Err { message: crate::trust_dialog::NOT_RECOGNISED.into() };
        };
        if self.control.dialog_deliveries.contains_key(&id) {
            return Response::Err { message: "already trusting the workspace".into() };
        }
        let request = dialog.request.clone();
        let (max_steps, walk) = walk_budget(dialog);
        self.control.dialog_deliveries.insert(
            id,
            DialogDelivery {
                by: Deliverer::Board,
                ticket,
                request,
                answer: DeliveryAnswer::Dialog(api::DialogAnswer::Accept),
                deadline: Instant::now() + walk,
                next: Instant::now(),
                steps: 0,
                max_steps,
                walk: Walk::default(),
                confirm: None,
                by_mod: None,
            },
        );
        self.feed.board("local", "trust_accept", Some(ticket));
        Response::Ok
    }

    /// The permission a phone draws for this session, until it expires.
    fn control_permission_of(&self, id: uuid::Uuid) -> Option<&api::Permission> {
        self.control
            .permissions
            .get(&id)
            .filter(|p| Instant::now() < p.deadline)
            .map(|p| &p.projection)
    }

    /// Does the phone draw a card it can answer for this session's stop
    /// (T-568): a permission, a plan, or one question with one choice?
    /// Then a send it refuses points at that card, not the pane.
    fn control_answers_here(&self, id: uuid::Uuid) -> bool {
        let Some(s) = self.board.sessions.iter().find(|s| s.id == id) else { return false };
        self.control_permission_of(id).is_some()
            || self.control_trust_of(s).is_some()
            || self.control_dialog_of(s).is_some_and(|d| match &d.content {
                api::DialogContent::Plan { .. } | api::DialogContent::Trust { .. } => true,
                api::DialogContent::Questions { questions } => {
                    dialog_answerable_for(s.kind, questions)
                }
            })
    }

    /// A phone's words on their way into this pane (T-568): a steer parked
    /// for its paste, or a delivery pasted and owed its Enter. A phone's ask
    /// still in the queue is not in flight: it waits for the turn to end,
    /// and a dialog's answer is what ends it.
    fn control_input_in_flight(&self, id: uuid::Uuid) -> bool {
        self.control.pending.contains_key(&id)
            && !self.queued.iter().any(|q| matches!(q.seat, QueuedSeat::Pane(s) if s == id))
    }

    /// A queued ask as a phone reads it (T-568): the board's own row
    /// (`pending_items`) in the phone's fields.
    fn control_queue(&self, q: &QueuedAsk, order: &[usize]) -> api::Queue {
        api::Queue {
            by: q.by.and_then(|c| self.board.ticket(c)).map(|c| c.short_key.clone()),
            sends: q.sends,
            held: q.held.map(str::to_string),
            waits: self.ask_waits_on(q.ticket, order),
            asking: self.ask_asking(q.ticket, order),
        }
    }

    /// A live agent's step (while it works) and its latest reply's first
    /// line, from the transcript the TUI's card reads (T-497). Re-read only
    /// when the file's length or mtime moved.
    fn control_words(&self, s: &SessionRecord) -> (Option<String>, Option<String>) {
        let Some(path) = preview_path(s) else { return (None, None) };
        let Ok(meta) = std::fs::metadata(path) else { return (None, None) };
        let mtime_ms = meta.modified().ok().and_then(mesimon_core::clock::epoch_ms).unwrap_or(0);
        let mut words = self.control.words.borrow_mut();
        let fresh = words.get(path).is_some_and(|w| w.len == meta.len() && w.mtime_ms == mtime_ms);
        if !fresh {
            let prior = words.get(path).map(|w| &w.read);
            let read = crate::agents::read_preview_after(s.kind, std::path::Path::new(path), prior)
                .unwrap_or_default();
            let preview = &read.preview;
            let doing = match preview.activity.as_ref() {
                Some(crate::agents::AgentActivity::Tool(step)) => api::step_line(step),
                Some(crate::agents::AgentActivity::Thinking) => Some("thinking".into()),
                None => None,
            };
            let said = preview.text.as_deref().and_then(api::reply_line);
            words.insert(path.to_string(), Words { len: meta.len(), mtime_ms, doing, said, read });
        }
        let w = &words[path];
        // A step is only news while the turn runs; a finished turn's
        // transcript still ends in the last tool it ran.
        let working = matches!(s.state, SessionState::Running);
        (w.doing.clone().filter(|_| working), w.said.clone())
    }
    fn control_target(&self, ticket: &str, session: &str) -> Option<uuid::Uuid> {
        let ticket = ulid::Ulid::from_string(ticket).ok()?;
        let id = uuid::Uuid::parse_str(session).ok()?;
        let rec = self.board.pane_target(ticket)?;
        (rec.id == id && rec.kind.is_agent() && rec.state.has_pane() && !rec.observe_only())
            .then_some(id)
    }
    fn control_preview(&self, by: &Principal, ticket: &str, session: &str) -> Reply {
        let Some(id) = self.control_target(ticket, session) else {
            return Reply::Rejected { message: "session is no longer available".into() };
        };
        if authorize(by, &Action::Read, &Resource::Session { id }).denied() {
            return Reply::Revoked;
        }
        match self.pane_tail(id, 50) {
            Response::PaneTail { lines, cols } => {
                Reply::Preview { lines, cols: (cols > 0).then_some(cols) }
            }
            _ => Reply::Rejected { message: "preview unavailable".into() },
        }
    }
    /// A page of a session's transcript (T-626): any agent record of the
    /// ticket's, asleep or gone too, since the file outlives the pane. Read
    /// on its own thread; `None` when the read is on its way.
    fn control_transcript(
        &mut self,
        by: &Principal,
        asker: (&str, BoardId, u64),
        target: (&str, &str),
        ask: crate::agents::transcript::Ask,
        conversation: Option<String>,
    ) -> Option<Reply> {
        let (ticket, session) = target;
        let rec = ulid::Ulid::from_string(ticket)
            .ok()
            .zip(uuid::Uuid::parse_str(session).ok())
            .and_then(|(ticket, id)| {
                self.board
                    .sessions
                    .iter()
                    .find(|s| s.id == id && s.ticket == ticket && s.kind.is_agent())
            });
        let Some(rec) = rec else {
            return Some(Reply::Rejected { message: "session is no longer available".into() });
        };
        if authorize(by, &Action::Read, &Resource::Session { id: rec.id }).denied() {
            return Some(Reply::Revoked);
        }
        let Some(path) = crate::cost::transcript_of(rec).map(|(p, _)| p) else {
            // Nothing written yet: an empty page, and the next ask reads on.
            return Some(Reply::Transcript {
                conversation: String::new(),
                rows: Vec::new(),
                from: 0,
                end: 0,
                next_before: None,
            });
        };
        if self.control.reading >= READING_MAX {
            return Some(Reply::Rejected { message: "transcript busy; try again".into() });
        }
        self.control.reading += 1;
        let (peer, grant, command) = (asker.0.to_string(), asker.1, asker.2);
        let (kind, tx) = (rec.kind, self.control.tx.clone());
        std::thread::spawn(move || {
            let reply =
                crate::agents::read_transcript_reply(kind, &path, ask, conversation.as_deref())
                    .unwrap_or_else(|| Reply::Rejected {
                        message: "transcript unavailable".into(),
                    });
            let _ = tx.send(Msg::TranscriptRead(peer, grant, command, reply));
        });
        None
    }
    /// A transcript page landed (T-626): answered to the peer that asked,
    /// while its grant still stands. Not kept as a receipt: a read is asked
    /// again, and a page is the largest answer there is.
    pub(super) fn control_transcript_read(
        &mut self,
        peer: &str,
        grant: BoardId,
        command: u64,
        reply: Reply,
    ) {
        self.control.reading = self.control.reading.saturating_sub(1);
        let Some(p) = self.control.peers.get(peer).filter(|p| p.grant == grant) else { return };
        let reply = if self.control_granted(grant, p.device) { reply } else { Reply::Revoked };
        self.control.answer(peer, command, reply);
    }
    #[allow(clippy::too_many_arguments)]
    fn control_prompt(
        &mut self,
        by: &Principal,
        (grant, device, command): (BoardId, DeviceId, u64),
        target: (&str, &str),
        text: String,
        queued: bool,
        tier: Option<String>,
    ) -> Reply {
        let (ticket, session) = target;
        let Some(id) = self.control_target(ticket, session) else {
            return Reply::Rejected {
                message: "session changed; select a live agent again".into(),
            };
        };
        if authorize(by, &Action::PromptExisting, &Resource::Session { id }).denied() {
            return Reply::Revoked;
        }
        let text = match mesimon_core::command::sanitize_prompt(&text) {
            Ok(Some(text)) => text,
            Ok(None) => return Reply::Rejected { message: "nothing to send".into() },
            Err(message) => return Reply::Rejected { message },
        };
        let Ok(ticket) = ulid::Ulid::from_string(ticket) else {
            return Reply::Rejected { message: "invalid ticket".into() };
        };
        if self.parked(id)
            || self.steering(id)
            || self.board.sessions.iter().any(|s| s.id == id && s.pending_submit)
        {
            return Reply::Rejected {
                message: "a prompt is already waiting for this session".into(),
            };
        }
        // The composer's tier pick (T-643) lands first, as the desk ask
        // field's `^n` does: a pick the pane did not launch on is owed as a
        // switch, which is a relaunch at idle, so the words queue and ride
        // it (`deliver` relaunches an owed idle pane instead of pasting).
        if let Some(pick) = tier {
            if let Err(message) = self.apply_ticket_tier(ticket, Some(pick)) {
                return Reply::Rejected { message };
            }
            self.persist_sessions();
            self.broadcast();
        }
        let queued = queued || self.tier_owed(id);
        // A steer goes in now, and at a dialog a paste is the dialog's
        // answer (T-568): refused in the board's words, before anything
        // queued is touched. A queued prompt waits for the turn, as the
        // board's own does, and is held while a question stands (T-565).
        if !queued && self.pane_waits_on_you(id) {
            return Reply::Rejected {
                message: api::answer_first(self.control_answers_here(id)).into(),
            };
        }
        // The ticket has one queued ask, and this prompt takes its place:
        // whose it was is said on the receipt (T-568), never dropped
        // silently. The crown's own `get_ticket` says so too.
        let replaced = self.queued.iter().find(|q| q.ticket == ticket).map(|q| api::Replaced {
            by: q
                .by
                .and_then(|c| self.board.ticket(c))
                .map(|c| c.short_key.clone())
                .or_else(|| q.held.map(|_| "held".into()))
                .unwrap_or_else(|| "you".into()),
        });
        let pending =
            Pending { grant, device, command, ticket, pasted: false, send_now_receipt: None };
        let reply = if queued {
            self.forget_queued(ticket, "queued_ask_replaced", "mesophon", "person");
            self.control.pending.insert(id, pending);
            self.control_queue_prompt(grant, command, (ticket, id), text)
        } else {
            // A steer refused before any key leaves the ask it would have
            // replaced as it was (T-958), and says so with no `replaced`.
            self.ahead_of_queue(ticket, "queued_ask_replaced", "mesophon", |d| {
                d.control.pending.insert(id, pending);
                d.control_steer((grant, device, command), (ticket, id), &text)
            })
        };
        match reply {
            Reply::Delivery { status, reason, .. } => Reply::Delivery { status, reason, replaced },
            other => other,
        }
    }

    /// A phone's Queue: the words park as the board's queued ask, and go when
    /// the turn ends, or now when the agent is already idle.
    fn control_queue_prompt(
        &mut self,
        grant: BoardId,
        command: u64,
        target: (ulid::Ulid, uuid::Uuid),
        text: String,
    ) -> Reply {
        let (ticket, id) = target;
        if let Err(message) =
            self.park_ask(ticket, QueuedSeat::Pane(id), text, super::Asker::Person, false, false)
        {
            self.control_cancel(id);
            return Reply::Rejected { message };
        }
        self.drain_queue();
        self.broadcast();
        if self.queued.iter().any(|q| q.ticket == ticket) {
            Reply::delivery("queued")
        } else if self.parked(id) {
            Reply::delivery("awaiting_delivery")
        } else {
            self.control
                .receipts
                .get(&grant)
                .and_then(|r| r.get(&command))
                .cloned()
                .unwrap_or(Reply::delivery("unknown"))
        }
    }

    /// A phone's Steer: the words go into the pane now, mid-turn.
    fn control_steer(
        &mut self,
        by: (BoardId, DeviceId, u64),
        target: (ulid::Ulid, uuid::Uuid),
        text: &str,
    ) -> Reply {
        let ((grant, device, command), (ticket, id)) = (by, target);
        match self.paste_to_ticket(ticket, text, Ack::PROMPT) {
            Ok(()) => {
                // A Codex steer (T-845) is owed its runtime's count, as
                // parked words are their paste: the phone waits for either.
                let waiting = self.parked(id) || self.steering(id);
                if waiting {
                    // This composer supplies exactly the user's words, without
                    // any outstanding initial ticket prefill.
                    if let Some(rec) = self.board.sessions.iter_mut().find(|r| r.id == id) {
                        rec.pending_prefill = false;
                    }
                    self.control.pending.insert(
                        id,
                        Pending {
                            grant,
                            device,
                            command,
                            ticket,
                            pasted: false,
                            send_now_receipt: None,
                        },
                    );
                }
                if !waiting {
                    self.control_submitted(id);
                }
                self.feed.board(
                    &format!("device:{}", device.to_hex()),
                    "mesophon_prompt",
                    Some(ticket),
                );
                Reply::delivery(if waiting { "awaiting_delivery" } else { "submitted" })
            }
            // Refused before any key for a person at the pane (T-958): the
            // board knows nothing was sent, and the phone hears why in the
            // refusal word it already reads. Only the pending request this
            // attempt installed goes: an older prompt's owed entry, a due
            // paste and `pending_submit` were none of its making, and
            // `ahead_of_queue` puts back any older phone pending it took.
            Err(message) if self.refused_for_person == Some(id) => {
                self.control.pending.remove(&id);
                Reply::Rejected { message }
            }
            Err(_) => {
                self.control_cancel(id);
                Reply::delivery("unknown")
            }
        }
    }
    fn control_queue_action(
        &mut self,
        by: &Principal,
        ticket: &str,
        session: &str,
        send_now: Option<(BoardId, DeviceId, u64)>,
    ) -> Reply {
        let Some(id) = self.control_target(ticket, session) else {
            return Reply::Rejected {
                message: "session changed; select a live agent again".into(),
            };
        };
        if authorize(by, &Action::PromptExisting, &Resource::Session { id }).denied() {
            return Reply::Revoked;
        }
        let Some(q) = self.queued.iter().find(|q| matches!(q.seat, QueuedSeat::Pane(s) if s == id))
        else {
            return Reply::Rejected { message: "nothing queued on this session".into() };
        };
        let ticket = q.ticket;
        if let Some((grant, device, command)) = send_now {
            // `send_queued_ask` refuses it too; this says where to answer.
            if self.pane_waits_on_you(id) {
                return Reply::Rejected {
                    message: api::answer_first(self.control_answers_here(id)).into(),
                };
            }
            if let Some(p) = self.control.pending.get_mut(&id) {
                p.send_now_receipt = Some((grant, device, command));
            } else {
                // A phone can send a locally queued prompt. Its deferred
                // native delivery must retain the phone's authorization too.
                self.control.pending.insert(
                    id,
                    Pending {
                        grant,
                        device,
                        command,
                        ticket,
                        pasted: false,
                        send_now_receipt: None,
                    },
                );
            }
            match self.send_queued_ask(ticket) {
                Response::Err { message } => Reply::Rejected { message },
                _ => {
                    let waiting = self.parked(id) || self.steering(id);
                    Reply::delivery(if waiting { "awaiting_delivery" } else { "submitted" })
                }
            }
        } else {
            let text = q.text.clone();
            self.drop_queued_ask(ticket);
            Reply::TakenBack { text }
        }
    }

    fn control_granted(&self, grant: BoardId, device: DeviceId) -> bool {
        self.control
            .stored
            .as_ref()
            .is_some_and(|s| s.grants.iter().any(|g| g.id == grant && g.device == device))
            && self.control.identity.is_some()
            && self
                .team
                .device
                .as_ref()
                .and_then(|d| d.credential.as_ref())
                .map(|c| hex::encode(&c.hash()))
                == self.control.identity
    }
    pub(super) fn control_delivery_principal(&self, id: uuid::Uuid) -> Option<Principal> {
        self.control
            .pending
            .get(&id)
            .map(|p| Principal::Paired { device: p.device.to_hex(), grant: p.grant.to_hex() })
    }
    pub(super) fn control_delivery_allowed(&mut self, id: uuid::Uuid) -> bool {
        if let Some(p) = self.control.pending.get(&id).cloned() {
            // A Codex seat parked for its tier switch (T-643) has no pane
            // until its runtime stops and the wake follows: the words wait
            // as its `Wake` entry, and so does the phone's receipt.
            let switching = self.board.sessions.iter().any(|s| s.id == id && s.tier_wake);
            if !self.control_granted(p.grant, p.device)
                || p.send_now_receipt
                    .is_some_and(|(grant, device, _)| !self.control_granted(grant, device))
                || (!switching
                    && self.control_target(&p.ticket.to_string(), &id.to_string()) != Some(id))
            {
                self.control_cancel(id);
                return false;
            }
        }
        true
    }
    pub(super) fn control_pasted(&mut self, id: uuid::Uuid) {
        if let Some(p) = self.control.pending.get_mut(&id) {
            p.pasted = true;
        }
    }
    pub(super) fn control_submitted(&mut self, id: uuid::Uuid) {
        if let Some(p) = self.control.pending.remove(&id) {
            if let Some((grant, _, command)) = p.send_now_receipt {
                self.control.remember(grant, command, Reply::delivery("submitted"));
            }
            self.control.remember(p.grant, p.command, Reply::delivery("submitted"));
        }
    }
    pub(super) fn control_cancel(&mut self, id: uuid::Uuid) {
        self.control_cancel_kept(id);
    }

    /// `control_cancel`, keeping what it took (T-958): the phone's pending
    /// delivery, the receipts it overwrote as they were, and the pane's owed
    /// entry and due paste, so `control_restore` can put them back when the
    /// send that replaced a queued ask was refused before any key.
    pub(super) fn control_cancel_kept(&mut self, id: uuid::Uuid) -> Option<Cancelled> {
        let p = self.control.pending.remove(&id)?;
        self.queued.retain(|q| !matches!(q.seat, QueuedSeat::Pane(s) if s == id));
        let codex_due = self.codex_input_due.remove(&id);
        let pending_submit =
            self.board.sessions.iter().find(|s| s.id == id).is_some_and(|s| s.pending_submit);
        let owed = self.drop_owed(id);
        let word = if p.pasted { "unknown" } else { "rejected" };
        let mut receipts = Vec::new();
        let slots = p.send_now_receipt.map(|(grant, _, command)| (grant, command));
        for (grant, command) in slots.into_iter().chain([(p.grant, p.command)]) {
            let before = self.control.receipts.get(&grant).and_then(|r| r.get(&command)).cloned();
            receipts.push((grant, command, before));
            self.control.remember(grant, command, Reply::delivery(word));
        }
        Some(Cancelled { id, pending: p, receipts, owed, codex_due, pending_submit })
    }

    /// Put back what `control_cancel_kept` took (T-958), as it was.
    pub(super) fn control_restore(&mut self, c: Cancelled) {
        for (grant, command, before) in c.receipts.into_iter().rev() {
            let receipts = self.control.receipts.entry(grant).or_default();
            match before {
                Some(reply) => receipts.insert(command, reply),
                None => receipts.remove(&command),
            };
        }
        if let Some(due) = c.codex_due {
            self.codex_input_due.insert(c.id, due);
        }
        if let Some(owed) = c.owed {
            self.owed.entry(c.id).or_insert(owed);
        }
        if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == c.id) {
            rec.pending_submit |= c.pending_submit;
        }
        self.control.pending.insert(c.id, c.pending);
    }
    fn control_revoke(&mut self, grant: BoardId) {
        // The agent started stays: it is the owner's, like one started at
        // the desk. Only the receipt goes.
        self.control.starts.retain(|_, w| w.grant != grant);
        let peers: Vec<_> = self
            .control
            .peers
            .iter()
            .filter(|(_, p)| p.grant == grant)
            .map(|(id, _)| id.clone())
            .collect();
        for peer in peers {
            self.control.answer(&peer, 0, Reply::Revoked);
            self.control.peers.remove(&peer);
            self.control.send(Wire::Close { peer });
        }
        let pending: Vec<_> = self
            .control
            .pending
            .iter()
            .filter(|(_, p)| {
                p.grant == grant || p.send_now_receipt.is_some_and(|(g, _, _)| g == grant)
            })
            .map(|(id, _)| *id)
            .collect();
        for id in pending {
            self.control_cancel(id);
        }
        self.uploads.discard_all(&crate::attachments::Owner::Grant(grant.to_hex()));
    }
    fn control_revoke_all(&mut self) {
        self.control.permissions.clear();
        let grants: Vec<_> = self
            .control
            .stored
            .as_ref()
            .map(|s| s.grants.iter().map(|g| g.id).collect())
            .unwrap_or_default();
        for grant in grants {
            self.control_revoke(grant);
        }
    }
}

/// Whether a renewing run's request is the same dialog as the wait it
/// finds: its tool and its input, as the phone was shown them.
fn takes_over(wait: &PermissionWait, tool: &str, input: &serde_json::Value) -> bool {
    matches!(wait.road, PermissionRoad::Hook { .. })
        && wait.projection.tool == tool
        && wait.projection.input == *input
}

impl PermissionWait {
    /// A renewing run's stream closing starts its grace (T-632).
    fn note_closed(&mut self, now: Instant) {
        if let PermissionRoad::Hook { stream, renews: true, closed: closed @ None } = &mut self.road
        {
            if permission_peer_closed(stream) {
                *closed = Some(now);
            }
        }
    }

    /// Past its day, its run gone (a renewing one's past the grace), or no
    /// device it was offered to still paired. A Codex wait has no run: it
    /// ends with its request (`control_codex_ask`) or its day.
    fn lapsed(&self, now: Instant, granted: impl Fn(BoardId) -> bool) -> bool {
        let run_gone = match &self.road {
            PermissionRoad::Hook { stream, closed, .. } => closed.map_or_else(
                || permission_peer_closed(stream),
                |at| now >= at + PERMISSION_RENEW_GRACE,
            ),
            PermissionRoad::Codex => false,
        };
        now >= self.deadline || run_gone || !self.grants.iter().any(|g| granted(*g))
    }

    /// A Codex approval's wait, judged open while its request stands.
    fn codex(&self) -> bool {
        matches!(self.road, PermissionRoad::Codex)
    }
}

fn permission_peer_closed(stream: &UnixStream) -> bool {
    use std::os::fd::AsRawFd;
    let mut byte = 0u8;
    let received = unsafe {
        libc::recv(
            stream.as_raw_fd(),
            (&mut byte as *mut u8).cast(),
            1,
            libc::MSG_PEEK | libc::MSG_DONTWAIT,
        )
    };
    // No more input belongs to this one-shot request. EOF is cancellation;
    // unexpected trailing bytes or a socket error also invalidate it.
    received >= 0
        || !matches!(
            std::io::Error::last_os_error().kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
        )
}

/// Why a phone may not start an agent on this ticket (T-498): one agent per
/// ticket, read as `spawn_session` reads it, in words for someone away from
/// the terminal. A parked one is no refusal since T-510: the start wakes it,
/// as the desk's Shift+Enter does.
fn seat_refusal(board: &Board, ticket: ulid::Ulid) -> Option<&'static str> {
    let held = board.live_agent(ticket)?;
    if held.codex_stopping {
        Some("this ticket's agent is still stopping: try again in a moment")
    } else if matches!(held.state, SessionState::Sleeping) {
        None
    } else {
        Some("this ticket already has an agent")
    }
}

/// The agent a mailed prompt queues on (T-881): the ticket's pane agent when
/// the letter is opened, and only when it is the session the page wrote to,
/// as the live prompt's `control_target` binds it: words for one
/// conversation never land in another. A wake keeps the session's id. A
/// parked or absent agent is refused: a wake or a start from a letter would
/// launch hours later with nobody watching.
fn mail_prompt_seat(
    board: &Board,
    ticket: ulid::Ulid,
    session: Option<&str>,
) -> Result<uuid::Uuid, &'static str> {
    match board.pane_target(ticket) {
        Some(rec) if session.and_then(|s| uuid::Uuid::parse_str(s).ok()) != Some(rec.id) => {
            Err("the ticket has another agent now: read the words again before sending")
        }
        Some(rec) if !rec.observe_only() => Ok(rec.id),
        Some(_) => Err("this ticket's agent is not one mesimon started"),
        None if board.live_agent(ticket).is_some() => {
            Err("the agent was asleep: wake it with these words")
        }
        None => Err("no agent on this ticket to take them"),
    }
}

/// Where a phone's start is (T-498).
#[derive(Debug, PartialEq, Eq)]
enum Progress {
    /// The ticket's worktree is being cut; the spawn waits for it.
    Provisioning,
    /// The session is up or coming up, and its first prompt is owed.
    Starting,
    /// It left `Spawning` and took its first prompt: it runs.
    Started,
    Failed(String),
}

/// What became of a start, read from the record its spawn made (`session`)
/// or, with none yet, from the provision it waits on. `pending_submit` is
/// the owed first prompt on both providers: it clears on the prompt's ack,
/// and also when the Enter is given up, which leaves the agent up with its
/// title in the box — started, as a plain start at the desk leaves it.
fn start_progress(
    board: &Board,
    session: Option<uuid::Uuid>,
    provisioning: bool,
    cut_failed: Option<String>,
) -> Progress {
    let Some(id) = session else {
        return match (provisioning, cut_failed) {
            (true, _) => Progress::Provisioning,
            (false, Some(why)) => {
                Progress::Failed(format!("the worktree could not be made: {why}"))
            }
            (false, None) => Progress::Failed("the agent did not start".into()),
        };
    };
    let Some(rec) = board.sessions.iter().find(|s| s.id == id) else {
        return Progress::Failed("the agent did not start".into());
    };
    match &rec.state {
        SessionState::Spawning => Progress::Starting,
        SessionState::Failed { .. } | SessionState::Exited { .. } => {
            Progress::Failed("the agent exited before it started".into())
        }
        _ if rec.pending_submit => Progress::Starting,
        _ => Progress::Started,
    }
}

/// The tags a filed ticket wears, each spelled exactly as the registry has
/// it: the phone offers only the vocabulary its snapshot carried, so a name
/// the board lacks means that view is stale, and a phone never adds a tag.
/// One tag per group is `mint_full`'s rule and is judged there.
/// A ticket's tags as a phone draws them, each with the TUI's tint.
/// Why a phone's note write stops before it is written (T-532).
#[derive(Debug, PartialEq)]
enum NoteGate<'a> {
    /// Deleting a note that is already gone: done, nothing to write.
    Gone,
    /// The note named does not exist.
    Missing,
    /// The note moved on since the browser opened it.
    Stale(&'a mesimon_core::board::NoteMeta),
    /// Emptying the description would make the next note the description.
    Description,
}
impl NoteGate<'_> {
    fn reply(
        &self,
        ticket: &str,
        author: impl Fn(&mesimon_core::board::NoteMeta) -> String,
    ) -> Reply {
        let reject = |message: &str| Reply::Rejected { message: message.into() };
        match self {
            NoteGate::Gone => Reply::NoteWritten { ticket: ticket.into(), note: None, rev: 0 },
            NoteGate::Missing => reject("the note is gone"),
            NoteGate::Stale(meta) => Reply::NoteStale {
                ticket: ticket.into(),
                note: api::NoteRow::of(meta, author(meta)),
            },
            NoteGate::Description => reject("the description cannot be emptied from here"),
        }
    }
}

/// The note a phone's write lands on, `None` for a fresh one, or why it
/// stops. `rev` is the revision the browser opened; none skips the check.
/// How a phone's ticket reached the host: sealed in the mailbox, named by
/// its envelope so a replay lands once (T-497), or live, with the pictures
/// its description links (T-670).
#[derive(Clone, Copy)]
enum Via<'a> {
    Mail(ObjectId),
    Live(&'a crate::attachments::Owner, &'a [ulid::Ulid]),
}

fn note_gate<'a>(
    t: &'a Ticket,
    note: Option<&str>,
    blank: bool,
    rev: Option<u64>,
) -> Result<Option<ulid::Ulid>, NoteGate<'a>> {
    let Some(note) = note else { return Ok(None) };
    let Some(meta) = ulid::Ulid::from_string(note).ok().and_then(|n| t.note(n)) else {
        return Err(if blank { NoteGate::Gone } else { NoteGate::Missing });
    };
    if rev.is_some_and(|r| r != meta.rev) {
        return Err(NoteGate::Stale(meta));
    }
    if blank && t.description().is_some_and(|d| d.id == meta.id) {
        return Err(NoteGate::Description);
    }
    Ok(Some(meta.id))
}

/// The tiers a phone may pick (T-643): the desk's `^n` ring for an empty
/// seat, in its order. A seat that holds an agent keeps its provider, so the
/// page offers it the tiers whose `provider` is the agent's.
fn projected_tiers(book: &mesimon_core::tier::Book) -> Vec<api::TierOption> {
    book.cycle(None)
        .into_iter()
        .map(|t| api::TierOption {
            provider: mesimon_core::tier::builtin_id(t.provider).into(),
            summary: t.summary(),
            id: t.id,
            name: t.name,
        })
        .collect()
}

fn projected_tags(board: &Board, t: &Ticket) -> Vec<api::TagOption> {
    t.tags
        .iter()
        .map(|g| api::TagOption { group: g.group, name: g.name.clone(), tint: board.tint_of(g) })
        .collect()
}

/// The ticket's open shell for a phone (T-948), as the TUI's card reads it
/// (`glyphs::card_glyph`): its unadopted `!` terminal, or a `Bash` record
/// with a pane; busy while a command runs in either (`glyphs::is_working`).
fn projected_shell(
    board: &Board,
    terminals: &BTreeMap<Option<ulid::Ulid>, Option<String>>,
    foregrounds: &HashMap<uuid::Uuid, String>,
    ticket: ulid::Ulid,
) -> Option<api::Shell> {
    let terminal = terminals.get(&Some(ticket));
    let shells: Vec<&SessionRecord> = board
        .sessions
        .iter()
        .filter(|s| s.ticket == ticket && s.kind == SessionKind::Bash && s.state.has_pane())
        .collect();
    if terminal.is_none() && shells.is_empty() {
        return None;
    }
    let busy = terminal.is_some_and(Option::is_some)
        || shells
            .iter()
            .any(|s| s.state == SessionState::Running && foregrounds.contains_key(&s.id));
    Some(api::Shell { busy })
}

/// The crown's latest edit of a ticket for a phone (T-623), while it is
/// kept: the card's word, whose agent did it by key, and when.
fn projected_crown_touch(
    board: &Board,
    touch: Option<&CrownTouch>,
    now: u64,
) -> Option<api::Crowned> {
    let t = touch.filter(|t| now.saturating_sub(t.at_ms) < CROWN_TOUCH_KEPT_MS)?;
    let by = t.from.and_then(|from| board.ticket(from)).map(|f| f.short_key.clone());
    Some(api::Crowned { action: t.action.clone(), by, at: t.at_ms })
}

/// A phone's ticket once it was picked up (T-497), on the browser's clock
/// unit; nothing for a ticket no phone filed.
/// The archived tickets a phone's Sent reads (T-665): filed by a paired
/// browser, newest archive first, at most `ARCHIVED_SENT`.
fn sent_archived(board: &Board) -> impl Iterator<Item = &Ticket> {
    board.archived_tickets().into_iter().filter(|t| t.from_phone()).take(ARCHIVED_SENT)
}

fn projected_pickup(t: &Ticket) -> Option<api::Picked> {
    let p = t.picked.as_ref().filter(|_| t.from_phone())?;
    let at = mesimon_core::board::stamp_secs(&p.at)? * 1000;
    Some(api::Picked { by: p.by.clone(), at })
}

/// Where a session's words are read: Codex's normalized artifact, else the
/// native transcript — the TUI's `peek::preview_path`, the same order.
fn preview_path(s: &SessionRecord) -> Option<&str> {
    s.agent_preview_path.as_deref().or(s.transcript_path.as_deref())
}

/// The title a phone's rename writes (T-530), scrubbed and capped as the
/// desk's, and refused blank; `None` when the ticket already has it.
fn phone_title(t: &Ticket, raw: &str) -> Result<Option<String>, &'static str> {
    let title = mesimon_core::board::sanitize_title(raw.trim());
    if title.trim().is_empty() {
        return Err("a ticket needs a title");
    }
    Ok((t.title != title).then_some(title))
}

/// Whether a phone's tag changes the ticket (T-530). A name must be one
/// the board has on that group, spelled as the board spells it: a phone
/// never adds a word. `None` takes the group's tag off.
fn phone_tag(
    board: &mesimon_core::board::Board,
    t: &Ticket,
    group: u8,
    name: Option<&str>,
) -> Result<bool, String> {
    if let Some(name) = name {
        if board.tag_def(group, name).is_none() {
            return Err(format!("tag {name} is no longer on this board"));
        }
    }
    Ok(t.tag_in(group).map(|r| r.name.as_str()) != name)
}

fn filed_tags(
    board: &mesimon_core::board::Board,
    picks: &[api::TagPick],
) -> Result<Vec<TagRef>, String> {
    let mut refs: Vec<TagRef> = Vec::new();
    for pick in picks {
        if board.tag_def(pick.group, &pick.name).is_none() {
            return Err(format!("tag {} is no longer on this board", pick.name));
        }
        if !refs.iter().any(|r| r.group == pick.group && r.name == pick.name) {
            refs.push(TagRef { group: pick.group, name: pick.name.clone() });
        }
    }
    Ok(refs)
}

/// What the crown's `answer_agent` carries (T-569, T-571): the one
/// question's `index` or `text`, or one answer per question.
pub(super) enum CrownAnswer {
    Index(usize),
    Text(String),
    Answers(Vec<api::QuestionAnswer>),
}

/// The crown's answer as one answer per question, each checked against its
/// question and refused in words: the count, a one-choice question given
/// `indices` or a several-choice one given `index`, an option out of range,
/// and words that are not one plain line.
fn crown_answers(
    key: &str,
    questions: &[api::Question],
    answer: CrownAnswer,
) -> std::result::Result<Vec<api::QuestionAnswer>, String> {
    use api::QuestionAnswer as Q;
    let n = questions.len();
    let answers = match answer {
        CrownAnswer::Answers(answers) if answers.len() != n => {
            return Err(format!(
                "answers carries {} answers and {key}'s dialog asks {}: one answer per \
                 question, in order",
                answers.len(),
                mesimon_core::text::plural(n, "question")
            ));
        }
        CrownAnswer::Answers(answers) => answers,
        _ if n > 1 => {
            return Err(format!(
                "{key}'s dialog asks {n} questions at once; answers carries one answer per \
                 question, in order"
            ));
        }
        CrownAnswer::Index(index) => vec![Q::Choice { index }],
        CrownAnswer::Text(text) => vec![Q::Text { text }],
    };
    for (i, (question, answer)) in questions.iter().zip(&answers).enumerate() {
        let which = if n == 1 { "the question".to_string() } else { format!("question {}", i + 1) };
        let options = question.options.len();
        let range = |index: usize| {
            format!(
                "index {index} is out of range: {which} has {options} options, 0 to {}",
                options.saturating_sub(1)
            )
        };
        match answer {
            Q::Choice { .. } if question.multi_select => {
                return Err(format!(
                    "{which} takes several choices: indices (from 0) or text, in answers"
                ));
            }
            Q::Choice { index } if *index >= options => return Err(range(*index)),
            Q::Choices { .. } if !question.multi_select => {
                return Err(format!("{which} takes one choice: index, not indices"));
            }
            Q::Choices { indices } if indices.is_empty() => {
                return Err(format!(
                    "indices is empty: {which} takes at least one option, or text"
                ));
            }
            Q::Choices { indices } => {
                if let Some(index) = indices.iter().find(|i| **i >= options) {
                    return Err(range(*index));
                }
                if indices.iter().collect::<HashSet<_>>().len() != indices.len() {
                    return Err(format!("indices names an option twice for {which}"));
                }
            }
            Q::Text { text } => {
                if let Some(why) = dialog_text_refusal("text", text) {
                    return Err(why);
                }
            }
            Q::Choice { .. } => {}
        }
    }
    Ok(answers)
}

/// An answer as the mod returns it in the dialog's place (T-576), the
/// native dialog's own shape: each question's text to its label, its labels
/// joined by `, ` (measured on 2.1.287: `"Cheese, Ham"`), or its words.
fn mod_answers(
    questions: &[api::Question],
    answers: &[api::QuestionAnswer],
) -> std::collections::BTreeMap<String, String> {
    questions.iter().zip(answers).map(|(q, a)| (q.question.clone(), said(q, a))).collect()
}

/// An answer as the feed and the card say it: each question's label, its
/// labels joined by `, `, or its words, and the questions joined by `; `.
fn answer_words(questions: &[api::Question], answers: &[api::QuestionAnswer]) -> String {
    questions.iter().zip(answers).map(|(q, a)| said(q, a)).collect::<Vec<_>>().join("; ")
}

/// One question's answer in words: its label, its labels joined by `, `, or
/// the words typed.
fn said(q: &api::Question, a: &api::QuestionAnswer) -> String {
    let label = |i: usize| q.options.get(i).map_or("", |o| o.label.as_str());
    match a {
        api::QuestionAnswer::Choice { index } => label(*index).to_string(),
        api::QuestionAnswer::Choices { indices } => {
            indices.iter().map(|i| label(*i)).collect::<Vec<_>>().join(", ")
        }
        api::QuestionAnswer::Text { text } => text.clone(),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum DialogStep {
    Up,
    Down,
    /// Space on a several-choice row (T-571): it ticks the row or unticks it.
    Toggle {
        question: usize,
        row: usize,
    },
    /// Enter that moves the dialog on without ending it (T-571): a choice
    /// inside a batch, or a several-choice question's `Next` or `Submit` row.
    Next,
    /// The last key: Enter on the one question's answer, or on the review's
    /// `Submit answers`.
    Submit,
    Reject,
    Paste {
        question: usize,
        text: String,
    },
    /// No key this pass (T-943): the field still reads empty after the
    /// paste, which the screen has not drawn yet.
    Wait,
}

/// What an answer's earlier keys did that the screen alone does not say
/// (T-571): the questions whose text row it pasted into, and the row its
/// last key ticked, so a tick the screen did not take is never sent twice.
#[derive(Clone, Debug, Default)]
struct Walk {
    pasted: Vec<usize>,
    toggled: Option<(usize, usize)>,
    /// The row a plan sent back's last arrow put the cursor on (T-943).
    at: Option<usize>,
}

/// The keys and the time a walk to one row and its Enter may take (T-567):
/// one question's one choice, a plan, a trust dialog, a Codex approval.
const ONE_WALK: (u8, Duration) = (12, Duration::from_secs(8));

/// How many keys an answer may take and how long its walk may run. The one
/// question with one choice, and a plan, keep `ONE_WALK`. A
/// batch or a several-choice question (T-571) has room for each tab's rows
/// crossed twice, a tick on every option, its text, its Enter and the
/// review's two keys, at the walk's 350 ms a key with margin, up to
/// `DIALOG_WALK_MAX`.
fn walk_budget(dialog: &api::Dialog) -> (u8, Duration) {
    let api::DialogContent::Questions { questions } = &dialog.content else {
        return ONE_WALK;
    };
    if !dialog_batched(questions) {
        return ONE_WALK;
    }
    let keys: usize = questions.iter().map(|q| 3 * q.options.len() + 8).sum::<usize>() + 4;
    let keys = u8::try_from(keys).unwrap_or(u8::MAX);
    let walk =
        Duration::from_millis(450 * u64::from(keys)).clamp(Duration::from_secs(8), DIALOG_WALK_MAX);
    (keys, walk)
}

/// The question shapes the board's answer road takes (T-566 says it to the
/// crown as `answerable`), each measured on Claude Code 2.1.287: one
/// question with one choice (T-567), a question that takes several, and a
/// batch of up to four (T-571). The screen tells a batch's questions apart
/// by their words, so no two may read alike.
pub(super) fn dialog_answerable(questions: &[api::Question]) -> bool {
    let words: HashSet<String> = questions.iter().map(|q| squeeze(&q.question)).collect();
    (1..=4).contains(&questions.len()) && words.len() == questions.len() && !words.contains("")
}

/// Whether the dialog walks tab by tab to a review (T-571): several
/// questions, or one that takes several choices. One question with one
/// choice is answered by its own Enter (T-567).
fn dialog_batched(questions: &[api::Question]) -> bool {
    questions.len() > 1 || questions.iter().any(|q| q.multi_select)
}

/// Why no key could be chosen (T-567), in the receipt's words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Miss {
    /// No row of the dialog reads as the answer.
    LabelNotFound,
    /// A row starts the answer, or the answer starts a row, and neither
    /// reads as the other: wrapped or cut in a way the joining cannot mend.
    LabelWrapped,
    /// Not the measured dialog: the question, the footer or exactly one
    /// selected row is missing, or the dialog is one only the pane answers.
    ShapeUnrecognised,
    /// A row the walk ticked did not change its mark (T-571).
    TickNotTaken,
    /// The batch's review lists an answer other than the one sent (T-571),
    /// so `Submit answers` is not pressed.
    AnswerDiffers,
    /// The cursor is not where the last arrow put it (T-943): a person's.
    CursorMoved,
    /// The plan dialog's feedback field holds words that are not the
    /// crown's (T-943): a person's, never added to or submitted.
    FieldTouched,
}
impl Miss {
    fn word(self) -> &'static str {
        match self {
            Miss::LabelNotFound => "label_not_found",
            Miss::LabelWrapped => "label_wrapped",
            Miss::ShapeUnrecognised => "shape_unrecognised",
            Miss::TickNotTaken => "tick_not_taken",
            Miss::AnswerDiffers => "answer_differs",
            Miss::CursorMoved => "cursor_moved",
            Miss::FieldTouched => "field_touched",
        }
    }
}

/// What an answer asks of a dialog, checked against its projection.
#[derive(Debug, PartialEq, Eq)]
enum Plan {
    /// One question, one choice (T-567): its row, and Enter submits.
    Single(api::QuestionAnswer),
    /// A batch or a several-choice question (T-571): each tab's answer, then
    /// the review's `Submit answers`.
    Batch(Vec<api::QuestionAnswer>),
    Accept,
    Reject,
}

/// Why a dialog's text row may not take `text`, in the words the crown
/// reads, its argument named `name` (`text`, or `change` for a plan sent
/// back, T-943); `None` when it may. Refused, never rewritten: a newline
/// would submit the dialog early, a cut changes what was meant.
pub(super) fn dialog_text_refusal(name: &str, text: &str) -> Option<String> {
    if text.contains(['\n', '\r']) {
        return Some(format!("{name} is one line; a newline would submit the dialog early"));
    }
    if text.len() > DIALOG_TEXT_MAX {
        return Some(format!(
            "{name} is {} bytes; a dialog takes at most {DIALOG_TEXT_MAX}",
            text.len()
        ));
    }
    (!dialog_text_ok(text)).then(|| {
        format!("{name} is blank or carries characters a dialog cannot take; plain words only")
    })
}

/// Words a dialog's text row may take: one line, Remote Control's cap, and
/// nothing `sanitize_prompt` would change.
fn dialog_text_ok(text: &str) -> bool {
    !text.contains(['\n', '\r'])
        && text.len() <= DIALOG_TEXT_MAX
        && matches!(mesimon_core::command::sanitize_prompt(text), Ok(Some(t)) if t == text)
}

/// Whether `answer` is one `question` takes: one option of a one-choice
/// question, distinct options of a several-choice one, or a line of words.
fn answer_fits(question: &api::Question, answer: &api::QuestionAnswer) -> bool {
    let n = question.options.len();
    match answer {
        api::QuestionAnswer::Choice { index } => !question.multi_select && *index < n,
        api::QuestionAnswer::Choices { indices } => {
            let distinct: HashSet<&usize> = indices.iter().collect();
            question.multi_select
                && !indices.is_empty()
                && distinct.len() == indices.len()
                && indices.iter().all(|i| *i < n)
        }
        api::QuestionAnswer::Text { text } => dialog_text_ok(text),
    }
}

/// The plan an answer makes on `dialog`, or `None` when the dialog is not a
/// measured shape or the answer is not one it takes. The single answer an
/// older browser sends fits only the one-question, one-choice dialog.
fn dialog_plan(dialog: &api::Dialog, response: &api::DialogAnswer) -> Option<Plan> {
    use api::{DialogAnswer as A, QuestionAnswer as Q};
    let api::DialogContent::Questions { questions } = &dialog.content else {
        return match (response, &dialog.content) {
            (A::Accept, _) => Some(Plan::Accept),
            // A trust dialog's `No, exit` ends the agent (T-844): the pane's
            // alone, never a phone's button.
            (A::Reject, api::DialogContent::Trust { .. }) => None,
            (A::Reject, _) => Some(Plan::Reject),
            _ => None,
        };
    };
    if !dialog_answerable(questions) {
        return None;
    }
    let answers = match response {
        A::Reject => return Some(Plan::Reject),
        A::Accept => return None,
        _ if dialog_batched(questions) && !matches!(response, A::Answers { .. }) => return None,
        A::Choice { index } => vec![Q::Choice { index: *index }],
        A::Text { text } => vec![Q::Text { text: text.clone() }],
        A::Answers { answers } => answers.clone(),
    };
    let fits = answers.len() == questions.len()
        && questions.iter().zip(&answers).all(|(q, a)| answer_fits(q, a));
    match fits {
        false => None,
        true if dialog_batched(questions) => Some(Plan::Batch(answers)),
        true => answers.into_iter().next().map(Plan::Single),
    }
}

/// Text with every whitespace run removed: a label Claude Code wrapped, at a
/// space or inside a word, reads the same once its rows are joined.
fn squeeze(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// A line without the `│` Claude Code draws down the left of a question
/// that wraps (measured on 2.1.287, T-571: one question or a batch alike).
fn unbarred(line: &str) -> &str {
    let line = line.trim();
    line.strip_prefix('│').map_or(line, str::trim)
}

/// Whether `screen` shows the measured native dialog: the question and its
/// footer, the batch's tab bar over one of its questions or its review, or
/// the plan menu.
fn dialog_shape(dialog: &api::Dialog, screen: &str) -> bool {
    match &dialog.content {
        api::DialogContent::Questions { questions } if dialog_batched(questions) => {
            batch_view(questions, screen).is_some()
        }
        api::DialogContent::Questions { questions } => {
            let screen: String = screen.lines().map(|l| squeeze(unbarred(l))).collect();
            let question = questions.first().map(|q| squeeze(&q.question)).unwrap_or_default();
            !question.is_empty()
                && screen.contains(&question)
                && screen.contains("Entertoselect")
                && screen.contains("Esctocancel")
        }
        api::DialogContent::Plan { .. } => {
            let screen = squeeze(screen);
            screen.contains("Wouldyouliketoproceed?")
                && screen.contains("Yes,manuallyapproveedits")
                && screen.contains("TellClaudewhattochange")
        }
        api::DialogContent::Trust { .. } => crate::trust_dialog::read_text(screen).is_some(),
    }
}

/// One row of a dialog, with the lines under it joined on.
struct Row {
    /// `None` for a several-choice question's `Next` or `Submit` row, which
    /// Claude Code draws unnumbered under the text row (T-571).
    number: Option<usize>,
    selected: bool,
    /// A several-choice row's mark: `[✔]` ticked, `[ ]` not (T-571).
    ticked: Option<bool>,
    text: String,
}

/// Why a walk stopped, with the keys already sent on record: a receipt
/// after `steps` keys says the cursor is no longer where the dialog opened.
fn walk_reason(reason: &str, steps: u8) -> String {
    if steps > 0 {
        format!("{reason}; cursor moved")
    } else {
        reason.to_string()
    }
}

/// A numbered row's parts: whether the cursor `❯` is on it, its number and
/// the rest of the line.
fn numbered(line: &str) -> Option<(bool, usize, &str)> {
    let (selected, number, label) = crate::plan_dialog::menu_row(line);
    Some((selected, number?, label))
}

/// The dialog's options (T-567): the numbered rows from its last `1.` down
/// to the footer, numbered one by one. A line under a row that is neither
/// numbered, the footer nor a rule is the rest of that row: a wrapped label,
/// or the description drawn under it. A several-choice row carries its
/// mark, and the `Next`/`Submit` row under the ticks is a row of its own
/// (T-571). `None` when the numbering skips.
fn dialog_rows(screen: &str) -> Option<Vec<Row>> {
    dialog_rows_to(screen, "Enter to select")
}

/// `dialog_rows`, with the words that close the list: Claude Code's
/// `Enter to select`, Codex's `enter to submit answer` or `Press enter to
/// confirm` (T-846).
fn dialog_rows_to(screen: &str, footer: &str) -> Option<Vec<Row>> {
    let lines: Vec<&str> = screen.lines().collect();
    let footer = lines.iter().rposition(|l| l.contains(footer)).unwrap_or(lines.len());
    let mut rows: Vec<Row> = Vec::new();
    let mut open = false;
    for line in &lines[..footer] {
        let line = line.trim();
        if let Some((selected, number, label)) = numbered(line) {
            let (ticked, label) = match (label.strip_prefix("[✔]"), label.strip_prefix("[ ]")) {
                (Some(rest), _) => (Some(true), rest.trim()),
                (_, Some(rest)) => (Some(false), rest.trim()),
                _ => (None, label),
            };
            rows.push(Row { number: Some(number), selected, ticked, text: label.to_string() });
            open = true;
            continue;
        }
        let (selected, _, rest) = crate::plan_dialog::menu_row(line);
        let ticks = rows.last().is_some_and(|r| r.ticked.is_some());
        if ticks && matches!(rest, "Next" | "Submit") {
            rows.push(Row { number: None, selected, ticked: None, text: rest.to_string() });
            open = false;
        } else if line.chars().all(|c| ('\u{2500}'..='\u{257f}').contains(&c)) {
            open = false;
        } else if let (true, Some(row)) = (open, rows.last_mut()) {
            row.text.push(' ');
            row.text.push_str(line);
        }
    }
    let first = rows.iter().rposition(|r| r.number == Some(1))?;
    let rows = rows.split_off(first);
    let numbers: Vec<usize> = rows.iter().filter_map(|r| r.number).collect();
    numbers.iter().enumerate().all(|(i, n)| *n == i + 1).then_some(rows)
}

/// Why no row reads as `want`: wrapped when a row starts with `want`, or
/// `want` starts with the row up to where the pane cut it (`…`).
fn label_miss(rows: &[Row], want: &str) -> Miss {
    let want = squeeze(want);
    let wrapped = rows.iter().any(|r| {
        let row = squeeze(&r.text);
        let head = row.split('…').next().unwrap_or_default();
        !head.is_empty() && (row.starts_with(&want) || want.starts_with(head))
    });
    if wrapped {
        Miss::LabelWrapped
    } else {
        Miss::LabelNotFound
    }
}

/// Where the cursor is: the one selected row, or none of the measured shape.
fn cursor(rows: &[Row]) -> Result<usize, Miss> {
    let mut selected = rows.iter().enumerate().filter(|(_, r)| r.selected);
    match (selected.next(), selected.next()) {
        (Some((at, _)), None) => Ok(at),
        _ => Err(Miss::ShapeUnrecognised),
    }
}

/// The one row reading as an option: its label, or its label then the
/// description Claude Code draws under it, wrapping aside.
fn option_row(rows: &[Row], label: &str, description: &str) -> Result<usize, Miss> {
    let (bare, described) = (squeeze(label), squeeze(&format!("{label}{description}")));
    let mut found = rows.iter().enumerate().filter(|(_, r)| {
        let row = squeeze(&r.text);
        row == bare || (!description.is_empty() && row == described)
    });
    match (found.next(), found.next()) {
        (Some((at, _)), None) => Ok(at),
        _ => Err(label_miss(rows, label)),
    }
}

/// The key that takes the cursor from row `from` toward row `to`, or `here`
/// once it is there.
fn toward(from: usize, to: usize, here: DialogStep) -> DialogStep {
    match from.cmp(&to) {
        std::cmp::Ordering::Less => DialogStep::Down,
        std::cmp::Ordering::Greater => DialogStep::Up,
        std::cmp::Ordering::Equal => here,
    }
}

/// What a batch or several-choice dialog shows (T-571).
enum View {
    /// One question's tab: which question, and its rows.
    Question { index: usize, rows: Vec<Row> },
    /// The review: each question and its answer as drawn, squeezed, and the
    /// `Submit answers` and `Cancel` rows.
    Review { answers: Vec<(String, String)>, rows: Vec<Row> },
}

/// Read the tab a batch or several-choice dialog shows, as measured on
/// Claude Code 2.1.287 (T-571). Under the tab bar `←  ☐ Color  ☒ Toppings
/// ✔ Submit  →` is either a question — its words, which must be exactly one
/// projected question's, then its rows down to the footer — or the review:
/// `Review your answers`, a `● question` and `→ answer` pair per question,
/// `Ready to submit your answers?` and its two rows. Anything else is no
/// view.
fn batch_view(questions: &[api::Question], screen: &str) -> Option<View> {
    let lines: Vec<&str> = screen.lines().collect();
    let bar = lines.iter().rposition(|l| {
        let l = l.trim();
        l.starts_with('←') && l.ends_with('→') && l.contains("✔ Submit")
    })?;
    let body = &lines[bar + 1..];
    let first = body.iter().position(|l| !l.trim().is_empty())?;
    if squeeze(body[first]) == "Reviewyouranswers" {
        let ready = body.iter().position(|l| squeeze(l) == "Readytosubmityouranswers?")?;
        let mut answers: Vec<(String, String)> = Vec::new();
        let mut answering = false;
        for line in &body[first + 1..ready] {
            let line = unbarred(line);
            if let Some(question) = line.strip_prefix('●') {
                answers.push((squeeze(question), String::new()));
                answering = false;
            } else if let Some(answer) = line.strip_prefix('→') {
                let item = answers.last_mut().filter(|_| !answering)?;
                item.1 = squeeze(answer);
                answering = true;
            } else if let Some(item) = answers.last_mut() {
                let part = if answering { &mut item.1 } else { &mut item.0 };
                part.push_str(&squeeze(line));
            }
        }
        let rows = dialog_rows(&body[ready + 1..].join("\n"))?;
        return Some(View::Review { answers, rows });
    }
    let at =
        first + body[first..].iter().position(|l| numbered(l).is_some_and(|(_, n, _)| n == 1))?;
    let words: String = body[first..at].iter().map(|l| squeeze(unbarred(l))).collect();
    let mut asked = questions.iter().enumerate().filter(|(_, q)| squeeze(&q.question) == words);
    let (Some((index, _)), None) = (asked.next(), asked.next()) else { return None };
    let tail = body[at..].join("\n");
    let footer = squeeze(&tail);
    if !footer.contains("Entertoselect") || !footer.contains("Esctocancel") {
        return None;
    }
    Some(View::Question { index, rows: dialog_rows(&tail)? })
}

/// Whether the review's `drawn` answer (squeezed) reads as `answer`. The
/// review lists several choices in the order they were ticked, joined by
/// `, `, so their order is not compared.
fn reads_answer(question: &api::Question, answer: &api::QuestionAnswer, drawn: &str) -> bool {
    fn any_order(drawn: &str, labels: &[String]) -> bool {
        labels.iter().enumerate().any(|(i, label)| {
            let Some(rest) = drawn.strip_prefix(label.as_str()) else { return false };
            let mut others = labels.to_vec();
            others.remove(i);
            match others.is_empty() {
                true => rest.is_empty(),
                false => rest.strip_prefix(',').is_some_and(|rest| any_order(rest, &others)),
            }
        })
    }
    let label = |i: &usize| question.options.get(*i).map(|o| squeeze(&o.label));
    match answer {
        api::QuestionAnswer::Choice { index } => label(index).is_some_and(|l| l == drawn),
        api::QuestionAnswer::Text { text } => squeeze(text) == drawn,
        api::QuestionAnswer::Choices { indices } => {
            let labels: Option<Vec<String>> = indices.iter().map(label).collect();
            labels.is_some_and(|labels| !labels.is_empty() && any_order(drawn, &labels))
        }
    }
}

/// Whether `dialog` on a `kind` seat takes Codex's own shapes (T-846): every
/// Codex dialog but the folder trust, whose walk (`trust_step`, from
/// `trust_dialog::read_text`) reads Claude Code's and Codex's alike (T-871).
fn codex_shapes(kind: SessionKind, dialog: &api::Dialog) -> bool {
    kind == SessionKind::Codex && !matches!(dialog.content, api::DialogContent::Trust { .. })
}

/// `dialog_step` for the harness whose pane shows the dialog (T-846):
/// Claude Code's measured shapes, or Codex's, and a Codex approval's rows.
fn dialog_step_for(
    kind: SessionKind,
    standing: Standing<'_>,
    screen: &str,
    walk: &Walk,
) -> Result<DialogStep, Miss> {
    match standing {
        Standing::Approval(projection, allow) => codex_approval_step(screen, allow, projection),
        // A codex's plan sent back is unmeasured (T-943): refused at the
        // call, and no key here.
        Standing::Revise(..) if kind == SessionKind::Codex => Err(Miss::ShapeUnrecognised),
        Standing::Revise(dialog, text) => revise_step(dialog, text, screen, walk),
        Standing::Dialog(dialog, response) if codex_shapes(kind, dialog) => {
            codex_dialog_step(dialog, response, screen)
        }
        Standing::Dialog(dialog, response) => dialog_step(dialog, response, screen, walk),
    }
}

/// `dialog_shape` for the harness whose pane shows the dialog (T-846).
fn dialog_shape_for(kind: SessionKind, dialog: &api::Dialog, screen: &str) -> bool {
    if codex_shapes(kind, dialog) {
        codex_dialog_shape(dialog, screen)
    } else {
        dialog_shape(dialog, screen)
    }
}

/// The answers a Codex dialog takes from the board (T-846), as measured on
/// Codex 0.163.0: one question's one choice (its row and Enter), a refusal
/// (Esc, which Codex reads as an interrupt) and a plan's accept (Enter on
/// `Yes, implement this plan`, T-420). Words typed into the question's
/// notes, several questions, several choices and a plan sent back are
/// unmeasured and stay the pane's.
fn codex_takes(dialog: &api::Dialog, response: &api::DialogAnswer) -> bool {
    match (&dialog.content, dialog_plan(dialog, response)) {
        (api::DialogContent::Questions { questions }, Some(Plan::Single(answer))) => {
            questions.len() == 1 && matches!(answer, api::QuestionAnswer::Choice { .. })
        }
        (api::DialogContent::Questions { .. }, Some(Plan::Reject)) => true,
        (api::DialogContent::Plan { .. }, Some(Plan::Accept)) => true,
        _ => false,
    }
}

/// The question shapes the board answers on a Codex seat (T-846): one
/// question with options, as `codex_takes` admits.
pub(super) fn dialog_answerable_for(kind: SessionKind, questions: &[api::Question]) -> bool {
    match kind {
        SessionKind::Codex => {
            questions.len() == 1
                && !questions[0].multi_select
                && !questions[0].options.is_empty()
                && dialog_answerable(questions)
        }
        _ => dialog_answerable(questions),
    }
}

/// Codex's question dialog, as measured on 0.163.0 (T-846): the question's
/// words, numbered rows (`label   description`, the cursor `›` on one), a
/// `None of the above` row Codex adds, and the footer `tab to add notes |
/// enter to submit answer | esc to interrupt`. The plan dialog is
/// `codex::plan_dialog`'s.
fn codex_dialog_shape(dialog: &api::Dialog, screen: &str) -> bool {
    match &dialog.content {
        api::DialogContent::Questions { questions } => {
            let squeezed: String = screen.lines().map(squeeze).collect();
            let question = questions.first().map(|q| squeeze(&q.question)).unwrap_or_default();
            questions.len() == 1
                && !question.is_empty()
                && squeezed.contains(&question)
                && squeezed.contains("entertosubmitanswer")
        }
        api::DialogContent::Plan { .. } => crate::agents::codex::plan_dialog(&input_screen(screen)),
        // The folder trust dialog never reaches Codex's shapes: `codex_shapes`
        // leaves it to the trust walk both harnesses share (T-871).
        api::DialogContent::Trust { .. } => false,
    }
}

/// A captured screen as the backend's `InputScreen`, without a cursor: the
/// plan-dialog check wants the lines alone.
fn input_screen(screen: &str) -> mesimon_backend_tmux::InputScreen {
    mesimon_backend_tmux::InputScreen {
        lines: screen.lines().map(str::to_string).collect(),
        cursor: None,
    }
}

/// One key of an answer into Codex's dialog (T-846): the cursor toward the
/// chosen option's row, then Enter (`enter to submit answer`); Esc for a
/// refusal; for a plan, Enter where the dialog stands at its `Yes` row
/// (`plan_dialog::codex_at_default`). Anything `codex_takes` does not admit
/// is no measured shape.
fn codex_dialog_step(
    dialog: &api::Dialog,
    response: &api::DialogAnswer,
    screen: &str,
) -> Result<DialogStep, Miss> {
    let plan = dialog_plan(dialog, response).ok_or(Miss::ShapeUnrecognised)?;
    if !codex_takes(dialog, response) || !codex_dialog_shape(dialog, screen) {
        return Err(Miss::ShapeUnrecognised);
    }
    match (&dialog.content, plan) {
        (api::DialogContent::Plan { .. }, Plan::Accept) => {
            crate::plan_dialog::codex_at_default(&input_screen(screen))
                .map(|_| DialogStep::Submit)
                .map_err(|_| Miss::ShapeUnrecognised)
        }
        (api::DialogContent::Questions { .. }, Plan::Reject) => {
            let rows =
                dialog_rows_to(screen, "enter to submit answer").ok_or(Miss::ShapeUnrecognised)?;
            cursor(&rows)?;
            Ok(DialogStep::Reject)
        }
        (
            api::DialogContent::Questions { questions },
            Plan::Single(api::QuestionAnswer::Choice { index }),
        ) => {
            let option = questions[0].options.get(index).ok_or(Miss::ShapeUnrecognised)?;
            let rows =
                dialog_rows_to(screen, "enter to submit answer").ok_or(Miss::ShapeUnrecognised)?;
            let current = cursor(&rows)?;
            let target = option_row(&rows, &option.label, &option.description)?;
            Ok(toward(current, target, DialogStep::Submit))
        }
        _ => Err(Miss::ShapeUnrecognised),
    }
}

/// One key of an approval's answer into Codex's dialog (T-846), as measured
/// on 0.163.0: numbered rows under the request, the first `Yes, proceed
/// (y)`, the last `No, and tell Codex what to do differently (esc)`, the
/// cursor `›` on one, and the footer `Press enter to confirm or esc to
/// cancel`. An allow walks to the first `Yes` row, a deny to the `No` row,
/// and Enter confirms. The heading is not read: a command's, an edit's and
/// a permission's dialogs share the rows. The key is bound to the request
/// the phone answered, not its rows alone: the screen must show the
/// request's reason (`Reason: …`), or, without one, the end of its command
/// (the item spells `/bin/zsh -lc '…'`, the dialog the words inside), so a
/// dialog that replaced it between the tap and the key gets nothing; a
/// request with neither is no shape the board presses. The request's
/// standing is re-read each key from the projection the snapshot keeps.
fn codex_approval_step(
    screen: &str,
    allow: bool,
    projection: &api::Permission,
) -> Result<DialogStep, Miss> {
    let squeezed: String = screen.lines().map(squeeze).collect();
    if !squeezed.contains("Pressentertoconfirm") {
        return Err(Miss::ShapeUnrecognised);
    }
    let reason = projection.input["reason"].as_str().map(squeeze).filter(|r| !r.is_empty());
    let command = projection.input["command"].as_str().map(|c| {
        let c = squeeze(c);
        let c = c.trim_end_matches(['\'', '"']);
        let start = c.char_indices().rev().nth(47).map_or(0, |(i, _)| i);
        c[start..].to_string()
    });
    let bound = match (reason, command) {
        (Some(reason), _) => squeezed.contains(&reason),
        (None, Some(tail)) if !tail.is_empty() => squeezed.contains(&tail),
        _ => false,
    };
    if !bound {
        return Err(Miss::LabelNotFound);
    }
    let rows = dialog_rows_to(screen, "Press enter to confirm").ok_or(Miss::ShapeUnrecognised)?;
    let current = cursor(&rows)?;
    let yes = rows.iter().position(|r| r.text.starts_with("Yes"));
    let no = rows.iter().rposition(|r| r.text.starts_with("No"));
    let (Some(yes), Some(no)) = (yes, no) else { return Err(Miss::ShapeUnrecognised) };
    if rows.iter().filter(|r| r.text.starts_with("No")).count() != 1 {
        return Err(Miss::ShapeUnrecognised);
    }
    Ok(toward(current, if allow { yes } else { no }, DialogStep::Submit))
}

/// Recognize only measured native menus, reading the whole dialog (T-567).
/// A row reads as an option when, wrapping aside, it is the option's label,
/// or its label and then its description. Missing, ambiguous or several
/// selections, unmeasured shapes and MCP forms yield no key. A batch or a
/// several-choice question walks its tabs (T-571). This never infers
/// success: only the hook edge does.
fn dialog_step(
    dialog: &api::Dialog,
    response: &api::DialogAnswer,
    screen: &str,
    walk: &Walk,
) -> Result<DialogStep, Miss> {
    let plan = dialog_plan(dialog, response).ok_or(Miss::ShapeUnrecognised)?;
    if let api::DialogContent::Trust { .. } = &dialog.content {
        return trust_step(&plan, screen);
    }
    if let api::DialogContent::Questions { questions } = &dialog.content {
        if dialog_batched(questions) {
            return batch_step(questions, &plan, screen, walk);
        }
    }
    let (label, description) = match (&dialog.content, &plan) {
        (api::DialogContent::Questions { questions }, Plan::Single(answer)) => match answer {
            api::QuestionAnswer::Choice { index } => {
                let option = &questions[0].options[*index];
                (option.label.clone(), option.description.clone())
            }
            api::QuestionAnswer::Text { .. } => ("Type something.".into(), String::new()),
            api::QuestionAnswer::Choices { .. } => return Err(Miss::ShapeUnrecognised),
        },
        (_, Plan::Accept) => ("Yes, manually approve edits".into(), String::new()),
        (_, Plan::Reject) => (String::new(), String::new()),
        _ => return Err(Miss::ShapeUnrecognised),
    };
    if !dialog_shape(dialog, screen) {
        return Err(Miss::ShapeUnrecognised);
    }
    let rows = dialog_rows(screen).ok_or(Miss::ShapeUnrecognised)?;
    let current = cursor(&rows)?;
    if plan == Plan::Reject {
        return Ok(DialogStep::Reject);
    }
    if let Plan::Single(api::QuestionAnswer::Text { text }) = &plan {
        if walk.pasted.contains(&0) {
            // Never Enter on a row that does not read the pasted text: on the
            // empty field it would decline the question instead.
            return if squeeze(&rows[current].text) == squeeze(text) {
                Ok(DialogStep::Submit)
            } else {
                Err(label_miss(&rows[current..=current], text))
            };
        }
        let target = option_row(&rows, &label, "")?;
        return Ok(toward(current, target, DialogStep::Paste { question: 0, text: text.clone() }));
    }
    let target = option_row(&rows, &label, &description)?;
    Ok(toward(current, target, DialogStep::Submit))
}

/// The plan dialog's feedback row as Claude Code draws it (measured on
/// 2.1.310, T-943): `N. Tell Claude what to change` while its field is
/// empty, the field's words in that place once typed or pasted, and under it
/// the hint `shift+tab to approve with this feedback` and the `ctrl+g to
/// edit in <editor> · <plan file>` line, which `dialog_rows` joins onto it.
const FEEDBACK_ROW: &str = "Tell Claude what to change";
const APPROVE_HINT: &str = "shift+tab to approve with this feedback";
const EDITOR_LINE: &str = "ctrl+g to edit in ";

/// One row of the plan dialog's menu as the walk reads it (T-943): whether
/// the cursor is on it, and its field, squeezed.
struct MenuRow {
    selected: bool,
    field: String,
}

/// The plan dialog's menu, parsed for the walk alone (T-943), as one extent:
/// it begins after the line that completes the dialog's question (`Would you
/// like to proceed?`, wrapped or not), and its rows are the numbered lines
/// after it, numbered 1, 2, 3 … with no skip and no restart. A numbered line
/// out of that order (a later `1.` among them) and a second question after
/// the first are a person's draft the walk cannot tell from the menu: `None`,
/// and the walk stops. A row's field is its numbered line and every line
/// under it to the next row, and the last row's runs to the end of the
/// screen, through blank and box-drawing lines, since a person's draft in the
/// native field may hold any of them. No footer is cut: 2.1.310 draws none,
/// and a line that merely holds `Enter to select` is the field's. From the
/// end of the last row come off trailing blank lines, then the editor line
/// (`ctrl+g to edit in <editor> · <file>`) with the approve hint line
/// directly above it when it is there, never the row's own first line. The
/// approve hint without its editor line under it is the field's. Whatever
/// is left is the field, compared whole. The cursor is read from the same
/// rows.
fn plan_menu(screen: &str) -> Option<Vec<MenuRow>> {
    let lines: Vec<&str> = screen.lines().map(str::trim).collect();
    let question = squeeze("Would you like to proceed?");
    let mut said = String::new();
    let asked = lines.iter().position(|line| {
        said.push_str(&squeeze(line));
        said.contains(&question)
    })?;
    let after: String = lines[asked + 1..].iter().map(|line| squeeze(line)).collect();
    if after.contains(&question) {
        return None;
    }
    let mut rows: Vec<(bool, Vec<&str>)> = Vec::new();
    for line in &lines[asked + 1..] {
        match numbered(line) {
            Some((selected, number, label)) if number == rows.len() + 1 => {
                rows.push((selected, vec![label]))
            }
            Some(_) => return None,
            None => {
                if let Some((_, block)) = rows.last_mut() {
                    block.push(line);
                }
            }
        }
    }
    let last = rows.len().checked_sub(1)?;
    for (at, (_, block)) in rows.iter_mut().enumerate() {
        while block.len() > 1 && block.last().is_some_and(|l| l.is_empty()) {
            block.pop();
        }
        if at != last {
            continue;
        }
        // The pair, or nothing: the approve hint is the hint only directly
        // above the editor line it is drawn with; alone, it is the field's.
        if block.len() > 2
            && block.last().is_some_and(|l| editor_line(l))
            && block[block.len() - 2] == APPROVE_HINT
        {
            block.truncate(block.len() - 2);
        } else if block.len() > 1 && block.last().is_some_and(|l| editor_line(l)) {
            block.pop();
        }
    }
    Some(
        rows.into_iter()
            .map(|(selected, block)| MenuRow { selected, field: squeeze(&block.concat()) })
            .collect(),
    )
}

/// The one row the cursor is on, or none of the measured shape.
fn menu_cursor(rows: &[MenuRow]) -> Result<usize, Miss> {
    let mut selected = rows.iter().enumerate().filter(|(_, r)| r.selected);
    match (selected.next(), selected.next()) {
        (Some((at, _)), None) => Ok(at),
        _ => Err(Miss::ShapeUnrecognised),
    }
}

/// The editor line under the plan dialog, as measured on 2.1.310:
/// `ctrl+g to edit in Nvim · ~/.claude/plans/<name>.md`.
fn editor_line(line: &str) -> bool {
    line.strip_prefix(EDITOR_LINE).is_some_and(|rest| rest.contains(" · "))
}

/// Whether `screen` draws Claude Code's plan dialog, its feedback field
/// empty or not (T-943): the question and the manual accept row, which every
/// measured version shows.
fn plan_dialog_drawn(screen: &str) -> bool {
    let screen = squeeze(screen);
    screen.contains("Wouldyouliketoproceed?") && screen.contains("Yes,manuallyapproveedits")
}

/// What a plan sent back settles as once its Enter went (T-943), or `None`
/// while it waits: an edge the refusal way, or the projection dropped, is
/// the dialog gone (`input_sent`); an accept that ended it is not this
/// answer (`unknown`, `plan_accepted`). At the window's end with the
/// projection still standing, only a screen read that succeeded can say the
/// dialog is gone, and only by showing what it left for: Claude's composer
/// (T-955). One that shows the plan is `dialog_stands`, any other screen
/// (blank, clipped, unfamiliar) is `shape_unrecognised`, and a capture that
/// failed is `pane_unreachable`. `capture` is read at most once, then.
fn revise_settled(
    edge: Option<DialogEdge>,
    standing: bool,
    due: bool,
    capture: impl FnOnce() -> Option<String>,
) -> Option<(&'static str, Option<&'static str>)> {
    match edge {
        Some(DialogEdge::Answered | DialogEdge::ModAnswered) => {
            Some(("unknown", Some("plan_accepted")))
        }
        Some(DialogEdge::Dismissed) => Some(("input_sent", None)),
        None if !standing => Some(("input_sent", None)),
        None if !due => None,
        None => match capture() {
            None => Some(("unknown", Some("pane_unreachable"))),
            Some(screen) if plan_dialog_drawn(&screen) => Some(("unknown", Some("dialog_stands"))),
            Some(screen) if composer_drawn(&screen) => Some(("input_sent", None)),
            Some(_) => Some(("unknown", Some("shape_unrecognised"))),
        },
    }
}

/// Whether `screen` shows Claude's composer (T-955), read as a launch's
/// words read it (`agents::claude::composer`): the screen a plan's dialog
/// leaves for, and one a dialog never reads as.
fn composer_drawn(screen: &str) -> bool {
    use crate::agents::claude::composer::{self, Composer};
    let lines = screen.lines().map(str::to_string).collect();
    composer::read(&mesimon_backend_tmux::InputScreen { lines, cursor: None }) != Composer::Absent
}

/// The plan dialog's cursor row on `screen`, if it shows exactly one.
fn plan_cursor(screen: &str) -> Option<usize> {
    plan_menu(screen).and_then(|rows| menu_cursor(&rows).ok())
}

/// One key of a plan sent back with words (T-943), read off the screen
/// before every key, the same for both roads (the mod road draws the native
/// dialog too). Before the paste: the cursor where the last arrow put it,
/// the one row whose field is the untouched `Tell Claude what to change`,
/// arrows toward it, and the paste only with the cursor on it. After: the
/// cursor still there and the field reading the words whole (squeezed, the
/// row's hints aside), then Enter; a field still empty is the paste not yet
/// drawn. Anything else sends nothing: words that are not the crown's are a
/// person's, never added to, and no other row is ever pressed.
fn revise_step(
    dialog: &api::Dialog,
    text: &str,
    screen: &str,
    walk: &Walk,
) -> Result<DialogStep, Miss> {
    if !matches!(dialog.content, api::DialogContent::Plan { .. }) || !plan_dialog_drawn(screen) {
        return Err(Miss::ShapeUnrecognised);
    }
    let rows = plan_menu(screen).ok_or(Miss::ShapeUnrecognised)?;
    let current = menu_cursor(&rows)?;
    let fields: Vec<&String> = rows.iter().map(|r| &r.field).collect();
    let label = squeeze(FEEDBACK_ROW);
    let Some(&row) = walk.pasted.first() else {
        if walk.at.is_some_and(|at| at != current) {
            return Err(Miss::CursorMoved);
        }
        let mut empty = fields.iter().enumerate().filter(|(_, f)| ***f == label);
        return match (empty.next(), empty.next()) {
            // The feedback row closes the menu; a numbered line after it is
            // a draft's, which the walk cannot tell from a row.
            (Some((target, _)), None) if target + 1 != fields.len() => Err(Miss::FieldTouched),
            (Some((target, _)), None) if target == current => {
                Ok(DialogStep::Paste { question: target, text: text.to_string() })
            }
            (Some((target, _)), None) => Ok(toward(current, target, DialogStep::Wait)),
            // The plan dialog is drawn and no row reads the empty field:
            // someone has typed into it.
            _ => Err(Miss::FieldTouched),
        };
    };
    if current != row {
        return Err(Miss::CursorMoved);
    }
    if row + 1 != fields.len() {
        return Err(Miss::FieldTouched);
    }
    match fields.get(row) {
        Some(field) if **field == squeeze(text) => Ok(DialogStep::Submit),
        Some(field) if **field == label => Ok(DialogStep::Wait),
        _ => Err(Miss::FieldTouched),
    }
}

/// One key of a trust dialog's accept (T-844): the dialog's own Yes row,
/// reached by its arrows and pressed by Enter, from `trust_dialog`'s read
/// of the screen before every key. A cursor on neither row, or a screen
/// that is not the measured dialog, sends nothing. No refusal road: the
/// dialog's `No, exit` ends the agent and stays the pane's.
fn trust_step(plan: &Plan, screen: &str) -> Result<DialogStep, Miss> {
    use crate::trust_dialog::{self, Step};
    if *plan != Plan::Accept {
        return Err(Miss::ShapeUnrecognised);
    }
    let dialog = trust_dialog::read_text(screen).ok_or(Miss::ShapeUnrecognised)?;
    match dialog.toward_yes().ok_or(Miss::ShapeUnrecognised)? {
        Step::Up => Ok(DialogStep::Up),
        Step::Down => Ok(DialogStep::Down),
        Step::Enter => Ok(DialogStep::Submit),
    }
}

/// One key of a batch or several-choice answer (T-571), off the tab the
/// screen shows. A one-choice tab is the single road with Enter moving on.
/// A several-choice tab ticks its options top to bottom until each reads as
/// the answer, Space once per row (a mark that did not change stops the
/// walk), types the words into its text row when the answer is words, and
/// presses its `Next`/`Submit` row. The review is pressed only when every
/// answer it lists reads as the one sent. A text row a person filled is
/// never cleared: that is no measured shape.
fn batch_step(
    questions: &[api::Question],
    plan: &Plan,
    screen: &str,
    walk: &Walk,
) -> Result<DialogStep, Miss> {
    let view = batch_view(questions, screen).ok_or(Miss::ShapeUnrecognised)?;
    let answers = match plan {
        Plan::Batch(answers) => answers,
        Plan::Reject => {
            let (View::Question { rows, .. } | View::Review { rows, .. }) = &view;
            return cursor(rows).map(|_| DialogStep::Reject);
        }
        _ => return Err(Miss::ShapeUnrecognised),
    };
    let (index, rows) = match view {
        View::Review { answers: drawn, rows } => {
            if drawn.len() != questions.len()
                || questions
                    .iter()
                    .zip(&drawn)
                    .any(|(q, (words, _))| squeeze(&q.question) != *words)
            {
                return Err(Miss::ShapeUnrecognised);
            }
            if questions
                .iter()
                .zip(answers)
                .zip(&drawn)
                .any(|((q, a), (_, drawn))| !reads_answer(q, a, drawn))
            {
                return Err(Miss::AnswerDiffers);
            }
            let current = cursor(&rows)?;
            let submit = rows
                .iter()
                .position(|r| r.number == Some(1) && squeeze(&r.text) == "Submitanswers")
                .ok_or(Miss::ShapeUnrecognised)?;
            return Ok(toward(current, submit, DialogStep::Submit));
        }
        View::Question { index, rows } => (index, rows),
    };
    let (question, answer) = (&questions[index], &answers[index]);
    let current = cursor(&rows)?;
    let n = question.options.len();
    let at = |number: usize| rows.iter().position(|r| r.number == Some(number));
    let text_row = at(n + 1).ok_or(Miss::ShapeUnrecognised)?;
    if !question.multi_select {
        if rows.iter().any(|r| r.ticked.is_some() || r.number.is_none()) {
            return Err(Miss::ShapeUnrecognised);
        }
        return match answer {
            api::QuestionAnswer::Choice { index: i } => {
                let option = &question.options[*i];
                let target = option_row(&rows, &option.label, &option.description)?;
                Ok(toward(current, target, DialogStep::Next))
            }
            api::QuestionAnswer::Text { text } if walk.pasted.contains(&index) => {
                if current == text_row && squeeze(&rows[current].text) == squeeze(text) {
                    Ok(DialogStep::Next)
                } else {
                    Err(label_miss(&rows[text_row..=text_row], text))
                }
            }
            api::QuestionAnswer::Text { text } => {
                if squeeze(&rows[text_row].text) != "Typesomething." {
                    return Err(Miss::LabelNotFound);
                }
                Ok(toward(
                    current,
                    text_row,
                    DialogStep::Paste { question: index, text: text.clone() },
                ))
            }
            api::QuestionAnswer::Choices { .. } => Err(Miss::ShapeUnrecognised),
        };
    }
    let button = rows.iter().position(|r| r.number.is_none()).ok_or(Miss::ShapeUnrecognised)?;
    if rows[text_row].ticked.is_none() {
        return Err(Miss::ShapeUnrecognised);
    }
    // Every option row is read before any key: its label, and a mark.
    let mut marks = Vec::with_capacity(question.options.len());
    for (i, option) in question.options.iter().enumerate() {
        let row = at(i + 1).ok_or(Miss::ShapeUnrecognised)?;
        option_row(&rows[row..=row], &option.label, &option.description)
            .map_err(|_| label_miss(&rows, &option.label))?;
        let ticked = rows[row].ticked.ok_or(Miss::ShapeUnrecognised)?;
        let want =
            matches!(answer, api::QuestionAnswer::Choices { indices } if indices.contains(&i));
        marks.push((row, ticked == want));
    }
    if let Some(&(row, _)) = marks.iter().find(|(_, right)| !right) {
        return match (current == row, walk.toggled == Some((index, row))) {
            (false, _) => Ok(toward(current, row, DialogStep::Up)),
            (true, true) => Err(Miss::TickNotTaken),
            (true, false) => Ok(DialogStep::Toggle { question: index, row }),
        };
    }
    let typed = &rows[text_row];
    match answer {
        api::QuestionAnswer::Text { text } => {
            let reads = squeeze(&typed.text) == squeeze(text);
            if reads && typed.ticked == Some(true) {
                // The words are in and ticked.
            } else if walk.pasted.contains(&index) {
                return Err(label_miss(&rows[text_row..=text_row], text));
            } else if squeeze(&typed.text) == "Typesomething" && typed.ticked == Some(false) {
                let paste = DialogStep::Paste { question: index, text: text.clone() };
                return Ok(toward(current, text_row, paste));
            } else {
                return Err(Miss::LabelNotFound);
            }
        }
        _ if typed.ticked == Some(true) => return Err(Miss::ShapeUnrecognised),
        _ => {}
    }
    Ok(toward(current, button, DialogStep::Next))
}

/// The dialog a plan or question call's input draws, within the bounds
/// measured for it: only the two dialogs (a Write's input is the file),
/// questions at most 16 KiB, one to four of one to eight options each, a
/// plan with words. The hook stream's road and the transcript's (T-933)
/// both. A plan is kept whole, however long (T-936: a 26 KiB design plan
/// was refused here, so the crown could neither read nor accept it): each
/// reader cuts it to what it carries, marked (`plan_head`).
fn dialog_content(tool: &str, input: &serde_json::Value) -> Option<api::DialogContent> {
    match tool {
        "AskUserQuestion" => {
            if serde_json::to_vec(input).map_or(true, |b| b.len() > 16 * 1024) {
                return None;
            }
            let questions =
                serde_json::from_value::<Vec<api::Question>>(input["questions"].clone()).ok()?;
            if questions.is_empty()
                || questions.len() > 4
                || questions.iter().any(|q| q.options.is_empty() || q.options.len() > 8)
            {
                return None;
            }
            Some(api::DialogContent::Questions { questions })
        }
        "ExitPlanMode" => {
            let markdown = input["plan"].as_str().filter(|p| !p.trim().is_empty())?;
            Some(api::DialogContent::Plan { markdown: markdown.into() })
        }
        _ => None,
    }
}

/// The most a plan's dialog takes of a phone's board answer, as the JSON it
/// is sent as (T-936): its escaping, its cut mark, its request and its own
/// fields included. One board answer fits 48 KiB (`Control::answer`), and
/// its trim drops no dialog; a 16 KiB head counted in raw bytes can escape
/// to six times that.
const PHONE_DIALOG_MAX_BYTES: usize = 16 * 1024;

/// A plan's markdown as a reader that carries at most `max` bytes of it
/// shows it (T-936): whole, or its head and a line that says it was cut.
pub(super) fn plan_head(markdown: &str, max: usize) -> String {
    let head = mesimon_core::text::cap_bytes(markdown, max);
    if head.len() == markdown.len() {
        return markdown.to_string();
    }
    format!(
        "{head}\n\n… the plan is cut here: its first {} KiB of {} KiB",
        head.len().div_ceil(1024),
        markdown.len().div_ceil(1024)
    )
}

/// A dialog as a phone's board carries it: a plan's head, as long as the
/// dialog's JSON stays within `PHONE_DIALOG_MAX_BYTES`, marked cut, and its
/// request the one an answer binds to.
fn phone_dialog(dialog: &api::Dialog) -> api::Dialog {
    let api::DialogContent::Plan { markdown } = &dialog.content else { return dialog.clone() };
    let sized = |head: usize| {
        let content = api::DialogContent::Plan { markdown: plan_head(markdown, head) };
        let cut = api::Dialog { request: dialog.request.clone(), content };
        let bytes = serde_json::to_vec(&cut).map_or(usize::MAX, |b| b.len());
        (cut, bytes)
    };
    let (whole, bytes) = sized(markdown.len());
    if bytes <= PHONE_DIALOG_MAX_BYTES {
        return whole;
    }
    // The longest head that fits, found by halving: its JSON grows with it.
    let (mut fits, mut over) = (0, markdown.len().min(PHONE_DIALOG_MAX_BYTES));
    while fits + 1 < over {
        let mid = (fits + over) / 2;
        if sized(mid).1 <= PHONE_DIALOG_MAX_BYTES {
            fits = mid;
        } else {
            over = mid;
        }
    }
    sized(fits).0
}

/// Whether this hook frame ends `dialog` (T-567), and how: its own tool's
/// `PostToolUse` says it took an answer; another tool's `PreToolUse` says it
/// is gone with none taken, the refusal road (T-447). Nothing else does.
fn dialog_edge(frame: &HookFrame, dialog: &api::Dialog) -> Option<DialogEdge> {
    // A trust dialog has no tool (T-844): the agent starting is its answer
    // taken, and nothing else the hooks say is about it.
    if let api::DialogContent::Trust { .. } = dialog.content {
        return (frame.event == "SessionStart").then_some(DialogEdge::Answered);
    }
    let own = dialog_own_call(frame, dialog);
    match frame.event.as_str() {
        // The mod's `answered` report rides in as a `PostToolUse` of its own
        // road with its reason kept (`on_mod_answer`), the only one a dialog
        // the mod closed has. A relayed `PostToolUse` of the engine's has no
        // reason: the person's answer, on the mod road too (T-577).
        "PostToolUse"
            if own
                && frame.road == mesimon_core::road::Road::Mod
                && frame.reason.as_deref() == Some("answered") =>
        {
            Some(DialogEdge::ModAnswered)
        }
        "PostToolUse" if own => Some(DialogEdge::Answered),
        "PreToolUse" if !own => Some(DialogEdge::Dismissed),
        _ => None,
    }
}

/// Whether this frame ends `dialog` with no answer to confirm: the turn or
/// the session ending, or its own call failing or denied. Another tool's
/// end never does (T-595): a model sends a question beside other tools in
/// one parallel batch, and they finish while the dialog stands.
fn dialog_dropped(frame: &HookFrame, dialog: &api::Dialog) -> bool {
    match frame.event.as_str() {
        "UserPromptSubmit" | "SessionStart" | "SessionEnd" | "PaneDied" | "Stop"
        | "StopFailure" => true,
        "PostToolUseFailure" | "PermissionDenied" => dialog_own_call(frame, dialog),
        _ => false,
    }
}

/// Whether a tool frame is the dialog's own call (T-595): its tool, and its
/// `tool_use_id` where the frame carries one.
fn dialog_own_call(frame: &HookFrame, dialog: &api::Dialog) -> bool {
    let tool = match dialog.content {
        api::DialogContent::Questions { .. } => "AskUserQuestion",
        api::DialogContent::Plan { .. } => "ExitPlanMode",
        api::DialogContent::Trust { .. } => return false,
    };
    frame.payload["tool_name"].as_str() == Some(tool)
        && frame.payload["tool_use_id"].as_str().is_none_or(|c| c == dialog.request)
}

#[cfg(test)]
mod tests {
    use super::*;
    /// A dialog's questions, none for a plan.
    fn questions_of(dialog: &api::Dialog) -> &[api::Question] {
        match &dialog.content {
            api::DialogContent::Questions { questions } => questions,
            api::DialogContent::Plan { .. } | api::DialogContent::Trust { .. } => &[],
        }
    }
    /// A walk that has pasted into the first question's text row.
    fn pasted() -> Walk {
        Walk { pasted: vec![0], ..Walk::default() }
    }
    fn question() -> api::Dialog {
        api::Dialog {
            request: "tool-1".into(),
            content: api::DialogContent::Questions {
                questions: vec![api::Question {
                    question: "Which color?".into(),
                    header: "Color".into(),
                    multi_select: false,
                    options: vec![
                        api::QuestionOption { label: "Blue".into(), description: "".into() },
                        api::QuestionOption { label: "Green".into(), description: "".into() },
                    ],
                }],
            },
        }
    }

    /// T-936: a plan is kept whole, however long, and each reader cuts it
    /// to what it carries, marked; questions keep their 16 KiB bound. A
    /// phone's head is bounded by the dialog's JSON, escaping, mark and
    /// request included, so the board answer it rides fits
    /// `Control::answer`'s 48 KiB: an ASCII plan, an escape-heavy one (a
    /// control byte escapes to six) and a long Unicode one.
    #[test]
    fn a_long_plan_is_kept_whole_and_cut_where_it_is_shown() {
        let long: String = (0..2000).map(|n| format!("{n}. a step of the design\n")).collect();
        assert!(long.len() > 40 * 1024);
        let Some(api::DialogContent::Plan { markdown }) =
            dialog_content("ExitPlanMode", &serde_json::json!({ "plan": long }))
        else {
            panic!("a long plan is a dialog");
        };
        assert_eq!(markdown, long, "kept whole");
        assert_eq!(plan_head("1. short", 16 * 1024), "1. short", "a short plan is not marked");
        let escaped: String = "\u{1}\"\\\t".repeat(10_000);
        let unicode: String =
            (0..1500).map(|n| format!("{n}. שלב בתכנון 🚀 עם טבלה | 表 | ✓\n")).collect();
        assert!(unicode.len() > 64 * 1024);
        for plan in [&long, &escaped, &unicode] {
            let dialog = api::Dialog {
                request: "toolu_1".into(),
                content: api::DialogContent::Plan { markdown: plan.clone() },
            };
            let phone = phone_dialog(&dialog);
            assert_eq!(phone.request, "toolu_1", "an answer binds to the same request");
            let bytes = serde_json::to_vec(&phone).unwrap().len();
            assert!(bytes <= PHONE_DIALOG_MAX_BYTES, "{bytes} bytes");
            let api::DialogContent::Plan { markdown: head } = &phone.content else { panic!() };
            let (cut, mark) = head.split_once("\n\n… the plan is cut here: ").expect("marked");
            assert!(plan.starts_with(cut) && !cut.is_empty(), "its head");
            assert!(mark.ends_with(&format!("of {} KiB", plan.len().div_ceil(1024))), "{mark}");
            // The board answer it rides, two agents on such plans at once.
            let ticket = |n: u32| api::Ticket {
                queued: None,
                queue: None,
                id: format!("01J{n}"),
                key: format!("T-{n}"),
                title: "a long plan".into(),
                column: "IN PROGRESS".into(),
                agent: Some(api::Agent {
                    permission: None,
                    dialog: Some(phone.clone()),
                    trust: None,
                    session: uuid::Uuid::nil().to_string(),
                    provider: "claude".into(),
                    state: "needs_you".into(),
                    promptable: false,
                    since: None,
                    doing: None,
                    said: None,
                    tier: None,
                }),
                tags: Vec::new(),
                picked: None,
                notes: 0,
                noted: String::new(),
                crown: false,
                crowned: None,
                workspace: None,
                tier: None,
                created: None,
                shell: None,
            };
            let reply = Reply::Board {
                title: "b".into(),
                columns: vec!["IN PROGRESS".into()],
                tickets: (1..=2).map(ticket).collect(),
                default_column: None,
                column_descriptions: BTreeMap::new(),
                allowed_tags: vec![],
                tiers: vec![],
                archived: vec![],
            };
            let answer = measure(&reply).unwrap();
            assert!(answer <= 48 * 1024, "the board answer: {answer} bytes");
        }
        let many = serde_json::json!({ "questions": [{
            "question": "x".repeat(17 * 1024), "header": "h",
            "options": [{ "label": "a", "description": "" }], "multiSelect": false }] });
        assert!(dialog_content("AskUserQuestion", &many).is_none(), "questions keep 16 KiB");
        assert!(dialog_content("Write", &serde_json::json!({ "plan": "1." })).is_none());
    }

    /// The folder trust dialog's accept (T-844): its own arrows toward the
    /// Yes row and Enter on it, read off the screen each time; a refusal is
    /// no answer of the board's, and an unmarked or unknown screen sends
    /// nothing. Codex's dialog walks the same way, on a Codex seat too.
    #[test]
    fn a_trust_dialog_walks_to_yes_and_takes_no_refusal() {
        let trust = api::Dialog {
            request: "trust-1".into(),
            content: api::DialogContent::Trust { folder: Some("/x".into()) },
        };
        let opened = " Accessing workspace: /x\n\n Quick safety check: Is this a project you created or one you trust?\n\n ❯ No, exit\n   Yes, I trust this folder\n\n Enter to confirm · Esc to cancel";
        let accept = api::DialogAnswer::Accept;
        assert_eq!(dialog_step(&trust, &accept, opened, &Walk::default()), Ok(DialogStep::Down));
        let on_yes = opened.replace(" ❯ No, exit", "   No, exit").replace("   Yes,", " ❯ Yes,");
        assert_eq!(dialog_step(&trust, &accept, &on_yes, &Walk::default()), Ok(DialogStep::Submit));
        assert!(dialog_shape(&trust, opened));
        assert!(!dialog_shape(&trust, "────\n❯ \n────"));
        // `No, exit` ends the agent: the pane's alone.
        assert_eq!(dialog_plan(&trust, &api::DialogAnswer::Reject), None);
        assert_eq!(
            dialog_step(&trust, &api::DialogAnswer::Reject, opened, &Walk::default()),
            Err(Miss::ShapeUnrecognised)
        );
        assert_eq!(
            dialog_step(&trust, &api::DialogAnswer::Choice { index: 0 }, opened, &Walk::default()),
            Err(Miss::ShapeUnrecognised)
        );
        for bad in [
            "────\n❯ \n────",
            &opened.replace(" ❯ No, exit", "   No, exit"),
            "Quick safety check\n ❯ Yes, I trust this folder\n",
        ] {
            assert_eq!(
                dialog_step(&trust, &accept, bad, &Walk::default()),
                Err(Miss::ShapeUnrecognised),
                "{bad}"
            );
        }
        let codex = "  Trust this folder? Codex can read, edit, and run files here.\n› 1. Trust and continue\n  2. Quit";
        assert_eq!(dialog_step(&trust, &accept, codex, &Walk::default()), Ok(DialogStep::Submit));
        let on_quit = codex.replace("› 1.", "  1.").replace("  2. Quit", "› 2. Quit");
        assert_eq!(dialog_step(&trust, &accept, &on_quit, &Walk::default()), Ok(DialogStep::Up));
        // A Codex seat's dialog takes that same walk through the harness
        // split (T-871): Codex's own shapes know no trust dialog.
        assert!(!codex_shapes(SessionKind::Codex, &trust));
        assert_eq!(
            dialog_step_for(
                SessionKind::Codex,
                Standing::Dialog(&trust, &accept),
                codex,
                &Walk::default()
            ),
            Ok(DialogStep::Submit)
        );
        assert_eq!(
            dialog_step_for(
                SessionKind::Codex,
                Standing::Dialog(&trust, &accept),
                &on_quit,
                &Walk::default()
            ),
            Ok(DialogStep::Up)
        );
        assert!(dialog_shape_for(SessionKind::Codex, &trust, codex));
        assert!(!dialog_shape_for(SessionKind::Codex, &trust, "› Ask Codex to do anything"));
        // The agent starting is the dialog's edge; no tool frame is about it.
        let frame = |event: &str, payload: serde_json::Value| HookFrame {
            session: "s".into(),
            event: event.into(),
            reason: None,
            pane: None,
            road: mesimon_core::road::Road::Hooks,
            payload,
        };
        let start = frame("SessionStart", serde_json::json!({"source": "startup"}));
        assert_eq!(dialog_edge(&start, &trust), Some(DialogEdge::Answered));
        let other = frame("PreToolUse", serde_json::json!({"tool_name": "Read"}));
        assert_eq!(dialog_edge(&other, &trust), None);
        assert!(dialog_dropped(&start, &trust));
    }

    #[test]
    fn dialog_keys_require_the_measured_shape_and_selected_label() {
        let dialog = question();
        let screen = "Which color?\n❯ 1. Blue\n  2. Green\n  3. Type something.\nEnter to select · Esc to cancel";
        assert_eq!(
            dialog_step(&dialog, &api::DialogAnswer::Choice { index: 1 }, screen, &Walk::default()),
            Ok(DialogStep::Down)
        );
        let selected = screen.replace("❯ 1.", "  1.").replace("  2.", "❯ 2.");
        assert_eq!(
            dialog_step(
                &dialog,
                &api::DialogAnswer::Choice { index: 1 },
                &selected,
                &Walk::default()
            ),
            Ok(DialogStep::Submit)
        );
        assert_eq!(
            dialog_step(&dialog, &api::DialogAnswer::Choice { index: 8 }, screen, &Walk::default()),
            Err(Miss::ShapeUnrecognised)
        );
        assert_eq!(
            dialog_step(&dialog, &api::DialogAnswer::Accept, screen, &Walk::default()),
            Err(Miss::ShapeUnrecognised)
        );
        for bad in [
            "ordinary composer",
            "Which color?\n❯ 1. Blue\nEnter to select",
            "Which color?\n❯ 1. Blue\n❯ 2. Green\nEnter to select · Esc to cancel",
            "Which color?\n  1. Blue\n  2. Green\nEnter to select · Esc to cancel",
            "Which color?\n❯ 1. Blue\n  3. Green\nEnter to select · Esc to cancel",
        ] {
            assert_eq!(
                dialog_step(
                    &dialog,
                    &api::DialogAnswer::Choice { index: 0 },
                    bad,
                    &Walk::default()
                ),
                Err(Miss::ShapeUnrecognised),
                "{bad}"
            );
        }
        let mut multiple = dialog.clone();
        if let api::DialogContent::Questions { questions } = &mut multiple.content {
            questions[0].multi_select = true;
        }
        assert_eq!(
            dialog_step(
                &multiple,
                &api::DialogAnswer::Choice { index: 0 },
                screen,
                &Walk::default()
            ),
            Err(Miss::ShapeUnrecognised)
        );
        let mut several = dialog.clone();
        if let api::DialogContent::Questions { questions } = &mut several.content {
            questions.push(questions[0].clone());
        }
        assert_eq!(
            dialog_step(
                &several,
                &api::DialogAnswer::Choice { index: 0 },
                screen,
                &Walk::default()
            ),
            Err(Miss::ShapeUnrecognised)
        );
    }

    #[test]
    fn question_text_is_pasted_into_the_selected_row_before_a_separate_enter() {
        let dialog = question();
        let answer = api::DialogAnswer::Text { text: "Purple".into() };
        let screen = "Which color?\n  1. Blue\n  2. Green\n❯ 3. Type something.\nEnter to select · Esc to cancel";
        assert_eq!(
            dialog_step(&dialog, &answer, screen, &Walk::default()),
            Ok(DialogStep::Paste { question: 0, text: "Purple".into() })
        );
        assert_eq!(
            dialog_step(&dialog, &answer, screen, &pasted()),
            Err(Miss::LabelNotFound),
            "never Enter on an empty text row"
        );
        assert_eq!(
            dialog_step(&dialog, &answer, &screen.replace("Type something.", "Purple"), &pasted()),
            Ok(DialogStep::Submit)
        );
        assert_eq!(
            dialog_step(&dialog, &answer, &screen.replace("Type something.", "Purp"), &pasted()),
            Err(Miss::LabelWrapped)
        );
        assert_eq!(
            dialog_step(
                &dialog,
                &api::DialogAnswer::Text { text: "a\nb".into() },
                screen,
                &Walk::default()
            ),
            Err(Miss::ShapeUnrecognised)
        );
        // A long answer wraps in the row it was pasted into, and still reads.
        let long = "a long answer that the pane wraps onto a second row of the dialog";
        let wrapped = screen.replace(
            "Type something.",
            "a long answer that the pane wraps onto a\n     second row of the dialog",
        );
        assert_eq!(
            dialog_step(
                &dialog,
                &api::DialogAnswer::Text { text: long.into() },
                &wrapped,
                &pasted()
            ),
            Ok(DialogStep::Submit)
        );
    }

    #[test]
    fn plan_accept_never_selects_auto_accept_and_reject_requires_the_dialog() {
        let dialog = api::Dialog {
            request: "p".into(),
            content: api::DialogContent::Plan { markdown: "plan".into() },
        };
        let screen = "1. Read the code\n2. Write the code\nWould you like to\n proceed?\n❯ 1. Yes, auto-accept edits\n  2. Yes, manually approve edits\n  3. Tell Claude what to change";
        assert_eq!(
            dialog_step(&dialog, &api::DialogAnswer::Accept, screen, &Walk::default()),
            Ok(DialogStep::Down)
        );
        assert_eq!(
            dialog_step(&dialog, &api::DialogAnswer::Reject, screen, &Walk::default()),
            Ok(DialogStep::Reject)
        );
        assert_eq!(
            dialog_step(&dialog, &api::DialogAnswer::Reject, "❯ composer", &Walk::default()),
            Err(Miss::ShapeUnrecognised)
        );
    }

    /// The friend's dialog (T-567): labels long enough to wrap, each with its
    /// description under it, a rule and a fourth row, then the footer.
    fn wrapped() -> (api::Dialog, &'static str) {
        let option = |label: &str, description: &str| api::QuestionOption {
            label: label.into(),
            description: description.into(),
        };
        let dialog = api::Dialog {
            request: "tool-2".into(),
            content: api::DialogContent::Questions {
                questions: vec![api::Question {
                    question: "Which way should the daemon confirm an answer it typed into a dialog for a phone?".into(),
                    header: "Confirm".into(),
                    multi_select: false,
                    options: vec![
                        option("Watch the hook edge for the record leaving RequiresAction, then reply", "The receipt waits for the agent."),
                        option("Reply the moment tmux accepts the Enter key", "Faster, and says nothing."),
                    ],
                }],
            },
        };
        let screen = "\
 ☐ Confirm
Which way should the daemon confirm an answer it typed into a dialog for a
phone?
  1. Watch the hook edge for the record leaving RequiresAction, then
     reply
     The receipt waits for the agent.
❯ 2. Reply the moment tmux accepts the Enter key
     Faster, and says nothing.
  3. Type something.
────────────────────────────────────────
  4. Chat about this
Enter to select · ↑/↓ to navigate · Esc to cancel";
        (dialog, screen)
    }

    #[test]
    fn a_wrapped_label_reads_whole_with_or_without_its_description() {
        let (dialog, screen) = wrapped();
        let first = api::DialogAnswer::Choice { index: 0 };
        assert_eq!(dialog_step(&dialog, &first, screen, &Walk::default()), Ok(DialogStep::Up));
        let on_first = screen.replace("❯ 2.", "  2.").replace("  1. Watch", "❯ 1. Watch");
        assert_eq!(
            dialog_step(&dialog, &first, &on_first, &Walk::default()),
            Ok(DialogStep::Submit)
        );
        // A word cut where the pane ran out of columns still reads.
        let broken =
            screen.replace("RequiresAction, then\n     reply", "Requires\n     Action, then reply");
        assert_eq!(dialog_step(&dialog, &first, &broken, &Walk::default()), Ok(DialogStep::Up));
        // Without the descriptions drawn, the labels alone read.
        let bare = screen
            .lines()
            .filter(|l| !l.contains("The receipt") && !l.contains("Faster"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(dialog_step(&dialog, &first, &bare, &Walk::default()), Ok(DialogStep::Up));
        // A label cut short is named as such, and a missing one as missing.
        let cut = screen.replace("RequiresAction, then\n     reply", "Requi…");
        assert_eq!(dialog_step(&dialog, &first, &cut, &Walk::default()), Err(Miss::LabelWrapped));
        let gone = screen.replace("Watch the hook edge", "Poll the pane");
        assert_eq!(dialog_step(&dialog, &first, &gone, &Walk::default()), Err(Miss::LabelNotFound));
    }

    #[test]
    fn a_long_dialog_is_read_from_its_own_first_row_to_its_footer() {
        let mut dialog = question();
        let labels = ["Blue", "Green", "Red", "Amber", "Teal", "Violet", "Grey", "Black"];
        if let api::DialogContent::Questions { questions } = &mut dialog.content {
            questions[0].options = labels
                .iter()
                .map(|l| api::QuestionOption {
                    label: (*l).into(),
                    description: format!("{l} paint"),
                })
                .collect();
        }
        // The agent's own numbered list sits above the dialog with the same
        // words, and more than fifty lines of its answer before that.
        let mut screen: Vec<String> = (0..60).map(|i| format!("earlier output {i}")).collect();
        screen.push("1. Blue".into());
        screen.push("❯ 2. Black".into());
        screen.push("Which color?".into());
        for (i, l) in labels.iter().enumerate() {
            let mark = if i == 0 { "❯" } else { " " };
            screen.push(format!("{mark} {}. {l}", i + 1));
            screen.push(format!("     {l} paint"));
        }
        screen.push("  9. Type something.".into());
        screen.push("Enter to select · ↑/↓ to navigate · Esc to cancel".into());
        let screen = screen.join("\n");
        assert_eq!(
            dialog_step(
                &dialog,
                &api::DialogAnswer::Choice { index: 7 },
                &screen,
                &Walk::default()
            ),
            Ok(DialogStep::Down)
        );
        assert_eq!(
            dialog_step(
                &dialog,
                &api::DialogAnswer::Choice { index: 0 },
                &screen,
                &Walk::default()
            ),
            Ok(DialogStep::Submit)
        );
    }

    #[test]
    fn only_the_dialog_s_own_hook_edge_ends_it() {
        let dialog = question();
        let frame = |event: &str, payload: serde_json::Value| HookFrame {
            session: "s".into(),
            event: event.into(),
            reason: None,
            pane: None,
            road: mesimon_core::road::Road::Hooks,
            payload,
        };
        let ask = |id: &str| serde_json::json!({"tool_name": "AskUserQuestion", "tool_use_id": id});
        assert_eq!(
            dialog_edge(&frame("PostToolUse", ask("tool-1")), &dialog),
            Some(DialogEdge::Answered)
        );
        assert_eq!(
            dialog_edge(
                &frame("PostToolUse", serde_json::json!({"tool_name": "AskUserQuestion"})),
                &dialog
            ),
            Some(DialogEdge::Answered)
        );
        assert_eq!(dialog_edge(&frame("PostToolUse", ask("tool-0")), &dialog), None);
        assert_eq!(
            dialog_edge(&frame("PostToolUse", serde_json::json!({"tool_name": "Read"})), &dialog),
            None
        );
        // The next tool says the dialog is gone; the dialog's own repeat does not.
        assert_eq!(
            dialog_edge(
                &frame("PreToolUse", serde_json::json!({"tool_name": "Bash", "tool_use_id": "t"})),
                &dialog
            ),
            Some(DialogEdge::Dismissed)
        );
        assert_eq!(dialog_edge(&frame("PreToolUse", ask("tool-1")), &dialog), None);
        assert_eq!(dialog_edge(&frame("PermissionRequest", ask("tool-1")), &dialog), None);
        assert_eq!(dialog_edge(&frame("PostToolUseFailure", ask("tool-1")), &dialog), None);
        assert_eq!(dialog_edge(&frame("Stop", serde_json::json!({})), &dialog), None);
    }

    /// T-595: a question sent beside a Bash call and a subagent in one
    /// parallel batch stands while they finish; only its own call's end, a
    /// later tool's start, the turn's end or the session's drops it. The
    /// same holds for a plan and for a batch of questions (one call, T-571).
    #[test]
    fn a_sibling_tool_s_end_leaves_the_dialog_standing() {
        let frame = |event: &str, payload: serde_json::Value| HookFrame {
            session: "s".into(),
            event: event.into(),
            reason: None,
            pane: None,
            road: mesimon_core::road::Road::Hooks,
            payload,
        };
        let tool = |name: &str, id: &str| serde_json::json!({"tool_name": name, "tool_use_id": id});
        let mut two = question();
        if let api::DialogContent::Questions { questions } = &mut two.content {
            questions
                .push(api::Question { question: "Which size?".into(), ..questions[0].clone() });
        }
        let plan = api::Dialog {
            request: "tool-1".into(),
            content: api::DialogContent::Plan { markdown: "plan".into() },
        };
        for (dialog, own) in
            [(question(), "AskUserQuestion"), (two, "AskUserQuestion"), (plan, "ExitPlanMode")]
        {
            let ends =
                |f: &HookFrame| dialog_edge(f, &dialog).is_some() || dialog_dropped(f, &dialog);
            // The batch's siblings finish, succeed or fail, while it stands.
            for sibling in ["Bash", "Agent"] {
                for event in ["PostToolUse", "PostToolUseFailure", "PermissionDenied"] {
                    assert!(
                        !ends(&frame(event, tool(sibling, "tool-2"))),
                        "{own}: {sibling} {event}"
                    );
                }
            }
            assert!(
                !ends(&frame("PostToolUse", serde_json::json!({"tool_name": "Bash"}))),
                "{own}"
            );
            assert!(!ends(&frame("PermissionRequest", tool("Bash", "tool-2"))), "{own}");
            assert!(!ends(&frame("SubagentStop", serde_json::json!({}))), "{own}");
            assert!(!ends(&frame("Notification", serde_json::json!({}))), "{own}");
            // Its own edges end it.
            assert_eq!(
                dialog_edge(&frame("PostToolUse", tool(own, "tool-1")), &dialog),
                Some(DialogEdge::Answered),
                "{own}"
            );
            assert!(dialog_dropped(&frame("PostToolUseFailure", tool(own, "tool-1")), &dialog));
            assert!(dialog_dropped(&frame("PermissionDenied", tool(own, "tool-1")), &dialog));
            assert!(!dialog_dropped(&frame("PermissionDenied", tool(own, "tool-0")), &dialog));
            for event in [
                "Stop",
                "StopFailure",
                "SessionEnd",
                "SessionStart",
                "UserPromptSubmit",
                "PaneDied",
            ] {
                assert!(
                    dialog_dropped(&frame(event, serde_json::json!({})), &dialog),
                    "{own}: {event}"
                );
            }
        }
    }

    // ---- T-571: the shapes measured on Claude Code 2.1.287 -----------------
    //
    // Captured from a live Haiku session in a private tmux pane (120x50, and
    // 80x40 for the long questions), `tmux capture-pane -p`, trailing spaces
    // trimmed, on 2026-10-02. Each is a whole screen from the rule above the
    // dialog down; the walks below move the cursor or a tick in them by hand.

    /// The batch's first tab, under the prompt that asked for it: the prompt's own words name every
    /// question, so a question is read only from under the tab bar.
    const B_COLOR: &str = r#"❯ Use the AskUserQuestion tool once, with exactly these three questions in this order, then reply with the single word
  ok. Question 1: header Color, question 'Which color should the button be?', not multi-select, options: Blue
  (description: Calm and familiar), Green (description: Reads as go), Red (description: Loud). Question 2: header
  Toppings, question 'Which toppings do you want?', multiSelect true, options: Cheese (description: Melted), Olives
  (description: Black ones), Basil (description: Fresh), Onion (description: Red onion). Question 3: header Size,
  question 'Which size?', not multi-select, options: Small (description: 8 inch), Large (description: 14 inch).

  Thought for 2s
────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
←  ☐ Color  ☐ Toppings  ☐ Size  ✔ Submit  →

Which color should the button be?

❯ 1. Blue
     Calm and familiar
  2. Green
     Reads as go
  3. Red
     Loud
  4. Type something.
────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
  5. Chat about this

Enter to select · Tab/Arrow keys to navigate · Esc to cancel"#;
    /// Its second tab after `Green` took Enter: a several-choice question, `☒ Color` answered.
    const B_TOPPINGS: &str = r#"────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
←  ☒ Color  ☐ Toppings  ☐ Size  ✔ Submit  →

Which toppings do you want?

❯ 1. [ ] Cheese
         Melted
  2. [ ] Olives
         Black ones
  3. [ ] Basil
         Fresh
  4. [ ] Onion
         Red onion
  5. [ ] Type something
     Next
────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
  6. Chat about this

Enter to select · Tab/Arrow keys to navigate · Esc to cancel"#;
    /// Space on `Cheese`: the row reads `[✔]` and the tab `☒`.
    const B_TICKED: &str = r#"────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
←  ☒ Color  ☒ Toppings  ☐ Size  ✔ Submit  →

Which toppings do you want?

❯ 1. [✔] Cheese
         Melted
  2. [ ] Olives
         Black ones
  3. [ ] Basil
         Fresh
  4. [ ] Onion
         Red onion
  5. [ ] Type something
     Next
────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
  6. Chat about this

Enter to select · Tab/Arrow keys to navigate · Esc to cancel"#;
    /// Words pasted into the text row tick it.
    const B_TYPED: &str = r#"────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
←  ☒ Color  ☒ Toppings  ☐ Size  ✔ Submit  →

Which toppings do you want?

  1. [✔] Cheese
         Melted
  2. [ ] Olives
         Black ones
  3. [ ] Basil
         Fresh
  4. [ ] Onion
         Red onion
❯ 5. [✔] Pineapple slices
     Next
────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
  6. Chat about this

Enter to select · Tab/Arrow keys to navigate · ctrl+g to edit in Vim · Esc to cancel"#;
    /// The cursor on the unnumbered `Next` row.
    const B_ON_NEXT: &str = r#"────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
←  ☒ Color  ☒ Toppings  ☐ Size  ✔ Submit  →

Which toppings do you want?

  1. [✔] Cheese
         Melted
  2. [ ] Olives
         Black ones
  3. [ ] Basil
         Fresh
  4. [ ] Onion
         Red onion
  5. [✔] Pineapple slices
❯    Next
────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
  6. Chat about this

Enter to select · Tab/Arrow keys to navigate · ctrl+g to edit in Vim · Esc to cancel"#;
    /// Enter on `Next`: the third tab.
    const B_SIZE: &str = r#"────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
←  ☒ Color  ☒ Toppings  ☐ Size  ✔ Submit  →

Which size?

❯ 1. Small
     8 inch
  2. Large
     14 inch
  3. Type something.
────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
  4. Chat about this

Enter to select · Tab/Arrow keys to navigate · Esc to cancel"#;
    /// Enter on `Large`: the review, no footer.
    const B_REVIEW: &str = r#"────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
←  ☒ Color  ☒ Toppings  ☒ Size  ✔ Submit  →

Review your answers

 ● Which color should the button be?
   → Green
 ● Which toppings do you want?
   → Cheese, Pineapple slices
 ● Which size?
   → Large

Ready to submit your answers?

❯ 1. Submit answers
  2. Cancel"#;
    /// One several-choice question alone: a tab bar, and `Submit` where a batch's middle question
    /// has `Next`.
    const M_TOPPINGS: &str = r#"────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
←  ☐ Toppings  ✔ Submit  →

Which toppings do you want?

❯ 1. [ ] Cheese
         Melted
  2. [ ] Olives
         Black ones
  3. [ ] Basil
         Fresh
  4. [ ] Onion
         Red onion
  5. [ ] Type something
     Submit
────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
  6. Chat about this

Enter to select · ↑/↓ to navigate · Esc to cancel"#;
    /// `Basil` then `Cheese` ticked, the cursor on `Submit`.
    const M_ON_SUBMIT: &str = r#"────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
←  ☒ Toppings  ✔ Submit  →

Which toppings do you want?

  1. [✔] Cheese
         Melted
  2. [ ] Olives
         Black ones
  3. [✔] Basil
         Fresh
  4. [ ] Onion
         Red onion
  5. [ ] Type something
❯    Submit
────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
  6. Chat about this

Enter to select · ↑/↓ to navigate · ctrl+g to edit in Vim · Esc to cancel"#;
    /// Its review lists the ticks in the order they were made.
    const M_REVIEW: &str = r#"────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
←  ☒ Toppings  ✔ Submit  →

Review your answers

 ● Which toppings do you want?
   → Basil, Cheese

Ready to submit your answers?

❯ 1. Submit answers
  2. Cancel"#;
    /// A one-choice question's words pasted into `Type something.`.
    const C_TYPED: &str = r#"────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
←  ☐ Color  ☐ Toppings  ✔ Submit  →

Which color should the button be?

  1. Blue
     Calm and familiar
  2. Green
     Reads as go
❯ 3. Mauve
────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
  4. Chat about this

Enter to select · Tab/Arrow keys to navigate · ctrl+g to edit in Vim · Esc to cancel"#;
    /// Enter on the words moved on: the last question's button reads `Submit`.
    const C_LAST: &str = r#"────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
←  ☒ Color  ☐ Toppings  ✔ Submit  →

Which toppings do you want?

❯ 1. [ ] Cheese
         Melted
  2. [ ] Olives
         Black ones
  3. [ ] Basil
         Fresh
  4. [ ] Type something
     Submit
────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
  5. Chat about this

Enter to select · Tab/Arrow keys to navigate · Esc to cancel"#;
    /// The review of the words and one tick.
    const C_REVIEW: &str = r#"────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
←  ☒ Color  ☒ Toppings  ✔ Submit  →

Review your answers

 ● Which color should the button be?
   → Mauve
 ● Which toppings do you want?
   → Olives

Ready to submit your answers?

❯ 1. Submit answers
  2. Cancel"#;
    /// Four questions at 80 columns: the tab bar fits, and a question that wraps is drawn with `│`
    /// down its left.
    const N_LONG: &str = r#"────────────────────────────────────────────────────────────────────────────────
←  ☐ Background  ☐ Typography  ☐ Persistence  ☐ Deployments  ✔ Submit  →

│ Which background colour should the settings page use when the system theme is
│ dark?

❯ 1. Charcoal
     A warm near-black
  2. Navy
     A deep blue
  3. Type something.
────────────────────────────────────────────────────────────────────────────────
  4. Chat about this

Enter to select · Tab/Arrow keys to navigate · Esc to cancel"#;
    /// One long question alone at 80 columns wears the same `│`.
    const S_LONG: &str = r#"────────────────────────────────────────────────────────────────────────────────
 ☐ Background

│ Which background colour should the settings page use when the system theme is
│ dark?

❯ 1. Charcoal
     A warm near-black
  2. Navy
     A deep blue
  3. Type something.
────────────────────────────────────────────────────────────────────────────────
  4. Chat about this

Enter to select · ↑/↓ to navigate · Esc to cancel"#;

    fn ask(question: &str, header: &str, multi: bool, options: &[(&str, &str)]) -> api::Question {
        api::Question {
            question: question.into(),
            header: header.into(),
            multi_select: multi,
            options: options
                .iter()
                .map(|(label, description)| api::QuestionOption {
                    label: (*label).into(),
                    description: (*description).into(),
                })
                .collect(),
        }
    }
    fn asked(questions: Vec<api::Question>) -> api::Dialog {
        api::Dialog {
            request: "toolu_b".into(),
            content: api::DialogContent::Questions { questions },
        }
    }
    /// The three questions the `B_` screens draw.
    fn measured_batch() -> api::Dialog {
        asked(vec![
            ask(
                "Which color should the button be?",
                "Color",
                false,
                &[("Blue", "Calm and familiar"), ("Green", "Reads as go"), ("Red", "Loud")],
            ),
            ask(
                "Which toppings do you want?",
                "Toppings",
                true,
                &[
                    ("Cheese", "Melted"),
                    ("Olives", "Black ones"),
                    ("Basil", "Fresh"),
                    ("Onion", "Red onion"),
                ],
            ),
            ask("Which size?", "Size", false, &[("Small", "8 inch"), ("Large", "14 inch")]),
        ])
    }
    fn answers(answers: Vec<api::QuestionAnswer>) -> api::DialogAnswer {
        api::DialogAnswer::Answers { answers }
    }

    #[test]
    fn a_batch_walks_each_tab_and_presses_submit_once() {
        use api::QuestionAnswer as Q;
        let dialog = measured_batch();
        let plan = answers(vec![
            Q::Choice { index: 1 },
            Q::Choices { indices: vec![0, 2] },
            Q::Choice { index: 1 },
        ]);
        let w = Walk::default();
        // The first tab: Green is a row down, and its Enter moves on.
        assert_eq!(dialog_step(&dialog, &plan, B_COLOR, &w), Ok(DialogStep::Down));
        let on_green =
            B_COLOR.replace("❯ 1. Blue", "  1. Blue").replace("  2. Green", "❯ 2. Green");
        assert_eq!(dialog_step(&dialog, &plan, &on_green, &w), Ok(DialogStep::Next));
        // The second: Cheese is ticked where the cursor stands, then Basil.
        assert_eq!(
            dialog_step(&dialog, &plan, B_TOPPINGS, &w),
            Ok(DialogStep::Toggle { question: 1, row: 0 })
        );
        let ticked = Walk { toggled: Some((1, 0)), ..Walk::default() };
        assert_eq!(dialog_step(&dialog, &plan, B_TICKED, &ticked), Ok(DialogStep::Down));
        // A tick the screen did not take is never sent twice.
        assert_eq!(dialog_step(&dialog, &plan, B_TOPPINGS, &ticked), Err(Miss::TickNotTaken));
        let on_basil = B_TICKED
            .replace("❯ 1. [✔] Cheese", "  1. [✔] Cheese")
            .replace("  3. [ ] Basil", "❯ 3. [ ] Basil");
        assert_eq!(
            dialog_step(&dialog, &plan, &on_basil, &w),
            Ok(DialogStep::Toggle { question: 1, row: 2 })
        );
        let both = on_basil.replace("❯ 3. [ ] Basil", "❯ 3. [✔] Basil");
        assert_eq!(dialog_step(&dialog, &plan, &both, &w), Ok(DialogStep::Down), "on to Next");
        let on_next =
            both.replace("❯ 3. [✔] Basil", "  3. [✔] Basil").replace("     Next", "❯    Next");
        assert_eq!(dialog_step(&dialog, &plan, &on_next, &w), Ok(DialogStep::Next));
        // The third: Large, a row down.
        assert_eq!(dialog_step(&dialog, &plan, B_SIZE, &w), Ok(DialogStep::Down));
        let on_large =
            B_SIZE.replace("❯ 1. Small", "  1. Small").replace("  2. Large", "❯ 2. Large");
        assert_eq!(dialog_step(&dialog, &plan, &on_large, &w), Ok(DialogStep::Next));
        // The review is pressed only when it lists the answer sent: the
        // measured one holds words this answer does not, and lists its ticks
        // in the order they were made.
        assert_eq!(dialog_step(&dialog, &plan, B_REVIEW, &w), Err(Miss::AnswerDiffers));
        let reviewed = B_REVIEW.replace("Cheese, Pineapple slices", "Basil, Cheese");
        assert_eq!(dialog_step(&dialog, &plan, &reviewed, &w), Ok(DialogStep::Submit));
        let on_cancel = reviewed
            .replace("❯ 1. Submit answers", "  1. Submit answers")
            .replace("  2. Cancel", "❯ 2. Cancel");
        assert_eq!(dialog_step(&dialog, &plan, &on_cancel, &w), Ok(DialogStep::Up));
        // The question alone: its button reads Submit, and its review too.
        let one = asked(vec![questions_of(&measured_batch())[1].clone()]);
        let plan = answers(vec![Q::Choices { indices: vec![0, 2] }]);
        assert_eq!(
            dialog_step(&one, &plan, M_TOPPINGS, &w),
            Ok(DialogStep::Toggle { question: 0, row: 0 })
        );
        assert_eq!(dialog_step(&one, &plan, M_ON_SUBMIT, &w), Ok(DialogStep::Next));
        assert_eq!(dialog_step(&one, &plan, M_REVIEW, &w), Ok(DialogStep::Submit));
        let fewer = answers(vec![Q::Choices { indices: vec![2] }]);
        assert_eq!(dialog_step(&one, &fewer, M_REVIEW, &w), Err(Miss::AnswerDiffers));
        assert_eq!(
            dialog_step(&one, &fewer, M_ON_SUBMIT, &w),
            Ok(DialogStep::Up),
            "unticks Cheese"
        );
    }

    #[test]
    fn words_go_into_a_question_s_text_row_and_a_person_s_ticked_words_stay() {
        use api::QuestionAnswer as Q;
        let w = Walk::default();
        // One-choice words in a batch: pasted on the row, then Enter moves on.
        let dialog = asked(vec![
            ask(
                "Which color should the button be?",
                "Color",
                false,
                &[("Blue", "Calm and familiar"), ("Green", "Reads as go")],
            ),
            ask(
                "Which toppings do you want?",
                "Toppings",
                true,
                &[("Cheese", "Melted"), ("Olives", "Black ones"), ("Basil", "Fresh")],
            ),
        ]);
        let plan = answers(vec![Q::Text { text: "Mauve".into() }, Q::Choices { indices: vec![1] }]);
        let at_text = C_TYPED.replace("❯ 3. Mauve", "❯ 3. Type something.");
        assert_eq!(
            dialog_step(&dialog, &plan, &at_text, &w),
            Ok(DialogStep::Paste { question: 0, text: "Mauve".into() })
        );
        let typed = Walk { pasted: vec![0], ..Walk::default() };
        assert_eq!(dialog_step(&dialog, &plan, C_TYPED, &typed), Ok(DialogStep::Next));
        let cut = C_TYPED.replace("3. Mauve", "3. Mauv");
        assert_eq!(dialog_step(&dialog, &plan, &cut, &typed), Err(Miss::LabelWrapped));
        assert_eq!(
            dialog_step(&dialog, &plan, C_TYPED, &w),
            Err(Miss::LabelNotFound),
            "words this walk did not paste are not its to send"
        );
        // The last question: Olives, then the row that reads Submit.
        assert_eq!(dialog_step(&dialog, &plan, C_LAST, &typed), Ok(DialogStep::Down));
        let on_olives = C_LAST
            .replace("❯ 1. [ ] Cheese", "  1. [ ] Cheese")
            .replace("  2. [ ] Olives", "❯ 2. [ ] Olives");
        assert_eq!(
            dialog_step(&dialog, &plan, &on_olives, &typed),
            Ok(DialogStep::Toggle { question: 1, row: 1 })
        );
        let olives = on_olives.replace("[ ] Olives", "[✔] Olives");
        assert_eq!(dialog_step(&dialog, &plan, &olives, &typed), Ok(DialogStep::Down));
        let on_submit = olives
            .replace("❯ 2. [✔] Olives", "  2. [✔] Olives")
            .replace("     Submit", "❯    Submit");
        assert_eq!(dialog_step(&dialog, &plan, &on_submit, &typed), Ok(DialogStep::Next));
        assert_eq!(dialog_step(&dialog, &plan, C_REVIEW, &typed), Ok(DialogStep::Submit));

        // Several-choice words: the options unticked, the words pasted on
        // the text row, which ticks itself.
        let dialog = measured_batch();
        let plan = answers(vec![
            Q::Choice { index: 1 },
            Q::Text { text: "Pineapple slices".into() },
            Q::Choice { index: 1 },
        ]);
        assert_eq!(dialog_step(&dialog, &plan, B_TOPPINGS, &w), Ok(DialogStep::Down));
        let at_text = B_TOPPINGS
            .replace("❯ 1. [ ] Cheese", "  1. [ ] Cheese")
            .replace("  5. [ ] Type something", "❯ 5. [ ] Type something");
        assert_eq!(
            dialog_step(&dialog, &plan, &at_text, &w),
            Ok(DialogStep::Paste { question: 1, text: "Pineapple slices".into() })
        );
        let pasted = Walk { pasted: vec![1], ..Walk::default() };
        assert_eq!(dialog_step(&dialog, &plan, B_TYPED, &pasted), Ok(DialogStep::Up), "Cheese off");
        let alone = B_TYPED.replace("[✔] Cheese", "[ ] Cheese");
        assert_eq!(dialog_step(&dialog, &plan, &alone, &pasted), Ok(DialogStep::Down));
        let short = alone.replace("Pineapple slices", "Pineapple");
        assert_eq!(dialog_step(&dialog, &plan, &short, &pasted), Err(Miss::LabelWrapped));
        // Words a person typed and ticked are never cleared for choices.
        let plan = answers(vec![
            Q::Choice { index: 1 },
            Q::Choices { indices: vec![0] },
            Q::Choice { index: 1 },
        ]);
        assert_eq!(dialog_step(&dialog, &plan, B_ON_NEXT, &w), Err(Miss::ShapeUnrecognised));
    }

    #[test]
    fn a_question_that_wraps_is_read_past_its_bar() {
        use api::QuestionAnswer as Q;
        let w = Walk::default();
        let background = ask(
            "Which background colour should the settings page use when the system theme is dark?",
            "Background",
            false,
            &[("Charcoal", "A warm near-black"), ("Navy", "A deep blue")],
        );
        let four = asked(vec![
            background.clone(),
            ask("Which font?", "Typography", false, &[("Inter", "Sans"), ("Lora", "Serif")]),
            ask(
                "Where should drafts be kept?",
                "Persistence",
                true,
                &[("Disk", "A file"), ("Memory", "Lost on exit")],
            ),
            ask(
                "Which regions?",
                "Deployments",
                true,
                &[("Europe", "Belgium"), ("America", "Iowa")],
            ),
        ]);
        let plan = answers(vec![
            Q::Choice { index: 1 },
            Q::Choice { index: 0 },
            Q::Choices { indices: vec![0] },
            Q::Choices { indices: vec![1] },
        ]);
        assert_eq!(dialog_step(&four, &plan, N_LONG, &w), Ok(DialogStep::Down));
        assert!(dialog_shape(&four, N_LONG));
        // One question alone wears the same bar, on the T-567 road.
        let one = asked(vec![background]);
        let navy = api::DialogAnswer::Choice { index: 1 };
        assert_eq!(dialog_step(&one, &navy, S_LONG, &w), Ok(DialogStep::Down));
        let charcoal = api::DialogAnswer::Choice { index: 0 };
        assert_eq!(dialog_step(&one, &charcoal, S_LONG, &w), Ok(DialogStep::Submit));
    }

    #[test]
    fn a_screen_that_drifts_from_the_batch_stops_the_walk() {
        use api::QuestionAnswer as Q;
        let dialog = measured_batch();
        let plan = answers(vec![
            Q::Choice { index: 1 },
            Q::Choices { indices: vec![0] },
            Q::Choice { index: 1 },
        ]);
        let w = Walk::default();
        let step = |screen: &str| dialog_step(&dialog, &plan, screen, &w);
        // Another dialog's words under the bar, or the prompt's words only.
        let mut other = measured_batch();
        if let api::DialogContent::Questions { questions } = &mut other.content {
            questions[0].question = "Which colour should the button be?".into();
        }
        assert_eq!(dialog_step(&other, &plan, B_COLOR, &w), Err(Miss::ShapeUnrecognised));
        let no_bar = B_COLOR.replace("←  ☐ Color  ☐ Toppings  ☐ Size  ✔ Submit  →", "");
        assert_eq!(step(&no_bar), Err(Miss::ShapeUnrecognised));
        let no_footer =
            B_COLOR.replace("Enter to select · Tab/Arrow keys to navigate · Esc to cancel", "");
        assert_eq!(step(&no_footer), Err(Miss::ShapeUnrecognised));
        let two = B_COLOR.replace("  2. Green", "❯ 2. Green");
        assert_eq!(step(&two), Err(Miss::ShapeUnrecognised));
        let none = B_COLOR.replace("❯ 1. Blue", "  1. Blue");
        assert_eq!(step(&none), Err(Miss::ShapeUnrecognised));
        let skips = B_COLOR.replace("  3. Red", "  4. Red");
        assert_eq!(step(&skips), Err(Miss::ShapeUnrecognised));
        // A mark Claude Code was not measured drawing is no tick.
        let marked = B_TOPPINGS.replace("[ ] Olives", "[x] Olives");
        assert_eq!(step(&marked), Err(Miss::LabelNotFound));
        // A review short of a question, and the composer once it is gone.
        let short = B_REVIEW.replace(" ● Which size?\n", "").replace("   → Large\n", "");
        assert_eq!(step(&short), Err(Miss::ShapeUnrecognised));
        assert_eq!(step("──────\n❯ \n──────\n  ? for shortcuts"), Err(Miss::ShapeUnrecognised));
        // A refusal is Escape on any tab of the dialog, never on the composer.
        let reject = api::DialogAnswer::Reject;
        for screen in [B_COLOR, B_TOPPINGS, B_REVIEW] {
            assert_eq!(dialog_step(&dialog, &reject, screen, &w), Ok(DialogStep::Reject));
        }
        assert_eq!(dialog_step(&dialog, &reject, "❯ composer", &w), Err(Miss::ShapeUnrecognised));
        assert!(dialog_shape(&dialog, B_COLOR) && dialog_shape(&dialog, B_REVIEW));
        assert!(!dialog_shape(&dialog, "❯ composer"));
    }

    #[test]
    fn the_measured_shapes_are_answerable_and_an_answer_fits_each_question() {
        use api::QuestionAnswer as Q;
        let dialog = measured_batch();
        let questions = questions_of(&dialog).to_vec();
        assert!(dialog_answerable(&questions));
        assert!(dialog_answerable(&questions[1..2]), "one several-choice question");
        let twins = vec![questions[0].clone(), questions[0].clone()];
        assert!(!dialog_answerable(&twins), "the screen cannot tell twins apart");
        let five = [&questions[..], &questions[..2]].concat();
        assert!(!dialog_answerable(&five));
        assert!(!dialog_answerable(&[]));
        let fits = |a: Vec<Q>| dialog_plan(&dialog, &answers(a));
        let good = vec![
            Q::Choice { index: 2 },
            Q::Choices { indices: vec![3, 0] },
            Q::Choice { index: 0 },
        ];
        assert_eq!(fits(good.clone()), Some(Plan::Batch(good)));
        for bad in [
            vec![Q::Choice { index: 0 }, Q::Choices { indices: vec![0] }],
            vec![Q::Choice { index: 3 }, Q::Choices { indices: vec![0] }, Q::Choice { index: 0 }],
            vec![Q::Choice { index: 0 }, Q::Choice { index: 0 }, Q::Choice { index: 0 }],
            vec![
                Q::Choices { indices: vec![0] },
                Q::Choices { indices: vec![0] },
                Q::Choice { index: 0 },
            ],
            vec![Q::Choice { index: 0 }, Q::Choices { indices: vec![] }, Q::Choice { index: 0 }],
            vec![
                Q::Choice { index: 0 },
                Q::Choices { indices: vec![1, 1] },
                Q::Choice { index: 0 },
            ],
            vec![Q::Choice { index: 0 }, Q::Choices { indices: vec![4] }, Q::Choice { index: 0 }],
            vec![
                Q::Text { text: "a\nb".into() },
                Q::Choices { indices: vec![0] },
                Q::Choice { index: 0 },
            ],
        ] {
            assert_eq!(fits(bad.clone()), None, "{bad:?}");
        }
        // An older browser's single answer fits only the one-choice question.
        assert_eq!(dialog_plan(&dialog, &api::DialogAnswer::Choice { index: 0 }), None);
        let single = question();
        assert_eq!(
            dialog_plan(&single, &answers(vec![Q::Choice { index: 1 }])),
            Some(Plan::Single(Q::Choice { index: 1 }))
        );
        // The walk's room grows with the batch and stays inside its ceiling.
        assert_eq!(walk_budget(&single), (12, Duration::from_secs(8)));
        let (keys, walk) = walk_budget(&dialog);
        assert_eq!(keys, 55);
        assert!(walk > Duration::from_secs(20) && walk <= DIALOG_WALK_MAX, "{walk:?}");
        let widest = asked(
            (0..4)
                .map(|i| {
                    let options: Vec<(String, String)> =
                        (0..8).map(|o| (format!("o{o}"), String::new())).collect();
                    let options: Vec<(&str, &str)> =
                        options.iter().map(|(l, d)| (l.as_str(), d.as_str())).collect();
                    ask(&format!("q{i}?"), "h", true, &options)
                })
                .collect(),
        );
        let (keys, walk) = walk_budget(&widest);
        assert!(keys >= 4 * 8 * 3, "{keys}");
        assert!(walk > Duration::from_secs(50) && walk <= DIALOG_WALK_MAX, "{walk:?}");
    }

    #[test]
    fn the_crown_s_answer_is_refused_question_by_question_in_words() {
        use api::QuestionAnswer as Q;
        let questions = questions_of(&measured_batch()).to_vec();
        let refused = |answer: CrownAnswer| crown_answers("T-2", &questions, answer).unwrap_err();
        assert!(refused(CrownAnswer::Index(0)).contains("asks 3 questions at once"));
        assert!(refused(CrownAnswer::Answers(vec![Q::Choice { index: 0 }; 2]))
            .contains("answers carries 2 answers and T-2's dialog asks 3 questions"));
        let mut three =
            vec![Q::Choice { index: 1 }, Q::Choices { indices: vec![0] }, Q::Choice { index: 1 }];
        let mut with = |at: usize, answer: Q| {
            three[at] = answer;
            let why = refused(CrownAnswer::Answers(three.clone()));
            three = vec![
                Q::Choice { index: 1 },
                Q::Choices { indices: vec![0] },
                Q::Choice { index: 1 },
            ];
            why
        };
        assert!(with(1, Q::Choice { index: 0 }).contains("question 2 takes several choices"));
        assert!(with(0, Q::Choices { indices: vec![0] }).contains("question 1 takes one choice"));
        assert!(with(0, Q::Choice { index: 5 })
            .contains("index 5 is out of range: question 1 has 3 options, 0 to 2"));
        assert!(with(1, Q::Choices { indices: vec![] }).contains("indices is empty"));
        assert!(with(1, Q::Choices { indices: vec![0, 0] }).contains("twice"));
        assert!(with(1, Q::Choices { indices: vec![9] }).contains("index 9 is out of range"));
        assert!(with(2, Q::Text { text: "a\nb".into() }).contains("one line"));
        let good = vec![
            Q::Choice { index: 1 },
            Q::Choices { indices: vec![0, 2] },
            Q::Choice { index: 1 },
        ];
        assert_eq!(
            crown_answers("T-2", &questions, CrownAnswer::Answers(good.clone())),
            Ok(good.clone())
        );
        assert_eq!(answer_words(&questions, &good), "Green; Cheese, Basil; Large");
        // The one question keeps T-569's words.
        let one = &questions[..1];
        let why = crown_answers("T-2", one, CrownAnswer::Index(3)).unwrap_err();
        assert!(why.contains("out of range: the question has 3 options"), "{why}");
        let several = &questions[1..2];
        let why = crown_answers("T-2", several, CrownAnswer::Index(0)).unwrap_err();
        assert!(why.contains("the question takes several choices"), "{why}");
    }

    /// A phone sees a ticket's tags in the TUI's tints, and a pickup only on
    /// a ticket a phone filed, in milliseconds.
    #[test]
    fn a_projected_ticket_carries_its_tags_and_a_phone_s_pickup() {
        let mut board = Board::with_default_columns();
        board.tags.push(mesimon_core::board::Tag { name: "BUG".into(), group: 1, color: Some(4) });
        let mut t = Ticket {
            id: ulid::Ulid(1),
            short_key: "T-1".into(),
            title: "from the phone".into(),
            column: "TODO".into(),
            order: "a0".into(),
            created_at: "@0".into(),
            created_by: String::new(),
            created_from: None,
            entered_at: None,
            woke_at: None,
            manual_merge: false,
            execution_policy: Default::default(),
            tier: None,
            envelope: None,
            workspace: None,
            import_origin: None,
            raised: None,
            previous_column: None,
            picked: None,
            tags: vec![mesimon_core::board::TagRef { name: "BUG".into(), group: 1 }],
            notes: Vec::new(),
            archived: None,
        };
        assert_eq!(
            projected_tags(&board, &t),
            vec![api::TagOption { group: 1, name: "BUG".into(), tint: 4 }]
        );
        t.picked = Some(mesimon_core::board::PickedUp {
            at: "@1790000000".into(),
            by: mesimon_core::board::PICKED_AT_DESK.into(),
        });
        t.created_by = "local".into();
        assert_eq!(projected_pickup(&t), None, "a person's own ticket is nobody's news");
        t.created_by = "device:ab12".into();
        assert_eq!(
            projected_pickup(&t),
            Some(api::Picked { by: "desk".into(), at: 1_790_000_000_000 })
        );
    }

    /// A phone hears what the crown last did to a ticket for the hour it is
    /// kept (T-623), with the doer by its key, and nothing past it.
    #[test]
    fn a_phone_reads_the_crown_s_last_touch_for_an_hour() {
        let mut board = Board::with_default_columns();
        let (crown, worker) = (ulid::Ulid(1), ulid::Ulid(2));
        board.tickets.push(Ticket {
            id: crown,
            short_key: "T-1".into(),
            title: "the crown".into(),
            column: "TODO".into(),
            order: "a0".into(),
            created_at: "@0".into(),
            created_by: String::new(),
            created_from: None,
            entered_at: None,
            woke_at: None,
            manual_merge: false,
            execution_policy: Default::default(),
            tier: None,
            envelope: None,
            workspace: None,
            import_origin: None,
            raised: None,
            previous_column: None,
            picked: None,
            tags: Vec::new(),
            notes: Vec::new(),
            archived: None,
        });
        let touch =
            CrownTouch { ticket: worker, action: "moved".into(), at_ms: 1_000, from: Some(crown) };
        let key = Some("T-1".to_string());
        assert_eq!(
            projected_crown_touch(&board, Some(&touch), 1_000 + CROWN_TOUCH_KEPT_MS - 1),
            Some(api::Crowned { action: "moved".into(), by: key, at: 1_000 })
        );
        assert_eq!(projected_crown_touch(&board, Some(&touch), 1_000 + CROWN_TOUCH_KEPT_MS), None);
        assert_eq!(projected_crown_touch(&board, None, 1_000), None);
        let gone = CrownTouch { from: Some(ulid::Ulid(9)), ..touch };
        assert_eq!(
            projected_crown_touch(&board, Some(&gone), 1_000).and_then(|c| c.by),
            None,
            "a doer no longer on the board is named by nobody"
        );
    }

    /// Sent reads a phone's own tickets once archived (T-665): never a
    /// person's, never one still on the board, the newest archive first and
    /// no more than Sent keeps.
    #[test]
    fn sent_reads_a_phone_s_archived_tickets_newest_first() {
        let mut board = Board::with_default_columns();
        let ticket = |n: u128, by: &str, archived_at: Option<u64>| Ticket {
            id: ulid::Ulid(n),
            short_key: format!("T-{n}"),
            title: format!("ticket {n}"),
            column: "TODO".into(),
            order: "a0".into(),
            created_at: "@0".into(),
            created_by: by.into(),
            created_from: None,
            entered_at: None,
            woke_at: None,
            manual_merge: false,
            execution_policy: Default::default(),
            tier: None,
            envelope: None,
            workspace: None,
            import_origin: None,
            raised: None,
            previous_column: None,
            picked: None,
            tags: Vec::new(),
            notes: Vec::new(),
            archived: archived_at.map(|at| mesimon_core::board::Archived {
                at: format!("@{at}"),
                by: "local".into(),
                until: None,
                needs_you: false,
            }),
        };
        board.tickets.push(ticket(1, "device:ab12", None));
        board.tickets.push(ticket(2, "local", Some(1_790_000_002)));
        board.tickets.push(ticket(3, "device:ab12", Some(1_790_000_001)));
        board.tickets.push(ticket(4, "device:cd34", Some(1_790_000_003)));
        let keys =
            |board: &Board| sent_archived(board).map(|t| t.short_key.clone()).collect::<Vec<_>>();
        assert_eq!(keys(&board), ["T-4", "T-3"]);
        for n in 10..(10 + ARCHIVED_SENT as u128) {
            board.tickets.push(ticket(n, "device:ab12", Some(1_790_000_000)));
        }
        let kept = keys(&board);
        assert_eq!(kept.len(), ARCHIVED_SENT);
        assert_eq!(&kept[..2], ["T-4", "T-3"], "the oldest archives are dropped first");
    }

    fn agent(ticket: ulid::Ulid, kind: SessionKind, state: SessionState) -> SessionRecord {
        SessionRecord::new(uuid::Uuid::new_v4(), kind, ticket, Vec::new(), "/".into(), state)
    }

    /// A phone's card sees the ticket's open shell (T-948), as the TUI's
    /// does: a `!` terminal and an adopted `Bash` record each, busy only
    /// while a command runs; never another ticket's, the checkout's own
    /// terminal, a dead shell or an agent.
    #[test]
    fn a_phone_sees_a_ticket_s_open_shell() {
        let (a, b) = (ulid::Ulid(1), ulid::Ulid(2));
        let mut board = Board::with_default_columns();
        let mut terminals = BTreeMap::new();
        let mut foregrounds = HashMap::new();
        let shell = |board: &Board, terminals: &_, foregrounds: &_, t| {
            projected_shell(board, terminals, foregrounds, t)
        };
        let open = Some(api::Shell { busy: false });
        let busy = Some(api::Shell { busy: true });
        // An agent, idle or working, is `agent`'s, never a shell.
        board.sessions.push(agent(a, SessionKind::Claude, SessionState::Running));
        terminals.insert(None, Some("cargo".to_string()));
        assert_eq!(shell(&board, &terminals, &foregrounds, a), None, "the checkout's own terminal");
        // The `!` terminal, at its prompt and then running a command.
        terminals.insert(Some(a), None);
        assert_eq!(shell(&board, &terminals, &foregrounds, a), open);
        assert_eq!(shell(&board, &terminals, &foregrounds, b), None, "another ticket's");
        terminals.insert(Some(a), Some("cargo".to_string()));
        assert_eq!(shell(&board, &terminals, &foregrounds, a), busy);
        // An adopted shell: there while its pane lives, busy on a foreground.
        let rec = agent(b, SessionKind::Bash, SessionState::Running);
        let id = rec.id;
        board.sessions.push(rec);
        assert_eq!(shell(&board, &terminals, &foregrounds, b), open);
        foregrounds.insert(id, "vim".to_string());
        assert_eq!(shell(&board, &terminals, &foregrounds, b), busy);
        board.sessions[1].state =
            SessionState::Exited { reason: mesimon_core::board::ExitReason::UserQuit };
        assert_eq!(shell(&board, &terminals, &foregrounds, b), None, "a dead shell");
    }

    /// A phone is offered the desk's `^n` ring (T-643): both built-ins while
    /// nobody made a tier, then the tiers a person made with the default in
    /// front when it is a built-in, each with its provider's word.
    #[test]
    fn a_phone_is_offered_the_desk_s_tier_ring() {
        use mesimon_core::board::AgentProvider;
        use mesimon_core::tier::{Book, Effort, MachineTiers, Tier};
        let mut board = Board::with_default_columns();
        let machine = MachineTiers::default();
        let words = |board: &Board| -> Vec<(String, String)> {
            projected_tiers(&Book::new(&machine, board))
                .into_iter()
                .map(|t| (t.name, t.provider))
                .collect()
        };
        assert_eq!(
            words(&board),
            [("claude".into(), "claude".into()), ("codex".into(), "codex".into())]
        );
        board.tiers.push(Tier {
            id: "01K".into(),
            name: "coder".into(),
            provider: AgentProvider::Codex,
            model: "gpt-5".into(),
            effort: Effort::High,
            description: String::new(),
        });
        let tiers = projected_tiers(&Book::new(&machine, &board));
        assert_eq!(
            tiers.iter().map(|t| (t.id.as_str(), t.provider.as_str())).collect::<Vec<_>>(),
            [("claude", "claude"), ("01K", "codex")]
        );
        assert_eq!(tiers[1].summary, "Codex ∙ gpt-5 ∙ high");
    }

    /// One agent per ticket (T-498): a phone's start finds the seat taken
    /// by a live or a stopping agent, and free once it exited. A parked one
    /// is the wake road (T-510), and a shell holds no agent seat.
    #[test]
    fn a_phone_start_needs_the_ticket_s_agent_seat_empty_or_asleep() {
        use mesimon_core::board::ExitReason;
        let mut board = Board::with_default_columns();
        let ticket = ulid::Ulid(7);
        assert_eq!(seat_refusal(&board, ticket), None);
        board.sessions.push(agent(ticket, SessionKind::Claude, SessionState::Running));
        assert_eq!(seat_refusal(&board, ticket), Some("this ticket already has an agent"));
        assert_eq!(seat_refusal(&board, ulid::Ulid(8)), None, "another ticket's seat");
        board.sessions[0].state = SessionState::Sleeping;
        assert_eq!(seat_refusal(&board, ticket), None, "a parked agent wakes");
        board.sessions[0].state = SessionState::Exited { reason: ExitReason::UserQuit };
        assert_eq!(seat_refusal(&board, ticket), None);
        board.sessions[0].kind = SessionKind::Codex;
        board.sessions[0].codex_stopping = true;
        assert!(seat_refusal(&board, ticket).is_some_and(|m| m.contains("stopping")));
        board.sessions[0] = agent(ticket, SessionKind::Bash, SessionState::Running);
        assert_eq!(seat_refusal(&board, ticket), None);
    }

    /// A mailed prompt (T-881) queues on the pane agent the ticket has when
    /// the letter opens, and is refused in words for a parked agent or none.
    #[test]
    fn a_mailed_prompt_queues_only_on_an_awake_agent() {
        use mesimon_core::board::StopReason;
        let mut board = Board::with_default_columns();
        let ticket = ulid::Ulid(7);
        let seat =
            |board: &Board, ticket, session: &str| mail_prompt_seat(board, ticket, Some(session));
        let none = uuid::Uuid::nil().to_string();
        assert_eq!(seat(&board, ticket, &none), Err("no agent on this ticket to take them"));
        board.sessions.push(agent(ticket, SessionKind::Claude, SessionState::Sleeping));
        let own = board.sessions[0].id.to_string();
        assert!(seat(&board, ticket, &own).is_err_and(|m| m.contains("asleep")));
        for state in
            [SessionState::Running, SessionState::Idle { stop_reason: StopReason::EndTurn }]
        {
            board.sessions[0].state = state;
            assert_eq!(seat(&board, ticket, &own), Ok(board.sessions[0].id));
        }
        // Written to another conversation, or to none: never this one's.
        assert!(seat(&board, ticket, &none).is_err_and(|m| m.contains("another agent")));
        assert!(mail_prompt_seat(&board, ticket, None).is_err());
        assert!(seat(&board, ticket, "not a uuid").is_err());
        board.sessions[0].kind = SessionKind::Codex;
        assert_eq!(seat(&board, ticket, &own), Ok(board.sessions[0].id));
        assert!(seat(&board, ulid::Ulid(8), &own).is_err(), "another ticket's agent");
        board.sessions[0] = agent(ticket, SessionKind::Bash, SessionState::Running);
        let shell = board.sessions[0].id.to_string();
        assert!(seat(&board, ticket, &shell).is_err(), "a shell is no agent");
    }

    /// A phone's receipt (T-498) is a clock while the worktree is cut and
    /// while the session comes up with its first prompt owed, two ticks
    /// once it took that prompt, and a refusal when it never got there.
    #[test]
    fn a_phone_start_reads_started_once_the_session_took_its_first_prompt() {
        use mesimon_core::board::{ExitReason, Reason, StopReason};
        let mut board = Board::with_default_columns();
        assert_eq!(start_progress(&board, None, true, None), Progress::Provisioning);
        assert_eq!(
            start_progress(&board, None, false, Some("disk full".into())),
            Progress::Failed("the worktree could not be made: disk full".into())
        );
        assert_eq!(
            start_progress(&board, None, false, None),
            Progress::Failed("the agent did not start".into())
        );
        let mut rec = agent(ulid::Ulid(7), SessionKind::Claude, SessionState::Spawning);
        rec.pending_submit = true;
        let id = rec.id;
        assert!(matches!(start_progress(&board, Some(id), false, None), Progress::Failed(_)));
        board.sessions.push(rec);
        assert_eq!(start_progress(&board, Some(id), false, None), Progress::Starting);
        board.sessions[0].state = SessionState::Idle { stop_reason: StopReason::Unknown };
        assert_eq!(start_progress(&board, Some(id), false, None), Progress::Starting, "owed");
        board.sessions[0].pending_submit = false;
        for state in [
            SessionState::Running,
            SessionState::Idle { stop_reason: StopReason::EndTurn },
            SessionState::RequiresAction { reason: Reason::Permission },
        ] {
            board.sessions[0].state = state;
            assert_eq!(start_progress(&board, Some(id), false, None), Progress::Started);
        }
        board.sessions[0].pending_submit = true;
        board.sessions[0].state = SessionState::Exited { reason: ExitReason::UserQuit };
        assert_eq!(
            start_progress(&board, Some(id), false, None),
            Progress::Failed("the agent exited before it started".into())
        );
    }

    /// A start still on its way keeps its receipt past the cap of 128 that
    /// every snapshot and preview counts toward, and loses it once settled.
    #[test]
    fn a_start_on_its_way_keeps_its_receipt_past_the_cap() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut control = Control::new(tx);
        let grant = BoardId::random();
        let device = mesimon_team::crypto::DeviceKeys::generate().id();
        control
            .starts
            .insert(ulid::Ulid(1), StartWait { grant, device, command: 5, session: None });
        control.remember(grant, 5, Reply::delivery("starting"));
        for id in 6..400 {
            control.remember(grant, id, Reply::Changed);
        }
        assert_eq!(control.receipts[&grant].len(), 128);
        assert!(control.receipts[&grant].contains_key(&5));
        control.starts.clear();
        control.remember(grant, 400, Reply::Changed);
        assert!(!control.receipts[&grant].contains_key(&5));
    }

    /// A phone's write lands on the note it names at the revision it
    /// opened, or stops: a moved note answers with itself, a gone one is
    /// gone (and deleting it again is done), and the description stays.
    #[test]
    fn a_phone_s_note_write_names_its_revision_and_keeps_the_description() {
        use mesimon_core::board::NoteMeta;
        let meta = |n: u64, rev| NoteMeta {
            id: ulid::Ulid::from_parts(n, 1),
            name: format!("note {n}"),
            rev,
            created_at: "@1".into(),
            created_by: "local".into(),
            edited_at: "@2".into(),
            edited_by: "agent:00000000-0000-0000-0000-000000000000".into(),
        };
        let t = Ticket {
            id: ulid::Ulid(1),
            short_key: "T-1".into(),
            title: "t".into(),
            column: "TODO".into(),
            order: "a0".into(),
            created_at: "@0".into(),
            created_by: String::new(),
            created_from: None,
            entered_at: None,
            woke_at: None,
            manual_merge: false,
            execution_policy: Default::default(),
            tier: None,
            envelope: None,
            workspace: None,
            import_origin: None,
            raised: None,
            previous_column: None,
            picked: None,
            tags: Vec::new(),
            notes: vec![meta(1, 1), meta(2, 3)],
            archived: None,
        };
        let (description, other) = (t.notes[0].id.to_string(), t.notes[1].id.to_string());
        assert_eq!(note_gate(&t, None, false, None), Ok(None));
        assert_eq!(note_gate(&t, Some(&other), false, Some(3)), Ok(Some(t.notes[1].id)));
        assert_eq!(note_gate(&t, Some(&other), true, Some(3)), Ok(Some(t.notes[1].id)));
        assert_eq!(note_gate(&t, Some(&other), false, None), Ok(Some(t.notes[1].id)));
        assert_eq!(note_gate(&t, Some(&other), false, Some(2)), Err(NoteGate::Stale(&t.notes[1])));
        assert_eq!(note_gate(&t, Some(&description), true, Some(1)), Err(NoteGate::Description));
        assert_eq!(note_gate(&t, Some(&description), false, Some(1)), Ok(Some(t.notes[0].id)));
        let gone = ulid::Ulid::from_parts(9, 9).to_string();
        assert_eq!(note_gate(&t, Some(&gone), true, Some(1)), Err(NoteGate::Gone));
        assert_eq!(note_gate(&t, Some(&gone), false, Some(1)), Err(NoteGate::Missing));
        assert_eq!(note_gate(&t, Some("not a ulid"), false, None), Err(NoteGate::Missing));
        let Reply::NoteStale { note, .. } =
            NoteGate::Stale(&t.notes[1]).reply("T", |_| "agent".into())
        else {
            panic!("stale")
        };
        assert_eq!((note.rev, note.by.as_str(), note.at), (3, "agent", 2000));
        assert!(matches!(
            NoteGate::Gone.reply("T", |_| String::new()),
            Reply::NoteWritten { note: None, .. }
        ));
    }

    #[test]
    fn a_filed_ticket_wears_only_tags_the_board_has() {
        let mut board = mesimon_core::board::Board::with_default_columns();
        board.register_tag(1, "BUG").unwrap();
        board.register_tag(2, "QUESTION").unwrap();
        let pick = |group, name: &str| api::TagPick { group, name: name.into() };
        let worn = filed_tags(&board, &[pick(1, "BUG"), pick(1, "BUG"), pick(2, "QUESTION")])
            .unwrap()
            .into_iter()
            .map(|t| (t.group, t.name))
            .collect::<Vec<_>>();
        assert_eq!(worn, vec![(1, "BUG".to_string()), (2, "QUESTION".to_string())]);
        for stale in [pick(1, "bug"), pick(2, "BUG"), pick(1, "NEW")] {
            assert!(filed_tags(&board, &[stale]).is_err());
        }
        assert!(board.tag_def(1, "NEW").is_none(), "a phone never registers a tag");
    }

    /// A phone's rename (T-530) is the desk's title, refused blank, and
    /// the same title again changes nothing; its tag is one the board has,
    /// on that group, and wearing what is worn changes nothing.
    #[test]
    fn a_phone_retitles_and_tags_with_the_board_s_own_words() {
        let mut board = Board::with_default_columns();
        board.register_tag(1, "BUG").unwrap();
        board.register_tag(1, "FEATURE").unwrap();
        board.register_tag(2, "QUESTION").unwrap();
        let t = Ticket {
            id: ulid::Ulid(1),
            short_key: "T-1".into(),
            title: "Fix it".into(),
            column: "TODO".into(),
            order: "a0".into(),
            created_at: "@0".into(),
            created_by: String::new(),
            created_from: None,
            entered_at: None,
            woke_at: None,
            manual_merge: false,
            execution_policy: Default::default(),
            tier: None,
            envelope: None,
            workspace: None,
            import_origin: None,
            raised: None,
            previous_column: None,
            picked: None,
            tags: vec![mesimon_core::board::TagRef { name: "BUG".into(), group: 1 }],
            notes: Vec::new(),
            archived: None,
        };
        assert_eq!(phone_title(&t, "  Fix it  "), Ok(None));
        assert_eq!(
            phone_title(&t, "Fix the\u{1b}[31m login"),
            Ok(Some("Fix the[31m login".into()))
        );
        for blank in ["", "   ", "\u{1b}"] {
            assert_eq!(phone_title(&t, blank), Err("a ticket needs a title"), "{blank:?}");
        }
        let long = phone_title(&t, &"ש".repeat(5000)).unwrap().unwrap();
        assert!(long.len() <= mesimon_core::board::TITLE_MAX_BYTES);
        assert_eq!(phone_tag(&board, &t, 1, Some("BUG")), Ok(false));
        assert_eq!(phone_tag(&board, &t, 1, Some("FEATURE")), Ok(true));
        assert_eq!(phone_tag(&board, &t, 1, None), Ok(true));
        assert_eq!(phone_tag(&board, &t, 2, None), Ok(false));
        assert_eq!(phone_tag(&board, &t, 2, Some("QUESTION")), Ok(true));
        for (group, stale) in [(1, "bug"), (2, "BUG"), (1, "NEW"), (11, "BUG")] {
            assert!(phone_tag(&board, &t, group, Some(stale)).is_err(), "{group} {stale}");
        }
        assert!(board.tag_def(1, "NEW").is_none(), "a phone never registers a tag");
    }

    #[test]
    fn permission_connection_stays_duplex_until_process_cancellation() {
        let (host, hook) = UnixStream::pair().unwrap();
        assert!(!permission_peer_closed(&host));
        drop(hook);
        assert!(permission_peer_closed(&host));
    }

    fn wait(stream: UnixStream, grants: Vec<BoardId>, renews: bool) -> PermissionWait {
        let now = Instant::now();
        PermissionWait {
            projection: api::Permission {
                request: "r1".into(),
                tool: "Bash".into(),
                input: serde_json::json!({"command": "ls"}),
                expires_at: 0,
            },
            road: PermissionRoad::Hook { stream, renews, closed: None },
            offered: now,
            deadline: now + Duration::from_secs(mesimon_core::mesophon::PERMISSION_HOLD_SECS),
            grants,
        }
    }

    /// The hook road's stream and close, for the tests that watch them.
    fn hook_road(p: &PermissionWait) -> (&UnixStream, Option<Instant>) {
        match &p.road {
            PermissionRoad::Hook { stream, closed, .. } => (stream, *closed),
            PermissionRoad::Codex => panic!("a hook road"),
        }
    }

    /// A dropped peer's close is not visible to a `recv` at once under load
    /// (the daemon asks again every tick); a test waits the kernel out.
    fn await_peer_closed(stream: &UnixStream) {
        let until = Instant::now() + Duration::from_secs(1);
        while !permission_peer_closed(stream) {
            assert!(Instant::now() < until, "the dropped peer's close never showed");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// T-632: a phone's answer is held past a minute, for whichever paired
    /// device comes back to it, until its run goes or its device is revoked.
    #[test]
    fn a_permission_wait_outlives_a_minute_and_a_phone_s_reconnect() {
        let phone = BoardId::random();
        let (host, hook) = UnixStream::pair().unwrap();
        let p = wait(host, vec![phone], false);
        let later = Instant::now() + Duration::from_secs(3600);
        assert!(!p.lapsed(later, |g| g == phone), "an hour on, the phone still answers");
        assert!(p.lapsed(later, |_| false), "a revoked phone ends it");
        assert!(p.lapsed(p.deadline, |g| g == phone), "a day ends it");
        drop(hook);
        await_peer_closed(hook_road(&p).0);
        assert!(p.lapsed(Instant::now(), |g| g == phone), "a hook set's run gone ends it");
    }

    /// T-846: a Codex approval's wait has no run to end it: it stands for
    /// its day or until its request leaves the snapshot, and the mod's
    /// renewal never takes it over.
    #[test]
    fn a_codex_approval_s_wait_has_no_run_and_is_never_renewed() {
        let phone = BoardId::random();
        let now = Instant::now();
        let p = PermissionWait {
            projection: api::Permission {
                request: "\"9\"".into(),
                tool: "command".into(),
                input: serde_json::json!({"command": "printf x > out"}),
                expires_at: 0,
            },
            road: PermissionRoad::Codex,
            offered: now,
            deadline: now + Duration::from_secs(mesimon_core::mesophon::PERMISSION_HOLD_SECS),
            grants: vec![phone],
        };
        assert!(p.codex());
        assert!(!p.lapsed(now + Duration::from_secs(3600), |g| g == phone));
        assert!(p.lapsed(p.deadline, |g| g == phone));
        assert!(p.lapsed(now, |_| false));
        assert!(!takes_over(&p, "command", &serde_json::json!({"command": "printf x > out"})));
    }

    /// The mod's rounds: a renewing run that closed keeps its wait for the
    /// grace, and the next run of the same dialog takes it over.
    #[test]
    fn a_renewing_run_s_wait_waits_for_the_next_round() {
        let phone = BoardId::random();
        let (host, hook) = UnixStream::pair().unwrap();
        let mut p = wait(host, vec![phone], true);
        let now = Instant::now();
        p.note_closed(now);
        assert_eq!(hook_road(&p).1, None, "open while its run lives");
        drop(hook);
        await_peer_closed(hook_road(&p).0);
        p.note_closed(now);
        assert_eq!(hook_road(&p).1, Some(now));
        assert!(!p.lapsed(now, |g| g == phone), "within the grace");
        assert!(p.lapsed(now + PERMISSION_RENEW_GRACE, |g| g == phone), "past it");
        assert!(takes_over(&p, "Bash", &serde_json::json!({"command": "ls"})));
        assert!(!takes_over(&p, "Bash", &serde_json::json!({"command": "rm x"})));
        assert!(!takes_over(&p, "Write", &serde_json::json!({"command": "ls"})));
    }

    #[test]
    fn pairing_proof_is_device_bound_and_expires() {
        let owner = mesimon_team::crypto::DeviceKeys::generate();
        let browser = mesimon_team::crypto::DeviceKeys::generate();
        let now = Instant::now();
        let invite = Invite {
            code: InviteCode::mint(&owner.public()),
            expires: now + Duration::from_secs(600),
        };
        let proof = hex::encode(&invite.code.proof(&browser.public()));
        assert!(invite.accepts(&browser.public(), &proof, now));
        assert!(!invite.accepts(&owner.public(), &proof, now));
        assert!(!invite.accepts(&browser.public(), &proof, invite.expires));
        assert!(!invite.accepts(&browser.public(), "invalid", now));
    }

    /// Codex 0.163.0's question dialog as captured on 2026-10-17 (T-846), the
    /// cursor on its first row.
    const CODEX_QUESTION: &str = "\
  Question 1/1 (1 unanswered)
  Choose fixture color?

  › 1. Blue               Use blue for the fixture.
    2. Green              Use green for the fixture.
    3. None of the above  Optionally, add details in notes (tab)

  tab to add notes | enter to submit answer | esc to interrupt
";

    /// Codex 0.163.0's command approval dialog as captured on 2026-10-17
    /// (T-846), its second row wrapped by the pane.
    const CODEX_APPROVAL: &str = "\
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

  Press enter to confirm or esc to cancel
";

    fn codex_question() -> api::Dialog {
        api::Dialog {
            request: "\"q1\"".into(),
            content: api::DialogContent::Questions {
                questions: vec![api::Question {
                    question: "Choose fixture color?".into(),
                    header: "Color".into(),
                    multi_select: false,
                    options: vec![
                        api::QuestionOption {
                            label: "Blue".into(),
                            description: "Use blue for the fixture.".into(),
                        },
                        api::QuestionOption {
                            label: "Green".into(),
                            description: "Use green for the fixture.".into(),
                        },
                    ],
                }],
            },
        }
    }

    /// T-846: a Codex question's choice walks `›` to the option's row (its
    /// label and description read as one) and presses Enter; a refusal is
    /// Esc; words, batches and Claude's screen are no shape.
    #[test]
    fn a_codex_question_walks_its_rows_by_label_and_description() {
        let dialog = codex_question();
        let choose = |i| api::DialogAnswer::Choice { index: i };
        let step = |response, screen: &str| {
            dialog_step_for(
                SessionKind::Codex,
                Standing::Dialog(&dialog, &response),
                screen,
                &Walk::default(),
            )
        };
        assert_eq!(step(choose(0), CODEX_QUESTION), Ok(DialogStep::Submit));
        assert_eq!(step(choose(1), CODEX_QUESTION), Ok(DialogStep::Down));
        let on_green = CODEX_QUESTION.replace("› 1.", "  1.").replace("  2.", "› 2.");
        assert_eq!(step(choose(1), &on_green), Ok(DialogStep::Submit));
        assert_eq!(step(choose(0), &on_green), Ok(DialogStep::Up));
        assert_eq!(step(api::DialogAnswer::Reject, CODEX_QUESTION), Ok(DialogStep::Reject));
        assert_eq!(step(choose(5), CODEX_QUESTION), Err(Miss::ShapeUnrecognised));
        assert_eq!(
            step(api::DialogAnswer::Text { text: "Purple".into() }, CODEX_QUESTION),
            Err(Miss::ShapeUnrecognised),
            "words go in Codex's notes row, unmeasured"
        );
        assert_eq!(
            step(
                api::DialogAnswer::Answers {
                    answers: vec![api::QuestionAnswer::Choice { index: 0 }]
                },
                CODEX_QUESTION
            ),
            Ok(DialogStep::Submit),
            "the crown's one answer per question, for one question"
        );
        for bad in [
            "Choose fixture color?\n❯ 1. Blue\n  2. Green\nEnter to select · Esc to cancel",
            "Question 1/1\nChoose fixture color?\n  1. Blue\n  2. Green\ntab to add notes | enter to submit answer",
            "Question 1/1\nChoose fixture color?\n› 1. Blue\n› 2. Green\nenter to submit answer",
            "Question 1/1\nWhich region?\n› 1. Blue\n  2. Green\nenter to submit answer",
            "› Ask Codex to do anything",
        ] {
            assert_eq!(step(choose(0), bad), Err(Miss::ShapeUnrecognised), "{bad}");
        }
        assert!(dialog_shape_for(SessionKind::Codex, &dialog, CODEX_QUESTION));
        assert!(!dialog_shape_for(SessionKind::Claude, &dialog, CODEX_QUESTION));
        let mut two = dialog.clone();
        if let api::DialogContent::Questions { questions } = &mut two.content {
            questions.push(questions[0].clone());
        }
        assert!(!codex_takes(
            &two,
            &api::DialogAnswer::Answers {
                answers: vec![
                    api::QuestionAnswer::Choice { index: 0 },
                    api::QuestionAnswer::Choice { index: 0 }
                ]
            }
        ));
        assert!(dialog_answerable_for(SessionKind::Codex, questions_of(&dialog)));
        assert!(!dialog_answerable_for(SessionKind::Codex, questions_of(&two)));
    }

    /// The approval the phone answered, as the snapshot carried it on 0.163.0:
    /// the item's command is the shell wrapper, the dialog shows the words
    /// inside, and the reason is drawn as `Reason: …`.
    fn codex_approval(reason: Option<&str>, command: Option<&str>) -> api::Permission {
        let mut input = serde_json::Map::new();
        if let Some(reason) = reason {
            input.insert("reason".into(), reason.into());
        }
        if let Some(command) = command {
            input.insert("command".into(), command.into());
        }
        api::Permission {
            request: "\"0\"".into(),
            tool: "command".into(),
            input: serde_json::Value::Object(input),
            expires_at: 0,
        }
    }

    /// T-846: an approval's allow is Enter on the `Yes, proceed` row, a deny
    /// walks down to the one `No` row; the heading is not read, a second
    /// `No` row or a missing footer is no shape, and a dialog that does not
    /// show the answered request's reason or command gets no key.
    #[test]
    fn a_codex_approval_walks_to_yes_or_no_and_confirms() {
        let reason = "May I write this disposable fixture file?";
        let command = "/bin/zsh -lc 'printf MESIMON_REFUSED_WRITE > \
                       /private/tmp/msmn-e2e-codex-runtime-apj3xgcw/approval-refused.txt'";
        let p = codex_approval(Some(reason), Some(command));
        assert_eq!(codex_approval_step(CODEX_APPROVAL, true, &p), Ok(DialogStep::Submit));
        assert_eq!(codex_approval_step(CODEX_APPROVAL, false, &p), Ok(DialogStep::Down));
        // T-873: the dialog walker takes an approval to these rows.
        let walked = |allow| {
            let standing = Standing::Approval(&p, allow);
            dialog_step_for(SessionKind::Codex, standing, CODEX_APPROVAL, &Walk::default())
        };
        assert_eq!(walked(true), Ok(DialogStep::Submit));
        assert_eq!(walked(false), Ok(DialogStep::Down));
        let on_two =
            CODEX_APPROVAL.replace("› 1.", "  1.").replace("  2. Yes, and", "› 2. Yes, and");
        assert_eq!(codex_approval_step(&on_two, true, &p), Ok(DialogStep::Up));
        assert_eq!(codex_approval_step(&on_two, false, &p), Ok(DialogStep::Down));
        let on_no = CODEX_APPROVAL.replace("› 1.", "  1.").replace("  3. No", "› 3. No");
        assert_eq!(codex_approval_step(&on_no, false, &p), Ok(DialogStep::Submit));
        assert_eq!(codex_approval_step(&on_no, true, &p), Ok(DialogStep::Up));
        let edits = CODEX_APPROVAL.replace(
            "Would you like to run the following command?",
            "Would you like to make the following edits?",
        );
        assert_eq!(codex_approval_step(&edits, true, &p), Ok(DialogStep::Submit));
        for bad in [
            CODEX_APPROVAL.replace("Press enter to confirm or esc to cancel", "").as_str(),
            CODEX_APPROVAL.replace("› 1.", "  1.").as_str(),
            CODEX_QUESTION,
            "› Ask Codex to do anything",
        ] {
            assert_eq!(codex_approval_step(bad, true, &p), Err(Miss::ShapeUnrecognised), "{bad}");
        }
        // Bound to the request: the command's tail stands in for a missing
        // reason; another request's dialog, or a request with neither, gets
        // no key.
        let by_command = codex_approval(None, Some(command));
        assert_eq!(codex_approval_step(CODEX_APPROVAL, true, &by_command), Ok(DialogStep::Submit));
        let other = codex_approval(Some("Delete the build directory?"), Some("rm -rf target"));
        assert_eq!(codex_approval_step(CODEX_APPROVAL, true, &other), Err(Miss::LabelNotFound));
        let other_command = codex_approval(None, Some("/bin/zsh -lc 'rm -rf target'"));
        assert_eq!(
            codex_approval_step(CODEX_APPROVAL, true, &other_command),
            Err(Miss::LabelNotFound)
        );
        let bare = codex_approval(None, None);
        assert_eq!(codex_approval_step(CODEX_APPROVAL, true, &bare), Err(Miss::LabelNotFound));
    }

    /// T-846: a Codex plan's accept is the T-420 press, at the `Yes` row
    /// alone; sending it back is unmeasured and stays the pane's.
    #[test]
    fn a_codex_plan_takes_the_accept_at_its_yes_row_alone() {
        let plan = api::Dialog {
            request: "plan:abc".into(),
            content: api::DialogContent::Plan { markdown: "Ship it.".into() },
        };
        let dialog = "Implement this plan?\n› Yes, implement this plan\n  No, stay in Plan mode\n";
        let step = |response, screen: &str| {
            dialog_step_for(
                SessionKind::Codex,
                Standing::Dialog(&plan, &response),
                screen,
                &Walk::default(),
            )
        };
        assert_eq!(step(api::DialogAnswer::Accept, dialog), Ok(DialogStep::Submit));
        let no = dialog.replace("› Yes", "  Yes").replace("  No", "› No");
        assert_eq!(step(api::DialogAnswer::Accept, &no), Err(Miss::ShapeUnrecognised));
        assert_eq!(step(api::DialogAnswer::Reject, dialog), Err(Miss::ShapeUnrecognised));
        assert!(dialog_shape_for(SessionKind::Codex, &plan, dialog));
        assert!(!dialog_shape_for(SessionKind::Codex, &plan, "› Ask Codex to do anything"));
    }

    /// T-846: a Codex question becomes the phone's dialog within the bounds
    /// Claude's keeps; a secret or option-less question draws no card.
    #[test]
    fn a_codex_question_is_projected_within_the_phone_s_bounds() {
        use crate::agents::codex::{AskOption, AskQuestion};
        let q = |options: usize, secret: bool| AskQuestion {
            question: "Choose fixture color?".into(),
            header: String::new(),
            options: (0..options)
                .map(|i| AskOption { label: format!("Option {i}"), description: String::new() })
                .collect(),
            secret,
        };
        let Some(api::DialogContent::Questions { questions }) =
            Daemon::codex_questions(&[q(2, false)])
        else {
            panic!("a dialog")
        };
        assert_eq!(questions[0].header, "Choose", "the first word stands in for a header");
        assert_eq!(questions[0].options.len(), 2);
        assert!(!questions[0].multi_select);
        assert!(Daemon::codex_questions(&[q(2, true)]).is_none(), "a secret stays the pane's");
        assert!(Daemon::codex_questions(&[q(0, false)]).is_none(), "free words are unmeasured");
        assert!(Daemon::codex_questions(&[q(9, false)]).is_none());
        assert!(Daemon::codex_questions(&[]).is_none());
        assert!(Daemon::codex_questions(&vec![q(2, false); 5]).is_none());
    }

    /// Claude Code 2.1.310's plan dialog as captured on 2026-10-24 (T-943),
    /// the plan's own numbered steps above it, with `field` drawn where the
    /// feedback row's label is and the cursor `❯` on row `at` (0-based).
    fn plan_screen(field: &str, at: usize) -> String {
        let rows = ["Yes, and use auto mode", "Yes, manually approve edits", field];
        let mut screen = String::from(
            "   Ready to code?\n   Here is Claude's plan:\n   Steps\n   1. Write a.txt with \
             the content hi.\n   2. Verify by reading a.txt.\n   Claude has written up a plan \
             and is ready to execute. Would you like to proceed?\n",
        );
        for (i, row) in rows.iter().enumerate() {
            let mark = if i == at { "❯" } else { " " };
            screen.push_str(&format!("   {mark} {}. {row}\n", i + 1));
        }
        screen.push_str("        shift+tab to approve with this feedback\n");
        screen.push_str("   ctrl+g to edit in Nvim · ~/.claude/plans/peaceful-brooks.md\n");
        screen
    }

    fn plan_dialog() -> api::Dialog {
        api::Dialog {
            request: "toolu_p".into(),
            content: api::DialogContent::Plan { markdown: "1. Write a.txt".into() },
        }
    }

    /// T-943: the walk to the feedback row reads its measured label, moves
    /// one row a key, and pastes only with the cursor on the empty field.
    #[test]
    fn a_plan_sent_back_walks_to_the_empty_feedback_field_and_pastes_there() {
        let d = plan_dialog();
        let words = "Use echo in Bash instead of the Write tool.";
        let empty = |at| plan_screen("Tell Claude what to change", at);
        assert_eq!(revise_step(&d, words, &empty(0), &Walk::default()), Ok(DialogStep::Down));
        let after_one = Walk { at: Some(1), ..Walk::default() };
        assert_eq!(revise_step(&d, words, &empty(1), &after_one), Ok(DialogStep::Down));
        let after_two = Walk { at: Some(2), ..Walk::default() };
        assert_eq!(
            revise_step(&d, words, &empty(2), &after_two),
            Ok(DialogStep::Paste { question: 2, text: words.into() })
        );
        // A cursor the arrow did not put there is a person's hand.
        assert_eq!(revise_step(&d, words, &empty(0), &after_one), Err(Miss::CursorMoved));
        // The older dialog's four rows (`No, keep planning` third): the
        // row is found by its label, never by its place.
        let four = "Would you like to proceed?\n ❯ 1. Yes, and use auto mode\n   2. Yes, \
                    manually approve edits\n   3. No, keep planning\n   4. Tell Claude what \
                    to change\n";
        assert_eq!(revise_step(&d, words, four, &Walk::default()), Ok(DialogStep::Down));
        let on_four = four.replace("❯ 1.", "  1.").replace("  4.", "❯ 4.");
        let walk = Walk { at: Some(3), ..Walk::default() };
        assert_eq!(
            revise_step(&d, words, &on_four, &walk),
            Ok(DialogStep::Paste { question: 3, text: words.into() })
        );
    }

    /// T-943: Enter only once the field reads the words whole, its hints
    /// aside; a field still empty waits for the paste to draw, and anything
    /// else (other words, a cursor gone, a field typed into first) sends
    /// nothing.
    #[test]
    fn a_plan_sent_back_presses_enter_only_on_its_own_words() {
        let d = plan_dialog();
        let words = "Use echo in Bash instead of the Write tool.";
        let pasted = Walk { pasted: vec![2], at: Some(2), ..Walk::default() };
        let typed = plan_screen(words, 2);
        assert_eq!(revise_step(&d, words, &typed, &pasted), Ok(DialogStep::Submit));
        let empty = plan_screen("Tell Claude what to change", 2);
        assert_eq!(revise_step(&d, words, &empty, &pasted), Ok(DialogStep::Wait));
        // A long line wraps under its row and still reads whole.
        let long = "Name the file b.txt instead, write the word bye into it, and verify it \
                    by reading it back before the turn ends.";
        let wrapped = plan_screen(
            "Name the file b.txt instead, write the word bye into it, and verify\n        it by \
             reading it back before the turn ends.",
            2,
        );
        assert_eq!(revise_step(&d, long, &wrapped, &pasted), Ok(DialogStep::Submit));
        // A prefix is not the words, nor are the words with more after them.
        let cut = plan_screen("Use echo in Bash", 2);
        assert_eq!(revise_step(&d, words, &cut, &pasted), Err(Miss::FieldTouched));
        let more = plan_screen(&format!("{words} and more"), 2);
        assert_eq!(revise_step(&d, words, &more, &pasted), Err(Miss::FieldTouched));
        // A person moved the cursor back to an accept row after the paste.
        assert_eq!(revise_step(&d, words, &plan_screen(words, 0), &pasted), Err(Miss::CursorMoved));
        // Words already in the field before any paste are a person's.
        let theirs = plan_screen("my own note", 0);
        assert_eq!(revise_step(&d, words, &theirs, &Walk::default()), Err(Miss::FieldTouched));
        // The crown's own words may spell a hint: inside the field's own
        // line they are the field's, read whole.
        let hinted = "see ctrl+g to edit in Nvim";
        assert_eq!(
            revise_step(&d, hinted, &plan_screen(hinted, 2), &pasted),
            Ok(DialogStep::Submit)
        );
    }

    /// T-943: a plan sent back claims `input_sent` only on evidence the
    /// dialog left. At the window's end with the projection standing, a
    /// failed capture is `unknown { pane_unreachable }`, never the dialog
    /// gone, and a screen still drawing it is `dialog_stands`.
    #[test]
    fn a_plan_sent_back_claims_the_dialog_gone_only_on_a_screen_read() {
        let never = || -> Option<String> { panic!("no capture before the window's end") };
        assert_eq!(revise_settled(None, true, false, never), None, "waits");
        assert_eq!(
            revise_settled(None, true, true, || None),
            Some(("unknown", Some("pane_unreachable")))
        );
        let up = plan_screen("Use echo", 2);
        assert_eq!(
            revise_settled(None, true, true, || Some(up.clone())),
            Some(("unknown", Some("dialog_stands")))
        );
        // Gone only on a screen that shows what the dialog left for (T-955).
        let rule = "─".repeat(40);
        let composer = format!("✻ Thinking…\n{rule}\n❯ \n{rule}\n  ? for shortcuts");
        assert_eq!(
            revise_settled(None, true, true, || Some(composer.clone())),
            Some(("input_sent", None))
        );
        let clipped = "   Claude has written up a plan and is ready to execute. Would you like \
                       to proceed?\n   ❯ 3. Tell Claude what to change\n";
        let question = "Would you like to proceed?";
        for unseen in ["", "   ", question, clipped, "❯ the composer"] {
            assert_eq!(
                revise_settled(None, true, true, || Some(unseen.into())),
                Some(("unknown", Some("shape_unrecognised"))),
                "{unseen:?}"
            );
        }
        assert_eq!(revise_settled(None, false, false, never), Some(("input_sent", None)));
        assert_eq!(
            revise_settled(Some(DialogEdge::Dismissed), true, false, never),
            Some(("input_sent", None))
        );
        for accepted in [DialogEdge::Answered, DialogEdge::ModAnswered] {
            assert_eq!(
                revise_settled(Some(accepted), true, true, never),
                Some(("unknown", Some("plan_accepted")))
            );
        }
    }

    /// T-943, the delivery read's two counterexamples: a person's draft that
    /// spells a hint, the real hint lines still drawn under it, never reads
    /// as the empty field (so no paste goes into it) nor as the crown's
    /// words (so no Enter sends theirs with them). Only whole lines at the
    /// row's end are hints.
    #[test]
    fn a_draft_that_spells_a_hint_is_never_the_field_the_crown_wants() {
        let d = plan_dialog();
        let draft = plan_screen("Tell Claude what to change ctrl+g to edit MY DRAFT", 2);
        let walk = Walk { at: Some(2), ..Walk::default() };
        assert_eq!(revise_step(&d, "Use echo", &draft, &walk), Err(Miss::FieldTouched));
        let pasted = Walk { pasted: vec![2], at: Some(2), ..Walk::default() };
        let more = plan_screen("Use echo ctrl+g to edit AND MY DRAFT", 2);
        assert_eq!(revise_step(&d, "Use echo", &more, &pasted), Err(Miss::FieldTouched));
        // The same draft on a line of its own under the field, a wrap.
        let wrapped = plan_screen("Use echo\n        ctrl+g to edit AND MY DRAFT", 2);
        assert_eq!(revise_step(&d, "Use echo", &wrapped, &pasted), Err(Miss::FieldTouched));
        // A draft line that is the approve hint itself is one line too many:
        // each hint comes off once, so the field keeps it.
        let twice = plan_screen("Use echo\n        shift+tab to approve with this feedback", 2);
        assert_eq!(revise_step(&d, "Use echo", &twice, &pasted), Err(Miss::FieldTouched));
        // The measured screen, hints and all, still reads the words alone.
        assert_eq!(
            revise_step(&d, "Use echo", &plan_screen("Use echo", 2), &pasted),
            Ok(DialogStep::Submit)
        );
    }

    /// T-943, the third read: the approve hint comes off only with the editor
    /// line under it. A capture that ends on it alone (the real footer cut
    /// off, or the line the person's own) keeps it as field text, so the
    /// field is not the crown's words, nor the empty label.
    #[test]
    fn the_approve_hint_alone_is_the_field_s() {
        let d = plan_dialog();
        let alone = |field: &str| {
            let screen = plan_screen(field, 2);
            let cut = screen.find("   ctrl+g to edit").expect("the editor line");
            screen[..cut].to_string()
        };
        let pasted = Walk { pasted: vec![2], at: Some(2), ..Walk::default() };
        assert_eq!(
            revise_step(&d, "Use echo", &alone("Use echo"), &pasted),
            Err(Miss::FieldTouched)
        );
        let walk = Walk { at: Some(2), ..Walk::default() };
        assert_eq!(
            revise_step(&d, "Use echo", &alone("Tell Claude what to change"), &walk),
            Err(Miss::FieldTouched)
        );
        // The pair still comes off whole, and an editor line alone too.
        assert_eq!(
            revise_step(&d, "Use echo", &plan_screen("Use echo", 2), &pasted),
            Ok(DialogStep::Submit)
        );
        let editor_only = plan_screen("Use echo", 2)
            .replace("        shift+tab to approve with this feedback\n", "");
        assert_eq!(revise_step(&d, "Use echo", &editor_only, &pasted), Ok(DialogStep::Submit));
    }

    /// T-943, the third read: a draft in the field that holds a numbered
    /// menu of its own, a later `1.` with a cursor, never replaces the
    /// dialog's menu. The rows run 1, 2, 3 from the question, and a
    /// numbered line out of that order stops the walk, before the paste
    /// and after it, as a second question does.
    #[test]
    fn a_draft_s_own_numbered_menu_never_restarts_the_plan_menu() {
        let d = plan_dialog();
        let pasted = Walk { pasted: vec![2], at: Some(2), ..Walk::default() };
        let menu = "my notes\n   1. Yes, and use auto mode\n   2. Yes, manually approve \
                    edits\n   ❯ 3. Use echo";
        let submit = plan_screen(menu, 2).replacen("❯ 3. my notes", "  3. my notes", 1);
        assert_eq!(revise_step(&d, "Use echo", &submit, &pasted), Err(Miss::ShapeUnrecognised));
        let empty = "my notes\n   1. a\n   2. b\n   ❯ 3. Tell Claude what to change";
        let paste = plan_screen(empty, 2).replacen("❯ 3. my notes", "  3. my notes", 1);
        let walk = Walk { at: Some(2), ..Walk::default() };
        assert_eq!(revise_step(&d, "Use echo", &paste, &walk), Err(Miss::ShapeUnrecognised));
        assert_eq!(plan_cursor(&paste), None, "no arrow is aimed by the draft's cursor");
        let again = plan_screen("Use echo\n Would you like to proceed?\n ❯ 1. Yes", 2);
        assert_eq!(revise_step(&d, "Use echo", &again, &pasted), Err(Miss::ShapeUnrecognised));
        // The plan's own numbered steps above the question are no rows.
        assert_eq!(
            revise_step(&d, "Use echo", &plan_screen("Use echo", 2), &pasted),
            Ok(DialogStep::Submit)
        );
    }

    /// T-943, the second read's counterexamples: a draft line that holds
    /// `Enter to select`, or that follows a blank line, is the field's and
    /// never cut off it, so the field is not the crown's words and no Enter
    /// goes; before the paste, the label with a draft under a blank line is
    /// not the empty field and no paste goes. A numbered line after the
    /// feedback row is no row of the dialog's: the walk stops.
    #[test]
    fn a_draft_past_a_blank_or_beside_a_footer_s_words_is_still_the_field() {
        let d = plan_dialog();
        let pasted = Walk { pasted: vec![2], at: Some(2), ..Walk::default() };
        let footer = plan_screen("Use echo\n        Enter to select AND MY DRAFT", 2);
        assert_eq!(revise_step(&d, "Use echo", &footer, &pasted), Err(Miss::FieldTouched));
        let blank = plan_screen(
            "Use echo\n        shift+tab to approve with this feedback\n\n        MY DRAFT",
            2,
        );
        assert_eq!(revise_step(&d, "Use echo", &blank, &pasted), Err(Miss::FieldTouched));
        let rule = plan_screen("Use echo\n        ────────\n        MY DRAFT", 2);
        assert_eq!(revise_step(&d, "Use echo", &rule, &pasted), Err(Miss::FieldTouched));
        let walk = Walk { at: Some(2), ..Walk::default() };
        let label = plan_screen("Tell Claude what to change\n\n        MY DRAFT", 2);
        assert_eq!(revise_step(&d, "Use echo", &label, &walk), Err(Miss::FieldTouched));
        let footer = plan_screen("Tell Claude what to change\n        Enter to select MY DRAFT", 2);
        assert_eq!(revise_step(&d, "Use echo", &footer, &walk), Err(Miss::FieldTouched));
        let numbered = plan_screen("Use echo\n   4. my own row", 2);
        assert_eq!(revise_step(&d, "Use echo", &numbered, &pasted), Err(Miss::FieldTouched));
        // Blank lines after the dialog are the screen's, not the field's.
        let tail = format!("{}\n\n\n", plan_screen("Use echo", 2));
        assert_eq!(revise_step(&d, "Use echo", &tail, &pasted), Ok(DialogStep::Submit));
        let empty = format!("{}\n\n", plan_screen("Tell Claude what to change", 2));
        assert_eq!(
            revise_step(&d, "Use echo", &empty, &walk),
            Ok(DialogStep::Paste { question: 2, text: "Use echo".into() })
        );
    }

    /// T-943: no plan dialog, two cursors, or a question's dialog: no key.
    #[test]
    fn a_plan_sent_back_sends_nothing_into_another_screen() {
        let d = plan_dialog();
        let words = "x";
        assert_eq!(
            revise_step(&d, words, "❯ ordinary composer", &Walk::default()),
            Err(Miss::ShapeUnrecognised)
        );
        let two = plan_screen("Tell Claude what to change", 0).replace("  2.", "❯ 2.");
        assert_eq!(revise_step(&d, words, &two, &Walk::default()), Err(Miss::ShapeUnrecognised));
        let question = api::Dialog {
            request: "toolu_p".into(),
            content: api::DialogContent::Questions { questions: vec![] },
        };
        let screen = plan_screen("Tell Claude what to change", 0);
        assert_eq!(
            revise_step(&question, words, &screen, &Walk::default()),
            Err(Miss::ShapeUnrecognised)
        );
    }

    /// T-943: the crown's words are refused, never rewritten: blank, a
    /// newline, past 1000 bytes, or anything `sanitize_prompt` would change
    /// (its trim among them); what passes is the words exactly.
    #[test]
    fn a_plan_change_is_refused_never_rewritten() {
        assert_eq!(dialog_text_refusal("change", "Use echo,  not Write."), None);
        for (bad, says) in [
            ("", "blank"),
            ("   ", "blank"),
            (" padded ", "characters"),
            ("one\ntwo", "one line"),
            ("one\rtwo", "one line"),
            ("\u{1b}[31mred", "characters"),
        ] {
            let why = dialog_text_refusal("change", bad).expect(bad);
            assert!(why.starts_with("change is") && why.contains(says), "{bad:?}: {why}");
        }
        let long = "a".repeat(DIALOG_TEXT_MAX + 1);
        assert!(dialog_text_refusal("change", &long).unwrap().contains("1000"));
        assert_eq!(dialog_text_refusal("change", &"a".repeat(DIALOG_TEXT_MAX)), None);
    }
}
