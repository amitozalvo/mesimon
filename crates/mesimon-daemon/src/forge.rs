//! The forge sample (T-739): the daemon's half of `mesimon_core::forge`.
//!
//! On the slow bucket, at most once a minute, a worker thread asks the
//! forge CLI the person already signed in to — `gh` on github.com, `glab`
//! on a GitLab host, found on the captured shell `PATH` — for the pull
//! requests the signed-in account authored in each repository the board's
//! tickets touch: one list call per repository, never one per ticket. The
//! worker then binds each PR to a ticket by a fact (the head commit in a
//! worktree leg's reflog, the ticket's own branch, a URL a `pr create` of
//! the ticket's own session printed) and hands the writer thread each
//! ticket's set. Reads only: no `create`, no merge, no comment, no push,
//! and the CLI's own update notifier and prompts are off, so nothing is
//! written anywhere on mesimon's account (promise 1).
//!
//! Every call is fenced as a fetch is (`gitstatus::fetch`): its own
//! session, a deadline, the process group killed on overrun, every prompt
//! door shut. A repository whose call failed is reported, not retried in a
//! loop: the next sample waits `BACKOFF`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use mesimon_core::command::Notice;
use mesimon_core::forge::{self, Facts, Forge, Pipeline, Pr, PrState, Remote, Review};
use serde::{Deserialize, Serialize};

use crate::paths::Paths;

/// How long one CLI call may take before its group is killed.
const CALL_TIMEOUT: Duration = Duration::from_secs(20);
/// The cadence between samples with every repository answering.
pub const EVERY: Duration = Duration::from_secs(60);
/// The cadence for a repository holding a PR whose pipeline runs or waits
/// (T-781): a deploy job can take seconds, and a minute's read never sees
/// it run.
pub const BUSY_EVERY: Duration = Duration::from_secs(15);
/// The cadence after a repository failed to answer.
pub const BACKOFF: Duration = Duration::from_secs(300);
/// The floor between two reads of one repository (T-761): a turn's end
/// asks its ticket's repositories at once, at most this often each.
pub const BURST_MS: u64 = 10_000;
/// How long a host's probe (which CLI, signed in or not) is believed.
const PROBE_FOR: Duration = Duration::from_secs(6 * 3600);
/// How many reflog entries a leg offers for binding.
const REFLOG_CAP: &str = "2000";
/// How many PRs one list call brings back.
const LIST_LIMIT: &str = "100";
/// How long a merge commit a newer one superseded is still read (T-771): a
/// day is past any deploy run, and the PR keeps what was read by then.
pub const SUPERSEDED_READ_MS: u64 = 24 * 3600 * 1000;

/// The CLI a host is read through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cli {
    pub forge: Forge,
    pub bin: PathBuf,
}

impl Cli {
    fn name(forge: Forge) -> &'static str {
        match forge {
            Forge::GitHub => "gh",
            Forge::GitLab => "glab",
        }
    }
}

/// `MESIMON_FORGE_EVERY_MS`: the sample cadence, a test seam.
pub fn every() -> Duration {
    std::env::var("MESIMON_FORGE_EVERY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .map(Duration::from_millis)
        .unwrap_or(EVERY)
}

/// `MESIMON_FORGE_BUSY_MS`: the busy cadence, a test seam.
pub fn busy_every() -> Duration {
    std::env::var("MESIMON_FORGE_BUSY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .map(Duration::from_millis)
        .unwrap_or(BUSY_EVERY)
}

/// `gh` or `glab` on the captured shell `PATH`, else the daemon's own.
pub fn find_cli(forge: Forge, captured_path: Option<&str>) -> Option<Cli> {
    let name = Cli::name(forge);
    let on = |var: &str| std::env::split_paths(var).map(|d| d.join(name)).find(|p| p.is_file());
    let bin =
        captured_path.and_then(on).or_else(|| std::env::var("PATH").ok().and_then(|p| on(&p)))?;
    Some(Cli { forge, bin })
}

/// A host's probe as the daemon keeps it: which CLI reads it, if any, and
/// when that was decided.
#[derive(Debug, Clone)]
pub struct HostProbe {
    pub cli: Option<Cli>,
    pub at: Instant,
    /// The signed-in account's username, where the CLI needs it on the
    /// command line (glab's `--author`).
    pub viewer: Option<String>,
}

impl HostProbe {
    pub fn stale(&self) -> bool {
        self.at.elapsed() >= PROBE_FOR
    }
}

/// One repository to list: where it is and how it is read.
#[derive(Debug, Clone)]
pub struct RepoQuery {
    pub remote: Remote,
    pub cli: Cli,
    pub viewer: Option<String>,
    /// A directory the CLI runs in; any checkout of it.
    pub dir: PathBuf,
}

impl RepoQuery {
    pub fn key(&self) -> String {
        format!("{}/{}", self.remote.host, self.remote.path)
    }
}

/// One leg of a ticket's worktree: the repository it is a checkout of, by
/// the remote's path, and where it stands.
#[derive(Debug, Clone)]
pub struct LegQuery {
    pub repo_path: String,
    pub dir: PathBuf,
}

/// One ticket's facts, to bind by.
#[derive(Debug, Clone)]
pub struct TicketQuery {
    pub ticket: ulid::Ulid,
    pub branch: String,
    pub legs: Vec<LegQuery>,
    pub opened: Vec<String>,
    /// Every PR, by repository and number, the ticket's reads held before
    /// (`State::held`): an MR outside it is read whole on this pass (T-784).
    pub held: Vec<(String, u64)>,
}

/// A host to probe on this pass: the candidate CLIs in the order to try.
#[derive(Debug, Clone)]
pub struct ProbeQuery {
    pub host: String,
    pub candidates: Vec<Cli>,
}

/// A repository whose host is probed on this same pass (T-752): listed
/// once the probe found its CLI, so a fresh daemon's first pass lists too.
#[derive(Debug, Clone)]
pub struct PendingRepo {
    pub remote: Remote,
    pub dir: PathBuf,
}

/// What one pass is asked.
#[derive(Debug, Clone, Default)]
pub struct Query {
    pub probes: Vec<ProbeQuery>,
    pub repos: Vec<RepoQuery>,
    /// The repositories of the hosts in `probes`, listed after the probe.
    pub pending: Vec<PendingRepo>,
    pub tickets: Vec<TicketQuery>,
    /// The environments some column waits for: a deployment is read only
    /// where one is asked for.
    pub envs: Vec<String>,
    /// Merge commits' pipelines as earlier passes read them, by repository
    /// path and oid: a passed or failed one is not asked again.
    pub commit_pipelines: HashMap<(String, String), Pipeline>,
    /// The environments each merge commit was seen reaching: a deployment
    /// is permanent, so one is read until every wanted environment is in.
    pub deployed: HashMap<(String, String), Vec<String>>,
}

/// What a pass saw change in a ticket's set (T-739): the edges a column's
/// `Arrives here when` fires on, and the ones the crown hears (T-744). Read
/// against the previous read, never a first one, which is a baseline.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Edge {
    /// Open PRs, by label, its ticket's reads never held (T-767): an
    /// opening, never a re-read.
    pub opened: Vec<String>,
    /// Newly held open PRs, by label, bound with a settled pipeline:
    /// passed, failed or waiting (T-769). A PR opened and judged between
    /// two reads says that state as its first edge.
    pub bound: Vec<(String, Pipeline)>,
    /// The worst pipeline over the open PRs turned passed (T-767): what
    /// `requires_pipeline` waits for, never a merged commit's pipeline,
    /// which is the deploy edges' (`waiting`, `deployed`).
    pub passed: bool,
    /// Every PR of the set is merged, and was not before: the ticket's
    /// arrival, on every repository at once (T-768). A closed PR is out of
    /// the set (`forge::ticket_set`).
    pub merged: bool,
    /// PRs, by label, read merged that were read open before (T-757): what
    /// the crown hears of a merge the worktree flags cannot say, the caller
    /// keeping them only then.
    pub landed: Vec<String>,
    /// Environments the ticket reached since, as a whole (T-768): no PR
    /// open, and every repository's newest merged PR's merge commit there.
    /// One repository's progress is not the ticket's.
    pub deployed: Vec<String>,
    /// PRs, by label, whose merge commit reached an environment since, with
    /// the environment: what the crown hears, per PR, before the ticket
    /// has arrived.
    pub reached: Vec<(String, String)>,
    /// The set's pipeline turned failed: the job's word, empty where the
    /// forge named none.
    pub failed: Option<String>,
    /// The open PRs' pipeline stopped on a person at a job it was not
    /// stopped on before: the job's word, empty where the forge named none.
    /// A merged PR's wait is its own (`stopped`).
    pub waiting: Option<String>,
    /// Merged PRs, by label, whose merge commit's pipeline stopped on a
    /// person at a job it was not stopped on before, with the job (T-782):
    /// said per PR, whatever the ticket's other PRs are doing, as a
    /// deployment is (`reached`).
    pub stopped: Vec<(String, String)>,
    /// Open PRs whose conflict appeared or cleared, and newly held ones
    /// bound in conflict (T-769). The caller keeps a clear only where its
    /// conflict was said (`State::say_conflicts`).
    pub conflict: Vec<Conflict>,
    /// Open PRs, by label, a reviewer's verdict newly landed on, a newly
    /// held one's included (T-769): approved or changes requested.
    pub review: Vec<(String, Review)>,
}

/// A conflict edge (T-744): which PR, and whether it appeared or cleared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub repo: String,
    pub number: u64,
    pub label: String,
    pub on: bool,
    /// The PR is newly held, so its conflict is part of no earlier read.
    pub fresh: bool,
}

impl Edge {
    pub fn any(&self) -> bool {
        !self.opened.is_empty()
            || !self.bound.is_empty()
            || self.passed
            || self.merged
            || !self.landed.is_empty()
            || !self.deployed.is_empty()
            || !self.reached.is_empty()
            || self.failed.is_some()
            || self.waiting.is_some()
            || !self.stopped.is_empty()
            || !self.conflict.is_empty()
            || !self.review.is_empty()
    }

    /// What the crown hears of the edge (T-744), one phrase per edge, each
    /// naming its PR: `merged !2`, `pipeline passed !1`, `waiting on
    /// deploy:production !1`, `deployed to staging !1`, `conflict !1`,
    /// `conflict cleared !1`, `review approved !1`, `changes requested !1`,
    /// `pipeline failed !1: test-linux`, in the forge's order: merged, then
    /// the pipelines, then deployed, then waiting (T-769, T-782) — a merge
    /// commit's deployment ran before the job its pipeline now waits on.
    /// Never an opening, which wakes
    /// no one by itself: the worker's turn end carries it (`opening`,
    /// T-773). A newly bound PR's settled state is said (T-769). A merge is said only where the caller left it in `landed`:
    /// the flags' merged wake says the one that lands the worktree's branch
    /// (T-757). `new` is the read the edge was taken on, for the labels.
    /// The openings in the crown's words (T-773): `opened !2`, said only
    /// on the wake of the turn that opened them.
    pub fn opening(&self) -> Vec<String> {
        self.opened.iter().map(|l| format!("opened {l}")).collect()
    }

    pub fn news(&self, new: &[Pr]) -> Vec<String> {
        let labels = |pick: &dyn Fn(&Pr) -> bool| -> String {
            new.iter().filter(|p| pick(p)).map(Pr::label).collect::<Vec<_>>().join(" ")
        };
        let tail =
            |label: String| if label.is_empty() { String::new() } else { format!(" {label}") };
        let mut out: Vec<String> = self.landed.iter().map(|l| format!("merged {l}")).collect();
        if let Some(job) = &self.failed {
            let label = labels(&|p| matches!(p.pipeline, Pipeline::Failed(_)));
            match forge::job_word(job) {
                j if j.is_empty() => out.push(format!("pipeline failed{}", tail(label))),
                j => out.push(format!("pipeline failed{}: {j}", tail(label))),
            }
        }
        if self.passed {
            out.push(format!("pipeline passed{}", tail(labels(&|p| p.state.is_open()))));
        }
        // A newly bound PR's pipeline, where the set's own edge above did
        // not already name it.
        for (label, pipeline) in &self.bound {
            match pipeline {
                Pipeline::Passed if !self.passed => out.push(format!("pipeline passed {label}")),
                Pipeline::Failed(job) if self.failed.is_none() => match forge::job_word(job) {
                    j if j.is_empty() => out.push(format!("pipeline failed {label}")),
                    j => out.push(format!("pipeline failed {label}: {j}")),
                },
                _ => {}
            }
        }
        for (label, env) in &self.reached {
            out.push(format!("deployed to {} {label}", forge::job_word(env)));
        }
        let wait = |job: &str, label: String| match forge::job_word(job) {
            j if j.is_empty() => format!("waiting on a manual job{}", tail(label)),
            j => format!("waiting on {j}{}", tail(label)),
        };
        if let Some(job) = &self.waiting {
            out.push(wait(
                job,
                labels(&|p| p.state.is_open() && matches!(p.pipeline, Pipeline::Waiting(_))),
            ));
        }
        for (label, pipeline) in &self.bound {
            if let Pipeline::Waiting(job) = pipeline {
                if self.waiting.is_none() {
                    out.push(wait(job, label.clone()));
                }
            }
        }
        for (label, job) in &self.stopped {
            out.push(wait(job, label.clone()));
        }
        for c in &self.conflict {
            out.push(if c.on {
                format!("conflict {}", c.label)
            } else {
                format!("conflict cleared {}", c.label)
            });
        }
        for (label, verdict) in &self.review {
            match verdict {
                Review::Approved => out.push(format!("review approved {label}")),
                Review::ChangesRequested => out.push(format!("changes requested {label}")),
                Review::None | Review::Required => {}
            }
        }
        out
    }
}

