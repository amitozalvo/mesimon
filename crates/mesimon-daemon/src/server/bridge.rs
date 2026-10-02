//! The mod road in the daemon (T-574): which road a Claude launch takes, the
//! commands queued for each session's mod and the bridge that polls them,
//! and the shadow that holds the mod's frames against the hook set's.
//!
//! Delivery is a long poll on purpose. The writer thread writes nothing to a
//! bridge: a bridge whose mod stopped reading (Claude on Ctrl+Z) would block
//! a push, and with it every board. `ModNext` answers at once when frames
//! wait and otherwise parks its reply, the way `answer_agent` parks its own
//! (T-569); the connection's thread does the write. A frame stays queued
//! until the bridge's next poll acks it, so delivery is at least once and the
//! mod drops a repeat by id. The ledger is memory-only — a queued pane ask
//! is, on purpose, and a mod's `submit` is its twin — and a frame belongs to
//! the pane it was queued for: a wake drops what the old pane never took.
//!
//! The turn roads (T-575, T-576) ride it: a prompt for a session whose mod
//! speaks `submit` goes down as one (`Daemon::mod_submit`), never typed into
//! the pane, and a question's answer as an `answer`. Each poll says what the
//! session's mod speaks (`ModNext::speaks`), so a session still on an older
//! mod is never sent a kind it would drop, and the paste road stays its road.

use super::*;
use crate::modroad::{self, Probe, RoadVerdict};
use crate::shadow::{self, Shadow};
use mesimon_core::road::{
    ModCommand, ModFrame, Road, RoadPref, MOD_ANSWER, MOD_PONG, MOD_SUBMIT, PAIRED_EVENTS,
};
use std::collections::VecDeque;

/// How long a ping waits for its pong.
pub(super) const PING_TIMEOUT_MS: u64 = 5_000;
/// The most frames one session's mod may have waiting; past it the oldest
/// go, with a journal line.
pub(super) const OUTBOX_CAP: usize = 256;

pub(super) struct Queued {
    frame: ModFrame,
    /// The pane the frame was queued for (`SessionRecord::pane_key`).
    pane: Option<String>,
}

/// A bridge's poll, parked until a frame is queued for its session.
struct Waiter {
    reply: Sender<ClientReply>,
}

struct Ping {
    reply: Sender<ClientReply>,
    at_ms: u64,
}

/// A session's bridge as its last poll described it: the pane it polls
/// from and the kinds its mod reads. Kept while that pane is the record's.
struct Bridge {
    pane: Option<String>,
    speaks: Vec<String>,
}

/// What the request in hand asked the writer to park (`Daemon::mod_park`).
pub(super) enum Park {
    Next { session: uuid::Uuid },
    Ping { id: String },
}

#[derive(Default)]
pub(super) struct ModRoad {
    /// Why the mod could not be laid, when it could not.
    pub(super) lay_error: Option<String>,
    probe: Option<Probe>,
    probing: bool,
    /// The last verdict written for `doctor`, so it is rewritten on change.
    verdict: Option<RoadVerdict>,
    outbox: HashMap<uuid::Uuid, VecDeque<Queued>>,
    waiters: HashMap<uuid::Uuid, Waiter>,
    pings: HashMap<String, Ping>,
    bridges: HashMap<uuid::Uuid, Bridge>,
    pub(super) shadow: Shadow,
}

impl ModRoad {
    /// At startup: the probe cache. The mod itself is laid the first time a
    /// launch or a probe needs it, so a board on `hooks` writes nothing.
    pub(super) fn start(paths: &Paths) -> Self {
        ModRoad { probe: modroad::load_probe(paths), ..Default::default() }
    }
}

fn reply_now(reply: Sender<ClientReply>, response: Response) {
    let _ = reply.send(ClientReply { response, delivered: None });
}

impl Daemon {
    /// The road this launch takes, and the folder `--plugin-dir` names when
    /// it is the mod. Decided once per launch and stamped on the record in
    /// the same block (`spawn_session`, `resume_session`). Claude only, and
    /// nothing is laid or probed for a launch the seam sends on `hooks`.
    pub(super) fn launch_road(&mut self, kind: SessionKind) -> (Road, Option<std::path::PathBuf>) {
        if kind != SessionKind::Claude {
            return (Road::Hooks, None);
        }
        let setting = modroad::read_setting();
        let (mut road, probe_line) = match setting.pref {
            RoadPref::Hooks => (Road::Hooks, None),
            RoadPref::Mod => (Road::Mod, None),
            RoadPref::Auto => self.auto_road(),
        };
        let mut folder = None;
        if road == Road::Mod {
            folder = self.lay_mod();
            if folder.is_none() {
                road = Road::Hooks;
            }
        }
        self.note_road(RoadVerdict {
            road,
            setting: setting.pref.word().into(),
            source: setting.source,
            probe: probe_line,
            lay_error: self.modroad.lay_error.clone(),
            fallback: false,
        });
        (road, folder)
    }

