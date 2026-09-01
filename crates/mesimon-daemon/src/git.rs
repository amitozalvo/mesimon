//! The one way the daemon runs git.
//!
//! Every invocation goes through [`git`], which pins the repo with `-C` and
//! scrubs the `GIT_*` targeting variables. The daemon may itself have been
//! spawned from inside a worktree session (dogfooding), and an inherited
//! `GIT_DIR` would silently retarget every `-C` — including `worktree add`,
//! `branch -D` and the fast-forward merge. One builder, so no call site can
//! forget.

use std::path::Path;
use std::process::Command;

/// A `git -C <repo>` command with the environment scrubbed. Add args and run.
pub fn git(repo: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(repo);
    for var in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE"] {
        cmd.env_remove(var);
    }
    cmd
}
