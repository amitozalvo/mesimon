//! A session's context fill (T-969): how much of its model's context window
//! the conversation's last request took, as its transcript records it.
//!
//! Claude Code writes every request's `usage`; the request's input is
//! `input_tokens` + `cache_read_input_tokens` + the cache writes, which is
//! the whole prompt the model read, so the last main-chain reply's sum is
//! the conversation's size. A compaction writes a `compact_boundary` line
//! and no reply until the next turn, so the last reading then predates it
//! (measured on 2.1.310: a manual `/compact` at 736,597 tokens, the next
//! reply read 92,957; on a two-message conversation the next reply read
//! more than the last, the system prompt and tools being most of it), and
//! the boundary's own `postTokens` counts the kept messages without the
//! system prompt and tools, so it is no reading of the same thing. The
//! auto-compaction's own requests are written as replies with no usage.
//!
//! Codex writes each request's `last_token_usage` and the engine's own
//! `model_context_window` on every `token_count` line (0.164.0), and a
//! `compacted` line at a compaction, after which one `token_count` reads no
//! input.
//!
//! The window is Claude Code's own table for the model id, read out of
//! 2.1.310: the five, the Opus 4.7 and 4.8 and the Sonnet 5 lines are 1M
//! natively, the older lines 200k, and those of them whose table names a
//! 1M maximum are 1M under a `[1m]` model. A model the table does not know
//! has no window here, and no reading.

use serde::{Deserialize, Serialize};

use crate::command::AgentContextView;
use crate::cost::FAST;

const M: u64 = 1_000_000;
const K200: u64 = 200_000;

/// Claude Code 2.1.310's windows by model id, longest first so
/// `claude-opus-4-5` is not read as `claude-opus-4`: the window, and the
/// window a `[1m]` model gets.
const CLAUDE_WINDOWS: &[(&str, u64, u64)] = &[
    ("claude-3-7-sonnet", K200, M),
    ("claude-sonnet-4-5", K200, M),
    ("claude-sonnet-4-6", K200, M),
    ("claude-sonnet-5-5", M, M),
    ("claude-haiku-4-5", K200, K200),
    ("claude-haiku-5-5", M, M),
    ("claude-mythos-5", M, M),
    ("claude-fable-5-1", M, M),
    ("claude-opus-4-1", K200, K200),
    ("claude-opus-4-5", K200, K200),
    ("claude-opus-4-6", K200, M),
    ("claude-opus-4-7", M, M),
    ("claude-opus-4-8", M, M),
    ("claude-opus-5-5", M, M),
    ("claude-sonnet-4", K200, M),
    ("claude-sonnet-5", M, M),
    ("claude-fable-5", M, M),
    ("claude-opus-4", K200, K200),
    ("claude-opus-5", M, M),
];

/// What a transcript last said about its conversation's size, kept on the
/// cost ledger's cursor beside how far it was read.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Fill {
    /// The last request's input, cached or not.
    #[serde(default)]
    pub tokens: u64,
    /// The model it ran on, as the transcript names it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    /// The window the engine itself named (Codex's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<u64>,
    /// A compaction came after the last request.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub compacted: bool,
}

/// Claude Code's window for `model`, `one_m` when the session's `--model`
/// names `[1m]`; `None` for a model its table does not know. A Bedrock id
/// (`anthropic.`) and a Vertex date (`@…`) are read through, as `cost::price`
/// reads them.
pub fn claude_window(model: &str, one_m: bool) -> Option<u64> {
    let base = model.strip_suffix(FAST).unwrap_or(model);
    let base = base.strip_prefix("anthropic.").unwrap_or(base);
    let base = base.split('@').next().unwrap_or(base);
    let mut table: Vec<&(&str, u64, u64)> = CLAUDE_WINDOWS.iter().collect();
    table.sort_by_key(|(id, ..)| std::cmp::Reverse(id.len()));
    table
        .into_iter()
        .find(|(id, ..)| crate::cost::matches_id(base, id))
        .map(|&(_, window, wide)| if one_m { wide } else { window })
}

/// The reading as `get_ticket` carries it, against `window`.
pub fn view(fill: &Fill, window: u64) -> Option<AgentContextView> {
    if window == 0 {
        return None;
    }
    let measured = !fill.compacted && fill.tokens > 0;
    Some(AgentContextView {
        tokens: measured.then_some(fill.tokens),
        window,
        pct: measured.then(|| (fill.tokens * 100 + window / 2) / window),
        compacted: fill.compacted,
    })
}

