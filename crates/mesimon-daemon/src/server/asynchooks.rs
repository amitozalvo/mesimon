//! T-1044: interactive Claude exposes no async-hook completion ledger.
//! Hold the Stop while unaccounted children remain, then allow a rewake
//! grace unless Claude records their silent completion at this Stop.

use super::*;
use crate::panes::HookProcess;

const SCAN_MS: u64 = 1_500;
const REWAKE_MS: u64 = 3_000;
const CAP_MS: u64 = 10 * 60_000;

#[derive(Default)]
pub(super) struct Holds {
    pending: HashMap<uuid::Uuid, PendingStop>,
    // A hook from an earlier turn can survive a rewake and the next Stop.
    known: HashMap<uuid::Uuid, HashMap<i32, Child>>,
}

#[derive(Clone)]
struct Child {
    born: u64,
    last_seen: u64,
    argv: String,
}

struct PendingStop {
    signal: Signal,
    detail: Option<String>,
    stopped: u64,
    began: u64,
    shells: Vec<String>,
    saw_child: bool,
    // Keep each vanished child's evidence until its own rewake grace expires.
    exited: Vec<(Child, u64)>,
    /// The resting state this Stop lifted to `Running` with no turn open
    /// (T-1200): a `/clear`'s `Idle{Unknown}`, say. `None` when a turn was
    /// already running and the lift only kept it so.
    lifted_from: Option<SessionState>,
}

fn cap_ms() -> u64 {
    std::env::var("MESIMON_ASYNC_HOOK_CAP_MS").ok().and_then(|s| s.parse().ok()).unwrap_or(CAP_MS)
}

fn same_process(row: &HookProcess, born: u64) -> bool {
    // Linux derives the epoch from /proc/uptime, with clock-tick precision.
    row.born_ms.abs_diff(born) < 20
}

fn accounted(argv: &str, shells: &[String]) -> bool {
    let words: Vec<_> = argv.split_whitespace().collect();
    let name = words.first().and_then(|s| s.rsplit('/').next()).unwrap_or("");
    if argv.contains("<defunct>") || name.starts_with('(') {
        return true;
    }
    // Claude's own keep-awake and status line, not turn work.
    if name == "caffeinate" || argv.contains("statusline") || argv.contains("status-line") {
        return true;
    }
    if words.windows(2).any(|w| {
        w[0].rsplit('/').next() == Some("mesimon")
            && matches!(w[1], "mod-bridge" | "mcp" | "hook" | "approve" | "gate")
    }) {
        return true;
    }
    // Launch-time servers are excluded by age too. This catches a server
    // reconnecting during the turn under the usual MCP argv spellings.
    if words.iter().any(|w| {
        let w = w.rsplit('/').next().unwrap_or(w).to_ascii_lowercase();
        w == "mcp"
            || w.contains("mcp-server")
            || w.contains("mcp_server")
            || w.starts_with("mcp@")
            || w.ends_with("-mcp")
    }) {
        return true;
    }
    // Stop carries each background shell's command, a Monitor's watch
    // included. Claude's Bash wrapper keeps that command on argv, unlike the
    // command-hook runner: inside `eval '…'`, so each `'` of the command is
    // spelled `'"'"'` there (2.1.325, T-1259). Read past that spelling, or a
    // command with a quote in it is hook work for the ten-minute cap.
    let unquoted = argv.replace(r#"'"'"'"#, "'");
    shells.iter().any(|s| on_argv(argv, s) || on_argv(&unquoted, s))
}

/// Is this listed command the one on `argv` (T-1259)? Never the whole text
/// at once: `ps` writes a newline or a tab its own way (`\012` on macOS),
/// and Claude Code caps a listed command at 1000 characters and ends a
/// clipped one `… [+N chars]`. So the command's runs of printable ASCII,
/// the cap's marker dropped, are found on `argv` in order. Each run is
/// read with its spaces folded to one, as `parse_ps_rows` rejoins argv
/// (T-1292): a watch's indented line, `\012    if …` to `ps`, is `\012 if …`.
fn on_argv(argv: &str, command: &str) -> bool {
    let command = command
        .rsplit_once("… [+")
        .filter(|(_, n)| n.strip_suffix(" chars]").is_some_and(|n| n.parse::<u32>().is_ok()))
        .map_or(command, |(kept, _)| kept);
    let argv = argv.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut rest = argv.as_str();
    let mut found = false;
    for run in command.split(|c: char| !(' '..='~').contains(&c)) {
        let run = run.split_whitespace().collect::<Vec<_>>().join(" ");
        if run.is_empty() {
            continue;
        }
        let Some(at) = rest.find(&run) else { return false };
        rest = &rest[at + run.len()..];
        found = true;
    }
    found
}

/// The full command as launched directly, by a shell -c, or by a script's
/// shebang. A command mentioned in another program's arguments is no match.
fn hook_command(argv: &str, command: &str) -> bool {
    if argv == command {
        return true;
    }
    let Some((shell, args)) = argv.split_once(' ') else { return false };
    matches!(shell.rsplit('/').next(), Some("sh" | "bash" | "zsh" | "dash"))
        && args.strip_prefix("-c ").unwrap_or(args) == command
}

/// Interactive Claude's Stop summary gives a duration to completed hooks.
/// An async dispatch has a command too, but no duration. Only consult the
/// bounded transcript tail when a grace is all that still holds the Stop.
fn silent_observers(path: &str, stopped: u64, at: u64, exited: &[(Child, u64)]) -> bool {
    let path = std::path::Path::new(path);
    let Ok(meta) = path.metadata() else { return false };
    if !meta.is_file() {
        return false;
    }
    let Some(text) = crate::agents::claude::tail::window_text(path, meta.len(), 64 * 1024) else {
        return false;
    };
    let Some(summary) = text
        .lines()
        .rev()
        .filter(|line| line.contains("\"stop_hook_summary\""))
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|v| {
            v["type"] == "system"
                && v["subtype"] == "stop_hook_summary"
                && v["isSidechain"] == false
                && v["hookLabel"].is_null()
        })
    else {
        return false;
    };
    let Some(finished) = mesimon_core::adopt::record_ms(&summary) else { return false };
    if !(stopped..=at).contains(&finished)
        || summary["hasOutput"] != false
        || summary["preventedContinuation"] != false
        || !summary["hookErrors"].as_array().is_some_and(Vec::is_empty)
    {
        return false;
    }
    let Some(infos) = summary["hookInfos"].as_array() else { return false };
    exited.iter().all(|(child, _)| {
        let mut matches = infos.iter().filter_map(|info| {
            let cmd = info["command"].as_str()?;
            hook_command(&child.argv, cmd).then_some((cmd, info["durationMs"].as_u64()))
        });
        let Some((command, Some(duration))) = matches.next() else { return false };
        // One completed invocation must account for exactly one sampled
        // process. A child that predates the dispatch or outlived the
        // summary cannot be the hook that completed.
        matches.next().is_none()
            && child.last_seen.saturating_sub(child.born) <= duration
            // A sampler dates its rows at query start, before ps reads them.
            && child.born.max(child.last_seen) <= finished
            && exited.iter().filter(|(c, _)| hook_command(&c.argv, command)).count() == 1
    })
}