/// The environments a ticket's set has reached as a whole (T-768): none
/// while a PR is open, else those every repository's newest merged PR's
/// merge commit reached, an older merge there being in its history; an
/// older PR's own deployments are its row's and the crown's (T-771).
fn arrived_envs(set: &[Pr]) -> Vec<String> {
    if set.iter().any(|p| p.state.is_open()) {
        return Vec::new();
    }
    let mut newest: Vec<&Pr> = Vec::new();
    for p in set.iter().filter(|p| p.state == PrState::Merged) {
        if !newest.iter().any(|n| n.repo == p.repo) {
            newest.push(p);
        }
    }
    let Some((first, rest)) = newest.split_first() else { return Vec::new() };
    first.deployed.iter().filter(|e| rest.iter().all(|p| p.deployed.contains(e))).cloned().collect()
}

/// The edge between two reads of one ticket's set. `held` is every PR, by
/// repository and number, the ticket's reads held before this one
/// (`State::held`): an open PR outside it is newly bound, and brings its
/// settled state in as its first edge (T-769).
pub fn edge(old: &[Pr], new: &[Pr], held: &[(String, u64)]) -> Edge {
    let was = |p: &Pr| old.iter().find(|o| o.repo == p.repo && o.number == p.number);
    let fresh: Vec<&Pr> = new
        .iter()
        .filter(|p| p.state.is_open() && !held.iter().any(|(r, n)| *r == p.repo && *n == p.number))
        .collect();
    let all_merged = |s: &[Pr]| !s.is_empty() && s.iter().all(|p| p.state == PrState::Merged);
    let before = arrived_envs(old);
    let (then, now) = (forge::pipeline_of(old), forge::pipeline_of(new));
    // The open PRs' own wait; a merged PR's is said per PR (T-782).
    let open_of = |s: &[Pr]| {
        if s.iter().any(|p| p.state.is_open()) {
            forge::pipeline_of(s)
        } else {
            Pipeline::None
        }
    };
    let (then_open, now_open) = (open_of(old), open_of(new));
    // The open set's own: a merged commit's pipeline passing is its deploy
    // run finishing, never a ticket's pipeline passing (T-767).
    let open_passed =
        |s: &[Pr]| s.iter().any(|p| p.state.is_open()) && forge::pipeline_of(s) == Pipeline::Passed;
    // An open PR read open before too: its conflict and its review are
    // edges between the two reads. One newly bound says them as they stand.
    let kept: Vec<(&Pr, &Pr)> = new
        .iter()
        .filter(|p| p.state.is_open())
        .filter_map(|p| was(p).filter(|o| o.state.is_open()).map(|o| (o, p)))
        .collect();
    let verdict = |r: Review| matches!(r, Review::Approved | Review::ChangesRequested);
    Edge {
        opened: fresh.iter().map(|p| p.label()).collect(),
        bound: fresh
            .iter()
            .filter(|p| {
                matches!(p.pipeline, Pipeline::Passed | Pipeline::Failed(_) | Pipeline::Waiting(_))
            })
            .map(|p| (p.label(), p.pipeline.clone()))
            .collect(),
        // A PR whose head moved since (T-780) runs a new commit's pipeline:
        // its pass is an edge though the last read's passed too.
        passed: open_passed(new)
            && (!open_passed(old) || kept.iter().any(|(o, p)| o.head_oid != p.head_oid)),
        merged: all_merged(new) && !all_merged(old),
        landed: new
            .iter()
            .filter(|p| p.state == PrState::Merged)
            .filter(|p| was(p).is_some_and(|o| o.state.is_open()))
            .map(Pr::label)
            .collect(),
        deployed: arrived_envs(new).into_iter().filter(|e| !before.contains(e)).collect(),
        reached: new
            .iter()
            .filter(|p| p.state == PrState::Merged)
            .flat_map(|p| {
                let had = was(p).filter(|o| o.state == PrState::Merged);
                p.deployed
                    .iter()
                    .filter(move |e| !had.is_some_and(|o| o.deployed.contains(e)))
                    .map(move |e| (p.label(), e.clone()))
            })
            .collect(),
        failed: match &now {
            Pipeline::Failed(job) if !matches!(then, Pipeline::Failed(_)) => Some(job.clone()),
            _ => None,
        },
        waiting: match &now_open {
            Pipeline::Waiting(job) if then_open != now_open => Some(job.clone()),
            _ => None,
        },
        stopped: new
            .iter()
            .filter(|p| p.state == PrState::Merged)
            .filter_map(|p| match &p.pipeline {
                Pipeline::Waiting(job)
                    if !was(p).is_some_and(|o| {
                        o.state == PrState::Merged && o.pipeline == p.pipeline
                    }) =>
                {
                    Some((p.label(), job.clone()))
                }
                _ => None,
            })
            .collect(),
        conflict: kept
            .iter()
            .filter(|(o, p)| o.conflict != p.conflict)
            .map(|(_, p)| (*p, false))
            .chain(fresh.iter().filter(|p| p.conflict).map(|p| (*p, true)))
            .map(|(p, fresh)| Conflict {
                repo: p.repo.clone(),
                number: p.number,
                label: p.label(),
                on: p.conflict,
                fresh,
            })
            .collect(),
        review: kept
            .iter()
            .filter(|(o, p)| o.review != p.review && verdict(p.review))
            .map(|(_, p)| *p)
            .chain(fresh.iter().filter(|p| verdict(p.review)).copied())
            .map(|p| (p.label(), p.review))
            .collect(),
    }
}

/// One repository's answer: its PRs, or why not.
#[derive(Debug, Clone)]
pub struct RepoReport {
    pub key: String,
    /// The remote's path, as a `Pr`'s `repo` spells it.
    pub path: String,
    pub error: Option<String>,
}

/// What one pass found.
#[derive(Debug, Clone, Default)]
pub struct Sample {
    pub at_ms: u64,
    pub probes: Vec<(String, HostProbe)>,
    pub repos: Vec<RepoReport>,
    /// Each queried ticket's set, newest first; a ticket with nothing bound
    /// is present with an empty set, so the writer can tell "read, none"
    /// from "not read".
    pub sets: HashMap<ulid::Ulid, Vec<Pr>>,
    /// What each queried ticket had pushed and opened as the pass began
    /// (T-761): a later look that finds the same knows the set still holds.
    pub basis: HashMap<ulid::Ulid, Basis>,
}

/// The local facts a ticket's set was read on (T-761): what its legs had
/// pushed (`pushed`) and the PR URLs its sessions had printed. A PR the
/// forge would show that this read did not is one a push or a `pr create`
/// made since, and each leaves a local trace.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Basis {
    pub pushed: Option<Vec<String>>,
    pub opened: Vec<String>,
}

/// A look at a ticket after its turn ended (T-761): what its legs have
/// pushed now, and whether a transcript tail the cost pass has not reached
/// holds a `pr create`. `None` in either is unknown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Local {
    pub at_ms: u64,
    pub pushed: Option<Vec<String>>,
    pub tail_opened: Option<bool>,
}

/// What a ticket's legs have pushed (T-761): every remote-tracking ref that
/// points at a commit in a leg's HEAD reflog, as `<repo> <ref> <oid>`,
/// sorted, `logs[i]` being `legs[i]`'s reflog. A push from the worktree
/// moves one of them; a commit nobody pushed moves none. `None` when git
/// could not list a leg's refs.
pub fn pushed(legs: &[LegQuery], logs: &[Vec<String>]) -> Option<Vec<String>> {
    let mut out = Vec::new();
    for (leg, log) in legs.iter().zip(logs) {
        if log.is_empty() {
            continue;
        }
        let o = crate::git::git(&leg.dir)
            .args([
                "--no-optional-locks",
                "for-each-ref",
                "--format=%(objectname) %(refname)",
                "refs/remotes",
            ])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())?;
        let log: std::collections::HashSet<&str> = log.iter().map(String::as_str).collect();
        for line in String::from_utf8_lossy(&o.stdout).lines() {
            let Some((oid, name)) = line.trim().split_once(' ') else { continue };
            if log.contains(oid) {
                out.push(format!("{} {name} {oid}", leg.repo_path));
            }
        }
    }
    out.sort();
    out.dedup();
    Some(out)
}

/// The look's half (T-761), on a worker: the legs' pushes and the tails.
pub fn look(legs: &[LegQuery], tails: &[(String, crate::cost::Cursor)], at_ms: u64) -> Local {
    let logs: Vec<Vec<String>> = legs.iter().map(|l| reflog(&l.dir)).collect();
    Local { at_ms, pushed: pushed(legs, &logs), tail_opened: crate::cost::opened_in_tails(tails) }
}

/// The pass, on a worker thread.
pub fn sample(mut q: Query, now_ms: u64) -> Sample {
    let mut out = Sample { at_ms: now_ms, ..Default::default() };
    for p in &q.probes {
        out.probes.push((p.host.clone(), probe_host(&p.host, &p.candidates)));
    }
    list_probed(&mut q, &out.probes);
    // What each ticket had pushed, read before any list (T-761): a push
    // after this read is one the lists may miss, so it must move the basis.
    let mut logs: Vec<Vec<Vec<String>>> = Vec::new();
    for t in &q.tickets {
        let legs: Vec<Vec<String>> = t.legs.iter().map(|l| reflog(&l.dir)).collect();
        out.basis
            .insert(t.ticket, Basis { pushed: pushed(&t.legs, &legs), opened: t.opened.clone() });
        logs.push(legs);
    }
    let mut prs: Vec<Pr> = Vec::new();
    for r in &q.repos {
        match list(r) {
            Ok(mut found) => {
                prs.append(&mut found);
                out.repos.push(RepoReport {
                    key: r.key(),
                    path: r.remote.path.clone(),
                    error: None,
                });
            }
            Err(e) => out.repos.push(RepoReport {
                key: r.key(),
                path: r.remote.path.clone(),
                error: Some(e),
            }),
        }
    }
    for (t, logs) in q.tickets.iter().zip(logs) {
        let facts = Facts {
            branch: t.branch.clone(),
            reflog: t
                .legs
                .iter()
                .zip(logs)
                .flat_map(|(l, log)| log.into_iter().map(move |oid| (l.repo_path.clone(), oid)))
                .collect(),
            opened: t.opened.clone(),
        };
        let bound: Vec<Pr> = prs.iter().filter(|p| forge::binds(p, &facts)).cloned().collect();
        let mut set = forge::ticket_set(bound);
        // GitLab's list serves a head minutes old after a push (T-784): an
        // open MR whose list head is not its leg's tip, or one newly bound,
        // is read whole, and its head pipeline is the pipeline. One the
        // list already has at the tip costs nothing more. The list says
        // nothing of a pipeline: the rest are asked by sha, one call each.
        let tips: Vec<(String, String)> = t
            .legs
            .iter()
            .map(|l| (l.repo_path.clone(), crate::worktree::branch_tip(&l.dir, &t.branch)))
            .collect();
        for pr in set.iter_mut().filter(|p| p.forge == Forge::GitLab && p.state.is_open()) {
            let Some(r) = q.repos.iter().find(|r| r.remote.path == pr.repo) else { continue };
            let head = if mr_unsure(pr, &t.branch, &tips, &t.held) {
                gitlab_mr(r, pr.number).and_then(|read| {
                    pr.take_read(&read.pr);
                    read.pipeline
                })
            } else {
                None
            };
            pr.pipeline = match head {
                Some((pipeline, id)) => named_job(r, pipeline, id),
                None => gitlab_pipeline(r, &pr.head_oid),
            };
        }
        merged_reads(
            &mut set,
            &q,
            now_ms,
            |pr| match q.repos.iter().find(|r| r.remote.path == pr.repo) {
                Some(r) => commit_pipeline(r, &pr.merge_oid),
                None => Pipeline::Unknown,
            },
            |pr| match q.repos.iter().find(|r| r.remote.path == pr.repo) {
                Some(r) => deployments(r, &pr.merge_oid, &q.envs),
                None => Vec::new(),
            },
        );
        out.sets.insert(t.ticket, set);
    }
    out
}