    /// Lay the mod (only what is missing or differs, so a folder something
    /// rewrote is put back before a session loads it). `None`, journalled,
    /// when it cannot be: the launch takes `hooks`.
    fn lay_mod(&mut self) -> Option<std::path::PathBuf> {
        match modroad::lay(&self.paths) {
            Ok(folder) => {
                self.modroad.lay_error = None;
                Some(folder)
            }
            Err(e) => {
                let e = format!("{e:#}");
                if self.modroad.lay_error.as_deref() != Some(&e) {
                    self.journal.line(&format!("mod not laid, launching on hooks: {e}"));
                }
                self.modroad.lay_error = Some(e);
                None
            }
        }
    }

    /// `auto`: the mod once a probe of the Claude Code a launch would run
    /// passed for this binary and this mod; `hooks` until then, and a probe
    /// is started. A binary that changed since (Claude Code updates itself)
    /// is probed again before it is trusted.
    fn auto_road(&mut self) -> (Road, Option<String>) {
        let bin = modroad::claude_binary(self.shell_env.path.as_deref());
        let key = bin.as_deref().and_then(modroad::probe_key);
        let cached = self.modroad.probe.as_ref().filter(|p| Some(&p.key) == key.as_ref());
        if let Some(p) = cached {
            return (
                if p.verdict.passed() { Road::Mod } else { Road::Hooks },
                Some(p.verdict.line()),
            );
        }
        let line = match (bin, key) {
            (Some(bin), Some(key)) => {
                self.start_road_probe(bin, key);
                "probing the Claude Code on PATH"
            }
            _ => "no claude on the captured PATH",
        };
        (Road::Hooks, Some(line.into()))
    }

    /// Every Claude launch asks `auto` (T-588), so the probe is not left for
    /// the first launch to start, which would then take the hook set: once
    /// the shell environment is captured, the Claude Code on its `PATH` is
    /// probed unless this binary and this mod already were.
    pub(super) fn warm_road_probe(&mut self) {
        if modroad::read_setting().pref != RoadPref::Auto {
            return;
        }
        let bin = modroad::claude_binary(self.shell_env.path.as_deref());
        let key = bin.as_deref().and_then(modroad::probe_key);
        if let (Some(bin), Some(key)) = (bin, key) {
            if self.modroad.probe.as_ref().is_none_or(|p| p.key != key) {
                self.start_road_probe(bin, key);
            }
        }
    }

    fn start_road_probe(&mut self, bin: std::path::PathBuf, key: modroad::ProbeKey) {
        if self.modroad.probing {
            return;
        }
        // The launcher applies the captured environment; a first capture
        // still running would hand the probe the daemon's own.
        if self.shell_env_capturing && !self.paths.shell_env_file().exists() {
            return;
        }
        let Some(folder) = self.lay_mod() else { return };
        let bin = bin.display().to_string();
        let version = self.launch(&[bin.clone(), "--version".into()], &[]);
        let validate = self
            .launch(&[bin, "plugin".into(), "validate".into(), folder.display().to_string()], &[]);
        let cwd = self.paths.state_dir.clone();
        let tx = self.tx.clone();
        self.modroad.probing = true;
        std::thread::spawn(move || {
            let verdict = modroad::probe(&version, &validate, &cwd);
            let _ = tx.send(Msg::RoadProbed(Probe { key, verdict }));
        });
    }

    /// A probe came back: cached, said in the feed when it changed what
    /// `auto` takes, and loudly when a Claude Code that passed before fails.
    pub(super) fn on_road_probed(&mut self, probe: Probe) {
        self.modroad.probing = false;
        let had_passed = self.modroad.probe.as_ref().is_some_and(|p| p.verdict.passed());
        let passes = probe.verdict.passed();
        modroad::save_probe(&self.paths, &probe);
        let line = probe.verdict.line();
        if had_passed && !passes {
            self.feed.board_outcome("automation", "claude_road_fallback", None, &line);
            self.journal.line(&format!("claude road falls back to hooks: {line}"));
        } else if had_passed != passes || self.modroad.probe.is_none() {
            let word = if passes { "mod" } else { "hooks" };
            self.feed.board_outcome("automation", &format!("claude_road:{word}"), None, &line);
        }
        let setting = modroad::read_setting();
        self.modroad.probe = Some(probe);
        if setting.pref == RoadPref::Auto {
            self.note_road(RoadVerdict {
                road: if passes { Road::Mod } else { Road::Hooks },
                setting: setting.pref.word().into(),
                source: setting.source,
                probe: Some(line),
                lay_error: self.modroad.lay_error.clone(),
                fallback: had_passed && !passes,
            });
        }
    }

