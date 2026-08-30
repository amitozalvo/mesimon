//! Ticket worktrees (M4, doc 12 subset): provisioning stages 0/1/1b/2/5, the
//! ownership marker, locking + crash-safe sweep, the merge story, the teardown
//! transaction, and the duplicate-branch scan.
//!
//! Every git call is an argv array — never a shell string (a ticket titled
//! `fix; rm -rf ~` is command injection otherwise). Read-path calls carry
//! `--no-optional-locks`. mesimon never writes the shared `.git/config`
//! (12 §12.2.3) and never registers Claude Code's WorktreeCreate hook.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths::Paths;

/// Local-only binding record (12 §12.10.1: path/status/oids never travel with
/// the ticket). Persisted as `worktrees.json` in the state dir.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Binding {
    pub path: PathBuf,
    pub branch: String,
    pub base_oid: String,
    pub branch_oid: String,
    pub status: BindingStatus,
    /// A `git worktree lock` we hold while sessions live in the tree.
    #[serde(default)]
    pub locked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BindingStatus {
    /// Waiting for a provisioning slot ("waiting to provision", never a spinner).
    Queued,
    Provisioning,
    Attached,
    /// Directory removed, branch kept — diffs still render from the object store.
    Evicted,
    /// Fail closed (D26); `stage` names what failed, in mesimon's words.
    Error { stage: String, message: String },
}

pub type Bindings = HashMap<ulid::Ulid, Binding>;

pub fn bindings_file(paths: &Paths) -> PathBuf {
    paths.state_dir.join("worktrees.json")
}

pub fn load_bindings(paths: &Paths) -> Result<Bindings> {
    let f = bindings_file(paths);
    if !f.is_file() {
        return Ok(Bindings::new());
    }
    serde_json::from_str(&std::fs::read_to_string(&f)?)
        .with_context(|| format!("parse {}", f.display()))
}

pub fn save_bindings(paths: &Paths, b: &Bindings) -> Result<()> {
    let f = bindings_file(paths);
    let tmp = f.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(b)?)?;
    std::fs::rename(&tmp, f)?;
    Ok(())
}

/// `~/.local/state/mesimon/<proj16>/worktrees/` — created once, 0700, spotlight
/// and Time Machine excluded (best effort).
pub fn ensure_root(paths: &Paths) -> Result<PathBuf> {
    let root = paths.state_dir.join("worktrees");
    if !root.is_dir() {
        std::fs::create_dir_all(&root)?;
        let perm = std::os::unix::fs::PermissionsExt::from_mode(0o700);
        std::fs::set_permissions(&root, perm)?;
        let _ = std::fs::write(root.join(".metadata_never_index"), "");
        // Time Machine exclusion, detached: tmutil can stall for 10+ s (TCC),
        // and ensure_root runs on the daemon's writer thread.
        #[cfg(target_os = "macos")]
        {
            let r = root.clone();
            std::thread::spawn(move || {
                let _ = Command::new("tmutil").args(["addexclusion"]).arg(&r).output();
            });
        }
    }
    Ok(root)
}