/// Each merged PR of a set, per PR (T-771): its merge commit's pipeline
/// (the build-and-deploy run on main) and the environments it reached, what
/// earlier passes learned kept and read again only while unsettled. A
/// pipeline is read until it passes or fails, deployments until every
/// wanted environment is in. A merge commit a newer one of the same
/// repository superseded is read only for `SUPERSEDED_READ_MS` after its
/// merge, so a ticket's old PRs cost nothing once the forge has moved on.
pub fn merged_reads(
    set: &mut [Pr],
    q: &Query,
    now_ms: u64,
    mut pipeline: impl FnMut(&Pr) -> Pipeline,
    mut deployments: impl FnMut(&Pr) -> Vec<String>,
) {
    let mut newest: Vec<String> = Vec::new();
    for pr in set.iter_mut().filter(|p| p.state == PrState::Merged && !p.merge_oid.is_empty()) {
        let superseded = newest.contains(&pr.repo);
        if !superseded {
            newest.push(pr.repo.clone());
        }
        let live = !superseded || now_ms.saturating_sub(pr.updated_ms) < SUPERSEDED_READ_MS;
        let key = (pr.repo.clone(), pr.merge_oid.clone());
        let last = q.commit_pipelines.get(&key);
        pr.pipeline = match last {
            Some(p @ (Pipeline::Passed | Pipeline::Failed(_))) => p.clone(),
            _ if live => match pipeline(pr) {
                Pipeline::Unknown => last.cloned().unwrap_or(Pipeline::Unknown),
                p => p,
            },
            Some(p) => p.clone(),
            None => pr.pipeline.clone(),
        };
        let mut known = q.deployed.get(&key).cloned().unwrap_or_default();
        if live && q.envs.iter().any(|e| !known.contains(e)) {
            for env in deployments(pr) {
                if !known.contains(&env) {
                    known.push(env);
                }
            }
        }
        pr.deployed = known;
    }
}

/// The pending repositories whose host's probe, answered on this pass,
/// found a CLI join the pass's list (T-752); the rest are not read.
pub fn list_probed(q: &mut Query, answered: &[(String, HostProbe)]) {
    for p in std::mem::take(&mut q.pending) {
        let Some((_, probe)) = answered.iter().find(|(h, _)| *h == p.remote.host) else { continue };
        let Some(cli) = probe.cli.clone() else { continue };
        if q.repos.iter().any(|r| r.remote == p.remote) {
            continue;
        }
        q.repos.push(RepoQuery { remote: p.remote, cli, viewer: probe.viewer.clone(), dir: p.dir });
    }
}

/// Which candidate is signed in to the host, in order; the viewer's
/// username beside it where the CLI needs one.
fn probe_host(host: &str, candidates: &[Cli]) -> HostProbe {
    for c in candidates {
        let ok = match c.forge {
            Forge::GitHub => run(&c.bin, &["auth", "status", "--hostname", host], None).is_ok(),
            Forge::GitLab => run(&c.bin, &["auth", "status", "--hostname", host], None).is_ok(),
        };
        if !ok {
            continue;
        }
        let viewer = match c.forge {
            Forge::GitHub => None,
            Forge::GitLab => run(&c.bin, &["api", "--hostname", host, "user"], None)
                .ok()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                .and_then(|v| v.get("username")?.as_str().map(str::to_string)),
        };
        return HostProbe { cli: Some(c.clone()), at: Instant::now(), viewer };
    }
    HostProbe { cli: None, at: Instant::now(), viewer: None }
}

/// The signed-in account's PRs in one repository, every state.
fn list(r: &RepoQuery) -> Result<Vec<Pr>, String> {
    match r.cli.forge {
        Forge::GitHub => {
            let repo = format!("{}/{}", r.remote.host, r.remote.path);
            let out = run(
                &r.cli.bin,
                &[
                    "pr",
                    "list",
                    "--repo",
                    &repo,
                    "--author",
                    "@me",
                    "--state",
                    "all",
                    "--limit",
                    LIST_LIMIT,
                    "--json",
                    "number,url,state,isDraft,headRefName,headRefOid,mergeCommit,mergeable,\
                     reviewDecision,updatedAt,statusCheckRollup",
                ],
                Some(&r.dir),
            )?;
            Ok(forge::parse_gh(&r.remote.path, &out))
        }
        Forge::GitLab => {
            // The signed-in account's MRs are the whole candidate set: with
            // the account unknown there is no list, never an unfiltered one
            // a stranger's MR could bind through.
            let Some(viewer) = r.viewer.as_deref() else {
                return Err("signed-in account unknown; glab api user did not answer".into());
            };
            let repo = format!("https://{}/{}", r.remote.host, r.remote.path);
            let out = run(
                &r.cli.bin,
                &[
                    "mr",
                    "list",
                    "--repo",
                    &repo,
                    "--author",
                    viewer,
                    "--all",
                    "--per-page",
                    LIST_LIMIT,
                    "--output",
                    "json",
                ],
                Some(&r.dir),
            )?;
            Ok(forge::parse_glab(&r.remote.path, &out))
        }
    }
}

/// An open MR whose list row may be stale (T-784): its head is not the
/// tip of the ticket's branch in the leg of its repository, or no tip says
/// and the ticket's reads never held it. An MR from another branch is
/// judged as one with no tip, as the pipeline gate does (`head_unread`).
pub fn mr_unsure(pr: &Pr, branch: &str, tips: &[(String, String)], held: &[(String, u64)]) -> bool {
    let ours = pr.head_ref.is_empty() || pr.head_ref == branch;
    match tips.iter().find(|(repo, tip)| *repo == pr.repo && !tip.is_empty()) {
        Some((_, tip)) if ours => *tip != pr.head_oid,
        _ => !held.iter().any(|(r, n)| *r == pr.repo && *n == pr.number),
    }
}

/// One open MR read whole (T-784). `None` when the call fails or says
/// something else: the list's row stands, and the pipeline gate holds it.
fn gitlab_mr(r: &RepoQuery, iid: u64) -> Option<forge::MrRead> {
    let project = r.remote.path.replace('/', "%2F");
    let path = format!("projects/{project}/merge_requests/{iid}");
    let out = run(&r.cli.bin, &["api", "--hostname", &r.remote.host, &path], Some(&r.dir)).ok()?;
    forge::parse_glab_mr(&r.remote.path, &out).filter(|m| m.pr.number == iid)
}

/// A GitLab commit's newest pipeline, by sha. `Unknown` when the call
/// fails: never read as green.
fn gitlab_pipeline(r: &RepoQuery, sha: &str) -> Pipeline {
    let project = r.remote.path.replace('/', "%2F");
    let path = format!("projects/{project}/pipelines?sha={sha}&per_page=1");
    let Ok(out) = run(&r.cli.bin, &["api", "--hostname", &r.remote.host, &path], Some(&r.dir))
    else {
        return Pipeline::Unknown;
    };
    match forge::gitlab_pipeline_id(&out) {
        Some(id) => named_job(r, forge::parse_gitlab_pipelines(&out), id),
        None => forge::parse_gitlab_pipelines(&out),
    }
}

/// A GitLab pipeline's wait or failure named by one more read, of its jobs
/// (T-772); a failed read, or any other word, leaves it as it is.
fn named_job(r: &RepoQuery, pipeline: Pipeline, id: u64) -> Pipeline {
    let Some(status) = pipeline.gitlab_job_status() else { return pipeline };
    let project = r.remote.path.replace('/', "%2F");
    let path = format!("projects/{project}/pipelines/{id}/jobs?per_page=100");
    match run(&r.cli.bin, &["api", "--hostname", &r.remote.host, &path], Some(&r.dir)) {
        Ok(jobs) => pipeline.with_job(forge::parse_gitlab_job(&jobs, status)),
        Err(_) => pipeline,
    }
}

/// A merge commit's pipeline on the forge: GitHub's check runs for the
/// commit, GitLab's newest pipeline by sha. `Unknown` when the call fails.
fn commit_pipeline(r: &RepoQuery, oid: &str) -> Pipeline {
    match r.cli.forge {
        Forge::GitHub => {
            let path = format!("repos/{}/commits/{oid}/check-runs?per_page=100", r.remote.path);
            match run(&r.cli.bin, &["api", "--hostname", &r.remote.host, &path], Some(&r.dir)) {
                Ok(out) => forge::parse_gh_check_runs(&out),
                Err(_) => Pipeline::Unknown,
            }
        }
        Forge::GitLab => gitlab_pipeline(r, oid),
    }
}

/// The environments, of those wanted, a merge commit reached: GitHub's
/// deployments by sha with a success status, GitLab's successful
/// deployments of each wanted environment naming the sha. In the order they
/// happened (`forge::deploy_order`, T-784).
fn deployments(r: &RepoQuery, oid: &str, envs: &[String]) -> Vec<String> {
    let mut out: Vec<(String, u64)> = Vec::new();
    match r.cli.forge {
        Forge::GitHub => {
            let path = format!("repos/{}/deployments?sha={oid}&per_page=30", r.remote.path);
            let Ok(list) =
                run(&r.cli.bin, &["api", "--hostname", &r.remote.host, &path], Some(&r.dir))
            else {
                return Vec::new();
            };
            for (id, env) in forge::parse_gh_deployments(&list) {
                if !envs.contains(&env) || out.iter().any(|(e, _)| *e == env) {
                    continue;
                }
                let path = format!("repos/{}/deployments/{id}/statuses?per_page=1", r.remote.path);
                if let Ok(status) =
                    run(&r.cli.bin, &["api", "--hostname", &r.remote.host, &path], Some(&r.dir))
                {
                    if let Some(at) = forge::parse_gh_deployment_success(&status) {
                        out.push((env, at));
                    }
                }
            }
        }
        Forge::GitLab => {
            let project = r.remote.path.replace('/', "%2F");
            for env in envs {
                let path = format!(
                    "projects/{project}/deployments?environment={env}&status=success\
                     &order_by=created_at&sort=desc&per_page=20"
                );
                if let Ok(list) =
                    run(&r.cli.bin, &["api", "--hostname", &r.remote.host, &path], Some(&r.dir))
                {
                    if let Some(at) = forge::parse_gitlab_deployed(&list, oid) {
                        out.push((env.clone(), at));
                    }
                }
            }
        }
    }
    forge::deploy_order(out, envs)
}

