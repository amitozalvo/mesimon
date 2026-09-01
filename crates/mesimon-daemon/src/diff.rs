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

/// Read-path git, bytes out. `--no-optional-locks` always.
fn git_bytes(repo: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let out =
        crate::git::git(repo).arg("--no-optional-locks").args(args).output().context("run git")?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(out.stdout)
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
    if let Ok(numstat) =
        git_bytes(repo, &["--no-pager", "diff", "--numstat", "-z", "--no-ext-diff", &range])
    {
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
}