impl Daemon {
    /// The signal the machine reads, and with it the resting state a lifted
    /// hold left when this frame began a turn over it (T-1200): the lift's
    /// withheld edge is this turn's begin, which `begin_lifted_turn` makes
    /// once the machine has read the frame.
    pub(super) fn hold_async_stop(
        &mut self,
        id: uuid::Uuid,
        sig: Signal,
        frame: &HookFrame,
        detail: Option<String>,
        now: u64,
    ) -> (Signal, Option<SessionState>) {
        let mut lifted = None;
        match &sig {
            // The lead's own tool, started or done, is a turn after the held
            // Stop: the one Claude Code runs on a `!` command's output opens
            // with no prompt (T-1140), and its first frame is a completion.
            // A held Stop that lifted a resting seat opened no turn, and the
            // machine already reads `Running`, so this frame moves nothing:
            // the lift's withheld edge is the new turn's begin (T-1200), or
            // its end, with a commit on a checkout, never wakes the crown.
            Signal::UserPromptSubmit
            | Signal::ToolStarted { nested: false }
            | Signal::ToolCompleted { nested: false } => {
                lifted = self.async_hooks.pending.remove(&id).and_then(|p| p.lifted_from);
            }
            Signal::PaneDied { .. }
            | Signal::SessionEnd { .. }
            | Signal::SessionStart { .. }
            | Signal::StopFailure { .. } => {
                self.async_hooks.pending.remove(&id);
                self.async_hooks.known.remove(&id);
            }
            _ => {}
        }
        if !matches!(sig, Signal::Stop { has_agent_id: false, .. }) {
            return (sig, lifted);
        }
        let Some(rec) = self.board.sessions.iter().find(|r| {
            r.id == id
                && r.kind == SessionKind::Claude
                && r.state.has_pane()
                && r.pane_key.is_some()
                && !r.observe_only()
                && r.state != (SessionState::Idle { stop_reason: StopReason::EndTurn })
        }) else {
            return (sig, lifted);
        };
        let shells = frame
            .payload
            .get("background_tasks")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter(|t| {
                matches!(
                    t.get("type").and_then(serde_json::Value::as_str),
                    Some("shell" | "monitor")
                )
            })
            .filter_map(|t| t.get("command").and_then(serde_json::Value::as_str))
            .map(str::to_owned)
            .collect();
        self.usage.turn_ended(Provider::Claude);
        self.cost_due = true;
        self.async_hooks.pending.insert(
            id,
            PendingStop {
                signal: sig,
                detail,
                stopped: now,
                began: self.turn_since.get(&id).copied().or(rec.launched_at).unwrap_or(now),
                shells,
                saw_child: false,
                exited: Vec::new(),
                lifted_from: (!mesimon_core::quiet::is_working(rec)).then(|| rec.state.clone()),
            },
        );
        self.pane_due.facts = true;
        self.pane_due.hook_rows = true;
        // Keep the existing turn open. No delivered edge, automove, or
        // crown turn-end probe exists until the saved Stop is released.
        (Signal::TurnStarted, lifted)
    }

