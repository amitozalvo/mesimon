//! The mod road in the daemon (T-574): which road a Claude launch takes, the
//! commands queued for each session's mod and the bridge that polls them,
//! and the mod's frames, which since T-577 are the session's only frames
//! (no hook set rides a mod launch; T-574's shadow ended with it).
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
//!
//! `auto` takes the mod only where the mod is proven to load (T-598): a
//! launch whose mod never reports is relaunched on the hook set
//! (`relaunch_silent_mods`), and the probe learns that this Claude Code has
//! mods off, so the launches after it take the hook set at once.

use super::*;
use crate::modroad::{self, Probe, RoadVerdict, Verdict};
use mesimon_core::road::{
    ModCommand, ModFrame, Road, RoadPref, MOD_ANSWER, MOD_FILL, MOD_LOAD_FAILED, MOD_PONG,
    MOD_SUBMIT, MOD_USAGE,
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
    /// The records relaunched on the hook set this daemon life (T-598):
    /// each once, and each stays on the hook set until the daemon restarts.
    relaunched: std::collections::HashSet<uuid::Uuid>,
    /// A mods-off verdict this daemon read at start: asked again once, before
    /// its six hours are up (T-598).
    recheck: bool,
}

impl ModRoad {
    /// At startup: the probe cache. The mod itself is laid the first time a
    /// launch or a probe needs it, so a board on `hooks` writes nothing.
    pub(super) fn start(paths: &Paths) -> Self {
        let probe = modroad::load_probe(paths);
        let recheck = probe.as_ref().is_some_and(Probe::mods_off);
        ModRoad { probe, recheck, ..Default::default() }
    }
}

fn reply_now(reply: Sender<ClientReply>, response: Response) {
    let _ = reply.send(ClientReply { response, delivered: None });
}

