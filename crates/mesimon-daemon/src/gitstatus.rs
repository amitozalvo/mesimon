//! Where the board's own checkout stands: branch, ahead/behind its upstream,
//! uncommitted changes (T-124).
//!
//! This is the REPO's state, not a ticket's worktree (`crate::worktree` owns
//! those). `git status --porcelain=v2 --branch -z`
//! carries the branch, the upstream, the ahead/behind pair and every changed
//! entry in one answer — and it runs OFF the writer thread (`server.rs`
//! spawns it and takes the result back as a `Msg`), because a `status` on a
//! large tree is tens of milliseconds the board should not wait on.
//!
//! The fetch is opt-in (`MESIMON_GIT_FETCH=<minutes>`) or a menu press, never
//! automatic: it reaches the network and writes remote-tracking refs, which
//! the README's allowlist names as the one thing mesimon writes in `.git`
//! beyond what it created. It runs on the same worker, BEFORE the sample, so
//! the two never race, and it is fenced three ways — every prompt door git
//! and ssh have is closed, the child is its own session so a hang cannot
//! hold the writer's flag and the whole process group can be killed on the
//! deadline (`Child::kill` reaches `git`, not the `git-remote-https`/`ssh` it
//! spawned, and those would keep the stderr pipe open forever).
//!
//! Scheduling — the flags, the cadence, the damping — stays in `server.rs`;
//! this module is the sampling and the parsing, like `resources.rs`.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use mesimon_core::command::{GitCommit, RepoGit, RepoSync};

/// A fetch that has not answered in this long is killed. A detached daemon
/// has no tty, so nothing on the other side of a passphrase prompt will ever
/// answer; a slow network gets the next cycle.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// One `git status` on `repo`, parsed. Not a repository, or no git at all,
/// reads as "not sampled" — the header draws nothing rather than a guess.
pub fn sample_one(repo: &Path) -> RepoGit {
    let out = crate::git::git(repo)
        .args(["--no-optional-locks", "status", "--porcelain=v2", "--branch", "-unormal", "-z"])
        .stdin(Stdio::null())
        .output();
    match out {
        Ok(o) if o.status.success() => parse(&o.stdout),
        _ => RepoGit::default(),
    }
}

/// History belongs only to the branch named by the sample.
fn with_commits(repo: &Path, mut g: RepoGit) -> RepoGit {
    if g.upstream.is_some() && !g.detached {
        g.to_push = commits(repo, "@{upstream}..HEAD", g.ahead);
        g.to_pull = commits(repo, "HEAD..@{upstream}", g.behind);
    }
    g
}

/// One nested repo of a workspace against its remote (T-455). Its own
/// upstream when one is configured; otherwise the one remote branch with the
/// same name — the author's workspace had eight of twelve repos pushed with
/// `git push gitlab main` and never linked, so `git status` saw nothing to
/// compare while `gitlab/main` sat right there. Read-only either way: no
/// config is written to make the link.
fn nested_sync(dir: &Path, name: &str, g: RepoGit) -> RepoSync {
    let mut s = RepoSync {
        name: name.to_string(),
        branch: g.branch.clone(),
        detached: g.detached,
        upstream: g.upstream.clone(),
        ahead: g.ahead,
        behind: g.behind,
        fetched_at_ms: fetched_at_ms(dir),
        ..RepoSync::default()
    };
    if s.detached {
        s.upstream = None;
        return s;
    }
    let rev = if s.upstream.is_some() {
        "@{upstream}".to_string()
    } else {
        let Some((refname, ahead, behind)) = same_name_remote(dir, &s.branch) else { return s };
        s.upstream = Some(refname.trim_start_matches("refs/remotes/").to_string());
        s.by_name = true;
        (s.ahead, s.behind) = (ahead, behind);
        refname
    };
    s.to_push = commits(dir, &format!("{rev}..HEAD"), s.ahead);
    s.to_pull = commits(dir, &format!("HEAD..{rev}"), s.behind);
    s
}

