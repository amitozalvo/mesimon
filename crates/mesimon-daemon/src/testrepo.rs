//! Scratch git repositories for the daemon's unit tests — one place, so the
//! worktree, diff and gitstatus tests build the same shapes.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Run git in `d`, asserting success.
pub(crate) fn run(d: &Path, args: &[&str]) {
    let out = Command::new("git").arg("-C").arg(d).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// Run git in `d` and return its stdout, asserting success.
pub(crate) fn read(d: &Path, args: &[&str]) -> String {
    let out = Command::new("git").arg("-C").arg(d).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// `git init` on `main` with a test identity and one commit of `file`.
pub(crate) fn init(d: &Path, file: &str, body: &str) {
    std::fs::create_dir_all(d).unwrap();
    run(d, &["init", "-q", "-b", "main"]);
    run(d, &["config", "user.email", "t@t"]);
    run(d, &["config", "user.name", "t"]);
    std::fs::write(d.join(file), body).unwrap();
    run(d, &["add", "."]);
    run(d, &["commit", "-qm", "init"]);
}

/// Write `file` and commit it.
pub(crate) fn commit(d: &Path, file: &str, body: &str, msg: &str) {
    std::fs::write(d.join(file), body).unwrap();
    run(d, &["add", "."]);
    run(d, &["commit", "-qm", msg]);
}

/// The author's shape in miniature (T-225): a meta repo tracking its own
/// notes and ignoring every child, over two independent repos, plus a
/// gitfile child (a worktree of `web`) and a declared submodule, neither of
/// which is a workspace repo. `None` without git.
pub(crate) fn workspace_scratch(name: &str) -> Option<PathBuf> {
    if !crate::worktree::have_git() {
        return None;
    }
    let root = std::env::temp_dir().join(format!("msmn-ws-{name}-{}", std::process::id()));
    std::fs::remove_dir_all(&root).ok();
    init(&root, "CLAUDE.md", "# ws\n");
    std::fs::write(root.join(".gitignore"), "*/\n").unwrap();
    std::fs::write(root.join(".gitmodules"), "[submodule \"v\"]\n\tpath = vendored\n\turl = x\n")
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
