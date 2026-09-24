//! Read-only diff service (M4b, docs/08 §1.2 plumbing — the flags are
//! load-bearing, every one fails silently if omitted). Runs on connection
//! threads, never the writer: everything here reads the object store and, for
//! the display-only in-flight flags, one `status` call in the worktree.

use std::path::Path;

use anyhow::{bail, Context, Result};
use mesimon_core::command::Response;
use mesimon_core::diff::{
    apply_status_flags, build_file_diff, merge_numstat, parse_numstat_z, parse_raw_z,
    parse_status_v2_z, FileDiff, FileEntry, Render,
};

use crate::worktree::{branch_tip, Binding, BindingStatus};

/// Above this, the pane shows `TooLarge` and `!` is the reader (docs/08 §1.3).
pub const MAX_PATCH_BYTES: u64 = 2 * 1024 * 1024;

/// At most this many untracked rows ride one checkout list. A repository
/// missing a `.gitignore` rule can hand `-uall` tens of thousands of paths,
/// and a response line has no length cap on the client's side (only requests
/// do, `ORCH_LINE_MAX_BYTES`).
pub const MAX_UNTRACKED_ROWS: usize = 2000;

/// git's own binary heuristic: a NUL inside the first 8000 bytes.
const BINARY_SNIFF_BYTES: usize = 8000;

/// The bytes one checkout list may read from disk for its untracked-file
/// badges, in total (T-254). `MAX_UNTRACKED_ROWS` caps the ROWS, and two
/// thousand rows under `MAX_PATCH_BYTES` each is four gigabytes of reads on
/// a connection thread for one `v` press; past this the row keeps
/// `adds: None`, the count-less row the TUI already draws, and opening the
/// file still works. A workspace shares one budget across its repos.
pub const UNTRACKED_BADGE_BUDGET: u64 = 16 * 1024 * 1024;

/// What is left of `UNTRACKED_BADGE_BUDGET` for the list being built.
struct BadgeBudget {
    remaining: u64,
}

impl BadgeBudget {
    fn new() -> Self {
        Self { remaining: UNTRACKED_BADGE_BUDGET }
    }

    /// No badges at all: the file road only needs the list's PATHS, so it
    /// reads no other file's bytes on the way to the one it opens.
    fn none() -> Self {
        Self { remaining: 0 }
    }

    /// Charge a file's length up front, before a byte of it is read: the
    /// budget bounds what the list will read in total, not what it did.
    fn take(&mut self, len: u64) -> bool {
        if len > self.remaining {
            return false;
        }
        self.remaining -= len;
        true
    }
}

/// Read-path git, bytes out. `--no-optional-locks` always.
fn git_bytes(repo: &Path, args: &[&str]) -> Result<Vec<u8>> {
    git_bytes_ok(repo, args, |code| code == 0)
}

/// `git_bytes` for the `--no-index` road, where **exit 1 is success**: that is
/// git's diff convention for "differences found", and every untracked file we
/// ask about has some. Anything above 1 is still a failure.
fn git_bytes_diff(repo: &Path, args: &[&str]) -> Result<Vec<u8>> {
    git_bytes_ok(repo, args, |code| code <= 1)
}

fn git_bytes_ok(repo: &Path, args: &[&str], ok: impl Fn(i32) -> bool) -> Result<Vec<u8>> {
    let out =
        crate::git::git(repo).arg("--no-optional-locks").args(args).output().context("run git")?;
    match out.status.code() {
        Some(code) if ok(code) => Ok(out.stdout),
        _ => bail!("{}", String::from_utf8_lossy(&out.stderr).trim()),
    }
}

/// The stable file list, BASE...BRANCH (three dots: the merge-base diff —
/// "what did this ticket change" — right even after base moved).
pub fn diff_list(repo: &Path, binding: &Binding) -> Result<Response> {
    let worktree_present = binding.status == BindingStatus::Attached && binding.path.is_dir();
    let range = format!("{}...{}", binding.base_oid, binding.branch);
    let files = diff_list_at(repo, &binding.path, &range, worktree_present)?;
    Ok(Response::DiffList {
        branch: binding.branch.clone(),
        base_oid: binding.base_oid.clone(),
        branch_oid: branch_tip(repo, &binding.branch),
        files,
        worktree_present,
    })
}

/// Ticket `v` on a WORKSPACE binding (T-368): every leg's BASE...BRANCH in
/// one list — the root leg's rows first and unprefixed, each child's
/// prefixed `<repo>/` in leg order — the checkout list's shape over the
/// ticket's own branch. A leg with no commits contributes nothing but its
/// untracked sightings, which is exactly why it is not skipped.
pub fn workspace_diff_list(repo_root: &Path, binding: &Binding) -> Result<Response> {
    let worktree_present = binding.status == BindingStatus::Attached && binding.path.is_dir();
    let mut files = Vec::new();
    let mut root: Option<(String, String)> = None;
    for leg in binding.legs(repo_root, "") {
        let present = worktree_present && leg.path.is_dir();
        let range = format!("{}...{}", leg.base_oid, binding.branch);
        let rows = diff_list_at(&leg.repo, &leg.path, &range, present)?;
        if leg.name.is_empty() {
            root = Some((leg.base_oid.clone(), branch_tip(&leg.repo, &binding.branch)));
        }
        files.extend(prefix_rows(&leg.name, rows));
    }
    let (base_oid, branch_oid) = root.unwrap_or_default();
    Ok(Response::DiffList {
        branch: binding.branch.clone(),
        base_oid,
        branch_oid,
        files,
        worktree_present,
    })
}

/// `DiffFile` on a workspace binding: the first path component names the
/// leg where it is a child's name, anything else is the root leg's own —
/// `checkout_diff_file`'s rule, over the binding's legs.
pub fn workspace_diff_file(
    repo_root: &Path,
    binding: &Binding,
    path: &str,
    context: u32,
) -> Result<FileDiff> {
    let legs = binding.legs(repo_root, "");
    if let Some((name, rest)) = path.split_once('/') {
        if let Some(leg) = legs.iter().find(|l| !l.name.is_empty() && l.name == name) {
            let range = format!("{}...{}", leg.base_oid, binding.branch);
            let fd = diff_file_at(&leg.repo, &range, rest, context)?;
            return Ok(prefix_file(name, fd));
        }
    }
    let Some(root) = legs.iter().find(|l| l.name.is_empty()) else {
        bail!("no such file in this diff");
    };
    diff_file_at(&root.repo, &format!("{}...{}", root.base_oid, binding.branch), path, context)
}

/// `<name>/` in front of every row's path (and rename source) — the one
/// prefixing the checkout road and the ticket road share. An empty name
/// (the root) prefixes nothing.
pub(crate) fn prefix_rows(name: &str, rows: Vec<FileEntry>) -> Vec<FileEntry> {
    if name.is_empty() {
        return rows;
    }
    rows.into_iter()
        .map(|mut f| {
            f.path = format!("{name}/{}", f.path);
            f.old_path = f.old_path.take().map(|old| format!("{name}/{old}"));
            f
        })
        .collect()
}

