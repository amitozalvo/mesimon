//! `reconcile(records, snapshot) -> links` — a pure function, written and tested
//! before any daemon code (D24). Matches persisted session records against what
//! the private tmux server actually holds after a daemon restart.
//!
//! v0.1 match chain is the primary key only: tmux session name == `sid16` of the
//! mesimon-minted UUID. The extended chain (branch → worktree → PR) arrives with
//! adoption in M3. Entities mesimon did not create are normal input, never errors.

use crate::board::{ExitReason, SessionRecord, SessionState, UnknownReason};

/// One pane row from `list-panes -a` on the private server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneSnapshot {
    pub session_name: String,
    pub pane_pid: i32,
    pub pane_dead: bool,
    /// Exit status when dead (tmux `#{pane_dead_status}`); None while alive.
    pub dead_status: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    /// Record matched a live pane.
    Live { session_name: String, pane_pid: i32 },
    /// Record matched a dead pane held by `remain-on-exit` — harvest status, then kill pane.
    DeadPane { session_name: String, status: i32 },
    /// Record has no pane at all (server restarted, pane reaped, or never spawned).
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reconciled {
    /// Per input record, in input order.
    pub links: Vec<(uuid::Uuid, Link)>,
    /// tmux sessions on our private server that no record claims.
    /// Normal input (D24) — surfaced, never treated as an error.
    pub foreign: Vec<String>,
}

/// The state a record should transition to for a given link.
/// `hook_instrumented`: the record's state is driven by a hook stream (a
/// Claude session). A bash session has no hooks — a live shell pane is
/// trivially `Running` and there is nothing stale about saying so.
pub fn state_for(link: &Link, prior: &SessionState, hook_instrumented: bool) -> SessionState {
    match link {
        Link::Live { .. } if hook_instrumented => match prior {
            // A live pane proves a process, nothing more. A persisted
            // RequiresAction / Idle / Throttled / Failed claim survives — it is
            // sticky by design and the hook stream picks up from where it left
            // off (needs-you additionally has the 15-min stale demote). But an
            // ACTIVITY claim (`Running` / `Spawning`) cannot be trusted across
            // daemon downtime: the `Stop` that ended the turn may have fired
            // into the void, and there is no polling to ever correct it
            // (dogfood 2026-08-30: sessions read "working" forever). Honest
            // answer until the next hook event: unknown, daemon restarted.
            SessionState::Running
            | SessionState::Spawning
            | SessionState::Exited { .. }
            | SessionState::Unknown { .. }
            | SessionState::Sleeping => {
                SessionState::Unknown { reason: UnknownReason::DaemonRestarted }
            }
            s => s.clone(),
        },
        Link::Live { .. } => match prior {
            // No hook stream: the pane is the whole truth.
            SessionState::Exited { .. } | SessionState::Unknown { .. } | SessionState::Sleeping => {
                SessionState::Running
            }
            s => s.clone(),
        },
        Link::DeadPane { status, .. } => match prior {
            // Already recorded as exited: keep the richer reason (publish-once).
            s @ SessionState::Exited { .. } => s.clone(),
            // A sleeping session's own kill left this pane behind (daemon died
            // mid-reap): harvest the pane, the record stays asleep.
            SessionState::Sleeping => SessionState::Sleeping,
            _ => SessionState::Exited {
                reason: if *status == 0 { ExitReason::UserQuit } else { ExitReason::Crashed },
            },
        },
        Link::Missing => match prior {
            // Already recorded as exited: keep the richer reason.
            s @ SessionState::Exited { .. } => s.clone(),
            // No pane IS the sleeping condition — a daemon restart must not
            // demote every sleeping session to Crashed.
            SessionState::Sleeping => SessionState::Sleeping,
            _ => SessionState::Exited { reason: ExitReason::Crashed },
        },
    }
}

