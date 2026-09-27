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
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use mesimon_core::command::Notice;
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
    /// The legs of a WORKSPACE binding (T-368): one entry per repository the
    /// ticket's branch lives in, the root (meta) leg named `""` first when
    /// the root is a repository. Empty is a single-repo binding — today's
    /// shape, its story told by the top-level fields — and is left off the
    /// wire, so a single-repo `worktrees.json` reads as it always did.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repos: Vec<RepoBinding>,
}

/// One leg of a workspace binding as persisted: which nested repo, and the
/// branch its checkout was on when the leg was cut — that leg's base.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoBinding {
    /// `""` is the root repository; otherwise the census name (`api`).
    pub name: String,
    pub base: String,
    pub base_oid: String,
    pub branch_oid: String,
}

/// A leg resolved against the checkout: where git runs (`repo`) and where
/// the ticket's tree for that repo is (`path`). A single-repo binding is
/// exactly one root leg, so every consumer iterates legs and never asks
/// whether a binding is a workspace one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Leg {
    /// `""` is the root.
    pub name: String,
    pub repo: PathBuf,
    pub path: PathBuf,
    pub base: String,
    pub base_oid: String,
    pub branch_oid: String,
}

impl Binding {
    pub fn is_workspace(&self) -> bool {
        !self.repos.is_empty()
    }

    /// Whether a root leg exists: a single-repo binding always has one; a
    /// workspace binding on a FOLDER root (no repository there) has none,
    /// and its container is a plain directory.
    pub fn has_root_leg(&self) -> bool {
        !self.is_workspace() || self.repos.iter().any(|r| r.name.is_empty())
    }

    /// Root leg first, then the children in `repos` order. A single-repo
    /// binding's base is not on the binding — it is the daemon's
    /// `base_branch`, re-resolved after every fetch — so the caller passes it.
    pub fn legs(&self, repo_root: &Path, single_base: &str) -> Vec<Leg> {
        if !self.is_workspace() {
            return vec![Leg {
                name: String::new(),
                repo: repo_root.to_path_buf(),
                path: self.path.clone(),
                base: single_base.to_string(),
                base_oid: self.base_oid.clone(),
                branch_oid: self.branch_oid.clone(),
            }];
        }
        self.repos
            .iter()
            .map(|r| Leg {
                name: r.name.clone(),
                repo: if r.name.is_empty() {
                    repo_root.to_path_buf()
                } else {
                    repo_root.join(&r.name)
                },
                path: if r.name.is_empty() { self.path.clone() } else { self.path.join(&r.name) },
                base: r.base.clone(),
                base_oid: r.base_oid.clone(),
                branch_oid: r.branch_oid.clone(),
            })
            .collect()
    }
}

/// A leg's word on the ticket page and in a detail: `root` for the root
/// leg, else its name.
pub fn leg_word(name: &str) -> &str {
    if name.is_empty() {
        "root"
    } else {
        name
    }
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
    Error {
        stage: String,
        message: String,
    },
}

pub type Bindings = HashMap<ulid::Ulid, Binding>;

/// On-disk schema stamp for `worktrees.json` (16 §6.2). Its own counter: a
/// bindings change must not force a sessions or ticket migration.
///
/// 2 (T-368): a binding may carry `repos`, the legs of a workspace worktree.
/// A schema-1 file reads through the default (no legs); a build that reads 1
/// bars its writes on a 2, because dropping the list would forget which
/// repositories hold the branch.
pub const BINDINGS_SCHEMA: u32 = 2;

/// `worktrees.json`. The legacy shape is a bare ULID-keyed map, which is why
/// `load_or_recover` probes for the `schema_version` key instead of reaching
/// for `#[serde(untagged)]`.
#[derive(Serialize, Deserialize)]
pub struct BindingsFile {
    pub schema_version: u32,
    pub bindings: Bindings,
}

pub fn bindings_file(paths: &Paths) -> PathBuf {
    paths.state_dir.join("worktrees.json")
}

/// Both on-disk shapes, from one parser: the versioned wrapper this build
/// writes, and the bare ULID-keyed map every existing user still has.
/// Shape-probed rather than `#[serde(untagged)]`, which would erase the line
/// and column that make a parse failure actionable. `"schema_version"` is 14
/// characters and can never be a 26-character ULID key, so this is exact.
///
/// `Err(Some(v))` is a file from a NEWER mesimon: valid bytes this build must
/// refuse rather than guess at. `Err(None)` is genuinely unparseable.
fn parse_bindings(text: &str) -> std::result::Result<Bindings, (Option<u32>, String)> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| (None, e.to_string()))?;
    if v.get("schema_version").is_none() {
        return serde_json::from_value::<Bindings>(v).map_err(|e| (None, e.to_string()));
    }
    let found = v.get("schema_version").and_then(|s| s.as_u64()).unwrap_or(1) as u32;
    if found > BINDINGS_SCHEMA {
        return Err((Some(found), format!("schema {found}")));
    }
    serde_json::from_value::<BindingsFile>(v)
        .map(|bf| bf.bindings)
        .map_err(|e| (None, e.to_string()))
}

/// Read-only load, safe to call off the writer thread (`serve_diff` does).
/// Never quarantines: only the writer thread renames, so the two can never
/// race on the same path.
pub fn load_bindings(paths: &Paths) -> Result<Bindings> {
    let f = bindings_file(paths);
    if !f.is_file() {
        return Ok(Bindings::new());
    }
    let text = std::fs::read_to_string(&f)?;
    parse_bindings(&text).map_err(|(_, detail)| anyhow::anyhow!("parse {}: {detail}", f.display()))
}

pub fn save_bindings(paths: &Paths, b: &Bindings) -> Result<()> {
    // One durability implementation for every file mesimon authors — this
    // used to be a second, fsync-less copy of store::write_atomic.
    let bf = BindingsFile { schema_version: BINDINGS_SCHEMA, bindings: b.clone() };
    crate::store::write_atomic(
        &bindings_file(paths),
        &serde_json::to_string_pretty(&bf)?,
        crate::store::PRIVATE,
    )
}

/// Rebuild bindings from what git and our own ownership markers still know.
///
/// `git worktree list` gives path + branch; the marker inside each admin dir
/// gives the ticket ULID (it is written at provision time and dies with
/// `worktree remove`). Only rows whose marker names a ticket are adopted — a
/// worktree that is not ours is never claimed (D26, fail closed).
///
/// This exists because losing `worktrees.json` used to orphan every worktree
/// and `msmn/*` branch with nothing to reconstruct from.
///
/// A workspace binding (T-368) is found through its children: every census
/// repo's `worktree list` is read for `msmn/` rows under `worktrees_root`
/// whose marker names a ticket, and those legs are grouped under the ticket
/// with the container as the parent of the leg's path. A root row the meta
/// scan found for the same ticket becomes the root leg.
pub fn rebuild_from_disk(repo: &Path, worktrees_root: &Path) -> Bindings {
    let mut out = Bindings::new();
    let rows = list_worktrees(repo).unwrap_or_default();
    let base = default_branch(repo).unwrap_or_else(|_| "main".into());
    for row in rows {
        let Some(branch) = row.branch.clone() else { continue }; // detached: not ours
        if !branch.starts_with(mesimon_core::workspace::BRANCH_NS) {
            continue;
        }
        let Some(ticket) = marker_ticket(&row.path) else { continue };
        let branch_oid = branch_tip(repo, &branch);
        // The merge base is what the diff viewer wants (BASE...BRANCH), and
        // it is recoverable exactly — unlike the original base_oid, which
        // only the lost file knew.
        let base_oid = git_read(repo, &["merge-base", &base, &branch])
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| branch_oid.clone());
        let status =
            if row.path.is_dir() { BindingStatus::Attached } else { BindingStatus::Evicted };
        out.insert(
            ticket,
            Binding {
                path: row.path.canonicalize().unwrap_or(row.path),
                branch,
                base_oid,
                branch_oid,
                status,
                locked: row.locked_reason.is_some(),
                repos: Vec::new(),
            },
        );
    }
    // The children of a workspace: a leg is a row on our namespace, under
    // the worktrees root, carrying our marker.
    let wt_root = worktrees_root.canonicalize().unwrap_or_else(|_| worktrees_root.to_path_buf());
    let root_found: Vec<ulid::Ulid> = out.keys().copied().collect();
    for name in crate::gitstatus::census(repo) {
        let child = repo.join(&name);
        let Ok(rows) = list_worktrees(&child) else { continue };
        let child_base = checked_out(&child)
            .map(|(b, _)| b)
            .or_else(|_| default_branch(&child))
            .unwrap_or_else(|_| "main".into());
        for row in rows {
            let Some(branch) = row.branch.clone() else { continue };
            if !branch.starts_with(mesimon_core::workspace::BRANCH_NS) {
                continue;
            }
            let path = row.path.canonicalize().unwrap_or(row.path.clone());
            if !path.starts_with(&wt_root) {
                continue;
            }
            let Some(ticket) = marker_ticket(&path) else { continue };
            let Some(container) = path.parent().map(Path::to_path_buf) else { continue };
            let branch_oid = branch_tip(&child, &branch);
            let base_oid = git_read(&child, &["merge-base", &child_base, &branch])
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|_| branch_oid.clone());
            let b = out.entry(ticket).or_insert_with(|| Binding {
                path: container,
                branch: branch.clone(),
                base_oid: String::new(),
                branch_oid: String::new(),
                status: BindingStatus::Attached,
                locked: row.locked_reason.is_some(),
                repos: Vec::new(),
            });
            if b.repos.is_empty() && root_found.contains(&ticket) {
                // The meta's own row: the root leg, from what the scan above
                // recovered, judged against the branch the meta stands on.
                let root_base = checked_out(repo).map(|(b, _)| b).unwrap_or_else(|_| base.clone());
                b.repos.push(RepoBinding {
                    name: String::new(),
                    base: root_base,
                    base_oid: b.base_oid.clone(),
                    branch_oid: b.branch_oid.clone(),
                });
            }
            b.repos.push(RepoBinding {
                name: name.clone(),
                base: child_base.clone(),
                base_oid,
                branch_oid,
            });
        }
    }
    for b in out.values_mut().filter(|b| b.is_workspace()) {
        let all_present = b.legs(repo, "").iter().all(|l| l.path.is_dir());
        b.status = if all_present { BindingStatus::Attached } else { BindingStatus::Evicted };
    }
    out
}

/// Every leg's directory is on disk and carries our marker — the test a
/// recovered or interrupted binding must pass before anything trusts it. A
/// folder container (no root leg) has no marker of its own; its directory
/// is enough.
pub fn verify_on_disk(ticket: ulid::Ulid, b: &Binding, repo_root: &Path) -> bool {
    b.path.is_dir()
        && b.legs(repo_root, "")
            .iter()
            .all(|leg| leg.path.is_dir() && marker_ticket(&leg.path) == Some(ticket))
}

/// Startup loader for `worktrees.json`: parse, and on failure quarantine and
/// rebuild from disk. Returns the bindings, any notices, and whether writes
/// are barred — barred means a file we could not read is still on disk (or a
/// rebuild we could not verify), so overwriting it would destroy the only
/// record of real worktrees and branches.
///
/// Never called off the writer thread: `serve_diff` keeps using the pure
/// `load_bindings`, so two threads can never race on the rename.
pub fn load_or_recover(paths: &Paths) -> (Bindings, Vec<Notice>, bool) {
    let f = bindings_file(paths);
    let mut notices = Vec::new();
    if !f.is_file() {
        return (Bindings::new(), notices, false);
    }
    let text = match std::fs::read_to_string(&f) {
        Ok(t) => t,
        Err(e) => {
            notices.push(
                Notice::new("worktrees_barred", "worktree bindings could not be opened")
                    .with_path(f.display())
                    .with_detail(e.to_string()),
            );
            return (rebuild_from_disk(&paths.repo_root, &paths.worktrees_root()), notices, true);
        }
    };

    let fault = match parse_bindings(&text) {
        Ok(b) => return (b, notices, false),
        Err((Some(found), _)) => {
            // Valid bytes from a newer mesimon: leave them exactly where they
            // are and pause worktree work, or the next save downgrades them.
            notices.push(
                Notice::new(
                    "future_version",
                    format!(
                        "worktree bindings were written by a newer mesimon \
                         (schema {found}, this build reads {BINDINGS_SCHEMA}) — \
                         worktree actions are paused"
                    ),
                )
                .with_path(f.display()),
            );
            return (rebuild_from_disk(&paths.repo_root, &paths.worktrees_root()), notices, true);
        }
        Err((None, detail)) => detail,
    };

    let moved = crate::store::quarantine(&f);
    let rebuilt = rebuild_from_disk(&paths.repo_root, &paths.worktrees_root());
    // Unbar only when every rebuilt binding verifies against disk: the
    // directory is there and its marker still names the same ticket. Anything
    // less and we keep the bar, so nothing is torn down on a guess.
    let verified =
        moved.is_some() && rebuilt.iter().all(|(id, b)| verify_on_disk(*id, b, &paths.repo_root));
    notices.push(
        Notice::new(
            "worktrees_barred",
            format!(
                "worktree bindings could not be read — {} recovered from git{}",
                rebuilt.len(),
                if verified { "" } else { "; worktree actions are paused" }
            ),
        )
        .with_path(f.display())
        .with_detail(fault),
    );
    (rebuilt, notices, !verified)
}