/// Every commit a worktree's HEAD pointed at, newest first; empty where
/// there is no reflog (a reclaimed worktree, a directory that is gone).
pub fn reflog(dir: &Path) -> Vec<String> {
    if !dir.is_dir() {
        return Vec::new();
    }
    let out = crate::git::git(dir)
        .args(["--no-optional-locks", "reflog", "show", "--format=%H", "-n", REFLOG_CAP, "HEAD"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect(),
        _ => Vec::new(),
    }
}

/// The remote a repository's base branch is judged against, parsed: the
/// branch's own remote, else `origin`, else the one remote there is.
pub fn remote_of(repo: &Path, base: &str) -> Option<Remote> {
    let mut names: Vec<String> = Vec::new();
    if let Some(r) = crate::gitstatus::remote_of(repo, base) {
        names.push(r);
    }
    names.push("origin".into());
    let out = crate::git::git(repo)
        .args(["--no-optional-locks", "remote"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let all: Vec<String> =
        String::from_utf8_lossy(&out.stdout).lines().map(|l| l.trim().to_string()).collect();
    if let [only] = all.as_slice() {
        names.push(only.clone());
    }
    for name in names {
        if !all.contains(&name) {
            continue;
        }
        let url = crate::git::git(repo)
            .args(["--no-optional-locks", "remote", "get-url", &name])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if url.status.success() {
            if let Some(r) = Remote::parse(&String::from_utf8_lossy(&url.stdout)) {
                return Some(r);
            }
        }
    }
    None
}

/// One fenced CLI call: stdout on success, the first stderr line (the
/// CLI's own words) or the timeout on failure.
fn run(bin: &Path, args: &[&str], cwd: Option<&Path>) -> Result<String, String> {
    let mut cmd = Command::new(bin);
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    // No prompt, no pager, no colour, no update stamp of its own: the CLI
    // writes nothing on mesimon's account, and nothing waits on a tty.
    for (k, v) in [
        ("GH_NO_UPDATE_NOTIFIER", "1"),
        ("GH_PROMPT_DISABLED", "1"),
        ("GH_PAGER", "cat"),
        ("PAGER", "cat"),
        ("NO_COLOR", "1"),
        ("CLICOLOR", "0"),
        ("GLAB_CHECK_UPDATE", "false"),
        ("GLAB_NO_PROMPT", "1"),
        ("GLAB_SHOW_WHATS_NEW", "false"),
        ("GLAB_PAGER", "cat"),
        ("TERM", "dumb"),
    ] {
        cmd.env(k, v);
    }
    cmd.env_remove("GH_FORCE_TTY");
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    // Its own session: no controlling tty even under a foreground daemon, and
    // a process group of its own to kill whole on the deadline.
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let name = bin.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut child = cmd.spawn().map_err(|e| format!("{name}: {e}"))?;
    let stderr = child.stderr.take();
    let first_line = std::thread::spawn(move || {
        let Some(stderr) = stderr else { return String::new() };
        let mut first = String::new();
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            // Drain to EOF so the child never blocks on a full pipe; keep the
            // first line that says something — a deprecation notice is not it.
            let t = line.trim();
            if first.is_empty() && !t.is_empty() && !t.starts_with("DEPRECATION") {
                first = t.to_string();
            }
        }
        first
    });
    let mut stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(o) = stdout.as_mut() {
            let _ = o.read_to_string(&mut s);
        }
        s
    });
    let deadline = Instant::now() + CALL_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                // SAFETY: a signal to a process group this daemon created.
                unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
                let _ = child.wait();
                let _ = first_line.join();
                let _ = reader.join();
                return Err(format!("{name} did not finish within {}s", CALL_TIMEOUT.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(e) => return Err(format!("{name}: {e}")),
        }
    };
    let first = first_line.join().unwrap_or_default();
    let out = reader.join().unwrap_or_default();
    if status.success() {
        Ok(out)
    } else if first.is_empty() {
        Err(format!("{name} exited {}", status.code().unwrap_or(-1)))
    } else {
        Err(mesimon_core::text::scrub_text(&first))
    }
}

/// The daemon's standing knowledge of the forge road.
#[derive(Debug, Default)]
pub struct State {
    /// Per host: which CLI reads it, and the signed-in account.
    pub probes: HashMap<String, HostProbe>,
    /// Per repository directory: its remote, parsed once.
    pub remotes: HashMap<PathBuf, Option<Remote>>,
    /// Each ticket's set, newest first, and when it was read.
    pub sets: HashMap<ulid::Ulid, Vec<Pr>>,
    pub at_ms: HashMap<ulid::Ulid, u64>,
    /// Every PR, by repository path and number, a ticket's reads have held
    /// since this start or in `forge.json` (T-767): a PR read again after
    /// a read that missed it, or reopened, is no opening.
    pub held: HashMap<ulid::Ulid, Vec<(String, u64)>>,
    /// The open PRs, by repository and number, whose `conflict` edge was
    /// said since this start and not cleared (T-769): a clear is said only
    /// where its conflict was. Memory only, so a restart's first read is a
    /// baseline for both.
    pub conflicted: HashMap<ulid::Ulid, Vec<(String, u64)>>,
    /// The repositories whose last call failed, with the CLI's words.
    pub errors: Vec<(String, String)>,
    /// Merge commits' pipelines as last read, by repository path and oid
    /// (T-771): settled once passed or failed.
    pub commit_pipelines: HashMap<(String, String), Pipeline>,
    /// The environments each merge commit was seen reaching.
    pub deployed: HashMap<(String, String), Vec<String>>,
    pub last: Option<Instant>,
    /// Unix ms of the last pass that answered, for the snapshot.
    pub last_ms: u64,
    pub inflight: bool,
    pub gen: u64,
    /// What each ticket's last read was taken on, and the look after its
    /// last turn (T-761): equal, the set read then still holds.
    pub basis: HashMap<ulid::Ulid, Basis>,
    pub local: HashMap<ulid::Ulid, Local>,
    /// The tickets whose turn's end asked a read (T-761), with when: a
    /// pass begun since answers them.
    pub owed: HashMap<ulid::Ulid, u64>,
    /// When each repository was last asked, by `RepoQuery::key`.
    pub asked: HashMap<String, Instant>,
    /// A turn's end's read is out.
    pub burst_inflight: bool,
    /// The tickets whose own look (T-761's) is out with a forge half: not
    /// yet known whether their turn's end owes a read (T-773).
    pub looking: HashSet<ulid::Ulid>,
}

impl State {
    /// The CLI a host is read through, if the probe said one.
    pub fn cli_for(&self, host: &str) -> Option<&Cli> {
        self.probes.get(host).and_then(|p| p.cli.as_ref())
    }

    /// Whether `set` holds an open PR its ticket's reads never held
    /// (T-767), remembering every PR of it.
    pub fn opens(&mut self, t: ulid::Ulid, set: &[Pr]) -> bool {
        let held = self.held.entry(t).or_default();
        let mut opens = false;
        for p in set {
            let id = (p.repo.clone(), p.number);
            if !held.contains(&id) {
                opens |= p.state.is_open();
                held.push(id);
            }
        }
        opens
    }

    /// Keeps the conflict edges that are news (T-769): a conflict once,
    /// until a clear that follows it, which is said only where the conflict
    /// was. `baseline`: the read is the first live one against a set
    /// `forge.json` brought back, which says no conflict and no clear of a
    /// PR that set held (a newly bound one's conflict is said).
    pub fn say_conflicts(&mut self, t: ulid::Ulid, edge: &mut Edge, baseline: bool) {
        let said = self.conflicted.entry(t).or_default();
        edge.conflict.retain(|c| {
            let id = (c.repo.clone(), c.number);
            let was = said.contains(&id);
            if !c.on {
                said.retain(|s| *s != id);
                return was;
            }
            if was || (baseline && !c.fresh) {
                return false;
            }
            said.push(id);
            true
        });
    }

    /// Something reads a forge on this board: a probe found a CLI.
    pub fn present(&self) -> bool {
        self.probes.values().any(|p| p.cli.is_some())
    }

    /// Whether a pass is due: not in flight, and the cadence (or the
    /// backoff after an error) passed.
    pub fn due(&self) -> bool {
        if self.inflight {
            return false;
        }
        let wait = if self.errors.is_empty() { every() } else { every().max(BACKOFF) };
        self.last.is_none_or(|t| t.elapsed() >= wait)
    }

    /// Whether a repository may be read off the minute: for a turn's end
    /// (T-761, `floor` is `BURST_MS`) or a pipeline in flight (T-781,
    /// `busy_every`): not read inside `floor`, and not one whose last call
    /// failed (the minute's backoff has it).
    pub fn due_for(&self, key: &str, floor: Duration) -> bool {
        !self.errors.iter().any(|(k, _)| k == key)
            && self.asked.get(key).is_none_or(|t| t.elapsed() >= floor)
    }

    /// The tickets holding an open or merged PR whose pipeline runs or
    /// waits on a person (T-781): their repositories are read every
    /// `busy_every`, so a deploy job of seconds shows on the card.
    pub fn busy(&self) -> HashSet<ulid::Ulid> {
        self.sets
            .iter()
            .filter(|(_, set)| {
                set.iter().any(|p| {
                    p.state != PrState::Closed
                        && matches!(p.pipeline, Pipeline::Running | Pipeline::Waiting(_))
                })
            })
            .map(|(t, _)| *t)
            .collect()
    }

    /// The forge's verdict on a ticket is current (T-739): no forge reads
    /// this board (every remote parsed to none, or every host probed to no
    /// CLI), or the ticket's set was read since its agent's last end of
    /// turn (`turn_end`, unix ms) — the turn that may have opened the PR —
    /// or the look after that turn found nothing a read could miss
    /// (`unchanged`, T-761). Before the first probe nothing is known and
    /// nothing is fresh: the train waits a bucket.
    pub fn fresh(&self, ticket: ulid::Ulid, turn_end: u64, opened: &[String]) -> bool {
        self.none()
            || self.at_ms.get(&ticket).is_some_and(|read_at| *read_at >= turn_end)
            || self.unchanged(ticket, turn_end, opened)
    }

    /// No forge reads this board: every remote parsed to none, or every
    /// host probed to no CLI.
    fn none(&self) -> bool {
        !self.remotes.is_empty()
            && (self.remotes.values().all(Option::is_none)
                || (!self.probes.is_empty() && self.probes.values().all(|p| p.cli.is_none())))
    }

    /// A read the ticket's turn's end waits for (T-773): its look is out
    /// and may find a push, or it found one and no read begun since has
    /// answered. Nothing on a board no forge reads.
    pub fn awaited(&self, ticket: ulid::Ulid) -> bool {
        !self.none() && (self.looking.contains(&ticket) || self.owed.contains_key(&ticket))
    }

    /// A read begun at `at_ms` landed with these tickets' sets: each owed
    /// since then is answered.
    pub fn answer<'a>(&mut self, tickets: impl IntoIterator<Item = &'a ulid::Ulid>, at_ms: u64) {
        for t in tickets {
            if self.owed.get(t).is_some_and(|since| at_ms >= *since) {
                self.owed.remove(t);
            }
        }
    }

    /// The set read before `ticket`'s turn ended still holds (T-761): the
    /// look after the turn found the legs' pushes as the read did, no `pr
    /// create` in a tail the cost pass has not reached, and no URL in
    /// `opened` (the ledger's now) that the read did not have.
    pub fn unchanged(&self, ticket: ulid::Ulid, turn_end: u64, opened: &[String]) -> bool {
        let (Some(basis), Some(local)) = (self.basis.get(&ticket), self.local.get(&ticket)) else {
            return false;
        };
        local.at_ms >= turn_end
            && local.pushed.is_some()
            && local.pushed == basis.pushed
            && local.tail_opened == Some(false)
            && opened.iter().all(|u| basis.opened.iter().any(|b| forge::same_url(b, u)))
    }

    /// The open PRs of a ticket's set.
    pub fn open_of(&self, ticket: ulid::Ulid) -> Vec<&Pr> {
        self.sets
            .get(&ticket)
            .map(|s| s.iter().filter(|p| p.state.is_open()).collect())
            .unwrap_or_default()
    }

    /// Any PR bound to the ticket, open, merged or closed (T-778): its
    /// work goes through pull requests, and the forge merges it.
    pub fn has_pr(&self, ticket: ulid::Ulid) -> bool {
        self.sets.get(&ticket).is_some_and(|s| !s.is_empty())
    }

    /// The ticket's merged PR for a repository at a tip, if the newest
    /// there is merged at that tip (`forge::merged_at`).
    pub fn merged_at(&self, ticket: ulid::Ulid, repo_path: &str, tip: &str) -> Option<&Pr> {
        self.sets.get(&ticket).and_then(|s| forge::merged_at(s, repo_path, tip))
    }

    /// The one line the advisory row and `doctor` say while a repository
    /// cannot be read.
    pub fn trouble(&self) -> Option<String> {
        let (key, why) = self.errors.first()?;
        Some(format!("{key}: {why}"))
    }
}

/// `forge.json`'s schema (T-752).
pub const FORGE_SCHEMA: u32 = 1;

/// One PR as `forge.json` keeps it: the wire's words, so the file reads
/// as the snapshot does and a word this build does not know drops the PR
/// rather than the file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrRecord {
    pub forge: String,
    pub repo: String,
    pub number: u64,
    pub url: String,
    pub state: String,
    pub head_oid: String,
    pub head_ref: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub merge_oid: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub conflict: bool,
    pub review: String,
    pub pipeline: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub job: String,
    pub updated_ms: u64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub deployed: Vec<String>,
}

impl PrRecord {
    fn of(p: &Pr) -> Self {
        PrRecord {
            forge: p.forge.word().into(),
            repo: p.repo.clone(),
            number: p.number,
            url: p.url.clone(),
            state: p.state.word().into(),
            head_oid: p.head_oid.clone(),
            head_ref: p.head_ref.clone(),
            merge_oid: p.merge_oid.clone(),
            conflict: p.conflict,
            review: p.review.word().into(),
            pipeline: p.pipeline.word().into(),
            job: p.pipeline.job().unwrap_or_default().into(),
            updated_ms: p.updated_ms,
            deployed: p.deployed.clone(),
        }
    }