pub fn reconcile(records: &[SessionRecord], snapshot: &[PaneSnapshot]) -> Reconciled {
    let mut claimed = vec![false; snapshot.len()];
    let mut links = Vec::with_capacity(records.len());

    for rec in records {
        let sid = rec.sid16();
        let found = snapshot.iter().position(|p| p.session_name == sid);
        let link = match found {
            Some(i) => {
                claimed[i] = true;
                let p = &snapshot[i];
                if p.pane_dead {
                    Link::DeadPane {
                        session_name: p.session_name.clone(),
                        status: p.dead_status.unwrap_or(-1),
                    }
                } else {
                    Link::Live { session_name: p.session_name.clone(), pane_pid: p.pane_pid }
                }
            }
            None => Link::Missing,
        };
        links.push((rec.id, link));
    }

    let foreign = snapshot
        .iter()
        .zip(&claimed)
        .filter(|(_, c)| !**c)
        .map(|(p, _)| p.session_name.clone())
        .collect();

    Reconciled { links, foreign }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{SessionKind, SessionRecord, SessionState, UnknownReason};

    fn rec(state: SessionState) -> SessionRecord {
        SessionRecord::new(
            uuid::Uuid::new_v4(),
            SessionKind::Claude,
            ulid::Ulid::new(),
            vec!["claude".into()],
            "/tmp".into(),
            state,
        )
    }

    fn pane(name: &str, dead: bool, status: Option<i32>) -> PaneSnapshot {
        PaneSnapshot {
            session_name: name.into(),
            pane_pid: 42,
            pane_dead: dead,
            dead_status: status,
        }
    }

    #[test]
    fn live_pane_is_running_only_without_hooks() {
        let r = rec(SessionState::unknown());
        let out = reconcile(std::slice::from_ref(&r), &[pane(&r.sid16(), false, None)]);
        assert_eq!(out.links.len(), 1);
        let link = &out.links[0].1;
        assert!(matches!(link, Link::Live { .. }));
        // Bash (no hook stream): the live pane is the whole truth.
        assert_eq!(state_for(link, &r.state, false), SessionState::Running);
        // Claude: a pane proves a process, not activity.
        assert_eq!(
            state_for(link, &r.state, true),
            SessionState::Unknown { reason: UnknownReason::DaemonRestarted }
        );
        assert!(out.foreign.is_empty());
    }

    #[test]
    fn stale_running_claim_demotes_across_restart() {
        // Dogfood 2026-08-30: a Stop that fired while the daemon was down left
        // "working" latched forever — restart must not trust activity claims.
        for prior in [SessionState::Running, SessionState::Spawning] {
            let r = rec(prior.clone());
            let out = reconcile(std::slice::from_ref(&r), &[pane(&r.sid16(), false, None)]);
            assert_eq!(
                state_for(&out.links[0].1, &r.state, true),
                SessionState::Unknown { reason: UnknownReason::DaemonRestarted },
                "{prior:?}"
            );
            // A bash session's Running is trivially true while the pane lives.
            assert_eq!(state_for(&out.links[0].1, &r.state, false), prior);
        }
    }

    #[test]
    fn dead_pane_yields_exit_reason() {
        let r = rec(SessionState::Running);
        let out = reconcile(std::slice::from_ref(&r), &[pane(&r.sid16(), true, Some(7))]);
        let link = &out.links[0].1;
        assert!(matches!(link, Link::DeadPane { status: 7, session_name: _ }));
        assert_eq!(
            state_for(link, &r.state, true),
            SessionState::Exited { reason: ExitReason::Crashed }
        );
    }

    #[test]
    fn clean_dead_pane_is_user_quit() {
        let r = rec(SessionState::Running);
        let out = reconcile(std::slice::from_ref(&r), &[pane(&r.sid16(), true, Some(0))]);
        assert_eq!(
            state_for(&out.links[0].1, &r.state, true),
            SessionState::Exited { reason: ExitReason::UserQuit }
        );
    }

    #[test]
    fn missing_pane_is_crashed_unless_already_exited() {
        let r = rec(SessionState::Running);
        let out = reconcile(std::slice::from_ref(&r), &[]);
        assert!(matches!(out.links[0].1, Link::Missing));
        assert_eq!(
            state_for(&out.links[0].1, &r.state, true),
            SessionState::Exited { reason: ExitReason::Crashed }
        );

        let r2 = rec(SessionState::Exited { reason: ExitReason::UserQuit });
        let out2 = reconcile(std::slice::from_ref(&r2), &[]);
        assert_eq!(
            state_for(&out2.links[0].1, &r2.state, true),
            SessionState::Exited { reason: ExitReason::UserQuit }
        );
    }

    #[test]
    fn live_pane_preserves_requires_action() {
        // A daemon restart must not wipe a persisted needs-you state while the
        // pane is still alive — the hook stream resumes from there.
        let state = SessionState::RequiresAction { reason: crate::board::Reason::Permission };
        let r = rec(state.clone());
        let out = reconcile(std::slice::from_ref(&r), &[pane(&r.sid16(), false, None)]);
        assert_eq!(state_for(&out.links[0].1, &r.state, true), state);
    }

    #[test]
    fn dead_pane_keeps_richer_exit_reason() {
        let state = SessionState::Exited { reason: ExitReason::Killed };
        let r = rec(state.clone());
        let out = reconcile(std::slice::from_ref(&r), &[pane(&r.sid16(), true, Some(1))]);
        assert_eq!(state_for(&out.links[0].1, &r.state, true), state);
    }

    #[test]
    fn sleeping_survives_restart_and_dead_pane() {
        // Missing: that IS the sleeping condition.
        let r = rec(SessionState::Sleeping);
        let out = reconcile(std::slice::from_ref(&r), &[]);
        assert!(matches!(out.links[0].1, Link::Missing));
        assert_eq!(state_for(&out.links[0].1, &r.state, true), SessionState::Sleeping);

        // DeadPane: daemon died between SIGTERM and kill-session — harvest, stay asleep.
        let out = reconcile(std::slice::from_ref(&r), &[pane(&r.sid16(), true, Some(0))]);
        assert_eq!(state_for(&out.links[0].1, &r.state, true), SessionState::Sleeping);

        // Live: a pane proves a process; the sleep never completed.
        let out = reconcile(std::slice::from_ref(&r), &[pane(&r.sid16(), false, None)]);
        assert_eq!(
            state_for(&out.links[0].1, &r.state, true),
            SessionState::Unknown { reason: UnknownReason::DaemonRestarted }
        );
    }

    #[test]
    fn foreign_sessions_surface_without_error() {
        let r = rec(SessionState::Running);
        let out = reconcile(
            std::slice::from_ref(&r),
            &[pane(&r.sid16(), false, None), pane("handmade", false, None)],
        );
        assert_eq!(out.foreign, vec!["handmade".to_string()]);
    }

    #[test]
    fn no_duplicate_claims() {
        // Two records can never share a sid16 (UUIDs), but a snapshot could hold
        // duplicates from a corrupt server — first claim wins, second surfaces foreign.
        let r = rec(SessionState::Running);
        let out = reconcile(
            std::slice::from_ref(&r),
            &[pane(&r.sid16(), false, None), pane(&r.sid16(), true, Some(1))],
        );
        assert!(matches!(out.links[0].1, Link::Live { .. }));
        assert_eq!(out.foreign, vec![r.sid16()]);
    }
}