/// Run git with argv, capture stdout; non-zero exit becomes an error carrying
/// stderr (callers rewrap into mesimon-voiced messages before the UI).
fn git(repo: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .context("run git")?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.first().copied().unwrap_or(""),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn git_read(repo: &Path, args: &[&str]) -> Result<String> {
    let mut v = vec!["--no-optional-locks"];
    v.extend_from_slice(args);
    git(repo, &v)
}

pub fn have_git() -> bool {
    Command::new("git").arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

/// origin/HEAD → {main, master, trunk} → current HEAD name.
pub fn default_branch(repo: &Path) -> Result<String> {
    if let Ok(s) = git_read(repo, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"]) {
        if let Some(b) = s.trim().strip_prefix("origin/") {
            return Ok(b.to_string());
        }
    }
    for cand in ["main", "master", "trunk"] {
        let refname = format!("refs/heads/{cand}");
        if git_read(repo, &["rev-parse", "--verify", "--quiet", &refname]).is_ok() {
            return Ok(cand.to_string());
        }
    }
    Ok(git_read(repo, &["rev-parse", "--abbrev-ref", "HEAD"])?.trim().to_string())
}

/// Provisioning stages 0/1/1b/2/5. Runs OFF the writer thread (add is ~1.8 s of
/// filesystem work). Returns the ready binding, or the failing stage.
pub fn provision(
    repo: &Path,
    root: &Path,
    ticket_id: ulid::Ulid,
    short_key: &str,
    title: &str,
) -> std::result::Result<Binding, (String, String)> {
    let branch = mesimon_core::workspace::branch_name(short_key, title);
    let dir = root.join(mesimon_core::workspace::dir_name(short_key, title));

    // Stage 0 — precheck.
    let base = default_branch(repo).map_err(|e| ("precheck".into(), e.to_string()))?;
    let base_ref = format!("refs/heads/{base}");
    let base_oid = git_read(repo, &["rev-parse", "--verify", "--quiet", &base_ref])
        .map_err(|_| ("precheck".into(), format!("base branch {base} has no commits")))?
        .trim()
        .to_string();
    // Branch collision: refuse in mesimon's words, never raw git text.
    let branch_ref = format!("refs/heads/{branch}");
    if git_read(repo, &["rev-parse", "--verify", "--quiet", &branch_ref]).is_ok() {
        return Err(("precheck".into(), format!("branch {branch} already exists")));
    }
    if dir.exists() {
        return Err(("precheck".into(), format!("{} already exists", dir.display())));
    }

    // Stage 1 — add. Never --force, never --relative-paths.
    let dir_s = dir.to_string_lossy().into_owned();
    git(repo, &["worktree", "add", "--quiet", &dir_s, "-b", &branch, &base_oid])
        .map_err(|e| ("add".into(), e.to_string()))?;

    // Stage 1b — ownership marker inside the admin dir (survives clean -xfd,
    // dies with worktree remove/prune, needs no shared-config write).
    let marker = git_read(&dir, &["rev-parse", "--git-path", "mesimon-ticket"])
        .map_err(|e| ("mark".into(), e.to_string()))?;
    let marker_path = {
        let p = PathBuf::from(marker.trim());
        if p.is_absolute() { p } else { dir.join(p) }
    };
    std::fs::write(&marker_path, format!("{ticket_id}\n{}\n1\n", repo.display()))
        .map_err(|e| ("mark".into(), e.to_string()))?;

    // Stage 2 — .worktreeinclude copy (Claude Code semantics: pattern match AND
    // gitignored). Skip silently when the repo has no file.
    if let Err(e) = copy_worktreeinclude(repo, &dir) {
        return Err(("include".into(), e.to_string()));
    }

    // Stage 5 — ready. Canonicalize so the path matches `worktree list` output
    // (macOS /var → /private/var); the abs path is the canonical key (12 §12.2).
    let dir = dir.canonicalize().unwrap_or(dir);
    Ok(Binding {
        path: dir,
        branch_oid: base_oid.clone(),
        base_oid,
        branch,
        status: BindingStatus::Attached,
        locked: false,
    })
}

/// Recreate an evicted worktree: the branch already exists, so `add` without
/// `-b`, then re-mark and re-include. Commits made on the branch survive
/// eviction — this restores the working directory only.
pub fn provision_existing(
    repo: &Path,
    ticket_id: ulid::Ulid,
    prior: &Binding,
) -> std::result::Result<Binding, (String, String)> {
    let branch_ref = format!("refs/heads/{}", prior.branch);
    if git_read(repo, &["rev-parse", "--verify", "--quiet", &branch_ref]).is_err() {
        return Err(("precheck".into(), format!("branch {} no longer exists", prior.branch)));
    }
    if prior.path.exists() {
        return Err(("precheck".into(), format!("{} already exists", prior.path.display())));
    }
    let dir_s = prior.path.to_string_lossy().into_owned();
    git(repo, &["worktree", "add", "--quiet", &dir_s, &prior.branch])
        .map_err(|e| ("add".into(), e.to_string()))?;
    let marker = git_read(&prior.path, &["rev-parse", "--git-path", "mesimon-ticket"])
        .map_err(|e| ("mark".into(), e.to_string()))?;
    let mp = PathBuf::from(marker.trim());
    let mp = if mp.is_absolute() { mp } else { prior.path.join(mp) };
    std::fs::write(mp, format!("{ticket_id}\n{}\n1\n", repo.display()))
        .map_err(|e| ("mark".into(), e.to_string()))?;
    if let Err(e) = copy_worktreeinclude(repo, &prior.path) {
        return Err(("include".into(), e.to_string()));
    }
    let branch_oid = git_read(repo, &["rev-parse", "--verify", "--quiet", &branch_ref])
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let mut b = prior.clone();
    b.path = b.path.canonicalize().unwrap_or(b.path);
    b.branch_oid = branch_oid;
    b.status = BindingStatus::Attached;
    b.locked = false;
    Ok(b)
}

/// Copy every gitignored file that matches a `.worktreeinclude` pattern into the
/// new worktree, preserving relative paths.
fn copy_worktreeinclude(repo: &Path, dest: &Path) -> Result<()> {
    let inc = repo.join(".worktreeinclude");
    let Ok(patterns) = std::fs::read_to_string(&inc) else { return Ok(()) };
    let patterns: Vec<&str> = patterns
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    if patterns.is_empty() {
        return Ok(());
    }
    // The gitignored universe (files only, NUL-safe).
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["--no-optional-locks", "ls-files", "--others", "--ignored", "--exclude-standard", "-z"])
        .output()?;
    if !out.status.success() {
        bail!("ls-files failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    for raw in out.stdout.split(|b| *b == 0) {
        if raw.is_empty() {
            continue;
        }
        let rel = String::from_utf8_lossy(raw).into_owned();
        if patterns.iter().any(|p| pattern_matches(p, &rel)) {
            let from = repo.join(&rel);
            let to = dest.join(&rel);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&from, &to)
                .with_context(|| format!("copy {rel}"))?;
        }
    }
    Ok(())
}

/// Minimal gitignore-style matcher for `.worktreeinclude` lines: literal
/// segments, `*` within a segment, `**` spanning segments, trailing `/` matches
/// the whole subtree. Enough for `.env`, `dir/`, `*.local`, `**/secret.json`.
fn pattern_matches(pattern: &str, path: &str) -> bool {
    let pattern = pattern.trim_start_matches('/');
    if let Some(dir) = pattern.strip_suffix('/') {
        return path.starts_with(&format!("{dir}/")) || segments_match(dir, path_parent_all(path));
    }
    segments_match(pattern, path)
        // A bare name also matches at any depth when the pattern has no '/'
        // (gitignore rule for unanchored patterns).
        || (!pattern.contains('/')
            && path.rsplit('/').next().map(|base| glob_seg(pattern, base)).unwrap_or(false))
}

fn path_parent_all(path: &str) -> &str {
    path
}

fn segments_match(pattern: &str, path: &str) -> bool {
    fn rec(pat: &[&str], path: &[&str]) -> bool {
        match (pat.first(), path.first()) {
            (None, None) => true,
            (Some(&"**"), _) => {
                rec(&pat[1..], path) || (!path.is_empty() && rec(pat, &path[1..]))
            }
            (Some(p), Some(s)) => glob_seg(p, s) && rec(&pat[1..], &path[1..]),
            _ => false,
        }
    }
    let pat: Vec<&str> = pattern.split('/').collect();
    let segs: Vec<&str> = path.split('/').collect();
    rec(&pat, &segs)
}

/// `*` and `?` within one path segment.
fn glob_seg(pat: &str, s: &str) -> bool {
    fn rec(p: &[u8], s: &[u8]) -> bool {
        match (p.first(), s.first()) {
            (None, None) => true,
            (Some(b'*'), _) => rec(&p[1..], s) || (!s.is_empty() && rec(p, &s[1..])),
            (Some(b'?'), Some(_)) => rec(&p[1..], &s[1..]),
            (Some(a), Some(b)) if a == b => rec(&p[1..], &s[1..]),
            _ => false,
        }
    }
    rec(pat.as_bytes(), s.as_bytes())
}

// ---- locking ----------------------------------------------------------------

pub fn lock(repo: &Path, wt: &Path, key: &str, session: uuid::Uuid, pid: u32) -> Result<()> {
    let reason = format!("mesimon: ticket {key} session {session} pid {pid}");
    let wt_s = wt.to_string_lossy().into_owned();
    git(repo, &["worktree", "lock", "--reason", &reason, &wt_s]).map(|_| ())
}

pub fn unlock(repo: &Path, wt: &Path) -> Result<()> {
    let wt_s = wt.to_string_lossy().into_owned();
    match git(repo, &["worktree", "unlock", &wt_s]) {
        Ok(_) => Ok(()),
        // "not locked" is fine — unlock is idempotent in the teardown order.
        Err(e) if e.to_string().contains("not locked") => Ok(()),
        Err(e) => Err(e),
    }
}

/// One parsed row of `git worktree list --porcelain -z`.
#[derive(Debug, Default, Clone)]
pub struct WtRow {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub locked_reason: Option<String>,
    pub prunable: bool,
}

pub fn list_worktrees(repo: &Path) -> Result<Vec<WtRow>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["--no-optional-locks", "worktree", "list", "--porcelain", "-z"])
        .output()
        .context("worktree list")?;
    if !out.status.success() {
        bail!("worktree list failed");
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut rows = Vec::new();
    let mut cur: Option<WtRow> = None;
    for rec in text.split('\0') {
        if rec.is_empty() {
            // Blank record = row separator in -z output.
            if let Some(r) = cur.take() {
                rows.push(r);
            }
            continue;
        }
        if let Some(p) = rec.strip_prefix("worktree ") {
            if let Some(r) = cur.take() {
                rows.push(r);
            }
            cur = Some(WtRow { path: PathBuf::from(p), ..Default::default() });
        } else if let Some(b) = rec.strip_prefix("branch refs/heads/") {
            if let Some(r) = cur.as_mut() {
                r.branch = Some(b.to_string());
            }
        } else if let Some(reason) = rec.strip_prefix("locked") {
            if let Some(r) = cur.as_mut() {
                r.locked_reason = Some(reason.trim().to_string());
            }
        } else if rec.starts_with("prunable") {
            if let Some(r) = cur.as_mut() {
                r.prunable = true;
            }
        }
    }
    if let Some(r) = cur.take() {
        rows.push(r);
    }
    Ok(rows)
}