    fn pr(self) -> Option<Pr> {
        Some(Pr {
            forge: Forge::from_word(&self.forge)?,
            state: PrState::from_word(&self.state)?,
            pipeline: Pipeline::from_words(&self.pipeline, Some(&self.job)),
            review: Review::from_word(&self.review),
            repo: self.repo,
            number: self.number,
            url: self.url,
            head_oid: self.head_oid,
            head_ref: self.head_ref,
            merge_oid: self.merge_oid,
            conflict: self.conflict,
            updated_ms: self.updated_ms,
            deployed: self.deployed,
        })
    }
}

/// What the passes read of one merge commit: its pipeline as last read and
/// the environments it was seen reaching.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CommitRecord {
    pub repo: String,
    pub oid: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub pipeline: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub job: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub deployed: Vec<String>,
}

/// `forge.json` (T-752): each ticket's set as the last pass read it, and
/// what the passes settled, so a restart's first snapshot carries the PRs.
/// No time in it: a quiet board writes nothing. Every field defaults, so a
/// file from an older build of this schema reads as what it held.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ForgeFile {
    pub schema_version: u32,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub sets: BTreeMap<ulid::Ulid, Vec<PrRecord>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub commits: Vec<CommitRecord>,
}

impl ForgeFile {
    pub fn of(state: &State) -> Self {
        let sets = state
            .sets
            .iter()
            .map(|(t, set)| (*t, set.iter().map(PrRecord::of).collect()))
            .collect();
        let mut keys: Vec<&(String, String)> =
            state.commit_pipelines.keys().chain(state.deployed.keys()).collect();
        keys.sort();
        keys.dedup();
        let commits = keys
            .into_iter()
            .map(|key| {
                let pipeline = state.commit_pipelines.get(key);
                CommitRecord {
                    repo: key.0.clone(),
                    oid: key.1.clone(),
                    pipeline: pipeline.map(|p| p.word().to_string()).unwrap_or_default(),
                    job: pipeline.and_then(Pipeline::job).unwrap_or_default().into(),
                    deployed: state.deployed.get(key).cloned().unwrap_or_default(),
                }
            })
            .collect();
        ForgeFile { schema_version: FORGE_SCHEMA, sets, commits }
    }

    /// No sets, at this build's schema: what an absent file reads as.
    pub fn default_of_schema() -> Self {
        ForgeFile { schema_version: FORGE_SCHEMA, ..ForgeFile::default() }
    }
}

impl State {
    /// Take a loaded `forge.json` back (T-752): the sets of the tickets the
    /// board still has, and the settled commits. `at_ms` stays empty — it
    /// is "read live since this start", what the train's freshness and the
    /// pipeline gate wait for.
    pub fn restore(&mut self, file: ForgeFile, live: impl Fn(ulid::Ulid) -> bool) {
        for (t, records) in file.sets {
            if !live(t) {
                continue;
            }
            let set = forge::ticket_set(records.into_iter().filter_map(PrRecord::pr).collect());
            self.opens(t, &set);
            self.sets.insert(t, set);
        }
        for c in file.commits {
            let key = (c.repo, c.oid);
            match Pipeline::from_words(&c.pipeline, Some(&c.job)) {
                Pipeline::Unknown => {}
                p => {
                    self.commit_pipelines.insert(key.clone(), p);
                }
            }
            if !c.deployed.is_empty() {
                self.deployed.insert(key, c.deployed);
            }
        }
    }
}

/// Startup loader: the file, any notices, and whether writes are barred,
/// on the other state files' contract (`store::load_versioned`).
pub fn load_or_recover(paths: &Paths) -> (ForgeFile, Vec<Notice>, bool) {
    let (file, notices, barred) = crate::store::load_versioned(
        &paths.forge_file(),
        FORGE_SCHEMA,
        "the board's pull requests",
        "",
    );
    (file.unwrap_or_default(), notices, barred)
}

/// A ticket's set as a pass read it, with the previous read's PRs of every
/// repository whose call failed on this pass carried over (T-752): a
/// forge that did not answer said nothing, and a card does not lose its
/// PRs to a timeout, nor a loaded set to the first pass after a restart.
pub fn carry(old: Option<&[Pr]>, new: Vec<Pr>, failed: &[&str]) -> Vec<Pr> {
    let Some(old) = old else { return new };
    let kept = old.iter().filter(|p| failed.contains(&p.repo.as_str())).cloned();
    let mut set: Vec<Pr> = new.into_iter().filter(|p| !failed.contains(&p.repo.as_str())).collect();
    set.extend(kept);
    forge::ticket_set(set)
}

