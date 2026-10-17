//! The forge sample (T-739): the daemon's half of `mesimon_core::forge`.
//!
//! On the slow bucket, at most once a minute, a worker thread asks the
//! forge CLI the person already signed in to — `gh` on github.com, `glab`
//! on a GitLab host, found on the captured shell `PATH` — for the pull
//! requests the signed-in account authored in each repository the board's
//! tickets touch: one list call per repository, never one per ticket. The
//! worker then binds each PR to a ticket by a fact (the head a commit a
//! worktree leg wrote, the ticket's own branch, a URL a `pr create` of
//! the ticket's own session printed) and hands the writer thread each
//! ticket's set. Reads only: no `create`, no merge, no comment, no push,
//! and the CLI's own update notifier and prompts are off, so nothing is
//! written anywhere on mesimon's account (promise 1).
//!
//! Every call is fenced as a fetch is (`gitstatus::fetch`): its own
//! session, a deadline, the process group killed on overrun, every prompt
//! door shut. A repository whose call failed is reported, not retried in a
//! loop: that repository waits `BACKOFF` (T-800), the rest are read.

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
/// `ssh -G`'s deadline (T-838): it runs on the writer thread, once per
/// SSH alias, and evaluates `Match exec` blocks of the person's own
/// config, so a slow one must not hold the board for a CLI call's 20 s.
const SSH_TIMEOUT: Duration = Duration::from_secs(3);
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
/// How many PRs one list call brings back. A list that comes back full may
/// have left an older PR out (T-800): a ticket's PR it does not name is
/// read by itself (`unlisted`).
const LIST_LIMIT: usize = 100;
/// The fields one GitHub PR is read with, listed or viewed.
const GH_FIELDS: &str = "number,title,url,state,isDraft,headRefName,headRefOid,mergeCommit,\
                         mergeable,reviewDecision,updatedAt,mergedAt,statusCheckRollup";
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

/// A cadence in milliseconds from a test seam, else `default`.
fn seam_ms(var: &str, default: Duration) -> Duration {
    std::env::var(var)
        .ok()
        .and_then(|v| v.parse().ok())
        .map(Duration::from_millis)
        .unwrap_or(default)
}

/// `MESIMON_FORGE_EVERY_MS`: the sample cadence, a test seam.
pub fn every() -> Duration {
    seam_ms("MESIMON_FORGE_EVERY_MS", EVERY)
}

/// `MESIMON_FORGE_BUSY_MS`: the busy cadence, a test seam.
pub fn busy_every() -> Duration {
    seam_ms("MESIMON_FORGE_BUSY_MS", BUSY_EVERY)
}

/// `MESIMON_FORGE_BACKOFF_MS`: the backoff, a test seam.
pub fn backoff() -> Duration {
    seam_ms("MESIMON_FORGE_BACKOFF_MS", BACKOFF)
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
    /// Why nothing reads the host, in the advisory row's words with the
    /// fix (`forge::sign_in_trouble`, `forge::missing_trouble`): a CLI that
    /// did not confirm its sign-in (T-798: no network yet, a timeout, a
    /// stale token, never signed in), or no CLI on PATH at all. On a board
    /// that merges through pull requests, the only one that reads a forge,
    /// every such host is trouble (T-810): asked again on the backoff, said
    /// on the advisory row and in `ForgeStatus.trouble`, never "no forge".
    pub lost: Option<String>,
}

impl HostProbe {
    /// No CLI was found for the host, and none was lost: nothing reads it.
    /// Since T-810 the probe always says why, so this is a probe from
    /// before one was made.
    pub fn reads_nothing(&self) -> bool {
        self.cli.is_none() && self.lost.is_none()
    }

    pub fn stale(&self) -> bool {
        self.at.elapsed() >= if self.lost.is_some() { backoff() } else { PROBE_FOR }
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
        self.remote.key()
    }

    /// One `api` read of `path` on the repository's host.
    fn api(&self, path: &str) -> Result<String, String> {
        run(&self.cli.bin, &["api", "--hostname", &self.remote.host, path], Some(&self.dir))
    }

    /// The repository's path as GitLab's API names a project.
    fn project(&self) -> String {
        query_word(&self.remote.path)
    }
}

/// One leg of a ticket's worktree: the repository it is a checkout of, by
/// the remote's path, and where it stands.
#[derive(Debug, Clone)]
pub struct LegQuery {
    pub repo_path: String,
    /// Every path a PR of this repository may name (`State::repo_paths`,
    /// T-800): a commit made here binds a PR opened against the fork's
    /// upstream too.
    pub repo_paths: Vec<String>,
    pub dir: PathBuf,
    /// The commit the leg was cut at: a tip still there is no work of the
    /// ticket's, and no merged head is asked whether it carries it (T-799).
    pub base_oid: String,
}

/// A merged head and a branch tip, by repository path: the key a "does
/// this head carry this tip" answer is kept under (T-799). Both are
/// commits, so the answer never changes.
pub type LandedKey = (String, String, String);

/// One ticket's facts, to bind by.
#[derive(Debug, Clone)]
pub struct TicketQuery {
    pub ticket: ulid::Ulid,
    pub branch: String,
    pub legs: Vec<LegQuery>,
    pub opened: Vec<String>,
    /// Every repository the ticket's set can come from: its legs' remotes
    /// and the ones its sessions' URLs name (T-795).
    pub repos: Vec<Remote>,
    /// The ticket's set as the board holds it: a PR in it a full list did
    /// not name is read by itself (`unlisted`, T-800).
    pub held: Vec<Pr>,
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
    /// The environments each merge commit was deployed to as the last pass
    /// read it (T-801): what stands where this pass cannot say otherwise.
    pub deployed: HashMap<(String, String), Vec<String>>,
    /// Each ticket's answers to "does this merged head carry this tip"
    /// (T-799), so a pass asks git or the forge only what moved.
    pub landed: HashMap<ulid::Ulid, HashMap<LandedKey, bool>>,
    /// Git's answers to "does this deployed commit carry this merge commit"
    /// (T-801), so a pass asks only what moved.
    pub carried: HashMap<CarriedKey, bool>,
}

/// A repository path, a merge commit and a deployed commit: the key a "does
/// the deployed commit carry the merge" answer is kept under (T-801). Both
/// are commits, so the answer never changes.
pub type CarriedKey = (String, String, String);

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
    /// arrival, on every repository at once (T-768). A closed PR is no PR
    /// to wait for (T-797): the set merged is one merged and none open.
    pub merged: bool,
    /// PRs, by label, read merged that were read open before (T-757): what
    /// the crown hears of a merge the worktree flags cannot say, the caller
    /// keeping them only then.
    pub landed: Vec<String>,
    /// Environments the ticket reached since, as a whole (T-801): no PR
    /// open, and every merged PR's merge commit deployed there. One PR's
    /// progress is not the ticket's.
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
}

impl Edge {
    pub fn any(&self) -> bool {
        *self != Edge::default()
    }

    /// The openings in the crown's words (T-773): `opened !2`, said only
    /// on the wake of the turn that opened them.
    pub fn opening(&self) -> Vec<String> {
        self.opened.iter().map(|l| format!("opened {l}")).collect()
    }

