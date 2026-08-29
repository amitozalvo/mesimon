//! `reconcile(records, snapshot) -> links` — a pure function, written and tested
//! before any daemon code (D24). Matches persisted session records against what
//! the private tmux server actually holds after a daemon restart.
//!
//! v0.1 match chain is the primary key only: tmux session name == `sid16` of the
//! mesimon-minted UUID. The extended chain (branch → worktree → PR) arrives with
//! adoption in M3. Entities mesimon did not create are normal input, never errors.

use crate::board::{ExitReason, SessionRecord, SessionState};

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
pub fn state_for(link: &Link, prior: &SessionState) -> SessionState {
    match link {
        Link::Live { .. } => match prior {
            // A live pane proves the process exists; anything the record thought
            // beyond that is stale after a restart. A persisted RequiresAction /
            // Idle / Throttled survives — the pane is still there and the hook
            // stream picks up from where it left off. A pane under a Sleeping
            // record means the sleep never finished killing — the process is real.
            SessionState::Exited { .. }
            | SessionState::Unknown { .. }
            | SessionState::Sleeping => SessionState::Running,
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
                    Link::DeadPane { session_name: p.session_name.clone(), status: p.dead_status.unwrap_or(-1) }
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
    use crate::board::{SessionKind, SessionRecord, SessionState};

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
        PaneSnapshot { session_name: name.into(), pane_pid: 42, pane_dead: dead, dead_status: status }
    }

    #[test]
    fn live_pane_links_and_runs() {
        let r = rec(SessionState::unknown());
        let out = reconcile(&[r.clone()], &[pane(&r.sid16(), false, None)]);
        assert_eq!(out.links.len(), 1);
        let link = &out.links[0].1;
        assert!(matches!(link, Link::Live { .. }));
        assert_eq!(state_for(link, &r.state), SessionState::Running);
        assert!(out.foreign.is_empty());
    }

    #[test]
    fn dead_pane_yields_exit_reason() {
        let r = rec(SessionState::Running);
        let out = reconcile(&[r.clone()], &[pane(&r.sid16(), true, Some(7))]);
        let link = &out.links[0].1;
        assert!(matches!(link, Link::DeadPane { status: 7, session_name: _ }));
        assert_eq!(
            state_for(link, &r.state),
            SessionState::Exited { reason: ExitReason::Crashed }
        );
    }

    #[test]
    fn clean_dead_pane_is_user_quit() {
        let r = rec(SessionState::Running);
        let out = reconcile(&[r.clone()], &[pane(&r.sid16(), true, Some(0))]);
        assert_eq!(
            state_for(&out.links[0].1, &r.state),
            SessionState::Exited { reason: ExitReason::UserQuit }
        );
    }

    #[test]
    fn missing_pane_is_crashed_unless_already_exited() {
        let r = rec(SessionState::Running);
        let out = reconcile(&[r.clone()], &[]);
        assert!(matches!(out.links[0].1, Link::Missing));
        assert_eq!(
            state_for(&out.links[0].1, &r.state),
            SessionState::Exited { reason: ExitReason::Crashed }
        );

        let r2 = rec(SessionState::Exited { reason: ExitReason::UserQuit });
        let out2 = reconcile(&[r2.clone()], &[]);
        assert_eq!(
            state_for(&out2.links[0].1, &r2.state),
            SessionState::Exited { reason: ExitReason::UserQuit }
        );
    }

    #[test]
    fn live_pane_preserves_requires_action() {
        // A daemon restart must not wipe a persisted needs-you state while the
        // pane is still alive — the hook stream resumes from there.
        let state = SessionState::RequiresAction { reason: crate::board::Reason::Permission };
        let r = rec(state.clone());
        let out = reconcile(&[r.clone()], &[pane(&r.sid16(), false, None)]);
        assert_eq!(state_for(&out.links[0].1, &r.state), state);
    }

    #[test]
    fn dead_pane_keeps_richer_exit_reason() {
        let state = SessionState::Exited { reason: ExitReason::Killed };
        let r = rec(state.clone());
        let out = reconcile(&[r.clone()], &[pane(&r.sid16(), true, Some(1))]);
        assert_eq!(state_for(&out.links[0].1, &r.state), state);
    }

    #[test]
    fn sleeping_survives_restart_and_dead_pane() {
        // Missing: that IS the sleeping condition.
        let r = rec(SessionState::Sleeping);
        let out = reconcile(&[r.clone()], &[]);
        assert!(matches!(out.links[0].1, Link::Missing));
        assert_eq!(state_for(&out.links[0].1, &r.state), SessionState::Sleeping);

        // DeadPane: daemon died between SIGTERM and kill-session — harvest, stay asleep.
        let out = reconcile(&[r.clone()], &[pane(&r.sid16(), true, Some(0))]);
        assert_eq!(state_for(&out.links[0].1, &r.state), SessionState::Sleeping);

        // Live: a pane proves a process; the sleep never completed.
        let out = reconcile(&[r.clone()], &[pane(&r.sid16(), false, None)]);
        assert_eq!(state_for(&out.links[0].1, &r.state), SessionState::Running);
    }

    #[test]
    fn foreign_sessions_surface_without_error() {
        let r = rec(SessionState::Running);
        let out = reconcile(
            &[r.clone()],
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
            &[r.clone()],
            &[pane(&r.sid16(), false, None), pane(&r.sid16(), true, Some(1))],
        );
        assert!(matches!(out.links[0].1, Link::Live { .. }));
        assert_eq!(out.foreign, vec![r.sid16()]);
    }
}
