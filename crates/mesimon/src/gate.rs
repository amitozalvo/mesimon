//! `mesimon gate` — the one deciding hook (D10, T-84).
//!
//! Registered as a `PreToolUse` hook on `Edit`/`Write`/`NotebookEdit`, it
//! refuses a structured write into a path mesimon owns and says nothing about
//! anything else. It is a separate subcommand from `mesimon hook` on purpose:
//! the observer's invariant is that it NEVER writes stdout (stdout on several
//! hook events is injected straight into the agent's context), and teaching it
//! to sometimes emit a decision would put that invariant one refactor away
//! from being untrue.
//!
//! Three properties, in the order they matter:
//!
//! 1. **Deny or nothing.** `Verdict` has no `Allow` variant. mesimon can
//!    tighten what the user already permitted; it can never widen it. `"ask"`
//!    is never emitted either — it collapses to a deny in headless, which
//!    would turn "no opinion" into a silent refusal in exactly the background
//!    session path.
//! 2. **The decision is local and static.** The guarded roots arrive in argv;
//!    no daemon round trip is involved, so the gate is sub-millisecond on the
//!    agent's critical path and, more importantly, a dead daemon cannot make
//!    it fail open. Reporting the denial to the feed happens afterwards and is
//!    allowed to fail.
//! 3. **`Bash` is not hooked, and that is written down.** `file_path` is a real
//!    argument, so this check is exact. A command string is not, and any
//!    shape-matching pre-filter is an evasion hole (`D=.mesimon; sed -i ...
//!    $D/...`). The honest scope is: the tiers govern mesimon's tools and its
//!    structured writes, not the agent's shell.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use mesimon_core::verdict::{RuleId, Verdict};

/// Same hard self-abort as the observer: a hook wedged on a slow filesystem
/// must never be what stalls the agent.
const ABORT_MS: u64 = 500;
const WRITE_TIMEOUT_MS: u64 = 250;

pub fn run(args: &[String]) -> ! {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(ABORT_MS));
        std::process::exit(0);
    });
    decide(args);
    std::process::exit(0);
}

fn decide(args: &[String]) {
    // Read stdin to EOF before anything else — a short read EPIPEs the agent
    // on a large payload, and the payload carries the file list.
    let mut body = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut body);

    let payload: serde_json::Value =
        serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
    let file_path = payload
        .get("tool_input")
        .and_then(|i| i.get("file_path"))
        .and_then(|p| p.as_str())
        .unwrap_or_default();
    if file_path.is_empty() {
        return; // nothing to judge; no opinion
    }
    let cwd = payload.get("cwd").and_then(|c| c.as_str()).unwrap_or_default();

    let mut roots = Vec::new();
    if let Some(p) = val(args, "--deny-board") {
        roots.push((RuleId::BoardDir, PathBuf::from(p)));
    }
    if let Some(p) = val(args, "--deny-state") {
        roots.push((RuleId::StateDir, PathBuf::from(p)));
    }

    let verdict = match guarded_by(file_path, cwd, &roots) {
        Some(rule) => Verdict::deny(rule),
        None => Verdict::NoOpinion,
    };
    // `NoOpinion` writes NOTHING. Not `{}`, not `"ask"`, not a newline.
    let Some(out) = verdict.to_hook_output() else { return };
    if let Ok(s) = serde_json::to_string(&out) {
        let mut stdout = std::io::stdout();
        let _ = stdout.write_all(s.as_bytes());
        let _ = stdout.flush();
    }
    if let Verdict::Deny { rule, .. } = &verdict {
        report(args, *rule, file_path);
    }
}

/// Which rule a write lands under, if any.
///
/// Both sides are resolved before comparing, because on macOS `/tmp` is a
/// symlink to `/private/tmp` and a lexical comparison would miss every path
/// that reached the guarded tree by a different name. `..` is folded
/// lexically first, so `<repo>/src/../.mesimon/x` is caught even though no
/// component of it exists yet.
pub fn guarded_by(file_path: &str, cwd: &str, roots: &[(RuleId, PathBuf)]) -> Option<RuleId> {
    let mut p = PathBuf::from(file_path);
    if p.is_relative() && !cwd.is_empty() {
        p = PathBuf::from(cwd).join(p);
    }
    let target = resolve(&p);
    roots.iter().find(|(_, root)| target.starts_with(resolve(root))).map(|(rule, _)| *rule)
}

/// Fold `.`/`..` lexically, then canonicalize the deepest ancestor that
/// exists and re-attach the rest. A file about to be created has no
/// canonical path of its own; its parent usually does.
fn resolve(p: &Path) -> PathBuf {
    let mut folded = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                folded.pop();
            }
            other => folded.push(other.as_os_str()),
        }
    }
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    let mut probe = folded.clone();
    loop {
        if let Ok(real) = probe.canonicalize() {
            let mut out = real;
            for part in tail.iter().rev() {
                out.push(part);
            }
            return out;
        }
        let Some(name) = probe.file_name().map(|n| n.to_os_string()) else { return folded };
        tail.push(name);
        if !probe.pop() {
            return folded;
        }
    }
}