    /// What the crown hears of the edge (T-744), one phrase per edge, each
    /// naming its PR: `merged !2`, `pipeline passed !1`, `waiting on
    /// deploy:production !1`, `deployed to staging !1`, `conflict !1`,
    /// `conflict cleared !1`, `review approved !1`, `changes requested !1`,
    /// `pipeline failed !1: test-linux`, in the forge's order: merged, then
    /// the pipelines, then deployed, then waiting (T-769, T-782) — a merge
    /// commit's deployment ran before the job its pipeline now waits on.
    /// Never an opening, which wakes no one by itself: the worker's turn
    /// end carries it (`opening`, T-773). A newly bound PR's settled state
    /// is said (T-769). A merge is said only where the caller left it in
    /// `landed`: the flags' merged wake says the one that lands the
    /// worktree's branch (T-757). `new` is the read the edge was taken on,
    /// for the labels.
    pub fn news(&self, new: &[Pr]) -> Vec<String> {
        let labels = |pick: &dyn Fn(&Pr) -> bool| -> String {
            new.iter().filter(|p| pick(p)).map(Pr::label).collect::<Vec<_>>().join(" ")
        };
        let tail =
            |label: String| if label.is_empty() { String::new() } else { format!(" {label}") };
        let mut out: Vec<String> = self.landed.iter().map(|l| format!("merged {l}")).collect();
        // The open PRs' failure where one is open, as `failed` is theirs
        // (T-795): a merged PR's settled failure is not this one.
        let open = new.iter().any(|p| p.state.is_open());
        if let Some(job) = &self.failed {
            let failing =
                |p: &Pr| matches!(p.pipeline, Pipeline::Failed(_)) && (!open || p.state.is_open());
            out.push(failed_words(&labels(&failing), job));
        }
        if self.passed {
            out.push(format!("pipeline passed{}", tail(labels(&|p| p.state.is_open()))));
        }
        // A newly bound PR's pipeline, where the set's own edge above did
        // not already name it.
        for (label, pipeline) in &self.bound {
            match pipeline {
                Pipeline::Passed if !self.passed => out.push(format!("pipeline passed {label}")),
                Pipeline::Failed(job) if self.failed.is_none() => {
                    out.push(failed_words(label, job))
                }
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

/// The environments a ticket's set has reached as a whole (T-801): none
/// while a PR is open, else those every merged PR's merge commit is
/// deployed to. A deploy of a later commit carries an earlier merge
/// (`forge::deployed_after`), so a batched deploy brings every PR it holds;
/// a closed PR waits for nothing (T-797). This replaced T-768's "each
/// repository's newest merged PR", which took an older merge being in the
/// newer one's history on trust.
fn arrived_envs(set: &[Pr]) -> Vec<String> {
    if set.iter().any(|p| p.state.is_open()) {
        return Vec::new();
    }
    let merged: Vec<&Pr> = set.iter().filter(|p| p.state == PrState::Merged).collect();
    let Some((first, rest)) = merged.split_first() else { return Vec::new() };
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
    // A closed PR stays in the set (T-797) and waits for nothing.
    let all_merged = |s: &[Pr]| {
        s.iter().any(|p| p.state == PrState::Merged) && !s.iter().any(|p| p.state.is_open())
    };
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
    // The open set's own: a merged commit's pipeline passing is its deploy
    // run finishing, never a ticket's pipeline passing (T-767).
    let (then_open, now_open) = (open_of(old), open_of(new));
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
        passed: now_open == Pipeline::Passed
            && (then_open != Pipeline::Passed
                || kept.iter().any(|(o, p)| o.head_oid != p.head_oid)),
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
            .map(|(_, p)| *p)
            .chain(fresh.iter().filter(|p| p.conflict).copied())
            .map(|p| Conflict {
                repo: p.repo.clone(),
                number: p.number,
                label: p.label(),
                on: p.conflict,
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
    /// Each queried ticket's repositories, as its query named them.
    pub repos_of: HashMap<ulid::Ulid, Vec<Remote>>,
    /// Each queried ticket's "does this merged head carry this tip" answers
    /// this pass stood on (T-799), kept or new: what the next pass is told.
    pub landed: HashMap<ulid::Ulid, HashMap<LandedKey, bool>>,
    /// The "does this deployed commit carry this merge" answers this pass
    /// stood on (T-801), kept or new.
    pub carried: HashMap<CarriedKey, bool>,
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
pub fn pushed(legs: &[LegQuery], logs: &[Vec<Moved>]) -> Option<Vec<String>> {
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
        let log: std::collections::HashSet<&str> = log.iter().map(|m| m.oid.as_str()).collect();
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
    let logs: Vec<Vec<Moved>> = legs.iter().map(|l| reflog(&l.dir)).collect();
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
    let mut logs: Vec<Vec<Vec<Moved>>> = Vec::new();
    for t in &q.tickets {
        out.repos_of.insert(t.ticket, t.repos.clone());
        let legs: Vec<Vec<Moved>> = t.legs.iter().map(|l| reflog(&l.dir)).collect();
        out.basis
            .insert(t.ticket, Basis { pushed: pushed(&t.legs, &legs), opened: t.opened.clone() });
        logs.push(legs);
    }
    let mut prs: Vec<Pr> = Vec::new();
    let mut full: HashSet<String> = HashSet::new();
    for r in &q.repos {
        let error = match list(r) {
            Ok(mut found) => {
                if found.len() >= LIST_LIMIT {
                    full.insert(r.remote.path.clone());
                }
                prs.append(&mut found);
                None
            }
            Err(e) => Some(e),
        };
        out.repos.push(RepoReport { key: r.key(), path: r.remote.path.clone(), error });
    }
    // What each repository's wanted environments run, read once a pass and
    // only where a set holds a merged PR there (T-801), and git's answers
    // on what a deployed commit carries.
    let mut env_reads: HashMap<String, Vec<forge::EnvRead>> = HashMap::new();
    let mut carried: HashMap<CarriedKey, bool> = HashMap::new();
    for (t, logs) in q.tickets.iter().zip(logs) {
        // Only the commits a leg wrote (T-796): the one it was cut at and
        // the ones a rebase, a pull or a reset brought in are the base's,
        // and under a fast-forward merge one of those is a merged PR's head.
        let facts = Facts {
            branch: t.branch.clone(),
            made: t
                .legs
                .iter()
                .zip(logs)
                .map(|(l, log)| {
                    let wrote = log.into_iter().filter(|m| m.wrote).map(|m| m.oid).collect();
                    (l.repo_paths.clone(), wrote)
                })
                .collect(),
            opened: t.opened.clone(),
        };
        let mut bound: Vec<Pr> = prs.iter().filter(|p| forge::binds(p, &facts)).cloned().collect();
        // A PR a full list cut off is read by itself, and keeps its last
        // reading where that read fails (T-800).
        let cut: Vec<Pr> = unlisted(&t.held, &bound, &full).into_iter().cloned().collect();
        for old in cut {
            let read = q
                .repos
                .iter()
                .find(|r| r.remote.path == old.repo)
                .and_then(|r| one_pr(r, old.number));
            bound.push(read.unwrap_or(old));
        }
        let mut set = forge::ticket_set(bound);
        // GitLab's list lags the MR by minutes: a head after a push (T-784),
        // a state after a merge (T-787). Every MR it calls open is read by
        // itself, a handful of calls on a board, and the read's state, head,
        // conflict and merge commit win; its head pipeline is the pipeline.
        // A failed read keeps the list's row, and the T-780 gate holds it.
        // The rest are asked by sha, one call each.
        let mut read = false;
        for pr in set.iter_mut().filter(|p| p.forge == Forge::GitLab && p.state.is_open()) {
            let Some(r) = q.repos.iter().find(|r| r.remote.path == pr.repo) else { continue };
            let head = gitlab_mr(r, pr.number).and_then(|one| {
                read = true;
                pr.take_read(&one.pr);
                one.pipeline
            });
            if !pr.state.is_open() {
                // Merged or closed after all: `merged_reads` reads the
                // merge commit's pipeline, never the head's.
                pr.pipeline = Pipeline::Unknown;
                continue;
            }
            pr.pipeline = match head {
                Some((pipeline, id)) => named_job(r, pipeline, id),
                None => gitlab_pipeline(r, &pr.head_oid),
            };
        }
        if read {
            // A read's state and time reorder the set, and a closed one
            // sheds its pipeline, as the list's would.
            set = forge::ticket_set(set);
        }
        merged_reads(
            &mut set,
            &q,
            now_ms,
            |pr| match q.repos.iter().find(|r| r.remote.path == pr.repo) {
                Some(r) => commit_pipeline(r, &pr.merge_oid),
                None => Pipeline::Unknown,
            },
            |pr| {
                env_reads
                    .entry(pr.repo.clone())
                    .or_insert_with(|| match q.repos.iter().find(|r| r.remote.path == pr.repo) {
                        Some(r) => environments(r, &q.envs),
                        None => Vec::new(),
                    })
                    .clone()
            },
            |pr, latest| {
                let key = (pr.repo.clone(), pr.merge_oid.clone(), latest.to_string());
                let answer = q.carried.get(&key).copied().or_else(|| {
                    let r = q.repos.iter().find(|r| r.remote.path == pr.repo)?;
                    carries_here(&r.dir, latest, &pr.merge_oid)
                })?;
                carried.insert(key, answer);
                Some(answer)
            },
        );
        let landed = landed_reads(&mut set, t, q.landed.get(&t.ticket), |pr, tip| {
            pr_carries(q.repos.iter().find(|r| r.remote.path == pr.repo)?, pr, tip)
        });
        out.landed.insert(t.ticket, landed);
        out.sets.insert(t.ticket, set);
    }
    out.carried = carried;
    out
}

/// Each merged PR of a set, per PR (T-771): its merge commit's pipeline
/// (the build-and-deploy run on main) and the environments it is deployed
/// to. A pipeline is read until it passes, or until it has failed or been
/// cancelled for `SUPERSEDED_READ_MS` after its merge (T-800): a run is
/// retried, and a retried green one must not stay failed. A merge commit a
/// newer one of the same repository superseded — newer by when it merged —
/// has its pipeline read only for `SUPERSEDED_READ_MS` after its merge, so
/// a ticket's old PRs cost nothing once the forge has moved on.
///
/// Deployments are every merged PR's, on every pass a column asks for one
/// (T-801): `environments` names what each wanted environment of the PR's
/// repository runs, read once a pass for the repository, and `carries`
/// whether a deployed commit carries the PR's merge commit, asked of git
/// here (`forge::deployed_after`). A batched deploy can come days after an
/// old PR merged, and a rollback can take one back, so no PR settles.
pub fn merged_reads(
    set: &mut [Pr],
    q: &Query,
    now_ms: u64,
    mut pipeline: impl FnMut(&Pr) -> Pipeline,
    mut environments: impl FnMut(&Pr) -> Vec<forge::EnvRead>,
    mut carries: impl FnMut(&Pr, &str) -> Option<bool>,
) {
    let newest: Vec<(String, u64)> =
        forge::newest_merged(set).iter().map(|p| (p.repo.clone(), p.number)).collect();
    for pr in set.iter_mut().filter(|p| p.state == PrState::Merged && !p.merge_oid.is_empty()) {
        let superseded = !newest.iter().any(|(r, n)| *r == pr.repo && *n == pr.number);
        let young = now_ms.saturating_sub(pr.merged_order()) < SUPERSEDED_READ_MS;
        let live = !superseded || young;
        let key = (pr.repo.clone(), pr.merge_oid.clone());
        let last = q.commit_pipelines.get(&key);
        pr.pipeline = match last {
            Some(p @ Pipeline::Passed) => p.clone(),
            Some(p @ (Pipeline::Failed(_) | Pipeline::Cancelled(_))) if !young => p.clone(),
            _ if live => match pipeline(pr) {
                Pipeline::Unknown => last.cloned().unwrap_or(Pipeline::Unknown),
                p => p,
            },
            Some(p) => p.clone(),
            None => pr.pipeline.clone(),
        };
        let was = q.deployed.get(&key).cloned().unwrap_or_default();
        if q.envs.is_empty() {
            pr.deployed = was;
            continue;
        }
        let reads = environments(pr);
        let now = forge::deployed_after(&pr.merge_oid, &was, &reads, |latest| carries(pr, latest));
        pr.deployed = now;
    }
}

/// Each merged PR of the ticket's own branch whose head is past a leg's
/// tip (`forge::merged_past`, T-799): does that head carry the tip? A
/// reviewer's commit on the forge puts the head one past the worktree, and
/// a squash of it lands the tip all the same, though no fetch brought the
/// head and no patch-id matches. Asked of git first, where the head is here
/// (a fetch brought it, or it was this worktree's own commit): exact and
/// free. Else of the forge, by the PR's commits (`forge`). Each answer is
/// kept by head and tip, so a pass asks only what moved, and a tip the
/// head carries is put on the PR (`Pr::landed_tips`) for `merged_at`. A leg
/// at the commit it was cut at is not asked: every PR descends from that.
/// Returns the answers this pass stood on, for the next pass.
pub fn landed_reads(
    set: &mut [Pr],
    t: &TicketQuery,
    known: Option<&HashMap<LandedKey, bool>>,
    mut forge: impl FnMut(&Pr, &str) -> Option<bool>,
) -> HashMap<LandedKey, bool> {
    let mut answers = HashMap::new();
    if !set.iter().any(|p| p.state == PrState::Merged && p.head_ref == t.branch) {
        return answers;
    }
    for leg in &t.legs {
        let tip = crate::worktree::branch_tip(&leg.dir, &t.branch);
        if tip == leg.base_oid {
            continue;
        }
        for pr in set.iter_mut() {
            // Its PRs name the leg's repository, or its fork's upstream (T-800).
            if !leg.repo_paths.contains(&pr.repo) || !forge::merged_past(pr, &t.branch, &tip) {
                continue;
            }
            let key = (pr.repo.clone(), pr.head_oid.clone(), tip.clone());
            let answer = known
                .and_then(|k| k.get(&key).copied())
                .or_else(|| carries_here(&leg.dir, &pr.head_oid, &tip))
                .or_else(|| forge(pr, &tip));
            // A forge that did not answer is asked again on the next pass.
            let Some(carries) = answer else { continue };
            answers.insert(key, carries);
            if carries && !pr.landed_tips.contains(&tip) {
                pr.landed_tips.push(tip.clone());
            }
        }
    }
    answers
}

/// Whether `head` carries `tip`, by git in `dir` (T-799): `None` where the
/// head is not here. Reads only — `merge-base` writes nothing, and a
/// partial clone is told not to fetch a missing head (`GIT_NO_LAZY_FETCH`),
/// which would write a pack into the repository (promise 1).
pub fn carries_here(dir: &Path, head: &str, tip: &str) -> Option<bool> {
    if !forge::is_oid(head) || !forge::is_oid(tip) {
        return None;
    }
    let status = crate::git::git(dir)
        .args(["--no-optional-locks", "merge-base", "--is-ancestor", tip, head])
        .env("GIT_NO_LAZY_FETCH", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    match status.code() {
        Some(0) => Some(true),
        Some(1) => Some(false),
        // 128: a commit git does not have.
        _ => None,
    }
}

/// How many commits one page of a PR's commits holds.
const COMMITS_PAGE: usize = 100;

/// Whether a merged PR's commits hold `tip` (T-799), on the forge: GitHub's
/// `pulls/:n/commits`, GitLab's merge request `commits`, a page at a time up
/// to the 250 commits GitHub lists. `None` when a call failed or answered
/// something else.
fn pr_carries(r: &RepoQuery, pr: &Pr, tip: &str) -> Option<bool> {
    for page in 1..=3 {
        let path = match r.cli.forge {
            Forge::GitHub => format!(
                "repos/{}/pulls/{}/commits?per_page={COMMITS_PAGE}&page={page}",
                r.remote.path, pr.number
            ),
            Forge::GitLab => format!(
                "projects/{}/merge_requests/{}/commits?per_page={COMMITS_PAGE}&page={page}",
                r.project(),
                pr.number
            ),
        };
        let oids = forge::parse_pr_commits(&r.api(&path).ok()?)?;
        if oids.iter().any(|o| o == tip) {
            return Some(true);
        }
        if oids.len() < COMMITS_PAGE {
            return Some(false);
        }
    }
    Some(false)
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

/// Setup's question (T-816): which forge the repository at `dir` is on,
/// and whether its CLI is signed in there. `None` where the repository has
/// no `origin` a forge could serve. Runs git and the CLIs, so off any
/// thread that draws.
pub fn setup_probe(dir: &Path) -> Option<(Forge, bool)> {
    let out = crate::git::git(dir).args(["remote", "get-url", "origin"]).output().ok()?;
    let remote = forge::Remote::parse(&String::from_utf8_lossy(&out.stdout))?;
    let found: Vec<Cli> = remote.candidates().iter().filter_map(|f| find_cli(*f, None)).collect();
    if let Some(cli) = probe_host(&remote.host, &found).cli {
        return Some((cli.forge, true));
    }
    let guess = remote
        .known_forge()
        .or_else(|| remote.host.contains("gitlab").then_some(Forge::GitLab))
        .or_else(|| found.first().map(|c| c.forge))
        .unwrap_or(Forge::GitHub);
    Some((guess, false))
}

/// Which candidate is signed in to the host, in order; the viewer's
/// username beside it where the CLI needs one. Where none is, the first
/// one's words.
fn probe_host(host: &str, candidates: &[Cli]) -> HostProbe {
    let mut lost = None;
    for c in candidates {
        if let Err(e) = auth_status(c, host, CALL_TIMEOUT) {
            lost.get_or_insert_with(|| forge::sign_in_trouble(c.forge, host, &e));
            continue;
        }
        let viewer = match c.forge {
            Forge::GitHub => None,
            Forge::GitLab => run(&c.bin, &["api", "--hostname", host, "user"], None)
                .ok()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                .and_then(|v| v.get("username")?.as_str().map(str::to_string)),
        };
        return HostProbe { cli: Some(c.clone()), at: Instant::now(), viewer, lost: None };
    }
    HostProbe { cli: None, at: Instant::now(), viewer: None, lost }
}

/// `auth status` for the host. gh checks only the active account where it
/// can (T-798): a second account's stale token fails the plain call, and a
/// gh older than `--active` is asked without it.
fn auth_status(c: &Cli, host: &str, timeout: Duration) -> Result<String, String> {
    let status = ["auth", "status", "--hostname", host];
    if c.forge == Forge::GitHub {
        match run_for(&c.bin, &[&status[..], &["--active"]].concat(), None, timeout) {
            Err(e) if e.contains("unknown flag") => {}
            other => return other,
        }
    }
    run_for(&c.bin, &status, None, timeout)
}

/// The signed-in account's PRs in one repository, every state.
fn list(r: &RepoQuery) -> Result<Vec<Pr>, String> {
    match r.cli.forge {
        Forge::GitHub => {
            let repo = r.key();
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
                    &LIST_LIMIT.to_string(),
                    "--json",
                    GH_FIELDS,
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
                    &LIST_LIMIT.to_string(),
                    "--output",
                    "json",
                ],
                Some(&r.dir),
            )?;
            Ok(forge::parse_glab(&r.remote.path, &out))
        }
    }
}

/// One PR read by itself, any state (T-800): what a full list left out.
/// `None` when the call fails or says something else.
fn one_pr(r: &RepoQuery, number: u64) -> Option<Pr> {
    match r.cli.forge {
        Forge::GitHub => {
            let n = number.to_string();
            let repo = r.key();
            let out = run(
                &r.cli.bin,
                &["pr", "view", &n, "--repo", &repo, "--json", GH_FIELDS],
                Some(&r.dir),
            )
            .ok()?;
            forge::parse_gh(&r.remote.path, &out).into_iter().find(|p| p.number == number)
        }
        Forge::GitLab => gitlab_mr(r, number).map(|m| m.pr),
    }
}

/// The PRs a ticket held that this pass's lists did not name though their
/// repository's list came back full (T-800): the account authored
/// `LIST_LIMIT` newer ones there, and the list cut the ticket's off. Each is
/// read by itself. A list with room to spare that leaves one out has said
/// it is gone.
pub fn unlisted<'a>(held: &'a [Pr], listed: &[Pr], full: &HashSet<String>) -> Vec<&'a Pr> {
    held.iter()
        .filter(|h| full.contains(&h.repo))
        .filter(|h| !listed.iter().any(|p| p.repo == h.repo && p.number == h.number))
        .collect()
}

/// One open MR read by itself (T-784, T-787). `None` when the call fails or
/// says something else: the list's row stands, and the pipeline gate holds it.
fn gitlab_mr(r: &RepoQuery, iid: u64) -> Option<forge::MrRead> {
    let out = r.api(&format!("projects/{}/merge_requests/{iid}", r.project())).ok()?;
    forge::parse_glab_mr(&r.remote.path, &out).filter(|m| m.pr.number == iid)
}

/// A GitLab commit's newest pipeline, by sha. `Unknown` when the call
/// fails: never read as green.
fn gitlab_pipeline(r: &RepoQuery, sha: &str) -> Pipeline {
    let Ok(out) = r.api(&format!("projects/{}/pipelines?sha={sha}&per_page=1", r.project())) else {
        return Pipeline::Unknown;
    };
    match forge::parse_gitlab_pipelines(&out) {
        (pipeline, Some(id)) => named_job(r, pipeline, id),
        (pipeline, None) => pipeline,
    }
}

/// A GitLab pipeline's run, wait or failure named by one more read, of its
/// jobs (T-772, T-794); a failed read, or any other word, leaves it as it is.
fn named_job(r: &RepoQuery, pipeline: Pipeline, id: u64) -> Pipeline {
    let Some(status) = pipeline.gitlab_job_status() else { return pipeline };
    match r.api(&format!("projects/{}/pipelines/{id}/jobs?per_page=100", r.project())) {
        Ok(jobs) => pipeline.with_job(forge::parse_gitlab_job(&jobs, status)),
        Err(_) => pipeline,
    }
}

/// A merge commit's pipeline on the forge: GitHub's check runs for the
/// commit and its commit statuses, the worse of the two (T-800); GitLab's
/// newest pipeline by sha. `Unknown` when a call fails.
fn commit_pipeline(r: &RepoQuery, oid: &str) -> Pipeline {
    match r.cli.forge {
        Forge::GitHub => {
            let path = &r.remote.path;
            let runs = match r.api(&format!("repos/{path}/commits/{oid}/check-runs?per_page=100")) {
                Ok(out) => forge::parse_gh_check_runs(&out),
                Err(_) => return Pipeline::Unknown,
            };
            let statuses = match r.api(&format!("repos/{path}/commits/{oid}/status")) {
                Ok(out) => forge::parse_gh_commit_status(&out),
                Err(_) => Pipeline::Unknown,
            };
            forge::gh_commit_pipeline(runs, statuses)
        }
        Forge::GitLab => gitlab_pipeline(r, oid),
    }
}

/// How many of an environment's deployments one read lists: GitLab's
/// successes, among which the one that finished last runs there.
const GITLAB_DEPLOYS_PAGE: usize = 20;
/// How many of an environment's newest GitHub deployments are walked for
/// the first that succeeded, one status read each.
const GH_DEPLOYS_WALK: usize = 10;

/// What each wanted environment of a repository runs (T-801), one read per
/// environment for the whole pass rather than one per merge commit. An
/// environment whose read failed is left out: this pass knows nothing of
/// it, and what its PRs held stands.
fn environments(r: &RepoQuery, envs: &[String]) -> Vec<forge::EnvRead> {
    envs.iter()
        .filter_map(|env| match r.cli.forge {
            Forge::GitHub => gh_environment(r, env),
            Forge::GitLab => {
                // A name may hold a space, a `+` or a `#` (T-795).
                let path = format!(
                    "projects/{}/deployments?environment={}&status=success\
                     &order_by=created_at&sort=desc&per_page={GITLAB_DEPLOYS_PAGE}",
                    r.project(),
                    query_word(env)
                );
                forge::parse_gitlab_env(env, &r.api(&path).ok()?)
            }
        })
        .collect()
}

/// A GitHub environment's latest successful deployment: its deployments
/// newest first, and the first whose statuses hold a success. Any success
/// counts, not the newest status alone (T-800): GitHub marks a deployment
/// `inactive` once a newer one succeeds. A status read that fails leaves
/// the environment unread, never an older deployment read as the latest,
/// which would read a merge it lacks as a rollback.
fn gh_environment(r: &RepoQuery, env: &str) -> Option<forge::EnvRead> {
    let path = format!(
        "repos/{}/deployments?environment={}&per_page={GH_DEPLOYS_WALK}",
        r.remote.path,
        query_word(env)
    );
    let list = r.api(&path).ok()?;
    let mut read = forge::EnvRead { env: env.to_string(), ..forge::EnvRead::default() };
    for (id, named, sha) in forge::parse_gh_deployments(&list) {
        if named != env || sha.is_empty() {
            continue;
        }
        let status = r
            .api(&format!("repos/{}/deployments/{id}/statuses?per_page=100", r.remote.path))
            .ok()?;
        if let Some(at) = forge::parse_gh_deployment_success(&status) {
            read.latest = sha.clone();
            read.at_ms = at;
            read.seen.push(sha);
            break;
        }
    }
    Some(read)
}

/// One move of a worktree's HEAD, as its reflog records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moved {
    /// The commit HEAD moved to.
    pub oid: String,
    /// The move wrote that commit (`forge::wrote_commit`): the worktree's
    /// own work, not the commit it was cut at or one a pull brought in.
    pub wrote: bool,
}

/// Every commit a worktree's HEAD pointed at, newest first; empty where
/// there is no reflog (a reclaimed worktree, a directory that is gone).
pub fn reflog(dir: &Path) -> Vec<Moved> {
    if !dir.is_dir() {
        return Vec::new();
    }
    let out = crate::git::git(dir)
        .args([
            "--no-optional-locks",
            "reflog",
            "show",
            "--format=%H %gs",
            "-n",
            REFLOG_CAP,
            "HEAD",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter_map(|l| {
                let (oid, subject) = l.split_once(' ').unwrap_or((l, ""));
                let oid = oid.trim();
                (!oid.is_empty())
                    .then(|| Moved { oid: oid.to_string(), wrote: forge::wrote_commit(subject) })
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// The remote a repository's base branch is judged against, parsed: the
/// branch's own remote, else `origin`, else the one remote there is.
pub fn remote_of(repo: &Path, base: &str) -> Option<Remote> {
    remotes_of(repo, base).map(|r| r.remote)
}

/// A repository's remotes as the forge reads them (T-800).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remotes {
    /// The one its base branch is judged against (`remote_of`): what is
    /// listed.
    pub remote: Remote,
    /// The repository path of every remote URL, fetch and push, on that
    /// one's host, its own first: a fork's `origin` beside its `upstream`.
    /// A PR opened against the upstream names the upstream's path, and is
    /// this repository's all the same.
    pub paths: Vec<String>,
}

/// `remote_of`, with every remote's path beside it (`Remotes`). One `git
/// remote -v`, which spells each URL after git's `insteadOf`, and one
/// `ssh -G` per SSH host, which names the host an `~/.ssh/config` alias
/// reaches (T-800): `git@github-work:o/r.git` is github.com's.
pub fn remotes_of(repo: &Path, base: &str) -> Option<Remotes> {
    let out = crate::git::git(repo)
        .args(["--no-optional-locks", "remote", "-v"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let listed = String::from_utf8_lossy(&out.stdout).into_owned();
    // `<name>\t<url> (fetch|push)`.
    let urls: Vec<(&str, &str, bool)> = listed
        .lines()
        .filter_map(|l| {
            let (name, rest) = l.split_once('\t')?;
            let (url, kind) = rest.rsplit_once(' ')?;
            Some((name, url, kind == "(fetch)"))
        })
        .collect();
    let mut names: Vec<&str> = Vec::new();
    for (name, _, _) in &urls {
        if !names.contains(name) {
            names.push(name);
        }
    }
    let tracking = crate::gitstatus::remote_of(repo, base);
    let mut order: Vec<&str> = tracking.iter().map(String::as_str).collect();
    order.push("origin");
    if let [only] = names.as_slice() {
        order.push(only);
    }
    let mut hosts: HashMap<String, Option<String>> = HashMap::new();
    let mut parse = |url: &str| {
        let mut r = Remote::parse(url)?;
        if let Some(alias) = forge::ssh_alias(url) {
            let resolved = hosts.entry(alias.clone()).or_insert_with(|| ssh_hostname(&alias, None));
            if let Some(host) = resolved {
                r.host = forge::canonical_host(host);
            }
        }
        Some(r)
    };
    let remote = order.iter().find_map(|name| {
        let (_, url, _) = urls.iter().find(|(n, _, fetch)| n == name && *fetch)?;
        parse(url)
    })?;
    let mut paths = vec![remote.path.clone()];
    for (_, url, _) in &urls {
        if let Some(r) = parse(url).filter(|r| r.host == remote.host) {
            if !paths.contains(&r.path) {
                paths.push(r.path);
            }
        }
    }
    Some(Remotes { remote, paths })
}

/// The host `ssh` reaches for `alias`, by `ssh -G` (which prints the
/// resolved configuration and connects to nothing), read from `config`
/// where given, else the person's own. `None` where ssh did not answer.
pub fn ssh_hostname(alias: &str, config: Option<&Path>) -> Option<String> {
    let mut args: Vec<&str> = vec!["-G"];
    let config = config.map(|c| c.to_string_lossy().into_owned());
    if let Some(c) = &config {
        args.extend(["-F", c]);
    }
    args.extend(["--", alias]);
    let out = run_for(Path::new("ssh"), &args, None, SSH_TIMEOUT).ok()?;
    forge::ssh_config_hostname(&out)
}

/// Whether `cli` holds a signed-in account for `host` (T-800), as the
/// probe asks it: `Err` with the CLI's own words where not. For `doctor`,
/// on a shorter deadline than a sample's.
pub fn signed_in(cli: &Cli, host: &str) -> Result<(), String> {
    auth_status(cli, host, Duration::from_secs(8)).map(|_| ())
}

/// One fenced CLI call: stdout on success, the first stderr line (the
/// CLI's own words) or the timeout on failure.
fn run(bin: &Path, args: &[&str], cwd: Option<&Path>) -> Result<String, String> {
    run_for(bin, args, cwd, CALL_TIMEOUT)
}

/// `run` with its own deadline.
fn run_for(
    bin: &Path,
    args: &[&str],
    cwd: Option<&Path>,
    timeout: Duration,
) -> Result<String, String> {
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
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                // SAFETY: a signal to a process group this daemon created.
                unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
                let _ = child.wait();
                let _ = first_line.join();
                let _ = reader.join();
                return Err(format!("{name} did not finish within {}s", timeout.as_secs()));
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
    /// Per repository directory: every remote's path on its remote's host
    /// (`Remotes::paths`, T-800), what a PR's `repo` is matched against.
    pub paths: HashMap<PathBuf, Vec<String>>,
    /// Each ticket's set, newest first, and when it was read.
    pub sets: HashMap<ulid::Ulid, Vec<Pr>>,
    pub at_ms: HashMap<ulid::Ulid, u64>,
    /// Every PR, by repository path and number, a ticket's reads have held
    /// since this start or in `forge.json` (T-767): a PR read again after
    /// a read that missed it, or reopened, is no opening.
    pub held: HashMap<ulid::Ulid, Vec<(String, u64)>>,
    /// The open PRs, by repository and number, whose `conflict` edge was
    /// said and not cleared (T-769): a clear is said only where its
    /// conflict was. In `forge.json` beside the sets (T-785), so a conflict
    /// said before a restart says its clear after it.
    pub conflicted: HashMap<ulid::Ulid, Vec<(String, u64)>>,
    /// URLs the backfill found in a ticket's old transcripts (T-810), each
    /// bound as a baseline by the read that first finds its PR
    /// (`baseline`): the work is old, so its binding is no news. Memory
    /// only; a restart's backfill has nothing left to find.
    pub quiet: HashMap<ulid::Ulid, Vec<String>>,
    /// The repositories whose last call failed, with the CLI's words.
    pub errors: Vec<(String, String)>,
    /// Merge commits' pipelines as last read, by repository path and oid
    /// (T-771): settled once passed or failed.
    pub commit_pipelines: HashMap<(String, String), Pipeline>,
    /// The environments each merge commit was deployed to as last read
    /// (T-801): a rollback takes one away, so it is the last read's word,
    /// never a sum of every read's.
    pub deployed: HashMap<(String, String), Vec<String>>,
    /// Each ticket's answers to "does this merged head carry this tip"
    /// (T-799), as its last read stood on them; a carried tip is also on
    /// its PR in `forge.json`, which seeds this after a restart.
    pub landed: HashMap<ulid::Ulid, HashMap<LandedKey, bool>>,
    /// Git's answers to "does this deployed commit carry this merge commit"
    /// (T-801): the minute's pass's replace them, a burst's add to them.
    /// Memory only: a restart asks git again, which is cheap.
    pub carried: HashMap<CarriedKey, bool>,
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
    /// was. The first read after a restart is no baseline (T-785): what was
    /// said came back with the sets, so it is judged like any other.
    pub fn say_conflicts(&mut self, t: ulid::Ulid, edge: &mut Edge) {
        let said = self.conflicted.entry(t).or_default();
        edge.conflict.retain(|c| {
            let id = (c.repo.clone(), c.number);
            let was = said.contains(&id);
            if !c.on {
                said.retain(|s| *s != id);
                return was;
            }
            if was {
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

    /// Takes a pass's probe of `host`, a failed sign-in kept as it came.
    /// T-798 kept one only where the host had a CLI or held PRs, and read
    /// any other as a board of no forge; on a board that merges through
    /// pull requests, the only one that probes, every worktree's host is
    /// one the person means read (T-810), so a CLI never signed in is
    /// trouble as a lost one is.
    pub fn take_probe(&mut self, host: String, probe: HostProbe) {
        self.probes.insert(host, probe);
    }

    /// Whether a pass is due: not in flight, and the cadence passed. A
    /// repository whose call failed backs off on its own
    /// (`backing_off`, T-800): one failing repository does not slow the
    /// board's every other one.
    pub fn due(&self) -> bool {
        !self.inflight && self.last.is_none_or(|t| t.elapsed() >= every())
    }

    /// Whether the minute's pass leaves a repository out: its last call
    /// failed less than `backoff()` ago.
    pub fn backing_off(&self, key: &str) -> bool {
        self.errors.iter().any(|(k, _)| k == key)
            && self.asked.get(key).is_some_and(|t| t.elapsed() < backoff())
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
                        && matches!(p.pipeline, Pipeline::Running(_) | Pipeline::Waiting(_))
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
    /// host probed to no CLI. A host whose sign-in was lost is not one
    /// (T-798): its tickets wait for it as for a repository that failed.
    fn none(&self) -> bool {
        !self.remotes.is_empty()
            && (self.remotes.values().all(Option::is_none)
                || (!self.probes.is_empty() && self.probes.values().all(HostProbe::reads_nothing)))
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

    /// The remote a repository directory is read through, parsed once
    /// (`remotes_of`) and kept, with its remotes' paths.
    pub fn remote(&mut self, repo: &Path, base: &str) -> Option<&Remote> {
        if !self.remotes.contains_key(repo) {
            let found = remotes_of(repo, base);
            if let Some(r) = &found {
                self.paths.insert(repo.to_path_buf(), r.paths.clone());
            }
            self.remotes.insert(repo.to_path_buf(), found.map(|r| r.remote));
        }
        self.remotes.get(repo).and_then(Option::as_ref)
    }

    /// Every path a PR of the repository at `dir` may name (T-800): its
    /// remote's, and its other remotes' on that host. Empty where no forge
    /// reads it.
    pub fn repo_paths(&self, dir: &Path) -> Vec<String> {
        match (self.remotes.get(dir), self.paths.get(dir)) {
            (Some(Some(_)), Some(paths)) => paths.clone(),
            (Some(Some(r)), None) => vec![r.path.clone()],
            _ => Vec::new(),
        }
    }

    /// Forgets every ticket not in `live`, in each per-ticket map.
    pub fn keep_tickets(&mut self, live: &HashSet<ulid::Ulid>) {
        self.sets.retain(|t, _| live.contains(t));
        self.at_ms.retain(|t, _| live.contains(t));
        self.held.retain(|t, _| live.contains(t));
        self.conflicted.retain(|t, _| live.contains(t));
        self.basis.retain(|t, _| live.contains(t));
        self.local.retain(|t, _| live.contains(t));
        self.owed.retain(|t, _| live.contains(t));
        self.landed.retain(|t, _| live.contains(t));
        self.quiet.retain(|t, _| live.contains(t));
        // And the merge commits no kept set names (T-838): the two maps
        // grew by every PR ever merged, were written whole to `forge.json`
        // and copied into every pass's and burst's query.
        let named: HashSet<(String, String)> = self
            .sets
            .values()
            .flatten()
            .filter(|p| p.state == PrState::Merged && !p.merge_oid.is_empty())
            .map(|p| (p.repo.clone(), p.merge_oid.clone()))
            .collect();
        self.commit_pipelines.retain(|k, _| named.contains(k));
        self.deployed.retain(|k, _| named.contains(k));
    }

    /// The PRs of `set` the backfill's URLs name for `t` (T-810), taken as
    /// already held: the caller reads them as its last read's too, so
    /// binding one is no opening, no feed line, no edge and no arrival.
    /// Each URL is spent on the read that finds its PR.
    pub fn baseline(&mut self, t: ulid::Ulid, set: &[Pr]) -> Vec<Pr> {
        let Some(urls) = self.quiet.get_mut(&t) else { return Vec::new() };
        let found: Vec<Pr> = set
            .iter()
            .filter(|p| urls.iter().any(|u| forge::same_url(u, &p.url)))
            .cloned()
            .collect();
        urls.retain(|u| !found.iter().any(|p| forge::same_url(u, &p.url)));
        if urls.is_empty() {
            self.quiet.remove(&t);
        }
        let held = self.held.entry(t).or_default();
        for p in &found {
            let id = (p.repo.clone(), p.number);
            if !held.contains(&id) {
                held.push(id);
            }
        }
        found
    }

    /// Whether an open PR of the ticket's set has the forge's conflict.
    pub fn pr_conflict(&self, ticket: ulid::Ulid) -> bool {
        self.sets.get(&ticket).is_some_and(|s| s.iter().any(|p| p.state.is_open() && p.conflict))
    }

    /// Any PR bound to the ticket, open, merged or closed (T-778): its
    /// work goes through pull requests, and the forge merges it. A closed
    /// one stays in the set for this (T-797).
    pub fn has_pr(&self, ticket: ulid::Ulid) -> bool {
        self.sets.get(&ticket).is_some_and(|s| !s.is_empty())
    }

    /// Whether the ticket's reads ever held the PR a printed URL names
    /// (T-797): in any state, a closed one included, and one a later list
    /// no longer shows. `held` holds every PR of every set.
    pub fn held_url(&self, ticket: ulid::Ulid, url: &str) -> bool {
        self.held.get(&ticket).is_some_and(|held| {
            held.iter().any(|(repo, number)| forge::url_names(url, repo, *number))
        }) || self.sets.get(&ticket).is_some_and(|s| s.iter().any(|p| forge::same_url(&p.url, url)))
    }

    /// The ticket's merged PR for the repository at `dir` at a tip
    /// (`forge::merged_at`), under any of its remotes' paths (T-800).
    pub fn merged_at(&self, ticket: ulid::Ulid, dir: &Path, tip: &str) -> Option<&Pr> {
        let paths = self.repo_paths(dir);
        self.sets.get(&ticket).and_then(|s| forge::merged_at(s, &paths, tip))
    }

    /// The one line the advisory row says while a host is read by no CLI
    /// (T-798, T-810: its sign-in failed or no CLI is on PATH, with the fix)
    /// or a repository cannot be read; `ForgeStatus.trouble` carries it to
    /// every client.
    pub fn trouble(&self) -> Option<String> {
        let lost = self
            .probes
            .iter()
            .filter_map(|(host, p)| Some((host, p.lost.as_ref()?)))
            .min_by(|a, b| a.0.cmp(b.0));
        if let Some((_, why)) = lost {
            return Some(why.clone());
        }
        let (key, why) = self.errors.first()?;
        // A key can come from a URL a session printed (T-795).
        Some(format!("{}: {why}", mesimon_core::text::scrub_text(key)))
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
    #[serde(skip_serializing_if = "String::is_empty")]
    pub title: String,
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
    /// When it merged (T-800); zero, and absent, in an older build's file.
    #[serde(skip_serializing_if = "is_zero")]
    pub merged_ms: u64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub deployed: Vec<String>,
    /// The branch tips its merged head carries (T-799). An older file has
    /// none, and the first pass asks again.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub landed_tips: Vec<String>,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

impl PrRecord {
    fn of(p: &Pr) -> Self {
        PrRecord {
            forge: p.forge.word().into(),
            repo: p.repo.clone(),
            number: p.number,
            title: p.title.clone(),
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
            merged_ms: p.merged_ms,
            deployed: p.deployed.clone(),
            landed_tips: p.landed_tips.clone(),
        }
    }

    fn pr(self) -> Option<Pr> {
        Some(Pr {
            forge: Forge::from_word(&self.forge)?,
            state: PrState::from_word(&self.state)?,
            pipeline: Pipeline::from_words(&self.pipeline, &self.job),
            review: Review::from_word(&self.review),
            repo: self.repo,
            number: self.number,
            title: self.title,
            url: self.url,
            head_oid: self.head_oid,
            head_ref: self.head_ref,
            merge_oid: self.merge_oid,
            conflict: self.conflict,
            updated_ms: self.updated_ms,
            merged_ms: self.merged_ms,
            deployed: self.deployed,
            landed_tips: self.landed_tips,
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
    /// Each ticket's PRs whose conflict was said and not cleared (T-785):
    /// `State::conflicted`, PRs its set still holds. An older file has
    /// none, and a clear of a conflict it said is then silent.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub conflicts: BTreeMap<ulid::Ulid, Vec<(String, u64)>>,
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
        let conflicts = state
            .conflicted
            .iter()
            .filter_map(|(t, said)| {
                let set = state.sets.get(t)?;
                let kept: Vec<(String, u64)> = said
                    .iter()
                    .filter(|(r, n)| set.iter().any(|p| p.repo == *r && p.number == *n))
                    .cloned()
                    .collect();
                (!kept.is_empty()).then_some((*t, kept))
            })
            .collect();
        ForgeFile { schema_version: FORGE_SCHEMA, sets, commits, conflicts }
    }

    /// No sets, at this build's schema: what an absent file reads as.
    pub fn default_of_schema() -> Self {
        ForgeFile { schema_version: FORGE_SCHEMA, ..ForgeFile::default() }
    }
}

impl State {
    /// Take a loaded `forge.json` back (T-752): the sets of the tickets the
    /// board still has, the conflicts said on them (T-785), and the settled
    /// commits. `at_ms` stays empty — it
    /// is "read live since this start", what the train's freshness and the
    /// pipeline gate wait for.
    pub fn restore(&mut self, file: ForgeFile, live: impl Fn(ulid::Ulid) -> bool) {
        for (t, records) in file.sets {
            if !live(t) {
                continue;
            }
            let set = forge::ticket_set(records.into_iter().filter_map(PrRecord::pr).collect());
            self.opens(t, &set);
            // What the file says a head carries is not asked again (T-799).
            let landed: HashMap<LandedKey, bool> = set
                .iter()
                .flat_map(|p| {
                    p.landed_tips
                        .iter()
                        .map(|tip| ((p.repo.clone(), p.head_oid.clone(), tip.clone()), true))
                })
                .collect();
            if !landed.is_empty() {
                self.landed.insert(t, landed);
            }
            self.sets.insert(t, set);
        }
        for (t, said) in file.conflicts {
            if self.sets.contains_key(&t) {
                self.conflicted.insert(t, said);
            }
        }
        for c in file.commits {
            let key = (c.repo, c.oid);
            match Pipeline::from_words(&c.pipeline, &c.job) {
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
/// repository that did not answer on this pass carried over (T-752,
/// T-795): one whose call failed, one on a host no CLI was found for, one
/// the pass did not list. A forge that did not answer said nothing, and a
/// card does not lose its PRs to a timeout, nor a loaded set to the first
/// pass after a restart. An open PR whose pipeline read failed keeps the
/// pipeline last read at the same head: unknown is no news.
pub fn carry(old: Option<&[Pr]>, new: Vec<Pr>, answered: &HashSet<&str>) -> Vec<Pr> {
    let Some(old) = old else { return new };
    let kept = old.iter().filter(|p| !answered.contains(p.repo.as_str())).cloned();
    let mut set: Vec<Pr> = new.into_iter().filter(|p| answered.contains(p.repo.as_str())).collect();
    for p in set.iter_mut().filter(|p| p.state.is_open() && p.pipeline == Pipeline::Unknown) {
        let last = old.iter().find(|o| {
            o.repo == p.repo
                && o.number == p.number
                && o.head_oid == p.head_oid
                && o.pipeline != Pipeline::Unknown
        });
        if let Some(o) = last {
            p.pipeline = o.pipeline.clone();
        }
    }
    set.extend(kept);
    forge::ticket_set(set)
}

/// Whether a pass read all of a ticket's set (T-795): every repository it
/// comes from answered, or sits on a host whose probe found no CLI
/// (`no_cli`), which nothing reads. One that failed, or that the pass did
/// not list (a burst skips a host not yet probed), leaves the ticket
/// unread: not current for the train, its owed read still owed.
pub fn read_whole(
    repos: Option<&Vec<Remote>>,
    reports: &[RepoReport],
    no_cli: impl Fn(&str) -> bool,
) -> bool {
    repos.into_iter().flatten().all(|r| {
        let key = r.key();
        reports.iter().any(|x| x.key == key && x.error.is_none()) || no_cli(&r.host)
    })
}

/// `s` as one segment or value of an API path: every byte but the
/// unreserved ones percent-encoded, `/` included.
fn query_word(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `pipeline failed !1: test-linux`, bare where no job is named, and with
/// no label where none is.
fn failed_words(label: &str, job: &str) -> String {
    let at = if label.is_empty() { String::new() } else { format!(" {label}") };
    match forge::job_word(job) {
        j if j.is_empty() => format!("pipeline failed{at}"),
        j => format!("pipeline failed{at}: {j}"),
    }
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
                        Pipeline::Failed(job) => out.push(failed_words(&label, job)),
                        Pipeline::Passed if was.pipeline != Pipeline::None => {
                            out.push(format!("pipeline passed {label}"))
                        }
                        Pipeline::Cancelled(_) => out.push(format!("pipeline cancelled {label}")),
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
            title: String::new(),
            url: format!("https://github.com/o/r/pull/{number}"),
            state,
            head_oid: "h".into(),
            head_ref: "b".into(),
            merge_oid: String::new(),
            conflict: false,
            review: Default::default(),
            pipeline,
            updated_ms: number,
            merged_ms: 0,
            deployed: Vec::new(),
            landed_tips: Vec::new(),
        }
    }

    #[test]
    fn the_delta_says_what_moved_and_nothing_on_the_first_read() {
        let before = [
            pr(1, PrState::Open, Pipeline::Running(String::new())),
            pr(2, PrState::Open, Pipeline::None),
        ];
        let after = [
            pr(1, PrState::Merged, Pipeline::Passed),
            pr(2, PrState::Open, Pipeline::Failed("test-linux".into())),
            pr(3, PrState::Open, Pipeline::Running(String::new())),
        ];
        assert!(delta(None, &after).is_empty());
        assert_eq!(
            delta(Some(&before), &after),
            ["merged #1", "pipeline passed #1", "pipeline failed #2: test-linux", "open #3 bound"]
        );
        let bare = [pr(2, PrState::Open, Pipeline::Failed(String::new()))];
        assert_eq!(delta(Some(&before), &bare), ["pipeline failed #2"]);
    }

    /// T-797: a closed PR stays in the set. The feed says the closing, the
    /// other PR merging beside it is the ticket's arrival, a closed PR
    /// alone arrives nowhere, and a URL its ticket's reads ever held is
    /// read, by number and path, whatever the case of the remote's path.
    #[test]
    fn a_closed_pr_stays_and_waits_for_nothing() {
        let open = [
            pr(1, PrState::Open, Pipeline::Running(String::new())),
            pr(2, PrState::Open, Pipeline::None),
        ];
        let closed = forge::ticket_set(vec![pr(2, PrState::Closed, Pipeline::Failed("t".into()))]);
        let mut merged = pr(1, PrState::Merged, Pipeline::Passed);
        merged.merge_oid = "m1".into();
        let now = forge::ticket_set(vec![merged, closed[0].clone()]);
        // Newest first: #2 closed after #1 merged.
        assert_eq!(delta(Some(&open), &now), ["closed #2", "merged #1", "pipeline passed #1"]);
        let e = edge(&open, &now);
        assert!(e.merged, "the open one merged; the closed one waits for nothing");
        assert!(e.failed.is_none(), "a closed PR's pipeline is none");
        assert!(!edge(&open[1..], &closed).merged, "closing alone merges nothing");
        assert!(!edge(&now, &now).merged);

        let mut s = State::default();
        let t = ulid::Ulid::new();
        s.opens(t, &closed);
        s.sets.insert(t, closed);
        assert!(s.has_pr(t));
        assert!(s.held_url(t, "https://github.com/o/r/pull/2"));
        assert!(s.held_url(t, "https://github.com/O/R/pull/2"));
        assert!(!s.held_url(t, "https://github.com/o/r/pull/3"));
        assert!(!s.held_url(ulid::Ulid::new(), "https://github.com/o/r/pull/2"));
    }

    #[test]
    fn the_edge_between_two_reads_names_what_a_column_waits_for() {
        let mut m = pr(1, PrState::Merged, Pipeline::Passed);
        m.merge_oid = "m1".into();
        let old = [pr(1, PrState::Open, Pipeline::Running(String::new()))];
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

    /// An environment read: `env` runs `latest`, seen deployed alone.
    fn runs(env: &str, latest: &str) -> forge::EnvRead {
        forge::EnvRead {
            env: env.into(),
            latest: latest.into(),
            at_ms: 0,
            seen: vec![latest.to_string()],
        }
    }

    /// T-771: two merged PRs on one ticket keep each its own merge commit's
    /// pipeline and deployments; the newer one's reads do not clear the
    /// older one's, and a settled PR's pipeline is never read again. Every
    /// merged PR's deployments are judged on every pass (T-801).
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
                Pipeline::Running(String::new())
            },
            |p| {
                deploys.push(p.merge_oid.clone());
                // Staging runs m2; production's read failed.
                vec![runs("staging", "m2")]
            },
            // Neither commit is here: git cannot say.
            |_, _| None,
        );
        assert_eq!(asked, ["m2"], "a passed pipeline is settled");
        assert_eq!(deploys, ["m2", "m1"], "every merged PR's deployments are judged");
        assert_eq!(
            (&set[0].pipeline, &set[0].deployed[..]),
            (&Pipeline::Running(String::new()), &["staging".to_string()][..])
        );
        assert_eq!(set[1].pipeline, Pipeline::Passed, "the older PR keeps its own");
        assert_eq!(set[1].deployed, ["staging", "production"], "what git cannot undo stands");

        // An unsettled older PR is read while its merge is recent, and keeps
        // what was read once the day is past; a failed read is no reading.
        q.commit_pipelines.insert(key("m1"), Pipeline::Running(String::new()));
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
            |_, _| None,
        );
        assert_eq!(asked, 2);
        assert_eq!(
            set[1].pipeline,
            Pipeline::Running(String::new()),
            "an unanswered call keeps the last read"
        );
        assert_eq!(set[1].deployed, ["staging"], "no environment read: what was stands");
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
            |_| Vec::new(),
            |_, _| None,
        );
        assert_eq!(*asked.borrow(), ["m2"], "a superseded day-old merge's pipeline is not read");
        assert_eq!(set[1].pipeline, Pipeline::Running(String::new()));
        assert_eq!(set[1].deployed, ["staging"]);
        // No column waits for a deployment: nothing is read, nothing drops.
        let quiet = Query { envs: Vec::new(), ..q.clone() };
        let mut set = [merged(1, "m1", now - 2000)];
        merged_reads(&mut set, &quiet, now, |_| Pipeline::Passed, |_| panic!("read"), |_, _| None);
        assert_eq!(set[0].deployed, ["staging"]);

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

    /// T-801, forge lab run 4: !13 and !15 merged and were never deployed
    /// by their own pipelines; production then ran !14's merge commit, which
    /// carries both. Each PR reads deployed, the crown hears each, and the
    /// ticket arrives. Where the history is not here only the exact commit
    /// counts; a rollback takes production back off both, and nothing moves
    /// the ticket out.
    #[test]
    fn a_batched_deploy_brings_every_merged_pr_it_carries() {
        let merged = |number: u64, oid: &str| {
            let mut p = pr(number, PrState::Merged, Pipeline::Waiting("deploy:production".into()));
            p.forge = Forge::GitLab;
            p.merge_oid = oid.into();
            p.merged_ms = number;
            p.deployed = vec!["staging".into()];
            p
        };
        let key = |oid: &str| ("o/r".to_string(), oid.to_string());
        let mut q =
            Query { envs: vec!["staging".into(), "production".into()], ..Default::default() };
        q.deployed.insert(key("e1b"), vec!["staging".into()]);
        q.deployed.insert(key("4e1"), vec!["staging".into()]);
        let old = [merged(15, "4e1"), merged(13, "e1b")];
        // ddc carries both merges; staging still runs !15's.
        let history = |merge: &str, latest: &str| match (merge, latest) {
            ("e1b" | "4e1", "ddc") | ("e1b", "4e1") => Some(true),
            (_, "old") => Some(false),
            _ => None,
        };
        let read = |q: &Query, prod: &str, git: &dyn Fn(&str, &str) -> Option<bool>| {
            let mut set = old.clone();
            merged_reads(
                &mut set,
                q,
                0,
                |p| p.pipeline.clone(),
                |_| vec![runs("staging", "4e1"), runs("production", prod)],
                |p, latest| git(&p.merge_oid, latest),
            );
            set
        };
        let new = read(&q, "ddc", &history);
        assert_eq!(new[0].deployed, ["staging", "production"]);
        assert_eq!(new[1].deployed, ["staging", "production"], "!13 by ancestry");
        assert_eq!(new[1].pipeline, old[1].pipeline, "a waiting job is never read as passed");
        let e = edge(&old, &new);
        assert_eq!(e.deployed, ["production"], "every merged PR is there");
        assert_eq!(e.news(&new), ["deployed to production !15", "deployed to production !13"]);

        // The history not fetched: only the exact commit counts.
        let none = |_: &str, _: &str| -> Option<bool> { None };
        let unfetched = read(&q, "ddc", &none);
        assert!(unfetched.iter().all(|p| p.deployed == ["staging"]), "{unfetched:?}");
        assert!(edge(&old, &unfetched).deployed.is_empty());
        // One PR there and one not: no arrival, the one is heard.
        let only = |m: &str, l: &str| (m == "e1b" && l == "ddc").then_some(true);
        let half = read(&q, "ddc", &only);
        let e = edge(&old, &half);
        assert!(e.deployed.is_empty(), "!15 is not on production");
        assert_eq!(e.news(&half), ["deployed to production !13"]);

        // A rollback: production runs a commit carrying neither.
        for p in &new {
            q.deployed.insert(key(&p.merge_oid), p.deployed.clone());
        }
        let back = read(&q, "old", &history);
        assert!(back.iter().all(|p| p.deployed == ["staging"]), "{back:?}");
        let e = edge(&new, &back);
        assert!(e.deployed.is_empty() && e.reached.is_empty(), "a lost deploy is no edge: {e:?}");
        assert!(!e.any(), "nothing moves the card back");
        // Production deploys it again: heard again, the ticket arrives again.
        let again = edge(&back, &new);
        assert_eq!(again.deployed, ["production"]);
    }

    /// T-800: a merge commit's failure or cancel is read again while its
    /// merge is young, so a retried run that passed reads passed, and
    /// settles once the day is past; the newest merged PR is the one that
    /// merged last, whatever a comment did to the older one's time.
    #[test]
    fn a_failed_merge_pipeline_is_read_again_while_young() {
        let now = 10 * SUPERSEDED_READ_MS;
        let merged = |number: u64, oid: &str, merged_ms: u64, updated_ms: u64| {
            let mut p = pr(number, PrState::Merged, Pipeline::Unknown);
            p.merge_oid = oid.into();
            p.merged_ms = merged_ms;
            p.updated_ms = updated_ms;
            p
        };
        let key = |oid: &str| ("o/r".to_string(), oid.to_string());
        let mut q = Query::default();
        for last in [Pipeline::Failed("deploy".into()), Pipeline::Cancelled(String::new())] {
            q.commit_pipelines.insert(key("m1"), last.clone());
            let mut set = [merged(1, "m1", now - 60_000, now - 60_000)];
            merged_reads(&mut set, &q, now, |_| Pipeline::Passed, |_| Vec::new(), |_, _| None);
            assert_eq!(set[0].pipeline, Pipeline::Passed, "{last:?} retried green reads green");
            let mut set = [merged(1, "m1", now - 60_000, now - 60_000)];
            merged_reads(&mut set, &q, now, |_| Pipeline::Unknown, |_| Vec::new(), |_, _| None);
            assert_eq!(set[0].pipeline, last, "an unanswered read keeps the last");
            let mut set = [merged(1, "m1", now - SUPERSEDED_READ_MS, now)];
            let mut asked = 0;
            merged_reads(
                &mut set,
                &q,
                now,
                |_| {
                    asked += 1;
                    Pipeline::Passed
                },
                |_| Vec::new(),
                |_, _| None,
            );
            assert_eq!((asked, &set[0].pipeline), (0, &last), "settled a day after its merge");
        }
        q.commit_pipelines.insert(key("m1"), Pipeline::Passed);
        let mut set = [merged(1, "m1", now - 60_000, now - 60_000)];
        merged_reads(
            &mut set,
            &q,
            now,
            |_| Pipeline::Failed(String::new()),
            |_| Vec::new(),
            |_, _| None,
        );
        assert_eq!(set[0].pipeline, Pipeline::Passed, "a pass is settled at once");

        // #1 merged two days ago and was commented on just now: #2, merged
        // yesterday, is the newest, and #1 is superseded and old.
        q.commit_pipelines.clear();
        let mut set = [
            merged(1, "m1", now - 2 * SUPERSEDED_READ_MS, now - 1000),
            merged(2, "m2", now - SUPERSEDED_READ_MS - 1000, now - SUPERSEDED_READ_MS - 1000),
        ];
        let mut asked: Vec<String> = Vec::new();
        merged_reads(
            &mut set,
            &q,
            now,
            |p| {
                asked.push(p.merge_oid.clone());
                Pipeline::Passed
            },
            |_| Vec::new(),
            |_, _| None,
        );
        assert_eq!(asked, ["m2"], "the PR that merged last is read, the commented one is not");
        let mut one = set[0].clone();
        one.deployed = vec!["staging".into()];
        let mut two = set[1].clone();
        two.deployed = vec!["production".into()];
        assert!(
            arrived_envs(&[one.clone(), two.clone()]).is_empty(),
            "every merged PR stands somewhere else (T-801)"
        );
        one.deployed.push("production".into());
        assert_eq!(arrived_envs(&[one, two]), ["production"]);
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
        let running = [merged(Pipeline::Running(String::new()), &["staging"])];
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
        let before =
            [open(Pipeline::Running(String::new())), merged(Pipeline::Running(String::new()), &[])];
        let now = [open(Pipeline::Passed), merged(Pipeline::Running(String::new()), &[])];
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
        let one = pr(1, PrState::Open, Pipeline::Running(String::new()));
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
        // An older merged PR on a repository must be there too (T-801): the
        // newest one's deploy stands for it only where git said it carries
        // it, which put the environment on the older PR's own row.
        let older = [all[0].clone(), all[1].clone(), gl("g/api", 2, PrState::Merged, &[])];
        assert!(edge(&both, &older).deployed.is_empty());
        let carried = [
            all[0].clone(),
            all[1].clone(),
            gl("g/api", 2, PrState::Merged, &["staging", "production"]),
        ];
        assert_eq!(edge(&both, &carried).deployed, ["staging", "production"]);
        // A pass is every open PR's.
        let running = [gl("g/web", 1, PrState::Open, &[]), {
            let mut p = gl("g/api", 3, PrState::Open, &[]);
            p.pipeline = Pipeline::Running(String::new());
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
        pushed[0].pipeline = Pipeline::Running(String::new());
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
        let running = gl(1, Pipeline::Running(String::new()));
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
        let mut conflicted = gl(1, Pipeline::Running(String::new()));
        conflicted.conflict = true;
        assert_eq!(
            words(std::slice::from_ref(&running), std::slice::from_ref(&conflicted)),
            ["conflict !1"]
        );
        assert_eq!(
            words(std::slice::from_ref(&conflicted), std::slice::from_ref(&running)),
            ["conflict cleared !1"]
        );
        let mut approved = gl(1, Pipeline::Running(String::new()));
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
        let mut fresh = gl(2, Pipeline::Running(String::new()));
        fresh.conflict = true;
        fresh.review = Review::Approved;
        let e = edge(std::slice::from_ref(&running), &[fresh.clone(), running.clone()]);
        assert!(!e.opened.is_empty());
        assert_eq!(e.news(&[fresh, running.clone()]), ["conflict !2", "review approved !2"]);
        let quiet = gl(2, Pipeline::Running(String::new()));
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
        let mut three = gl(3, PrState::Open, Pipeline::Running(String::new()));
        three.conflict = true;
        three.review = Review::ChangesRequested;
        let now = [three.clone()];
        let e = edge(&[], &now);
        assert_eq!(e.news(&now), ["conflict !3", "changes requested !3"]);
        assert!(e.conflict[0].on);
        // Held before (a read missed it): no opening, nothing said.
        let held = [("o/r".to_string(), 3)];
        let e = super::edge(&[], &now, &held);
        assert!(!e.any(), "{e:?}");
    }

    /// T-769, finding 6: a conflict is said once, and its clear only after
    /// it; a newly bound PR's conflict is said. And T-785: what was said
    /// rides `forge.json`, so a conflict said before a restart says its
    /// clear after it, and one never said stays silent across it.
    #[test]
    fn a_conflict_cleared_follows_a_conflict_said() {
        let t = ulid::Ulid::new();
        let mut s = State::default();
        let clean = pr(4, PrState::Open, Pipeline::Running(String::new()));
        let mut stuck = clean.clone();
        stuck.conflict = true;
        let said = |s: &mut State, old: &Pr, new: &Pr| {
            let mut e = edge(std::slice::from_ref(old), std::slice::from_ref(new));
            s.say_conflicts(t, &mut e);
            s.sets.insert(t, vec![new.clone()]);
            e.news(std::slice::from_ref(new))
        };
        // A restart: the state as `forge.json` brings it back.
        let restart = |s: &State| {
            let text = serde_json::to_string(&ForgeFile::of(s)).unwrap();
            let mut back = State::default();
            back.restore(crate::store::parse_versioned(&text, FORGE_SCHEMA).unwrap(), |_| true);
            back
        };
        // D1: the set held !4 in conflict from a read whose edge said
        // nothing; its clear is no news.
        assert!(said(&mut s, &stuck, &clean).is_empty(), "a phantom clear");
        // Said, then cleared: both.
        assert_eq!(said(&mut s, &clean, &stuck), ["conflict #4"]);
        assert_eq!(said(&mut s, &stuck, &clean), ["conflict cleared #4"]);
        assert!(said(&mut s, &stuck, &clean).is_empty(), "cleared once");
        // Lab run 3's D1: said, a restart, still in conflict on the first
        // read, then clear: the clear is said, once.
        assert_eq!(said(&mut s, &clean, &stuck), ["conflict #4"]);
        let mut s = restart(&s);
        assert!(said(&mut s, &stuck, &stuck).is_empty(), "the first read after it");
        assert_eq!(said(&mut s, &stuck, &clean), ["conflict cleared #4"]);
        let mut s = restart(&s);
        assert!(said(&mut s, &stuck, &clean).is_empty(), "cleared once, across a restart");
        // Said, and cleared while the daemon was down: the first read says it.
        assert_eq!(said(&mut s, &clean, &stuck), ["conflict #4"]);
        let mut s = restart(&s);
        assert_eq!(said(&mut s, &stuck, &clean), ["conflict cleared #4"]);
        // Never said: a restart does not make its clear news.
        s.sets.insert(t, vec![stuck.clone()]);
        let mut s = restart(&s);
        assert!(said(&mut s, &stuck, &clean).is_empty(), "never said, never cleared");
        // A conflict that came while the daemon was down is news.
        let mut s = restart(&s);
        assert_eq!(said(&mut s, &clean, &stuck), ["conflict #4"]);
        // A PR bound since: its conflict is said, and said once.
        let mut s = State::default();
        let mut e = edge(&[], std::slice::from_ref(&stuck));
        s.say_conflicts(t, &mut e);
        assert_eq!(e.news(std::slice::from_ref(&stuck)), ["conflict #4"]);
        let mut again = edge(&[], std::slice::from_ref(&stuck));
        s.say_conflicts(t, &mut again);
        assert!(again.conflict.is_empty(), "said already");
        assert_eq!(said(&mut s, &stuck, &clean), ["conflict cleared #4"]);
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
            gl("g/api", 8, PrState::Open, Pipeline::Running(String::new())),
        ];
        let merged = [
            gl("g/web", 2, PrState::Merged, Pipeline::Running(String::new())),
            gl("g/api", 8, PrState::Open, Pipeline::Running(String::new())),
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
            HostProbe { cli: Some(cli), at: Instant::now(), viewer: None, lost: None },
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

    /// T-810: a PR a backfilled URL names is taken as held the read it
    /// first binds, so the caller reads it as no news; the URL is spent,
    /// and a PR no URL names is left to the read's own edges.
    #[test]
    fn a_backfilled_url_binds_its_pr_as_a_baseline_once() {
        let t = ulid::Ulid::new();
        let mut s = State::default();
        let old = pr(7, PrState::Open, Pipeline::Passed);
        let new = pr(8, PrState::Merged, Pipeline::Passed);
        s.quiet.insert(t, vec![old.url.clone(), "https://github.com/o/r/pull/500".into()]);
        let set = vec![old.clone(), new];
        let quiet = s.baseline(t, &set);
        assert_eq!(quiet.iter().map(|p| p.number).collect::<Vec<_>>(), [7]);
        assert!(s.held[&t].contains(&(old.repo.clone(), 7)), "held, so no opening");
        assert!(!s.held[&t].iter().any(|(_, n)| *n == 8), "the read's own PR is its own");
        assert_eq!(s.quiet[&t], ["https://github.com/o/r/pull/500"], "spent on its read");
        assert!(s.baseline(t, &set).is_empty(), "once");
        assert!(s.baseline(ulid::Ulid::new(), &set).is_empty());
    }

    /// A host nothing reads is trouble, never a board of no forge (T-798,
    /// T-810): a CLI that did not confirm its sign-in, held PRs or not, and
    /// no CLI on PATH at all. Each is asked again on the backoff, named on
    /// the advisory row with its fix, and the trouble clears once a CLI
    /// reads the host.
    #[test]
    fn a_host_nothing_reads_is_trouble_never_no_forge() {
        let t = ulid::Ulid::new();
        let cli = Cli { forge: Forge::GitHub, bin: PathBuf::from("/bin/gh") };
        let lost = |why: String| HostProbe {
            cli: None,
            at: Instant::now(),
            viewer: None,
            lost: Some(why),
        };
        let failed = || lost(forge::sign_in_trouble(Forge::GitHub, "github.com", "not logged in"));
        let board = || {
            let mut s = State::default();
            let remote = Remote { host: "github.com".into(), path: "o/r".into() };
            s.remotes.insert(PathBuf::from("/r"), Some(remote));
            s
        };
        // Never read, no PR: a CLI never signed in is trouble all the same.
        let mut s = board();
        s.take_probe("github.com".into(), failed());
        assert!(!s.probes["github.com"].reads_nothing());
        assert!(!s.fresh(t, 5_000, &[]), "a forge that did not answer is no board of no forge");
        assert_eq!(
            s.trouble().as_deref(),
            Some("github.com sign-in: not logged in (gh auth login --hostname github.com)")
        );
        // No CLI on PATH: the same, with its fix.
        let mut s = board();
        let missing = forge::missing_trouble("gitlab.com", &[Forge::GitLab, Forge::GitHub]);
        s.take_probe("gitlab.com".into(), lost(missing));
        assert_eq!(
            s.trouble().as_deref(),
            Some(
                "gitlab.com: no glab or gh on PATH (install glab, then glab auth login --hostname \
                 gitlab.com)"
            )
        );
        // Asked again on the backoff.
        let mut old = failed();
        old.at = Instant::now() - BACKOFF;
        assert!(old.stale());
        // Signed in again: the trouble clears.
        let mut s = board();
        s.sets.insert(t, vec![pr(7, PrState::Open, Pipeline::Passed)]);
        s.take_probe("github.com".into(), failed());
        s.take_probe(
            "github.com".into(),
            HostProbe { cli: Some(cli), at: Instant::now(), viewer: None, lost: None },
        );
        assert_eq!(s.trouble(), None);
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
        s.sets.insert(open, vec![pr(1, PrState::Open, Pipeline::Running(String::new()))]);
        // Deployed to staging, waiting on the manual production job.
        s.sets.insert(
            merged,
            vec![pr(2, PrState::Merged, Pipeline::Waiting("deploy:production".into()))],
        );
        s.sets.insert(settled, vec![pr(3, PrState::Merged, Pipeline::Passed)]);
        s.sets.insert(closed, vec![pr(4, PrState::Closed, Pipeline::Running(String::new()))]);
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
        let legs = vec![LegQuery {
            repo_path: "o/r".into(),
            repo_paths: vec!["o/r".into()],
            dir: dir.clone(),
            base_oid: String::new(),
        }];
        let now = |legs: &[LegQuery]| {
            let logs: Vec<Vec<Moved>> = legs.iter().map(|l| reflog(&l.dir)).collect();
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

    /// T-800: a full list (the account authored a hundred newer PRs in the
    /// repository) that leaves a ticket's PR out has not said it is gone:
    /// that one is read by itself. A list with room to spare has.
    #[test]
    fn a_pr_a_full_list_cut_off_is_read_by_itself() {
        let held = [pr(3, PrState::Open, Pipeline::Passed), pr(4, PrState::Merged, Pipeline::None)];
        let listed =
            [pr(4, PrState::Merged, Pipeline::None), pr(900, PrState::Open, Pipeline::None)];
        let full = HashSet::from(["o/r".to_string()]);
        let cut: Vec<u64> = unlisted(&held, &listed, &full).iter().map(|p| p.number).collect();
        assert_eq!(cut, [3]);
        assert!(unlisted(&held, &listed, &HashSet::new()).is_empty(), "room to spare: gone");
        let mut elsewhere = held.clone();
        elsewhere[0].repo = "o/s".into();
        assert!(unlisted(&elsewhere[..1], &listed, &full).is_empty(), "another repository's list");
    }

    /// T-800: a fork's checkout reads its `origin` and names its upstream
    /// beside it, so a PR opened against the upstream is the leg's; a
    /// remote on another host is not; `insteadOf` is git's own spelling.
    #[test]
    fn a_forks_remotes_name_the_upstream_beside_origin() {
        let dir = std::env::temp_dir().join(format!("msmn-forge-fork-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| {
            let st = Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(args)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap();
            assert!(st.success(), "git {args:?}");
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["remote", "add", "origin", "https://github.com/me/r.git"]);
        git(&["remote", "add", "upstream", "gh:o/r.git"]);
        git(&["config", "url.https://github.com/.insteadOf", "gh:"]);
        git(&["remote", "add", "mirror", "https://gitlab.com/o/r.git"]);
        let r = remotes_of(&dir, "main").unwrap();
        assert_eq!(r.remote.key(), "github.com/me/r", "origin, the base branch tracking none");
        assert_eq!(r.paths, ["me/r", "o/r"], "the upstream beside it, the other host's not");
        assert_eq!(remote_of(&dir, "main"), Some(r.remote.clone()));
        let mut s = State::default();
        assert_eq!(s.remote(&dir, "main").map(Remote::key).as_deref(), Some("github.com/me/r"));
        assert_eq!(s.repo_paths(&dir), ["me/r", "o/r"]);
        assert!(s.repo_paths(&dir.join("nope")).is_empty());
        let t = ulid::Ulid::new();
        let mut upstream = pr(7, PrState::Merged, Pipeline::Passed);
        upstream.repo = "o/r".into();
        upstream.head_oid = "tip".into();
        s.sets.insert(t, vec![upstream]);
        assert!(s.merged_at(t, &dir, "tip").is_some(), "the upstream's PR lands the fork's leg");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T-800: an `~/.ssh/config` alias names the host it reaches.
    #[test]
    fn an_ssh_alias_resolves_through_ssh_config() {
        if Command::new("ssh").arg("-V").output().is_err() {
            eprintln!("skipped: no ssh");
            return;
        }
        let dir = std::env::temp_dir().join(format!("msmn-forge-ssh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let config = dir.join("config");
        std::fs::write(&config, "Host github-work\n  HostName ssh.github.com\n  Port 443\n")
            .unwrap();
        let host = ssh_hostname("github-work", Some(&config));
        assert_eq!(host.as_deref(), Some("ssh.github.com"));
        assert_eq!(forge::canonical_host(&host.unwrap()), "github.com");
        assert_eq!(ssh_hostname("plain.example", Some(&config)).as_deref(), Some("plain.example"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T-799: #42 merged with its head one past the worktree's tip (a
    /// reviewer's commit). Where the head is here, git says it carries the
    /// tip and the forge is not asked; where it is not, the forge's commits
    /// say, once — the next pass stands on the answer. A head the tip has
    /// moved past, a failed call, a leg still at its cut and a side branch
    /// land nothing.
    #[test]
    fn a_merged_head_past_the_tip_is_asked_of_git_then_once_of_the_forge() {
        let dir = std::env::temp_dir().join(format!("msmn-forge-landed-{}", std::process::id()));
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
        let branch = "msmn/T-9-x";
        git(&["init", "-q", "-b", "main"]);
        git(&["commit", "-q", "--allow-empty", "-m", "base"]);
        let base = git(&["rev-parse", "HEAD"]);
        git(&["checkout", "-qb", branch]);
        git(&["commit", "-q", "--allow-empty", "-m", "work"]);
        let tip = git(&["rev-parse", "HEAD"]);
        // The reviewer's commit, fetched: on the remote-tracking ref only.
        let fetched =
            git(&["commit-tree", "-p", &tip, "-m", "suggestion", &format!("{tip}^{{tree}}")]);
        git(&["update-ref", &format!("refs/remotes/origin/{branch}"), &fetched]);
        let remote_only = "e".repeat(40);
        let t = TicketQuery {
            ticket: ulid::Ulid(9),
            branch: branch.into(),
            legs: vec![LegQuery {
                repo_path: "o/r".into(),
                repo_paths: vec!["o/r".into()],
                dir: dir.clone(),
                base_oid: base.clone(),
            }],
            opened: Vec::new(),
            repos: Vec::new(),
            held: Vec::new(),
        };
        let merged = |head: &str| {
            let mut p = pr(42, PrState::Merged, Pipeline::Passed);
            p.head_oid = head.into();
            p.head_ref = branch.into();
            p.merge_oid = "f".repeat(40);
            p
        };
        let here = ["o/r".to_string()];
        let mut asked = 0;

        // Fetched: git answers, the forge is never asked.
        let mut set = [merged(&fetched)];
        let known = landed_reads(&mut set, &t, None, |_, _| {
            asked += 1;
            None
        });
        assert_eq!(set[0].landed_tips, std::slice::from_ref(&tip));
        assert_eq!(known.get(&("o/r".into(), fetched.clone(), tip.clone())), Some(&true));
        assert_eq!(forge::merged_at(&set, &here, &tip).map(|p| p.number), Some(42));
        assert_eq!(asked, 0, "a head here is git's to answer");

        // Never fetched: the forge's commits answer, and the next pass
        // stands on that answer without a call.
        let mut set = [merged(&remote_only)];
        let known = landed_reads(&mut set, &t, None, |p, at| {
            asked += 1;
            Some(p.number == 42 && at == tip)
        });
        assert_eq!((asked, set[0].landed_tips.clone()), (1, vec![tip.clone()]));
        let mut again = [merged(&remote_only)];
        let kept = landed_reads(&mut again, &t, Some(&known), |_, _| {
            asked += 1;
            Some(false)
        });
        assert_eq!((asked, again[0].landed_tips.clone()), (1, vec![tip.clone()]), "asked once");
        assert_eq!(kept, known, "the answers the pass stood on, for the next");

        // A forge that does not answer lands nothing and keeps no answer.
        let mut set = [merged(&remote_only)];
        assert!(landed_reads(&mut set, &t, None, |_, _| None).is_empty());
        assert!(set[0].landed_tips.is_empty());

        // The worker committed after the merge: the head is behind the tip,
        // which git says without the forge.
        git(&["commit", "-q", "--allow-empty", "-m", "after"]);
        let after = git(&["rev-parse", "HEAD"]);
        let mut set = [merged(&tip)];
        let known = landed_reads(&mut set, &t, None, |_, _| {
            asked += 1;
            Some(true)
        });
        assert!(set[0].landed_tips.is_empty());
        assert_eq!(known.get(&("o/r".into(), tip.clone(), after.clone())), Some(&false));
        assert_eq!(forge::merged_at(&set, &here, &after), None, "a commit after is unlanded");

        // A side branch's PR lands only at its head, and a leg still at the
        // commit it was cut at is not asked.
        let mut side = merged(&remote_only);
        side.head_ref = "fix/a".into();
        let mut set = [side];
        assert!(landed_reads(&mut set, &t, None, |_, _| Some(true)).is_empty());
        git(&["reset", "-q", "--hard", &base]);
        let mut set = [merged(&remote_only)];
        assert!(landed_reads(&mut set, &t, None, |_, _| Some(true)).is_empty());
        assert!(set[0].landed_tips.is_empty());
        assert_eq!(asked, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A carried tip survives a restart on its PR (T-799), and seeds the
    /// answers so the first pass asks nothing again.
    #[test]
    fn a_carried_tip_round_trips_and_is_not_asked_again() {
        let t = ulid::Ulid(3);
        let mut s = State::default();
        let mut p = pr(42, PrState::Merged, Pipeline::Passed);
        p.landed_tips = vec!["a".repeat(40)];
        s.sets.insert(t, vec![p.clone()]);
        let text = serde_json::to_string(&ForgeFile::of(&s)).unwrap();
        assert!(text.contains("landed_tips"), "{text}");
        let mut back = State::default();
        back.restore(crate::store::parse_versioned(&text, FORGE_SCHEMA).unwrap(), |_| true);
        assert_eq!(back.sets[&t], vec![p]);
        let key = ("o/r".to_string(), "h".to_string(), "a".repeat(40));
        assert_eq!(back.landed[&t].get(&key), Some(&true));
        let landed = forge::merged_at(&back.sets[&t], &["o/r".into()], &"a".repeat(40));
        assert_eq!(landed.map(|p| p.number), Some(42));
        let plain = serde_json::to_string(&ForgeFile::of(&{
            let mut q = State::default();
            q.sets.insert(t, vec![pr(1, PrState::Open, Pipeline::None)]);
            q
        }))
        .unwrap();
        assert!(!plain.contains("landed_tips"), "written only where there is one: {plain}");
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
        assert!(log.iter().all(|m| m.oid.len() == 40 && m.wrote));
        assert!(reflog(&dir.join("nope")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T-796: a worktree cut at the base's tip, which a fast-forward merged
    /// MR named as its head, wrote none of the commits it was cut at or
    /// rebased onto; the ones it committed and the rebase's picks it did.
    #[test]
    fn a_worktree_wrote_its_commits_and_never_its_cut_or_its_rebases_base() {
        let root = std::env::temp_dir().join(format!("msmn-forge-wrote-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let git = |dir: &Path, args: &[&str]| -> String {
            let out = Command::new("git")
                .arg("-C")
                .arg(dir)
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
        let repo = root.join("repo");
        let wt = root.join("wt");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "base"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "their merged MR"]);
        let cut = git(&repo, &["rev-parse", "HEAD"]);
        git(&repo, &["worktree", "add", "-q", "-b", "msmn/T-1-x", wt.to_str().unwrap(), &cut]);
        let written = |dir: &Path| -> Vec<String> {
            reflog(dir).into_iter().filter(|m| m.wrote).map(|m| m.oid).collect()
        };
        assert!(!reflog(&wt).is_empty(), "the cut is in the reflog");
        assert_eq!(written(&wt), Vec::<String>::new(), "a fresh worktree wrote nothing");
        git(&wt, &["commit", "-q", "--allow-empty", "-m", "mine"]);
        let mine = git(&wt, &["rev-parse", "HEAD"]);
        assert_eq!(written(&wt), std::slice::from_ref(&mine));
        // Another merge lands on the base and the worktree rebases onto it:
        // the base's tip is the rebase's start, and its pick is new work.
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "their next MR"]);
        let next = git(&repo, &["rev-parse", "main"]);
        git(&wt, &["rebase", "-q", "--keep-empty", "main"]);
        let picked = git(&wt, &["rev-parse", "HEAD"]);
        let got = written(&wt);
        assert!(got.contains(&mine) && got.contains(&picked), "{got:?}");
        assert!(!got.contains(&cut) && !got.contains(&next), "{got:?}");
        // A reset to the base and a fast-forward pull of it write nothing.
        git(&wt, &["reset", "-q", "--hard", "main"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "their third MR"]);
        let third = git(&repo, &["rev-parse", "main"]);
        git(&wt, &["merge", "-q", "--ff-only", "main"]);
        assert_eq!(git(&wt, &["rev-parse", "HEAD"]), third);
        assert!(!written(&wt).contains(&third));
        let _ = std::fs::remove_dir_all(&root);
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
        // One failing repository backs off alone (T-800): the pass is due
        // on its minute, and leaves that one out until `BACKOFF` passed.
        s.last = Some(Instant::now() - EVERY);
        assert!(s.due(), "an error does not hold the whole board's pass");
        s.asked.insert("github.com/o/r".into(), Instant::now() - EVERY);
        assert!(s.backing_off("github.com/o/r"));
        assert!(!s.backing_off("github.com/o/s"), "the others are read");
        s.asked.insert("github.com/o/r".into(), Instant::now() - BACKOFF);
        assert!(!s.backing_off("github.com/o/r"), "and it again after the backoff");
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
                    lost: None,
                },
            ),
            (
                "github.com".to_string(),
                HostProbe { cli: None, at: Instant::now(), viewer: None, lost: None },
            ),
        ];
        list_probed(&mut q, &answered);
        let keys: Vec<String> = q.repos.iter().map(RepoQuery::key).collect();
        assert_eq!(keys, vec!["gitlab.lab/g/a", "gitlab.lab/g/b"]);
        assert!(q.repos.iter().all(|r| r.cli == glab && r.viewer.as_deref() == Some("me")));
        assert_eq!(q.repos[1].dir, PathBuf::from("/r/b"));
        assert!(q.pending.is_empty());
    }

    /// A repository that did not answer keeps the previous read's PRs; one
    /// that answered says what it says now (T-752, T-795).
    #[test]
    fn a_repository_that_did_not_answer_carries_its_prs_over() {
        let mut other = pr(9, PrState::Open, Pipeline::Running(String::new()));
        other.repo = "o/other".into();
        let old = vec![pr(1, PrState::Open, Pipeline::Running(String::new())), other.clone()];
        let new = vec![pr(1, PrState::Merged, Pipeline::Passed)];
        let only_r = HashSet::from(["o/r"]);
        let set = carry(Some(&old), new.clone(), &only_r);
        assert_eq!(set.len(), 2);
        assert!(set.iter().any(|p| p.number == 1 && p.state == PrState::Merged));
        assert!(set.contains(&other), "o/other did not answer: failed, unlisted or no CLI");
        let both = HashSet::from(["o/r", "o/other"]);
        assert_eq!(carry(Some(&old), new.clone(), &both), new, "every repository answered");
        assert_eq!(carry(None, new.clone(), &only_r), new, "a first read carries nothing");
        // An open PR's pipeline read that failed keeps the last one at the
        // same head, and only there.
        let was = pr(1, PrState::Open, Pipeline::Passed);
        let unread = pr(1, PrState::Open, Pipeline::Unknown);
        let set = carry(Some(std::slice::from_ref(&was)), vec![unread.clone()], &only_r);
        assert_eq!(set[0].pipeline, Pipeline::Passed);
        let mut pushed = unread.clone();
        pushed.head_oid = "moved".into();
        let set = carry(Some(std::slice::from_ref(&was)), vec![pushed], &only_r);
        assert_eq!(set[0].pipeline, Pipeline::Unknown, "a new head's pipeline is its own");
    }

    /// An open PR's failure names the open PRs failing, never a merged
    /// one whose settled failure is older news (T-795).
    #[test]
    fn a_failure_names_the_open_prs_that_fail() {
        let merged = pr(1, PrState::Merged, Pipeline::Failed("deploy".into()));
        let green = pr(2, PrState::Open, Pipeline::Passed);
        let red = pr(2, PrState::Open, Pipeline::Failed("test".into()));
        let old = vec![green, merged.clone()];
        let new = vec![red, merged];
        assert_eq!(edge(&old, &new).news(&new), ["pipeline failed #2: test"]);
    }

    /// A path segment or a query value is percent-encoded whole (T-795).
    #[test]
    fn an_api_word_is_percent_encoded() {
        assert_eq!(query_word("g/sub/p"), "g%2Fsub%2Fp");
        assert_eq!(query_word("Production EU+1#x"), "Production%20EU%2B1%23x");
        assert_eq!(query_word("a-b_c.d~e"), "a-b_c.d~e");
    }

    /// A ticket is read only where every repository it comes from answered,
    /// or nothing reads its host (T-795).
    #[test]
    fn a_ticket_is_read_only_where_every_repository_answered() {
        let remote = |host: &str, path: &str| Remote { host: host.into(), path: path.into() };
        let report = |key: &str, error: Option<&str>| RepoReport {
            key: key.into(),
            path: key.split_once('/').unwrap().1.into(),
            error: error.map(str::to_string),
        };
        let repos = vec![remote("github.com", "o/a"), remote("gitlab.com", "g/b")];
        let ok = [report("github.com/o/a", None), report("gitlab.com/g/b", None)];
        assert!(read_whole(Some(&repos), &ok, |_| false));
        let failed = [report("github.com/o/a", None), report("gitlab.com/g/b", Some("502"))];
        assert!(!read_whole(Some(&repos), &failed, |_| false), "a failed call reads nothing");
        let unlisted = [report("github.com/o/a", None)];
        assert!(!read_whole(Some(&repos), &unlisted, |_| false), "nor a repository not listed");
        assert!(
            read_whole(Some(&repos), &unlisted, |h| h == "gitlab.com"),
            "a host no CLI reads is no repository to wait for"
        );
        assert!(read_whole(None, &[], |_| false), "a ticket with no remote");
    }

    /// `forge.json` round-trips the sets, the conflicts said (T-785) and
    /// the settled commits (T-752),
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
        // The conflicts said (T-785): a PR the set no longer holds is not
        // written, nor a ticket the board no longer has.
        s.conflicted.insert(a, vec![("o/r".into(), 4), ("o/r".into(), 99)]);
        s.conflicted.insert(gone, vec![("o/r".into(), 5)]);
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
        assert_eq!(restored.conflicted, HashMap::from([(a, vec![("o/r".to_string(), 4)])]));
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
        let dir = PathBuf::from("/repo");
        s.remotes.insert(dir.clone(), Remote::parse("https://gitlab.com/g/p"));
        assert!(s.merged_at(t, &dir, "h").is_some());
        assert_eq!(set[0].merged_ms, 0, "an older file names no merge time");
    }
}