/// `~/.local/state/mesimon/<proj16>/worktrees/` — created once, 0700, spotlight
/// and Time Machine excluded (best effort).
pub fn ensure_root(paths: &Paths) -> Result<PathBuf> {
    let root = paths.worktrees_root();
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
    let out = crate::git::git(repo).args(args).output().context("run git")?;
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

/// The remote's HEAD → {main, master, trunk} → current HEAD name.
///
/// The remote asked is the one the checkout's branch tracks
/// (`branch.<b>.remote`), then `origin`, then the only remote there is. It
/// was a literal `origin/HEAD` (T-225): the author's v2 repos name their one
/// remote `gitlab`, so the first rung never fired there and a stale `main`
/// on the second beat the real deploy branch the remote's HEAD names.
pub fn default_branch(repo: &Path) -> Result<String> {
    let head = git_read(repo, &["rev-parse", "--abbrev-ref", "HEAD"])
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let mut remotes: Vec<String> = Vec::new();
    let mut consider = |r: String| {
        if !r.is_empty() && !remotes.contains(&r) {
            remotes.push(r);
        }
    };
    if let Some(r) = crate::gitstatus::remote_of(repo, &head) {
        consider(r);
    }
    consider("origin".into());
    if let Ok(list) = git_read(repo, &["remote"]) {
        let all: Vec<&str> = list.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        if let [only] = all[..] {
            consider(only.to_string());
        }
    }
    for remote in &remotes {
        let refname = format!("refs/remotes/{remote}/HEAD");
        if let Ok(s) = git_read(repo, &["symbolic-ref", "--short", &refname]) {
            if let Some(b) = s.trim().strip_prefix(&format!("{remote}/")) {
                return Ok(b.to_string());
            }
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

/// The remote-tracking ref the base branch is mirrored by — `origin/main` —
/// when git has one (T-267). A forge merges a PR THERE, and a fetch is the
/// only thing that moves it, so it is the ref an upstream merge lands on
/// while the user's own `main` stays where they left it.
///
/// The remote asked is the one the base branch tracks, then `origin`, then
/// the only remote there is: `default_branch`'s ladder, for its reasons.
pub fn upstream_base(repo: &Path, base: &str) -> Option<String> {
    let mut remotes: Vec<String> = Vec::new();
    let mut consider = |r: String| {
        if !r.is_empty() && !remotes.contains(&r) {
            remotes.push(r);
        }
    };
    if let Some(r) = crate::gitstatus::remote_of(repo, base) {
        consider(r);
    }
    consider("origin".into());
    if let Ok(list) = git_read(repo, &["remote"]) {
        let all: Vec<&str> = list.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        if let [only] = all[..] {
            consider(only.to_string());
        }
    }
    for remote in &remotes {
        let refname = format!("refs/remotes/{remote}/{base}");
        if git_read(repo, &["rev-parse", "--verify", "--quiet", &refname]).is_ok() {
            return Some(format!("{remote}/{base}"));
        }
    }
    None
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

    // Stages 1/1b/2.
    add_leg(repo, &dir, &branch, Some(&base_oid), ticket_id)?;

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
        repos: Vec::new(),
    })
}

/// Stages 1/1b/2 for ONE leg: `worktree add` (with `-b <branch> <start>`
/// when `start` is given — a fresh branch; without it the branch exists and
/// is checked out as is), the ownership marker, the `.worktreeinclude` copy.
/// The road every provision takes, single-repo or per leg.
fn add_leg(
    repo: &Path,
    wt: &Path,
    branch: &str,
    start: Option<&str>,
    ticket_id: ulid::Ulid,
) -> std::result::Result<(), (String, String)> {
    // Stage 1 — add. Never --force, never --relative-paths.
    let wt_s = wt.to_string_lossy().into_owned();
    match start {
        Some(oid) => git(repo, &["worktree", "add", "--quiet", &wt_s, "-b", branch, oid]),
        None => git(repo, &["worktree", "add", "--quiet", &wt_s, branch]),
    }
    .map_err(|e| ("add".into(), e.to_string()))?;

    // Stage 1b — ownership marker inside the admin dir (survives clean -xfd,
    // dies with worktree remove/prune, needs no shared-config write).
    let marker = git_read(wt, &["rev-parse", "--git-path", "mesimon-ticket"])
        .map_err(|e| ("mark".into(), e.to_string()))?;
    let marker_path = {
        let p = PathBuf::from(marker.trim());
        if p.is_absolute() {
            p
        } else {
            wt.join(p)
        }
    };
    std::fs::write(&marker_path, format!("{ticket_id}\n{}\n1\n", repo.display()))
        .map_err(|e| ("mark".into(), e.to_string()))?;

    // Stage 2 — .worktreeinclude copy (Claude Code semantics: pattern match AND
    // gitignored). Skip silently when the repo has no file.
    copy_worktreeinclude(repo, wt).map_err(|e| ("include".into(), e.to_string()))
}

/// The branch a checkout stands on and its tip. `Err` names a detached or
/// unborn HEAD — a leg's base is the branch its checkout is on, and a
/// checkout on no branch has no base to offer (fail closed, D26).
fn checked_out(repo: &Path) -> std::result::Result<(String, String), String> {
    let name = git_read(repo, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .map(|s| s.trim().to_string())
        .map_err(|_| "is on a detached HEAD — check out a branch there first".to_string())?;
    let refname = format!("refs/heads/{name}");
    let oid = git_read(repo, &["rev-parse", "--verify", "--quiet", &refname])
        .map(|s| s.trim().to_string())
        .map_err(|_| format!("branch {name} has no commits"))?;
    Ok((name, oid))
}

/// Provision a WORKSPACE worktree (T-368): the container under
/// `worktrees_root` — a worktree of the meta on the ticket branch when the
/// root is a repository (`is_meta`), a plain directory holding the root's
/// `CLAUDE.md` otherwise — and inside it one worktree per census repo, each
/// on the same `msmn/` branch, based on the branch that repo's checkout is
/// on. Every repo is cut (author, 2026-09-22: no guessing which ones a
/// ticket will touch). Sequential, off the writer thread; `progress` is told
/// `(done, total)` after every leg.
///
/// Fail closed: the precheck writes nothing and refuses on the first repo
/// that is detached or already holds the branch; a leg that fails to add
/// takes every leg already added with it, branches included.
#[allow(clippy::too_many_arguments)]
pub fn provision_workspace(
    root: &Path,
    worktrees_root: &Path,
    census: &[String],
    is_meta: bool,
    ticket_id: ulid::Ulid,
    short_key: &str,
    title: &str,
    progress: &mut dyn FnMut(u32, u32),
) -> std::result::Result<Binding, (String, String)> {
    let branch = mesimon_core::workspace::branch_name(short_key, title);
    let dir = worktrees_root.join(mesimon_core::workspace::dir_name(short_key, title));

    // Stage 0 — precheck, every repo, nothing written.
    if dir.exists() {
        return Err(("precheck".into(), format!("{} already exists", dir.display())));
    }
    let branch_ref = format!("refs/heads/{branch}");
    let mut plan: Vec<(String, PathBuf, String, String)> = Vec::new();
    let names = std::iter::once(String::new()).filter(|_| is_meta).chain(census.iter().cloned());
    for name in names {
        let repo = if name.is_empty() { root.to_path_buf() } else { root.join(&name) };
        let (base, head) = checked_out(&repo)
            .map_err(|e| ("precheck".into(), format!("{} {e}", leg_word(&name))))?;
        if git_read(&repo, &["rev-parse", "--verify", "--quiet", &branch_ref]).is_ok() {
            return Err((
                "precheck".into(),
                format!("branch {branch} already exists in {}", leg_word(&name)),
            ));
        }
        plan.push((name, repo, base, head));
    }
    if plan.is_empty() {
        return Err(("precheck".into(), "no repositories to cut a worktree of".into()));
    }

    let total = plan.len() as u32;
    let mut added: Vec<(PathBuf, PathBuf)> = Vec::new(); // (repo, wt), in add order
    let rollback = |added: &[(PathBuf, PathBuf)]| {
        for (repo, wt) in added.iter().rev() {
            let _ = remove(repo, wt);
            let _ = delete_branch(repo, &branch, true);
        }
        if !is_meta {
            std::fs::remove_dir_all(&dir).ok();
        }
    };

    // The container.
    if !is_meta {
        if let Err(e) = std::fs::create_dir_all(&dir) {
            return Err(("add".into(), e.to_string()));
        }
        let claude = root.join("CLAUDE.md");
        if claude.is_file() {
            let _ = std::fs::copy(&claude, dir.join("CLAUDE.md"));
        }
    }

    let mut repos = Vec::with_capacity(plan.len());
    for (i, (name, repo, base, head)) in plan.iter().enumerate() {
        let wt = if name.is_empty() { dir.clone() } else { dir.join(name) };
        if let Err((stage, message)) = add_leg(repo, &wt, &branch, Some(head), ticket_id) {
            // `add -b` mints the branch before it lays the tree, so a failed
            // add can leave the branch behind; the precheck saw none, so
            // one standing at `head` is ours to take back.
            if branch_tip(repo, &branch) == *head {
                let _ = delete_branch(repo, &branch, true);
            }
            rollback(&added);
            return Err((stage, format!("{}: {message}", leg_word(name))));
        }
        added.push((repo.clone(), wt));
        repos.push(RepoBinding {
            name: name.clone(),
            base: base.clone(),
            base_oid: head.clone(),
            branch_oid: head.clone(),
        });
        progress(i as u32 + 1, total);
    }

    let dir = dir.canonicalize().unwrap_or(dir);
    let root_leg = repos.iter().find(|r| r.name.is_empty());
    Ok(Binding {
        path: dir,
        branch,
        base_oid: root_leg.map(|r| r.base_oid.clone()).unwrap_or_default(),
        branch_oid: root_leg.map(|r| r.branch_oid.clone()).unwrap_or_default(),
        status: BindingStatus::Attached,
        locked: false,
        repos,
    })
}

/// The ticket ULID a worktree's ownership marker names, if the marker exists
/// and parses. First line of `$(git rev-parse --git-path mesimon-ticket)`.
pub fn marker_ticket(dir: &Path) -> Option<ulid::Ulid> {
    let marker = git_read(dir, &["rev-parse", "--git-path", "mesimon-ticket"]).ok()?;
    let p = PathBuf::from(marker.trim());
    let p = if p.is_absolute() { p } else { dir.join(p) };
    let body = std::fs::read_to_string(p).ok()?;
    body.lines().next()?.parse().ok()
}

/// Startup settle for bindings a dead daemon left mid-provision. `Queued` and
/// `Provisioning` are thread-backed states; loaded from disk they have no
/// thread behind them, and `resolve_spawn_cwd` would park every future spawn
/// behind them forever ("session starts when ready" that never comes).
/// Resolve each from what actually landed on disk:
/// dir + our marker → `Attached`; dir without marker → fail closed (D26);
/// branch only → `Evicted` (`provision_existing` replays); nothing → drop the
/// binding, the next spawn provisions fresh. Returns whether anything changed.
pub fn reconcile_interrupted(repo: &Path, bindings: &mut Bindings) -> bool {
    let mut changed = false;
    bindings.retain(|ticket, b| {
        if !matches!(b.status, BindingStatus::Queued | BindingStatus::Provisioning) {
            return true;
        }
        changed = true;
        if b.is_workspace() {
            // A workspace binding (T-368): whole, or not at all. Every leg on
            // disk with our marker → `Attached`. Anything partial is taken
            // down — the legs that landed are removed, and a leg's branch
            // goes only where it never left its base (an interrupted replay
            // of an evicted binding has branches carrying work). A surviving
            // branch keeps the binding as `Evicted`; none drops it.
            if verify_on_disk(*ticket, b, repo) {
                b.status = BindingStatus::Attached;
                return true;
            }
            let legs = b.legs(repo, "");
            for leg in legs.iter().rev() {
                if leg.path.is_dir() {
                    let _ = remove(&leg.repo, &leg.path);
                }
                let tip = branch_tip(&leg.repo, &b.branch);
                if !tip.is_empty() && tip == leg.base_oid {
                    let _ = delete_branch(&leg.repo, &b.branch, true);
                }
            }
            if !b.has_root_leg() {
                std::fs::remove_dir_all(&b.path).ok();
            }
            let kept = legs.iter().any(|leg| !branch_tip(&leg.repo, &b.branch).is_empty());
            if kept {
                b.status = BindingStatus::Evicted;
                b.locked = false;
            }
            return kept;
        }
        if b.path.is_dir() {
            if marker_ticket(&b.path) == Some(*ticket) {
                b.status = BindingStatus::Attached;
            } else {
                b.status = BindingStatus::Error {
                    stage: "reconcile".into(),
                    message: format!(
                        "interrupted provision left {} — remove it, then retry",
                        b.path.display()
                    ),
                };
            }
            return true;
        }
        let branch_exists = !b.branch.is_empty()
            && git_read(
                repo,
                &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{}", b.branch)],
            )
            .is_ok();
        if branch_exists {
            b.status = BindingStatus::Evicted;
            true
        } else {
            false
        }
    });
    changed
}

/// Recreate an evicted worktree: the branch already exists, so `add` without
/// `-b`, then re-mark and re-include. Commits made on the branch survive
/// eviction — this restores the working directory only.
pub fn provision_existing(
    repo: &Path,
    ticket_id: ulid::Ulid,
    prior: &Binding,
) -> std::result::Result<Binding, (String, String)> {
    if prior.is_workspace() {
        return provision_existing_workspace(repo, ticket_id, prior);
    }
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

/// The evicted road for a workspace binding: every leg whose tree is gone
/// is re-added on its existing branch; a leg still standing with our marker
/// (a half-evicted container) is kept. Rollback removes only the legs THIS
/// call added and never a branch — the branches carry the work eviction
/// preserved.
fn provision_existing_workspace(
    repo_root: &Path,
    ticket_id: ulid::Ulid,
    prior: &Binding,
) -> std::result::Result<Binding, (String, String)> {
    let legs = prior.legs(repo_root, "");
    let branch_ref = format!("refs/heads/{}", prior.branch);
    for leg in &legs {
        if git_read(&leg.repo, &["rev-parse", "--verify", "--quiet", &branch_ref]).is_err() {
            return Err((
                "precheck".into(),
                format!("branch {} no longer exists in {}", prior.branch, leg_word(&leg.name)),
            ));
        }
        if leg.path.exists() && marker_ticket(&leg.path) != Some(ticket_id) {
            return Err(("precheck".into(), format!("{} already exists", leg.path.display())));
        }
    }
    if !prior.has_root_leg() {
        if let Err(e) = std::fs::create_dir_all(&prior.path) {
            return Err(("add".into(), e.to_string()));
        }
    }
    let mut added: Vec<&Leg> = Vec::new();
    for leg in &legs {
        if leg.path.is_dir() {
            continue;
        }
        if let Err((stage, message)) = add_leg(&leg.repo, &leg.path, &prior.branch, None, ticket_id)
        {
            for done in added.iter().rev() {
                let _ = remove(&done.repo, &done.path);
            }
            return Err((stage, format!("{}: {message}", leg_word(&leg.name))));
        }
        added.push(leg);
    }
    let mut b = prior.clone();
    b.path = b.path.canonicalize().unwrap_or(b.path);
    for r in &mut b.repos {
        let leg_repo =
            if r.name.is_empty() { repo_root.to_path_buf() } else { repo_root.join(&r.name) };
        r.branch_oid = branch_tip(&leg_repo, &prior.branch);
    }
    if let Some(root) = b.repos.iter().find(|r| r.name.is_empty()) {
        b.branch_oid = root.branch_oid.clone();
    }
    b.status = BindingStatus::Attached;
    b.locked = false;
    Ok(b)
}

/// Copy every gitignored file that matches a `.worktreeinclude` pattern into the
/// new worktree, preserving relative paths.
fn copy_worktreeinclude(repo: &Path, dest: &Path) -> Result<()> {
    let inc = repo.join(".worktreeinclude");
    let Ok(patterns) = std::fs::read_to_string(&inc) else { return Ok(()) };
    let patterns: Vec<&str> =
        patterns.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
    if patterns.is_empty() {
        return Ok(());
    }
    // The gitignored universe (files only, NUL-safe).
    let out = crate::git::git(repo)
        .args([
            "--no-optional-locks",
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "-z",
        ])
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
            std::fs::copy(&from, &to).with_context(|| format!("copy {rel}"))?;
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
            (Some(&"**"), _) => rec(&pat[1..], path) || (!path.is_empty() && rec(pat, &path[1..])),
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
    let out = crate::git::git(repo)
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
        let pid = reason.rsplit(' ').next().and_then(|p| p.parse::<i32>().ok());
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

/// Fast-forward possible: base's tip is an ancestor of the branch tip.
/// False = base moved since the branch was cut — the agent rebases first
/// (mesimon never mints merge commits; history stays linear).
pub fn ff_possible(repo: &Path, branch: &str, base: &str) -> bool {
    crate::git::git(repo)
        .args(["--no-optional-locks", "merge-base", "--is-ancestor", base, branch])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
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
    crate::git::git(repo)
        .args(["--no-optional-locks", "merge-base", "--is-ancestor", branch, base])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Git config a user could set that would move a patch-id on ONE side of the
/// comparison only, or break the read outright — pinned on the command line,
/// where nothing can reach them (T-267). `diff.orderFile` naming a file that
/// is gone is fatal to every diff; `log.follow` applies to `log` and never to
/// `diff-tree`; the rest change the patch text.
const GIT_PINS: [&str; 22] = [
    "-c",
    "diff.renames=false",
    "-c",
    "diff.noprefix=false",
    "-c",
    "diff.mnemonicPrefix=false",
    "-c",
    "diff.relative=false",
    "-c",
    "diff.context=3",
    "-c",
    "diff.interHunkContext=0",
    "-c",
    "diff.external=",
    "-c",
    "diff.orderFile=/dev/null",
    "-c",
    "core.abbrev=40",
    "-c",
    "log.follow=false",
    "-c",
    "log.showSignature=false",
];

/// The diff flags both sides of a patch-id comparison carry. `--no-renames`
/// is the load-bearing one: rename detection is on by default, it changes the
/// id, and its PAIRING depends on which paths are in the diff — so the path
/// filter on the target side would flip it there and nowhere else.
/// `--full-index` is for binaries, whose ids carry the abbreviated blob oids
/// and would otherwise drift as the repo grows; `--no-ext-diff`/`--no-textconv`
/// keep a user's own diff program from running on the daemon's worker thread.
///
/// `--unified=0` is what makes a squash on a busy base match at all (T-438).
/// A patch-id hashes every line of a hunk, context included, and the squash
/// commit's context is the target's text at merge time — so one commit on
/// the base within three lines of the branch's edit, landed before the PR
/// was, moved the id and the ticket read unmerged forever. With no context
/// only the changed lines are hashed, which is the same text on both sides
/// however the surroundings moved. `--diff-algorithm=myers` pins the other
/// thing a user's config could move: which lines a diff calls changed.
const DIFF_FLAGS: [&str; 11] = [
    "-r",
    "-p",
    "--no-color",
    "--no-renames",
    "--no-textconv",
    "--no-ext-diff",
    "--full-index",
    "--unified=0",
    "--diff-algorithm=myers",
    "--src-prefix=a/",
    "--dst-prefix=b/",
];

/// `log`'s own: one compact header line per commit, which is what carries the
/// commit id out through `patch-id`, and no signature or notes to be mistaken
/// for a patch.
const LOG_FLAGS: [&str; 5] = [
    "--no-merges",
    "--no-abbrev-commit",
    "--no-notes",
    "--no-show-signature",
    "--pretty=tformat:commit %H",
];

/// How far before the branch's own last commit the target's history is
/// walked. A squash lands AFTER the work it squashes, so the window only has
/// to reach back over clock skew and a rebase or two; a week is generous and
/// still turns hundreds of commits into a handful.
const SINCE_SLACK_SECS: u64 = 7 * 24 * 3600;

/// A hard stop on the walk, for a repository with a week of commits in it.
const CONTENT_SCAN_MAX: &str = "500";

/// Bindings that may run a fresh scan in one pass. A fetch moves the target
/// and stales every memo at once; without this, thirteen bindings would scan
/// together on the writer's own worker.
const CONTENT_SCANS_PER_PASS: u32 = 2;

/// `git` with the read-path options and the pinned config — the base of every
/// command whose output a patch-id is taken of. `--literal-pathspecs` because
/// the paths fed back as pathspecs are file names, and a file may be called
/// `x[1].txt`.
fn git_patch(repo: &Path) -> Command {
    let mut c = crate::git::git(repo);
    c.args(["--no-optional-locks", "--literal-pathspecs"]).args(GIT_PINS);
    c
}

/// `git <args> | git patch-id --stable`, as `(patch id, commit id)` pairs.
///
/// A `log` stream names the commit each patch came from; a `diff-tree` stream
/// has no header line and the second field is forty zeros, which the caller
/// that uses that form ignores. An empty patch produces no line at all.
/// Failure of either half is an empty answer: this decides a WORD on a card,
/// never a write.
fn patch_ids(repo: &Path, args: &[&str]) -> Vec<(String, String)> {
    let Ok(mut left) = git_patch(repo)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return Vec::new();
    };
    // The pipe's read end goes to the child, so nothing here has to drain it
    // and the two cannot deadlock on each other.
    let Some(out) = left.stdout.take() else {
        let _ = left.wait();
        return Vec::new();
    };
    let right = crate::git::git(repo)
        .args(["patch-id", "--stable"])
        .stdin(Stdio::from(out))
        .stderr(Stdio::null())
        .output();
    let _ = left.wait();
    let Ok(right) = right else { return Vec::new() };
    String::from_utf8_lossy(&right.stdout)
        .lines()
        .filter_map(|l| {
            let (id, commit) = l.split_once(' ')?;
            Some((id.to_string(), commit.trim().to_string()))
        })
        .collect()
}

/// The commit on `target` that already carries this branch's work, when there
/// is one (T-267) — the answer `merge-base --is-ancestor` cannot give.
///
/// A forge's "Squash and merge" lands ONE commit whose patch is the branch's
/// whole diff, and none of the branch's own commits are ancestors of anything
/// afterwards; a rebase-merge lands them rewritten. Either way the ahead
/// count `compute_flags` reads says "not merged" forever, and the ticket goes
/// on asking its agent to rebase work that is done.
///
/// Patch-ids are how git answers this itself (`git cherry`). The usual trick
/// writes a dangling squash commit with `commit-tree` and asks `cherry` about
/// it; mesimon writes NOTHING to the repository (README promise 1), so both
/// sides are computed and compared here instead:
///
/// - **ours** — the branch as ONE patch (`mb..branch` in a single diff), which
///   is what a squash lands
/// - **theirs** — the target's commits since the merge base, filtered to the
///   files the branch touched and to the week before the branch's last commit,
///   which is what keeps this cheap on a busy base
/// - **each** — the branch's commits one at a time, asked only when the first
///   comparison missed, which is what a rebase-merge lands
///
/// Restricting the target side to our paths is not a narrowing of the
/// comparison: `--stable` sums the file stanzas independently, so a squash
/// that also touched a lockfile still matches the branch that did not.
pub fn content_merged(repo: &Path, branch: &str, target: &str, tip_time: u64) -> Option<String> {
    let mb = git_read(repo, &["merge-base", target, branch]).ok()?.trim().to_string();
    if mb.is_empty() {
        return None;
    }
    // NUL-separated and never rename-detected: `--name-only` quotes a
    // non-ASCII path, and a rename would name only the destination, whose
    // stanza could then never match ours.
    let names = git_patch(repo)
        .args(["diff-tree", "-r", "--no-commit-id", "--no-renames", "--name-only", "-z"])
        .args([&mb, branch])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let paths: Vec<String> =
        names.split('\0').filter(|s| !s.is_empty()).map(String::from).collect();
    if paths.is_empty() {
        // A branch whose net change is nothing has no patch to look for.
        return None;
    }
    let mut ours_argv: Vec<&str> = vec!["diff-tree"];
    ours_argv.extend_from_slice(&DIFF_FLAGS);
    ours_argv.extend_from_slice(&[&mb, branch]);
    let ours = patch_ids(repo, &ours_argv).into_iter().next().map(|(id, _)| id);
    let range = format!("{mb}..{target}");
    let since = format!("--since=@{}", tip_time.saturating_sub(SINCE_SLACK_SECS));
    let mut theirs_argv: Vec<&str> = vec!["log"];
    theirs_argv.extend_from_slice(&DIFF_FLAGS);
    theirs_argv.extend_from_slice(&LOG_FLAGS);
    theirs_argv.extend_from_slice(&["--max-count", CONTENT_SCAN_MAX]);
    if tip_time > 0 {
        theirs_argv.push(&since);
    }
    theirs_argv.push(&range);
    theirs_argv.push("--");
    theirs_argv.extend(paths.iter().map(String::as_str));
    let theirs = patch_ids(repo, &theirs_argv);
    if theirs.is_empty() {
        return None;
    }
    if let Some(id) = &ours {
        if let Some((_, commit)) = theirs.iter().find(|(p, _)| p == id) {
            return Some(commit.clone());
        }
    }
    // Rebase-merge: every one of the branch's own patches is up there, and
    // the newest of them is the commit to name. Asked only now, because a
    // squash — the common case — has already answered above.
    let mine_range = format!("{mb}..{branch}");
    let mut mine_argv: Vec<&str> = vec!["log"];
    mine_argv.extend_from_slice(&DIFF_FLAGS);
    mine_argv.extend_from_slice(&LOG_FLAGS);
    mine_argv.push(&mine_range);
    let each: Vec<String> = patch_ids(repo, &mine_argv).into_iter().map(|(id, _)| id).collect();
    if !each.is_empty() && each.iter().all(|id| theirs.iter().any(|(p, _)| p == id)) {
        return theirs.iter().find(|(p, _)| *p == each[0]).map(|(_, c)| c.clone());
    }
    None
}

/// One binding's question for `compute_flags`: the branch, the base it was
/// cut from (`Binding::base_oid`), and the last content-merge verdict so the
/// patch scan runs only when a tip has actually moved (T-267).
#[derive(Debug, Clone)]
pub struct FlagInput {
    pub ticket: ulid::Ulid,
    pub branch: String,
    pub base_oid: String,
    pub seen: Option<ContentSeen>,
}

/// Whether a branch's work is on the ref a merge would land it in — asked
/// the expensive way, and remembered so it is not asked again. A verdict that
/// NAMED a commit is permanent: that commit never leaves the target's
/// history, so one `--is-ancestor` re-affirms it however far the target moves
/// afterwards, and the scan runs once in the life of the branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentSeen {
    pub branch_tip: String,
    /// The ref it was judged against — `main` or `origin/main`.
    pub target: String,
    pub target_tip: String,
    pub merged: bool,
    /// The commit carrying the patch when patch equality is what found it,
    /// empty when the branch is a plain ancestor of the target.
    pub oid: String,
}

/// One binding's answer — the three flags the card and the train read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flags {
    pub ticket: ulid::Ulid,
    pub merged: bool,
    pub ahead: u32,
    pub needs_rebase: bool,
    /// The branch's tip at the sample, so the snapshot road and the train
    /// can key a refusal on it without a fork of their own.
    pub tip: String,
    /// Where the work landed, when that is worth saying: `origin/main`, or
    /// `main` where a squash and not a fast-forward is what put it there
    /// (T-267). Empty for the ordinary ancestor merge, and while unmerged.
    pub merged_in: String,
    /// The commit on that ref carrying the branch's patch, when a squash or a
    /// rebase-merge is what put it there. Empty for a plain ancestor merge.
    pub merged_oid: String,
    /// The verdict to remember for the next pass.
    pub seen: Option<ContentSeen>,
}

/// Every binding's flags, sampled together, plus the base branch they were
/// judged against and its tip, and the branches checked out twice.
#[derive(Debug, Clone, Default)]
pub struct WtFlags {
    pub base: String,
    pub base_tip: String,
    pub flags: Vec<Flags>,
    pub conflicts: Vec<String>,
}

/// Every local branch's tip and last commit time in one fork:
/// `refs/heads/<name>` → `Tip`, and the upstream ref's own beside it when one
/// was asked for (T-267 adds the second pattern, and the date the content
/// scan's window hangs off, to the same fork). The full refname is asked for,
/// not `refname:short`, which git abbreviates differently when a
/// remote-tracking ref shares the name — and the upstream tip is returned
/// separately rather than under `origin/main` in the map, where a local
/// branch of that name would collide with it.
fn branch_tips(repo: &Path, upstream: Option<&str>) -> (HashMap<String, Tip>, Tip) {
    let up_ref = upstream.map(|u| format!("refs/remotes/{u}"));
    let mut args = vec![
        "for-each-ref",
        "--format=%(refname) %(objectname) %(committerdate:unix)",
        "refs/heads/",
    ];
    if let Some(r) = &up_ref {
        args.push(r);
    }
    let mut tips: HashMap<String, Tip> = HashMap::new();
    let mut up_tip = Tip::default();
    if let Ok(out) = git_read(repo, &args) {
        for line in out.lines() {
            let mut parts = line.split(' ');
            let (Some(name), Some(oid)) = (parts.next(), parts.next()) else { continue };
            let tip = Tip {
                oid: oid.to_string(),
                time: parts.next().and_then(|t| t.trim().parse().ok()).unwrap_or(0),
            };
            if let Some(short) = name.strip_prefix("refs/heads/") {
                tips.insert(short.to_string(), tip);
            } else if up_ref.as_deref() == Some(name) {
                up_tip = tip;
            }
        }
    }
    (tips, up_tip)
}

/// A ref's tip and the commit time on it.
#[derive(Debug, Clone, Default)]
struct Tip {
    oid: String,
    time: u64,
}

/// One binding's content question, so the verdict below reads as a sentence
/// rather than as eight arguments.
struct Scan<'a> {
    repo: &'a Path,
    branch: &'a str,
    tip: &'a Tip,
    target: &'a str,
    target_tip: &'a str,
    /// The target IS the local base, whose ancestry the counts have already
    /// answered — so the verdict below asks git that question only where it
    /// is a new one.
    target_is_base: bool,
}

/// Has the branch's work landed on the target — as an ancestor, or as a patch
/// a forge squashed or rebased on (T-267)? Three roads, cheapest first:
///
/// 1. a verdict that named a commit, re-affirmed with ONE `--is-ancestor`:
///    permanent, and what keeps a merged ticket merged as the base runs on
/// 2. a verdict whose three tips have not moved: free
/// 3. the scan, under the pass's budget — a fetch stales every memo at once,
///    and the sample must not turn into thirteen walks of the base's history
fn content_verdict(
    scan: &Scan,
    seen: Option<&ContentSeen>,
    budget: &mut u32,
) -> Option<ContentSeen> {
    let fresh = |merged: bool, oid: String| ContentSeen {
        branch_tip: scan.tip.oid.clone(),
        target: scan.target.to_string(),
        target_tip: scan.target_tip.to_string(),
        merged,
        oid,
    };
    if let Some(s) = seen {
        // A branch that has moved has work that may not be up there, whatever
        // was true of the tip before it.
        if s.branch_tip == scan.tip.oid {
            if s.target == scan.target && s.target_tip == scan.target_tip {
                return Some(s.clone());
            }
            if s.merged && !s.oid.is_empty() && is_merged(scan.repo, &s.oid, scan.target) {
                return Some(fresh(true, s.oid.clone()));
            }
        }
    }
    if *budget == 0 {
        // Not answered this pass: `None` leaves the old verdict standing —
        // stale, so the next pass asks — where writing a fresh "no" here
        // would look answered and never be asked again.
        return None;
    }
    *budget -= 1;
    if !scan.target_is_base && is_merged(scan.repo, scan.branch, scan.target) {
        return Some(fresh(true, String::new()));
    }
    Some(match content_merged(scan.repo, scan.branch, scan.target, scan.tip.time) {
        Some(oid) => fresh(true, oid),
        None => fresh(false, String::new()),
    })
}

/// The merged / ahead / needs-rebase flags for every binding, judged
/// against `base`, in `2 + n` git forks — read-only, `--no-optional-locks`,
/// safe on any thread (T-216: the four-forks-per-binding form ran on the
/// writer thread, and thirteen bindings held every keypress ~0.5 s once
/// every 10 s).
///
/// `rev-list --left-right --count base...branch` answers three of the old
/// four questions at once: the left count is what base has that the branch
/// lacks (zero ⇔ `ff_possible`), the right is what the branch has that base
/// lacks (`ahead_count`; zero ⇔ `is_merged`). "Merged" additionally needs
/// the tip to have MOVED off the creation base — a fresh branch is trivially
/// an ancestor of base, and that is "no work yet" (dogfood 2026-08-30). A
/// branch git no longer has reads as the old helpers read it: not merged,
/// nothing ahead, and no fast-forward, so needs-rebase.
///
/// Ancestry is not the only way to be merged since T-267: a PR squashed or
/// rebased into `origin/main` leaves every one of the branch's own commits
/// behind, and the card would read "main moved" forever. `content_verdict`
/// asks the second question — patch equality — against ONE target ref for
/// the pass, and remembers the answer on `FlagInput::seen`, so the sample
/// stays `2 + n` forks until a tip moves. `ahead` and `needs_rebase` keep
/// meaning what they meant: they are about the LOCAL base, which is what a
/// fast-forward here would move.
pub fn compute_flags(
    repo: &Path,
    base: &str,
    upstream: Option<&str>,
    inputs: &[FlagInput],
) -> WtFlags {
    let (tips, upstream_tip) = branch_tips(repo, upstream);
    let base = base.to_string();
    let base_tip = tips.get(&base).cloned().unwrap_or_default();
    // ONE target for the pass (T-267): the ref a merged PR lands on where
    // there is one holding everything the local base holds, else the local
    // base itself. The condition answers the other case for free — a squash
    // merged HERE and not pushed leaves the base ahead of the upstream, and
    // the base is then what to look in.
    let (target, target_tip) = match upstream {
        Some(u)
            if !upstream_tip.oid.is_empty()
                && (base_tip.oid.is_empty() || is_merged(repo, &base, u)) =>
        {
            (u.to_string(), upstream_tip.oid.clone())
        }
        _ => (base.clone(), base_tip.oid.clone()),
    };
    let mut budget = CONTENT_SCANS_PER_PASS;
    let flags = inputs
        .iter()
        .filter(|i| !i.branch.is_empty())
        .map(|i| {
            let tip = tips.get(&i.branch).cloned().unwrap_or_default();
            let counts = git_read(
                repo,
                &["rev-list", "--left-right", "--count", &format!("{base}...{}", i.branch)],
            )
            .ok()
            .and_then(|s| {
                let mut it = s.split_whitespace().map(|n| n.parse::<u32>().ok());
                Some((it.next()??, it.next()??))
            });
            let (ancestor, ahead, ff) = match counts {
                Some((behind, ahead)) => {
                    (!tip.oid.is_empty() && tip.oid != i.base_oid && ahead == 0, ahead, behind == 0)
                }
                None => (false, 0, false),
            };
            // The work can be up there without the commits being: a squashed
            // or rebased PR. Asked only where ancestry said no and the branch
            // has work at all — and, in the steady state, answered with no
            // fork at all. `merged_in` stays empty for the ordinary merge,
            // an ancestor of the checkout's own default branch, which the
            // ticket page has always called just `merged`; it is filled only
            // where the ref or the commit is news.
            let mut merged_in = String::new();
            let mut merged_oid = String::new();
            let mut seen = None;
            if !ancestor && !tip.oid.is_empty() && tip.oid != i.base_oid {
                let scan = Scan {
                    repo,
                    branch: &i.branch,
                    tip: &tip,
                    target: &target,
                    target_tip: &target_tip,
                    target_is_base: target == base,
                };
                match content_verdict(&scan, i.seen.as_ref(), &mut budget) {
                    Some(verdict) => {
                        if verdict.merged {
                            merged_in = verdict.target.clone();
                            merged_oid = verdict.oid.clone();
                        }
                        seen = Some(verdict);
                    }
                    // Out of budget: carry the old verdict, claim nothing.
                    None => seen = i.seen.clone(),
                }
            }
            let merged = ancestor || !merged_in.is_empty();
            Flags {
                ticket: i.ticket,
                merged,
                ahead,
                needs_rebase: !merged && !ff,
                tip: tip.oid,
                merged_in,
                merged_oid,
                seen,
            }
        })
        .collect();
    let conflicts = list_worktrees(repo).map(|rows| branch_conflicts(&rows)).unwrap_or_default();
    WtFlags { base, base_tip: base_tip.oid, flags, conflicts }
}

/// Fast-forward `base` to the branch tip — the ONLY merge mesimon performs
/// (callers verified `ff_possible`; non-ff goes through the agent-rebase
/// stage instead, so history stays linear and tests ran on the merged state).
///
/// - base checked out in the main checkout: `git merge --ff-only` (git itself
///   refuses when dirty files would be overwritten — surfaced as the reason).
/// - base elsewhere: ff-only ref update, no checkout touched.
pub fn ff_merge(repo: &Path, branch: &str, base: &str) -> Result<()> {
    let head = git_read(repo, &["rev-parse", "--abbrev-ref", "HEAD"])?.trim().to_string();
    if head == base {
        git(repo, &["merge", "--ff-only", branch]).map(|_| ())
    } else {
        let refspec = format!("{branch}:refs/heads/{base}");
        git(repo, &["push", "--quiet", ".", &refspec]).map(|_| ())
    }
}

/// A refusal reason fit for the status line: git's dirty-checkout refusal
/// (multi-line, one path per line) becomes one actionable sentence; anything
/// else is flattened to a single line — the wire must never carry text a
/// one-line footer cannot render. Never EMPTY, since T-289 hangs a standing
/// notice off it: a git that refused without a word still owes the user one.
pub fn merge_refusal_detail(err: &str, base: &str) -> String {
    if err.contains("local changes") || err.contains("would be overwritten") {
        return format!("uncommitted changes in the {base} checkout — commit or stash them first");
    }
    let one_line = err.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.is_empty() {
        return format!("the {base} checkout refused the fast-forward");
    }
    one_line
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

/// The archive's reclaim gate (T-278): does an archived ticket's worktree go?
/// Only a tree that exists to reclaim (`Attached`, or `Evicted` with the
/// branch still to delete — a provision in flight or failed is nobody's to
/// remove), only with a branch, only when its work has LANDED (`merged` is
/// `ticket_merged`'s answer, the card's and the DONE gate's), only with
/// nothing on the ticket holding a pane (the archive gate already refuses
/// an awake session; said again here so the rule reads whole), and never
/// while the bindings file is barred (D26: an unreadable file is no ground
/// for an irreversible move). Unmerged work keeps its worktree.
pub fn reclaim_on_archive(b: &Binding, merged: bool, awake: usize, barred: bool) -> bool {
    matches!(b.status, BindingStatus::Attached | BindingStatus::Evicted)
        && !b.branch.is_empty()
        && merged
        && awake == 0
        && !barred
}

/// Step 5: `-d` (if-merged); the `-D` escalation is a separate explicit call.
pub fn delete_branch(repo: &Path, branch: &str, force: bool) -> Result<()> {
    let flag = if force { "-D" } else { "-d" };
    git(repo, &["branch", flag, branch]).map(|_| ())
}

/// Steps 1/2/4/5 for every leg, then the container: the tree removed where
/// it stands, the branch deleted per `delete` (`Some(false)` is `-d`,
/// `Some(true)` the `-D` a discard confirmed, `None` keeps it). Children go
/// before the root leg — removing the meta's worktree first would take the
/// children's gitfiles with it and leave every child repo a prunable row —
/// and a folder container is a plain directory removed last. Returns
/// whether any leg's branch survived. Callers kill the sessions first
/// (step 0) and never call this while a live process has a leg as cwd.
pub fn teardown(repo_root: &Path, b: &Binding, delete: &dyn Fn(&Leg) -> Option<bool>) -> bool {
    let legs = b.legs(repo_root, "");
    let mut kept = false;
    for leg in legs.iter().rev() {
        if leg.path.is_dir() {
            let _ = remove(&leg.repo, &leg.path);
        }
        if !b.branch.is_empty() {
            if let Some(force) = delete(leg) {
                let _ = delete_branch(&leg.repo, &b.branch, force);
            }
            kept |= !branch_tip(&leg.repo, &b.branch).is_empty();
        }
    }
    if b.is_workspace() && !b.has_root_leg() && b.path.is_dir() {
        std::fs::remove_dir_all(&b.path).ok();
    }
    kept
}

// ---- legs: flags per repo, one answer per ticket ------------------------------

/// One repository's question for a flags pass: the base and upstream it is
/// judged against, and the legs of every ticket that has one there.
#[derive(Debug, Clone)]
pub struct RepoQuery {
    /// `""` is the root.
    pub name: String,
    pub repo: PathBuf,
    pub base: String,
    pub upstream: Option<String>,
    pub inputs: Vec<FlagInput>,
}

/// That repository's answer: `compute_flags` as it was, tagged with the
/// query it answers.
#[derive(Debug, Clone)]
pub struct RepoSample {
    pub name: String,
    pub base: String,
    pub flags: WtFlags,
}

/// `compute_flags` once per repository, on any thread. A single-repo board
/// is one query, and the same `2 + n` forks it always was.
pub fn compute_repo_flags(queries: &[RepoQuery]) -> Vec<RepoSample> {
    queries
        .iter()
        .map(|q| RepoSample {
            name: q.name.clone(),
            base: q.base.clone(),
            flags: compute_flags(&q.repo, &q.base, q.upstream.as_deref(), &q.inputs),
        })
        .collect()
}

/// One leg's flags as the daemon keeps them between passes: the snapshot's
/// per-repo row, `ticket_merged`'s content memo, and `aggregate`'s input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoFlags {
    pub name: String,
    pub base: String,
    pub base_tip: String,
    pub base_oid: String,
    pub tip: String,
    pub ahead: u32,
    pub merged: bool,
    pub needs_rebase: bool,
    pub conflict: bool,
    pub merged_in: String,
    pub merged_oid: String,
    pub content: Option<ContentSeen>,
}

impl RepoFlags {
    /// The branch has moved off the leg's creation base — there is work here
    /// to judge. A branch git no longer has is treated as touched too, so a
    /// lost branch keeps reading as it always did.
    pub fn touched(&self) -> bool {
        self.tip != self.base_oid
    }

    /// One leg's answer out of a sample: the leg, the repository's sample
    /// it was asked in, and the ticket's row there. `content` is the
    /// sampler's memo, carried for the next pass.
    pub fn of(leg: &Leg, sample: &RepoSample, f: &Flags, branch: &str) -> Self {
        RepoFlags {
            name: leg.name.clone(),
            base: leg.base.clone(),
            base_tip: sample.flags.base_tip.clone(),
            base_oid: leg.base_oid.clone(),
            tip: f.tip.clone(),
            ahead: f.ahead,
            merged: f.merged,
            needs_rebase: f.needs_rebase,
            conflict: sample.flags.conflicts.iter().any(|c| c == branch),
            merged_in: f.merged_in.clone(),
            merged_oid: f.merged_oid.clone(),
            content: f.seen.clone(),
        }
    }
}

/// The merge state as the agent tools spell it — `get_ticket`'s
/// `merge_state` and the crown's wake (T-469) — in precedence order: a
/// merged branch is merged whatever else is true of it.
pub fn merge_word(merged: bool, needs_rebase: bool, ahead: u32) -> &'static str {
    if merged {
        "merged"
    } else if needs_rebase {
        "needs_rebase"
    } else if ahead > 0 {
        "ahead"
    } else {
        "clean"
    }
}

/// The ticket's one answer over its legs — what the card, the DONE gate and
/// the train read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Aggregate {
    pub merged: bool,
    pub ahead: u32,
    pub needs_rebase: bool,
    pub conflict: bool,
    pub tip: String,
    pub base_tip: String,
    pub merged_in: String,
    pub merged_oid: String,
}