impl Daemon {
    /// The road this launch takes, and the folder `--plugin-dir` names when
    /// it is the mod. Decided once per launch and stamped on the record in
    /// the same block (`spawn_session`, `resume_session`). Claude only, and
    /// nothing is laid or probed for a launch the seam sends on `hooks`. A
    /// record relaunched on the hook set this daemon life stays there
    /// (T-598), whatever a probe has said since.
    pub(super) fn launch_road(
        &mut self,
        kind: SessionKind,
        session: uuid::Uuid,
    ) -> (Road, Option<std::path::PathBuf>) {
        if kind != SessionKind::Claude {
            return (Road::Hooks, None);
        }
        let setting = modroad::read_setting();
        let (mut road, probe_line) = match setting.pref {
            RoadPref::Hooks => (Road::Hooks, None),
            RoadPref::Mod => (Road::Mod, None),
            RoadPref::Auto => self.auto_road(),
        };
        if road == Road::Mod
            && setting.pref == RoadPref::Auto
            && self.modroad.relaunched.contains(&session)
        {
            road = Road::Hooks;
            self.journal.line(&format!(
                "session {session} stays on the hook set: its mod never reported once this daemon life"
            ));
        }
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
            mods_off: self.modroad.probe.as_ref().is_some_and(Probe::mods_off)
                && road == Road::Hooks
                && setting.pref == RoadPref::Auto,
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
    /// is probed again before it is trusted, and a mods-off verdict is asked
    /// again when it is due (T-598) while this launch takes the hook set.
    fn auto_road(&mut self) -> (Road, Option<String>) {
        let bin = modroad::claude_binary(self.shell_env.path.as_deref());
        let key = bin.as_deref().and_then(modroad::probe_key);
        let cached = self.modroad.probe.as_ref().filter(|p| Some(&p.key) == key.as_ref());
        if let Some(p) = cached {
            let answer =
                (if p.verdict.passed() { Road::Mod } else { Road::Hooks }, Some(p.verdict.line()));
            if self.mods_off_due(p) {
                if let (Some(bin), Some(key)) = (bin, key) {
                    self.start_road_probe(bin, key);
                }
            }
            return answer;
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
            if self.modroad.probe.as_ref().is_none_or(|p| p.key != key || self.mods_off_due(p)) {
                self.start_road_probe(bin, key);
            }
        }
    }

    /// A mods-off verdict to ask again (T-598): one read at this daemon's
    /// start, or one older than `MODS_OFF_TTL_MS`.
    fn mods_off_due(&self, probe: &Probe) -> bool {
        probe.mods_off() && (self.modroad.recheck || probe.recheck_due(now_ms()))
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
        let load_probe = match modroad::lay_load_probe(&self.paths) {
            Ok(folder) => folder,
            Err(e) => {
                self.journal.line(&format!("the mod's load probe not laid: {e:#}"));
                return;
            }
        };
        let bin = bin.display().to_string();
        let version = self.launch(&[bin.clone(), "--version".into()], &[]);
        let validate = self.launch(
            &[bin.clone(), "plugin".into(), "validate".into(), folder.display().to_string()],
            &[],
        );
        let load = self
            .launch(&[bin, "plugin".into(), "test".into(), load_probe.display().to_string()], &[]);
        let cwd = self.paths.state_dir.clone();
        let tx = self.tx.clone();
        self.modroad.probing = true;
        self.modroad.recheck = false;
        std::thread::spawn(move || {
            let verdict = modroad::probe(&version, &validate, &load, &cwd, now_ms());
            let _ = tx.send(Msg::RoadProbed(Probe { key, verdict }));
        });
    }

    /// A probe came back.
    pub(super) fn on_road_probed(&mut self, probe: Probe) {
        self.modroad.probing = false;
        self.learn_road(probe);
    }

    /// A verdict, from a probe or a relaunch: cached, said in the feed when
    /// it changed what `auto` takes, and loudly when a Claude Code that
    /// passed before fails.
    fn learn_road(&mut self, probe: Probe) {
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
        let mods_off = probe.mods_off();
        self.modroad.probe = Some(probe);
        if setting.pref == RoadPref::Auto {
            self.note_road(RoadVerdict {
                road: if passes { Road::Mod } else { Road::Hooks },
                setting: setting.pref.word().into(),
                source: setting.source,
                probe: Some(line),
                lay_error: self.modroad.lay_error.clone(),
                fallback: had_passed && !passes,
                mods_off,
            });
        }
    }

    /// Mod launches that never reported (T-598), relaunched on the hook set.
    ///
    /// A launch on the mod alone carries no hook set, so a Claude Code that
    /// does not load the mod (2.1.288's remote flag turned mods off with
    /// `claude plugin validate` still passing) leaves a session that reports
    /// nothing: no attention, no automove, no cost. Under `auto` such a
    /// launch is judged silent while it is still `spawning` and its mod's
    /// bridge has not polled from its pane, once its pane shows Claude Code's
    /// composer a bridge wait or more after the launch: a mod that loads
    /// brings its bridge up at `session.start`, before the composer paints.
    /// Never without the composer: a pane held by a startup dialog (a folder
    /// to trust) has loaded nothing yet, and ending it would take the
    /// person's dialog and teach the probe a mods-off it never saw; such a
    /// launch is judged once the person answered. It is ended and relaunched through
    /// the wake road on the hook set (`--resume` where a conversation exists,
    /// a fresh start where none does), its launch words kept for the new
    /// pane's `SessionStart`, and the probe learns that this Claude Code has
    /// mods off. Once per record per daemon life.
    ///
    /// The order against `rescue_silent_mods` (T-577), which arms a silent
    /// mod launch's words for the paste road at twice the bridge wait: this
    /// is judged first in the tick, at the bridge wait, and a relaunched
    /// record is on the hook set, so the rescue never sees it. The rescue
    /// stays for the `mod` seam, for a launch whose bridge polled and whose
    /// `SessionStart` still never came, for a pane a dialog held past it
    /// (its words then wait on the composer, and a relaunch that follows
    /// takes them back), and for a relaunch that failed.
    pub(super) fn relaunch_silent_mods(&mut self, now: u64) -> bool {
        if modroad::read_setting().pref != RoadPref::Auto {
            return false;
        }
        let wait = mod_bridge_wait_ms();
        let due: Vec<(uuid::Uuid, u64)> = self
            .board
            .sessions
            .iter()
            .filter(|r| {
                r.kind == SessionKind::Claude
                    && r.frames_by_mod()
                    && r.state == SessionState::Spawning
                    && !self.modroad.relaunched.contains(&r.id)
            })
            .map(|r| (r.id, now.saturating_sub(r.state_changed_at.unwrap_or(now))))
            .filter(|(_, age)| *age >= wait)
            .collect();
        let mut changed = false;
        for (id, age) in due {
            if self.mod_bridged(id) {
                continue;
            }
            if !self.composer_shown(id) {
                continue;
            }
            changed |= self.relaunch_on_hooks(id, age, now);
        }
        changed
    }

    /// Whether a Claude pane shows its composer (`composer::read`).
    fn composer_shown(&self, id: uuid::Uuid) -> bool {
        use crate::agents::claude::composer::{self, Composer};
        let Some(rec) = self.board.sessions.iter().find(|s| s.id == id) else { return false };
        self.backend
            .capture_input_screen(&rec.sid16())
            .is_ok_and(|screen| composer::read(&screen) != Composer::Absent)
    }

    /// End a silent mod launch and launch it again on the hook set.
    fn relaunch_on_hooks(&mut self, id: uuid::Uuid, age: u64, now: u64) -> bool {
        let Some(rec) = self.board.sessions.iter().find(|s| s.id == id) else { return false };
        let ticket = rec.ticket;
        let prior = rec.state.clone();
        let pending_submit = rec.pending_submit;
        let conversed = rec.transcript_path.is_some() || rec.claude_session_id.is_some();
        let plan = rec.argv.windows(2).any(|w| w[0] == "--permission-mode" && w[1] == "plan");
        let reason = format!(
            "no SessionStart and no bridge from its mod {} s after the launch",
            age.div_ceil(1000)
        );
        self.modroad.relaunched.insert(id);
        self.journal.line(&format!("session {id}: {reason}; relaunched on the hook set"));
        // This Claude Code does not load the mod: every later `auto` launch
        // takes the hook set at once, until a probe says it loads again.
        if let Some(p) = self.modroad.probe.clone().filter(|p| p.verdict.passed()) {
            let version = p.verdict.version().unwrap_or_default().to_string();
            self.learn_road(Probe {
                key: p.key,
                verdict: Verdict::ModsOff { version, seen_at: now },
            });
        }
        // The words are the new pane's (a wake drops what the old one owed),
        // and its own `SessionStart` arms them, on the paste road.
        let owed = self.owed.remove(&id);
        self.mod_forget(id);
        if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
            rec.state = SessionState::Sleeping;
        }
        match self.resume_session_with_cleanup_ack(id, true, false, plan) {
            Response::Spawned { fresh, .. } => {
                match owed {
                    Some(mut owed) => {
                        // Words the rescue pasted into the old pane are the
                        // new pane's to receive.
                        if owed.parked.is_none() {
                            owed.parked = owed.sent.take();
                        }
                        owed.mod_road = false;
                        owed.next_press = None;
                        owed.ready_by = None;
                        owed.bridge_by = None;
                        owed.frame = None;
                        owed.taken = false;
                        owed.sent = None;
                        self.owed.insert(id, owed);
                        if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
                            rec.pending_submit = pending_submit;
                        }
                    }
                    // A composed spawn types nothing; a plain one typed the
                    // title for the person to edit, and a fresh pane gets it
                    // again (typed, never submitted).
                    None if fresh && !conversed => {
                        let title = self
                            .board
                            .ticket(ticket)
                            .map(|t| t.title.trim().to_string())
                            .filter(|t| !t.is_empty());
                        let sid16 =
                            self.board.sessions.iter().find(|s| s.id == id).map(|s| s.sid16());
                        if let (Some(title), Some(sid16)) = (title, sid16) {
                            let _ = self.backend.send_text(&sid16, &format!("{title} "));
                        }
                    }
                    None => {}
                }
                self.feed.board_outcome(
                    "automation",
                    "claude_road_relaunch",
                    Some(ticket),
                    &reason,
                );
            }
            other => {
                // Nothing was relaunched: the launch stands as it was, and
                // the paste rescue still nets its words.
                let why = match other {
                    Response::Err { message } => message,
                    _ => "its directory is being provisioned".into(),
                };
                self.pending_resumes.retain(|p| p.session != id);
                if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
                    rec.state = prior;
                }
                if let Some(owed) = owed {
                    self.owed.insert(id, owed);
                }
                self.journal
                    .line(&format!("session {id}: the relaunch on the hook set failed: {why}"));
                self.feed.board_outcome(
                    "automation",
                    "claude_road_relaunch_failed",
                    Some(ticket),
                    &why,
                );
            }
        }
        true
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
            // The gate's roots (T-577), as `mesimon gate` gets them in argv:
            // the decision is the mod's alone, so a dead daemon still denies.
            ("MESIMON_MOD_GATE_BOARD".into(), self.paths.board_dir.display().to_string()),
            ("MESIMON_MOD_GATE_STATE".into(), self.paths.state_dir.display().to_string()),
            ("MESIMON_MOD_GATE_ALLOW".into(), self.paths.worktrees_root().display().to_string()),
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

    // ---- Up: the mod's frames.

    /// A frame the mod relayed (`road: mod`). A pong answers its ping; the
    /// mod's own reports are read here; every other frame is the session's
    /// own report, ingested exactly as the hook set's was (T-577) when its
    /// pane reports through the mod alone, and dropped otherwise: a record
    /// an earlier build launched on the mod still carries its hook set, whose
    /// frames are the ones ingested until its next wake.
    pub(super) fn on_mod_hook(&mut self, frame: HookFrame) {
        if frame.event == MOD_PONG {
            if let Some(ping) = frame.reason.as_deref().and_then(|id| self.modroad.pings.remove(id))
            {
                let ms = now_ms().saturating_sub(ping.at_ms);
                reply_now(ping.reply, Response::ModPonged { ms });
            }
            return;
        }
        let Ok(session) = frame.session.parse::<uuid::Uuid>() else { return };
        let Some(rec) = self.board.sessions.iter().find(|s| s.id == session) else { return };
        if rec.road != Road::Mod {
            return;
        }
        let by_mod = rec.frames_by_mod();
        // The mod's own reports (T-575, T-576): the only source of what they
        // say, on every mod session.
        if frame.event == MOD_SUBMIT || frame.event == MOD_FILL || frame.event == MOD_ANSWER {
            self.feed.hook_event_by(
                &frame.session,
                &frame.event,
                frame.reason.as_deref(),
                Road::Mod,
            );
            if frame.event == MOD_SUBMIT {
                self.on_mod_submit(session, &frame);
            } else if frame.event == MOD_FILL {
                self.on_mod_fill(session, &frame);
            } else {
                self.on_mod_answer(session, frame);
            }
            return;
        }
        if frame.event == MOD_LOAD_FAILED {
            self.on_mod_load_failed(&frame);
            return;
        }
        if frame.event == MOD_USAGE {
            self.feed.hook_event_by(
                &frame.session,
                &frame.event,
                frame.reason.as_deref(),
                Road::Mod,
            );
            self.on_mod_usage(session, &frame);
            return;
        }
        if by_mod {
            self.on_hook(frame);
        }
    }

    /// A turn's end as the mod reports it (T-581): the account's windows go
    /// into the machine's quota reading, and the turn's count into its
    /// ticket's cost where the session reports through the mod alone (a
    /// record an earlier build launched keeps its hook set, and the tail
    /// counts it). A count with no transcript to fence is left to the tail.
    fn on_mod_usage(&mut self, session: uuid::Uuid, frame: &HookFrame) {
        let Some(rec) = self.board.sessions.iter().find(|s| s.id == session) else { return };
        let (ticket, by_mod, path) = (rec.ticket, rec.frames_by_mod(), rec.transcript_path.clone());
        let now = now_ms();
        let quota = frame.payload.get("rateLimits").is_some_and(|l| self.usage.merge_mod(l, now));
        let counted = by_mod
            && match (path, frame.payload.get("usage").and_then(mesimon_core::cost::mod_turn)) {
                (Some(path), Some((model, tokens))) => {
                    let subagent = frame.payload.get("agentId").is_some_and(|a| a.is_string());
                    self.costs.count_mod(ticket, &path, subagent, now, &model, tokens)
                }
                _ => false,
            };
        if counted {
            self.persist_costs();
        }
        if counted || quota {
            self.broadcast();
        }
    }

    /// The mod's report that reads of its pane variables failed before one
    /// succeeded (T-594): a feed line on the ticket and a journal line with
    /// the error, so a mod that was silent for a while says why.
    fn on_mod_load_failed(&mut self, frame: &HookFrame) {
        let Ok(session) = frame.session.parse::<uuid::Uuid>() else { return };
        let Some(ticket) = self
            .board
            .sessions
            .iter()
            .find(|s| s.id == session && s.road == Road::Mod)
            .map(|s| s.ticket)
        else {
            return;
        };
        let reads = frame.payload["reads"].as_u64().unwrap_or(0);
        let at = frame.payload["at"].as_str().unwrap_or("?");
        let error = frame.payload["error"].as_str().unwrap_or("no error given");
        let line = format!(
            "{reads} read(s) of the pane variables failed, the first at {}: {}",
            mesimon_core::text::cap_bytes(at, 40),
            mesimon_core::text::cap_bytes(error, 200)
        );
        self.journal.line(&format!("mod of session {session}: {line}"));
        self.feed.board_outcome("daemon", "mod_load_failed", Some(ticket), &line);
    }

    /// The tick's share: the pings that waited out their time, and the
    /// frames whose pane is gone.
    pub(super) fn tick_mod_road(&mut self) {
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

    /// A session lost its pane or its record: its frames and its parked
    /// poll go.
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
    }
}