/// A Claude session's reading: the model's window from the table, wide
/// where the session's `--model` (`model_arg`) names `[1m]`.
pub fn claude_view(fill: &Fill, model_arg: Option<&str>) -> Option<AgentContextView> {
    let one_m = model_arg.is_some_and(|m| m.ends_with("[1m]"));
    view(fill, claude_window(&fill.model, one_m)?)
}

/// A Codex session's reading: the window its own rollout named.
pub fn codex_view(fill: &Fill) -> Option<AgentContextView> {
    view(fill, fill.window?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill(tokens: u64, model: &str) -> Fill {
        Fill { tokens, model: model.into(), ..Fill::default() }
    }

    #[test]
    fn the_window_is_claude_codes_table_and_one_m_widens_only_where_it_may() {
        assert_eq!(claude_window("claude-opus-5-5", false), Some(M));
        assert_eq!(claude_window("claude-fable-5-1", false), Some(M));
        assert_eq!(claude_window("claude-sonnet-5-5:fast", false), Some(M));
        assert_eq!(claude_window("claude-opus-4-5-20251101", false), Some(K200), "not opus-4's");
        assert_eq!(claude_window("claude-opus-4-20250514", false), Some(K200));
        assert_eq!(claude_window("claude-sonnet-4-5-20250929", false), Some(K200));
        assert_eq!(claude_window("claude-sonnet-4-5-20250929", true), Some(M));
        assert_eq!(claude_window("claude-opus-4-6", true), Some(M));
        assert_eq!(claude_window("claude-opus-4-5", true), Some(K200), "no 1M maximum");
        assert_eq!(claude_window("claude-haiku-4-5-20251001", true), Some(K200));
        assert_eq!(claude_window("anthropic.claude-opus-5-5", false), Some(M));
        assert_eq!(claude_window("claude-opus-4-6@20260101", false), Some(K200));
        assert_eq!(claude_window("claude-opus-55", false), None, "a prefix is not an id");
        assert_eq!(claude_window("gpt-6-astra", false), None);
        assert_eq!(claude_window("", false), None);
    }

    #[test]
    fn the_reading_is_the_share_of_the_window_and_absent_where_unknown() {
        let v = claude_view(&fill(800_000, "claude-opus-5-5"), Some("opus")).unwrap();
        assert_eq!((v.tokens, v.window, v.pct, v.compacted), (Some(800_000), M, Some(80), false));
        // Rounded to the nearest point.
        let v = claude_view(&fill(150_000, "claude-sonnet-4-5"), Some("sonnet")).unwrap();
        assert_eq!((v.window, v.pct), (K200, Some(75)));
        let v = claude_view(&fill(150_000, "claude-sonnet-4-5"), Some("sonnet[1m]")).unwrap();
        assert_eq!((v.window, v.pct), (M, Some(15)));
        assert_eq!(view(&fill(1_995, "x"), 1_000).unwrap().pct, Some(200), "said as it is");
        assert!(claude_view(&fill(10, "claude-opus-55"), None).is_none(), "an unknown model");
        // Codex says its own window; none said, no reading.
        let mut codex = fill(244_733, "gpt-6-astra");
        assert!(codex_view(&codex).is_none());
        codex.window = Some(258_400);
        assert_eq!(codex_view(&codex).unwrap().pct, Some(95));
        assert!(view(&codex, 0).is_none());
    }

    #[test]
    fn after_a_compaction_the_reading_waits_for_the_next_reply() {
        let mut f = fill(736_597, "claude-fable-5-1");
        f.compacted = true;
        let v = claude_view(&f, None).unwrap();
        assert_eq!((v.tokens, v.pct, v.compacted, v.window), (None, None, true, M));
        let json = serde_json::to_value(&v).unwrap();
        assert_eq!(json, serde_json::json!({ "window": 1_000_000, "compacted": true }));
        let v = claude_view(&fill(92_957, "claude-fable-5-1"), None).unwrap();
        let json = serde_json::to_value(&v).unwrap();
        assert_eq!(json, serde_json::json!({ "tokens": 92_957, "window": 1_000_000, "pct": 9 }));
        // No request measured yet: the window alone.
        let v = claude_view(&fill(0, "claude-fable-5-1"), None).unwrap();
        assert_eq!((v.tokens, v.pct), (None, None));
    }
}