/// Fold a ticket's legs into one answer. The legs that were TOUCHED judge
/// it: `merged` is every touched leg merged, `needs_rebase` any touched leg
/// behind its base. An untouched leg is silent — on a workspace most legs
/// never see a commit, and one whose base moved on has nothing to rebase.
/// With no leg touched every leg judges, which is exactly the single-leg
/// answer (`compute_flags` calls a fresh branch not merged: no work yet).
/// `ahead` sums every leg; `tip` and `base_tip` join the legs' with `+`, so
/// a change in any leg is a change in the key the train and the refusal
/// memo hang off.
pub fn aggregate(legs: &[RepoFlags]) -> Aggregate {
    let touched: Vec<&RepoFlags> = legs.iter().filter(|l| l.touched()).collect();
    let judged: Vec<&RepoFlags> = if touched.is_empty() { legs.iter().collect() } else { touched };
    let merged = !judged.is_empty() && judged.iter().all(|l| l.merged);
    let landed = judged
        .iter()
        .find(|l| l.name.is_empty() && !l.merged_in.is_empty())
        .or_else(|| judged.iter().find(|l| !l.merged_in.is_empty()));
    Aggregate {
        merged,
        ahead: legs.iter().map(|l| l.ahead).sum(),
        needs_rebase: judged.iter().any(|l| l.needs_rebase),
        conflict: legs.iter().any(|l| l.conflict),
        tip: legs.iter().map(|l| l.tip.as_str()).collect::<Vec<_>>().join("+"),
        base_tip: legs.iter().map(|l| l.base_tip.as_str()).collect::<Vec<_>>().join("+"),
        merged_in: landed.map(|l| l.merged_in.clone()).unwrap_or_default(),
        merged_oid: landed.map(|l| l.merged_oid.clone()).unwrap_or_default(),
    }
}

