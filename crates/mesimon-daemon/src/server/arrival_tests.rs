//! A finished forge arrival waits for the ticket's own work (T-1275): a
//! merge or a deployment reached while the ticket's agent works, or while
//! a leg's branch is past what the set carries, moves no card until that
//! stops, then moves it on the edge it came with; an open PR of the set
//! holds it by the edge itself.

use mesimon_core::board::{Arrival, Column, SessionState, StopReason};
use mesimon_core::forge::{Forge, Pipeline, Pr, PrState, Remote};

use crate::server::crownask::ledger_tests::{bed, Bed};
use crate::worktree::RepoFlags;

fn pr(number: u64, state: PrState, head: &str, deployed: &[&str]) -> Pr {
    Pr {
        forge: Forge::GitHub,
        repo: "o/r".into(),
        number,
        title: String::new(),
        url: format!("https://github.com/o/r/pull/{number}"),
        state,
        head_oid: head.into(),
        head_ref: "b".into(),
        target_ref: "main".into(),
        merge_oid: if state == PrState::Merged { format!("m{number}") } else { String::new() },
        conflict: false,
        review: Default::default(),
        pipeline: Pipeline::Passed,
        pipeline_run: 0,
        updated_ms: number,
        merged_ms: number,
        deployed: deployed.iter().map(|e| e.to_string()).collect(),
        landed_tips: Vec::new(),
        reviewed: false,
    }
}

/// The bed's board as a forge board: IN PROGRESS, REVIEW, MERGED on a
/// merge, PRODUCTION on a deploy there, DONE; the worker in IN PROGRESS,
/// its agent working (the bed's), its one leg the root, a checkout of
/// `o/r` at `tip`.
fn forge_bed(name: &str, tip: &str) -> Bed {
    let mut b = bed(&format!("arrival-{name}"));
    let col = |name: &str, order: &str, on: Arrival, env: Option<&str>| {
        let mut c = Column::new(name, order);
        c.settings.arrives_on = on;
        c.settings.arrives_env = env.map(str::to_string);
        c
    };
    b.d.board.columns = vec![
        col("TODO", "a", Arrival::Nothing, None),
        col("IN PROGRESS", "b", Arrival::Nothing, None),
        col("REVIEW", "c", Arrival::Nothing, None),
        col("MERGED", "d", Arrival::Merged, None),
        col("PRODUCTION", "e", Arrival::Deployed, Some("production")),
        col("DONE", "f", Arrival::Nothing, None),
    ];
    let w = b.worker;
    b.d.board.ticket_mut(w).unwrap().column = "IN PROGRESS".into();
    let root = b.d.paths.repo_root.clone();
    b.d.forge.remotes.insert(root, Some(Remote { host: "github.com".into(), path: "o/r".into() }));
    let leg = RepoFlags {
        base_oid: "base".into(),
        tip: tip.into(),
        base: "main".into(),
        ..Default::default()
    };
    b.d.wt_repos.insert(w, vec![leg]);
    b
}

/// An archive keeps the last settled PRs for its card. An unfinished
/// sibling still gets the full read, as does the ticket after restoration.
#[test]
fn the_forge_skips_only_settled_archives() {
    use crate::forge::{Cli, HostProbe};
    use crate::worktree::{Binding, BindingStatus};
    use mesimon_core::board::Archived;
    let mut b = forge_bed("archive-query", "head");
    let w = b.worker;
    let dir = b.d.paths.repo_root.join("worktree");
    b.d.worktrees.insert(
        w,
        Binding::new(
            dir.clone(),
            "msmn/T-2".into(),
            "base".into(),
            "head".into(),
            BindingStatus::Attached,
        ),
    );
    b.d.forge.probes.insert(
        "github.com".into(),
        HostProbe {
            cli: Some(Cli { forge: Forge::GitHub, bin: "gh".into() }),
            at: std::time::Instant::now(),
            viewer: None,
            lost: None,
        },
    );
    b.d.board.ticket_mut(w).unwrap().archived =
        Some(Archived { at: "@0".into(), by: "local".into(), until: None, needs_you: false });
    assert!(b.d.forge_query(None).is_some(), "unread archive");
    let merged = pr(1, PrState::Merged, "h1", &[]);
    b.d.forge.sets.insert(w, vec![merged.clone(), pr(2, PrState::Open, "h2", &[])]);
    b.d.forge.at_ms.insert(w, 1);
    assert!(b.d.forge_query(None).is_some(), "one PR is still open");
    b.d.forge.sets.insert(w, vec![merged.clone()]);
    assert!(b.d.forge_query(None).is_none());
    assert!(!b.d.forge.reflogs.contains_key(&dir));
    assert_eq!(b.d.forge.sets[&w], [merged], "the archive still shows its PR");
    b.d.forge.owed.insert(w, 2);
    assert!(b.d.forge_query(None).is_some(), "an explicit read is still owed");
    b.d.forge.owed.clear();
    b.d.board.ticket_mut(w).unwrap().archived = None;
    assert!(b.d.forge_query(None).is_some(), "restored ticket");
}