    /// The turn this frame began over a lifted hold (T-1200), as the edge
    /// the lift withheld: automove, the open turn and its age then follow
    /// as from any begin. `lifted` is `hold_async_stop`'s for the same
    /// frame. Called after the machine read the frame, so the prompt's own
    /// bookkeeping (T-186's hand rule) comes first.
    pub(super) fn begin_lifted_turn(
        &mut self,
        id: uuid::Uuid,
        lifted: Option<SessionState>,
        hook: &str,
    ) -> bool {
        let Some(from) = lifted else { return false };
        if self.board.session(id).is_none_or(|r| r.state != SessionState::Running) {
            return false;
        }
        let change = Change {
            from,
            to: SessionState::Running,
            attention_added: false,
            confidence: Confidence::High,
        };
        self.apply_change(id, &change, None, Some(hook))
    }

    pub(super) fn async_hook_view(&self, id: uuid::Uuid) -> Option<mesimon_core::board::HookWait> {
        let pending = self.async_hooks.pending.get(&id)?;
        Some(mesimon_core::board::HookWait {
            count: self
                .async_hooks
                .known
                .get(&id)
                .map_or(0, |rows| rows.len().min(u32::MAX as usize) as u32),
            since_ms: pending.stopped,
        })
    }

    pub(super) fn async_stop_pending(&self, id: uuid::Uuid) -> bool {
        self.async_hooks.pending.contains_key(&id)
    }

    pub(super) fn tick_async_stops(&mut self, now: u64) -> bool {
        let expired: Vec<_> = self
            .async_hooks
            .pending
            .iter()
            .filter(|(_, p)| now.saturating_sub(p.stopped) >= cap_ms())
            .map(|(id, _)| *id)
            .collect();
        let mut changed = false;
        for id in expired {
            let ticket = self.board.session(id).map(|r| r.ticket);
            self.feed.board_outcome(
                "automation",
                "async_hook_cap",
                ticket,
                &format!(
                    "{} ms elapsed; releasing the held Stop without confirmed hook completion",
                    cap_ms()
                ),
            );
            self.async_hooks.known.remove(&id);
            changed |= self.release_async_stop(id);
        }
        if !self.async_hooks.pending.is_empty() && self.ticks.is_multiple_of(4) {
            self.pane_due.facts = true;
            self.pane_due.hook_rows = true;
        }
        changed
    }

    pub(super) fn sample_async_stops(
        &mut self,
        facts: Option<&[mesimon_backend_tmux::PaneFacts]>,
        rows: Result<&[HookProcess], ()>,
        at: u64,
    ) -> bool {
        let mut ready = Vec::new();
        let mut changed = false;
        for (id, pending) in &mut self.async_hooks.pending {
            // Never use a sample that started before this Stop.
            if at < pending.stopped {
                continue;
            }
            let Some(rec) = self.board.session(*id) else { continue };
            let pane = facts.and_then(|fs| fs.iter().find(|f| f.session_name == rec.sid16()));
            let missing = match pane {
                Some(p) if p.pane_dead => Some("pane is dead"),
                None if facts.is_none() => Some("pane facts read failed"),
                None => Some("pane missing from sample"),
                Some(p) => match rows {
                    Err(()) => Some("process read failed (argv, parentage or birth unavailable)"),
                    Ok(rs) if !rs.iter().any(|r| r.pid == p.pane_pid) => {
                        Some("pane process missing from sample")
                    }
                    _ => None,
                },
            };
            if let Some(why) = missing {
                // Only positively seen children earn a hold. An unreadable
                // machine must not impose the ten-minute cap on every turn.
                if pane.is_some_and(|p| p.pane_dead)
                    || at.saturating_sub(pending.stopped) >= SCAN_MS
                {
                    ready.push((*id, Some(why)));
                }
                continue;
            }
            let (Some(pane), Ok(rows)) = (pane, rows) else { continue };
            let known = self.async_hooks.known.entry(*id).or_default();
            let before = known.len();
            // A child kept from an earlier Stop is read against this one's
            // list too (T-1292): a background shell or watch this Stop lists
            // is the agent's, whatever an earlier reading made of it. An
            // async hook is never listed, so it stays held.
            known.retain(|pid, child| {
                if let Some(row) = rows.iter().find(|r| {
                    r.pid == *pid && r.ppid == pane.pane_pid && same_process(r, child.born)
                }) {
                    if !accounted(&row.argv, &pending.shells) {
                        child.last_seen = at;
                        return true;
                    }
                }
                if child.last_seen >= pending.stopped {
                    pending.exited.push((child.clone(), at));
                }
                false
            });
            pending.exited.retain(|(_, gone)| at.saturating_sub(*gone) < REWAKE_MS);
            for row in rows.iter().filter(|r| r.ppid == pane.pane_pid) {
                let born = row.born_ms;
                // Stop hooks may be dispatched after our own Stop observer.
                if row.pgid != row.pid
                    || born < pending.began
                    || born > pending.stopped.saturating_add(SCAN_MS)
                    || accounted(&row.argv, &pending.shells)
                {
                    continue;
                }
                known.entry(row.pid).or_insert_with(|| Child {
                    born,
                    last_seen: at,
                    argv: row.argv.clone(),
                });
            }
            changed |= known.len() != before;
            if !known.is_empty() {
                if !pending.saw_child {
                    self.feed.board_outcome(
                        "automation",
                        "async_hook_wait",
                        Some(rec.ticket),
                        &format!(
                            "{} unaccounted child processes; delivery and park wait",
                            known.len()
                        ),
                    );
                }
                pending.saw_child = true;
            } else if at.saturating_sub(pending.stopped) >= SCAN_MS
                && (pending.exited.is_empty()
                    || rec.transcript_path.as_deref().is_some_and(|path| {
                        silent_observers(path, pending.stopped, at, &pending.exited)
                    }))
            {
                ready.push((*id, None));
            }
        }
        for (id, missing) in ready {
            if let Some(why) = missing {
                let ticket = self.board.session(id).map(|r| r.ticket);
                self.feed.board_outcome("automation", "async_hook_unobserved", ticket, why);
                self.async_hooks.known.remove(&id);
            }
            changed |= self.release_async_stop(id);
        }
        changed
    }

