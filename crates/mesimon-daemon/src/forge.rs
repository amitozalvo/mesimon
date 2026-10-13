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

use std::collections::{BTreeMap, HashMap};
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
/// The cadence after a repository failed to answer.
pub const BACKOFF: Duration = Duration::from_secs(300);
/// How long a host's probe (which CLI, signed in or not) is believed.
const PROBE_FOR: Duration = Duration::from_secs(6 * 3600);
/// How many reflog entries a leg offers for binding.
const REFLOG_CAP: &str = "2000";
/// How many PRs one list call brings back.
const LIST_LIMIT: &str = "100";

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
    /// Merge commits whose pipeline settled on an earlier pass, by
    /// repository path and oid: not asked again.
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
    /// A PR newly bound and open.
    pub opened: bool,
    /// The set's pipeline turned passed.
    pub passed: bool,
    /// Every PR of the set is merged, and was not before.
    pub merged: bool,
    /// Environments a merge commit reached since.
    pub deployed: Vec<String>,
    /// The set's pipeline turned failed: the job's word, empty where the
    /// forge named none.
    pub failed: Option<String>,
    /// The set's pipeline stopped on a person at a job it was not stopped
    /// on before: the job's word, empty where the forge named none.
    pub waiting: Option<String>,
    /// Open PRs, by label, whose conflict appeared (`true`) or cleared.
    pub conflict: Vec<(String, bool)>,
    /// Open PRs, by label, a reviewer's verdict newly landed on: approved
    /// or changes requested.
    pub review: Vec<(String, Review)>,
}

impl Edge {
    pub fn any(&self) -> bool {
        self.opened
            || self.passed
            || self.merged
            || !self.deployed.is_empty()
            || self.failed.is_some()
            || self.waiting.is_some()
            || !self.conflict.is_empty()
            || !self.review.is_empty()
    }