/// What merging the branch across its legs did, or why it could not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegMerge {
    /// No leg's tip has left its creation base: nothing to merge yet.
    Nothing,
    /// Every touched leg is already an ancestor of its base.
    Already,
    /// Legs whose base moved past them, `(name, base)` each — judged before
    /// anything was written, so nothing was.
    NeedsRebase(Vec<(String, String)>),
    /// Fast-forwarded these legs, `(name, base)` each.
    Merged(Vec<(String, String)>),
    /// Landed `landed`, then `leg` refused with git's words; the legs before
    /// it stay merged and the next call continues from here.
    Refused { landed: Vec<(String, String)>, leg: String, base: String, error: String },
}

/// Merge the branch leg by leg, ff-only, in leg order: every touched,
/// unmerged leg is judged ff-able first (or the answer is `NeedsRebase` and
/// nothing moves), then each is fast-forwarded into its own base. Not
/// atomic across repositories — git has no such thing — so a refusal
/// midway leaves the ticket half landed, and says so.
pub fn merge_legs(legs: &[Leg], branch: &str) -> LegMerge {
    let mut any_touched = false;
    let mut moved = Vec::new();
    let mut pending: Vec<&Leg> = Vec::new();
    for leg in legs {
        let tip = branch_tip(&leg.repo, branch);
        if tip.is_empty() || tip == leg.base_oid {
            continue;
        }
        any_touched = true;
        if is_merged(&leg.repo, branch, &leg.base) {
            continue;
        }
        if !ff_possible(&leg.repo, branch, &leg.base) {
            moved.push((leg.name.clone(), leg.base.clone()));
            continue;
        }
        pending.push(leg);
    }
    if !any_touched {
        return LegMerge::Nothing;
    }
    if !moved.is_empty() {
        return LegMerge::NeedsRebase(moved);
    }
    if pending.is_empty() {
        return LegMerge::Already;
    }
    let mut landed = Vec::new();
    for leg in pending {
        match ff_merge(&leg.repo, branch, &leg.base) {
            Ok(()) => landed.push((leg.name.clone(), leg.base.clone())),
            Err(e) => {
                return LegMerge::Refused {
                    landed,
                    leg: leg.name.clone(),
                    base: leg.base.clone(),
                    error: e.to_string(),
                }
            }
        }
    }
    LegMerge::Merged(landed)
}

