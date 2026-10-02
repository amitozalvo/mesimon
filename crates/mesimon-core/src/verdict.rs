//! The deny-only hook verdict (D10).
//!
//! The static write-protection gate may only tighten permissions. Remote human
//! PermissionRequest answers use a separate type and subcommand (T-395).
//! Attention hooks (D15) stay pure observers with empty stdout.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Which rule fired. Named, not free text, because every denial is written to
/// the activity feed and the user has to be able to ask "what is this rule".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleId {
    /// A structured write into `<repo>/.mesimon/` — the board itself.
    BoardDir,
    /// A structured write into `~/.local/state/mesimon/<proj16>/` — sessions,
    /// worktree bindings, hook settings, the feed.
    StateDir,
}

impl RuleId {
    /// The rule's stable tag, for the feed and for `doctor`.
    pub fn tag(&self) -> &'static str {
        match self {
            RuleId::BoardDir => "board_dir",
            RuleId::StateDir => "state_dir",
        }
    }

    /// What the agent is told. Addressed to the agent, so it is allowed to be
    /// in the second person — this is a denial reason, not a tool description,
    /// and it is only ever emitted when a call is already being refused.
    pub fn reason(&self) -> &'static str {
        match self {
            RuleId::BoardDir => {
                "mesimon owns .mesimon/ — the board is edited through mesimon, not by writing \
                 its files. Use mesimon's scoped MCP tools instead."
            }
            RuleId::StateDir => {
                "mesimon owns its state directory — sessions, worktree bindings and hook \
                 settings are not editable by an agent."
            }
        }
    }
}

/// The ONLY value the static write-protection gate may return.
///
/// There is deliberately NO `Allow` variant. mesimon can tighten what the user
/// already permitted; this gate can never widen it. Adding a variant here is a change
/// to the product's security posture, not a feature request.
///
/// Deliberately NOT `#[non_exhaustive]`: that attribute exists to make ADDING a
/// variant a non-breaking change, which is the opposite of the intent. The enum
/// is closed and exhaustively matched in `to_hook_output`, so a new variant
/// fails to compile until someone edits the serializer on purpose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Deny { rule: RuleId, reason: String },
    NoOpinion,
}

impl Verdict {
    pub fn deny(rule: RuleId) -> Self {
        Verdict::Deny { rule, reason: rule.reason().to_string() }
    }

    /// What goes on stdout. `None` means write NOTHING and exit 0.
    ///
    /// `NoOpinion` MUST be empty stdout — never `{"decision":"ask"}`. `ask`
    /// collapses to a deny in headless, which would turn "no opinion" into a
    /// silent denial in exactly the background-session path.
    pub fn to_hook_output(&self) -> Option<Value> {
        match self {
            Verdict::NoOpinion => None,
            Verdict::Deny { reason, .. } => Some(json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "deny",
                    "permissionDecisionReason": reason,
                }
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire shape, byte for byte. If Claude Code's contract moves, this
    /// test is where it is noticed — not in a silently-ignored hook.
    #[test]
    fn deny_serializes_exactly() {
        let v = Verdict::Deny { rule: RuleId::BoardDir, reason: "nope".into() };
        let s = serde_json::to_string(&v.to_hook_output().unwrap()).unwrap();
        assert_eq!(
            s,
            r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"nope"}}"#
        );
    }

    #[test]
    fn no_opinion_is_empty_stdout_never_ask() {
        assert!(Verdict::NoOpinion.to_hook_output().is_none());
    }

    /// The type-system point, asserted as a fact rather than a comment: the
    /// only decision mesimon can emit is "deny".
    #[test]
    fn the_only_decision_word_is_deny() {
        for rule in [RuleId::BoardDir, RuleId::StateDir] {
            let out = Verdict::deny(rule).to_hook_output().unwrap();
            assert_eq!(out["hookSpecificOutput"]["permissionDecision"], "deny");
        }
    }

    /// A type alone is a comment. This walks the workspace source and asserts
    /// that no line which mentions `permissionDecision` also carries `"allow"`
    /// or `"ask"` — the two words that would turn this hook from something
    /// that can only tighten into something that can widen, or (for `"ask"`)
    /// into a silent deny in headless. The mod's TypeScript (T-577) is
    /// walked beside the Rust, in either quote.
    #[test]
    fn no_source_line_can_decide_allow_or_ask() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .expect("workspace root")
            .join("crates");
        let mut checked = 0usize;
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.flatten() {
                let path = e.path();
                if path.is_dir() {
                    // Claude Code's own declarations, laid into a mod folder
                    // at a load, are its words, not ours.
                    if path.file_name().is_none_or(|n| n != ".claude-plugin") {
                        stack.push(path);
                    }
                    continue;
                }
                if !matches!(path.extension().and_then(|x| x.to_str()), Some("rs" | "ts")) {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&path) else { continue };
                for (i, line) in text.lines().enumerate() {
                    // Comments may discuss the banned words — this very test's
                    // doc comment does. Only code can decide anything.
                    let code = line.trim_start();
                    if code.starts_with("//") || code.starts_with('*') {
                        continue;
                    }
                    if !line.contains("permissionDecision") {
                        continue;
                    }
                    checked += 1;
                    for banned in ["\"allow\"", "\"ask\"", "'allow'", "'ask'"] {
                        assert!(
                            !line.contains(banned),
                            "{}:{}: {banned} in a permissionDecision position",
                            path.display(),
                            i + 1
                        );
                    }
                }
            }
        }
        assert!(
            checked > 0,
            "the grep found no permissionDecision at all — it has stopped working"
        );
    }

    #[test]
    fn every_rule_has_a_tag_and_a_reason() {
        for rule in [RuleId::BoardDir, RuleId::StateDir] {
            assert!(!rule.tag().is_empty());
            assert!(rule.reason().len() > 40, "a denial the agent cannot act on is a dead end");
        }
    }
}