    /// What the crown hears of the edge (T-744), one phrase per edge, each
    /// naming its PR: `pipeline passed !1`, `waiting on deploy:production
    /// !1`, `deployed to staging !1`, `conflict !1`, `conflict cleared !1`,
    /// `review approved !1`, `changes requested !1`, `pipeline failed !1:
    /// test-linux`. Never an opening (the worker's turn end carries it) and
    /// never a merge (the flags' merged wake says it). `new` is the read the
    /// edge was taken on, for the labels.
    pub fn news(&self, new: &[Pr]) -> Vec<String> {
        let labels = |pick: &dyn Fn(&Pr) -> bool| -> String {
            new.iter().filter(|p| pick(p)).map(Pr::label).collect::<Vec<_>>().join(" ")
        };
        // The PRs `pipeline_of` read the set's word off: the open ones, or
        // the newest merged once nothing is open.
        let lead = || {
            let open = labels(&|p| p.state.is_open());
            if open.is_empty() {
                new.iter().find(|p| p.state == PrState::Merged).map(Pr::label).unwrap_or_default()
            } else {
                open
            }
        };
        let tail =
            |label: String| if label.is_empty() { String::new() } else { format!(" {label}") };
        let mut out = Vec::new();
        if let Some(job) = &self.failed {
            let label = labels(&|p| matches!(p.pipeline, Pipeline::Failed(_)));
            match forge::job_word(job) {
                j if j.is_empty() => out.push(format!("pipeline failed{}", tail(label))),
                j => out.push(format!("pipeline failed{}: {j}", tail(label))),
            }
        }
        if self.passed {
            out.push(format!("pipeline passed{}", tail(lead())));
        }
        if let Some(job) = &self.waiting {
            let label = labels(&|p| matches!(p.pipeline, Pipeline::Waiting(_)));
            match forge::job_word(job) {
                j if j.is_empty() => out.push(format!("waiting on a manual job{}", tail(label))),
                j => out.push(format!("waiting on {j}{}", tail(label))),
            }
        }
        for env in &self.deployed {
            let label = new
                .iter()
                .find(|p| p.state == PrState::Merged && p.deployed.contains(env))
                .map(Pr::label)
                .unwrap_or_default();
            out.push(format!("deployed to {}{}", forge::job_word(env), tail(label)));
        }
        for (label, on) in &self.conflict {
            out.push(if *on {
                format!("conflict {label}")
            } else {
                format!("conflict cleared {label}")
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

/// The edge between two reads of one ticket's set.
pub fn edge(old: &[Pr], new: &[Pr]) -> Edge {
    let was = |p: &Pr| old.iter().find(|o| o.repo == p.repo && o.number == p.number);
    let all_merged = |s: &[Pr]| !s.is_empty() && s.iter().all(|p| p.state == PrState::Merged);
    let envs = |s: &[Pr]| -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for p in s.iter().filter(|p| p.state == PrState::Merged) {
            for e in &p.deployed {
                if !out.contains(e) {
                    out.push(e.clone());
                }
            }
        }
        out
    };
    let before = envs(old);
    let (then, now) = (forge::pipeline_of(old), forge::pipeline_of(new));
    // An open PR read open before too: a conflict or a review on one just
    // bound is part of its opening, which is not news.
    let kept: Vec<(&Pr, &Pr)> = new
        .iter()
        .filter(|p| p.state.is_open())
        .filter_map(|p| was(p).filter(|o| o.state.is_open()).map(|o| (o, p)))
        .collect();
    Edge {
        opened: new.iter().any(|p| p.state.is_open() && was(p).is_none()),
        passed: now == Pipeline::Passed && then != Pipeline::Passed,
        merged: all_merged(new) && !all_merged(old),
        deployed: envs(new).into_iter().filter(|e| !before.contains(e)).collect(),
        failed: match &now {
            Pipeline::Failed(job) if !matches!(then, Pipeline::Failed(_)) => Some(job.clone()),
            _ => None,
        },
        waiting: match &now {
            Pipeline::Waiting(job) if then != now => Some(job.clone()),
            _ => None,
        },
        conflict: kept
            .iter()
            .filter(|(o, p)| o.conflict != p.conflict)
            .map(|(_, p)| (p.label(), p.conflict))
            .collect(),
        review: kept
            .iter()
            .filter(|(o, p)| {
                o.review != p.review
                    && matches!(p.review, Review::Approved | Review::ChangesRequested)
            })
            .map(|(_, p)| (p.label(), p.review))
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
}

/// The pass, on a worker thread.
pub fn sample(mut q: Query, now_ms: u64) -> Sample {
    let mut out = Sample { at_ms: now_ms, ..Default::default() };
    for p in &q.probes {
        out.probes.push((p.host.clone(), probe_host(&p.host, &p.candidates)));
    }
    list_probed(&mut q, &out.probes);
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
    for t in &q.tickets {
        let facts = Facts {
            branch: t.branch.clone(),
            reflog: t
                .legs
                .iter()
                .flat_map(|l| reflog(&l.dir).into_iter().map(move |oid| (l.repo_path.clone(), oid)))
                .collect(),
            opened: t.opened.clone(),
        };
        let bound: Vec<Pr> = prs.iter().filter(|p| forge::binds(p, &facts)).cloned().collect();
        let mut set = forge::ticket_set(bound);
        // GitLab's list says nothing of a pipeline: ask by sha for the open
        // ones that bound, one call each — the set is small.
        for pr in set.iter_mut().filter(|p| p.forge == Forge::GitLab && p.state.is_open()) {
            if let Some(r) = q.repos.iter().find(|r| r.remote.path == pr.repo) {
                pr.pipeline = gitlab_pipeline(r, &pr.head_oid);
            }
        }
        // The newest merged PR per repository: its merge commit's pipeline
        // (the build-and-deploy run on main) unless an earlier pass settled
        // it, and the environments it reached where a column waits for one.
        let mut seen_repo: Vec<String> = Vec::new();
        for pr in set.iter_mut().filter(|p| p.state == PrState::Merged && !p.merge_oid.is_empty()) {
            if seen_repo.contains(&pr.repo) {
                continue;
            }
            seen_repo.push(pr.repo.clone());
            let Some(r) = q.repos.iter().find(|r| r.remote.path == pr.repo) else { continue };
            let key = (pr.repo.clone(), pr.merge_oid.clone());
            pr.pipeline = match q.commit_pipelines.get(&key) {
                Some(p) => p.clone(),
                None => commit_pipeline(r, &pr.merge_oid),
            };
            let mut known = q.deployed.get(&key).cloned().unwrap_or_default();
            if q.envs.iter().any(|e| !known.contains(e)) {
                for env in deployments(r, &pr.merge_oid, &q.envs) {
                    if !known.contains(&env) {
                        known.push(env);
                    }
                }
            }
            pr.deployed = known;
        }
        out.sets.insert(t.ticket, set);
    }
    out
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

/// A GitLab commit's newest pipeline, by sha. `Unknown` when the call
/// fails: never read as green.
fn gitlab_pipeline(r: &RepoQuery, sha: &str) -> Pipeline {
    let project = r.remote.path.replace('/', "%2F");
    let path = format!("projects/{project}/pipelines?sha={sha}&per_page=1");
    match run(&r.cli.bin, &["api", "--hostname", &r.remote.host, &path], Some(&r.dir)) {
        Ok(out) => forge::parse_gitlab_pipelines(&out),
        Err(_) => Pipeline::Unknown,
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
/// deployments of each wanted environment naming the sha.
fn deployments(r: &RepoQuery, oid: &str, envs: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    match r.cli.forge {
        Forge::GitHub => {
            let path = format!("repos/{}/deployments?sha={oid}&per_page=30", r.remote.path);
            let Ok(list) =
                run(&r.cli.bin, &["api", "--hostname", &r.remote.host, &path], Some(&r.dir))
            else {
                return out;
            };
            for (id, env) in forge::parse_gh_deployments(&list) {
                if !envs.contains(&env) || out.contains(&env) {
                    continue;
                }
                let path = format!("repos/{}/deployments/{id}/statuses?per_page=1", r.remote.path);
                if let Ok(status) =
                    run(&r.cli.bin, &["api", "--hostname", &r.remote.host, &path], Some(&r.dir))
                {
                    if forge::parse_gh_deployment_success(&status) {
                        out.push(env);
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
                    if forge::parse_gitlab_deployed(&list, oid) {
                        out.push(env.clone());
                    }
                }
            }
        }
    }
    out
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
    /// The repositories whose last call failed, with the CLI's words.
    pub errors: Vec<(String, String)>,
    /// Merge commits whose pipeline settled, by repository path and oid.
    pub commit_pipelines: HashMap<(String, String), Pipeline>,
    /// The environments each merge commit was seen reaching.
    pub deployed: HashMap<(String, String), Vec<String>>,
    pub last: Option<Instant>,
    /// Unix ms of the last pass that answered, for the snapshot.
    pub last_ms: u64,
    pub inflight: bool,
    pub gen: u64,
}

impl State {
    /// The CLI a host is read through, if the probe said one.
    pub fn cli_for(&self, host: &str) -> Option<&Cli> {
        self.probes.get(host).and_then(|p| p.cli.as_ref())
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

    /// The open PRs of a ticket's set.
    pub fn open_of(&self, ticket: ulid::Ulid) -> Vec<&Pr> {
        self.sets
            .get(&ticket)
            .map(|s| s.iter().filter(|p| p.state.is_open()).collect())
            .unwrap_or_default()
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

/// What the passes settled of one merge commit: its terminal pipeline and
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
            let set: Vec<Pr> = records.into_iter().filter_map(PrRecord::pr).collect();
            self.sets.insert(t, forge::ticket_set(set));
        }
        for c in file.commits {
            let key = (c.repo, c.oid);
            if let p @ (Pipeline::Passed | Pipeline::Failed(_)) =
                Pipeline::from_words(&c.pipeline, Some(&c.job))
            {
                self.commit_pipelines.insert(key.clone(), p);
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
        assert!(e.passed && e.merged && !e.opened && e.deployed.is_empty());
        m.deployed = vec!["staging".into()];
        let e = edge(&[pr(1, PrState::Merged, Pipeline::Passed)], &[m.clone()]);
        assert_eq!(e.deployed, ["staging"]);
        assert!(!e.merged, "merged before too");
        let e = edge(&[m.clone()], &[pr(2, PrState::Open, Pipeline::None), m.clone()]);
        assert!(e.opened && !e.merged, "a follow-up PR opens");
        assert!(!edge(&[], &[]).any());
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
        // A PR bound already conflicted and approved: its opening, not news.
        let mut fresh = gl(2, Pipeline::Running);
        fresh.conflict = true;
        fresh.review = Review::Approved;
        let e = edge(std::slice::from_ref(&running), &[fresh.clone(), running.clone()]);
        assert!(e.opened);
        assert!(e.news(&[fresh, running.clone()]).is_empty());
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