fn column(b: &Bed) -> String {
    b.d.board.ticket(b.worker).unwrap().column.clone()
}

fn quiet(b: &mut Bed) {
    b.d.board.sessions[0].state = SessionState::Idle { stop_reason: StopReason::EndTurn };
}

/// The pass's edge for `set`, read as the board would against `old`.
fn arrive(b: &mut Bed, old: &[Pr], set: Vec<Pr>) -> bool {
    let w = b.worker;
    let edge = crate::forge::edge(old, &set, &[], true);
    b.d.forge.sets.insert(w, set);
    b.d.forge_arrivals(vec![(w, edge)])
}

/// The simbly board's T-467: the first MR reached production while the
/// worker's second turn ran. The card stays; the turn's end releases the
/// held deployment on the next pass, with no second deployment edge.
#[test]
fn a_deploy_reached_while_the_agent_works_moves_the_card_once_its_turn_ends() {
    let mut b = forge_bed("working", "h1");
    let merged = vec![pr(1, PrState::Merged, "h1", &[])];
    b.d.forge.sets.insert(b.worker, merged.clone());
    let deployed = vec![pr(1, PrState::Merged, "h1", &["production"])];
    assert!(!arrive(&mut b, &merged, deployed));
    assert_eq!(column(&b), "IN PROGRESS");
    assert_eq!(b.d.arrival_stay(b.worker), Some("agent working"));
    let held = b.d.forge.deferred.get(&b.worker).cloned().unwrap_or_default();
    assert_eq!(held.deployed, ["production"], "{held:?}");
    // Still working: a pass releases nothing.
    assert!(!b.d.release_arrivals());
    assert_eq!(column(&b), "IN PROGRESS");
    // The turn ended and the board put the card in REVIEW.
    quiet(&mut b);
    b.d.board.ticket_mut(b.worker).unwrap().column = "REVIEW".into();
    assert!(b.d.release_arrivals());
    assert_eq!(column(&b), "PRODUCTION");
    assert!(b.d.forge.deferred.is_empty());
    // The hold survives a restart: it is in `forge.json`.
    let mut again = forge_bed("working-file", "h1");
    again.d.forge.sets.insert(again.worker, vec![pr(1, PrState::Merged, "h1", &["production"])]);
    again.d.forge.deferred.insert(again.worker, held);
    let file = crate::forge::ForgeFile::of(&again.d.forge);
    let text = serde_json::to_string(&file).unwrap();
    let back: crate::forge::ForgeFile = serde_json::from_str(&text).unwrap();
    let mut state = crate::forge::State::default();
    state.restore(back, |_| true);
    assert_eq!(
        state.deferred.get(&again.worker).map(|d| d.deployed.clone()),
        Some(vec!["production".to_string()])
    );
}