    /// Write `road.json` for `doctor` when what it would say changed; a
    /// fallback stays said until a probe passes again.
    fn note_road(&mut self, mut verdict: RoadVerdict) {
        // A board that never left `hooks` writes nothing for it.
        if verdict.setting == RoadPref::Hooks.word()
            && self.modroad.verdict.is_none()
            && modroad::read_verdict(&self.paths).is_none()
        {
            return;
        }
        if let Some(prev) = &self.modroad.verdict {
            if prev.fallback && verdict.road == Road::Hooks && verdict.setting == "auto" {
                verdict.fallback = true;
            }
            if *prev == verdict {
                return;
            }
        }
        modroad::write_verdict(&self.paths, &verdict);
        self.modroad.verdict = Some(verdict);
    }

    /// The mod variables a mod launch's pane carries, read by the mod with
    /// `$.env.get` (literal names, so they are fixed here and in
    /// `register.ts`).
    pub(super) fn mod_vars(&self, session: uuid::Uuid) -> Vec<(String, String)> {
        vec![
            ("CLAUDE_CODE_PLUGIN_DIR_WATCH".into(), "0".into()),
            ("MESIMON_MOD_BIN".into(), crate::hook_settings::mesimon_bin().display().to_string()),
            ("MESIMON_MOD_HOOK_SOCK".into(), self.paths.hook_sock().display().to_string()),
            ("MESIMON_MOD_ORCH_SOCK".into(), self.paths.orch_sock().display().to_string()),
            ("MESIMON_MOD_SESSION".into(), session.to_string()),
        ]
    }

    // ---- Down: the daemon's commands to one session's mod.

    /// Queue a command for a session's mod, and hand it to a bridge that is
    /// waiting. Returns the frame's id.
    pub(super) fn mod_enqueue(&mut self, session: uuid::Uuid, command: ModCommand) -> String {
        let id = ulid::Ulid::new().to_string();
        let pane =
            self.board.sessions.iter().find(|s| s.id == session).and_then(|s| s.pane_key.clone());
        let frame = ModFrame { id: id.clone(), command };
        let queue = self.modroad.outbox.entry(session).or_default();
        queue.push_back(Queued { frame, pane });
        if queue.len() > OUTBOX_CAP {
            queue.pop_front();
            self.journal.line(&format!("mod outbox full for {session}: the oldest frame dropped"));
        }
        if let Some(waiter) = self.modroad.waiters.remove(&session) {
            let frames = self.mod_frames(session);
            reply_now(waiter.reply, Response::ModFrames { frames });
        }
        id
    }

    fn mod_frames(&self, session: uuid::Uuid) -> Vec<serde_json::Value> {
        self.modroad
            .outbox
            .get(&session)
            .map(|q| q.iter().filter_map(|f| serde_json::to_value(&f.frame).ok()).collect())
            .unwrap_or_default()
    }

    /// `ModNext` from a session's bridge: drop what it acked, answer with
    /// what waits, or park. A refusal is final for that bridge (it exits and
    /// the mod does not respawn it).
    pub(super) fn mod_next(
        &mut self,
        session: uuid::Uuid,
        ack: Option<String>,
        pane: Option<String>,
        speaks: Vec<String>,
    ) -> Response {
        let Some(rec) = self.board.sessions.iter().find(|s| s.id == session) else {
            return Response::Err { message: "unknown session".into() };
        };
        if !rec.state.has_pane() {
            return Response::Err { message: "session has no pane".into() };
        }
        if let (Some(theirs), Some(ours)) = (&pane, &rec.pane_key) {
            if theirs != ours {
                return Response::Err { message: "not this session's pane".into() };
            }
        }
        let mut taken = Vec::new();
        if let (Some(ack), Some(queue)) = (ack, self.modroad.outbox.get_mut(&session)) {
            if let Some(at) = queue.iter().position(|q| q.frame.id == ack) {
                taken.extend(queue.drain(..=at).map(|q| q.frame.id));
            }
        }
        // A newer poll takes the seat: the older one is told, and its bridge
        // (a straggler, or this bridge's own dead connection) goes.
        if let Some(old) = self.modroad.waiters.remove(&session) {
            reply_now(old.reply, Response::Err { message: "superseded".into() });
        }
        // The first poll from this pane is the mod come up: what a launch
        // parked for it goes down now (T-575).
        let first = self.modroad.bridges.get(&session).is_none_or(|b| b.pane != pane);
        self.modroad.bridges.insert(session, Bridge { pane, speaks });
        for id in taken {
            self.mod_taken(session, &id);
        }
        if first {
            self.mod_bridge_up(session);
        }
        let frames = self.mod_frames(session);
        if frames.is_empty() {
            self.mod_park = Some(Park::Next { session });
            // Unread: the writer parks the reply instead of sending this.
            Response::Ok
        } else {
            Response::ModFrames { frames }
        }
    }

