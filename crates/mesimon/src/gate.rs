//! `mesimon gate` — the static deny-only deciding hook (D10, T-84).
//!
//! Registered as a `PreToolUse` hook on `Edit`/`Write`/`NotebookEdit` for
//! Claude or `apply_patch` for Codex, it
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
    let mut resolved = args.to_vec();
    if args.iter().any(|a| a == "--from-env") {
        for (key, variable) in [
            ("--session", "MESIMON_SESSION"),
            ("--sock", "MESIMON_HOOK_SOCK"),
            ("--deny-board", "MESIMON_GATE_BOARD"),
            ("--deny-state", "MESIMON_GATE_STATE"),
            ("--allow", "MESIMON_GATE_ALLOW"),
        ] {
            if let Ok(value) = std::env::var(variable) {
                resolved.extend([key.into(), value]);
            }
        }
    }
    decide(&resolved);
    std::process::exit(0);
}

fn decide(args: &[String]) {
    // Read stdin to EOF before anything else — a short read EPIPEs the agent
    // on a large payload, and the payload carries the file list.
    let mut body = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut body);

    let payload: serde_json::Value =
        serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
    let cwd = payload.get("cwd").and_then(|c| c.as_str()).unwrap_or_default();
    if args.iter().any(|a| a == "--from-env")
        && payload["tool_name"] == "apply_patch"
        && !guard_context_complete(args)
    {
        // A stable trusted definition gets its roots from this invocation.
        // Losing that context cannot turn a protected write into no opinion.
        let out = serde_json::json!({"hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": "Mesimon write guard context is unavailable",
        }});
        let _ = std::io::stdout().write_all(out.to_string().as_bytes());
        return;
    }

    let mut roots = Vec::new();
    if let Some(p) = val(args, "--deny-board") {
        roots.push((RuleId::BoardDir, PathBuf::from(p)));
    }
    if let Some(p) = val(args, "--deny-state") {
        roots.push((RuleId::StateDir, PathBuf::from(p)));
    }
    let allow: Vec<PathBuf> = vals(args, "--allow").map(PathBuf::from).collect();

    let Some((rule, file_path)) = first_denied_path(&payload, args, cwd, &roots, &allow) else {
        return; // No opinion: ordinary writes and unrelated tools stay silent.
    };
    let verdict = Verdict::deny(rule);
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

fn guard_context_complete(args: &[String]) -> bool {
    ["--deny-board", "--deny-state", "--allow"]
        .iter()
        .all(|key| val(args, key).is_some_and(|root| Path::new(root).is_absolute()))
}

/// Normalize provider-specific structured writes before applying shared path
/// rules. Provider selection is explicit; legacy invocations remain Claude.
fn first_denied_path<'a>(
    payload: &'a serde_json::Value,
    args: &[String],
    cwd: &str,
    roots: &[(RuleId, PathBuf)],
    allow: &[PathBuf],
) -> Option<(RuleId, &'a str)> {
    structured_paths(payload, val(args, "--provider"))
        .into_iter()
        .find_map(|path| guarded_by(path, cwd, roots, allow).map(|rule| (rule, path)))
}