/// Crash-safe sweep at daemon start: unlock every worktree whose lock reason is
/// ours (`mesimon: `) AND whose embedded pid is dead. Never touch a reason
/// mesimon does not own.
pub fn sweep_stale_locks(repo: &Path) -> Result<()> {
    for row in list_worktrees(repo)? {
        let Some(reason) = &row.locked_reason else { continue };
        if !reason.starts_with("mesimon: ") {
            continue;
        }
        let pid = reason
            .rsplit(' ')
            .next()
            .and_then(|p| p.parse::<i32>().ok());
        let dead = pid.map(|p| unsafe { libc::kill(p, 0) } != 0).unwrap_or(true);
        if dead {
            let _ = unlock(repo, &row.path);
        }
    }
    Ok(())
}

/// Duplicate-branch scan (12 §12.6.7): two worktrees on one branch silently
/// delete each other's commits. Returns the offending branch names.
pub fn branch_conflicts(rows: &[WtRow]) -> Vec<String> {
    let mut seen: HashMap<&str, u32> = HashMap::new();
    for r in rows {
        if let Some(b) = &r.branch {
            *seen.entry(b.as_str()).or_default() += 1;
        }
    }
    seen.into_iter().filter(|(_, n)| *n > 1).map(|(b, _)| b.to_string()).collect()
}