    /// Whether the session's mod is up: its bridge has polled from the
    /// record's own pane (T-575), whatever it speaks.
    pub(super) fn mod_bridged(&self, session: uuid::Uuid) -> bool {
        let Some(rec) = self.board.sessions.iter().find(|s| s.id == session) else {
            return false;
        };
        rec.road == Road::Mod
            && rec.state.has_pane()
            && self.modroad.bridges.get(&session).is_some_and(|b| {
                b.pane.is_none() || rec.pane_key.is_none() || b.pane == rec.pane_key
            })
    }

    /// Whether the session's mod is up and reads `kind` (T-575): its bridge
    /// has polled from the record's own pane and its mod declared the kind.
    /// Every turn road asks this before it leaves the paste road and the
    /// screen: a mod that never came up, or an older one, keeps them.
    pub(super) fn mod_speaks(&self, session: uuid::Uuid, kind: &str) -> bool {
        let Some(rec) = self.board.sessions.iter().find(|s| s.id == session) else {
            return false;
        };
        rec.road == Road::Mod
            && rec.state.has_pane()
            && self.modroad.bridges.get(&session).is_some_and(|b| {
                (b.pane.is_none() || rec.pane_key.is_none() || b.pane == rec.pane_key)
                    && b.speaks.iter().any(|k| k == kind)
            })
    }

    /// Take a frame back before its bridge printed it: a road that gave up
    /// on the mod must not have it delivered late as well. `false` when it
    /// was already gone.
    pub(super) fn mod_unqueue(&mut self, session: uuid::Uuid, id: &str) -> bool {
        let Some(queue) = self.modroad.outbox.get_mut(&session) else { return false };
        let before = queue.len();
        queue.retain(|q| q.frame.id != id);
        before != queue.len()
    }

    /// `ModPing` from a person (or the harness): queue a ping and park the
    /// reply until the pong.
    pub(super) fn mod_ping(&mut self, session: uuid::Uuid) -> Response {
        let Some(rec) = self.board.sessions.iter().find(|s| s.id == session) else {
            return Response::Err { message: "unknown session".into() };
        };
        if rec.road != Road::Mod {
            return Response::Err {
                message: "that session was not launched on the mod road".into(),
            };
        }
        if !rec.state.has_pane() {
            return Response::Err { message: "that session has no pane".into() };
        }
        let id = self.mod_enqueue(session, ModCommand::Ping);
        self.mod_park = Some(Park::Ping { id });
        Response::Ok
    }

    /// The writer's half of a parked request: keep the reply, or hand it
    /// back to be sent now.
    pub(super) fn mod_park_reply(
        &mut self,
        park: Park,
        reply: Sender<ClientReply>,
    ) -> Option<Sender<ClientReply>> {
        match park {
            Park::Next { session } => {
                self.modroad.waiters.insert(session, Waiter { reply });
            }
            Park::Ping { id } => {
                self.modroad.pings.insert(id, Ping { reply, at_ms: now_ms() });
            }
        }
        None
    }

    // ---- Up: the mod's frames, held against the hook set's.

    /// A hook-set frame the writer is ingesting: offered to the shadow when
    /// its session was handed the mod.
    pub(super) fn shadow_hooks_frame(&mut self, frame: &HookFrame) {
        self.shadow_offer(frame, Road::Hooks);
    }

