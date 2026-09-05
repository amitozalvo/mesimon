//! Read-only diff service (M4b, docs/08 §1.2 plumbing — the flags are
//! load-bearing, every one fails silently if omitted). Runs on connection
//! threads, never the writer: everything here reads the object store and, for
//! the display-only in-flight flags, one `status` call in the worktree.

use std::path::Path;

use anyhow::{bail, Context, Result};
use mesimon_core::command::Response;
use mesimon_core::diff::{
    apply_status_flags, build_file_diff, merge_numstat, parse_numstat_z, parse_raw_z,
    parse_status_v2_z, FileDiff, Render,
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

/// Read-path git, bytes out. `--no-optional-locks` always.
fn git_bytes(repo: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let out =
        crate::git::git(repo).arg("--no-optional-locks").args(args).output().context("run git")?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(out.stdout)
}

/// `git_bytes` for the `--no-index` road, where **exit 1 is success**: that is
/// git's diff convention for "differences found", and every untracked file we
/// ask about has some. Anything above 1 is still a failure.
fn git_bytes_diff(repo: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let out =
        crate::git::git(repo).arg("--no-optional-locks").args(args).output().context("run git")?;
    match out.status.code() {
        Some(0) | Some(1) => Ok(out.stdout),
        _ => bail!("{}", String::from_utf8_lossy(&out.stderr).trim()),
    }
}

/// The stable file list, BASE...BRANCH (three dots: the merge-base diff —
/// "what did this ticket change" — right even after base moved).
pub fn diff_list(repo: &Path, binding: &Binding) -> Result<Response> {
    let range = format!("{}...{}", binding.base_oid, binding.branch);
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
            &range,
        ],
    )
    .map_err(|e| anyhow::anyhow!("git diff failed: {}", first_line(&e.to_string())))?;
    let mut files = parse_raw_z(&raw);
    // `--find-renames` here as well as on the raw call: without it the counts
    // come from whatever `diff.renames` is set to, and a user who set it false
    // gets a rename's badge off the add's numstat row.
    if let Ok(numstat) = git_bytes(
        repo,
        &["--no-pager", "diff", "--numstat", "-z", "--find-renames", "--no-ext-diff", &range],
    ) {
        merge_numstat(&mut files, &parse_numstat_z(&numstat));
    }
    let worktree_present = binding.status == BindingStatus::Attached && binding.path.is_dir();
    if worktree_present {
        // Display only: an un-added agent file is invisible to every diff
        // query, and it is exactly the change the reviewer least wants to
        // miss. Never -uno here (docs/08 §2's ban).
        if let Ok(status) =
            git_bytes(&binding.path, &["status", "--porcelain=v2", "-unormal", "-z"])
        {
            apply_status_flags(&mut files, &parse_status_v2_z(&status));
        }
    }
    Ok(Response::DiffList {
        branch: binding.branch.clone(),
        base_oid: binding.base_oid.clone(),
        branch_oid: branch_tip(repo, &binding.branch),
        files,
        worktree_present,
    })
}