/// `FETCH_HEAD`'s mtime in unix ms, 0 when there is none. A census repo's
/// `.git` is a directory, so the file is where git writes it. One `stat`,
/// read-only.
fn fetched_at_ms(dir: &Path) -> u64 {
    std::fs::metadata(dir.join(".git/FETCH_HEAD"))
        .and_then(|m| m.modified())
        .ok()
        .and_then(mesimon_core::clock::epoch_ms)
        .unwrap_or(0)
}

/// `refs/remotes/<remote>/<branch>` when exactly one remote carries the
/// branch, with HEAD's (ahead, behind) against it. Two remotes with the name
/// is a guess this refuses to make. `*` matches one path segment in git's
/// ref patterns, and a branch name cannot hold a glob character, so the
/// pattern names exactly the candidates.
fn same_name_remote(dir: &Path, branch: &str) -> Option<(String, u32, u32)> {
    if branch.is_empty() {
        return None;
    }
    let out = crate::git::git(dir)
        .args(["--no-optional-locks", "for-each-ref", "--format=%(refname)"])
        .arg(format!("refs/remotes/*/{branch}"))
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut refs = text.lines().filter(|l| !l.is_empty());
    let (Some(only), None) = (refs.next(), refs.next()) else { return None };
    // An unborn HEAD has nothing to count, and the comparison fails here.
    let out = crate::git::git(dir)
        .args(["--no-optional-locks", "rev-list", "--left-right", "--count"])
        .arg(format!("HEAD...{only}"))
        .arg("--")
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let counts = String::from_utf8_lossy(&out.stdout);
    let mut n = counts.split_whitespace().map(|t| t.parse::<u32>().ok());
    let (Some(Some(ahead)), Some(Some(behind))) = (n.next(), n.next()) else { return None };
    Some((only.to_string(), ahead, behind))
}

/// Bounded, read-only history on the same worker as status. Only local refs
/// are read: incoming commits change when the user or Mesimon fetches.
fn commits(repo: &Path, range: &str, count: u32) -> Option<Vec<GitCommit>> {
    if count == 0 {
        return Some(Vec::new());
    }
    let out = crate::git::git(repo)
        .args([
            "--no-pager",
            "log",
            "--no-show-signature",
            "--format=%H%x00%s",
            "-z",
            "--max-count=100",
            range,
            "--",
        ])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let mut fields = out.stdout.split(|b| *b == 0);
    let mut commits = Vec::new();
    while let (Some(oid), Some(subject)) = (fields.next(), fields.next()) {
        commits.push(GitCommit {
            oid: String::from_utf8_lossy(oid).into_owned(),
            subject: String::from_utf8_lossy(subject).chars().take(512).collect(),
        });
    }
    Some(commits)
}

/// The board root's sample (T-225): the root's own status, plus the census
/// of nested repositories and, when there are any, their changed counts
/// summed in. A folder of repos with no repository at the root still reads
/// as sampled — there IS a checkout under the board, nineteen of them — so
/// the header speaks and `v` has something to open; its `branch` stays
/// empty because the root has no HEAD to name.
pub fn sample(root: &Path) -> RepoGit {
    let mut g = sample_one(root);
    let repos = census(root);
    if repos.is_empty() {
        return with_commits(root, g);
    }
    // A root that is a repository keeps its own branch and arrows whatever
    // is nested under it — the mesimon checkout carries `mt/`, a scratch
    // repo on a branch called `orphan`, and for an hour the board called
    // that the checkout (author 2026-09-05: "why does it say orphan and not
    // main"). Only a FOLDER holding exactly one repo takes that repo's
    // branch: there the child is the checkout, and `1 repo` says nothing.
    if !g.sampled {
        if let [only] = &repos[..] {
            let mut child = with_commits(&root.join(only), sample_one(&root.join(only)));
            child.sampled = true;
            child.repos = repos;
            return child;
        }
    }
    // Each child's branch against its remote rides along where the header
    // names the workspace by its count — two or more. One child under a
    // repository root is not a workspace to the header (`mt/` again), and
    // its lists would be the only ones drawn about a repo it never names.
    let workspace = repos.len() > 1;
    for name in &repos {
        let dir = root.join(name);
        let child = sample_one(&dir);
        g.changed += child.changed;
        if workspace && child.sampled {
            g.nested.push(nested_sync(&dir, name, child));
        }
    }
    g.sampled = true;
    g.repos = repos;
    with_commits(root, g)
}