fn structured_paths<'a>(payload: &'a serde_json::Value, provider: Option<&str>) -> Vec<&'a str> {
    let input = &payload["tool_input"];
    match provider {
        Some("codex") if payload["tool_name"] == "apply_patch" => {
            input["command"].as_str().map(codex_patch_paths).unwrap_or_default()
        }
        // `NotebookEdit` names its file `notebook_path` (T-577 found the
        // gate never judged one).
        None | Some("claude") => ["file_path", "notebook_path"]
            .into_iter()
            .filter_map(|key| input[key].as_str())
            .filter(|path| !path.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

/// Codex 0.153.4's apply_patch grammar names paths in these four headers.
/// A rename writes its destination AND deletes its source. Patch content is
/// prefixed with '+', '-', or a space, so headers embedded in file contents
/// cannot become destinations. We do not parse shell commands or apply hunks;
/// Codex owns patch validity. Even an invalid patch may receive a denial when
/// it names a protected path. No early return may skip a later file header.
fn codex_patch_paths(patch: &str) -> Vec<&str> {
    const HEADERS: [&str; 4] =
        ["*** Add File: ", "*** Delete File: ", "*** Update File: ", "*** Move to: "];
    patch
        .lines()
        .filter_map(|line| HEADERS.iter().find_map(|header| line.strip_prefix(header)))
        // Codex trims path headers. Also inspect the literal spelling so a
        // future parser preserving surrounding spaces cannot evade the gate.
        .flat_map(|path| [path, path.trim()])
        .filter(|path| !path.is_empty())
        .collect()
}

/// Which rule a write lands under, if any.
///
/// Both sides are resolved before comparing, because on macOS `/tmp` is a
/// symlink to `/private/tmp` and a lexical comparison would miss every path
/// that reached the guarded tree by a different name. `..` is folded
/// lexically first, so `<repo>/src/../.mesimon/x` is caught even though no
/// component of it exists yet.
///
/// `allow` is checked first: a subtree inside a guarded root that is the
/// agent's own to write. Ticket worktrees live under the state dir, so
/// without it every edit in a worktree session was refused.
pub fn guarded_by(
    file_path: &str,
    cwd: &str,
    roots: &[(RuleId, PathBuf)],
    allow: &[PathBuf],
) -> Option<RuleId> {
    let mut p = PathBuf::from(file_path);
    if p.is_relative() && !cwd.is_empty() {
        p = PathBuf::from(cwd).join(p);
    }
    let target = resolve(&p);
    if allow.iter().any(|a| target.starts_with(resolve(a))) {
        return None;
    }
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

fn val<'a>(args: &'a [String], key: &'a str) -> Option<&'a str> {
    vals(args, key).next()
}

/// Every value of a repeatable flag, in argv order.
fn vals<'a>(args: &'a [String], key: &'a str) -> impl Iterator<Item = &'a str> + 'a {
    args.windows(2).filter(move |w| w[0] == key).map(|w| w[1].as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codex_denial(patch: &str, dir: &Path) -> Option<(RuleId, String)> {
        let payload = serde_json::json!({
            "tool_name": "apply_patch",
            "tool_input": { "command": patch },
        });
        first_denied_path(
            &payload,
            &["--provider".into(), "codex".into()],
            &dir.join("repo").display().to_string(),
            &roots(dir),
            &[dir.join("state/worktrees")],
        )
        .map(|(rule, path)| (rule, path.to_owned()))
    }

    #[test]
    fn trusted_environment_hook_needs_all_absolute_guard_roots() {
        let args = [
            "--deny-board",
            "/repo/.mesimon",
            "--deny-state",
            "/state",
            "--allow",
            "/state/worktrees",
        ]
        .map(str::to_string);
        assert!(guard_context_complete(&args));
        assert!(!guard_context_complete(&args[..4]));
        let mut relative = args.clone();
        relative[1] = ".mesimon".into();
        assert!(!guard_context_complete(&relative));
        assert!(!guard_context_complete(&[]));
    }

    #[test]
    fn codex_checks_later_files_after_an_ordinary_edit() {
        let d = tmp();
        let patch = "*** Begin Patch\n*** Add File: src/new.rs\n+ordinary\n\
            *** Update File: .mesimon/board/columns.toml\n@@\n-old\n+new\n*** End Patch";
        let (rule, path) = codex_denial(patch, &d).unwrap();
        assert_eq!(rule, RuleId::BoardDir);
        assert_eq!(path, ".mesimon/board/columns.toml");
        assert_eq!(
            Verdict::deny(rule).to_hook_output().unwrap(),
            serde_json::json!({"hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": rule.reason(),
            }})
        );
    }

    #[test]
    fn codex_checks_deletions_and_both_rename_paths() {
        let d = tmp();
        for operation in [
            "*** Delete File: .mesimon/board/columns.toml",
            "*** Update File: src/main.rs\n*** Move to: .mesimon/board/columns.toml\n@@\n-a\n+b",
            "*** Update File: .mesimon/board/columns.toml\n*** Move to: src/main.rs\n@@\n-a\n+b",
            "*** Add File: src/../.mesimon/board/columns.toml\n+x",
            "*** Add File:  .mesimon/board/columns.toml  \n+x",
        ] {
            let patch = format!("*** Begin Patch\n{operation}\n*** End Patch");
            assert_eq!(codex_denial(&patch, &d).unwrap().0, RuleId::BoardDir, "{operation}");
        }
    }

    #[test]
    fn codex_preserves_worktree_allowlist_and_checks_state_beside_it() {
        let d = tmp();
        let allowed = d.join("state/worktrees/T-1/src/new.rs");
        let patch =
            format!("*** Begin Patch\n*** Add File: {}\n+x\n*** End Patch", allowed.display());
        assert!(codex_denial(&patch, &d).is_none());
        let patch = format!(
            "*** Begin Patch\n*** Add File: {}\n+x\n*** Delete File: {}\n*** End Patch",
            allowed.display(),
            d.join("state/sessions.json").display(),
        );
        assert_eq!(codex_denial(&patch, &d).unwrap().0, RuleId::StateDir);
    }

    #[test]
    fn codex_checks_symlink_destinations() {
        let d = tmp();
        let link = d.join("repo/src").join(format!("guard-link-{}", uuid::Uuid::new_v4()));
        std::os::unix::fs::symlink(d.join("repo/.mesimon"), &link).unwrap();
        let patch = format!(
            "*** Begin Patch\n*** Add File: {}/new.toml\n+x\n*** End Patch",
            link.display(),
        );
        let denial = codex_denial(&patch, &d);
        std::fs::remove_file(link).unwrap();
        assert_eq!(denial.unwrap().0, RuleId::BoardDir);
    }

    #[test]
    fn codex_patch_content_cannot_masquerade_as_a_path_header() {
        let d = tmp();
        let patch = concat!(
            "*** Begin Patch\r\n*** Update File: src/main.rs\r\n@@\r\n",
            "+*** Add File: .mesimon/board/columns.toml\r\n",
            "-*** Delete File: .mesimon/board/columns.toml\r\n",
            " *** Move to: .mesimon/board/columns.toml\r\n*** End Patch\r\n",
        );
        assert!(codex_denial(patch, &d).is_none());
        assert!(codex_denial("*** Begin Patch\n*** Add File: \n+x\n*** End Patch", &d).is_none());
    }

    #[test]
    fn provider_is_explicit_and_codex_never_parses_shell_input() {
        let payload = serde_json::json!({
            "tool_name": "apply_patch",
            "tool_input": {"command": "*** Begin Patch\n*** Delete File: .mesimon/a\n*** End Patch"},
        });
        assert!(structured_paths(&payload, None).is_empty());
        assert!(structured_paths(&payload, Some("claude")).is_empty());
        assert!(!structured_paths(&payload, Some("codex")).is_empty());
        let mut shell = payload;
        shell["tool_name"] = serde_json::json!("Bash");
        assert!(structured_paths(&shell, Some("codex")).is_empty());
        let notebook = serde_json::json!({"tool_name": "NotebookEdit",
            "tool_input": {"notebook_path": ".mesimon/a.ipynb", "new_source": "x"}});
        assert_eq!(structured_paths(&notebook, None), vec![".mesimon/a.ipynb"]);
        let legacy = serde_json::json!({"tool_input": {"file_path": ".mesimon/a"}});
        assert_eq!(structured_paths(&legacy, None), vec![".mesimon/a"]);
        assert_eq!(structured_paths(&legacy, Some("claude")), vec![".mesimon/a"]);
        assert!(structured_paths(&legacy, Some("codex")).is_empty());
    }

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
        assert_eq!(
            guarded_by(&p.display().to_string(), "", &roots(&d), &[]),
            Some(RuleId::BoardDir)
        );
    }

    #[test]
    fn a_state_write_is_denied() {
        let d = tmp();
        let p = d.join("state/sessions.json");
        assert_eq!(
            guarded_by(&p.display().to_string(), "", &roots(&d), &[]),
            Some(RuleId::StateDir)
        );
    }

    /// The common case, and the one that must stay fast and quiet: ordinary
    /// source edits get no opinion at all.
    #[test]
    fn ordinary_source_is_untouched() {
        let d = tmp();
        let p = d.join("repo/src/main.rs");
        assert_eq!(guarded_by(&p.display().to_string(), "", &roots(&d), &[]), None);
    }

    /// `<repo>/src/../.mesimon/x` names a guarded file by a path in which no
    /// component of the guarded part exists yet.
    #[test]
    fn dot_dot_cannot_walk_in_sideways() {
        let d = tmp();
        let p = d.join("repo/src/../.mesimon/board/columns.toml");
        assert_eq!(
            guarded_by(&p.display().to_string(), "", &roots(&d), &[]),
            Some(RuleId::BoardDir)
        );
    }

    /// A relative `file_path` is resolved against the payload's `cwd`, which
    /// is what a session running inside its own worktree actually sends.
    #[test]
    fn a_relative_path_resolves_against_cwd() {
        let d = tmp();
        let cwd = d.join("repo");
        assert_eq!(
            guarded_by(".mesimon/board/columns.toml", &cwd.display().to_string(), &roots(&d), &[]),
            Some(RuleId::BoardDir)
        );
        assert_eq!(guarded_by("src/main.rs", &cwd.display().to_string(), &roots(&d), &[]), None);
    }

    /// A file that does not exist yet still resolves: Write creates files, and
    /// a gate that only caught edits to existing files would catch nothing on
    /// the first attempt.
    #[test]
    fn a_file_that_does_not_exist_yet_is_still_judged() {
        let d = tmp();
        let p = d.join("repo/.mesimon/board/tickets/T-9/ticket.toml");
        assert!(!p.exists());
        assert_eq!(
            guarded_by(&p.display().to_string(), "", &roots(&d), &[]),
            Some(RuleId::BoardDir)
        );
    }

    /// A directory whose name merely starts with a guarded root's name is not
    /// under it. `starts_with` on `Path` is component-wise, and this is the
    /// test that says so on purpose.
    #[test]
    fn a_sibling_with_a_shared_prefix_is_not_guarded() {
        let d = tmp();
        std::fs::create_dir_all(d.join("repo/.mesimon-notes")).unwrap();
        let p = d.join("repo/.mesimon-notes/x.md");
        assert_eq!(guarded_by(&p.display().to_string(), "", &roots(&d), &[]), None);
    }

    #[test]
    fn no_roots_means_no_opinion() {
        assert_eq!(guarded_by("/anything/at/all", "", &[], &[]), None);
    }

    /// Ticket worktrees are provisioned UNDER the state dir. The gate must
    /// let the agent edit its own checkout, or a worktree session cannot
    /// write a single file.
    #[test]
    fn a_worktree_under_the_state_dir_is_the_agents_own() {
        let d = tmp();
        std::fs::create_dir_all(d.join("state/worktrees/T-1-x/src")).unwrap();
        let allow = vec![d.join("state/worktrees")];
        let p = d.join("state/worktrees/T-1-x/src/main.rs");
        assert_eq!(guarded_by(&p.display().to_string(), "", &roots(&d), &allow), None);
        // The exemption is the worktrees subtree and nothing beside it.
        let s = d.join("state/sessions.json");
        assert_eq!(
            guarded_by(&s.display().to_string(), "", &roots(&d), &allow),
            Some(RuleId::StateDir)
        );
    }
}
