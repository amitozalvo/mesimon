//! Where the board's own checkout stands: branch, ahead/behind its upstream,
//! uncommitted changes (T-124).
//!
//! This is the REPO's state, not a ticket's worktree (`crate::worktree` owns
//! those). One fork per sample — `git status --porcelain=v2 --branch -z`
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

use mesimon_core::command::RepoGit;

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
        return g;
    }
    for name in &repos {
        g.changed += sample_one(&root.join(name)).changed;
    }
    g.sampled = true;
    g.repos = repos;
    g
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
        let _ = std::fs::remove_dir_all(&dir);
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