/// The feed's words for what changed in a ticket's set between two reads:
/// a PR newly bound, one merged or closed, a pipeline that failed or
/// passed. Nothing on the first read, which would say every old PR over.
pub fn delta(old: Option<&[Pr]>, new: &[Pr]) -> Vec<String> {
    let Some(old) = old else { return Vec::new() };
    let mut out = Vec::new();
    for pr in new {
        let label = pr.label();
        match old.iter().find(|o| o.repo == pr.repo && o.number == pr.number) {
            None => out.push(format!("{} {label} bound", pr.state.word())),
            Some(was) => {
                if was.state != pr.state && matches!(pr.state, PrState::Merged | PrState::Closed) {
                    out.push(format!("{} {label}", pr.state.word()));
                }
                if was.pipeline != pr.pipeline {
                    match &pr.pipeline {
                        Pipeline::Failed(job) if job.is_empty() => {
                            out.push(format!("pipeline failed {label}"))
                        }
                        Pipeline::Failed(job) => out.push(format!(
                            "pipeline failed {label}: {}",
                            mesimon_core::text::scrub_text(job)
                        )),
                        Pipeline::Passed if was.pipeline != Pipeline::None => {
                            out.push(format!("pipeline passed {label}"))
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The edge with `old` as everything held before: a PR outside it is
    /// newly bound.
    fn edge(old: &[Pr], new: &[Pr]) -> Edge {
        let held: Vec<(String, u64)> = old.iter().map(|p| (p.repo.clone(), p.number)).collect();
        super::edge(old, new, &held)
    }

    fn pr(number: u64, state: PrState, pipeline: Pipeline) -> Pr {
        Pr {
            forge: Forge::GitHub,
            repo: "o/r".into(),
            number,
            url: format!("https://github.com/o/r/pull/{number}"),
            state,
            head_oid: "h".into(),
            head_ref: "b".into(),
            merge_oid: String::new(),
            conflict: false,
            review: Default::default(),
            pipeline,
            updated_ms: number,
            deployed: Vec::new(),
        }
    }

    /// T-784: an open MR is read whole when its list head is not the leg's
    /// tip, or no tip judges it and it is newly bound; one at the tip, or
    /// held with nothing to judge it by, costs nothing more.
    #[test]
    fn an_mr_is_read_whole_only_where_the_list_may_be_stale() {
        let mut mr = pr(9, PrState::Open, Pipeline::Unknown);
        let held = vec![("o/r".to_string(), 9)];
        let at = |tip: &str| vec![("o/r".to_string(), tip.to_string())];
        assert!(!mr_unsure(&mr, "b", &at("h"), &held), "the list has the tip");
        assert!(mr_unsure(&mr, "b", &at("pushed"), &held), "the list lags the push");
        assert!(!mr_unsure(&mr, "b", &at(""), &held), "no tip, held");
        assert!(mr_unsure(&mr, "b", &at(""), &[]), "no tip, newly bound");
        assert!(mr_unsure(&mr, "b", &[], &[]), "no leg, newly bound");
        mr.head_ref = "other".into();
        assert!(!mr_unsure(&mr, "b", &at("pushed"), &held), "another branch's tip is no judge");
    }

    #[test]
    fn the_delta_says_what_moved_and_nothing_on_the_first_read() {
        let before =
            [pr(1, PrState::Open, Pipeline::Running), pr(2, PrState::Open, Pipeline::None)];
        let after = [
            pr(1, PrState::Merged, Pipeline::Passed),
            pr(2, PrState::Open, Pipeline::Failed("test-linux".into())),
            pr(3, PrState::Open, Pipeline::Running),
        ];
        assert!(delta(None, &after).is_empty());
        assert_eq!(
            delta(Some(&before), &after),
            ["merged #1", "pipeline passed #1", "pipeline failed #2: test-linux", "open #3 bound"]
        );
        let bare = [pr(2, PrState::Open, Pipeline::Failed(String::new()))];
        assert_eq!(delta(Some(&before), &bare), ["pipeline failed #2"]);
    }

    #[test]
    fn the_edge_between_two_reads_names_what_a_column_waits_for() {
        let mut m = pr(1, PrState::Merged, Pipeline::Passed);
        m.merge_oid = "m1".into();
        let old = [pr(1, PrState::Open, Pipeline::Running)];
        let e = edge(&old, &[m.clone()]);
        assert!(e.merged && e.opened.is_empty() && e.deployed.is_empty());
        assert!(!e.passed, "the merge commit's pipeline is no pass (T-767)");
        m.deployed = vec!["staging".into()];
        let e = edge(&[pr(1, PrState::Merged, Pipeline::Passed)], &[m.clone()]);
        assert_eq!(e.deployed, ["staging"]);
        assert!(!e.merged, "merged before too");
        let e = edge(&[m.clone()], &[pr(2, PrState::Open, Pipeline::None), m.clone()]);
        assert!(!e.opened.is_empty() && !e.merged, "a follow-up PR opens");
        assert!(!edge(&[], &[]).any());
    }

    /// T-771: two merged PRs on one ticket keep each its own merge commit's
    /// pipeline and deployments; the newer one's reads do not clear the
    /// older one's, and a settled, covered PR is never read again.
    #[test]
    fn each_merged_pr_keeps_its_own_pipeline_and_deployments() {
        let merged = |number: u64, oid: &str, updated_ms: u64| {
            let mut p = pr(number, PrState::Merged, Pipeline::Unknown);
            p.forge = Forge::GitLab;
            p.merge_oid = oid.into();
            p.updated_ms = updated_ms;
            p
        };
        let now = 10 * SUPERSEDED_READ_MS;
        let key = |oid: &str| ("o/r".to_string(), oid.to_string());
        let mut q =
            Query { envs: vec!["staging".into(), "production".into()], ..Default::default() };
        q.commit_pipelines.insert(key("m1"), Pipeline::Passed);
        q.deployed.insert(key("m1"), vec!["staging".into(), "production".into()]);
        let mut set = [merged(2, "m2", now - 1000), merged(1, "m1", now - 2000)];
        let mut asked: Vec<String> = Vec::new();
        let mut deploys: Vec<String> = Vec::new();
        merged_reads(
            &mut set,
            &q,
            now,
            |p| {
                asked.push(p.merge_oid.clone());
                Pipeline::Running
            },
            |p| {
                deploys.push(p.merge_oid.clone());
                vec!["staging".into()]
            },
        );
        assert_eq!((asked, deploys), (vec!["m2".to_string()], vec!["m2".to_string()]));
        assert_eq!(
            (&set[0].pipeline, &set[0].deployed[..]),
            (&Pipeline::Running, &["staging".to_string()][..])
        );
        assert_eq!(set[1].pipeline, Pipeline::Passed, "the older PR keeps its own");
        assert_eq!(set[1].deployed, ["staging", "production"]);

        // An unsettled older PR is read while its merge is recent, and keeps
        // what was read once the day is past; a failed read is no reading.
        q.commit_pipelines.insert(key("m1"), Pipeline::Running);
        q.deployed.insert(key("m1"), vec!["staging".into()]);
        let mut set = [merged(2, "m2", now - 1000), merged(1, "m1", now - 2000)];
        let mut asked = 0;
        merged_reads(
            &mut set,
            &q,
            now,
            |_| {
                asked += 1;
                Pipeline::Unknown
            },
            |_| Vec::new(),
        );
        assert_eq!(asked, 2);
        assert_eq!(set[1].pipeline, Pipeline::Running, "an unanswered call keeps the last read");
        let mut set = [merged(2, "m2", now - 1000), merged(1, "m1", now - SUPERSEDED_READ_MS)];
        let asked = std::cell::RefCell::new(Vec::<String>::new());
        merged_reads(
            &mut set,
            &q,
            now,
            |p| {
                asked.borrow_mut().push(p.merge_oid.clone());
                Pipeline::Passed
            },
            |p| {
                asked.borrow_mut().push(p.merge_oid.clone());
                Vec::new()
            },
        );
        assert_eq!(*asked.borrow(), ["m2", "m2"], "a superseded day-old merge is not read");
        assert_eq!(set[1].pipeline, Pipeline::Running);
        assert_eq!(set[1].deployed, ["staging"]);

        // The newer PR reaching an environment the older one stands in is
        // news; the older one's standing deployment is not said again.
        let mut one = merged(1, "m1", 1);
        one.pipeline = Pipeline::Passed;
        one.deployed = vec!["staging".into(), "production".into()];
        let two = merged(2, "m2", 2);
        let mut two_deployed = two.clone();
        two_deployed.deployed = vec!["staging".into()];
        let old = [two.clone(), one.clone()];
        let new = [two_deployed, one];
        let e = edge(&old, &new);
        assert_eq!(e.deployed, ["staging"]);
        assert_eq!(e.news(&new), ["deployed to staging !2"]);
        assert!(edge(&new, &new).deployed.is_empty());
    }

    /// T-767: with every PR merged, the merge commit's pipeline passing is
    /// the deploy run finishing — no `passed`, no `pipeline passed` words,
    /// the deployment as before; an open PR's pass is one.
    #[test]
    fn a_merged_commits_pipeline_passing_is_no_pass() {
        let merged = |pipeline: Pipeline, deployed: &[&str]| {
            let mut p = pr(1, PrState::Merged, pipeline);
            p.forge = Forge::GitLab;
            p.merge_oid = "m1".into();
            p.deployed = deployed.iter().map(|d| d.to_string()).collect();
            p
        };
        let running = [merged(Pipeline::Running, &["staging"])];
        let after = [merged(Pipeline::Passed, &["staging", "production"])];
        let e = edge(&running, &after);
        assert!(!e.passed);
        assert_eq!(e.deployed, ["production"]);
        assert_eq!(e.news(&after), ["deployed to production !1"]);
        let waiting = [merged(Pipeline::Waiting("deploy:production".into()), &["staging"])];
        let e = edge(&waiting, &after);
        assert!(!e.passed && e.waiting.is_none() && e.stopped.is_empty());
        assert_eq!(e.news(&after), ["deployed to production !1"]);
        // A merged commit's job stopping on a person is still heard, as
        // that PR's own (T-782).
        let e = edge(&running, &waiting);
        assert!(e.waiting.is_none());
        assert_eq!(e.stopped, [("!1".to_string(), "deploy:production".to_string())]);
        assert_eq!(e.news(&waiting), ["waiting on deploy:production !1"]);
        // One open PR passing beside a merged one: a pass, by its label.
        let open = |pipeline: Pipeline| {
            let mut p = pr(2, PrState::Open, pipeline);
            p.forge = Forge::GitLab;
            p
        };
        let before = [open(Pipeline::Running), merged(Pipeline::Running, &[])];
        let now = [open(Pipeline::Passed), merged(Pipeline::Running, &[])];
        let e = edge(&before, &now);
        assert!(e.passed);
        assert_eq!(e.news(&now), ["pipeline passed !2"]);
        // A follow-up PR opening green on a merged, passed set is a pass too.
        let done = [merged(Pipeline::Passed, &[])];
        let e = edge(&done, &[open(Pipeline::Passed), merged(Pipeline::Passed, &[])]);
        assert!(e.passed && !e.opened.is_empty());
    }

    /// T-767: an opening is a PR the ticket's reads never held; one read
    /// again after a read that missed it, or a baseline's, is not.
    #[test]
    fn an_opening_is_said_once() {
        let mut s = State::default();
        let t = ulid::Ulid::new();
        let one = pr(1, PrState::Open, Pipeline::Running);
        assert!(s.opens(t, std::slice::from_ref(&one)), "the first read holds it");
        assert!(!s.opens(t, std::slice::from_ref(&one)));
        assert!(!s.opens(t, &[]));
        assert!(!s.opens(t, std::slice::from_ref(&one)), "back after a read that missed it");
        let two = pr(2, PrState::Merged, Pipeline::Passed);
        assert!(!s.opens(t, &[two, one.clone()]), "a merged PR opens nothing");
        assert!(s.opens(t, &[pr(3, PrState::Draft, Pipeline::None), one]));
        assert!(!s.opens(ulid::Ulid::new(), &[]));
    }

    /// T-768: a ticket with a PR on two repositories arrives as its least
    /// advanced PR: one merged and deployed beside one open is neither
    /// merged nor deployed, both merged and one deployed is merged and not
    /// deployed, both deployed is deployed. The crown hears each PR's
    /// deployment as it comes.
    #[test]
    fn a_ticket_arrives_as_its_least_advanced_pr() {
        let gl = |repo: &str, n: u64, state: PrState, deployed: &[&str]| {
            let mut p = pr(n, state, Pipeline::Passed);
            p.forge = Forge::GitLab;
            p.repo = repo.into();
            p.merge_oid = if state == PrState::Merged { format!("m{n}") } else { String::new() };
            p.deployed = deployed.iter().map(|d| d.to_string()).collect();
            p
        };
        let open = [gl("g/web", 1, PrState::Open, &[]), gl("g/api", 3, PrState::Open, &[])];
        let web =
            [gl("g/web", 1, PrState::Merged, &["staging"]), gl("g/api", 3, PrState::Open, &[])];
        let e = edge(&open, &web);
        assert!(!e.merged && e.deployed.is_empty(), "api's !3 is open: {e:?}");
        assert_eq!(e.news(&web), ["merged !1", "deployed to staging !1"]);
        let both =
            [gl("g/web", 1, PrState::Merged, &["staging"]), gl("g/api", 3, PrState::Merged, &[])];
        let e = edge(&web, &both);
        assert!(e.merged && e.deployed.is_empty(), "api's merge commit is not on staging: {e:?}");
        assert_eq!(e.news(&both), ["merged !3"]);
        let staged = [
            gl("g/web", 1, PrState::Merged, &["staging"]),
            gl("g/api", 3, PrState::Merged, &["staging"]),
        ];
        let e = edge(&both, &staged);
        assert!(!e.merged);
        assert_eq!(e.deployed, ["staging"]);
        assert_eq!(e.news(&staged), ["deployed to staging !3"]);
        // web on production first: heard, and no arrival until api is too.
        let prod = [
            gl("g/web", 1, PrState::Merged, &["staging", "production"]),
            gl("g/api", 3, PrState::Merged, &["staging"]),
        ];
        let e = edge(&staged, &prod);
        assert!(e.deployed.is_empty());
        assert_eq!(e.news(&prod), ["deployed to production !1"]);
        let all = [
            gl("g/web", 1, PrState::Merged, &["staging", "production"]),
            gl("g/api", 3, PrState::Merged, &["staging", "production"]),
        ];
        assert_eq!(edge(&prod, &all).deployed, ["production"]);
        // An older merged PR on a repository is in its newer one's history:
        // only the newest is asked, and the newest decides.
        let older = [all[0].clone(), all[1].clone(), gl("g/api", 2, PrState::Merged, &[])];
        assert_eq!(edge(&both, &older).deployed, ["staging", "production"]);
        // A pass is every open PR's.
        let running = [gl("g/web", 1, PrState::Open, &[]), {
            let mut p = gl("g/api", 3, PrState::Open, &[]);
            p.pipeline = Pipeline::Running;
            p
        }];
        assert!(!edge(&running, &running).passed);
        assert!(edge(&running, &open).passed);
    }

    /// T-780: a push the forge ran and passed between two reads is a new
    /// commit's pass, though the read before passed too; the same head
    /// passing again is none.
    #[test]
    fn a_pass_on_a_moved_head_is_an_edge() {
        let was = [pr(5, PrState::Open, Pipeline::Passed)];
        let mut pushed = was.clone();
        pushed[0].head_oid = "h2".into();
        assert!(!edge(&was, &was).passed);
        let e = edge(&was, &pushed);
        assert!(e.passed);
        assert_eq!(e.news(&pushed), ["pipeline passed #5"]);
        pushed[0].pipeline = Pipeline::Running;
        assert!(!edge(&was, &pushed).passed);
    }

    /// T-757: a PR read open and then merged is `landed`, by label, and the
    /// crown hears `merged !2` — a later PR's too, beside one merged before.
    #[test]
    fn a_pr_that_merges_lands_by_its_label() {
        let gl = |n: u64, state: PrState| {
            let mut p = pr(n, state, Pipeline::Passed);
            p.forge = Forge::GitLab;
            p
        };
        let (one, two) = (gl(1, PrState::Merged), gl(2, PrState::Open));
        let after = [gl(2, PrState::Merged), one.clone()];
        let e = edge(&[two.clone(), one.clone()], &after);
        assert_eq!(e.landed, ["!2"]);
        assert!(e.any());
        assert_eq!(e.news(&after), ["merged !2"]);
        // Merged on both reads, or first read merged: no landing.
        assert!(edge(&after, &after).landed.is_empty());
        assert_eq!(edge(std::slice::from_ref(&two), &after).landed, ["!2"]);
        assert!(edge(&[], &after).landed.is_empty(), "never read open");
        // The caller keeps it out where the flags say the merge.
        let mut quiet = edge(&[two], &after);
        quiet.landed.clear();
        assert!(quiet.news(&after).is_empty());
    }

    /// T-744: each edge the crown hears, in its words, naming its PR — and
    /// nothing for a first read's facts, an opening, or a comment.
    #[test]
    fn the_crown_hears_each_forge_edge_by_its_pr() {
        let gl = |n: u64, pipeline: Pipeline| {
            let mut p = pr(n, PrState::Open, pipeline);
            p.forge = Forge::GitLab;
            p
        };
        let running = gl(1, Pipeline::Running);
        let words = |old: &[Pr], new: &[Pr]| edge(old, new).news(new);
        assert_eq!(
            words(std::slice::from_ref(&running), &[gl(1, Pipeline::Passed)]),
            ["pipeline passed !1"]
        );
        let e = edge(
            std::slice::from_ref(&running),
            &[gl(1, Pipeline::Waiting("deploy:production".into()))],
        );
        assert_eq!(e.waiting.as_deref(), Some("deploy:production"));
        assert_eq!(
            e.news(&[gl(1, Pipeline::Waiting("deploy:production".into()))]),
            ["waiting on deploy:production !1"]
        );
        // A manual job GitLab named nothing for; the same wait read twice
        // is no second edge.
        let manual = gl(1, Pipeline::Waiting(String::new()));
        assert_eq!(
            words(std::slice::from_ref(&running), std::slice::from_ref(&manual)),
            ["waiting on a manual job !1"]
        );
        assert!(words(std::slice::from_ref(&manual), std::slice::from_ref(&manual)).is_empty());
        assert_eq!(
            words(std::slice::from_ref(&running), &[gl(1, Pipeline::Failed("test".into()))]),
            ["pipeline failed !1: test"]
        );
        let mut conflicted = gl(1, Pipeline::Running);
        conflicted.conflict = true;
        assert_eq!(
            words(std::slice::from_ref(&running), std::slice::from_ref(&conflicted)),
            ["conflict !1"]
        );
        assert_eq!(
            words(std::slice::from_ref(&conflicted), std::slice::from_ref(&running)),
            ["conflict cleared !1"]
        );
        let mut approved = gl(1, Pipeline::Running);
        approved.review = Review::Approved;
        assert_eq!(
            words(std::slice::from_ref(&running), std::slice::from_ref(&approved)),
            ["review approved !1"]
        );
        let mut asked = approved.clone();
        asked.review = Review::ChangesRequested;
        assert_eq!(words(std::slice::from_ref(&approved), &[asked]), ["changes requested !1"]);
        let mut required = approved.clone();
        required.review = Review::Required;
        assert!(
            words(std::slice::from_ref(&approved), &[required]).is_empty(),
            "a dismissal is no verdict"
        );
        // A comment moves `updated_ms` alone: no edge.
        let mut touched = running.clone();
        touched.updated_ms += 1000;
        assert!(!edge(std::slice::from_ref(&running), &[touched]).any());
        // A PR bound already conflicted and approved says both (T-769);
        // bound running and clean, its opening alone, which is not news.
        let mut fresh = gl(2, Pipeline::Running);
        fresh.conflict = true;
        fresh.review = Review::Approved;
        let e = edge(std::slice::from_ref(&running), &[fresh.clone(), running.clone()]);
        assert!(!e.opened.is_empty());
        assert_eq!(e.news(&[fresh, running.clone()]), ["conflict !2", "review approved !2"]);
        let quiet = gl(2, Pipeline::Running);
        let e = edge(std::slice::from_ref(&running), &[quiet.clone(), running.clone()]);
        assert!(!e.opened.is_empty());
        assert!(e.news(&[quiet, running.clone()]).is_empty());
        // The merge commit reached an environment; several edges on one
        // pass are one list.
        let mut merged = gl(1, Pipeline::Passed);
        merged.state = PrState::Merged;
        let mut deployed = merged.clone();
        deployed.deployed = vec!["staging".into()];
        assert_eq!(words(std::slice::from_ref(&merged), &[deployed]), ["deployed to staging !1"]);
        let mut both = gl(1, Pipeline::Passed);
        both.review = Review::Approved;
        assert_eq!(
            words(std::slice::from_ref(&running), &[both]),
            ["pipeline passed !1", "review approved !1"]
        );
    }

    /// T-769, finding 4: a PR bound with a settled state says it as its
    /// first edge — a pipeline passed, failed or waiting beside a set whose
    /// own edge does not name it, a conflict, a verdict — and one read
    /// again after a read that missed it says nothing.
    #[test]
    fn a_freshly_bound_pr_says_its_settled_state() {
        let gl = |n: u64, state: PrState, pipeline: Pipeline| {
            let mut p = pr(n, state, pipeline);
            p.forge = Forge::GitLab;
            p
        };
        // B3: !1 merged, !2 bound already passed.
        let one = gl(1, PrState::Merged, Pipeline::Passed);
        let now = [gl(2, PrState::Open, Pipeline::Passed), one.clone()];
        let e = edge(std::slice::from_ref(&one), &now);
        assert!(!e.opened.is_empty() && e.passed);
        assert_eq!(e.news(&now), ["pipeline passed !2"]);
        // Beside an open PR that passed already: the set's edge is silent,
        // the new PR's is not.
        let green = gl(1, PrState::Open, Pipeline::Passed);
        let now = [gl(2, PrState::Open, Pipeline::Passed), green.clone()];
        let e = edge(std::slice::from_ref(&green), &now);
        assert!(!e.passed, "the set was green before");
        assert_eq!(e.news(&now), ["pipeline passed !2"]);
        let red = gl(1, PrState::Open, Pipeline::Failed("lint".into()));
        let now = [gl(2, PrState::Open, Pipeline::Failed("test".into())), red.clone()];
        assert_eq!(edge(std::slice::from_ref(&red), &now).news(&now), ["pipeline failed !2: test"]);
        let now = [gl(2, PrState::Open, Pipeline::Waiting("deploy".into())), red.clone()];
        assert_eq!(edge(std::slice::from_ref(&red), &now).news(&now), ["waiting on deploy !2"]);
        // C1: !3 bound in conflict, with changes requested.
        let mut three = gl(3, PrState::Open, Pipeline::Running);
        three.conflict = true;
        three.review = Review::ChangesRequested;
        let now = [three.clone()];
        let e = edge(&[], &now);
        assert_eq!(e.news(&now), ["conflict !3", "changes requested !3"]);
        assert!(e.conflict[0].fresh && e.conflict[0].on);
        // Held before (a read missed it): no opening, nothing said.
        let held = [("o/r".to_string(), 3)];
        let e = super::edge(&[], &now, &held);
        assert!(!e.any(), "{e:?}");
    }

    /// T-769, finding 6: a conflict is said once, and its clear only after
    /// it; the first live read against `forge.json`'s set says neither for
    /// a PR that set held, and a newly bound PR's conflict is still said.
    #[test]
    fn a_conflict_cleared_follows_a_conflict_said() {
        let t = ulid::Ulid::new();
        let mut s = State::default();
        let clean = pr(4, PrState::Open, Pipeline::Running);
        let mut stuck = clean.clone();
        stuck.conflict = true;
        let said = |s: &mut State, old: &Pr, new: &Pr, baseline: bool| {
            let mut e = edge(std::slice::from_ref(old), std::slice::from_ref(new));
            s.say_conflicts(t, &mut e, baseline);
            e.news(std::slice::from_ref(new))
        };
        // D1: the set held !4 in conflict from a read whose edge said
        // nothing; its clear is no news.
        assert!(said(&mut s, &stuck, &clean, false).is_empty(), "a phantom clear");
        // Said, then cleared: both.
        assert_eq!(said(&mut s, &clean, &stuck, false), ["conflict #4"]);
        assert_eq!(said(&mut s, &stuck, &clean, false), ["conflict cleared #4"]);
        assert!(said(&mut s, &stuck, &clean, false).is_empty(), "cleared once");
        // A restart: the loaded set is a baseline both ways.
        let mut s = State::default();
        assert!(said(&mut s, &clean, &stuck, true).is_empty());
        assert!(said(&mut s, &stuck, &clean, false).is_empty());
        let mut s = State::default();
        assert_eq!(said(&mut s, &clean, &stuck, false), ["conflict #4"]);
        let mut s = State::default();
        assert!(said(&mut s, &stuck, &clean, true).is_empty(), "said by the last daemon, or not");
        // A PR bound since: its conflict is said on the baseline read too,
        // and said once.
        let mut s = State::default();
        let mut e = edge(&[], std::slice::from_ref(&stuck));
        s.say_conflicts(t, &mut e, true);
        assert_eq!(e.news(std::slice::from_ref(&stuck)), ["conflict #4"]);
        let mut again = edge(&[], std::slice::from_ref(&stuck));
        s.say_conflicts(t, &mut again, false);
        assert!(again.conflict.is_empty(), "said already");
        assert_eq!(said(&mut s, &stuck, &clean, false), ["conflict cleared #4"]);
    }

    /// T-769, finding 3, and T-782: one pass's words in the forge's order
    /// — merged, the pipeline, deployed, waiting: the deployment to staging
    /// ran before the job the pipeline waits on.
    #[test]
    fn a_pass_says_merged_before_its_pipeline_and_its_deployment() {
        let open = pr(1, PrState::Open, Pipeline::Passed);
        let mut merged = pr(1, PrState::Merged, Pipeline::Waiting("deploy:production".into()));
        merged.merge_oid = "m1".into();
        merged.deployed = vec!["staging".into()];
        let now = [merged];
        assert_eq!(
            edge(&[open], &now).news(&now),
            ["merged #1", "deployed to staging #1", "waiting on deploy:production #1"]
        );
        // A bound PR's failure is a pipeline word, its wait comes last.
        let red = pr(2, PrState::Open, Pipeline::Failed("lint".into()));
        let mut held = pr(3, PrState::Open, Pipeline::Waiting("deploy".into()));
        held.repo = "o/s".into();
        let mut fresh_red = red.clone();
        fresh_red.repo = "o/t".into();
        let now = [held, fresh_red];
        assert_eq!(
            edge(&[], &now).news(&now),
            ["pipeline failed #2: lint", "waiting on deploy #3"]
        );
    }

    /// T-782, lab finding 1: on a ticket of two repositories, the merged
    /// PR's own wait is said while the other PR is open — the set's
    /// pipeline is the open one's — and said once; an open PR's wait names
    /// that PR alone.
    #[test]
    fn a_merged_prs_wait_is_its_own_beside_an_open_one() {
        let gl = |repo: &str, n: u64, state: PrState, pipeline: Pipeline| {
            let mut p = pr(n, state, pipeline);
            p.forge = Forge::GitLab;
            p.repo = repo.into();
            if state == PrState::Merged {
                p.merge_oid = format!("m{n}");
            }
            p
        };
        let open = [
            gl("g/web", 2, PrState::Open, Pipeline::Passed),
            gl("g/api", 8, PrState::Open, Pipeline::Running),
        ];
        let merged = [
            gl("g/web", 2, PrState::Merged, Pipeline::Running),
            gl("g/api", 8, PrState::Open, Pipeline::Running),
        ];
        let e = edge(&open, &merged);
        assert_eq!(e.landed, ["!2"]);
        assert!(e.stopped.is_empty() && e.waiting.is_none());
        let mut waits = merged.clone();
        waits[0].pipeline = Pipeline::Waiting("deploy:production".into());
        waits[0].deployed = vec!["staging".into()];
        let e = edge(&merged, &waits);
        assert!(e.waiting.is_none(), "api's !8 is running: {e:?}");
        assert_eq!(e.news(&waits), ["deployed to staging !2", "waiting on deploy:production !2"]);
        assert!(!edge(&waits, &waits).any(), "said once");
        // api's !8 stops on its own job: its label alone.
        let mut both = waits.clone();
        both[1].pipeline = Pipeline::Waiting("approve".into());
        assert_eq!(edge(&waits, &both).news(&both), ["waiting on approve !8"]);
        // Merged and waiting on the same read it was open: both words.
        let e = edge(&open, &waits);
        assert_eq!(
            e.news(&waits),
            ["merged !2", "deployed to staging !2", "waiting on deploy:production !2"]
        );
    }

    #[test]
    fn the_forge_is_current_with_no_forge_after_a_read_or_on_a_look_that_found_nothing() {
        let t = ulid::Ulid::new();
        let remote = Remote { host: "github.com".into(), path: "o/r".into() };
        let cli = Cli { forge: Forge::GitHub, bin: PathBuf::from("/bin/gh") };
        // No forge: every remote parsed to none.
        let mut s = State::default();
        s.remotes.insert(PathBuf::from("/r"), None);
        assert!(s.fresh(t, 5_000, &[]), "no forge reads this board");
        // A forge: nothing read, nothing known.
        let mut s = State::default();
        s.remotes.insert(PathBuf::from("/r"), Some(remote));
        s.probes.insert(
            "github.com".into(),
            HostProbe { cli: Some(cli), at: Instant::now(), viewer: None },
        );
        assert!(!s.fresh(t, 5_000, &[]), "never read");
        // Read before the turn ended, on a basis that had pushed one ref.
        let pushed = vec!["o/r refs/remotes/origin/msmn/T-1 abc".to_string()];
        s.at_ms.insert(t, 4_000);
        s.basis.insert(t, Basis { pushed: Some(pushed.clone()), opened: Vec::new() });
        assert!(!s.fresh(t, 5_000, &[]), "no look yet: wait for the read");
        // The look after the turn found the same: current, no network.
        s.local.insert(
            t,
            Local { at_ms: 5_100, pushed: Some(pushed.clone()), tail_opened: Some(false) },
        );
        assert!(s.fresh(t, 5_000, &[]), "nothing pushed, nothing opened");
        // A URL the read did not have, or a `pr create` in the unread tail.
        let url = "https://github.com/o/r/pull/7".to_string();
        assert!(!s.fresh(t, 5_000, std::slice::from_ref(&url)), "a PR opened since");
        s.basis.get_mut(&t).unwrap().opened.push(url.clone());
        assert!(s.fresh(t, 5_000, std::slice::from_ref(&url)), "that URL the read had");
        s.local.get_mut(&t).unwrap().tail_opened = Some(true);
        assert!(!s.fresh(t, 5_000, &[]), "a pr create the ledger has not reached");
        s.local.get_mut(&t).unwrap().tail_opened = None;
        assert!(!s.fresh(t, 5_000, &[]), "an unreadable tail is unknown");
        // Pushed in the turn: waits for a read begun after the turn's end.
        s.local.insert(
            t,
            Local {
                at_ms: 5_100,
                pushed: Some(vec!["o/r refs/remotes/origin/msmn/T-1 def".into()]),
                tail_opened: Some(false),
            },
        );
        assert!(!s.fresh(t, 5_000, &[]), "pushed: the read is owed");
        s.at_ms.insert(t, 5_000);
        assert!(s.fresh(t, 5_000, &[]), "a read at the turn's end answers it");
        // A look from before the turn's end proves nothing about it.
        s.at_ms.insert(t, 4_000);
        s.local.insert(t, Local { at_ms: 4_900, pushed: Some(pushed), tail_opened: Some(false) });
        assert!(!s.fresh(t, 5_000, &[]), "the look predates the turn's end");
        // Git could not say what was pushed: unknown, never unchanged.
        s.basis.insert(t, Basis { pushed: None, opened: Vec::new() });
        s.local.insert(t, Local { at_ms: 5_100, pushed: None, tail_opened: Some(false) });
        assert!(!s.fresh(t, 5_000, &[]));
    }

    #[test]
    fn the_floor_holds_a_repository_read_inside_the_burst_and_one_that_failed() {
        let mut s = State::default();
        let burst = Duration::from_millis(BURST_MS);
        assert!(s.due_for("github.com/o/r", burst), "never asked");
        s.asked.insert("github.com/o/r".into(), Instant::now());
        assert!(!s.due_for("github.com/o/r", burst), "inside the floor");
        s.asked
            .insert("github.com/o/r".into(), Instant::now() - Duration::from_millis(BURST_MS + 1));
        assert!(s.due_for("github.com/o/r", burst), "past the floor");
        s.errors.push(("github.com/o/r".into(), "boom".into()));
        assert!(!s.due_for("github.com/o/r", burst), "a failing repository waits for the backoff");
    }

    #[test]
    fn a_pipeline_in_flight_reads_its_repository_every_fifteen_seconds() {
        let mut s = State::default();
        let (open, merged, settled, closed) =
            (ulid::Ulid::new(), ulid::Ulid::new(), ulid::Ulid::new(), ulid::Ulid::new());
        s.sets.insert(open, vec![pr(1, PrState::Open, Pipeline::Running)]);
        // Deployed to staging, waiting on the manual production job.
        s.sets.insert(
            merged,
            vec![pr(2, PrState::Merged, Pipeline::Waiting("deploy:production".into()))],
        );
        s.sets.insert(settled, vec![pr(3, PrState::Merged, Pipeline::Passed)]);
        s.sets.insert(closed, vec![pr(4, PrState::Closed, Pipeline::Running)]);
        assert_eq!(s.busy(), HashSet::from([open, merged]));
        // The minute's pass is not due, the busy repository is at 15 s.
        s.last = Some(Instant::now());
        assert!(!s.due());
        s.asked.insert("github.com/o/r".into(), Instant::now() - Duration::from_secs(14));
        assert!(!s.due_for("github.com/o/r", BUSY_EVERY));
        s.asked.insert("github.com/o/r".into(), Instant::now() - BUSY_EVERY);
        assert!(s.due_for("github.com/o/r", BUSY_EVERY));
        // A repository whose call failed waits for the backoff.
        s.errors.push(("github.com/o/r".into(), "boom".into()));
        assert!(!s.due_for("github.com/o/r", BUSY_EVERY));
        assert!(BUSY_EVERY < EVERY && EVERY < BACKOFF);
    }

    #[test]
    fn a_push_moves_what_a_ticket_pushed_and_a_commit_alone_does_not() {
        let dir = std::env::temp_dir().join(format!("msmn-forge-pushed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(args)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .stderr(Stdio::null())
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}");
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        git(&["init", "-q", "-b", "main"]);
        std::fs::write(dir.join("a"), "a").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "one"]);
        let legs = vec![LegQuery { repo_path: "o/r".into(), dir: dir.clone() }];
        let now = |legs: &[LegQuery]| {
            let logs: Vec<Vec<String>> = legs.iter().map(|l| reflog(&l.dir)).collect();
            pushed(legs, &logs)
        };
        assert_eq!(now(&legs), Some(Vec::new()), "nothing pushed");
        std::fs::write(dir.join("a"), "b").unwrap();
        git(&["commit", "-qam", "two"]);
        assert_eq!(now(&legs), Some(Vec::new()), "a commit is not a push");
        // A push updates the remote-tracking ref to the pushed commit.
        let tip = git(&["rev-parse", "HEAD"]);
        git(&["update-ref", "refs/remotes/origin/msmn/T-1", &tip]);
        assert_eq!(now(&legs), Some(vec![format!("o/r refs/remotes/origin/msmn/T-1 {tip}")]));
        // A ref at a commit this worktree never held is someone else's.
        git(&["commit", "-q", "--allow-empty", "-m", "three"]);
        let other = git(&["commit-tree", "-m", "x", &format!("{tip}^{{tree}}")]);
        git(&["update-ref", "refs/remotes/origin/other", &other]);
        assert_eq!(now(&legs).unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_reflog_is_read_off_a_worktree_and_nothing_off_a_missing_one() {
        let dir = std::env::temp_dir().join(format!("msmn-forge-reflog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| {
            let st = Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(args)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap();
            assert!(st.success(), "git {args:?}");
        };
        git(&["init", "-q", "-b", "main"]);
        std::fs::write(dir.join("a"), "a").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "one"]);
        std::fs::write(dir.join("a"), "b").unwrap();
        git(&["commit", "-qam", "two"]);
        let log = reflog(&dir);
        assert_eq!(log.len(), 2, "{log:?}");
        assert!(log.iter().all(|o| o.len() == 40));
        assert!(reflog(&dir.join("nope")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_state_is_due_on_its_cadence_and_backs_off_after_trouble() {
        let mut s = State::default();
        assert!(s.due(), "never read is due");
        s.last = Some(Instant::now());
        assert!(!s.due());
        s.inflight = true;
        s.last = None;
        assert!(!s.due(), "one pass at a time");
        s.inflight = false;
        s.errors.push(("github.com/o/r".into(), "gh: not logged in".into()));
        assert_eq!(s.trouble().as_deref(), Some("github.com/o/r: gh: not logged in"));
        assert!(!s.present());
    }

    #[test]
    fn a_missing_cli_is_none_and_a_present_one_is_found_on_the_captured_path() {
        let dir = std::env::temp_dir().join(format!("msmn-forge-cli-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("glab"), "#!/bin/sh\n").unwrap();
        let path = dir.to_string_lossy().into_owned();
        assert_eq!(find_cli(Forge::GitLab, Some(&path)).map(|c| c.bin), Some(dir.join("glab")));
        assert_eq!(
            find_cli(Forge::GitHub, Some(&path)).map(|c| c.forge),
            find_cli(Forge::GitHub, None).map(|c| c.forge),
            "falls through to the daemon's own PATH"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A fresh daemon has no probe (T-752): the hosts it probes on its
    /// first pass are listed on that same pass once a CLI answers, and a
    /// host whose probe found none is not.
    #[test]
    fn a_fresh_state_lists_the_probed_hosts_on_the_same_pass() {
        let remote = |host: &str, path: &str| Remote { host: host.into(), path: path.into() };
        let mut q = Query {
            pending: vec![
                PendingRepo { remote: remote("gitlab.lab", "g/a"), dir: "/r".into() },
                PendingRepo { remote: remote("gitlab.lab", "g/b"), dir: "/r/b".into() },
                PendingRepo { remote: remote("github.com", "o/r"), dir: "/r/c".into() },
            ],
            ..Default::default()
        };
        assert!(q.repos.is_empty(), "nothing to list before the probe");
        let glab = Cli { forge: Forge::GitLab, bin: "/bin/glab".into() };
        let answered = vec![
            (
                "gitlab.lab".to_string(),
                HostProbe {
                    cli: Some(glab.clone()),
                    at: Instant::now(),
                    viewer: Some("me".into()),
                },
            ),
            ("github.com".to_string(), HostProbe { cli: None, at: Instant::now(), viewer: None }),
        ];
        list_probed(&mut q, &answered);
        let keys: Vec<String> = q.repos.iter().map(RepoQuery::key).collect();
        assert_eq!(keys, vec!["gitlab.lab/g/a", "gitlab.lab/g/b"]);
        assert!(q.repos.iter().all(|r| r.cli == glab && r.viewer.as_deref() == Some("me")));
        assert_eq!(q.repos[1].dir, PathBuf::from("/r/b"));
        assert!(q.pending.is_empty());
    }

    /// A repository whose call failed keeps the previous read's PRs; one
    /// that answered says what it says now (T-752).
    #[test]
    fn a_failed_call_carries_its_repositorys_prs_over() {
        let mut other = pr(9, PrState::Open, Pipeline::Running);
        other.repo = "o/other".into();
        let old = vec![pr(1, PrState::Open, Pipeline::Running), other.clone()];
        let new = vec![pr(1, PrState::Merged, Pipeline::Passed)];
        let set = carry(Some(&old), new.clone(), &["o/other"]);
        assert_eq!(set.len(), 2);
        assert!(set.iter().any(|p| p.number == 1 && p.state == PrState::Merged));
        assert!(set.contains(&other));
        assert_eq!(carry(Some(&old), new.clone(), &[]), new, "every call answered");
        assert_eq!(carry(None, new.clone(), &["o/r"]), new, "a first read carries nothing");
    }

    /// `forge.json` round-trips the sets and the settled commits (T-752),
    /// leaves `at_ms` empty (no live read yet), drops a ticket the board no
    /// longer has, and a newer build's file is refused.
    #[test]
    fn the_forge_file_round_trips_and_a_newer_one_is_refused() {
        let (a, gone) = (ulid::Ulid(1), ulid::Ulid(2));
        let mut s = State::default();
        let mut merged = pr(3, PrState::Merged, Pipeline::Failed("deploy:prod".into()));
        merged.merge_oid = "m".into();
        merged.deployed = vec!["staging".into()];
        merged.conflict = true;
        merged.review = Review::Approved;
        let set = forge::ticket_set(vec![
            merged,
            pr(4, PrState::Draft, Pipeline::Waiting(String::new())),
        ]);
        s.sets.insert(a, set.clone());
        s.sets.insert(gone, vec![pr(5, PrState::Open, Pipeline::None)]);
        s.at_ms.insert(a, 7);
        s.commit_pipelines.insert(("o/r".into(), "m".into()), Pipeline::Passed);
        s.deployed.insert(("o/r".into(), "m".into()), vec!["staging".into()]);
        s.deployed.insert(("o/r".into(), "n".into()), vec!["prod".into()]);
        let text = serde_json::to_string_pretty(&ForgeFile::of(&s)).unwrap();
        assert!(!text.contains("at_ms"), "no time in the file: {text}");
        let back: ForgeFile = crate::store::parse_versioned(&text, FORGE_SCHEMA).unwrap();
        let mut restored = State::default();
        restored.restore(back, |t| t == a);
        assert_eq!(restored.sets.get(&a), Some(&set));
        assert!(!restored.sets.contains_key(&gone));
        assert!(restored.at_ms.is_empty());
        assert_eq!(restored.commit_pipelines, s.commit_pipelines);
        assert_eq!(restored.deployed, s.deployed);
        assert_eq!(
            text,
            serde_json::to_string_pretty(&ForgeFile::of(&{
                let mut r = State::default();
                r.restore(crate::store::parse_versioned(&text, FORGE_SCHEMA).unwrap(), |_| true);
                r
            }))
            .unwrap(),
            "a load and a save write the same bytes"
        );
        let empty = serde_json::to_string(&ForgeFile::default_of_schema()).unwrap();
        assert_eq!(empty, format!("{{\"schema_version\":{FORGE_SCHEMA}}}"));
        let newer = format!("{{\"schema_version\": {}}}", FORGE_SCHEMA + 1);
        assert!(matches!(
            crate::store::parse_versioned::<ForgeFile>(&newer, FORGE_SCHEMA),
            Err((Some(_), _))
        ));
        assert!(matches!(
            crate::store::parse_versioned::<ForgeFile>("{", FORGE_SCHEMA),
            Err((None, _))
        ));
    }

    /// The back-compat fixture (T-752): schema 1 as this build first wrote
    /// it, missing optional fields, and a word a later build may add. The
    /// PR with an unknown state is dropped; the file is not.
    #[test]
    fn a_schema_one_forge_file_reads() {
        let t = ulid::Ulid(1);
        let text = format!(
            r#"{{
  "schema_version": 1,
  "sets": {{
    "{t}": [
      {{"forge": "gitlab", "repo": "g/p", "number": 1, "url": "https://gitlab.lab/g/p/-/merge_requests/1",
        "state": "merged", "head_oid": "h", "head_ref": "msmn/T-3-x", "merge_oid": "m",
        "review": "none", "pipeline": "passed", "updated_ms": 5, "deployed": ["production"]}},
      {{"forge": "gitlab", "repo": "g/p", "number": 2, "state": "queued"}}
    ]
  }},
  "commits": [{{"repo": "g/p", "oid": "m", "pipeline": "passed"}}],
  "later_field": true
}}"#
        );
        let file: ForgeFile = crate::store::parse_versioned(&text, FORGE_SCHEMA).unwrap();
        let mut s = State::default();
        s.restore(file, |_| true);
        let set = &s.sets[&t];
        assert_eq!(set.len(), 1);
        assert_eq!(
            (set[0].label(), set[0].state, set[0].pipeline.clone()),
            ("!1".into(), PrState::Merged, Pipeline::Passed)
        );
        assert_eq!(set[0].deployed, vec!["production".to_string()]);
        assert_eq!(s.commit_pipelines[&("g/p".to_string(), "m".to_string())], Pipeline::Passed);
        assert!(s.merged_at(t, "g/p", "h").is_some());
    }
}
