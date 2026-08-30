//! Build identity: the short git sha and build date that `mesimon --version`
//! and the doctor header print (16 §8.1 shows `v0.4.1 (a1b2c3d, 2026-08-20)`).
//! A bug report that names an exact commit is worth the 20 ms this costs.

use std::process::Command;

fn main() {
    // Re-run when HEAD moves, so the sha never goes stale. `.git` may be a
    // FILE in a worktree (gitdir: pointer) — watch it either way; when it is
    // absent entirely (a release tarball) the placeholders below stand.
    for p in [".git/HEAD", ".git/refs/heads", "../../.git/HEAD", "../../.git/refs/heads"] {
        if std::path::Path::new(p).exists() {
            println!("cargo:rerun-if-changed={p}");
        }
    }

    let sha = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".into());
    // A dirty tree is worth saying out loud: it means the sha does not fully
    // describe the binary.
    let dirty = git(&["status", "--porcelain"]).is_some_and(|s| !s.trim().is_empty());
    let sha = if dirty { format!("{sha}+dirty") } else { sha };
    let date = Command::new("date")
        .args(["-u", "+%Y-%m-%d"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".into());

    println!("cargo:rustc-env=MESIMON_GIT_SHA={sha}");
    println!("cargo:rustc-env=MESIMON_BUILD_DATE={date}");
}

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