    fn release_async_stop(&mut self, id: uuid::Uuid) -> bool {
        let Some(pending) = self.async_hooks.pending.remove(&id) else { return false };
        let by = Principal::Automation { rule: "async_hook_completion".into() };
        if matches!(
            authorize(&by, &Action::Mutate, &Resource::Session { id }),
            Decision::Deny { .. }
        ) {
            return false;
        }
        let Some(machine) = self.machines.get_mut(&id) else { return false };
        // The Stop was observed at `stopped`; its ordinary settle has
        // already elapsed while we sampled. Do not charge it a second time.
        let (change, decision) = machine.apply_explained(&pending.signal, pending.stopped);
        self.feed.state_decision(id, "async_hook_release", &decision);
        if let Some(change) = change {
            self.apply_change(id, &change, pending.detail, Some("async_hook_release"));
        }
        // The snapshot loses its hook-wait decoration even if the machine
        // still has an ordinary settle to flush on its next tick.
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Exercise the daemon's release and feed, not only an availability predicate.
    fn unavailable(
        facts: Option<Vec<mesimon_backend_tmux::PaneFacts>>,
        rows: Result<Vec<HookProcess>, ()>,
        why: &str,
        dead: bool,
    ) {
        let dir = std::env::temp_dir()
            .join(format!("msmn-hku-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]));
        let (mut d, sid, _rx) = crate::server::queued_person_push_tests::daemon(&dir);
        let id = d.board.sessions[0].id;
        let rec = &mut d.board.sessions[0];
        rec.state = SessionState::Running;
        rec.pane_key = Some("1:%0".into());
        d.machines.insert(id, Machine::new(SessionState::Running, 1_000));
        let frame = HookFrame {
            session: id.to_string(),
            event: "Stop".into(),
            reason: None,
            pane: None,
            road: Road::Hooks,
            payload: serde_json::json!({}),
        };
        let stop = Signal::Stop {
            stop_hook_active: false,
            has_agent_id: false,
            blocking_tasks: false,
            monitoring_tasks: false,
            teammates: 0,
        };
        assert!(matches!(d.hold_async_stop(id, stop, &frame, None, 1_000).0, Signal::TurnStarted));
        let facts = facts.map(|mut fs| {
            for f in &mut fs {
                f.session_name = sid.clone();
            }
            fs
        });
        // Even previously seen work cannot turn a failed reader into a cap wait.
        d.async_hooks.pending.get_mut(&id).unwrap().saw_child = true;
        d.sample_async_stops(facts.as_deref(), rows.as_deref().map_err(|_| ()), 1_001);
        assert_eq!(d.async_stop_pending(id), !dead);
        d.sample_async_stops(facts.as_deref(), rows.as_deref().map_err(|_| ()), 1_000 + SCAN_MS);
        assert!(!d.async_stop_pending(id), "no cap wait for {why}");
        d.feed.flush().unwrap();
        let feed = std::fs::read_to_string(d.paths.activity_log()).unwrap();
        assert!(feed.contains("async_hook_unobserved") && feed.contains(why), "{feed}");
        assert!(!feed.contains("async_hook_cap"));
        let change = d.machines.get_mut(&id).unwrap().tick(10_000).unwrap();
        assert_eq!(change.to, SessionState::Idle { stop_reason: StopReason::EndTurn });
        drop(d);
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn pane(dead: bool) -> mesimon_backend_tmux::PaneFacts {
        mesimon_backend_tmux::PaneFacts {
            session_name: String::new(),
            pane_pid: 10,
            pane_dead: dead,
            reaped: dead,
            server_pid: 1,
            current_command: "claude".into(),
            attached: false,
            activity: None,
            title: String::new(),
        }
    }

    // T-1140: a `!` command queued behind a turn is a young child that holds
    // its Stop; the turn Claude Code then runs on the output opens with no
    // prompt. That turn's own tool completion cancels the hold, so its later
    // release cannot land an idle edge inside the turn. A subagent's does not.
    #[test]
    fn the_leads_tool_completion_cancels_a_held_stop() {
        let dir = std::env::temp_dir()
            .join(format!("msmn-hkc-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]));
        let (mut d, _sid, _rx) = crate::server::queued_person_push_tests::daemon(&dir);
        let id = d.board.sessions[0].id;
        let rec = &mut d.board.sessions[0];
        rec.state = SessionState::Running;
        rec.pane_key = Some("1:%0".into());
        d.machines.insert(id, Machine::new(SessionState::Running, 1_000));
        let frame = HookFrame {
            session: id.to_string(),
            event: "Stop".into(),
            reason: None,
            pane: None,
            road: Road::Mod,
            payload: serde_json::json!({}),
        };
        let stop = Signal::Stop {
            stop_hook_active: false,
            has_agent_id: false,
            blocking_tasks: false,
            monitoring_tasks: false,
            teammates: 0,
        };
        assert!(matches!(d.hold_async_stop(id, stop, &frame, None, 1_000).0, Signal::TurnStarted));
        let nested = Signal::ToolCompleted { nested: true };
        assert!(matches!(
            d.hold_async_stop(id, nested, &frame, None, 2_000).0,
            Signal::ToolCompleted { nested: true }
        ));
        assert!(d.async_stop_pending(id), "a subagent's completion keeps the hold");
        let lead = Signal::ToolCompleted { nested: false };
        assert!(matches!(
            d.hold_async_stop(id, lead, &frame, None, 3_000).0,
            Signal::ToolCompleted { nested: false }
        ));
        assert!(!d.async_stop_pending(id), "the lead's completion is a turn of its own");
        drop(d);
        std::fs::remove_dir_all(dir).unwrap();
    }

    // T-1200, crown_e2e's lost wake in order: a `/clear` leaves the seat at
    // `Idle{Unknown}` with no turn open, a bare `Stop` is held and lifts it
    // to `Running`, and the next prompt lands before the sampler releases
    // the hold. The prompt is the turn's begin all the same: its end is a
    // fresh one, which a checkout's commit needs to be told delivered.
    #[test]
    fn a_prompt_over_a_lifted_hold_opens_its_turn() {
        let dir = std::env::temp_dir()
            .join(format!("msmn-hkl-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]));
        let (mut d, _sid, _rx) = crate::server::queued_person_push_tests::daemon(&dir);
        d.queued.clear();
        let id = d.board.sessions[0].id;
        let ticket = d.board.sessions[0].ticket;
        let clear = SessionState::Idle { stop_reason: StopReason::Unknown };
        let rec = &mut d.board.sessions[0];
        rec.state = clear.clone();
        rec.pane_key = Some("1:%0".into());
        d.machines.insert(id, Machine::new(clear, 1_000));
        let frame = |event: &str| HookFrame {
            session: id.to_string(),
            event: event.into(),
            reason: None,
            pane: None,
            road: Road::Hooks,
            payload: serde_json::json!({}),
        };
        let state = |d: &Daemon| d.board.session(id).unwrap().state.clone();

        d.on_hook(frame("Stop"));
        assert!(d.async_stop_pending(id), "the bare Stop is held");
        assert_eq!(state(&d), SessionState::Running, "and lifts the seat");
        assert!(!d.turns_open.contains(&ticket), "a held Stop opens no turn");

        d.on_hook(frame("UserPromptSubmit"));
        assert!(!d.async_stop_pending(id), "the prompt is a turn after the hold");
        assert_eq!(state(&d), SessionState::Running);
        assert!(d.turns_open.contains(&ticket), "the prompt opened its turn");
        assert!(d.turn_since.contains_key(&id), "and dated it");

        // A prompt over a hold that kept a running turn open replays nothing.
        d.on_hook(frame("Stop"));
        assert!(d.async_stop_pending(id));
        d.on_hook(frame("UserPromptSubmit"));
        d.feed.flush().unwrap();
        let feed = std::fs::read_to_string(d.paths.activity_log()).unwrap();
        let begins = feed
            .lines()
            .filter(|l| {
                l.contains("\"kind\":\"session_state\"") && l.contains("\"to\":\"running\"")
            })
            .count();
        assert_eq!(begins, 2, "the lift, then the prompt's begin, once: {feed}");
        drop(d);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_pane_row_releases_after_scan() {
        unavailable(Some(vec![]), Ok(vec![]), "pane missing from sample", false);
    }

    #[test]
    fn missing_pane_process_releases_after_scan() {
        unavailable(Some(vec![pane(false)]), Ok(vec![]), "pane process missing from sample", false);
    }

    #[test]
    fn failed_process_read_releases_after_scan() {
        unavailable(Some(vec![pane(false)]), Err(()), "process read failed", false);
    }

    #[test]
    fn failed_pane_read_releases_after_scan() {
        unavailable(None, Err(()), "pane facts read failed", false);
    }

    #[test]
    fn dead_pane_releases_before_scan_even_without_processes() {
        unavailable(Some(vec![pane(true)]), Err(()), "pane is dead", true);
    }

    #[test]
    fn infrastructure_and_reported_shells_are_not_hook_work() {
        for argv in [
            "/bin/mesimon mod-bridge --session abc",
            "/bin/mesimon mcp --call get_ticket",
            "node /pkg/exa-mcp-server",
            "caffeinate -i -t 300",
            "bash /home/me/statusline.sh",
            "/bin/zsh -c eval 'sleep 600'",
            "<defunct>",
        ] {
            assert!(accounted(argv, &["sleep 600".into()]), "{argv}");
        }
        for argv in ["python3 /plugin/review.py", "/bin/sh -c review", "sleep 30"] {
            assert!(!accounted(argv, &["sleep 600".into()]), "{argv}");
        }
    }

    /// T-1259: the argv Claude Code 2.1.325 gave a crown's background
    /// command, whose `--format='%h %s'` kept its `Stop` held for the cap
    /// and the crown's wakes behind it.
    #[test]
    fn a_reported_shell_with_a_quote_in_its_command_is_not_hook_work() {
        let command = "git log -1 --format='%h %s' && ~/deploy.sh > /tmp/deploy.log 2>&1";
        let argv = concat!(
            "/bin/zsh -c source /home/me/.claude/shell-snapshots/snapshot-zsh-1-a.sh ",
            "2>/dev/null || true && eval 'git log -1 --format='\"'\"'%h %s'\"'\"' && ",
            "~/deploy.sh > /tmp/deploy.log 2>&1' < /dev/null && pwd -P >| /tmp/claude-b9c2-cwd",
        );
        assert!(accounted(argv, &[command.into()]));
        // A shell the Stop did not list stays hook work (a `!` command's, T-1140).
        assert!(!accounted(argv, &["git log -1 --format='%h'".into()]));
        assert!(!accounted(argv, &[]));
    }

    /// T-1259: a Monitor's watch as the same crown armed it, a loop with a
    /// Python script on lines of its own. `ps` writes each newline `\012`.
    #[test]
    fn a_reported_watch_on_several_lines_is_not_hook_work() {
        let command = "while true; do glab api jobs | python3 -c \"\nimport sys,json\n\
                       d={j['name']:j['status'] for j in json.load(sys.stdin)}\n\
                       print(d)\" || true; sleep 45; done";
        let argv = concat!(
            "/bin/zsh -c source /home/me/.claude/shell-snapshots/snapshot-zsh-1-a.sh ",
            r#"2>/dev/null || true && eval 'while true; do glab api jobs | python3 -c ""#,
            r#"\012import sys,json\012d={j['"'"'name'"'"']:j['"'"'status'"'"'] "#,
            r#"for j in json.load(sys.stdin)}\012print(d)" || true; sleep 45; done' "#,
            "< /dev/null && pwd -P >| /tmp/claude-8b3b-cwd",
        );
        assert!(accounted(argv, &[command.into()]));
        // The same lines in another order are another command.
        let other = "print(d)\nimport sys,json";
        assert!(!accounted(argv, &[other.into()]));
        // A blank command names nothing.
        assert!(!accounted(argv, &["\n \t".into()]));
    }

    /// T-1292: the simbly crown's watch, a Python script with an indented
    /// line, as the daemon reads it: `parse_ps_rows` folds argv's spaces, so
    /// `\012    if` is `\012 if` there. Its every `Stop` was held for the cap.
    #[test]
    fn a_reported_watch_with_an_indented_line_is_not_hook_work() {
        let command = "while true; do out=$(glab api jobs | python3 -I -c \"\nimport json,sys\n\
                       for j in json.load(sys.stdin):\n    if j['name'] in ('a','b'): \
                       print(j['name']+': '+j['status'])\n\" | sort); sleep 60; done";
        let line = concat!(
            "90809 73462 05:00 /bin/zsh -c source /home/me/.claude/shell-snapshots/snapshot-zsh-1-a.sh ",
            r#"2>/dev/null || true && eval 'while true; do out=$(glab api jobs | python3 -I -c ""#,
            r#"\012import json,sys\012for j in json.load(sys.stdin):\012    if j['"'"'name'"'"'] "#,
            r#"in ('"'"'a'"'"','"'"'b'"'"'): print(j['"'"'name'"'"']+'"'"': '"'"'+j['"'"'status'"'"'])"#,
            r#"\012" | sort); sleep 60; done' < /dev/null && pwd -P >| /tmp/claude-0a7c-cwd"#,
        );
        let rows = mesimon_core::board::parse_ps_rows(line);
        assert!(accounted(&rows[0].args, &[command.into()]), "{}", rows[0].args);
        // The words still have to be the command's, in its order.
        assert!(!accounted(&rows[0].args, &["for j in x:\n    if k".into()]));
    }

    /// T-1292: a held Stop across a seat's turns. A watch counted at a Stop
    /// that did not list it (the indented line above, before its fix) is the
    /// agent's at the next Stop that does, as is a background command; an
    /// async hook no Stop lists is held until it exits and the grace passes.
    #[test]
    fn listed_shells_release_every_stop_and_an_async_hook_still_holds() {
        let dir = std::env::temp_dir()
            .join(format!("msmn-hks-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]));
        let (mut d, sid, _rx) = crate::server::queued_person_push_tests::daemon(&dir);
        let id = d.board.sessions[0].id;
        let rec = &mut d.board.sessions[0];
        rec.state = SessionState::Running;
        rec.pane_key = Some("1:%0".into());
        rec.launched_at = Some(0);
        d.machines.insert(id, Machine::new(SessionState::Running, 0));
        let mut facts = vec![pane(false)];
        facts[0].session_name = sid;
        let watch = "while true; do echo tick; sleep 30; done";
        let deploy = "~/deploy.sh 2>&1 | tail -40";
        let wrap = |c: &str| {
            format!("/bin/zsh -c source /s.sh 2>/dev/null || true && eval '{c}' < /dev/null")
        };
        let row =
            |pid, born, argv: String| HookProcess { pid, ppid: 10, pgid: pid, born_ms: born, argv };
        let claude =
            || HookProcess { pid: 10, ppid: 1, pgid: 10, born_ms: 0, argv: "claude".into() };
        let stop = |d: &mut Daemon, listed: &[&str], at: u64| {
            let tasks: Vec<_> = listed
                .iter()
                .map(|c| serde_json::json!({"id": "t", "type": "shell", "status": "running", "command": c}))
                .collect();
            let frame = HookFrame {
                session: id.to_string(),
                event: "Stop".into(),
                reason: None,
                pane: None,
                road: Road::Mod,
                payload: serde_json::json!({ "background_tasks": tasks }),
            };
            let sig = Signal::Stop {
                stop_hook_active: false,
                has_agent_id: false,
                blocking_tasks: false,
                monitoring_tasks: !listed.is_empty(),
                teammates: 0,
            };
            d.hold_async_stop(id, sig, &frame, None, at);
        };

        // A Stop that does not list the watch counts it, and a prompt opens
        // the next turn over the hold: the watch stays known.
        stop(&mut d, &[], 1_000);
        let rows = vec![claude(), row(20, 900, wrap(watch))];
        d.sample_async_stops(Some(&facts), Ok(&rows), 1_001);
        assert!(d.async_hooks.known[&id].contains_key(&20));
        let frame = HookFrame {
            session: id.to_string(),
            event: "UserPromptSubmit".into(),
            reason: None,
            pane: None,
            road: Road::Mod,
            payload: serde_json::json!({}),
        };
        d.hold_async_stop(id, Signal::UserPromptSubmit, &frame, None, 5_000);
        assert!(!d.async_stop_pending(id));

        // Its next Stop lists the watch, and a deploy started in this turn.
        stop(&mut d, &[watch, deploy], 10_000);
        let both = || vec![claude(), row(20, 900, wrap(watch)), row(21, 9_000, wrap(deploy))];
        let rows = both();
        d.sample_async_stops(Some(&facts), Ok(&rows), 10_001);
        assert!(d.async_stop_pending(id), "the scan window first");
        d.sample_async_stops(Some(&facts), Ok(&rows), 10_000 + SCAN_MS);
        assert!(!d.async_stop_pending(id), "both listed: released after the scan");
        assert!(d.async_hooks.known[&id].is_empty());

        // And the one after, both still running: released again.
        stop(&mut d, &[watch, deploy], 20_000);
        d.sample_async_stops(Some(&facts), Ok(&rows), 20_000 + SCAN_MS);
        assert!(!d.async_stop_pending(id), "a second Stop over the same shells");

        // An async hook beside them is no listed shell: held while it runs,
        // and through the rewake grace after it exits.
        stop(&mut d, &[watch, deploy], 30_000);
        let hook = row(30, 30_050, "python3 /plugin/review.py".into());
        let mut with_hook = both();
        with_hook.push(hook);
        d.sample_async_stops(Some(&facts), Ok(&with_hook), 30_001);
        d.sample_async_stops(Some(&facts), Ok(&with_hook), 30_000 + SCAN_MS + 5_000);
        assert!(d.async_stop_pending(id), "the hook holds its Stop");
        assert_eq!(d.async_hooks.known[&id].len(), 1, "the hook alone is counted");
        d.sample_async_stops(Some(&facts), Ok(&rows), 40_000);
        assert!(d.async_stop_pending(id), "the rewake grace after it exits");
        d.sample_async_stops(Some(&facts), Ok(&rows), 40_000 + REWAKE_MS);
        assert!(!d.async_stop_pending(id));
        drop(d);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_silent_stop_observer_releases_on_exit_and_an_async_review_keeps_its_grace() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let (mut d, sid, _rx) = crate::server::queued_person_push_tests::daemon(dir);
        let transcript = dir.join("session.jsonl");
        let observer = dir.join("observer.sh");
        std::fs::write(&observer, "cat >/dev/null\n").unwrap();
        let command = format!("sh {}", observer.display());
        let output = std::process::Command::new("sh").arg(&observer).output().unwrap();
        assert!(output.status.success() && output.stdout.is_empty());

        let rec = &mut d.board.sessions[0];
        let id = rec.id;
        rec.state = SessionState::Running;
        rec.pane_key = Some("1:%0".into());
        rec.launched_at = Some(0);
        rec.transcript_path = Some(transcript.display().to_string());
        let mut facts = [pane(false)];
        facts[0].session_name = sid;
        let frame = HookFrame {
            session: id.to_string(),
            event: "Stop".into(),
            reason: None,
            pane: None,
            road: Road::Hooks,
            payload: serde_json::json!({}),
        };
        let stop = Signal::Stop {
            stop_hook_active: false,
            has_agent_id: false,
            blocking_tasks: false,
            monitoring_tasks: false,
            teammates: 0,
        };
        let process = |pid, argv: &str| HookProcess {
            pid,
            ppid: if pid == 10 { 1 } else { 10 },
            pgid: pid,
            born_ms: 1_000,
            argv: argv.into(),
        };
        let sample = |d: &mut Daemon, rows: &[HookProcess], at| {
            d.sample_async_stops(Some(&facts), Ok(rows), at);
        };
        let review = "python3 /fixture/review.py";
        for with_review in [false, true] {
            d.machines.insert(id, Machine::new(SessionState::Running, 0));
            d.hold_async_stop(id, stop.clone(), &frame, None, 1_000);
            let mut rows = vec![process(10, "claude"), process(20, &command)];
            if with_review {
                rows.push(process(30, review));
            }
            sample(&mut d, &rows, 1_010);
            assert_eq!(d.async_hook_view(id).unwrap().count, rows.len() as u32 - 1);
            // Claude's real summary shape: only a finished command has a duration.
            let mut infos = vec![serde_json::json!({"command": command, "durationMs": 30})];
            if with_review {
                infos.push(serde_json::json!({"command": review}));
            }
            std::fs::write(
                &transcript,
                serde_json::json!({
                    "type": "system", "subtype": "stop_hook_summary", "isSidechain": false,
                    "timestamp": "1970-01-01T00:00:01.040Z", "hookInfos": infos,
                    "hookErrors": [], "hasOutput": false, "preventedContinuation": false,
                })
                .to_string(),
            )
            .unwrap();
            rows.remove(1);
            let mut idle_after = None;
            for at in (2_000..=9_000).step_by(500) {
                if with_review && at == 5_000 {
                    rows.pop();
                }
                sample(&mut d, &rows, at);
                if let Some(change) = d.machines.get_mut(&id).unwrap().tick(at) {
                    assert_eq!(change.to, SessionState::Idle { stop_reason: StopReason::EndTurn });
                    idle_after = Some(at - 1_000);
                    break;
                }
            }
            assert_eq!(idle_after, Some(if with_review { 4_000 + REWAKE_MS } else { SCAN_MS }));
        }
    }

    #[test]
    fn a_summary_only_accounts_for_a_unique_silent_completion_at_this_stop() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let command = "/opt/observer/hook.sh";
        let completed = json!({"command": command, "durationMs": 50});
        let summary = json!({
            "type": "system", "subtype": "stop_hook_summary", "isSidechain": false,
            "timestamp": "1970-01-01T00:00:01.050Z", "hookInfos": [completed],
            "hookErrors": [], "hasOutput": false, "preventedContinuation": false,
        });
        let check = |record: &serde_json::Value, children: &[&str]| {
            std::fs::write(&path, record.to_string()).unwrap();
            let exited = children
                .iter()
                .map(|argv| {
                    (Child { argv: argv.to_string(), born: 1_000, last_seen: 1_020 }, 2_000)
                })
                .collect::<Vec<_>>();
            silent_observers(path.to_str().unwrap(), 1_000, 2_000, &exited)
        };
        let shell = "/bin/sh -c /opt/observer/hook.sh";
        assert!(check(&summary, &[shell]));
        assert!(check(&summary, &["/bin/bash /opt/observer/hook.sh"]));
        assert!(!check(&summary, &[shell, command]));
        assert!(!check(&summary, &["python3 review.py /opt/observer/hook.sh"]));
        // A query that started before completion can sample a later process.
        let late = Child { argv: shell.into(), born: 1_055, last_seen: 1_025 };
        assert!(!silent_observers(path.to_str().unwrap(), 1_000, 2_000, &[(late, 2_000)]));
        for (field, value) in [
            ("hasOutput", json!(true)),
            ("hookErrors", json!(["exit 1"])),
            ("preventedContinuation", json!(true)),
            ("timestamp", json!("1970-01-01T00:00:00.999Z")),
            ("hookInfos", json!([{"command": command}])),
            ("hookInfos", json!([completed, {"command": command}])),
        ] {
            let mut record = summary.clone();
            record[field] = value;
            assert!(!check(&record, &[shell]), "{record}");
        }
    }

    /// Claude Code lists at most 1000 characters of a command and marks the
    /// clip (2.1.325's own schema text): the kept part is what is matched.
    #[test]
    fn a_reported_shell_clipped_at_the_cap_is_matched_by_what_was_kept() {
        let argv = "/bin/zsh -c eval 'sleep 600 && echo a very long tail' < /dev/null";
        assert!(accounted(argv, &["sleep 600 && echo a very… [+412 chars]".into()]));
        assert!(!accounted(argv, &["sleep 700 && echo a very… [+412 chars]".into()]));
        // Not the marker: the text is the command's own, and is not on argv.
        assert!(!accounted(argv, &["sleep 600… [+many chars]".into()]));
    }

    #[test]
    fn a_reused_pid_does_not_inherit_a_previous_hooks_lifetime() {
        let row = HookProcess { pid: 10, ppid: 1, pgid: 10, born_ms: 5_000, argv: "review".into() };
        assert!(same_process(&row, 5_010));
        assert!(!same_process(&row, 1_000));
    }
}