/// A later PR opened while the deployment was held: the set no longer
/// stands on it, so the turn's end drops it and the card stays; that PR's
/// own merge and deployment are edges of their own and move the card.
#[test]
fn a_held_deploy_the_set_moved_past_is_dropped_and_the_later_prs_own_deploy_moves_the_card() {
    let mut b = forge_bed("dropped", "h1");
    let merged = vec![pr(1, PrState::Merged, "h1", &[])];
    let deployed = vec![pr(1, PrState::Merged, "h1", &["production"])];
    assert!(!arrive(&mut b, &merged, deployed.clone()));
    // !2 opened from the branch's new tip, then the turn ended.
    let mut opened = deployed.clone();
    opened.insert(0, pr(2, PrState::Open, "h2", &[]));
    b.d.wt_repos.get_mut(&b.worker).unwrap()[0].tip = "h2".into();
    assert!(!arrive(&mut b, &deployed, opened.clone()));
    quiet(&mut b);
    assert!(b.d.release_arrivals(), "the hold went");
    assert_eq!(column(&b), "IN PROGRESS");
    assert!(b.d.forge.deferred.is_empty());
    // !2 merged and deployed: its own edges take the card.
    let mut done = opened.clone();
    done[0] = pr(2, PrState::Merged, "h2", &["production"]);
    assert!(arrive(&mut b, &opened, done));
    assert_eq!(column(&b), "PRODUCTION");
}

/// A leg's branch past the set — committed work no PR carries — holds a
/// merge and a deployment though no agent works; the branch caught up (the
/// work went into a PR that merged at its tip), the hold goes.
#[test]
fn a_branch_ahead_of_the_set_holds_the_arrival_until_its_work_lands() {
    let mut b = forge_bed("ahead", "h2");
    quiet(&mut b);
    let open = vec![pr(1, PrState::Open, "h1", &[])];
    let deployed = vec![pr(1, PrState::Merged, "h1", &["production"])];
    assert!(!arrive(&mut b, &open, deployed.clone()));
    assert_eq!(column(&b), "IN PROGRESS");
    assert_eq!(b.d.arrival_stay(b.worker), Some("branch ahead"));
    let held = b.d.forge.deferred.get(&b.worker).cloned().unwrap_or_default();
    assert!(held.merged && held.deployed == ["production"], "{held:?}");
    assert!(!b.d.release_arrivals());
    // At the merged head, the set carries the branch: the arrival is its.
    b.d.wt_repos.get_mut(&b.worker).unwrap()[0].tip = "h1".into();
    assert_eq!(b.d.arrival_stay(b.worker), None);
    assert!(b.d.release_arrivals());
    assert_eq!(column(&b), "PRODUCTION");
    // A ticket whose branch is at the merged head moves at once.
    let mut c = forge_bed("caught-up", "h1");
    quiet(&mut c);
    assert!(arrive(&mut c, &open, deployed));
    assert_eq!(column(&c), "PRODUCTION");
}

/// An open PR of the set is a hold already (T-768): the set is not merged
/// and reached nowhere, so the first PR's deployment is no edge at all.
#[test]
fn an_open_sibling_pr_holds_the_deploy_by_the_edge_itself() {
    let mut b = forge_bed("sibling", "h1");
    quiet(&mut b);
    let before = vec![pr(2, PrState::Open, "h2", &[]), pr(1, PrState::Merged, "h1", &[])];
    let after =
        vec![pr(2, PrState::Open, "h2", &[]), pr(1, PrState::Merged, "h1", &["production"])];
    let edge = crate::forge::edge(&before, &after, &[], true);
    assert!(!edge.merged && edge.deployed.is_empty(), "{edge:?}");
    assert!(!arrive(&mut b, &before, after));
    assert_eq!(column(&b), "IN PROGRESS");
    assert!(b.d.forge.deferred.is_empty(), "nothing to hold");
}

/// A held merge released after the card went on past its column never
/// takes the card back, and an opening still moves a card whose finished
/// arrival is held.
#[test]
fn a_released_arrival_only_moves_the_card_right() {
    let mut b = forge_bed("right", "h1");
    let open = vec![pr(1, PrState::Open, "h1", &[])];
    let merged = vec![pr(1, PrState::Merged, "h1", &[])];
    assert!(!arrive(&mut b, &open, merged));
    let held = b.d.forge.deferred.get(&b.worker).cloned().unwrap_or_default();
    assert!(held.merged, "{held:?}");
    quiet(&mut b);
    b.d.board.ticket_mut(b.worker).unwrap().column = "DONE".into();
    assert!(b.d.release_arrivals(), "the hold went");
    assert_eq!(column(&b), "DONE");
}
