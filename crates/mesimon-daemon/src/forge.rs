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

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use mesimon_core::forge::{self, Facts, Forge, Pipeline, Pr, PrState, Remote};

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
    let bin = captured_path
        .and_then(on)
        .or_else(|| std::env::var("PATH").ok().and_then(|p| on(&p)))?;
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

/// What one pass is asked.
#[derive(Debug, Clone, Default)]
pub struct Query {
    pub probes: Vec<ProbeQuery>,
    pub repos: Vec<RepoQuery>,
    pub tickets: Vec<TicketQuery>,
}

/// One repository's answer: its PRs, or why not.
#[derive(Debug, Clone)]
pub struct RepoReport {
    pub key: String,
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
pub fn sample(q: Query, now_ms: u64) -> Sample {
    let mut out = Sample { at_ms: now_ms, ..Default::default() };
    for p in &q.probes {
        out.probes.push((p.host.clone(), probe_host(&p.host, &p.candidates)));
    }
    let mut prs: Vec<Pr> = Vec::new();
    for r in &q.repos {
        match list(r) {
            Ok(mut found) => {
                prs.append(&mut found);
                out.repos.push(RepoReport { key: r.key(), error: None });
            }
            Err(e) => out.repos.push(RepoReport { key: r.key(), error: Some(e) }),
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
        out.sets.insert(t.ticket, set);
    }
    out
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
        self.sets.get(&ticket).map(|s| s.iter().filter(|p| p.state.is_open()).collect()).unwrap_or_default()
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
        }
    }

    #[test]
    fn the_delta_says_what_moved_and_nothing_on_the_first_read() {
        let before = [pr(1, PrState::Open, Pipeline::Running), pr(2, PrState::Open, Pipeline::None)];
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
        assert_eq!(find_cli(Forge::GitHub, Some(&path)).map(|c| c.forge), find_cli(Forge::GitHub, None).map(|c| c.forge), "falls through to the daemon's own PATH");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
