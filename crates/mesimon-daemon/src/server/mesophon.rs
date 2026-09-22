//! Owner control is authenticated and applied here, on the board's single writer.
use super::*;
use crate::team::control_io::{self, Event as NetEvent};
use mesimon_core::mesophon::{self as api, Answer, Info, LocalAction, Reply};
use mesimon_team::{
    control::{Channel, Wire},
    crypto::{BoardId, DeviceId, DevicePublic, ObjectId},
    hex,
    invite::InviteCode,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::mpsc::SyncSender};

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
}
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
    permissions: HashMap<uuid::Uuid, PermissionWait>,
    phases: HashMap<ulid::Ulid, api::Phase>,
    suppress_awareness: bool,
    dialogs: HashMap<uuid::Uuid, api::Dialog>,
    dialog_deliveries: HashMap<uuid::Uuid, DialogDelivery>,
    incarnation: ObjectId,
    origin: String,
    online: bool,
    error: Option<String>,
    retry: Instant,
    dirty: bool,
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
            permissions: HashMap::new(),
            phases: HashMap::new(),
            suppress_awareness: false,
            dialogs: HashMap::new(),
            dialog_deliveries: HashMap::new(),
            incarnation: ObjectId::random(),
            origin: String::new(),
            online: false,
            error: None,
            retry: Instant::now(),
            dirty: false,
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
        let receipts = self.receipts.entry(grant).or_default();
        receipts.insert(id, reply);
        while receipts.len() > 128 {
            receipts.pop_first();
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
                self.control.error = None;
                self.control.origin.clear();
                self.control.invite = None;
                self.control.jobs = None;
                self.control.online = false;
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
        if self.control.dirty && self.ticks % 4 == 0 {
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
            NetEvent::Online(_)
                | NetEvent::Offline
                | NetEvent::Frame(Wire::Peer { .. } | Wire::Gone { .. })
        );
        match event {
            NetEvent::Online(origin) => {
                self.control.origin = origin;
                self.control.online = true;
                self.control.error = None;
                self.control_publish();
            }
            NetEvent::Offline => {
                self.control.online = false;
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
                features: vec!["permission".into(), "dialog".into(), "awareness".into()],
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
                Response::PaneTail { lines } => lines.join("\n"),
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

    fn control_board(&self) -> Reply {
        Reply::Board {
            title: self
                .paths
                .repo_root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            columns: self.board.columns.iter().map(|c| c.name.clone()).collect(),
            tickets: self
                .board
                .tickets
                .iter()
                .filter(|t| !t.is_archived())
                .map(|t| api::Ticket {
                    id: t.id.to_string(),
                    queued: self.queued.iter().find(|q| q.ticket == t.id).map(|q| q.text.clone()),
                    key: t.short_key.clone(),
                    title: t.title.clone(),
                    column: t.column.clone(),
                    agent: self.board.live_agent(t.id).map(|s| api::Agent {
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
                        provider: if s.kind == SessionKind::Codex { "codex" } else { "claude" }
                            .into(),
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
                        promptable: self
                            .control_target(&t.id.to_string(), &s.id.to_string())
                            .is_some(),
                    }),
                })
                .collect(),
        }
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
            Response::PaneTail { lines } => Reply::Preview { lines },
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
