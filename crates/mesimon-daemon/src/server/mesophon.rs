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
struct Grant {
    id: BoardId,
    device: DeviceId,
    public: DevicePublic,
    name: String,
}
#[derive(Clone, Serialize, Deserialize)]
struct Stored {
    schema: u32,
    board: BoardId,
    host: DeviceId,
    grants: Vec<Grant>,
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
/// How many filed envelopes `Stored::filed` remembers.
const FILED_KEEP: usize = 256;
/// Past this many bytes a board goes to the browser without its agents'
/// words, well inside the 48 KiB an answer may carry.
const BOARD_WORDS_BUDGET: usize = 40 * 1024;
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
struct DialogDelivery {
    grant: BoardId,
    device: DeviceId,
    command: u64,
    ticket: ulid::Ulid,
    request: String,
    response: api::DialogAnswer,
    deadline: Instant,
    next: Instant,
    steps: u8,
    pasted: bool,
}
struct PermissionWait {
    projection: api::Permission,
    stream: UnixStream,
    deadline: Instant,
    /// Only connections present when the prompt was offered may answer it.
    peers: Vec<String>,
}
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
    tx: Sender<Msg>,
    jobs: Option<SyncSender<Wire>>,
    generation: u64,
    stored: Option<Stored>,
    barred: bool,
    keys: Option<mesimon_team::crypto::DeviceKeys>,
    identity: Option<String>,
    invite: Option<Invite>,
    peers: HashMap<String, Peer>,
    high: HashMap<BoardId, u64>,
    receipts: HashMap<BoardId, BTreeMap<u64, Reply>>,
    pending: HashMap<uuid::Uuid, Pending>,
    /// Starts on their way, by ticket (T-498).
    starts: HashMap<ulid::Ulid, StartWait>,
    permissions: HashMap<uuid::Uuid, PermissionWait>,
    phases: HashMap<ulid::Ulid, api::Phase>,
    suppress_awareness: bool,
    dialogs: HashMap<uuid::Uuid, api::Dialog>,
    dialog_deliveries: HashMap<uuid::Uuid, DialogDelivery>,
    incarnation: ObjectId,
    origin: String,
    online: bool,
    /// This relay keeps mail for the host while it is away (T-497).
    mail: bool,
    error: Option<String>,
    retry: Instant,
    dirty: bool,
    /// Each transcript's words as a phone sees them, keyed by path (T-497).
    words: RefCell<HashMap<String, Words>>,
}
/// An agent's step and latest reply line, read from its transcript, and the
/// file's length and mtime when read: a browser asks for the board every two
/// seconds, and a transcript is re-read only when it changed.
struct Words {
    len: u64,
    mtime_ms: u64,
    doing: Option<String>,
    said: Option<String>,
}
impl Control {
    pub(super) fn new(tx: Sender<Msg>) -> Self {
        Self {
            tx,
            jobs: None,
            generation: 0,
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
            phases: HashMap::new(),
            suppress_awareness: false,
            dialogs: HashMap::new(),
            dialog_deliveries: HashMap::new(),
            incarnation: ObjectId::random(),
            origin: String::new(),
            online: false,
            mail: false,
            error: None,
            retry: Instant::now(),
            dirty: false,
            words: RefCell::new(HashMap::new()),
        }
    }
    fn send(&mut self, wire: Wire) {
        if self.jobs.as_ref().is_some_and(|tx| tx.try_send(wire).is_err()) {
            self.jobs = None;
            self.online = false;
            self.peers.clear();
            self.error = Some("connection interrupted".into());
        }
    }
    fn answer(&mut self, peer: &str, id: u64, reply: Reply) {
        if let (Some(p), Some(keys)) = (self.peers.get_mut(peer), self.keys.as_ref()) {
            let reply = if serde_json::to_vec(&reply).map_or(true, |v| v.len() > 48 * 1024) {
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
        if !cfg!(debug_assertions) && std::env::var("MESIMON_MESOPHON").as_deref() != Ok("1") {
            return;
        }
        match std::fs::read(self.paths.state_dir.join("mesophon.json")) {
            Ok(bytes) => match serde_json::from_slice::<Stored>(&bytes) {
                Ok(s)
                    if s.schema == 1
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
    fn control_save(&self, s: &Stored) -> anyhow::Result<()> {
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
        }
    }
    pub(super) fn control_local(&mut self, action: LocalAction) -> Response {
        let fail = |m: &str| Response::Err { message: m.into() };
        if !cfg!(debug_assertions) && std::env::var("MESIMON_MESOPHON").as_deref() != Ok("1") {
            return fail("Mesophon preview is not enabled");
        }
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
                    let s = Stored {
                        schema: 1,
                        board: BoardId::random(),
                        host: keys.id(),
                        grants: Vec::new(),
                        filed: Vec::new(),
                        noted: Vec::new(),
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
                match std::fs::remove_file(self.paths.state_dir.join("mesophon.json")) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => return fail("could not disable Mesophon"),
                }
                self.control_revoke_all();
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
                self.control.generation += 1;
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
        self.control_awareness();
    }
    pub(super) fn control_tick(&mut self) {
        self.control_expire_permissions();
        self.control_deliver_dialogs();
        if self.control.stored.is_none() {
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
    }
    pub(super) fn on_control(&mut self, generation: u64, event: NetEvent) {
        if generation != self.control.generation || self.control.stored.is_none() {
            return;
        }
        let announce = matches!(
            &event,
            NetEvent::Online(..)
                | NetEvent::Offline
                | NetEvent::Frame(Wire::Peer { .. } | Wire::Gone { .. } | Wire::Error { .. })
        );
        match event {
            NetEvent::Online(origin, mail) => {
                self.control.origin = origin;
                self.control.online = true;
                self.control.mail = mail;
                self.control.error = None;
                self.control_publish();
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
            NetEvent::Frame(Wire::Published) => {}
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
                    // Notes read and written from the ticket page (T-532).
                    "notes",
                    // Tickets for an away host wait at the relay (T-497).
                    if self.control.mail { "mailbox" } else { "" },
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
                .unwrap_or(Reply::Delivery { status: "unknown".into() });
            self.control.answer(peer, command.id, reply);
            return;
        }
        if let Some(p) = self.control.peers.get_mut(peer) {
            p.high = command.id;
        }
        let by = Principal::Paired { device: device.to_hex(), grant: grant.to_hex() };
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
                    Reply::Delivery { status: "observed".into() }
                }
            }
            api::Request::Dialog { ticket, session, request, response } => self
                .control_dialog_answer(
                    &by, grant, device, command.id, &ticket, &session, &request, response,
                ),
            api::Request::Permission { ticket, session, request, decision } => {
                self.control_permission_answer(&by, peer, &ticket, &session, &request, decision)
            }
            api::Request::Snapshot => {
                if let Some(p) = self.control.peers.get_mut(peer) {
                    p.subscribed = true;
                }
                if authorize(&by, &Action::Read, &Resource::Board).denied() {
                    Reply::Revoked
                } else {
                    self.control_board()
                }
            }
            api::Request::Preview { ticket, session } => {
                self.control_preview(&by, &ticket, &session)
            }
            api::Request::Prompt { ticket, session, text, queued } => self.control_prompt(
                &by,
                grant,
                device,
                command.id,
                (&ticket, &session),
                text,
                queued,
            ),
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
                .unwrap_or(Reply::Delivery { status: "unknown".into() }),
            api::Request::Create { title, description, column, tags } => {
                self.control_create(&by, title, description, column, &tags, None)
            }
            api::Request::Start { ticket, prompt } => {
                self.control_start_agent(&by, (grant, device, command.id), &ticket, prompt)
            }
            api::Request::Rename { ticket, title } => self.control_rename(&by, &ticket, &title),
            api::Request::Move { ticket, column, before } => {
                self.control_move(&by, &ticket, &column, before.as_deref())
            }
            api::Request::Tag { ticket, group, name } => {
                self.control_tag(&by, &ticket, group, name)
            }
            api::Request::Notes { ticket } => self.control_notes(&by, &ticket),
            api::Request::Note { ticket, note } => self.control_note(&by, &ticket, &note),
            api::Request::WriteNote { ticket, note, text, rev } => {
                self.control_write_note(&by, &ticket, note.as_deref(), text, rev)
            }
            api::Request::TellAgent { ticket, note } => {
                self.control_tell_agent(&by, &ticket, &note)
            }
        };
        self.control.remember(grant, command.id, reply.clone());
        self.control.answer(peer, command.id, reply);
    }
    pub(super) fn control_observe_dialog(&mut self, id: uuid::Uuid, frame: &HookFrame) {
        if frame.payload.get("agent_id").is_some() {
            return;
        }
        if matches!(
            frame.event.as_str(),
            "UserPromptSubmit"
                | "SessionStart"
                | "SessionEnd"
                | "PaneDied"
                | "Stop"
                | "StopFailure"
                | "PostToolUse"
                | "PostToolUseFailure"
        ) {
            self.control.dialogs.remove(&id);
        }
        if !matches!(frame.event.as_str(), "PreToolUse" | "PermissionRequest") {
            return;
        }
        let Some(tool) = frame.payload["tool_name"].as_str() else { return };
        let input = &frame.payload["tool_input"];
        if serde_json::to_vec(input).map_or(true, |b| b.len() > 16 * 1024) {
            return;
        }
        let content = match tool {
            "AskUserQuestion" => {
                let Ok(questions) =
                    serde_json::from_value::<Vec<api::Question>>(input["questions"].clone())
                else {
                    return;
                };
                if questions.is_empty()
                    || questions.len() > 4
                    || questions.iter().any(|q| q.options.is_empty() || q.options.len() > 8)
                {
                    return;
                }
                api::DialogContent::Questions { questions }
            }
            "ExitPlanMode" => {
                let Some(markdown) = input["plan"].as_str().filter(|p| !p.trim().is_empty()) else {
                    return;
                };
                api::DialogContent::Plan { markdown: markdown.into() }
            }
            _ => return,
        };
        // PermissionRequest may repeat the same PreToolUse payload. Keep its
        // identity stable while a browser is selecting that exact dialog.
        if frame.event == "PermissionRequest"
            && frame.payload["tool_use_id"].is_null()
            && self.control.dialogs.get(&id).is_some_and(|d| d.content == content)
        {
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
        self.control.dialogs.insert(id, api::Dialog { request, content });
        self.control_changed();
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
        if self.control.dialog_deliveries.contains_key(&id)
            || self.control.pending.contains_key(&id)
        {
            return Reply::Rejected { message: "input is already pending".into() };
        }
        let Some(dialog) = self.control.dialogs.get(&id).filter(|d| d.request == request) else {
            return Reply::Rejected { message: "dialog changed; check the pane".into() };
        };
        if dialog_target(dialog, &response).is_none() {
            return Reply::Rejected {
                message: "this dialog shape is not verified; answer in the pane".into(),
            };
        }
        let Ok(ticket) = ulid::Ulid::from_string(ticket) else {
            return Reply::Rejected { message: "invalid ticket".into() };
        };
        self.control.dialog_deliveries.insert(
            id,
            DialogDelivery {
                grant,
                device,
                command,
                ticket,
                request: request.into(),
                response,
                deadline: Instant::now() + Duration::from_secs(8),
                next: Instant::now(),
                steps: 0,
                pasted: false,
            },
        );
        Reply::Delivery { status: "awaiting_delivery".into() }
    }

    fn control_deliver_dialogs(&mut self) {
        use mesimon_backend_tmux::DialogKey;
        let ready: Vec<_> = self
            .control
            .dialog_deliveries
            .iter()
            .filter(|(_, p)| Instant::now() >= p.next)
            .map(|(id, _)| *id)
            .collect();
        for id in ready {
            let Some(mut pending) = self.control.dialog_deliveries.remove(&id) else { continue };
            let by = Principal::Paired {
                device: pending.device.to_hex(),
                grant: pending.grant.to_hex(),
            };
            let allowed = self.control_granted(pending.grant, pending.device)
                && !authorize(&by, &Action::PromptExisting, &Resource::Session { id }).denied()
                && self.control_target(&pending.ticket.to_string(), &id.to_string()) == Some(id)
                && self.board.sessions.iter().any(|s| {
                    s.id == id
                        && matches!(
                            s.state,
                            SessionState::RequiresAction {
                                reason: mesimon_core::board::Reason::Question
                                    | mesimon_core::board::Reason::Plan
                            }
                        )
                });
            let dialog = self.control.dialogs.get(&id).filter(|d| d.request == pending.request);
            let screen = match self.pane_tail(id, 50) {
                Response::PaneTail { lines, .. } => lines.join("\n"),
                _ => String::new(),
            };
            let step = if allowed && Instant::now() < pending.deadline && pending.steps < 12 {
                dialog.and_then(|d| dialog_step(d, &pending.response, &screen, pending.pasted))
            } else {
                None
            };
            let sid = id.simple().to_string()[..16].to_string();
            let outcome = match step {
                Some(DialogStep::Up) => self.backend.dialog_key(&sid, DialogKey::Up).map(|_| false),
                Some(DialogStep::Down) => {
                    self.backend.dialog_key(&sid, DialogKey::Down).map(|_| false)
                }
                Some(DialogStep::Reject) => {
                    self.backend.dialog_key(&sid, DialogKey::Escape).map(|_| true)
                }
                Some(DialogStep::Submit) => self.backend.send_enter(&sid).map(|_| true),
                Some(DialogStep::Paste(text)) => self.backend.paste_input(&sid, &text).map(|_| {
                    pending.pasted = true;
                    false
                }),
                None => Err(anyhow::anyhow!("dialog outcome unknown")),
            };
            match outcome {
                Ok(false) => {
                    pending.steps += 1;
                    pending.next = Instant::now() + Duration::from_millis(350);
                    self.control.dialog_deliveries.insert(id, pending);
                }
                result => {
                    self.control.dialogs.remove(&id);
                    let reply = Reply::Delivery {
                        status: if result.is_ok() { "input_sent" } else { "unknown" }.into(),
                    };
                    self.control.remember(pending.grant, pending.command, reply);
                    self.feed.board(by.actor(), "mesophon_dialog_answer", Some(pending.ticket));
                    self.control_changed();
                }
            }
        }
    }

    pub(super) fn control_prompt_edge(&mut self, prompt: bool) {
        self.control.suppress_awareness = prompt;
    }
    fn control_awareness(&mut self) {
        let Some(stored) = &self.control.stored else { return };
        let board = stored.board.to_hex();
        let mut changes = Vec::new();
        for ticket in self.board.tickets.iter().filter(|t| !t.is_archived()) {
            let Some(session) = self
                .board
                .sessions
                .iter()
                .filter(|s| s.ticket == ticket.id && s.kind.is_agent())
                .min_by_key(|s| attention::rank(&s.state))
            else {
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
            || self.control.permissions.contains_key(&id)
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
        let peers: Vec<_> = self
            .control
            .peers
            .iter()
            .filter(|(_, p)| p.subscribed && self.control_granted(p.grant, p.device))
            .map(|(id, _)| id.clone())
            .collect();
        if peers.is_empty() {
            return;
        }
        // Closing this stream gives the deciding process empty stdout. Never
        // hold the writer for a phone: all waiting is state plus a deadline.
        let _ = stream.set_nonblocking(true);
        self.control.permissions.insert(
            id,
            PermissionWait {
                projection: api::Permission {
                    request: uuid::Uuid::new_v4().to_string(),
                    tool: tool.into(),
                    input: input.clone(),
                    expires_at: now_ms() + 40_000,
                },
                stream,
                deadline: Instant::now() + Duration::from_secs(40),
                peers,
            },
        );
        self.control_changed();
    }

    pub(super) fn control_cancel_permission(&mut self, id: uuid::Uuid) {
        if self.control.permissions.remove(&id).is_some() {
            self.control_changed();
        }
    }

    fn control_expire_permissions(&mut self) {
        let expired: Vec<_> = self
            .control
            .permissions
            .iter()
            .filter(|(id, p)| {
                Instant::now() >= p.deadline
                    || permission_peer_closed(&p.stream)
                    || !p.peers.iter().any(|peer| {
                        self.control
                            .peers
                            .get(peer)
                            .is_some_and(|peer| self.control_granted(peer.grant, peer.device))
                    })
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

    fn control_permission_answer(
        &mut self,
        by: &Principal,
        peer: &str,
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
        if !self
            .control
            .permissions
            .get(&id)
            .is_some_and(|p| p.projection.request == request && p.peers.iter().any(|p| p == peer))
        {
            return Reply::Rejected {
                message: "permission expired or was already answered; check the pane".into(),
            };
        }
        let Some(mut pending) = self.control.permissions.remove(&id) else {
            return Reply::Rejected { message: "permission is no longer pending".into() };
        };
        // No second browser can answer after this point. Successful transport
        // is not proof Claude executed the tool; report only decision delivery.
        let sent = serde_json::to_vec(&decision)
            .ok()
            .is_some_and(|bytes| pending.stream.write_all(&bytes).is_ok());
        self.feed.board(
            by.actor(),
            "mesophon_permission_answer",
            ulid::Ulid::from_string(ticket).ok(),
        );
        self.control_changed();
        Reply::Delivery { status: if sent { "decision_sent" } else { "unknown" }.into() }
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
        envelope: Option<ObjectId>,
    ) -> Reply {
        let reject = |message: String| Reply::Rejected { message };
        let Some(column) = column.or_else(|| self.board.landing_column()) else {
            return reject("the board has no columns".into());
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
        let mint = Mint {
            column,
            title,
            workspace: None,
            from: None,
            tags,
            note: Some((note, Vec::new())),
            tier: None,
            envelope: envelope.map(ObjectId::to_hex),
        };
        match self.mint_full(by, mint) {
            Ok(id) => {
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
        if self.worktrees_barred && self.ticket_wants_worktree(id) {
            return reject(&self.barred_message("worktrees"));
        }
        // The words cross the same boundary a desk prompt does: scrubbed,
        // capped and blank-is-none (`sanitize_prompt`).
        let words = prompt.as_deref().and_then(mesimon_core::command::sanitize_prompt);
        let asleep = self
            .board
            .live_agent(id)
            .filter(|s| matches!(s.state, SessionState::Sleeping))
            .map(|s| s.id);
        let answer = match (asleep, words) {
            (Some(_), Some(text)) => self.prompt_sleeping(id, text, false),
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
        Reply::Delivery { status: status.into() }
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
            let delivery = |status: &str| Reply::Delivery { status: status.into() };
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

    /// A new title from the owner's phone (T-530): the desk's rename,
    /// scrubbed and capped the same way, and refused blank. The same title
    /// again writes nothing.
    fn control_rename(&mut self, by: &Principal, ticket: &str, title: &str) -> Reply {
        let reject = |message: &str| Reply::Rejected { message: message.into() };
        let Some(id) = self.control_ticket(ticket) else {
            return reject("ticket unavailable");
        };
        if let Decision::Deny { reason } =
            authorize(by, &Action::RenameTicket, &Resource::Ticket { id })
        {
            return reject(&format!("denied: {reason}"));
        }
        if let Some(message) = self.team_viewer_refusal() {
            return reject(&message);
        }
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
        let Some(id) = self.control_ticket(ticket) else {
            return reject("ticket unavailable");
        };
        if let Decision::Deny { reason } =
            authorize(by, &Action::TagTicket, &Resource::Ticket { id })
        {
            return reject(&format!("denied: {reason}"));
        }
        if let Some(message) = self.team_viewer_refusal() {
            return reject(&message);
        }
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
        let reply =
            self.control_create(&by, ticket.title, ticket.description, column, &tags, Some(id));
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
        if let Some(f) =
            self.control.stored.as_ref().and_then(|s| s.noted.iter().find(|f| f.envelope == id))
        {
            return serde_json::from_value(f.answer.clone())
                .unwrap_or(Reply::Delivery { status: "unknown".into() });
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
        if let Some(mut s) = self.control.stored.clone() {
            let kept = serde_json::to_value(&answer).unwrap_or_default();
            s.noted.push(NoteFiled { envelope: id, answer: kept });
            let excess = s.noted.len().saturating_sub(FILED_KEEP);
            s.noted.drain(..excess);
            // A failed save costs the replay guard only: an edit is answered
            // again by its words, a delete by the note being gone.
            let _ = self.control_save(&s);
            self.control.stored = Some(s);
        }
        answer
    }

    /// A ticket a phone reads or writes notes on: on the board, not archived.
    fn control_note_ticket(&self, ticket: &str) -> Option<ulid::Ulid> {
        ulid::Ulid::from_string(ticket)
            .ok()
            .filter(|id| self.board.ticket(*id).is_some_and(|t| !t.is_archived()))
    }

    /// One note's metadata and body as they stand, or `None`.
    fn control_note_body(
        &self,
        ticket: &str,
        note: Option<&str>,
    ) -> Option<(ulid::Ulid, mesimon_core::board::NoteMeta, String)> {
        let id = self.control_note_ticket(ticket)?;
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
    fn control_author(&self, meta: &mesimon_core::board::NoteMeta) -> String {
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

    fn control_notes(&self, by: &Principal, ticket: &str) -> Reply {
        let Some(id) = self.control_note_ticket(ticket) else {
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
        if serde_json::to_vec(&reply).is_ok_and(|v| v.len() > BOARD_WORDS_BUDGET) {
            if let Reply::Notes { ticket, notes, .. } = reply {
                return Reply::Notes { ticket, notes, description: None };
            }
        }
        reply
    }

    fn control_note(&self, by: &Principal, ticket: &str, note: &str) -> Reply {
        let Some(id) = self.control_note_ticket(ticket) else {
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
        let Some(id) = self.control_note_ticket(ticket) else {
            return reject("ticket unavailable");
        };
        if let Decision::Deny { reason } =
            authorize(by, &Action::Annotate, &Resource::Ticket { id })
        {
            return reject(&format!("denied: {reason}"));
        }
        if let Some(message) = self.team_viewer_refusal() {
            return reject(&message);
        }
        let Some(t) = self.board.ticket(id) else { return reject("ticket unavailable") };
        let note = match note_gate(t, note, text.trim().is_empty(), rev) {
            Ok(note) => note,
            Err(gate) => return gate.reply(ticket, |meta| self.control_author(meta)),
        };
        match self.write_note(id, note, text, by) {
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

    /// The desk's second `^s` (T-532): mesimon's own sentence, naming the
    /// note, pasted to the ticket's awake agent. A prompt into a session the
    /// phone may already send, so it is authorized as one.
    fn control_tell_agent(&mut self, by: &Principal, ticket: &str, note: &str) -> Reply {
        let reject = |message: &str| Reply::Rejected { message: message.into() };
        let Some(id) = self.control_note_ticket(ticket) else {
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
                Reply::Delivery { status: "submitted".into() }
            }
            Response::Err { message } => reject(&message),
            other => reject(&format!("unexpected answer: {other:?}")),
        }
    }

    fn control_board(&self) -> Reply {
        let columns = self.board.sorted_columns();
        let mut allowed_tags: Vec<_> = self
            .board
            .tags
            .iter()
            .map(|t| api::TagOption { group: t.group, name: t.name.clone(), tint: t.tint() })
            .collect();
        allowed_tags.sort_by_key(|t| t.group);
        let mut reply = Reply::Board {
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
                .map(|t| api::Ticket {
                    id: t.id.to_string(),
                    queued: self.queued.iter().find(|q| q.ticket == t.id).map(|q| q.text.clone()),
                    key: t.short_key.clone(),
                    title: t.title.clone(),
                    column: t.column.clone(),
                    tags: projected_tags(&self.board, t),
                    picked: projected_pickup(t),
                    notes: u32::try_from(t.notes.len()).unwrap_or(u32::MAX),
                    noted: api::notes_stamp(&t.notes),
                    agent: self.board.live_agent(t.id).map(|s| {
                        let (doing, said) = self.control_words(s);
                        self.control_agent(t, s, doing, said)
                    }),
                })
                .collect(),
        };
        // A transcript no live agent reads any more leaves the cache.
        let live: HashSet<&str> = self.board.sessions.iter().filter_map(preview_path).collect();
        self.control.words.borrow_mut().retain(|path, _| live.contains(path.as_str()));
        // The board must fit one answer (`Control::answer`'s cap): the
        // agents' words are what a crowded board goes without first.
        if serde_json::to_vec(&reply).is_ok_and(|v| v.len() > BOARD_WORDS_BUDGET) {
            if let Reply::Board { tickets, .. } = &mut reply {
                for agent in tickets.iter_mut().filter_map(|t| t.agent.as_mut()) {
                    (agent.doing, agent.said) = (None, None);
                }
            }
        }
        reply
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
            since: s.state_changed_at,
            doing,
            said,
            dialog: self
                .control
                .dialogs
                .get(&s.id)
                .filter(|_| {
                    matches!(
                        s.state,
                        SessionState::RequiresAction {
                            reason: mesimon_core::board::Reason::Question
                                | mesimon_core::board::Reason::Plan
                        }
                    )
                })
                .cloned(),
            permission: self
                .control
                .permissions
                .get(&s.id)
                .filter(|p| Instant::now() < p.deadline)
                .map(|p| p.projection.clone()),
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
            let preview = crate::agents::read_preview(s.kind, std::path::Path::new(path));
            let doing = preview.as_ref().and_then(|p| match p.activity.as_ref()? {
                crate::agents::AgentActivity::Tool(step) => api::step_line(step),
                crate::agents::AgentActivity::Thinking => Some("thinking".into()),
            });
            let said = preview.and_then(|p| api::reply_line(p.text.as_deref()?));
            words.insert(path.to_string(), Words { len: meta.len(), mtime_ms, doing, said });
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
        (rec.id == id
            && rec.kind.is_agent()
            && rec.state.has_pane()
            && !(rec.provenance == Provenance::Adopted && rec.argv.is_empty()))
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
    #[allow(clippy::too_many_arguments)]
    fn control_prompt(
        &mut self,
        by: &Principal,
        grant: BoardId,
        device: DeviceId,
        command: u64,
        target: (&str, &str),
        text: String,
        queued: bool,
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
        if text.len() > mesimon_core::command::PROMPT_MAX_BYTES {
            return Reply::Rejected { message: "prompt is too long".into() };
        }
        let Some(text) = mesimon_core::command::sanitize_prompt(&text) else {
            return Reply::Rejected { message: "nothing to send".into() };
        };
        let Ok(ticket) = ulid::Ulid::from_string(ticket) else {
            return Reply::Rejected { message: "invalid ticket".into() };
        };
        if self.parked(id) || self.board.sessions.iter().any(|s| s.id == id && s.pending_submit) {
            return Reply::Rejected {
                message: "a prompt is already waiting for this session".into(),
            };
        }
        self.forget_queued(ticket, "queued_ask_replaced", "mesophon");
        self.control.pending.insert(
            id,
            Pending { grant, device, command, ticket, pasted: false, send_now_receipt: None },
        );
        if queued {
            if let Err(message) =
                self.park_ask(ticket, QueuedSeat::Pane(id), text, None, false, false)
            {
                self.control_cancel(id);
                return Reply::Rejected { message };
            }
            self.drain_queue();
            self.broadcast();
            return if self.queued.iter().any(|q| q.ticket == ticket) {
                Reply::Delivery { status: "queued".into() }
            } else if self.parked(id) {
                Reply::Delivery { status: "awaiting_delivery".into() }
            } else {
                self.control
                    .receipts
                    .get(&grant)
                    .and_then(|r| r.get(&command))
                    .cloned()
                    .unwrap_or(Reply::Delivery { status: "unknown".into() })
            };
        }
        match self.paste_to_ticket(ticket, &text, Ack::PROMPT) {
            Ok(()) => {
                let waiting = self.parked(id);
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
                Reply::Delivery {
                    status: if waiting { "awaiting_delivery" } else { "submitted" }.into(),
                }
            }
            Err(_) => {
                self.control_cancel(id);
                Reply::Delivery { status: "unknown".into() }
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
                _ => Reply::Delivery {
                    status: if self.parked(id) { "awaiting_delivery" } else { "submitted" }.into(),
                },
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
            if !self.control_granted(p.grant, p.device)
                || p.send_now_receipt
                    .is_some_and(|(grant, device, _)| !self.control_granted(grant, device))
                || self.control_target(&p.ticket.to_string(), &id.to_string()) != Some(id)
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
                self.control.remember(
                    grant,
                    command,
                    Reply::Delivery { status: "submitted".into() },
                );
            }
            self.control.remember(
                p.grant,
                p.command,
                Reply::Delivery { status: "submitted".into() },
            );
        }
    }
    pub(super) fn control_cancel(&mut self, id: uuid::Uuid) {
        if let Some(p) = self.control.pending.remove(&id) {
            self.queued.retain(|q| !matches!(q.seat, QueuedSeat::Pane(s) if s == id));
            self.codex_input_due.remove(&id);
            self.drop_owed(id);
            if let Some((grant, _, command)) = p.send_now_receipt {
                self.control.remember(
                    grant,
                    command,
                    Reply::Delivery {
                        status: if p.pasted { "unknown" } else { "rejected" }.into(),
                    },
                );
            }
            self.control.remember(
                p.grant,
                p.command,
                Reply::Delivery { status: if p.pasted { "unknown" } else { "rejected" }.into() },
            );
        }
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

fn projected_tags(board: &Board, t: &Ticket) -> Vec<api::TagOption> {
    t.tags
        .iter()
        .map(|g| api::TagOption { group: g.group, name: g.name.clone(), tint: board.tint_of(g) })
        .collect()
}

/// A phone's ticket once it was picked up (T-497), on the browser's clock
/// unit; nothing for a ticket no phone filed.
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

#[derive(Debug, PartialEq, Eq)]
enum DialogStep {
    Up,
    Down,
    Submit,
    Reject,
    Paste(String),
}

fn dialog_target(dialog: &api::Dialog, response: &api::DialogAnswer) -> Option<String> {
    match (&dialog.content, response) {
        (api::DialogContent::Questions { questions }, answer)
            if questions.len() == 1 && !questions[0].multi_select =>
        {
            match answer {
                api::DialogAnswer::Choice { index } => {
                    questions[0].options.get(*index).map(|o| o.label.clone())
                }
                api::DialogAnswer::Text { text }
                    if !text.contains(['\n', '\r'])
                        && text.len() <= 1000
                        && mesimon_core::command::sanitize_prompt(text).as_deref()
                            == Some(text.as_str()) =>
                {
                    Some("Type something.".into())
                }
                api::DialogAnswer::Reject => Some(String::new()),
                _ => None,
            }
        }
        (api::DialogContent::Plan { .. }, api::DialogAnswer::Accept) => {
            Some("Yes, manually approve edits".into())
        }
        (api::DialogContent::Plan { .. }, api::DialogAnswer::Reject) => Some(String::new()),
        _ => None,
    }
}

/// Recognize only measured native menus. Missing/wrapped/ambiguous selection,
/// multi-question and MCP forms yield no action. This does not infer success.
fn dialog_step(
    dialog: &api::Dialog,
    response: &api::DialogAnswer,
    screen: &str,
    pasted: bool,
) -> Option<DialogStep> {
    let target = dialog_target(dialog, response)?;
    let normalized = screen.split_whitespace().collect::<Vec<_>>().join(" ");
    match &dialog.content {
        api::DialogContent::Questions { questions } => {
            let question = questions[0].question.split_whitespace().collect::<Vec<_>>().join(" ");
            if question.is_empty()
                || !normalized.contains(&question)
                || !normalized.contains("Enter to select")
                || !normalized.contains("Esc to cancel")
            {
                return None;
            }
        }
        api::DialogContent::Plan { .. } => {
            if !normalized.contains("Would you like to proceed?")
                || !normalized.contains("Yes, manually approve edits")
                || !normalized.contains("Tell Claude what to change")
            {
                return None;
            }
        }
    }
    let mut selected = None;
    let mut options = Vec::new();
    for line in screen.lines() {
        let line = line.trim();
        let (active, line) = line.strip_prefix('❯').map_or((false, line), |s| (true, s.trim()));
        let Some((number, label)) = line.split_once(". ") else { continue };
        let Ok(number) = number.parse::<usize>() else { continue };
        if active {
            if selected.is_some() {
                return None;
            }
            selected = Some((number, label.trim()));
        }
        options.push((number, label.trim()));
    }
    let (current, label) = selected?;
    if matches!(response, api::DialogAnswer::Reject) {
        return Some(DialogStep::Reject);
    }
    if let api::DialogAnswer::Text { text } = response {
        if pasted {
            return (label == text).then_some(DialogStep::Submit);
        }
    }
    let targets: Vec<_> = options.iter().filter(|(_, label)| *label == target).collect();
    if targets.len() != 1 {
        return None;
    }
    let (number, _) = targets[0];
    if current < *number {
        return Some(DialogStep::Down);
    }
    if current > *number {
        return Some(DialogStep::Up);
    }
    match response {
        api::DialogAnswer::Text { text } => Some(DialogStep::Paste(text.clone())),
        _ => Some(DialogStep::Submit),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn dialog_keys_require_the_measured_shape_and_selected_label() {
        let dialog = question();
        let screen = "Which color?\n❯ 1. Blue\n  2. Green\n  3. Type something.\nEnter to select · Esc to cancel";
        assert_eq!(
            dialog_step(&dialog, &api::DialogAnswer::Choice { index: 1 }, screen, false),
            Some(DialogStep::Down)
        );
        let selected = screen.replace("❯ 1.", "  1.").replace("  2.", "❯ 2.");
        assert_eq!(
            dialog_step(&dialog, &api::DialogAnswer::Choice { index: 1 }, &selected, false),
            Some(DialogStep::Submit)
        );
        assert_eq!(
            dialog_step(&dialog, &api::DialogAnswer::Choice { index: 8 }, screen, false),
            None
        );
        assert_eq!(dialog_step(&dialog, &api::DialogAnswer::Accept, screen, false), None);
        for bad in [
            "ordinary composer",
            "Which color?\n❯ 1. Blue\nEnter to select",
            "Which color?\n❯ 1. Blue\n❯ 2. Green\nEnter to select · Esc to cancel",
        ] {
            assert_eq!(
                dialog_step(&dialog, &api::DialogAnswer::Choice { index: 0 }, bad, false),
                None
            );
        }
        let mut multiple = dialog.clone();
        if let api::DialogContent::Questions { questions } = &mut multiple.content {
            questions[0].multi_select = true;
        }
        assert_eq!(
            dialog_step(&multiple, &api::DialogAnswer::Choice { index: 0 }, screen, false),
            None
        );
    }

    #[test]
    fn question_text_is_pasted_into_the_selected_row_before_a_separate_enter() {
        let dialog = question();
        let answer = api::DialogAnswer::Text { text: "Purple".into() };
        let screen = "Which color?\n  1. Blue\n  2. Green\n❯ 3. Type something.\nEnter to select · Esc to cancel";
        assert_eq!(
            dialog_step(&dialog, &answer, screen, false),
            Some(DialogStep::Paste("Purple".into()))
        );
        assert_eq!(
            dialog_step(&dialog, &answer, screen, true),
            None,
            "never Enter on an empty text row"
        );
        assert_eq!(
            dialog_step(&dialog, &answer, &screen.replace("Type something.", "Purple"), true),
            Some(DialogStep::Submit)
        );
        assert_eq!(
            dialog_step(&dialog, &api::DialogAnswer::Text { text: "a\nb".into() }, screen, false),
            None
        );
    }

    #[test]
    fn plan_accept_never_selects_auto_accept_and_reject_requires_the_dialog() {
        let dialog = api::Dialog {
            request: "p".into(),
            content: api::DialogContent::Plan { markdown: "plan".into() },
        };
        let screen = "Would you like to\n proceed?\n❯ 1. Yes, auto-accept edits\n  2. Yes, manually approve edits\n  3. Tell Claude what to change";
        assert_eq!(
            dialog_step(&dialog, &api::DialogAnswer::Accept, screen, false),
            Some(DialogStep::Down)
        );
        assert_eq!(
            dialog_step(&dialog, &api::DialogAnswer::Reject, screen, false),
            Some(DialogStep::Reject)
        );
        assert_eq!(dialog_step(&dialog, &api::DialogAnswer::Reject, "❯ composer", false), None);
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

    fn agent(ticket: ulid::Ulid, kind: SessionKind, state: SessionState) -> SessionRecord {
        SessionRecord::new(uuid::Uuid::new_v4(), kind, ticket, Vec::new(), "/".into(), state)
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
        control.remember(grant, 5, Reply::Delivery { status: "starting".into() });
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
}