pub(crate) fn prefix_file(name: &str, mut fd: FileDiff) -> FileDiff {
    if !name.is_empty() {
        fd.path = format!("{name}/{}", fd.path);
        fd.old_path = fd.old_path.take().map(|old| format!("{name}/{old}"));
    }
    fd
}

/// `diff_list`'s body for ONE checkout: the rows of `range` in `repo` —
/// BASE...BRANCH for a ticket, PARENT..COMMIT for a commit — with the working
/// tree's own status flags from `wt` where it stands.
fn diff_list_at(
    repo: &Path,
    wt: &Path,
    range: &str,
    worktree_present: bool,
) -> Result<Vec<FileEntry>> {
    let raw = git_bytes(
        repo,
        &[
            "--no-pager",
            "diff",
            "--raw",
            "-z",
            "--abbrev=40",
            "--find-renames",
            "--no-ext-diff",
            range,
        ],
    )
    .map_err(|e| anyhow::anyhow!("git diff failed: {}", first_line(&e.to_string())))?;
    let mut files = parse_raw_z(&raw);
    // `--find-renames` here as well as on the raw call: without it the counts
    // come from whatever `diff.renames` is set to, and a user who set it false
    // gets a rename's badge off the add's numstat row.
    if let Ok(numstat) = git_bytes(
        repo,
        &["--no-pager", "diff", "--numstat", "-z", "--find-renames", "--no-ext-diff", range],
    ) {
        merge_numstat(&mut files, &parse_numstat_z(&numstat));
    }
    if worktree_present {
        // Display only: an un-added agent file is invisible to every diff
        // query, and it is exactly the change the reviewer least wants to
        // miss. Never -uno here (docs/08 §2's ban).
        if let Ok(status) = git_bytes(wt, &["status", "--porcelain=v2", "-unormal", "-z"]) {
            apply_status_flags(&mut files, &parse_status_v2_z(&status));
        }
    }
    Ok(files)
}

/// One file's hunks. A git failure on the per-file call is a render state
/// (`Unresolvable`, stderr verbatim in the errored register), not a wire
/// error — the pane must show it.
pub fn diff_file(repo: &Path, binding: &Binding, path: &str, context: u32) -> Result<FileDiff> {
    diff_file_at(repo, &format!("{}...{}", binding.base_oid, binding.branch), path, context)
}

fn diff_file_at(repo: &Path, range: &str, path: &str, context: u32) -> Result<FileDiff> {
    // The list entry supplies modes/blobs for classification (and the rename
    // source path). ~30 ms even on huge diffs [M]; keeps the wire stateless.
    let raw = git_bytes(
        repo,
        &[
            "--no-pager",
            "diff",
            "--raw",
            "-z",
            "--abbrev=40",
            "--find-renames",
            "--no-ext-diff",
            range,
        ],
    )?;
    let files = parse_raw_z(&raw);
    let Some(entry) = files.iter().find(|f| f.path == path) else {
        bail!("no such file in this diff");
    };
    let ctx = format!("-U{}", context.clamp(0, 999));
    let mut args: Vec<&str> = vec![
        "--no-pager",
        "diff",
        "--no-color",
        "--no-ext-diff",
        &ctx,
        "--src-prefix=a/",
        "--dst-prefix=b/",
    ];
    // Deviation from 08 §1.2's verbatim command: on a rename the per-file
    // call needs rename detection and both pathspecs, or git emits an
    // add-only patch for the new path.
    if entry.old_path.is_some() {
        args.push("--find-renames");
    }
    args.push(range);
    args.push("--");
    if let Some(old) = &entry.old_path {
        args.push(old);
    }
    args.push(path);
    match git_bytes(repo, &args) {
        Ok(patch) => Ok(build_file_diff(entry, &patch, MAX_PATCH_BYTES)),
        Err(e) => Ok(FileDiff {
            path: entry.path.clone(),
            old_path: entry.old_path.clone(),
            render: Render::Unresolvable { message: first_line(&e.to_string()).to_string() },
            hunks: Vec::new(),
        }),
    }
}

/// A row of the push / pull lists, opened: the commit against its first
/// parent — what it changed on the branch it landed on, a merge included —
/// or against the empty tree when it is a root. Nothing of the working tree
/// rides it, so `worktree_present` is false and no status is read.
pub fn commit_diff_list(repo: &Path, oid: &str) -> Result<Response> {
    let (parent, range) = commit_range(repo, oid)?;
    let files = diff_list_at(repo, repo, &range, false)?;
    Ok(Response::DiffList {
        branch: String::new(),
        base_oid: parent,
        branch_oid: oid.to_string(),
        files,
        worktree_present: false,
    })
}

pub fn commit_diff_file(repo: &Path, oid: &str, path: &str, context: u32) -> Result<FileDiff> {
    let (_, range) = commit_range(repo, oid)?;
    diff_file_at(repo, &range, path, context)
}

/// `(parent, "PARENT..COMMIT")`. The oid arrives from a client and lands in
/// git's argv, so it must be a full object name and nothing else: a leading
/// `-` would be an option, and `HEAD~3` or `a..b` a revision expression the
/// list never offered.
fn commit_range(repo: &Path, oid: &str) -> Result<(String, String)> {
    if !matches!(oid.len(), 40 | 64) || !oid.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("not a commit id: {oid}");
    }
    let short: String = oid.chars().take(7).collect();
    git_bytes(repo, &["rev-parse", "--verify", "--quiet", &format!("{oid}^{{commit}}")])
        .map_err(|_| anyhow::anyhow!("no such commit here: {short}"))?;
    let parent = match git_bytes(repo, &["rev-parse", "--verify", "--quiet", &format!("{oid}^1")]) {
        Ok(out) => String::from_utf8_lossy(&out).trim().to_string(),
        Err(_) => empty_tree(repo)?,
    };
    let range = format!("{parent}..{oid}");
    Ok((parent, range))
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or(s)
}

// ---------------------------------------------------------------------------
// The board's own checkout (T-221): HEAD vs the working tree.
//
// The branch diff answers "what did this ticket change"; this answers "what is
// uncommitted here". Most tickets are shared_checkout, so their agents' work
// lives exactly here and nothing else in mesimon could show it.
//
// Two gaps are known and deliberate. A path in a merge conflict gives
// `git diff HEAD` a COMBINED diff (`@@@`), which `parse_hunk_header` reads as
// zero hunks, so it renders as "no content change" — the branch diff never
// meets this, because it compares two commits. And an untracked directory git
// refuses to descend into (another repository) stays a display-only row, the
// same shape the branch diff gives every untracked path.
// ---------------------------------------------------------------------------

/// The empty tree, for a repository whose HEAD is unborn. Asked of git rather
/// than spelled: `4b825dc…` is the SHA-1 value and wrong in a SHA-256 repo.
fn empty_tree(repo: &Path) -> Result<String> {
    let out = git_bytes(repo, &["hash-object", "-t", "tree", "/dev/null"])?;
    Ok(String::from_utf8_lossy(&out).trim().to_string())
}