/// Where the sampled branch lives: the root when it is a repository, else
/// the one nested repo that stands in for it. The fetch runs there, because
/// `branch.<b>.remote` is that repository's config.
pub fn branch_dir(root: &Path, git: &RepoGit) -> std::path::PathBuf {
    stand_in(root, &git.repos)
}

fn stand_in(root: &Path, repos: &[String]) -> std::path::PathBuf {
    match repos {
        [only] if !root.join(".git").exists() => root.join(only),
        _ => root.to_path_buf(),
    }
}

/// The repository a listed commit is read in (T-455): a nested repo the
/// list named — a census name and nothing else, so a client cannot aim git
/// at any other directory — or else where the board's own branch is sampled.
/// That second road is the one-repo folder's too: its lists are the child's,
/// and the root it sits in is no repository at all.
pub fn commit_dir(root: &Path, repo: Option<&str>) -> Result<std::path::PathBuf, String> {
    let repos = census(root);
    match repo {
        Some(name) if repos.iter().any(|r| r == name) => Ok(root.join(name)),
        Some(name) => Err(format!("no repository {name} in this workspace")),
        None => Ok(stand_in(root, &repos)),
    }
}

/// The root's immediate child directories that are repositories of their own
/// — `workspace::nested_repos` over one `readdir` and a `symlink_metadata`
/// of `<child>/.git` each. Depth one, never recursive; 1.5 ms on the
/// author's twenty-repo workspace. An unreadable root is an empty census.
pub fn census(root: &Path) -> Vec<String> {
    use mesimon_core::workspace::{nested_repos, submodule_paths, GitMark};
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    let mut children: Vec<(String, GitMark)> = Vec::new();
    for e in entries.flatten() {
        // The child itself through symlinks is fine (a symlinked repo dir is
        // a repo); `.git` is probed without following, the way git does.
        if !e.path().is_dir() {
            continue;
        }
        let Ok(name) = e.file_name().into_string() else { continue };
        let mark = match std::fs::symlink_metadata(e.path().join(".git")) {
            Ok(md) if md.is_dir() => GitMark::Dir,
            Ok(md) if md.is_file() => GitMark::File,
            _ => GitMark::None,
        };
        children.push((name, mark));
    }
    let submodules = std::fs::read_to_string(root.join(".gitmodules"))
        .map(|t| submodule_paths(&t))
        .unwrap_or_default();
    nested_repos(children.iter().map(|(n, m)| (n.as_str(), *m)), &submodules)
}