/// Tell the daemon a rule fired, so the denial reaches the activity feed.
/// Best-effort by design: this runs after the decision is already on stdout.
fn report(args: &[String], rule: RuleId, file_path: &str) {
    let (Some(sock), Some(session)) = (val(args, "--sock"), val(args, "--session")) else {
        return;
    };
    let Ok(mut stream) = UnixStream::connect(sock) else { return };
    let _ = stream.set_write_timeout(Some(Duration::from_millis(WRITE_TIMEOUT_MS)));
    let header = serde_json::json!({
        "v": 1u32,
        "session": session,
        "event": "GateDenied",
        "reason": rule.tag(),
    });
    let Ok(mut buf) = serde_json::to_vec(&header) else { return };
    buf.push(b'\n');
    // The path, and nothing else. D11's rule is about prompt text; a refused
    // file path is the one fact the audit line exists to carry.
    if let Ok(p) = serde_json::to_vec(&serde_json::json!({ "file_path": file_path })) {
        buf.extend_from_slice(&p);
    }
    let _ = stream.write_all(&buf);
}

fn val<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
    args.iter().position(|a| a == key).and_then(|i| args.get(i + 1)).map(String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(dir: &Path) -> Vec<(RuleId, PathBuf)> {
        vec![(RuleId::BoardDir, dir.join("repo/.mesimon")), (RuleId::StateDir, dir.join("state"))]
    }

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("msmn-gate-{}", std::process::id()));
        std::fs::create_dir_all(d.join("repo/.mesimon/board/tickets")).unwrap();
        std::fs::create_dir_all(d.join("repo/src")).unwrap();
        std::fs::create_dir_all(d.join("state/hooks")).unwrap();
        d
    }

    #[test]
    fn a_board_write_is_denied() {
        let d = tmp();
        let p = d.join("repo/.mesimon/board/tickets/T-1/ticket.toml");
        assert_eq!(guarded_by(&p.display().to_string(), "", &roots(&d)), Some(RuleId::BoardDir));
    }

    #[test]
    fn a_state_write_is_denied() {
        let d = tmp();
        let p = d.join("state/sessions.json");
        assert_eq!(guarded_by(&p.display().to_string(), "", &roots(&d)), Some(RuleId::StateDir));
    }

    /// The common case, and the one that must stay fast and quiet: ordinary
    /// source edits get no opinion at all.
    #[test]
    fn ordinary_source_is_untouched() {
        let d = tmp();
        let p = d.join("repo/src/main.rs");
        assert_eq!(guarded_by(&p.display().to_string(), "", &roots(&d)), None);
    }

    /// `<repo>/src/../.mesimon/x` names a guarded file by a path in which no
    /// component of the guarded part exists yet.
    #[test]
    fn dot_dot_cannot_walk_in_sideways() {
        let d = tmp();
        let p = d.join("repo/src/../.mesimon/board/columns.toml");
        assert_eq!(guarded_by(&p.display().to_string(), "", &roots(&d)), Some(RuleId::BoardDir));
    }

    /// A relative `file_path` is resolved against the payload's `cwd`, which
    /// is what a session running inside its own worktree actually sends.
    #[test]
    fn a_relative_path_resolves_against_cwd() {
        let d = tmp();
        let cwd = d.join("repo");
        assert_eq!(
            guarded_by(".mesimon/board/columns.toml", &cwd.display().to_string(), &roots(&d)),
            Some(RuleId::BoardDir)
        );
        assert_eq!(guarded_by("src/main.rs", &cwd.display().to_string(), &roots(&d)), None);
    }

    /// A file that does not exist yet still resolves: Write creates files, and
    /// a gate that only caught edits to existing files would catch nothing on
    /// the first attempt.
    #[test]
    fn a_file_that_does_not_exist_yet_is_still_judged() {
        let d = tmp();
        let p = d.join("repo/.mesimon/board/tickets/T-9/ticket.toml");
        assert!(!p.exists());
        assert_eq!(guarded_by(&p.display().to_string(), "", &roots(&d)), Some(RuleId::BoardDir));
    }

    /// A directory whose name merely starts with a guarded root's name is not
    /// under it. `starts_with` on `Path` is component-wise, and this is the
    /// test that says so on purpose.
    #[test]
    fn a_sibling_with_a_shared_prefix_is_not_guarded() {
        let d = tmp();
        std::fs::create_dir_all(d.join("repo/.mesimon-notes")).unwrap();
        let p = d.join("repo/.mesimon-notes/x.md");
        assert_eq!(guarded_by(&p.display().to_string(), "", &roots(&d)), None);
    }

    #[test]
    fn no_roots_means_no_opinion() {
        assert_eq!(guarded_by("/anything/at/all", "", &[]), None);
    }
}