/// `# branch.oid <oid>` out of a porcelain v2 `--branch -z` stream. `None` on
/// `(initial)`, which is how an unborn HEAD reports.
fn head_oid(bytes: &[u8]) -> Option<String> {
    for rec in bytes.split(|b| *b == 0) {
        if let Some(rest) = rec.strip_prefix(b"# branch.oid ") {
            let oid = String::from_utf8_lossy(rest).trim().to_string();
            return (oid != "(initial)").then_some(oid);
        }
    }
    None
}

/// The mode git would record for a worktree file, so an untracked row can be
/// classified like any other. `None` for anything that is not a file or a
/// symlink — a fifo or a socket would block `--no-index` on open.
fn worktree_mode(path: &Path) -> Option<String> {
    let md = std::fs::symlink_metadata(path).ok()?;
    if md.file_type().is_symlink() {
        return Some("120000".to_string());
    }
    if !md.file_type().is_file() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if md.permissions().mode() & 0o111 != 0 {
            return Some("100755".to_string());
        }
    }
    Some("100644".to_string())
}

/// The adds badge for an untracked file, without forking git — its whole
/// content is the diff. `None` where git would print `Binary files … differ`,
/// past the patch ceiling, or once the list's byte budget is spent, which are
/// the rows that get no count anyway. The read streams and stops at the first
/// NUL inside the sniff window, so a binary costs its first chunk, never its
/// whole length, and no file is ever held in memory at once.
fn untracked_adds(path: &Path, max_bytes: u64, budget: &mut BadgeBudget) -> Option<u32> {
    use std::io::Read;
    let md = std::fs::metadata(path).ok()?;
    if md.len() > max_bytes || !budget.take(md.len()) {
        return None;
    }
    let mut file = std::fs::File::open(path).ok()?;
    let mut buf = [0u8; 64 * 1024];
    let mut seen = 0usize;
    let mut n = 0u32;
    let mut last = b'\n';
    loop {
        let got = match file.read(&mut buf) {
            Ok(0) => break,
            Ok(got) => got,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return None,
        };
        let chunk = &buf[..got];
        if seen < BINARY_SNIFF_BYTES {
            let window = &chunk[..got.min(BINARY_SNIFF_BYTES - seen)];
            if window.contains(&0) {
                return None;
            }
        }
        seen = seen.saturating_add(got);
        n += chunk.iter().filter(|b| **b == b'\n').count() as u32;
        last = chunk[got - 1];
    }
    if last != b'\n' {
        n += 1;
    }
    Some(n)
}

/// The checkout's file list: branch, the HEAD it is measured against, and one
/// row per changed path — staged, unstaged, or untracked.
///
/// One `status` call carries three answers (branch, HEAD oid, the flags), so
/// the branch name costs no extra fork and a detached HEAD reads as the short
/// oid, the same convention `gitstatus` and the header already use.
fn checkout_entries(
    repo: &Path,
    budget: &mut BadgeBudget,
) -> Result<(String, String, Vec<mesimon_core::diff::FileEntry>)> {
    // `-uall`, not `-unormal`: `-unormal` collapses an untracked directory to
    // one `? dir/` row, and `git diff --no-index` cannot open a directory, so
    // that row could never be read. Ignored files stay out either way.
    let status = git_bytes(repo, &["status", "--porcelain=v2", "--branch", "-uall", "-z"])
        .map_err(|e| anyhow::anyhow!("git status failed: {}", first_line(&e.to_string())))?;
    let g = crate::gitstatus::parse(&status);
    // Pin the oid the status reported: a commit landing between these calls
    // must not leave the range and `base_oid` describing different HEADs.
    let base = match head_oid(&status) {
        Some(oid) => oid,
        None => empty_tree(repo)?,
    };
    let raw = git_bytes(
        repo,
        &[
            "--no-pager",
            "diff",
            "--raw",
            "-z",
            "--abbrev=40",
            "--find-renames",
            "--no-ext-diff",
            &base,
        ],
    )
    .map_err(|e| anyhow::anyhow!("git diff failed: {}", first_line(&e.to_string())))?;
    let mut files = parse_raw_z(&raw);
    // `--find-renames` here too. Without it the counts come from `diff.renames`,
    // and a user who set it false gets a rename's badge off the add's numstat.
    if let Ok(numstat) = git_bytes(
        repo,
        &["--no-pager", "diff", "--numstat", "-z", "--find-renames", "--no-ext-diff", &base],
    ) {
        merge_numstat(&mut files, &parse_numstat_z(&numstat));
    }
    apply_status_flags(&mut files, &parse_status_v2_z(&status));

    // An untracked row is an ADD here, not a display-only sighting: the
    // checkout can serve its content from `--no-index`, so it is stamped like
    // one and every reader downstream — the fetch, the gutter, the hunk pane —
    // treats it as the add it is. A path git would not descend into keeps the
    // empty status, which still means "there is no patch behind this row".
    let mut untracked = 0usize;
    files.retain_mut(|f| {
        if !f.untracked || !f.status.is_empty() {
            return true;
        }
        untracked += 1;
        if untracked > MAX_UNTRACKED_ROWS {
            return false;
        }
        if let Some(mode) = worktree_mode(&repo.join(&f.path)) {
            f.status = "A".to_string();
            f.old_mode = "000000".to_string();
            f.new_mode = mode;
            f.adds = untracked_adds(&repo.join(&f.path), MAX_PATCH_BYTES, budget);
            f.dels = f.adds.map(|_| 0);
        }
        true
    });
    Ok((g.branch, base, files))
}

/// `DiffList` for the checkout target. On a workspace (T-225) it is ONE list
/// over the root and every nested repo — the root's own rows first and
/// unprefixed, then each child's prefixed `<repo>/`, in census order — which
/// is the header's summed count spelled out, row by row.
pub fn checkout_diff_list(root: &Path) -> Result<Response> {
    let repos = crate::gitstatus::census(root);
    let mut budget = BadgeBudget::new();
    if repos.is_empty() {
        let (branch, base_oid, files) = checkout_entries(root, &mut budget)?;
        return Ok(Response::DiffList {
            branch,
            base_oid,
            // No second tip: the working tree is not a ref.
            branch_oid: String::new(),
            files,
            worktree_present: true,
        });
    }
    // A folder of repos has no HEAD of its own: the root contributes nothing
    // and the list is its children's. A meta repo contributes its rows minus
    // the one `? child/` sighting git leaves for a nested repository it will
    // not descend into — that child's rows follow, under its name.
    let (root_branch, mut files, base_oid) = match checkout_entries(root, &mut budget) {
        Ok((branch, base, mut rows)) => {
            rows.retain(|f| !repos.iter().any(|r| f.path.trim_end_matches('/') == r));
            (Some(branch), rows, base)
        }
        Err(_) => (None, Vec::new(), String::new()),
    };
    let mut child_branches = Vec::new();
    for name in &repos {
        let Ok((branch, _, rows)) = checkout_entries(&root.join(name), &mut budget) else {
            continue;
        };
        child_branches.push(branch);
        files.extend(prefix_rows(name, rows));
    }
    // The identity row's word is the header's: the root's branch where the
    // root is a repository, the one child's where a folder holds exactly
    // one, else the count.
    let branch = match (root_branch, &repos[..]) {
        (Some(b), _) => b,
        (None, [_]) => child_branches.pop().unwrap_or_default(),
        (None, _) => mesimon_core::workspace::repos_word(repos.len()),
    };
    Ok(Response::DiffList {
        branch,
        base_oid,
        branch_oid: String::new(),
        files,
        worktree_present: true,
    })
}