/// The porcelain v2 `--branch -z` stream, distilled. Headers are `# key value`
/// records; every other record is one changed entry — and under `-z` a
/// rename (`2 …`) is followed by a SECOND NUL-terminated field, the original
/// path, which is why the walk is a cursor and not a count of NULs.
pub fn parse(bytes: &[u8]) -> RepoGit {
    let mut g = RepoGit { sampled: true, ..RepoGit::default() };
    let mut oid = String::new();
    let mut fields = bytes.split(|b| *b == 0);
    while let Some(rec) = fields.next() {
        if rec.is_empty() {
            continue;
        }
        if let Some(rest) = rec.strip_prefix(b"# ") {
            let rest = String::from_utf8_lossy(rest);
            let (key, val) = rest.split_once(' ').unwrap_or((&rest, ""));
            match key {
                "branch.oid" => oid = val.to_string(),
                "branch.head" if val == "(detached)" => g.detached = true,
                "branch.head" => g.branch = val.to_string(),
                "branch.upstream" => g.upstream = Some(val.to_string()),
                "branch.ab" => {
                    for tok in val.split_whitespace() {
                        if let Some(n) = tok.strip_prefix('+') {
                            g.ahead = n.parse().unwrap_or(0);
                        } else if let Some(n) = tok.strip_prefix('-') {
                            g.behind = n.parse().unwrap_or(0);
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        match rec.first() {
            Some(b'1') | Some(b'u') | Some(b'?') => g.changed += 1,
            Some(b'2') => {
                g.changed += 1;
                // The rename's original path rides as its own field.
                fields.next();
            }
            _ => {}
        }
    }
    if g.detached {
        // An unborn HEAD reports `(initial)`, never detached, so the oid here
        // is always a real one; seven is what `git log --oneline` shows.
        g.branch = oid.chars().take(7).collect();
    }
    g
}

/// The remote the branch's upstream lives on (`branch.<name>.remote`), or
/// None when there is none or it is `.` (a local branch as upstream — nothing
/// to reach over the network for).
pub fn remote_of(repo: &Path, branch: &str) -> Option<String> {
    if branch.is_empty() {
        return None;
    }
    let key = format!("branch.{branch}.remote");
    let out = crate::git::git(repo)
        .args(["--no-optional-locks", "config", "--get", &key])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let remote = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!remote.is_empty() && remote != ".").then_some(remote)
}

/// `git fetch <remote>`, fenced. Ok on success; Err carries the first stderr
/// line (git's own words) or the timeout.
///
/// `-c gc.auto=0 -c maintenance.auto=false`: a fetch may otherwise trigger
/// `gc --auto`, which repacks objects and packed-refs — writes far outside the
/// allowlist clause. `--no-write-fetch-head` (git 2.29) keeps `FETCH_HEAD`
/// untouched; an older git refuses the option and that refusal is the honest
/// answer. `GIT_SSH_COMMAND` is deliberately NOT set: it would override
/// `core.sshCommand` and the user's own identity setup — their agent socket
/// and credential helper reach the fetch exactly as they reach their shell.
pub fn fetch(repo: &Path, remote: &str) -> Result<(), String> {
    let mut cmd = crate::git::git(repo);
    cmd.args([
        "-c",
        "gc.auto=0",
        "-c",
        "maintenance.auto=false",
        "fetch",
        "--quiet",
        "--no-write-fetch-head",
        "--no-tags",
        "--no-recurse-submodules",
        remote,
    ]);
    // Every prompt door: git's tty prompt, git's askpass (an empty value
    // shadows `core.askPass` AND `SSH_ASKPASS` in git's own ladder), and
    // OpenSSH's GUI askpass under a DISPLAY.
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.env("GIT_ASKPASS", "");
    cmd.env("SSH_ASKPASS_REQUIRE", "never");
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    // Its own session: no controlling tty even under a foreground daemon, and
    // a process group of its own to kill whole on the deadline.
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = cmd.spawn().map_err(|e| format!("git: {e}"))?;
    let stderr = child.stderr.take();
    let first_line = std::thread::spawn(move || {
        let Some(stderr) = stderr else { return String::new() };
        let mut first = String::new();
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            // Drain to EOF so the child never blocks on a full pipe; keep the
            // first non-empty line, which is where git says what went wrong.
            if first.is_empty() && !line.trim().is_empty() {
                first = line.trim().to_string();
            }
        }
        first
    });
    let deadline = Instant::now() + FETCH_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                // The group, not the child: `git-remote-https` / `ssh` are
                // its children and hold the pipe.
                // SAFETY: a signal to a process group this daemon created.
                unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
                let _ = child.wait();
                let _ = first_line.join();
                return Err(format!("fetch did not finish within {}s", FETCH_TIMEOUT.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(e) => return Err(format!("git: {e}")),
        }
    };
    let first = first_line.join().unwrap_or_default();
    if status.success() {
        Ok(())
    } else if first.is_empty() {
        Err(format!("git fetch exited {}", status.code().unwrap_or(-1)))
    } else {
        Err(first)
    }
}

/// How many fetches a press runs at once. Twelve repos one after another is
/// twelve round trips of waiting; four at a time is three, and still a
/// handful of connections to one forge.
const FETCH_WIDTH: usize = 4;

/// What one fetch pass brought back: the board's own branch, if it was
/// fetched, and each nested repo that was, by census name.
#[derive(Debug, Default)]
pub struct Fetched {
    pub root: Option<Result<(), String>>,
    pub nested: Vec<(String, Result<(), String>)>,
}

/// The remote a nested repo's comparison reads (T-455): its branch's
/// configured remote, or for a same-name match the remote the match came
/// from — the segment before the first `/`, exactly, because the pattern's
/// `*` matched one segment. None where nothing is over the network: no
/// comparison, a detached HEAD, a local branch as upstream.
pub fn nested_remote(dir: &Path, s: &RepoSync) -> Option<String> {
    let upstream = s.upstream.as_deref().filter(|_| !s.detached)?;
    if s.by_name {
        return upstream.split_once('/').map(|(remote, _)| remote.to_string());
    }
    remote_of(dir, &s.branch)
}

/// One fetch pass (T-455): the board's own branch (`own` is where it lives
/// and its name) and each of `nested`, `FETCH_WIDTH` at a time, each with
/// [`fetch`]'s fences. A repo with no remote to reach is left out of the
/// answer rather than failed — there was nothing to try.
pub fn fetch_pass(
    root: &Path,
    own: Option<(std::path::PathBuf, String)>,
    nested: &[RepoSync],
) -> Fetched {
    let mut jobs: Vec<(Option<String>, std::path::PathBuf, String)> = Vec::new();
    if let Some((dir, branch)) = own {
        if let Some(remote) = remote_of(&dir, &branch) {
            jobs.push((None, dir, remote));
        }
    }
    for s in nested {
        let dir = root.join(&s.name);
        if let Some(remote) = nested_remote(&dir, s) {
            jobs.push((Some(s.name.clone()), dir, remote));
        }
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results: Vec<(Option<String>, Result<(), String>)> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..FETCH_WIDTH.min(jobs.len()))
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some((name, dir, remote)) = jobs.get(i) else { break done };
                        done.push((name.clone(), fetch(dir, remote)));
                    }
                })
            })
            .collect();
        workers.into_iter().flat_map(|w| w.join().unwrap_or_default()).collect()
    });
    let mut out = Fetched::default();
    for (name, verdict) in results {
        match name {
            None => out.root = Some(verdict),
            Some(name) => out.nested.push((name, verdict)),
        }
    }
    out
}

