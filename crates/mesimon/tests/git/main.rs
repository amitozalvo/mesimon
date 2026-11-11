//! The `git` e2e binary: worktrees, the merge train, the forge, diffs, archive and reclaim.
//!
//! Each file beside this one is a module of it, so the area links once (T-1197).
//! A file without its `mod` line here is never compiled:
//! `guard::every_test_file_is_a_module` fails on one.

#[path = "../common/mod.rs"]
mod common;
#[path = "../common/guard.rs"]
mod guard;

mod archive_e2e;
mod archive_reclaim_e2e;
mod column_reclaim_e2e;
mod diff_e2e;
mod forge_e2e;
mod gitstatus_e2e;
mod merge_train_e2e;
mod pr_merge_e2e;
mod workspace_e2e;
mod worktree_e2e;
mod worktree_init_e2e;