/// `DiffFile` for the checkout target. On a workspace the first path
/// component names the nested repo the row came from — a census name routes
/// to that child with the rest of the path, anything else is the root's own
/// (a directory that is a nested repo is never a tracked path of the meta,
/// so the two cannot collide). The child's list is what the rest is checked
/// against, so a `..` can go nowhere.
pub fn checkout_diff_file(root: &Path, path: &str, context: u32) -> Result<FileDiff> {
    let repos = crate::gitstatus::census(root);
    if let Some((name, rest)) = path.split_once('/') {
        if repos.iter().any(|r| r == name) {
            let fd = checkout_diff_file_one(&root.join(name), rest, context)?;
            return Ok(prefix_file(name, fd));
        }
    }
    checkout_diff_file_one(root, path, context)
}

/// One repository's `DiffFile`. Like the branch road, a path the list did
/// not name is refused — which is also what keeps `--no-index`, whose two
/// operands are plain paths, from being pointed anywhere but at this checkout.
fn checkout_diff_file_one(repo: &Path, path: &str, context: u32) -> Result<FileDiff> {
    let (_, base, files) = checkout_entries(repo, &mut BadgeBudget::none())?;
    let Some(entry) = files.iter().find(|f| f.path == path) else {
        bail!("no such file in this diff");
    };
    let ctx = format!("-U{}", context.clamp(0, 999));
    let mut args: Vec<&str> = vec![
        "--no-pager",
        "diff",
        "--no-color",
        "--no-ext-diff",
        &ctx,
        "--src-prefix=a/",
        "--dst-prefix=b/",
    ];
    let untracked = entry.untracked && entry.status == "A";
    if untracked {
        // The whole file is the patch. Size is checked here rather than in
        // `build_file_diff` so a 500 MB stray never becomes 500 MB of stdout.
        let bytes = std::fs::metadata(repo.join(path)).map(|m| m.len()).unwrap_or(0);
        if bytes > MAX_PATCH_BYTES {
            return Ok(FileDiff {
                path: entry.path.clone(),
                old_path: None,
                render: Render::TooLarge { bytes },
                hunks: Vec::new(),
            });
        }
        args.push("--no-index");
        args.push("--");
        args.push("/dev/null");
        args.push(path);
        return Ok(match git_bytes_diff(repo, &args) {
            Ok(patch) => build_file_diff(entry, &patch, MAX_PATCH_BYTES),
            Err(e) => unresolvable(entry, &e.to_string()),
        });
    }
    if entry.old_path.is_some() {
        args.push("--find-renames");
    }
    args.push(&base);
    args.push("--");
    if let Some(old) = &entry.old_path {
        args.push(old);
    }
    args.push(path);
    Ok(match git_bytes(repo, &args) {
        Ok(patch) => build_file_diff(entry, &patch, MAX_PATCH_BYTES),
        Err(e) => unresolvable(entry, &e.to_string()),
    })
}