/// `MESIMON_GIT_FETCH=<minutes>` — the periodic fetch's cadence, zero when
/// unset, unparsable or 0. Read once at daemon start; `doctor` prints it.
pub fn fetch_every_from_env() -> Duration {
    fetch_every(std::env::var("MESIMON_GIT_FETCH").ok().as_deref())
}

/// The cadence a `MESIMON_GIT_FETCH` value means.
pub fn fetch_every(value: Option<&str>) -> Duration {
    let minutes: u64 = value.and_then(|v| v.trim().parse().ok()).unwrap_or(0);
    Duration::from_secs(minutes * 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn z(records: &[&str]) -> Vec<u8> {
        let mut v = Vec::new();
        for r in records {
            v.extend_from_slice(r.as_bytes());
            v.push(0);
        }
        v
    }

    #[test]
    fn a_clean_tracking_branch_in_sync() {
        let g = parse(&z(&[
            "# branch.oid 0123456789abcdef0123456789abcdef01234567",
            "# branch.head main",
            "# branch.upstream origin/main",
            "# branch.ab +0 -0",
        ]));
        assert!(g.sampled);
        assert_eq!(g.branch, "main");
        assert!(!g.detached);
        assert_eq!(g.upstream.as_deref(), Some("origin/main"));
        assert_eq!((g.ahead, g.behind, g.changed), (0, 0, 0));
    }

    #[test]
    fn ahead_behind_and_every_kind_of_change_counts_once() {
        let g = parse(&z(&[
            "# branch.oid 0123456789abcdef0123456789abcdef01234567",
            "# branch.head feature",
            "# branch.upstream origin/feature",
            "# branch.ab +2 -1",
            "1 .M N... 100644 100644 100644 abc abc src/lib.rs",
            "1 M. N... 100644 100644 100644 abc abc src/main.rs",
            // A rename: the original path is its own NUL field and must not
            // count as an entry of its own.
            "2 R. N... 100644 100644 100644 abc abc R100 new.rs",
            "old.rs",
            "u UU N... 100644 100644 100644 100644 abc abc abc conflict.rs",
            "? notes.txt",
        ]));
        assert_eq!((g.ahead, g.behind), (2, 1));
        assert_eq!(g.changed, 5, "five entries, not six");
    }

    #[test]
    fn no_upstream_means_no_arrows() {
        let g = parse(&z(&[
            "# branch.oid 0123456789abcdef0123456789abcdef01234567",
            "# branch.head topic",
        ]));
        assert_eq!(g.branch, "topic");
        assert!(g.upstream.is_none());
        assert_eq!((g.ahead, g.behind), (0, 0));
    }

    #[test]
    fn detached_head_names_the_short_oid() {
        let g = parse(&z(&[
            "# branch.oid a1b2c3d4e5f60123456789abcdef0123456789ab",
            "# branch.head (detached)",
            "? x",
        ]));
        assert!(g.detached);
        assert_eq!(g.branch, "a1b2c3d");
        assert_eq!(g.changed, 1);
    }

    #[test]
    fn an_unborn_branch_is_a_branch() {
        let g = parse(&z(&["# branch.oid (initial)", "# branch.head main"]));
        assert_eq!(g.branch, "main");
        assert!(!g.detached);
    }

    #[test]
    fn empty_output_is_a_clean_sample() {
        let g = parse(b"");
        assert!(g.sampled);
        assert_eq!(g.changed, 0);
    }

    #[test]
    fn a_real_checkout_samples_its_own_branch() {
        if !crate::worktree::have_git() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("msmn-gitstatus-real-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| {
            let out = Command::new("git").arg("-C").arg(&dir).args(args).output().unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        run(&["init", "-q", "-b", "main"]);
        run(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "one",
        ]);
        std::fs::write(dir.join("f"), "x").unwrap();
        let g = sample(&dir);
        assert!(g.sampled);
        assert_eq!(g.branch, "main");
        assert!(g.upstream.is_none());
        assert_eq!(g.changed, 1);
        assert!(remote_of(&dir, "main").is_none(), "no upstream, no remote");
        assert!(g.to_push.is_none());
        assert!(g.to_pull.is_none());

        run(&["branch", "upstream"]);
        run(&["branch", "--set-upstream-to=upstream", "main"]);
        let commit = |subject: &str| {
            run(&[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                subject,
            ])
        };
        commit("older outgoing");
        commit("newer outgoing 統一碼");
        run(&["checkout", "-q", "upstream"]);
        commit("incoming");
        run(&["checkout", "-q", "main"]);
        let g = sample(&dir);
        assert_eq!((g.ahead, g.behind), (2, 1));
        let outgoing = g.to_push.unwrap();
        assert_eq!(
            outgoing.iter().map(|c| c.subject.as_str()).collect::<Vec<_>>(),
            ["newer outgoing 統一碼", "older outgoing"]
        );
        assert!(outgoing.iter().all(|c| c.oid.len() == 40));
        assert_eq!(g.to_pull.unwrap()[0].subject, "incoming");
        assert!(commits(&dir, "missing..HEAD", 1).is_none());

        for _ in 0..100 {
            commit("another outgoing");
        }
        let g = sample(&dir);
        assert_eq!(g.ahead, 102);
        assert_eq!(g.to_push.unwrap().len(), 100, "snapshot history is bounded");
        run(&["checkout", "-q", "--detach"]);
        let g = sample(&dir);
        assert!(g.detached);
        assert!(g.to_push.is_none());
        assert!(g.to_pull.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A folder of four repos, one per way a child can stand (T-455): a
    /// linked upstream, a same-name remote branch nobody linked, no remote
    /// at all, and a detached HEAD.
    #[test]
    fn a_workspace_lists_each_repo_against_its_remote() {
        if !crate::worktree::have_git() {
            return;
        }
        let root = std::env::temp_dir().join(format!("msmn-gitstatus-ws-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let git = |repo: &str, args: &[&str]| -> String {
            let out = Command::new("git")
                .arg("-C")
                .arg(root.join(repo))
                .args(["-c", "user.name=t", "-c", "user.email=t@t"])
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        let commit = |repo: &str, subject: &str| {
            git(repo, &["commit", "-q", "--allow-empty", "-m", subject]);
        };
        for repo in ["api", "docs", "tools", "web"] {
            std::fs::create_dir_all(root.join(repo)).unwrap();
            git(repo, &["init", "-q", "-b", "main"]);
            commit(repo, "base");
        }
        // api: linked to a local branch, one commit each way.
        git("api", &["branch", "upstream"]);
        git("api", &["branch", "--set-upstream-to=upstream", "main"]);
        commit("api", "api outgoing");
        git("api", &["checkout", "-q", "upstream"]);
        commit("api", "api incoming");
        git("api", &["checkout", "-q", "main"]);
        // web: `gitlab/main` is there and nothing links it; two ahead.
        git("web", &["update-ref", "refs/remotes/gitlab/main", "HEAD"]);
        commit("web", "web older");
        commit("web", "web newer");
        // tools: detached.
        git("tools", &["checkout", "-q", "--detach"]);

        let g = sample(&root);
        assert_eq!(g.repos, ["api", "docs", "tools", "web"]);
        assert!(g.branch.is_empty() && g.upstream.is_none(), "a folder has no branch");
        let by = |name: &str| g.nested.iter().find(|s| s.name == name).unwrap().clone();
        let api = by("api");
        assert_eq!((api.upstream.as_deref(), api.by_name), (Some("upstream"), false));
        assert_eq!((api.ahead, api.behind), (1, 1));
        assert_eq!(api.to_push.unwrap()[0].subject, "api outgoing");
        assert_eq!(api.to_pull.unwrap()[0].subject, "api incoming");
        let web = by("web");
        assert_eq!((web.upstream.as_deref(), web.by_name), (Some("gitlab/main"), true));
        assert_eq!((web.ahead, web.behind), (2, 0));
        let subjects: Vec<_> = web.to_push.unwrap().into_iter().map(|c| c.subject).collect();
        assert_eq!(subjects, ["web newer", "web older"]);
        assert_eq!(web.to_pull, Some(Vec::new()));
        let docs = by("docs");
        assert!(docs.upstream.is_none() && docs.to_push.is_none(), "no remote, no comparison");
        assert_eq!(docs.fetched_at_ms, 0, "never fetched");
        std::fs::write(root.join("docs/.git/FETCH_HEAD"), "").unwrap();
        let docs = sample(&root).nested.into_iter().find(|s| s.name == "docs").unwrap();
        assert!(docs.fetched_at_ms > 0, "a fetch leaves FETCH_HEAD behind");
        let tools = by("tools");
        assert!(tools.detached && tools.upstream.is_none());
        assert_eq!(g.nested_ahead_behind(), (3, 1));

        // Two remotes carrying the name is a guess the fallback refuses.
        git("web", &["update-ref", "refs/remotes/origin/main", "HEAD"]);
        let web = sample(&root).nested.into_iter().find(|s| s.name == "web").unwrap();
        assert!(web.upstream.is_none(), "{web:?}");

        // A commit is read in the repo its list named, and only a census
        // name routes anywhere.
        assert_eq!(commit_dir(&root, Some("api")).unwrap(), root.join("api"));
        assert!(commit_dir(&root, Some("..")).is_err());
        assert!(commit_dir(&root, Some("api/../web")).is_err());
        assert!(commit_dir(&root, Some("missing")).is_err());
        assert_eq!(commit_dir(&root, None).unwrap(), root);

        // A folder of ONE: the lists are that repo's, and so is the road a
        // listed commit is opened on — the root holds no repository.
        for repo in ["docs", "tools", "web"] {
            std::fs::remove_dir_all(root.join(repo)).unwrap();
        }
        let g = sample(&root);
        assert!(g.nested.is_empty(), "one repo is not a workspace to the header");
        assert_eq!(g.to_push.unwrap()[0].subject, "api outgoing");
        assert_eq!(commit_dir(&root, None).unwrap(), root.join("api"));

        // One nested repo under a repository root stays the root's view.
        git("", &["init", "-q", "-b", "main"]);
        assert!(sample(&root).nested.is_empty());
        assert_eq!(commit_dir(&root, None).unwrap(), root);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A fetch press over a workspace (T-455), against bare repositories on
    /// disk: each repo reaches its own remote — the linked one, the one
    /// matched by name — a broken remote fails only its own repo, a local
    /// upstream is not fetched at all, and no `FETCH_HEAD` is written.
    #[test]
    fn a_fetch_pass_reaches_each_repo_by_its_own_remote() {
        if !crate::worktree::have_git() {
            return;
        }
        let base =
            std::env::temp_dir().join(format!("msmn-gitstatus-fetch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("ws");
        let git = |dir: &Path, args: &[&str]| -> String {
            std::fs::create_dir_all(dir).unwrap();
            let out = Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(["-c", "user.name=t", "-c", "user.email=t@t"])
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        let bare = |name: &str| {
            let dir = base.join(format!("{name}.git"));
            git(&base, &["init", "-q", "--bare", "-b", "main", dir.to_str().unwrap()]);
            dir
        };
        for repo in ["api", "local", "web", "broken"] {
            git(&root.join(repo), &["init", "-q", "-b", "main"]);
            git(&root.join(repo), &["commit", "-q", "--allow-empty", "-m", "base"]);
        }
        // api: linked the usual way. web: pushed without `-u`, never linked.
        let api_remote = bare("api");
        git(&root.join("api"), &["remote", "add", "origin", api_remote.to_str().unwrap()]);
        git(&root.join("api"), &["push", "-q", "-u", "origin", "main"]);
        let web_remote = bare("web");
        git(&root.join("web"), &["remote", "add", "gitlab", web_remote.to_str().unwrap()]);
        git(&root.join("web"), &["push", "-q", "gitlab", "main"]);
        // broken: linked to a remote that is not there.
        git(&root.join("broken"), &["remote", "add", "origin", "/nonexistent/msmn-remote.git"]);
        git(&root.join("broken"), &["config", "branch.main.remote", "origin"]);
        git(&root.join("broken"), &["config", "branch.main.merge", "refs/heads/main"]);
        // local: its upstream is a local branch — nothing over the network.
        git(&root.join("local"), &["branch", "upstream"]);
        git(&root.join("local"), &["branch", "-q", "--set-upstream-to=upstream", "main"]);
        // Somebody else pushes to both remotes.
        for (remote, name) in [(&api_remote, "api-other"), (&web_remote, "web-other")] {
            let other = base.join(name);
            git(&base, &["clone", "-q", remote.to_str().unwrap(), other.to_str().unwrap()]);
            git(&other, &["commit", "-q", "--allow-empty", "-m", "from elsewhere"]);
            git(&other, &["push", "-q", "origin", "main"]);
        }

        let sync = |name: &str, upstream: &str, by_name: bool| RepoSync {
            name: name.into(),
            branch: "main".into(),
            upstream: Some(upstream.into()),
            by_name,
            ..RepoSync::default()
        };
        let nested = [
            sync("api", "origin/main", false),
            sync("broken", "origin/main", false),
            sync("local", "upstream", false),
            sync("web", "gitlab/main", true),
        ];
        assert_eq!(nested_remote(&root.join("web"), &nested[3]).as_deref(), Some("gitlab"));
        assert_eq!(nested_remote(&root.join("local"), &nested[2]), None);
        let detached = RepoSync { detached: true, ..nested[0].clone() };
        assert_eq!(nested_remote(&root.join("api"), &detached), None);

        let mut got = fetch_pass(&root, None, &nested);
        got.nested.sort_by(|a, b| a.0.cmp(&b.0));
        assert!(got.root.is_none());
        let names: Vec<_> = got.nested.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["api", "broken", "web"], "a local upstream is not fetched");
        assert!(got.nested[0].1.is_ok() && got.nested[2].1.is_ok(), "{got:?}");
        assert!(got.nested[1].1.is_err(), "{got:?}");
        for (repo, tracking) in [("api", "origin/main"), ("web", "gitlab/main")] {
            let subject = git(&root.join(repo), &["log", "-1", "--format=%s", tracking]);
            assert_eq!(subject, "from elsewhere", "{repo}'s remote-tracking ref moved");
            assert!(!root.join(repo).join(".git/FETCH_HEAD").exists(), "no FETCH_HEAD in {repo}");
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_cadence_is_minutes_and_off_by_default() {
        assert_eq!(fetch_every(None), Duration::ZERO);
        assert_eq!(fetch_every(Some("5")), Duration::from_secs(300));
        assert_eq!(fetch_every(Some(" 1 ")), Duration::from_secs(60));
        assert_eq!(fetch_every(Some("x")), Duration::ZERO);
        assert_eq!(fetch_every(Some("0")), Duration::ZERO);
    }
}