    /// A frame the mod relayed (`road: mod`): never ingested. A pong answers
    /// its ping; anything else is paired.
    pub(super) fn on_shadow_hook(&mut self, frame: HookFrame) {
        if frame.event == MOD_PONG {
            if let Some(ping) = frame.reason.as_deref().and_then(|id| self.modroad.pings.remove(id))
            {
                let ms = now_ms().saturating_sub(ping.at_ms);
                reply_now(ping.reply, Response::ModPonged { ms });
            }
            return;
        }
        // The mod's own reports (T-575, T-576): the only source of what they
        // say, so they are read here and paired with nothing.
        if frame.event == MOD_SUBMIT || frame.event == MOD_ANSWER {
            let Ok(session) = frame.session.parse::<uuid::Uuid>() else { return };
            if !self.board.sessions.iter().any(|s| s.id == session && s.road == Road::Mod) {
                return;
            }
            self.feed.hook_event(&frame.session, &frame.event, frame.reason.as_deref());
            if frame.event == MOD_SUBMIT {
                self.on_mod_submit(session, &frame);
            } else {
                self.on_mod_answer(session, frame);
            }
            return;
        }
        self.shadow_offer(&frame, Road::Mod);
    }

    fn shadow_offer(&mut self, frame: &HookFrame, road: Road) {
        if !PAIRED_EVENTS.contains(&frame.event.as_str()) {
            return;
        }
        let Ok(session) = frame.session.parse::<uuid::Uuid>() else { return };
        if !self.board.sessions.iter().any(|s| s.id == session && s.road == Road::Mod) {
            return;
        }
        let key = shadow::Key { session, event: frame.event.clone(), reason: frame.reason.clone() };
        let at = if frame.accepted_ms == 0 { now_ms() } else { frame.accepted_ms };
        self.modroad.shadow.offer(key, road, shadow::digest(&frame.event, &frame.payload), at);
    }

    /// The tick's share (`sent_ms`, when the tick was sent): the shadow's
    /// lines, the pings that waited out their time, and the frames whose
    /// pane is gone.
    pub(super) fn tick_mod_road(&mut self, sent_ms: u64) {
        for d in self.modroad.shadow.sweep(sent_ms) {
            let ticket = self.board.sessions.iter().find(|s| s.id == d.session).map(|s| s.ticket);
            self.feed.road_disagree(d.session, ticket, &d.event, d.outcome.word(), d.count);
        }
        let now = now_ms();
        let late: Vec<String> = self
            .modroad
            .pings
            .iter()
            .filter(|(_, p)| now.saturating_sub(p.at_ms) >= PING_TIMEOUT_MS)
            .map(|(id, _)| id.clone())
            .collect();
        for id in late {
            if let Some(p) = self.modroad.pings.remove(&id) {
                reply_now(
                    p.reply,
                    Response::Err {
                        message: format!(
                            "no pong in {} s: the session's bridge is not polling",
                            PING_TIMEOUT_MS / 1000
                        ),
                    },
                );
            }
        }
        // A frame belongs to the pane it was queued for; a parked poll and a
        // bridge to a session that still has one.
        let sessions: Vec<uuid::Uuid> = self
            .modroad
            .outbox
            .keys()
            .chain(self.modroad.waiters.keys())
            .chain(self.modroad.bridges.keys())
            .copied()
            .collect();
        for session in sessions {
            let rec = self.board.sessions.iter().find(|s| s.id == session);
            let pane = rec.filter(|r| r.state.has_pane()).map(|r| r.pane_key.clone());
            match pane {
                None => {
                    self.mod_forget(session);
                }
                Some(pane) => {
                    if self
                        .modroad
                        .bridges
                        .get(&session)
                        .is_some_and(|b| b.pane.is_some() && pane.is_some() && b.pane != pane)
                    {
                        self.modroad.bridges.remove(&session);
                    }
                    if let Some(queue) = self.modroad.outbox.get_mut(&session) {
                        let before = queue.len();
                        queue.retain(|q| q.pane.is_none() || pane.is_none() || q.pane == pane);
                        let dropped = before - queue.len();
                        if queue.is_empty() {
                            self.modroad.outbox.remove(&session);
                        }
                        if dropped > 0 {
                            self.journal.line(&format!(
                                "mod frames for {session}'s old pane dropped: {dropped}"
                            ));
                        }
                    }
                }
            }
        }
    }

    /// A session lost its pane or its record: its frames, its parked poll
    /// and its shadow go.
    pub(super) fn mod_forget(&mut self, session: uuid::Uuid) {
        if let Some(q) = self.modroad.outbox.remove(&session) {
            if !q.is_empty() {
                self.journal
                    .line(&format!("mod frames for {session} dropped with its pane: {}", q.len()));
            }
        }
        if let Some(w) = self.modroad.waiters.remove(&session) {
            reply_now(w.reply, Response::Err { message: "session has no pane".into() });
        }
        self.modroad.bridges.remove(&session);
        self.modroad.shadow.forget(session);
    }
}