/// `(name, base)` pairs as a detail names them: the root's base bare, a
/// child's as `api/main`.
pub fn landed_words(pairs: &[(String, String)]) -> String {
    pairs
        .iter()
        .map(|(n, b)| if n.is_empty() { b.clone() } else { format!("{n}/{b}") })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(status: BindingStatus, branch: &str) -> Binding {
        Binding {
            path: PathBuf::from("/nowhere"),
            branch: branch.into(),
            base_oid: String::new(),
            branch_oid: String::new(),
            status,
            locked: false,
            repos: Vec::new(),
        }
    }

    #[test]
    fn archive_reclaims_only_a_landed_quiet_tree() {
        let attached = binding(BindingStatus::Attached, "msmn/T-1-x");
        assert!(reclaim_on_archive(&attached, true, 0, false), "merged, quiet: goes");
        assert!(!reclaim_on_archive(&attached, false, 0, false), "unmerged work stays");
        assert!(!reclaim_on_archive(&attached, true, 1, false), "a pane holds it");
        assert!(!reclaim_on_archive(&attached, true, 0, true), "barred bindings: nothing moves");
        // Evicted (dir already gone) still has a merged branch to delete.
        assert!(reclaim_on_archive(&binding(BindingStatus::Evicted, "msmn/T-1-x"), true, 0, false));
        // Nothing to reclaim: no branch, or a provision that is not a tree yet.
        assert!(!reclaim_on_archive(&binding(BindingStatus::Attached, ""), true, 0, false));
        for status in [
            BindingStatus::Queued,
            BindingStatus::Provisioning,
            BindingStatus::Error { stage: "add".into(), message: "x".into() },
        ] {
            assert!(!reclaim_on_archive(&binding(status, "msmn/T-1-x"), true, 0, false));
        }
    }

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

    /// The recovery that makes a lost worktrees.json survivable: git still
    /// knows the path and branch, and our marker still names the ticket.
    #[test]
    fn rebuild_from_disk_recovers_bindings_from_markers() {
        let Some(repo) = scratch_repo("rebuild") else { return };
        let root = repo.join("_wtroot");
        std::fs::create_dir_all(&root).unwrap();
        let b = provision(&repo, &root, ulid::Ulid(1), "T-1", "Fix thing").unwrap();

        let rebuilt = rebuild_from_disk(&repo, &repo.join("_wtroot"));
        let got = rebuilt.get(&ulid::Ulid(1)).expect("ticket recovered from its marker");
        assert_eq!(got.branch, b.branch);
        assert_eq!(got.path, b.path);
        assert_eq!(got.status, BindingStatus::Attached);
        assert!(!got.base_oid.is_empty(), "merge-base recovers the diff base");
        std::fs::remove_dir_all(&repo).ok();
    }

    /// A worktree that is not ours is never claimed (D26, fail closed): no
    /// marker, no adoption, even inside our own repo.
    #[test]
    fn rebuild_ignores_worktrees_without_our_marker() {
        let Some(repo) = scratch_repo("rebuildforeign") else { return };
        let foreign = repo.join("_foreign");
        let ok = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["worktree", "add", "--quiet", foreign.to_str().unwrap(), "-b", "msmn/hand-made"])
            .output()
            .unwrap();
        assert!(ok.status.success(), "{}", String::from_utf8_lossy(&ok.stderr));
        assert!(rebuild_from_disk(&repo, &repo.join("_wtroot")).is_empty(), "no marker, no claim");
        std::fs::remove_dir_all(&repo).ok();
    }

    /// Every existing user's worktrees.json is a bare ULID-keyed map; it must
    /// keep loading, and what we write must load back.
    #[test]
    fn bindings_accept_legacy_and_versioned_shapes() {
        let legacy = r#"{"00000000000000000000000001":{"path":"/tmp/x","branch":"msmn/T-1-x",
            "base_oid":"a","branch_oid":"b","status":{"status":"attached"}}}"#;
        let b = parse_bindings(legacy).expect("legacy bare map still loads");
        assert_eq!(b.len(), 1);

        let versioned = serde_json::to_string(&BindingsFile {
            schema_version: BINDINGS_SCHEMA,
            bindings: b.clone(),
        })
        .unwrap();
        assert_eq!(parse_bindings(&versioned).unwrap().len(), 1);

        // A newer file is refused with its version, not treated as garbage.
        let future = versioned
            .replace(&format!("\"schema_version\":{BINDINGS_SCHEMA}"), "\"schema_version\":99");
        assert_eq!(parse_bindings(&future).unwrap_err().0, Some(99));
    }

    #[test]
    fn reconcile_interrupted_settles_stuck_statuses() {
        let Some(repo) = scratch_repo("reconcile") else { return };
        let root = repo.join("_wtroot");
        std::fs::create_dir_all(&root).unwrap();

        // Fully-landed provision, but the binding was persisted still
        // Provisioning (daemon died between the worktree add and the ack).
        let done = provision(&repo, &root, ulid::Ulid(1), "T-1", "Landed").unwrap();
        let mut bindings = Bindings::new();
        bindings
            .insert(ulid::Ulid(1), Binding { status: BindingStatus::Provisioning, ..done.clone() });
        // Branch exists, dir never landed → replay via the evicted path.
        let evicted = provision(&repo, &root, ulid::Ulid(2), "T-2", "Half").unwrap();
        remove(&repo, &evicted.path).unwrap();
        bindings
            .insert(ulid::Ulid(2), Binding { status: BindingStatus::Queued, ..evicted.clone() });
        // Nothing landed (queue_provision's placeholder) → binding drops.
        bindings.insert(
            ulid::Ulid(3),
            Binding {
                path: PathBuf::new(),
                branch: String::new(),
                base_oid: String::new(),
                branch_oid: String::new(),
                status: BindingStatus::Queued,
                locked: false,
                repos: Vec::new(),
            },
        );
        // Dir exists but is not ours (no marker) → fail closed.
        let foreign = root.join("foreign");
        std::fs::create_dir_all(&foreign).unwrap();
        bindings.insert(
            ulid::Ulid(4),
            Binding {
                path: foreign,
                branch: "msmn/T-4-x".into(),
                base_oid: String::new(),
                branch_oid: String::new(),
                status: BindingStatus::Provisioning,
                locked: false,
                repos: Vec::new(),
            },
        );
        // Attached stays untouched.
        let ok = provision(&repo, &root, ulid::Ulid(5), "T-5", "Fine").unwrap();
        bindings.insert(ulid::Ulid(5), ok.clone());

        assert!(reconcile_interrupted(&repo, &mut bindings));
        assert_eq!(bindings[&ulid::Ulid(1)].status, BindingStatus::Attached);
        assert_eq!(bindings[&ulid::Ulid(2)].status, BindingStatus::Evicted);
        assert!(!bindings.contains_key(&ulid::Ulid(3)));
        assert!(matches!(bindings[&ulid::Ulid(4)].status, BindingStatus::Error { .. }));
        assert_eq!(bindings[&ulid::Ulid(5)].status, BindingStatus::Attached);
        // Second pass is a no-op.
        assert!(!reconcile_interrupted(&repo, &mut bindings));

        std::fs::remove_dir_all(&repo).ok();
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
        assert!(ff_possible(&repo, &b.branch, "main"));
        ff_merge(&repo, &b.branch, "main").unwrap();
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

        // Main moved → no fast-forward → the agent-rebase stage, never a
        // merge commit or an in-repo conflict.
        assert!(!ff_possible(&repo, &b.branch, "main"));
        assert!(ff_merge(&repo, &b.branch, "main").is_err());

        // Dirty worktree audit counts the stray file.
        std::fs::write(b.path.join("stray.txt"), "x").unwrap();
        let (n, summary) = audit(&b.path).unwrap();
        assert_eq!(n, 1, "{summary}");

        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn dirty_main_checkout_refusal_maps_to_one_line() {
        // Pure mapping: git's multi-line dirty refusal → one actionable
        // sentence; any other error still flattens to a single line.
        let raw = "git merge failed: error: Your local changes to the following files \
                   would be overwritten by merge:\n\ta.txt\nPlease commit your changes \
                   or stash them before you merge.\nAborting";
        let d = merge_refusal_detail(raw, "main");
        assert!(!d.contains('\n') && !d.contains('\t'));
        assert_eq!(d, "uncommitted changes in the main checkout — commit or stash them first");
        assert_eq!(merge_refusal_detail("boom\nline two", "main"), "boom line two");
        assert_eq!(
            merge_refusal_detail("  \n ", "main"),
            "the main checkout refused the fast-forward",
            "a wordless refusal still says something: the notice hangs off this"
        );

        // The real thing: ff-able branch, dirty base checkout on overlapping
        // files — git refuses the ff, the mapped detail is the sentence.
        let Some(repo) = scratch_repo("dirtymain") else { return };
        let root = repo.join("_wtroot");
        std::fs::create_dir_all(&root).unwrap();
        let b = provision(&repo, &root, ulid::Ulid(9), "T-9", "dirty").unwrap();
        let run = |d: &Path, args: &[&str]| {
            let ok = Command::new("git").arg("-C").arg(d).args(args).output().unwrap();
            assert!(ok.status.success(), "{}", String::from_utf8_lossy(&ok.stderr));
        };
        std::fs::write(b.path.join("a.txt"), "branch side\n").unwrap();
        run(&b.path, &["commit", "-aqm", "branch change"]);
        std::fs::write(repo.join("a.txt"), "uncommitted main edit\n").unwrap();
        assert!(ff_possible(&repo, &b.branch, "main"));
        let err = ff_merge(&repo, &b.branch, "main").unwrap_err();
        let mapped = merge_refusal_detail(&err.to_string(), "main");
        assert!(mapped.contains("uncommitted changes"), "unexpected: {mapped}");

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

    /// T-225: the remote's HEAD is read off the remote the branch tracks —
    /// or the only remote there is — not a literal `origin`. A clone with
    /// `-o gitlab` whose origin defaults to `trunk` while a local `main` also
    /// exists used to answer `main`.
    #[test]
    fn default_branch_reads_the_head_of_the_remote_the_checkout_uses() {
        let Some(bare_src) = scratch_repo("dfltsrc") else { return };
        let run = |d: &Path, args: &[&str]| {
            let out = Command::new("git").arg("-C").arg(d).args(args).output().unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8_lossy(&out.stdout).into_owned()
        };
        // The source's default branch is `trunk`; `main` exists there too.
        run(&bare_src, &["branch", "-m", "main", "trunk"]);
        run(&bare_src, &["branch", "main"]);
        let clone = bare_src.with_file_name("msmn-wt-dfltclone");
        std::fs::remove_dir_all(&clone).ok();
        run(
            &bare_src,
            &["clone", "-q", "-o", "gitlab", bare_src.to_str().unwrap(), clone.to_str().unwrap()],
        );
        assert_eq!(default_branch(&clone).unwrap(), "trunk", "the remote's HEAD, via `gitlab`");
        // With no tracking on the checked-out branch, the sole remote is asked.
        run(&clone, &["checkout", "-q", "-b", "local-only"]);
        assert_eq!(default_branch(&clone).unwrap(), "trunk");
        // No remote HEAD anywhere: the ladder falls to a local `main`.
        run(&clone, &["branch", "main"]);
        run(&clone, &["remote", "rm", "gitlab"]);
        assert_eq!(default_branch(&clone).unwrap(), "main");
        std::fs::remove_dir_all(&clone).ok();
        std::fs::remove_dir_all(&bare_src).ok();
    }

    /// A shell over one scratch repo: run git, commit a file, ask the flags.
    struct Repo {
        dir: PathBuf,
    }

    impl Repo {
        fn new(name: &str) -> Option<Self> {
            scratch_repo(name).map(|dir| Repo { dir })
        }
        fn run(&self, args: &[&str]) {
            let out = Command::new("git").arg("-C").arg(&self.dir).args(args).output().unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        }
        fn commit(&self, file: &str, body: &str, msg: &str) {
            std::fs::write(self.dir.join(file), body).unwrap();
            self.run(&["add", "."]);
            self.run(&["commit", "-qm", msg]);
        }
        /// The flags for one branch, judged against `main` (and an upstream
        /// where the test made one), starting from `seen`.
        fn flags(&self, branch: &str, upstream: Option<&str>, seen: Option<ContentSeen>) -> Flags {
            let base_oid = branch_tip(&self.dir, "main");
            let inputs = vec![FlagInput {
                ticket: ulid::Ulid(1),
                branch: branch.to_string(),
                base_oid,
                seen,
            }];
            let got = compute_flags(&self.dir, "main", upstream, &inputs);
            got.flags.into_iter().next().expect("one binding, one answer")
        }
    }

    impl Drop for Repo {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.dir).ok();
        }
    }

    /// The ticket's case (T-267): the PR was squashed, so not one of the
    /// branch's commits is an ancestor of anything — and the work is still up
    /// there. `is_merged` says no and must; the flags say yes and name the
    /// commit that carries it.
    #[test]
    fn a_squash_merged_branch_reads_merged() {
        let Some(r) = Repo::new("squash") else { return };
        // Two commits on the branch, so the squash is a real one.
        r.run(&["checkout", "-q", "-b", "work"]);
        r.commit("b.txt", "one\n", "b");
        r.commit("c.txt", "two\n", "c");
        // A branch nobody merged, cut from the same place.
        r.run(&["checkout", "-q", "main"]);
        r.run(&["checkout", "-q", "-b", "other"]);
        r.commit("d.txt", "other\n", "d");
        r.run(&["checkout", "-q", "main"]);
        r.run(&["merge", "-q", "--squash", "work"]);
        r.run(&["commit", "-qm", "work (#12)"]);

        assert!(!is_merged(&r.dir, "work", "main"), "a squash leaves no ancestor behind");
        let landed = content_merged(&r.dir, "work", "main", 0).expect("the patch is on main");
        assert_eq!(landed, branch_tip(&r.dir, "main"), "the squash commit is what carries it");

        let f = r.flags("work", None, None);
        assert!(f.merged, "the branch reads merged");
        assert_eq!(f.merged_in, "main");
        assert_eq!(f.merged_oid, landed);
        assert!(!f.needs_rebase, "and stops asking for a rebase");
        assert!(f.ahead > 0, "its own commits are still ahead — that is the point");

        let other = r.flags("other", None, None);
        assert!(!other.merged && other.needs_rebase, "the branch nobody merged is untouched");
        assert!(content_merged(&r.dir, "other", "main", 0).is_none());
    }

    /// The other forge button: every commit replayed on the base. The
    /// combined patch matches nothing there, so the per-commit road is what
    /// answers.
    #[test]
    fn a_rebase_merged_branch_reads_merged() {
        let Some(r) = Repo::new("rebase-merge") else { return };
        r.run(&["checkout", "-q", "-b", "work"]);
        r.commit("b.txt", "one\n", "b");
        r.commit("c.txt", "two\n", "c");
        let picks = [
            branch_tip(&r.dir, "work"),
            String::from_utf8_lossy(
                &Command::new("git")
                    .arg("-C")
                    .arg(&r.dir)
                    .args(["rev-parse", "work~1"])
                    .output()
                    .unwrap()
                    .stdout,
            )
            .trim()
            .to_string(),
        ];
        r.run(&["checkout", "-q", "main"]);
        // Main moves first, which is what makes the replayed commits new
        // objects rather than the very ones the branch holds.
        r.commit("z.txt", "elsewhere\n", "z");
        r.run(&["cherry-pick", &picks[1]]);
        r.run(&["cherry-pick", &picks[0]]);

        assert!(!is_merged(&r.dir, "work", "main"));
        assert!(content_merged(&r.dir, "work", "main", 0).is_some(), "each patch is up there");
        assert!(r.flags("work", None, None).merged);
    }

    /// The busy-base case (T-438): main edits a line within three of the
    /// branch's own before the PR lands. The squash commit's diff then carries
    /// main's text as context, and a patch-id taken WITH context is a
    /// different id — the ticket read unmerged forever. Only the changed
    /// lines may be hashed.
    #[test]
    fn a_squash_on_a_base_that_moved_nearby_reads_merged() {
        let Some(r) = Repo::new("squash-drift") else { return };
        let lines = |a: &str, b: &str, c: &str| {
            (1..=30)
                .map(|n| match n {
                    10 => a.to_string(),
                    12 => b.to_string(),
                    20 => c.to_string(),
                    n => n.to_string(),
                })
                .collect::<Vec<_>>()
                .join("\n")
                + "\n"
        };
        r.commit("f.txt", &lines("10", "12", "20"), "seed");
        r.run(&["checkout", "-q", "-b", "work"]);
        r.commit("f.txt", &lines("ten", "12", "20"), "ten");
        r.commit("f.txt", &lines("ten", "12", "twenty"), "twenty");
        r.run(&["checkout", "-q", "main"]);
        // Two lines from the branch's edit: inside its context, outside its hunk.
        r.commit("f.txt", &lines("10", "twelve", "20"), "main moved nearby");
        r.run(&["merge", "-q", "--squash", "work"]);
        r.run(&["commit", "-qm", "work (!7)"]);

        assert!(!is_merged(&r.dir, "work", "main"));
        let landed = content_merged(&r.dir, "work", "main", 0).expect("the patch is on main");
        assert_eq!(landed, branch_tip(&r.dir, "main"));
        let f = r.flags("work", None, None);
        assert!(f.merged && !f.needs_rebase, "the branch reads merged");

        // The same drift on the rebase-merge road: each commit replayed onto
        // the moved base, each carrying the base's context.
        r.run(&["checkout", "-q", "-b", "work2", "main~2"]);
        r.commit("f.txt", &lines("TEN", "12", "20"), "TEN");
        r.commit("f.txt", &lines("TEN", "12", "TWENTY"), "TWENTY");
        let tip = branch_tip(&r.dir, "work2");
        r.run(&["checkout", "-q", "main"]);
        r.run(&["reset", "-q", "--hard", "main~1"]);
        r.run(&["cherry-pick", "work2~1"]);
        r.run(&["cherry-pick", &tip]);
        assert!(!is_merged(&r.dir, "work2", "main"));
        assert!(content_merged(&r.dir, "work2", "main", 0).is_some(), "each patch is up there");
    }

    /// The durability the memo buys: the base runs on over the very files the
    /// branch touched, and the ticket stays merged. Cold (no memo) it is the
    /// window that must still hold it; warm it is one `--is-ancestor` on the
    /// commit already named, whatever the target's tip is now.
    #[test]
    fn a_squash_stays_merged_as_the_base_runs_on() {
        let Some(r) = Repo::new("durable") else { return };
        r.run(&["checkout", "-q", "-b", "work"]);
        r.commit("b.txt", "one\n", "b");
        r.run(&["checkout", "-q", "main"]);
        r.run(&["merge", "-q", "--squash", "work"]);
        r.run(&["commit", "-qm", "work (#12)"]);
        let first = r.flags("work", None, None);
        assert!(first.merged && !first.merged_oid.is_empty());

        for n in 0..3 {
            r.commit("b.txt", &format!("one\nand {n}\n"), &format!("after {n}"));
        }
        let cold = r.flags("work", None, None);
        assert!(cold.merged, "the squash is still in the base's history");
        let warm = r.flags("work", None, first.seen.clone());
        assert!(warm.merged, "and the verdict re-affirms without a scan");
        assert_eq!(warm.merged_oid, first.merged_oid);
    }

    /// A verdict is reused while nothing moved, and dropped the moment the
    /// branch does: work committed after the merge is work that has not
    /// landed.
    #[test]
    fn a_commit_after_the_merge_reads_unmerged_again() {
        let Some(r) = Repo::new("moved") else { return };
        r.run(&["checkout", "-q", "-b", "work"]);
        r.commit("b.txt", "one\n", "b");
        r.run(&["checkout", "-q", "main"]);
        r.run(&["merge", "-q", "--squash", "work"]);
        r.run(&["commit", "-qm", "work (#12)"]);
        let merged = r.flags("work", None, None);
        assert!(merged.merged);
        assert_eq!(
            r.flags("work", None, merged.seen.clone()).seen,
            merged.seen,
            "reused as it was"
        );

        r.run(&["checkout", "-q", "work"]);
        r.commit("e.txt", "more\n", "e");
        r.run(&["checkout", "-q", "main"]);
        let after = r.flags("work", None, merged.seen);
        assert!(!after.merged, "the branch has work the base does not");
    }

    /// The whole point of the upstream half: the PR is merged on the forge
    /// and the user only FETCHES. Local `main` never moves, and the ticket
    /// still reads merged — into `origin/main`, which is what the page says.
    #[test]
    fn an_upstream_squash_is_found_after_a_fetch() {
        let Some(r) = Repo::new("upstream") else { return };
        let bare =
            r.dir.parent().unwrap().join(format!("msmn-wt-upstream-{}.git", std::process::id()));
        std::fs::remove_dir_all(&bare).ok();
        let init = Command::new("git")
            .args(["init", "-q", "--bare", "-b", "main"])
            .arg(&bare)
            .output()
            .unwrap();
        assert!(init.status.success());
        r.run(&["remote", "add", "origin", &bare.display().to_string()]);
        r.run(&["push", "-q", "-u", "origin", "main"]);
        r.run(&["checkout", "-q", "-b", "work"]);
        r.commit("b.txt", "one\n", "b");
        // The forge's squash: made away from this checkout and pushed, so
        // local `main` is exactly where the user left it.
        r.run(&["checkout", "-q", "-b", "pr", "main"]);
        r.run(&["merge", "-q", "--squash", "work"]);
        r.run(&["commit", "-qm", "work (#12)"]);
        r.run(&["push", "-q", "origin", "pr:main"]);
        r.run(&["checkout", "-q", "main"]);
        r.run(&["branch", "-qD", "pr"]);
        r.run(&["fetch", "-q", "origin"]);

        assert_eq!(upstream_base(&r.dir, "main").as_deref(), Some("origin/main"));
        assert!(!is_merged(&r.dir, "work", "main"), "local main knows nothing about it");
        let f = r.flags("work", Some("origin/main"), None);
        assert!(f.merged, "but origin/main carries the patch");
        assert_eq!(f.merged_in, "origin/main");
        assert!(!f.merged_oid.is_empty());
        assert!(
            !r.flags("work", None, None).merged,
            "and without the upstream there is nothing to find: local main never moved"
        );
        std::fs::remove_dir_all(&bare).ok();
    }

    /// A fetch stales every memo at once, so a pass scans a couple of
    /// bindings and leaves the rest for the next one — and the ones it left
    /// keep NO verdict, or they would look answered and never be asked again.
    #[test]
    fn a_pass_scans_a_budget_of_bindings_and_the_rest_wait() {
        let Some(r) = Repo::new("budget") else { return };
        let branches = ["one", "two", "three"];
        for b in branches {
            r.run(&["checkout", "-q", "-b", b, "main"]);
            r.commit(&format!("{b}.txt"), b, b);
            r.run(&["checkout", "-q", "main"]);
            r.run(&["merge", "-q", "--squash", b]);
            r.run(&["commit", "-qm", &format!("{b} (#1)")]);
        }
        let base_oid = branch_tip(&r.dir, "main");
        let inputs: Vec<FlagInput> = branches
            .iter()
            .enumerate()
            .map(|(i, b)| FlagInput {
                ticket: ulid::Ulid(i as u128 + 1),
                branch: b.to_string(),
                base_oid: base_oid.clone(),
                seen: None,
            })
            .collect();
        let first = compute_flags(&r.dir, "main", None, &inputs);
        let merged = first.flags.iter().filter(|f| f.merged).count();
        assert_eq!(
            merged, CONTENT_SCANS_PER_PASS as usize,
            "the pass spends its budget and no more"
        );
        assert!(
            first.flags.iter().any(|f| !f.merged && f.seen.is_none()),
            "and the one it skipped remembers nothing"
        );
        // The next pass, carrying what the first learned, finishes the job.
        let inputs: Vec<FlagInput> = inputs
            .into_iter()
            .zip(&first.flags)
            .map(|(i, f)| FlagInput { seen: f.seen.clone(), ..i })
            .collect();
        let second = compute_flags(&r.dir, "main", None, &inputs);
        assert!(
            second.flags.iter().all(|f| f.merged),
            "every branch is merged: {:?}",
            second.flags
        );
    }

    /// Nothing to find, and nothing to crash on.
    #[test]
    fn content_merge_says_no_where_there_is_nothing_to_say_yes_to() {
        let Some(r) = Repo::new("nothing") else { return };
        r.run(&["checkout", "-q", "-b", "empty"]);
        assert!(content_merged(&r.dir, "empty", "main", 0).is_none(), "no work, no patch");
        assert!(content_merged(&r.dir, "work-that-is-not-there", "main", 0).is_none());
        assert!(content_merged(&r.dir, "main", "refs/remotes/origin/nope", 0).is_none());
    }

    /// `compute_flags` says what the four single-question helpers said, per
    /// binding, in one sample: fresh is not merged, a commit is ahead, an
    /// ancestor that moved is merged, a base that moved past needs a
    /// rebase, and a branch git lost is nothing ahead and no fast-forward.
    #[test]
    fn compute_flags_agrees_with_the_single_question_helpers() {
        let Some(repo) = scratch_repo("flags") else { return };
        let run = |args: &[&str]| {
            let ok = Command::new("git").arg("-C").arg(&repo).args(args).output().unwrap();
            assert!(ok.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&ok.stderr));
        };
        let old_tip = branch_tip(&repo, "main");
        run(&["branch", "work"]);
        run(&["branch", "landed"]);
        run(&["branch", "stale"]);
        for b in ["work", "landed"] {
            run(&["checkout", "-q", b]);
            std::fs::write(repo.join(format!("{b}.txt")), b).unwrap();
            run(&["add", "."]);
            run(&["commit", "-qm", b]);
        }
        run(&["checkout", "-q", "main"]);
        run(&["merge", "-q", "--ff-only", "landed"]);
        // Cut from the tip main now stands on: no work, nothing to rebase.
        run(&["branch", "fresh"]);
        let new_tip = branch_tip(&repo, "main");
        let inputs: Vec<FlagInput> = ["fresh", "work", "landed", "stale", "gone"]
            .iter()
            .enumerate()
            .map(|(i, b)| FlagInput {
                ticket: ulid::Ulid(i as u128 + 1),
                branch: b.to_string(),
                base_oid: if *b == "fresh" { new_tip.clone() } else { old_tip.clone() },
                seen: None,
            })
            .collect();
        let got = compute_flags(&repo, "main", None, &inputs);
        assert_eq!(got.base_tip, branch_tip(&repo, "main"));
        assert!(!got.base_tip.is_empty());
        for (f, i) in got.flags.iter().zip(&inputs) {
            assert_eq!(f.ticket, i.ticket);
            let tip = branch_tip(&repo, &i.branch);
            // Ancestry is not the only way to be merged since T-267, so the
            // oracle is both questions — none of these five branches was
            // squashed, so the second one answers no throughout.
            let merged = !tip.is_empty()
                && tip != i.base_oid
                && (is_merged(&repo, &i.branch, "main")
                    || content_merged(&repo, &i.branch, "main", 0).is_some());
            assert_eq!(f.merged, merged, "{} merged", i.branch);
            assert_eq!(f.ahead, ahead_count(&repo, &i.branch, "main"), "{} ahead", i.branch);
            let needs = !merged && !ff_possible(&repo, &i.branch, "main");
            assert_eq!(f.needs_rebase, needs, "{} needs_rebase", i.branch);
        }
        let by =
            |name: &str| got.flags[inputs.iter().position(|i| i.branch == name).unwrap()].clone();
        assert!(!by("fresh").merged && by("fresh").ahead == 0 && !by("fresh").needs_rebase);
        assert!(!by("work").merged && by("work").ahead == 1 && by("work").needs_rebase);
        assert!(by("landed").merged && by("landed").ahead == 0);
        assert!(!by("stale").merged && by("stale").ahead == 0 && by("stale").needs_rebase);
        assert!(!by("gone").merged && by("gone").ahead == 0 && by("gone").needs_rebase);
        assert!(got.conflicts.is_empty());
        std::fs::remove_dir_all(&repo).ok();
    }

    // ---- workspace worktrees: one leg per repo (T-368) ------------------------

    use crate::testrepo::{self, workspace_scratch};

    fn ws_binding(name: &str) -> Binding {
        Binding {
            path: PathBuf::from("/wt/T-1-x"),
            branch: "msmn/T-1-x".into(),
            base_oid: "r0".into(),
            branch_oid: "r0".into(),
            status: BindingStatus::Attached,
            locked: false,
            repos: vec![
                RepoBinding {
                    name: String::new(),
                    base: "master".into(),
                    base_oid: "r0".into(),
                    branch_oid: "r0".into(),
                },
                RepoBinding {
                    name: name.into(),
                    base: "main".into(),
                    base_oid: "a0".into(),
                    branch_oid: "a1".into(),
                },
            ],
        }
    }

    #[test]
    fn legs_of_a_single_binding_is_one_root_leg() {
        let b = binding(BindingStatus::Attached, "msmn/T-1-x");
        let legs = b.legs(Path::new("/repo"), "main");
        assert_eq!(legs.len(), 1);
        assert_eq!(legs[0].name, "");
        assert_eq!(legs[0].repo, Path::new("/repo"));
        assert_eq!(legs[0].path, Path::new("/nowhere"));
        assert_eq!(legs[0].base, "main", "the daemon's base, not the binding's");
        assert!(!b.is_workspace() && b.has_root_leg());
        assert_eq!(leg_word(""), "root");
        assert_eq!(leg_word("api"), "api");
    }

    #[test]
    fn legs_of_a_workspace_binding_map_root_and_children() {
        let b = ws_binding("api");
        let legs = b.legs(Path::new("/repo"), "ignored");
        assert_eq!(legs.len(), 2);
        assert_eq!(legs[0].name, "");
        assert_eq!(legs[0].repo, Path::new("/repo"));
        assert_eq!(legs[0].path, Path::new("/wt/T-1-x"));
        assert_eq!(legs[0].base, "master");
        assert_eq!(legs[1].name, "api");
        assert_eq!(legs[1].repo, Path::new("/repo/api"));
        assert_eq!(legs[1].path, Path::new("/wt/T-1-x/api"));
        assert_eq!((legs[1].base.as_str(), legs[1].base_oid.as_str()), ("main", "a0"));
        assert!(b.is_workspace() && b.has_root_leg());
        let mut folder = b.clone();
        folder.repos.remove(0);
        assert!(!folder.has_root_leg());
    }

    /// Schema 2 round-trips its legs; a single-repo binding is written
    /// without a `repos` key at all, so the file reads as it always did.
    #[test]
    fn schema_two_carries_legs_and_leaves_them_off_a_single_binding() {
        let single = binding(BindingStatus::Attached, "msmn/T-1-x");
        let text = serde_json::to_string(&single).unwrap();
        assert!(!text.contains("repos"), "{text}");
        let ws = ws_binding("api");
        let mut bindings = Bindings::new();
        bindings.insert(ulid::Ulid(1), ws.clone());
        let file =
            serde_json::to_string(&BindingsFile { schema_version: BINDINGS_SCHEMA, bindings })
                .unwrap();
        assert!(file.contains("\"schema_version\":2"));
        let back = parse_bindings(&file).unwrap();
        assert_eq!(back[&ulid::Ulid(1)].repos, ws.repos);
        // A schema-1 wrapper still loads, legless.
        let one = file
            .replace("\"schema_version\":2", "\"schema_version\":1")
            .replace(&serde_json::to_string(&ws.repos).unwrap(), "[]");
        assert!(parse_bindings(&one).unwrap()[&ulid::Ulid(1)].repos.is_empty());
    }

    fn ws_provision(
        root: &Path,
        is_meta: bool,
        progress: &mut dyn FnMut(u32, u32),
    ) -> std::result::Result<Binding, (String, String)> {
        let wtroot = root.join("_wtroot");
        std::fs::create_dir_all(&wtroot).unwrap();
        let census = crate::gitstatus::census(root);
        provision_workspace(root, &wtroot, &census, is_meta, ulid::Ulid(1), "T-1", "Fix", progress)
    }

    #[test]
    fn workspace_provision_cuts_every_census_repo() {
        let Some(root) = workspace_scratch("wsprov") else { return };
        let mut seen = Vec::new();
        let b = ws_provision(&root, true, &mut |d, t| seen.push((d, t))).unwrap();
        assert_eq!(seen, vec![(1, 3), (2, 3), (3, 3)]);
        assert_eq!(b.branch, "msmn/T-1-fix");
        assert_eq!(b.status, BindingStatus::Attached);
        let names: Vec<&str> = b.repos.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["", "api", "web"], "root first, then census order");
        for leg in b.legs(&root, "") {
            assert!(leg.path.is_dir(), "{}", leg.path.display());
            assert_eq!(marker_ticket(&leg.path), Some(ulid::Ulid(1)), "{}", leg.name);
            assert_eq!(leg.base, "main");
            assert_eq!(leg.base_oid, branch_tip(&leg.repo, "main"));
            assert_eq!(
                testrepo::read(&leg.path, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(),
                "msmn/T-1-fix"
            );
        }
        assert!(b.path.join("CLAUDE.md").is_file(), "the meta's files are in the container");
        assert!(b.path.join("api/server.ts").is_file());
        assert!(b.path.join("web/page.tsx").is_file());
        // The gitfile child (a worktree of `web`, sharing its refs) stays on
        // its own branch, and the declared submodule was never touched.
        assert_eq!(
            testrepo::read(&root.join(".wt-web"), &["rev-parse", "--abbrev-ref", "HEAD"]).trim(),
            "feedback"
        );
        assert!(branch_tip(&root.join("vendored"), "msmn/T-1-fix").is_empty());
        assert!(verify_on_disk(ulid::Ulid(1), &b, &root));
        assert!(!verify_on_disk(ulid::Ulid(2), &b, &root));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn workspace_provision_on_a_folder_has_no_root_leg() {
        let Some(root) = workspace_scratch("wsfolder") else { return };
        std::fs::remove_dir_all(root.join(".git")).unwrap();
        let b = ws_provision(&root, false, &mut |_, _| {}).unwrap();
        let names: Vec<&str> = b.repos.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["api", "web"]);
        assert!(!b.has_root_leg());
        assert!(b.base_oid.is_empty() && b.branch_oid.is_empty());
        assert!(b.path.join("CLAUDE.md").is_file(), "the root's CLAUDE.md is copied in");
        assert!(!b.path.join(".git").exists(), "a plain directory, not a worktree");
        assert!(verify_on_disk(ulid::Ulid(1), &b, &root));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn workspace_provision_fails_closed_on_a_detached_child() {
        let Some(root) = workspace_scratch("wsdetached") else { return };
        testrepo::run(&root.join("web"), &["checkout", "-q", "--detach"]);
        let (stage, message) = ws_provision(&root, true, &mut |_, _| {}).unwrap_err();
        assert_eq!(stage, "precheck");
        assert!(message.starts_with("web "), "{message}");
        assert!(message.contains("detached"), "{message}");
        assert!(
            std::fs::read_dir(root.join("_wtroot")).unwrap().next().is_none(),
            "nothing written"
        );
        for leg in ["", "api", "web"] {
            assert!(branch_tip(&root.join(leg), "msmn/T-1-fix").is_empty(), "{leg}");
        }
        // A branch already held in one child is refused the same way.
        testrepo::run(&root.join("web"), &["checkout", "-q", "main"]);
        testrepo::run(&root.join("api"), &["branch", "msmn/T-1-fix"]);
        let (stage, message) = ws_provision(&root, true, &mut |_, _| {}).unwrap_err();
        assert_eq!(stage, "precheck");
        assert!(message.ends_with("already exists in api"), "{message}");
        std::fs::remove_dir_all(&root).ok();
    }

    /// A child that fails to add takes the legs already added with it.
    #[test]
    fn workspace_provision_rolls_back_a_failed_child() {
        let Some(root) = workspace_scratch("wsrollback") else { return };
        // The precheck reads `web` fine; its add fails because git cannot
        // write the admin entry under a read-only `worktrees/` dir.
        let admin = root.join("web/.git/worktrees");
        let perm = |mode: u32| {
            std::fs::set_permissions(&admin, std::os::unix::fs::PermissionsExt::from_mode(mode))
                .unwrap()
        };
        perm(0o555);
        let mut seen = Vec::new();
        let err = ws_provision(&root, true, &mut |d, t| seen.push((d, t))).unwrap_err();
        perm(0o755);
        assert_eq!(err.0, "add");
        assert!(err.1.starts_with("web: "), "{}", err.1);
        assert_eq!(seen, vec![(1, 3), (2, 3)], "root and api landed before web failed");
        assert!(
            std::fs::read_dir(root.join("_wtroot")).unwrap().next().is_none(),
            "the container and every leg are gone"
        );
        for leg in ["", "api", "web"] {
            assert!(branch_tip(&root.join(leg), "msmn/T-1-fix").is_empty(), "{leg} branch gone");
            assert!(
                !testrepo::read(&root.join(leg), &["worktree", "list", "--porcelain"])
                    .contains("T-1-fix"),
                "{leg} still lists the tree"
            );
        }
        // And the next attempt succeeds outright.
        assert!(ws_provision(&root, true, &mut |_, _| {}).is_ok());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn rebuild_from_disk_recovers_a_workspace_binding() {
        let Some(root) = workspace_scratch("wsrebuild") else { return };
        let b = ws_provision(&root, true, &mut |_, _| {}).unwrap();
        let rebuilt = rebuild_from_disk(&root, &root.join("_wtroot"));
        let got = rebuilt.get(&ulid::Ulid(1)).expect("the workspace binding is recovered");
        assert_eq!(got.path, b.path);
        assert_eq!(got.branch, b.branch);
        assert_eq!(got.status, BindingStatus::Attached);
        let names: Vec<&str> = got.repos.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["", "api", "web"]);
        for r in &got.repos {
            assert_eq!(r.base, "main");
            assert!(!r.base_oid.is_empty() && !r.branch_oid.is_empty());
        }
        // A folder root: found through the children alone.
        std::fs::remove_dir_all(root.join(".git")).unwrap();
        let rebuilt = rebuild_from_disk(&root, &root.join("_wtroot"));
        let got = rebuilt.get(&ulid::Ulid(1)).unwrap();
        let names: Vec<&str> = got.repos.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["api", "web"]);
        assert_eq!(got.path, b.path);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn reconcile_settles_a_partial_workspace() {
        let Some(root) = workspace_scratch("wsreconcile") else { return };
        let b = ws_provision(&root, true, &mut |_, _| {}).unwrap();
        // Whole on disk, persisted mid-provision → Attached.
        let mut bindings = Bindings::new();
        bindings
            .insert(ulid::Ulid(1), Binding { status: BindingStatus::Provisioning, ..b.clone() });
        assert!(reconcile_interrupted(&root, &mut bindings));
        assert_eq!(bindings[&ulid::Ulid(1)].status, BindingStatus::Attached);

        // `web`'s leg gone, no work anywhere → everything is taken down and
        // the binding drops.
        remove(&root.join("web"), &b.path.join("web")).unwrap();
        bindings
            .insert(ulid::Ulid(1), Binding { status: BindingStatus::Provisioning, ..b.clone() });
        assert!(reconcile_interrupted(&root, &mut bindings));
        assert!(!bindings.contains_key(&ulid::Ulid(1)));
        assert!(!b.path.exists(), "the container went with the legs");
        for leg in ["", "api", "web"] {
            assert!(branch_tip(&root.join(leg), &b.branch).is_empty(), "{leg} branch deleted");
        }

        // Provision again, commit on `api`'s leg, lose `web`'s tree: the
        // legs go but `api`'s branch carries work and stays → Evicted.
        let b = ws_provision(&root, true, &mut |_, _| {}).unwrap();
        testrepo::commit(&b.path.join("api"), "new.ts", "x\n", "work");
        remove(&root.join("web"), &b.path.join("web")).unwrap();
        bindings.insert(ulid::Ulid(1), Binding { status: BindingStatus::Queued, ..b.clone() });
        assert!(reconcile_interrupted(&root, &mut bindings));
        assert_eq!(bindings[&ulid::Ulid(1)].status, BindingStatus::Evicted);
        assert!(!branch_tip(&root.join("api"), &b.branch).is_empty(), "api's work survives");
        assert!(branch_tip(&root.join("web"), &b.branch).is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn provision_existing_replays_a_workspace() {
        let Some(root) = workspace_scratch("wsreplay") else { return };
        let b = ws_provision(&root, true, &mut |_, _| {}).unwrap();
        testrepo::commit(&b.path.join("api"), "new.ts", "x\n", "work");
        let kept = teardown(&root, &b, &|_| None);
        assert!(kept, "no branch deleted");
        assert!(!b.path.exists());
        let again = provision_existing(&root, ulid::Ulid(1), &b).unwrap();
        assert_eq!(again.status, BindingStatus::Attached);
        assert_eq!(again.repos.len(), 3);
        for leg in again.legs(&root, "") {
            assert!(leg.path.is_dir(), "{}", leg.name);
            assert_eq!(marker_ticket(&leg.path), Some(ulid::Ulid(1)));
        }
        assert!(again.path.join("api/new.ts").is_file(), "the work came back");
        let api = again.repos.iter().find(|r| r.name == "api").unwrap();
        assert_eq!(api.branch_oid, branch_tip(&root.join("api"), &b.branch), "tip re-read");
        assert_ne!(api.branch_oid, api.base_oid);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn workspace_teardown_removes_every_leg_and_the_container() {
        let Some(root) = workspace_scratch("wsteardown") else { return };
        let b = ws_provision(&root, true, &mut |_, _| {}).unwrap();
        testrepo::commit(&b.path.join("api"), "new.ts", "x\n", "work");
        ff_merge(&root.join("api"), &b.branch, "main").unwrap();
        let kept = teardown(&root, &b, &|leg| {
            Some(!(leg.name == "api" || branch_tip(&leg.repo, "msmn/T-1-fix") == leg.base_oid))
        });
        assert!(!kept);
        assert!(!b.path.exists());
        for leg in ["", "api", "web"] {
            assert!(branch_tip(&root.join(leg), &b.branch).is_empty(), "{leg}");
            assert!(
                !testrepo::read(&root.join(leg), &["worktree", "list", "--porcelain"])
                    .contains("T-1-fix"),
                "{leg} still lists the tree"
            );
        }
        // A folder container is a plain directory, removed last.
        std::fs::remove_dir_all(root.join(".git")).unwrap();
        let b = ws_provision(&root, false, &mut |_, _| {}).unwrap();
        assert!(!teardown(&root, &b, &|_| Some(true)));
        assert!(!b.path.exists());
        std::fs::remove_dir_all(&root).ok();
    }

    fn leg_flags(
        name: &str,
        base_oid: &str,
        tip: &str,
        ahead: u32,
        merged: bool,
        nr: bool,
    ) -> RepoFlags {
        RepoFlags {
            name: name.into(),
            base: "main".into(),
            base_tip: format!("bt-{name}"),
            base_oid: base_oid.into(),
            tip: tip.into(),
            ahead,
            merged,
            needs_rebase: nr,
            conflict: false,
            merged_in: String::new(),
            merged_oid: String::new(),
            content: None,
        }
    }

    /// A single leg's aggregate is that leg, field for field — the
    /// single-repo board reads exactly what it read before.
    #[test]
    fn aggregate_of_one_leg_is_that_leg() {
        for (tip, ahead, merged, nr) in [
            ("b0", 0, false, false),
            ("b1", 1, false, true),
            ("b2", 0, true, false),
            ("", 0, false, true),
        ] {
            let l = leg_flags("", "b0", tip, ahead, merged, nr);
            let a = aggregate(std::slice::from_ref(&l));
            assert_eq!((a.merged, a.ahead, a.needs_rebase), (merged, ahead, nr), "{tip}");
            assert_eq!(a.tip, tip);
            assert_eq!(a.base_tip, "bt-");
        }
    }

    #[test]
    fn aggregate_judges_touched_legs_and_sums_ahead() {
        // `api` has work, `web` is untouched with a moved base: `web` is silent.
        let api = leg_flags("api", "a0", "a3", 3, false, false);
        let web_stale = leg_flags("web", "w0", "w0", 0, false, true);
        let a = aggregate(&[api.clone(), web_stale.clone()]);
        assert!(!a.merged && a.ahead == 3 && !a.needs_rebase);
        assert_eq!(a.tip, "a3+w0");
        assert_eq!(a.base_tip, "bt-api+bt-web");
        // `api` landed, `web` untouched: merged.
        let api_landed = leg_flags("api", "a0", "a3", 0, true, false);
        assert!(aggregate(&[api_landed.clone(), web_stale.clone()]).merged);
        // Two touched, one landed, one behind: not merged, needs a rebase.
        let web_behind = leg_flags("web", "w0", "w1", 1, false, true);
        let a = aggregate(&[api_landed.clone(), web_behind]);
        assert!(!a.merged && a.needs_rebase && a.ahead == 1);
        // Nothing touched: every leg judges, like the single leg.
        let fresh = leg_flags("api", "a0", "a0", 0, false, false);
        let a = aggregate(&[fresh, web_stale]);
        assert!(!a.merged && a.needs_rebase, "single-repo stale parity");
        // `merged_in`: the root's, else the first leg with one.
        let mut root = leg_flags("", "r0", "r1", 1, true, false);
        root.merged_in = "origin/master".into();
        let mut api_in = api_landed;
        api_in.merged_in = "main".into();
        assert_eq!(aggregate(&[api_in.clone(), root]).merged_in, "origin/master");
        assert_eq!(aggregate(&[api_in]).merged_in, "main");
        // A conflict anywhere is the ticket's.
        let mut c = api;
        c.conflict = true;
        assert!(aggregate(&[c]).conflict);
    }

    #[test]
    fn workspace_flags_come_back_per_leg() {
        let Some(root) = workspace_scratch("wsflags") else { return };
        let b = ws_provision(&root, true, &mut |_, _| {}).unwrap();
        testrepo::commit(&b.path.join("api"), "new.ts", "x\n", "work");
        let queries: Vec<RepoQuery> = b
            .legs(&root, "")
            .into_iter()
            .map(|leg| RepoQuery {
                name: leg.name.clone(),
                repo: leg.repo.clone(),
                base: leg.base.clone(),
                upstream: None,
                inputs: vec![FlagInput {
                    ticket: ulid::Ulid(1),
                    branch: b.branch.clone(),
                    base_oid: leg.base_oid.clone(),
                    seen: None,
                }],
            })
            .collect();
        let samples = compute_repo_flags(&queries);
        assert_eq!(samples.len(), 3);
        let legs: Vec<RepoFlags> = samples
            .iter()
            .zip(&queries)
            .map(|(s, q)| {
                let f = &s.flags.flags[0];
                RepoFlags {
                    name: s.name.clone(),
                    base: s.base.clone(),
                    base_tip: s.flags.base_tip.clone(),
                    base_oid: q.inputs[0].base_oid.clone(),
                    tip: f.tip.clone(),
                    ahead: f.ahead,
                    merged: f.merged,
                    needs_rebase: f.needs_rebase,
                    conflict: false,
                    merged_in: f.merged_in.clone(),
                    merged_oid: f.merged_oid.clone(),
                    content: f.seen.clone(),
                }
            })
            .collect();
        assert_eq!(legs[1].name, "api");
        assert_eq!(legs[1].ahead, 1);
        assert!(!legs[0].touched() && !legs[2].touched());
        let a = aggregate(&legs);
        assert!(!a.merged && a.ahead == 1 && !a.needs_rebase, "{a:?}");
        ff_merge(&root.join("api"), &b.branch, "main").unwrap();
        let samples = compute_repo_flags(&queries);
        assert!(samples[1].flags.flags[0].merged);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn workspace_merge_lands_leg_by_leg_and_stops_at_a_refusal() {
        let Some(root) = workspace_scratch("wsmerge") else { return };
        let b = ws_provision(&root, true, &mut |_, _| {}).unwrap();
        let legs = b.legs(&root, "");
        assert_eq!(merge_legs(&legs, &b.branch), LegMerge::Nothing);

        testrepo::commit(&b.path.join("api"), "new.ts", "x\n", "api work");
        testrepo::commit(&b.path.join("web"), "page.tsx", "branch side\n", "web work");
        // `web`'s checkout is dirty on the very file: git refuses that ff.
        std::fs::write(root.join("web/page.tsx"), "uncommitted\n").unwrap();
        let got = merge_legs(&legs, &b.branch);
        match got {
            LegMerge::Refused { landed, leg, base, error } => {
                assert_eq!(landed, vec![("api".to_string(), "main".to_string())]);
                assert_eq!((leg.as_str(), base.as_str()), ("web", "main"));
                assert!(merge_refusal_detail(&error, &base).contains("uncommitted"), "{error}");
            }
            other => panic!("{other:?}"),
        }
        assert!(root.join("api/new.ts").is_file(), "api landed");
        assert!(!is_merged(&root.join("web"), &b.branch, "main"));
        // Clean the checkout: the next call continues where it stopped.
        testrepo::run(&root.join("web"), &["checkout", "-q", "--", "page.tsx"]);
        assert_eq!(
            merge_legs(&legs, &b.branch),
            LegMerge::Merged(vec![("web".to_string(), "main".to_string())])
        );
        assert_eq!(merge_legs(&legs, &b.branch), LegMerge::Already);
        assert_eq!(
            landed_words(&[("".into(), "master".into()), ("api".into(), "main".into())]),
            "master, api/main"
        );

        // `web`'s main moves on and the leg commits again: only `web` is named.
        testrepo::commit(&root.join("web"), "other.tsx", "main moved\n", "main moved");
        testrepo::commit(&b.path.join("web"), "more.tsx", "y\n", "more web work");
        testrepo::commit(&b.path.join("api"), "more.ts", "y\n", "more api work");
        assert_eq!(
            merge_legs(&legs, &b.branch),
            LegMerge::NeedsRebase(vec![("web".to_string(), "main".to_string())])
        );
        assert!(
            !is_merged(&root.join("api"), &b.branch, "main"),
            "nothing moved on a rebase answer"
        );
        std::fs::remove_dir_all(&root).ok();
    }
}