/// One file's hunks. A git failure on the per-file call is a render state
/// (`Unresolvable`, stderr verbatim in the errored register), not a wire
/// error — the pane must show it.
pub fn diff_file(repo: &Path, binding: &Binding, path: &str, context: u32) -> Result<FileDiff> {
    let range = format!("{}...{}", binding.base_oid, binding.branch);
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
            &range,
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
    args.push(&range);
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
/// or past the patch ceiling, which are the two rows that get no count anyway.
fn untracked_adds(path: &Path, max_bytes: u64) -> Option<u32> {
    let md = std::fs::metadata(path).ok()?;
    if md.len() > max_bytes {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if bytes.iter().take(BINARY_SNIFF_BYTES).any(|b| *b == 0) {
        return None;
    }
    let mut n = bytes.iter().filter(|b| **b == b'\n').count() as u32;
    if bytes.last().is_some_and(|b| *b != b'\n') {
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
fn checkout_entries(repo: &Path) -> Result<(String, String, Vec<mesimon_core::diff::FileEntry>)> {
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
            f.adds = untracked_adds(&repo.join(&f.path), MAX_PATCH_BYTES);
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
    if repos.is_empty() {
        let (branch, base_oid, files) = checkout_entries(root)?;
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
    let (mut files, base_oid) = match checkout_entries(root) {
        Ok((_, base, mut rows)) => {
            rows.retain(|f| !repos.iter().any(|r| f.path.trim_end_matches('/') == r));
            (rows, base)
        }
        Err(_) => (Vec::new(), String::new()),
    };
    for name in &repos {
        let Ok((_, _, rows)) = checkout_entries(&root.join(name)) else { continue };
        files.extend(rows.into_iter().map(|mut f| {
            f.path = format!("{name}/{}", f.path);
            f.old_path = f.old_path.take().map(|old| format!("{name}/{old}"));
            f
        }));
    }
    // One nested repo is the checkout and the identity row names its branch,
    // as the header does; more than one is named by the count.
    let branch = match &repos[..] {
        [only] => crate::gitstatus::sample_one(&root.join(only)).branch,
        _ => mesimon_core::workspace::repos_word(repos.len()),
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
            let mut fd = checkout_diff_file_one(&root.join(name), rest, context)?;
            fd.path = format!("{name}/{}", fd.path);
            fd.old_path = fd.old_path.take().map(|old| format!("{name}/{old}"));
            return Ok(fd);
        }
    }
    checkout_diff_file_one(root, path, context)
}

/// One repository's `DiffFile`. Like the branch road, a path the list did
/// not name is refused — which is also what keeps `--no-index`, whose two
/// operands are plain paths, from being pointed anywhere but at this checkout.
fn checkout_diff_file_one(repo: &Path, path: &str, context: u32) -> Result<FileDiff> {
    let (_, base, files) = checkout_entries(repo)?;
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

    // ---- a workspace: repositories nested under the root (T-225) --------------

    /// The author's shape in miniature: a meta repo tracking its own notes
    /// and ignoring every child, over two independent repos, plus a gitfile
    /// child (a worktree of `web`) and a declared submodule, neither of
    /// which is a workspace repo.
    fn workspace_scratch(name: &str) -> Option<PathBuf> {
        if !have_git() {
            return None;
        }
        let root = std::env::temp_dir().join(format!("msmn-ws-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        let run = |d: &Path, args: &[&str]| {
            let out = Command::new("git").arg("-C").arg(d).args(args).output().unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        let init = |d: &Path, file: &str, body: &str| {
            std::fs::create_dir_all(d).unwrap();
            run(d, &["init", "-q", "-b", "main"]);
            run(d, &["config", "user.email", "t@t"]);
            run(d, &["config", "user.name", "t"]);
            std::fs::write(d.join(file), body).unwrap();
            run(d, &["add", "."]);
            run(d, &["commit", "-qm", "init"]);
        };
        init(&root, "CLAUDE.md", "# ws\n");
        std::fs::write(root.join(".gitignore"), "*/\n").unwrap();
        std::fs::write(
            root.join(".gitmodules"),
            "[submodule \"v\"]\n\tpath = vendored\n\turl = x\n",
        )
        .unwrap();
        run(&root, &["add", ".gitignore", ".gitmodules"]);
        run(&root, &["commit", "-qm", "ignore"]);
        init(&root.join("web"), "page.tsx", "hello\n");
        init(&root.join("api"), "server.ts", "one\ntwo\n");
        init(&root.join("vendored"), "lib.c", "int x;\n");
        run(&root.join("web"), &["worktree", "add", "-q", "../.wt-web", "-b", "feedback"]);
        std::fs::create_dir_all(root.join("node_modules/dep")).unwrap();
        Some(root)
    }

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
                assert_eq!(branch, "2 repos");
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

    /// One nested repo is the checkout (author: "if one repo no need to show
    /// '1 repo', show the branch"): the sample carries ITS branch and arrows,
    /// the root's changes still count, and the list's word is that branch.
    #[test]
    fn one_nested_repo_is_the_checkout() {
        let Some(root) = workspace_scratch("one") else { return };
        let run = |d: &Path, args: &[&str]| {
            let out = Command::new("git").arg("-C").arg(d).args(args).output().unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        run(&root.join("web"), &["worktree", "remove", "--force", "../.wt-web"]);
        std::fs::remove_dir_all(root.join("web")).unwrap();
        run(&root.join("api"), &["checkout", "-q", "-b", "feat"]);
        std::fs::write(root.join("api/server.ts"), "one\nCHANGED\n").unwrap();
        std::fs::write(root.join("CLAUDE.md"), "# ws\nmore\n").unwrap();
        let g = crate::gitstatus::sample(&root);
        assert_eq!(g.repos, vec!["api"]);
        assert_eq!((g.branch.as_str(), g.changed), ("feat", 2), "{g:?}");
        assert_eq!(crate::gitstatus::branch_dir(&root, &g), root.join("api"));
        match checkout_diff_list(&root).unwrap() {
            Response::DiffList { branch, files, .. } => {
                assert_eq!(branch, "feat");
                let paths: Vec<String> = files.into_iter().map(|f| f.path).collect();
                assert_eq!(paths, vec!["CLAUDE.md", "api/server.ts"]);
            }
            _ => unreachable!(),
        }
        std::fs::remove_dir_all(&root).ok();
    }
}