fn unresolvable(entry: &mesimon_core::diff::FileEntry, message: &str) -> FileDiff {
    FileDiff {
        path: entry.path.clone(),
        old_path: entry.old_path.clone(),
        render: Render::Unresolvable { message: first_line(message).to_string() },
        hunks: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktree::{have_git, provision};
    use mesimon_core::diff::Sign;
    use std::path::PathBuf;
    use std::process::Command;

    fn scratch(name: &str) -> Option<(PathBuf, Binding)> {
        if !have_git() {
            return None;
        }
        let repo = std::env::temp_dir().join(format!("msmn-diff-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&repo).ok();
        std::fs::create_dir_all(&repo).unwrap();
        let run = |d: &Path, args: &[&str]| {
            let out = Command::new("git").arg("-C").arg(d).args(args).output().unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        run(&repo, &["init", "-q", "-b", "main"]);
        run(&repo, &["config", "user.email", "t@t"]);
        run(&repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("keep.txt"), "line one\nline two\n").unwrap();
        std::fs::write(repo.join("renamed-src.txt"), "stable content\nmore\n").unwrap();
        std::fs::write(repo.join("tool.sh"), "#!/bin/sh\necho hi\n").unwrap();
        std::fs::write(repo.join("logo.bin"), [0u8, 159, 146, 150]).unwrap();
        run(&repo, &["add", "."]);
        run(&repo, &["commit", "-qm", "init"]);

        let root = repo.join("_wtroot");
        std::fs::create_dir_all(&root).unwrap();
        let b = provision(&repo, &root, ulid::Ulid(9), "T-9", "diff me").unwrap();

        // Agent work on the branch: edit, rename, chmod, binary, Hebrew name.
        let wt = b.path.clone();
        std::fs::write(wt.join("keep.txt"), "line one\nCHANGED two\n").unwrap();
        run(&wt, &["mv", "renamed-src.txt", "renamed-dst.txt"]);
        std::fs::write(wt.join("logo.bin"), [255u8, 216, 255, 0]).unwrap();
        std::fs::write(wt.join("מסמך.txt"), "שלום\n").unwrap();
        run(&wt, &["add", "."]);
        // After `add .` — a re-add from disk would wipe the staged chmod.
        run(&wt, &["update-index", "--chmod=+x", "tool.sh"]);
        run(&wt, &["commit", "-qm", "agent work"]);
        Some((repo, b))
    }

    fn list_files(resp: &Response) -> &Vec<mesimon_core::diff::FileEntry> {
        match resp {
            Response::DiffList { files, .. } => files,
            other => panic!("expected DiffList, got {other:?}"),
        }
    }

    #[test]
    fn list_covers_rename_chmod_binary_and_hebrew() {
        let Some((repo, b)) = scratch("list") else { return };
        let resp = diff_list(&repo, &b).unwrap();
        let files = list_files(&resp);
        let by = |p: &str| files.iter().find(|f| f.path == p).unwrap();

        assert_eq!(by("keep.txt").status, "M");
        assert_eq!(by("keep.txt").adds, Some(1));
        let ren = by("renamed-dst.txt");
        assert_eq!(ren.status, "R");
        assert_eq!(ren.old_path.as_deref(), Some("renamed-src.txt"));
        let sh = by("tool.sh");
        assert_eq!((sh.old_mode.as_str(), sh.new_mode.as_str()), ("100644", "100755"));
        assert_eq!(by("logo.bin").adds, None, "binary numstat is None");
        assert_eq!(by("מסמך.txt").status, "A", "non-ASCII path survives -z verbatim");
        match &resp {
            Response::DiffList { worktree_present, branch_oid, .. } => {
                assert!(*worktree_present);
                assert_eq!(branch_oid.len(), 40);
            }
            _ => unreachable!(),
        }
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn list_flags_dirty_and_untracked() {
        let Some((repo, b)) = scratch("flags") else { return };
        std::fs::write(b.path.join("keep.txt"), "dirty edit\n").unwrap();
        std::fs::write(b.path.join(".env.local"), "SECRET=1\n").unwrap();
        let resp = diff_list(&repo, &b).unwrap();
        let files = list_files(&resp);
        assert!(files.iter().find(|f| f.path == "keep.txt").unwrap().dirty);
        let env = files.iter().find(|f| f.path == ".env.local").unwrap();
        assert!(env.untracked);
        assert_eq!(env.status, "", "untracked-only row is display-only");
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn file_text_rename_modeonly_binary() {
        let Some((repo, b)) = scratch("file") else { return };
        let fd = diff_file(&repo, &b, "keep.txt", 3).unwrap();
        assert_eq!(fd.render, Render::Text);
        assert!(fd.hunks[0]
            .lines
            .iter()
            .any(|l| l.sign == Sign::Add && l.text.contains("CHANGED")));

        // Rename resolves to hunks (the --find-renames + both-paths fix);
        // content unchanged → rename record with zero hunks, never add-only.
        let fd = diff_file(&repo, &b, "renamed-dst.txt", 3).unwrap();
        assert_eq!(fd.old_path.as_deref(), Some("renamed-src.txt"));
        assert_eq!(fd.render, Render::Text);
        assert!(fd.hunks.is_empty(), "pure rename has no content hunks");

        let fd = diff_file(&repo, &b, "tool.sh", 3).unwrap();
        assert_eq!(
            fd.render,
            Render::ModeOnly { old_mode: "100644".into(), new_mode: "100755".into() }
        );

        let fd = diff_file(&repo, &b, "logo.bin", 3).unwrap();
        assert_eq!(fd.render, Render::Binary);

        assert!(diff_file(&repo, &b, "nope.txt", 3).is_err());
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn evicted_worktree_still_serves_from_object_store() {
        let Some((repo, mut b)) = scratch("evict") else { return };
        crate::worktree::remove(&repo, &b.path).unwrap();
        b.status = BindingStatus::Evicted;
        let resp = diff_list(&repo, &b).unwrap();
        match &resp {
            Response::DiffList { worktree_present, files, .. } => {
                assert!(!worktree_present);
                assert!(files.iter().any(|f| f.path == "keep.txt"));
            }
            _ => unreachable!(),
        }
        let fd = diff_file(&repo, &b, "keep.txt", 3).unwrap();
        assert_eq!(fd.render, Render::Text);
        std::fs::remove_dir_all(&repo).ok();
    }

    // ---- the board's own checkout (T-221) --------------------------------

    /// A plain repository with one commit and no worktree — the shape every
    /// shared_checkout ticket actually works in.
    fn checkout_scratch(name: &str) -> Option<PathBuf> {
        if !have_git() {
            return None;
        }
        let repo = std::env::temp_dir().join(format!("msmn-chk-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&repo).ok();
        std::fs::create_dir_all(&repo).unwrap();
        let run = |d: &Path, args: &[&str]| {
            let out = Command::new("git").arg("-C").arg(d).args(args).output().unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        run(&repo, &["init", "-q", "-b", "main"]);
        run(&repo, &["config", "user.email", "t@t"]);
        run(&repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("keep.txt"), "line one\nline two\n").unwrap();
        std::fs::write(repo.join("moved-src.txt"), "stable\n").unwrap();
        std::fs::write(repo.join("tool.sh"), "#!/bin/sh\necho hi\n").unwrap();
        std::fs::write(repo.join("clean.txt"), "untouched\n").unwrap();
        run(&repo, &["add", "."]);
        run(&repo, &["commit", "-qm", "init"]);
        Some(repo)
    }

    fn dirty_checkout(name: &str) -> Option<PathBuf> {
        let repo = checkout_scratch(name)?;
        let run = |d: &Path, args: &[&str]| {
            let out = Command::new("git").arg("-C").arg(d).args(args).output().unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        // Unstaged edit, staged-and-then-edited (MM), a staged rename, a
        // worktree-only chmod, and four kinds of untracked.
        std::fs::write(repo.join("keep.txt"), "line one\nCHANGED two\n").unwrap();
        std::fs::write(repo.join("clean.txt"), "staged\n").unwrap();
        run(&repo, &["add", "clean.txt"]);
        std::fs::write(repo.join("clean.txt"), "staged then edited\n").unwrap();
        run(&repo, &["mv", "moved-src.txt", "moved-dst.txt"]);
        run(&repo, &["update-index", "--chmod=+x", "tool.sh"]);
        run(&repo, &["reset", "-q", "--", "tool.sh"]);
        std::fs::set_permissions(
            repo.join("tool.sh"),
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();
        std::fs::write(repo.join("stray.txt"), "one\ntwo\nno trailing newline").unwrap();
        std::fs::write(repo.join("blob.bin"), [0u8, 159, 146, 150]).unwrap();
        std::fs::create_dir_all(repo.join("newdir")).unwrap();
        std::fs::write(repo.join("newdir/inside.txt"), "nested\n").unwrap();
        std::os::unix::fs::symlink("keep.txt", repo.join("link.txt")).unwrap();
        Some(repo)
    }

    fn files_of(resp: &Response) -> Vec<mesimon_core::diff::FileEntry> {
        match resp {
            Response::DiffList { files, .. } => files.clone(),
            other => panic!("expected DiffList, got {other:?}"),
        }
    }

    #[test]
    fn checkout_list_is_one_row_per_path_with_untracked_as_adds() {
        let Some(repo) = dirty_checkout("list") else { return };
        let resp = checkout_diff_list(&repo).unwrap();
        let files = files_of(&resp);
        let by = |p: &str| files.iter().find(|f| f.path == p).cloned().unwrap();

        match &resp {
            Response::DiffList { branch, base_oid, branch_oid, worktree_present, .. } => {
                assert_eq!(branch, "main");
                assert_eq!(base_oid.len(), 40, "the HEAD the diff was taken against");
                assert!(branch_oid.is_empty(), "the working tree is not a ref");
                assert!(worktree_present, "the checkout IS the working tree");
            }
            _ => unreachable!(),
        }

        assert_eq!(by("keep.txt").status, "M");
        assert_eq!(by("keep.txt").adds, Some(1));
        // Staged AND edited since: still one row, HEAD to the working tree.
        assert_eq!(files.iter().filter(|f| f.path == "clean.txt").count(), 1);
        assert_eq!(by("clean.txt").status, "M");
        let moved = by("moved-dst.txt");
        assert_eq!(moved.status, "R");
        assert_eq!(moved.old_path.as_deref(), Some("moved-src.txt"));

        // An untracked file is an ADD here, not a sighting.
        let stray = by("stray.txt");
        assert_eq!(stray.status, "A");
        assert!(stray.untracked);
        assert_eq!(stray.old_mode, "000000");
        assert_eq!(stray.new_mode, "100644");
        assert_eq!(stray.adds, Some(3), "the last line counts without its newline");
        assert_eq!(stray.dels, Some(0));
        assert_eq!(by("blob.bin").adds, None, "a binary stray gets no count");
        assert_eq!(by("link.txt").new_mode, "120000");
    }

    #[test]
    fn untracked_adds_streams_and_stops_at_a_nul() {
        let dir = std::env::temp_dir().join(format!("msmn-diff-adds-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Longer than one read chunk, so a newline count that only looked at
        // the first chunk would come up short.
        let text = "a line\n".repeat(20_000);
        std::fs::write(dir.join("long.txt"), &text).unwrap();
        std::fs::write(dir.join("tail.txt"), "x\ny").unwrap();
        std::fs::write(dir.join("empty.txt"), "").unwrap();
        let mut bin = vec![b'z'; 100];
        bin.push(0);
        bin.extend_from_slice(b"\n\n\n");
        std::fs::write(dir.join("bin"), &bin).unwrap();

        let mut budget = BadgeBudget::new();
        assert_eq!(
            untracked_adds(&dir.join("long.txt"), MAX_PATCH_BYTES, &mut budget),
            Some(20_000)
        );
        assert_eq!(untracked_adds(&dir.join("tail.txt"), MAX_PATCH_BYTES, &mut budget), Some(2));
        assert_eq!(untracked_adds(&dir.join("empty.txt"), MAX_PATCH_BYTES, &mut budget), Some(0));
        assert_eq!(untracked_adds(&dir.join("bin"), MAX_PATCH_BYTES, &mut budget), None);
        assert_eq!(
            budget.remaining,
            UNTRACKED_BADGE_BUDGET - text.len() as u64 - 3 - bin.len() as u64,
            "every file read is charged by its length, the binary included"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn untracked_adds_stops_counting_once_the_budget_is_spent() {
        let dir = std::env::temp_dir().join(format!("msmn-diff-budget-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        std::fs::write(dir.join("b.txt"), "three\n").unwrap();
        let mut budget = BadgeBudget { remaining: 8 };
        assert_eq!(untracked_adds(&dir.join("a.txt"), MAX_PATCH_BYTES, &mut budget), Some(2));
        assert_eq!(
            untracked_adds(&dir.join("b.txt"), MAX_PATCH_BYTES, &mut budget),
            None,
            "past the budget the row keeps no count rather than reading on"
        );
        assert_eq!(budget.remaining, 0, "a refused file is not charged");
        assert_eq!(
            untracked_adds(&dir.join("b.txt"), MAX_PATCH_BYTES, &mut BadgeBudget::none()),
            None,
            "the file road's budget reads nothing"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn checkout_list_expands_an_untracked_directory() {
        let Some(repo) = dirty_checkout("uall") else { return };
        let files = files_of(&checkout_diff_list(&repo).unwrap());
        // `-unormal` would give one `newdir/` row, which cannot be opened.
        assert!(files.iter().any(|f| f.path == "newdir/inside.txt"));
        assert!(!files.iter().any(|f| f.path.ends_with('/')));
    }

    #[test]
    fn checkout_file_renders_an_untracked_file_as_all_adds() {
        let Some(repo) = dirty_checkout("adds") else { return };
        let fd = checkout_diff_file(&repo, "stray.txt", 3).unwrap();
        assert_eq!(fd.render, Render::Text);
        let lines: Vec<_> = fd.hunks.iter().flat_map(|h| h.lines.iter()).collect();
        assert_eq!(lines.len(), 3);
        assert!(lines.iter().all(|l| l.sign == Sign::Add), "a new file is nothing but adds");
        assert_eq!(lines[0].text, "one");

        // And a tracked edit still reads normally on the same road.
        let fd = checkout_diff_file(&repo, "keep.txt", 3).unwrap();
        assert_eq!(fd.render, Render::Text);
        assert!(fd.hunks[0].lines.iter().any(|l| l.text.contains("CHANGED")));
    }

    #[test]
    fn checkout_file_reads_an_untracked_symlink_as_a_symlink() {
        let Some(repo) = dirty_checkout("link") else { return };
        assert_eq!(checkout_diff_file(&repo, "link.txt", 3).unwrap().render, Render::Symlink);
        assert_eq!(checkout_diff_file(&repo, "blob.bin", 3).unwrap().render, Render::Binary);
    }

    /// The T-221 classification bug, end to end through real git: the raw
    /// record's destination blob is forty zeros, so blob equality could never
    /// have answered here.
    #[test]
    fn checkout_worktree_chmod_is_a_mode_change() {
        let Some(repo) = dirty_checkout("chmod") else { return };
        let files = files_of(&checkout_diff_list(&repo).unwrap());
        let sh = files.iter().find(|f| f.path == "tool.sh").unwrap();
        assert_eq!((sh.old_mode.as_str(), sh.new_mode.as_str()), ("100644", "100755"));
        assert_eq!(sh.new_blob, "0".repeat(40), "the worktree side has no oid");
        assert_eq!(
            checkout_diff_file(&repo, "tool.sh", 3).unwrap().render,
            Render::ModeOnly { old_mode: "100644".into(), new_mode: "100755".into() }
        );
    }

    #[test]
    fn checkout_list_on_an_unborn_head_uses_the_empty_tree() {
        let Some(repo) = checkout_scratch("unborn") else { return };
        // A fresh repository with a staged file and nothing committed.
        let fresh = repo.join("fresh");
        std::fs::create_dir_all(&fresh).unwrap();
        let run = |args: &[&str]| {
            let out = Command::new("git").arg("-C").arg(&fresh).args(args).output().unwrap();
            assert!(out.status.success(), "git {args:?}");
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(fresh.join("first.txt"), "hello\n").unwrap();
        run(&["add", "first.txt"]);
        let files = files_of(&checkout_diff_list(&fresh).unwrap());
        let first = files.iter().find(|f| f.path == "first.txt").unwrap();
        assert_eq!(first.status, "A", "staged against the empty tree, not untracked");
        assert_eq!(checkout_diff_file(&fresh, "first.txt", 3).unwrap().render, Render::Text);
    }

    #[test]
    fn checkout_file_refuses_a_path_outside_the_list() {
        let Some(repo) = dirty_checkout("escape") else { return };
        // `--no-index` takes two plain paths; the list is what fences it.
        assert!(checkout_diff_file(&repo, "../../etc/passwd", 3).is_err());
        assert!(checkout_diff_file(&repo, "clean-but-unchanged", 3).is_err());
    }

    // ---- one commit of the checkout's history (the push / pull rows) -----------

    /// `checkout_scratch`'s repository with a history to open: a commit on
    /// `main`, a side branch merged into it, and the oid of every step.
    fn history(name: &str) -> Option<(PathBuf, [String; 4])> {
        let repo = checkout_scratch(name)?;
        let git = |args: &[&str]| {
            let out = Command::new("git").arg("-C").arg(&repo).args(args).output().unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        let root = git(&["rev-list", "--max-parents=0", "HEAD"]);
        std::fs::write(repo.join("keep.txt"), "line one\nline 2\n").unwrap();
        git(&["commit", "-qam", "edit keep"]);
        let edit = git(&["rev-parse", "HEAD"]);
        git(&["checkout", "-qb", "side", &root]);
        std::fs::write(repo.join("side.txt"), "from the side\n").unwrap();
        git(&["add", "side.txt"]);
        git(&["commit", "-qm", "side"]);
        git(&["checkout", "-q", "main"]);
        git(&["merge", "-q", "--no-ff", "-m", "merge side", "side"]);
        let merge = git(&["rev-parse", "HEAD"]);
        let side = git(&["rev-parse", "side"]);
        Some((repo, [root, edit, side, merge]))
    }

    #[test]
    fn a_commit_diffs_against_its_first_parent() {
        let Some((repo, [root, edit, _, merge])) = history("commit") else { return };
        let resp = commit_diff_list(&repo, &edit).unwrap();
        match &resp {
            Response::DiffList { base_oid, branch_oid, worktree_present, .. } => {
                assert_eq!(base_oid, &root, "measured against its parent");
                assert_eq!(branch_oid, &edit);
                assert!(!worktree_present, "a commit reads no working tree");
            }
            other => panic!("expected DiffList, got {other:?}"),
        }
        let files = files_of(&resp);
        assert_eq!(files.len(), 1, "{files:?}");
        assert_eq!(
            (files[0].path.as_str(), files[0].adds, files[0].dels),
            ("keep.txt", Some(1), Some(1))
        );
        let fd = commit_diff_file(&repo, &edit, "keep.txt", 3).unwrap();
        let lines: Vec<_> = fd.hunks[0].lines.iter().map(|l| (l.sign, l.text.as_str())).collect();
        assert!(lines.contains(&(Sign::Del, "line two")) && lines.contains(&(Sign::Add, "line 2")));
        // A merge reads against the branch it landed on: what the side
        // branch brought in, and nothing `main` already had.
        let files = files_of(&commit_diff_list(&repo, &merge).unwrap());
        let paths: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["side.txt"]);
    }

    #[test]
    fn a_root_commit_diffs_against_the_empty_tree() {
        let Some((repo, [root, ..])) = history("root") else { return };
        let files = files_of(&commit_diff_list(&repo, &root).unwrap());
        assert!(files.iter().all(|f| f.status == "A"), "{files:?}");
        assert!(files.iter().any(|f| f.path == "keep.txt"), "{files:?}");
        assert_eq!(commit_diff_file(&repo, &root, "keep.txt", 3).unwrap().render, Render::Text);
    }

    /// The oid reaches git's argv: only a full object name may.
    #[test]
    fn a_commit_id_is_hex_and_nothing_else() {
        let Some((repo, [_, edit, ..])) = history("refuse") else { return };
        let short: String = edit.chars().take(12).collect();
        for oid in ["--output=/tmp/x", "HEAD", "HEAD~1", &short, &format!("{edit}..HEAD")] {
            assert!(commit_diff_list(&repo, oid).is_err(), "{oid:?} was accepted");
            assert!(commit_diff_file(&repo, oid, "keep.txt", 3).is_err(), "{oid:?} was accepted");
        }
        // Well formed but not here, and well formed but not a commit.
        assert!(commit_diff_list(&repo, &"a".repeat(40)).is_err());
        let tree = String::from_utf8(
            Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(["rev-parse", "HEAD^{tree}"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap();
        assert!(commit_diff_list(&repo, tree.trim()).is_err(), "a tree is not a commit");
    }

    // ---- a workspace: repositories nested under the root (T-225) --------------

    use crate::testrepo::workspace_scratch;

    #[test]
    fn census_names_own_repos_only() {
        let Some(root) = workspace_scratch("census") else { return };
        assert_eq!(crate::gitstatus::census(&root), vec!["api", "web"]);
        // A plain checkout has no census, and a folder that is not a repo
        // still has its children.
        let Some(plain) = checkout_scratch("census-plain") else { return };
        assert!(crate::gitstatus::census(&plain).is_empty());
        std::fs::remove_dir_all(root.join(".git")).unwrap();
        assert_eq!(crate::gitstatus::census(&root), vec!["api", "web"]);
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&plain).ok();
    }

    #[test]
    fn workspace_sample_sums_the_children() {
        let Some(root) = workspace_scratch("sample") else { return };
        let g = crate::gitstatus::sample(&root);
        assert!(g.sampled);
        assert_eq!(g.repos, vec!["api", "web"]);
        assert_eq!((g.branch.as_str(), g.changed), ("main", 0));
        std::fs::write(root.join("api/server.ts"), "one\nCHANGED\n").unwrap();
        std::fs::write(root.join("api/stray.md"), "new\n").unwrap();
        std::fs::write(root.join("web/page.tsx"), "bye\n").unwrap();
        std::fs::write(root.join("CLAUDE.md"), "# ws\nmore\n").unwrap();
        let g = crate::gitstatus::sample(&root);
        assert_eq!(g.changed, 4, "{g:?}");
        // A folder of repos: no root branch, still sampled, still summed.
        std::fs::remove_dir_all(root.join(".git")).unwrap();
        let g = crate::gitstatus::sample(&root);
        assert!(g.sampled && g.branch.is_empty(), "{g:?}");
        assert_eq!((g.repos.len(), g.changed), (2, 3), "{g:?}");
        std::fs::remove_dir_all(&root).ok();
    }

    /// Ticket `v` on a workspace binding (T-368): one list over the legs,
    /// the root's rows bare and each child's under its name, the same
    /// routing on the file call, and the same three refusals.
    #[test]
    fn ticket_diff_on_a_workspace_binding_prefixes_the_children() {
        let Some(root) = workspace_scratch("wsdiff") else { return };
        let wtroot = root.join("_wtroot");
        std::fs::create_dir_all(&wtroot).unwrap();
        let census = crate::gitstatus::census(&root);
        let b = crate::worktree::provision_workspace(
            &root,
            &wtroot,
            &census,
            true,
            ulid::Ulid(1),
            "T-1",
            "Fix",
            &mut |_, _| {},
        )
        .unwrap();
        crate::testrepo::commit(&b.path.join("api"), "server.ts", "one\nCHANGED\n", "api");
        crate::testrepo::commit(&b.path, "CLAUDE.md", "# ws\nmore\n", "root");
        std::fs::write(b.path.join("web/stray.tsx"), "un-added\n").unwrap();
        let resp = workspace_diff_list(&root, &b).unwrap();
        match &resp {
            Response::DiffList { branch, base_oid, branch_oid, worktree_present, .. } => {
                assert_eq!(branch, &b.branch);
                assert_eq!(base_oid, &b.base_oid, "the root leg's base");
                assert_eq!(branch_oid, &branch_tip(&root, &b.branch));
                assert!(*worktree_present);
            }
            _ => unreachable!(),
        }
        let paths: Vec<String> = files_of(&resp).into_iter().map(|f| f.path).collect();
        assert_eq!(paths, vec!["CLAUDE.md", "api/server.ts", "web/stray.tsx"]);
        let fd = workspace_diff_file(&root, &b, "api/server.ts", 3).unwrap();
        assert_eq!(fd.path, "api/server.ts");
        assert!(matches!(fd.render, Render::Text), "{:?}", fd.render);
        assert_eq!(fd.hunks.len(), 1);
        assert_eq!(workspace_diff_file(&root, &b, "CLAUDE.md", 3).unwrap().path, "CLAUDE.md");
        assert!(workspace_diff_file(&root, &b, "web/x", 3).is_err());
        assert!(workspace_diff_file(&root, &b, "api/../CLAUDE.md", 3).is_err());
        assert!(workspace_diff_file(&root, &b, "server.ts", 3).is_err(), "not the root's");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn workspace_list_is_one_list_prefixed_by_repo() {
        let Some(root) = workspace_scratch("list") else { return };
        std::fs::write(root.join("api/server.ts"), "one\nCHANGED\n").unwrap();
        std::fs::write(root.join("api/stray.md"), "new\n").unwrap();
        std::fs::write(root.join("web/page.tsx"), "bye\n").unwrap();
        std::fs::write(root.join("CLAUDE.md"), "# ws\nmore\n").unwrap();
        let resp = checkout_diff_list(&root).unwrap();
        match &resp {
            Response::DiffList { branch, base_oid, branch_oid, worktree_present, .. } => {
                assert_eq!(branch, "main", "the root is a repo: its branch leads");
                assert_eq!(base_oid.len(), 40, "the meta's HEAD");
                assert!(branch_oid.is_empty() && *worktree_present);
            }
            _ => unreachable!(),
        }
        let paths: Vec<String> = files_of(&resp).into_iter().map(|f| f.path).collect();
        assert_eq!(paths, vec!["CLAUDE.md", "api/server.ts", "api/stray.md", "web/page.tsx"]);

        // A meta that does NOT ignore its children lists each as a `? child/`
        // sighting git will not descend into: dropped, because the child's
        // own rows follow under that very name.
        std::fs::write(root.join(".gitignore"), "").unwrap();
        let paths: Vec<String> =
            files_of(&checkout_diff_list(&root).unwrap()).into_iter().map(|f| f.path).collect();
        assert!(paths.iter().any(|p| p == ".gitignore"), "{paths:?}");
        assert!(!paths.iter().any(|p| p == "api/" || p == "web/"), "{paths:?}");
        assert!(paths.iter().any(|p| p == "api/server.ts"), "{paths:?}");
        // The gitfile child and node_modules ARE the root's own untracked
        // sightings — they are not repos, so nothing else speaks for them.
        assert!(paths.iter().any(|p| p == ".wt-web/"), "{paths:?}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn workspace_file_routes_on_the_prefix() {
        let Some(root) = workspace_scratch("file") else { return };
        std::fs::write(root.join("api/server.ts"), "one\nCHANGED\n").unwrap();
        std::fs::write(root.join("api/stray.md"), "new\n").unwrap();
        std::fs::write(root.join("CLAUDE.md"), "# ws\nmore\n").unwrap();
        let fd = checkout_diff_file(&root, "api/server.ts", 3).unwrap();
        assert_eq!(fd.path, "api/server.ts");
        assert_eq!(fd.hunks.len(), 1, "{fd:?}");
        let fd = checkout_diff_file(&root, "api/stray.md", 3).unwrap();
        assert_eq!((fd.path.as_str(), fd.hunks.len()), ("api/stray.md", 1));
        let fd = checkout_diff_file(&root, "CLAUDE.md", 3).unwrap();
        assert_eq!(fd.path, "CLAUDE.md");
        // Refused: a child path its list does not name, a `..` through the
        // prefix, a repo that is not in the census.
        assert!(checkout_diff_file(&root, "api/page.tsx", 3).is_err());
        assert!(checkout_diff_file(&root, "api/../CLAUDE.md", 3).is_err());
        assert!(checkout_diff_file(&root, "vendored/lib.c", 3).is_err());
        std::fs::remove_dir_all(&root).ok();
    }

    /// A root that is a repository keeps its own branch whatever is nested
    /// under it (the mesimon checkout's `mt/` scratch repo sits on a branch
    /// called `orphan`); only a FOLDER holding exactly one repo takes that
    /// repo's branch — and never says `1 repo` (author 2026-09-05).
    #[test]
    fn one_nested_repo_names_the_root_or_stands_in_for_a_folder() {
        let Some(root) = workspace_scratch("one") else { return };
        let run = |d: &Path, args: &[&str]| {
            let out = Command::new("git").arg("-C").arg(d).args(args).output().unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        run(&root.join("web"), &["worktree", "remove", "--force", "../.wt-web"]);
        std::fs::remove_dir_all(root.join("web")).unwrap();
        run(&root.join("api"), &["checkout", "-q", "-b", "orphan"]);
        std::fs::write(root.join("api/server.ts"), "one\nCHANGED\n").unwrap();
        std::fs::write(root.join("CLAUDE.md"), "# ws\nmore\n").unwrap();
        // The root is a repo: its branch, the child's changes still counted.
        let g = crate::gitstatus::sample(&root);
        assert_eq!(g.repos, vec!["api"]);
        assert_eq!((g.branch.as_str(), g.changed), ("main", 2), "{g:?}");
        assert_eq!(crate::gitstatus::branch_dir(&root, &g), root);
        match checkout_diff_list(&root).unwrap() {
            Response::DiffList { branch, files, .. } => {
                assert_eq!(branch, "main");
                let paths: Vec<String> = files.into_iter().map(|f| f.path).collect();
                assert_eq!(paths, vec!["CLAUDE.md", "api/server.ts"]);
            }
            _ => unreachable!(),
        }
        // A folder of one: the child is the checkout.
        std::fs::remove_dir_all(root.join(".git")).unwrap();
        let g = crate::gitstatus::sample(&root);
        assert_eq!((g.branch.as_str(), g.changed), ("orphan", 1), "{g:?}");
        assert_eq!(crate::gitstatus::branch_dir(&root, &g), root.join("api"));
        match checkout_diff_list(&root).unwrap() {
            Response::DiffList { branch, files, .. } => {
                assert_eq!(branch, "orphan");
                assert_eq!(files.len(), 1);
            }
            _ => unreachable!(),
        }
        std::fs::remove_dir_all(&root).ok();
    }
}
