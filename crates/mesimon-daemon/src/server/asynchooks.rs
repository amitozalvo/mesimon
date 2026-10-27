//! T-1044: interactive Claude exposes no async-hook completion ledger.
//! Hold the Stop while unaccounted children remain, then allow a rewake
//! grace. This is process evidence, not a claim that a review passed.

use super::*;
use crate::panes::HookProcess;

const SCAN_MS: u64 = 1_500;
const REWAKE_MS: u64 = 3_000;
const CAP_MS: u64 = 10 * 60_000;

#[derive(Default)]
pub(super) struct Holds {
    pending: HashMap<uuid::Uuid, PendingStop>,
    // A hook from an earlier turn can survive a rewake and the next Stop.
    known: HashMap<uuid::Uuid, HashMap<i32, u64>>,
}

struct PendingStop {
    signal: Signal,
    detail: Option<String>,
    stopped: u64,
    began: u64,
    shells: Vec<String>,
    saw_child: bool,
    empty_since: Option<u64>,
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
    // Stop carries each background shell's command. Claude's Bash wrapper
    // keeps that command on argv, unlike the command-hook runner.
    shells.iter().any(|s| !s.is_empty() && argv.contains(s))
}

impl Daemon {
    pub(super) fn hold_async_stop(
        &mut self,
        id: uuid::Uuid,
        sig: Signal,
        frame: &HookFrame,
        detail: Option<String>,
        now: u64,
    ) -> Signal {
        match &sig {
            Signal::UserPromptSubmit | Signal::ToolStarted { nested: false } => {
                self.async_hooks.pending.remove(&id);
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
            return sig;
        }
        let Some(rec) = self.board.sessions.iter().find(|r| {
            r.id == id
                && r.kind == SessionKind::Claude
                && r.state.has_pane()
                && r.pane_key.is_some()
                && !r.observe_only()
                && r.state != (SessionState::Idle { stop_reason: StopReason::EndTurn })
        }) else {
            return sig;
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
                empty_since: None,
            },
        );
        self.pane_due.facts = true;
        self.pane_due.hook_rows = true;
        // Keep the existing turn open. No delivered edge, automove, or
        // crown turn-end probe exists until the saved Stop is released.
        Signal::TurnStarted
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
            known.retain(|pid, born| {
                rows.iter().any(|r| {
                    r.pid == *pid
                        && r.ppid == pane.pane_pid
                        && same_process(r, *born)
                        && !r.argv.contains("<defunct>")
                        && !r.argv.starts_with('(')
                })
            });
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
                known.entry(row.pid).or_insert(born);
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
                pending.empty_since = None;
            } else {
                let empty = *pending.empty_since.get_or_insert(at);
                if at.saturating_sub(pending.stopped) >= SCAN_MS
                    && (!pending.saw_child || at.saturating_sub(empty) >= REWAKE_MS)
                {
                    ready.push((*id, None));
                }
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
        assert!(matches!(d.hold_async_stop(id, stop, &frame, None, 1_000), Signal::TurnStarted));
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

    #[test]
    fn a_reused_pid_does_not_inherit_a_previous_hooks_lifetime() {
        let row = HookProcess { pid: 10, ppid: 1, pgid: 10, born_ms: 5_000, argv: "review".into() };
        assert!(same_process(&row, 5_010));
        assert!(!same_process(&row, 1_000));
    }
}