// ---- merge story ------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub enum MergeCheck {
    /// Branch tip already an ancestor of the default branch.
    AlreadyMerged,
    Clean,
    Conflicts,
}

/// Current tip of a branch (empty when the ref is gone).
pub fn branch_tip(repo: &Path, branch: &str) -> String {
    let refname = format!("refs/heads/{branch}");
    git_read(repo, &["rev-parse", "--verify", "--quiet", &refname])
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// Commits on the branch not yet in base — "merge available" when > 0.
pub fn ahead_count(repo: &Path, branch: &str, base: &str) -> u32 {
    let range = format!("{base}..{branch}");
    git_read(repo, &["rev-list", "--count", &range])
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

pub fn is_merged(repo: &Path, branch: &str, base: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["--no-optional-locks", "merge-base", "--is-ancestor", branch, base])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// In-memory preflight (`merge-tree --write-tree`): touches nothing.
pub fn merge_check(repo: &Path, branch: &str, base: &str) -> Result<MergeCheck> {
    if is_merged(repo, branch, base) {
        return Ok(MergeCheck::AlreadyMerged);
    }
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["--no-optional-locks", "merge-tree", "--write-tree", base, branch])
        .output()
        .context("merge-tree")?;
    match out.status.code() {
        Some(0) => Ok(MergeCheck::Clean),
        Some(1) => Ok(MergeCheck::Conflicts),
        _ => bail!("merge preflight failed: {}", String::from_utf8_lossy(&out.stderr).trim()),
    }
}

/// Perform the clean merge of `branch` into `base`.
///
/// - main checkout HAS `base` checked out: refuse if the working tree is dirty
///   in paths the merge touches, else `git merge --no-edit branch`.
/// - main checkout is elsewhere: fast-forward the ref without a checkout
///   (`git push . branch:refs/heads/base`, ff-only); a non-ff merge with base
///   not checked out is refused with the reason.
pub fn merge_into_base(repo: &Path, branch: &str, base: &str) -> Result<()> {
    let head = git_read(repo, &["rev-parse", "--abbrev-ref", "HEAD"])?.trim().to_string();
    if head == base {
        // Overlap check: merge must not eat user working-tree state.
        let touched = git_read(
            repo,
            &["diff", "--name-only", "-z", &format!("{base}...{branch}")],
        )?;
        let dirty = git_read(repo, &["status", "--porcelain=v2", "-z", "-uno"])?;
        let touched: Vec<&str> = touched.split('\0').filter(|s| !s.is_empty()).collect();
        for line in dirty.split('\0').filter(|s| !s.is_empty()) {
            // porcelain v2: "1 <XY> ... <path>" — path is the last field.
            if let Some(path) = line.split(' ').next_back() {
                if touched.contains(&path) {
                    bail!("main checkout has uncommitted changes in {path} — commit or stash first");
                }
            }
        }
        git(repo, &["merge", "--no-edit", branch]).map(|_| ())
    } else {
        // ff-only ref update, no checkout touched.
        let refspec = format!("{branch}:refs/heads/{base}");
        git(repo, &["push", "--quiet", ".", &refspec]).map_err(|_| {
            anyhow!("{base} is not checked out and the merge is not fast-forward — check out {base} first")
        }).map(|_| ())
    }
}

// ---- teardown ---------------------------------------------------------------

/// Step 3 of the teardown order: OUR audit, never git's inverted refusal.
/// Returns (modified+untracked count, human summary) — empty = clean.
pub fn audit(wt: &Path) -> Result<(usize, String)> {
    let out = git_read(wt, &["status", "--porcelain=v2", "-unormal", "-z"])?;
    let mut modified = 0usize;
    let mut untracked = 0usize;
    for line in out.split('\0').filter(|s| !s.is_empty()) {
        match line.as_bytes().first() {
            Some(b'?') => untracked += 1,
            Some(b'1') | Some(b'2') | Some(b'u') => modified += 1,
            _ => {}
        }
    }
    let n = modified + untracked;
    let summary = format!("{modified} modified, {untracked} untracked will be lost");
    Ok((n, summary))
}

/// Steps 1/2/4 of the teardown order (callers kill sessions first — step 0):
/// unlock → remove --force (single force defeats only the stale-lock refusal;
/// NEVER -f -f) → fallback rm -rf + prune.
pub fn remove(repo: &Path, wt: &Path) -> Result<()> {
    let _ = unlock(repo, wt);
    let wt_s = wt.to_string_lossy().into_owned();
    if git(repo, &["worktree", "remove", "--force", &wt_s]).is_err() {
        std::fs::remove_dir_all(wt).ok();
        git(repo, &["worktree", "prune"])?;
    }
    Ok(())
}

/// Step 5: `-d` (if-merged); the `-D` escalation is a separate explicit call.
pub fn delete_branch(repo: &Path, branch: &str, force: bool) -> Result<()> {
    let flag = if force { "-D" } else { "-d" };
    git(repo, &["branch", flag, branch]).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_repo(name: &str) -> Option<PathBuf> {
        if !have_git() {
            return None;
        }
        let dir = std::env::temp_dir().join(format!("msmn-wt-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| {
            let ok = Command::new("git").arg("-C").arg(&dir).args(args).output().unwrap();
            assert!(ok.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&ok.stderr));
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(dir.join("a.txt"), "hello\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-qm", "init"]);
        Some(dir)
    }

    #[test]
    fn provision_merge_teardown_roundtrip() {
        let Some(repo) = scratch_repo("round") else { return };
        let root = repo.join("_wtroot");
        std::fs::create_dir_all(&root).unwrap();

        let b = provision(&repo, &root, ulid::Ulid(1), "T-1", "Fix thing").unwrap();
        assert_eq!(b.branch, "msmn/T-1-fix-thing");
        assert!(b.path.join("a.txt").is_file());
        assert_eq!(b.status, BindingStatus::Attached);

        // Marker written and readable.
        let marker = git_read(&b.path, &["rev-parse", "--git-path", "mesimon-ticket"]).unwrap();
        let mp = PathBuf::from(marker.trim());
        let mp = if mp.is_absolute() { mp } else { b.path.join(mp) };
        assert!(std::fs::read_to_string(mp).unwrap().starts_with(&ulid::Ulid(1).to_string()));

        // Same branch again refuses at precheck.
        let e = provision(&repo, &root, ulid::Ulid(2), "T-1", "Fix thing").unwrap_err();
        assert_eq!(e.0, "precheck");

        // Commit in the worktree → merge check clean → merge into main.
        std::fs::write(b.path.join("b.txt"), "wt\n").unwrap();
        let run_wt = |args: &[&str]| {
            let ok = Command::new("git").arg("-C").arg(&b.path).args(args).output().unwrap();
            assert!(ok.status.success(), "{}", String::from_utf8_lossy(&ok.stderr));
        };
        run_wt(&["add", "."]);
        run_wt(&["commit", "-qm", "wt work"]);
        assert!(!is_merged(&repo, &b.branch, "main"));
        assert_eq!(merge_check(&repo, &b.branch, "main").unwrap(), MergeCheck::Clean);
        merge_into_base(&repo, &b.branch, "main").unwrap();
        assert!(is_merged(&repo, &b.branch, "main"));
        assert!(repo.join("b.txt").is_file());

        // Clean teardown: audit clean → remove → branch -d succeeds (merged).
        let (n, _) = audit(&b.path).unwrap();
        assert_eq!(n, 0);
        remove(&repo, &b.path).unwrap();
        assert!(!b.path.exists());
        delete_branch(&repo, &b.branch, false).unwrap();

        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn conflict_detected_and_dirty_audit_counts() {
        let Some(repo) = scratch_repo("conf") else { return };
        let root = repo.join("_wtroot");
        std::fs::create_dir_all(&root).unwrap();
        let b = provision(&repo, &root, ulid::Ulid(3), "T-3", "clash").unwrap();

        // Diverge: same file changed on main and on the branch.
        std::fs::write(repo.join("a.txt"), "main side\n").unwrap();
        let run = |d: &Path, args: &[&str]| {
            let ok = Command::new("git").arg("-C").arg(d).args(args).output().unwrap();
            assert!(ok.status.success(), "{}", String::from_utf8_lossy(&ok.stderr));
        };
        run(&repo, &["commit", "-aqm", "main change"]);
        std::fs::write(b.path.join("a.txt"), "branch side\n").unwrap();
        run(&b.path, &["commit", "-aqm", "branch change"]);

        assert_eq!(merge_check(&repo, &b.branch, "main").unwrap(), MergeCheck::Conflicts);

        // Dirty worktree audit counts the stray file.
        std::fs::write(b.path.join("stray.txt"), "x").unwrap();
        let (n, summary) = audit(&b.path).unwrap();
        assert_eq!(n, 1, "{summary}");

        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn lock_sweep_only_touches_dead_mesimon_locks() {
        let Some(repo) = scratch_repo("lock") else { return };
        let root = repo.join("_wtroot");
        std::fs::create_dir_all(&root).unwrap();
        let b = provision(&repo, &root, ulid::Ulid(4), "T-4", "locked").unwrap();

        // Foreign lock reason survives the sweep.
        let wt_s = b.path.to_string_lossy().into_owned();
        git(&repo, &["worktree", "lock", "--reason", "user: keep out", &wt_s]).unwrap();
        sweep_stale_locks(&repo).unwrap();
        let rows = list_worktrees(&repo).unwrap();
        let row = rows.iter().find(|r| r.path == b.path).unwrap();
        assert!(row.locked_reason.is_some(), "foreign lock must survive");
        unlock(&repo, &b.path).unwrap();

        // Dead-pid mesimon lock is swept.
        lock(&repo, &b.path, "T-4", uuid::Uuid::nil(), 9_999_999).unwrap();
        sweep_stale_locks(&repo).unwrap();
        let rows = list_worktrees(&repo).unwrap();
        let row = rows.iter().find(|r| r.path == b.path).unwrap();
        assert!(row.locked_reason.is_none(), "dead mesimon lock must sweep");

        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn duplicate_branch_scan() {
        let rows = vec![
            WtRow { path: "/a".into(), branch: Some("msmn/T-1-x".into()), ..Default::default() },
            WtRow { path: "/b".into(), branch: Some("msmn/T-1-x".into()), ..Default::default() },
            WtRow { path: "/c".into(), branch: Some("msmn/T-2-y".into()), ..Default::default() },
        ];
        let dup = branch_conflicts(&rows);
        assert_eq!(dup, vec!["msmn/T-1-x".to_string()]);
    }

    #[test]
    fn worktreeinclude_patterns() {
        assert!(pattern_matches(".env", ".env"));
        assert!(pattern_matches(".env", "sub/.env")); // unanchored bare name
        assert!(pattern_matches("*.local", "conf.local"));
        assert!(pattern_matches("**/secret.json", "a/b/secret.json"));
        assert!(pattern_matches("vendor/", "vendor/pkg/x.go"));
        assert!(!pattern_matches("*.local", "conf.remote"));
        assert!(!pattern_matches("vendor/", "notvendor/x"));
    }

    #[test]
    fn include_copies_matching_ignored_files() {
        let Some(repo) = scratch_repo("inc") else { return };
        std::fs::write(repo.join(".gitignore"), ".env\nnode_modules/\n").unwrap();
        std::fs::write(repo.join(".worktreeinclude"), ".env\n").unwrap();
        std::fs::write(repo.join(".env"), "SECRET=1\n").unwrap();
        std::fs::create_dir_all(repo.join("node_modules")).unwrap();
        std::fs::write(repo.join("node_modules/big.js"), "x").unwrap();
        let run = |args: &[&str]| {
            let ok = Command::new("git").arg("-C").arg(&repo).args(args).output().unwrap();
            assert!(ok.status.success());
        };
        run(&["add", ".gitignore", ".worktreeinclude"]);
        run(&["commit", "-qm", "ignore"]);

        let root = repo.join("_wtroot");
        std::fs::create_dir_all(&root).unwrap();
        let b = provision(&repo, &root, ulid::Ulid(5), "T-5", "env").unwrap();
        assert_eq!(std::fs::read_to_string(b.path.join(".env")).unwrap(), "SECRET=1\n");
        // Not in .worktreeinclude → not copied.
        assert!(!b.path.join("node_modules/big.js").exists());

        std::fs::remove_dir_all(&repo).ok();
    }
}
