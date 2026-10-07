//! What a ticket's agents cost (T-327) — mesimon's estimate, never a bill.
//!
//! The tokens are exact: every assistant message's `usage` in a Claude Code
//! transcript (one line per content block, each repeating the message's final
//! usage, so a message is counted once by its id), and a Codex rollout's
//! running `token_count` totals. The dollars are arithmetic: those tokens at
//! the model's published API list price, fetched from Anthropic's pricing
//! page on 2026-10-01. A plan subscriber pays a flat fee, so for them the
//! figure says what the work would have cost on the API — the words beside
//! it say so. A model this table does not know is counted and left unpriced
//! rather than guessed (every Codex model, today: no published price was
//! verified for this build).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Token counts by the five ways a provider bills them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Tokens {
    #[serde(default, rename = "in", skip_serializing_if = "is_zero")]
    pub input: u64,
    #[serde(default, rename = "out", skip_serializing_if = "is_zero")]
    pub output: u64,
    #[serde(default, rename = "w5m", skip_serializing_if = "is_zero")]
    pub write_5m: u64,
    #[serde(default, rename = "w1h", skip_serializing_if = "is_zero")]
    pub write_1h: u64,
    #[serde(default, rename = "rd", skip_serializing_if = "is_zero")]
    pub read: u64,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

impl Tokens {
    pub fn add(&mut self, o: &Tokens) {
        self.input += o.input;
        self.output += o.output;
        self.write_5m += o.write_5m;
        self.write_1h += o.write_1h;
        self.read += o.read;
    }

    pub fn total(&self) -> u64 {
        self.input + self.output + self.write_5m + self.write_1h + self.read
    }

    pub fn is_empty(&self) -> bool {
        self.total() == 0
    }
}

/// A model's published price, USD per million tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Price {
    pub input: f64,
    pub write_5m: f64,
    pub write_1h: f64,
    pub read: f64,
    pub output: f64,
}

/// Cache writes at 1.25× (5 minutes) and 2× (1 hour) the input price; reads
/// at the model's own fraction of it.
const fn priced(input: f64, read: f64, output: f64) -> Price {
    Price { input, write_5m: input * 1.25, write_1h: input * 2.0, read, output }
}

/// Anthropic's list prices (platform.claude.com/docs/en/about-claude/pricing,
/// read 2026-10-01), longest id first so `claude-opus-5-5` is not read as
/// `claude-opus-5`. A transcript's model id may carry a date suffix, which
/// the prefix match ignores.
const PRICES: &[(&str, Price)] = &[
    ("claude-fable-5-1", priced(10.0, 0.25, 50.0)),
    ("claude-mythos-5-1", priced(10.0, 0.25, 50.0)),
    ("claude-fable-5", priced(10.0, 1.0, 50.0)),
    ("claude-mythos-5", priced(10.0, 1.0, 50.0)),
    ("claude-opus-5-5", priced(4.0, 0.20, 20.0)),
    ("claude-opus-5", priced(5.0, 0.50, 25.0)),
    ("claude-opus-4-8", priced(5.0, 0.50, 25.0)),
    ("claude-opus-4-7", priced(5.0, 0.50, 25.0)),
    ("claude-opus-4-6", priced(5.0, 0.50, 25.0)),
    ("claude-opus-4-5", priced(5.0, 0.50, 25.0)),
    ("claude-opus-4-1", priced(15.0, 1.50, 75.0)),
    ("claude-opus-4", priced(15.0, 1.50, 75.0)),
    ("claude-sonnet-5-5", priced(2.0, 0.20, 10.0)),
    ("claude-sonnet-5", priced(2.0, 0.20, 10.0)),
    ("claude-sonnet-4", priced(3.0, 0.30, 15.0)),
    ("claude-haiku-4-5", priced(1.0, 0.10, 5.0)),
    ("claude-3-5-haiku", priced(0.80, 0.08, 4.0)),
];

/// The fast-mode suffix a model key wears: fast mode bills every token class
/// at twice the standard rate.
pub const FAST: &str = ":fast";

/// A model's price, or `None` where this build has none. `anthropic.` (a
/// Bedrock id) and an `@date` (a Vertex id) are read through; their
/// platforms bill at their own prices, and the figure stays an estimate.
pub fn price(model: &str) -> Option<Price> {
    let (base, fast) = match model.strip_suffix(FAST) {
        Some(b) => (b, true),
        None => (model, false),
    };
    let base = base.strip_prefix("anthropic.").unwrap_or(base);
    let base = base.split('@').next().unwrap_or(base);
    let p = PRICES.iter().find(|(id, _)| matches_id(base, id)).map(|(_, p)| *p)?;
    let k = if fast { 2.0 } else { 1.0 };
    Some(Price {
        input: p.input * k,
        write_5m: p.write_5m * k,
        write_1h: p.write_1h * k,
        read: p.read * k,
        output: p.output * k,
    })
}

/// `id` exactly, or `id` followed by a `-` and more (a version or a date);
/// `claude-opus-4` matches `claude-opus-4-20250514` but `claude-opus-5`
/// does not match `claude-opus-5-5` (the longer id sorts first and wins).
fn matches_id(model: &str, id: &str) -> bool {
    model == id || model.strip_prefix(id).is_some_and(|rest| rest.starts_with('-'))
}

/// USD for `tokens` of `model`, or `None` where it is unpriced.
pub fn usd(model: &str, t: &Tokens) -> Option<f64> {
    Some(usd_at(&price(model)?, t))
}

/// USD for `tokens` at price `p`: `usd` with the lookup done, for a reader
/// that prices many buckets of one model (T-688).
pub fn usd_at(p: &Price, t: &Tokens) -> f64 {
    let m = |n: u64, per: f64| n as f64 * per / 1_000_000.0;
    m(t.input, p.input)
        + m(t.output, p.output)
        + m(t.write_5m, p.write_5m)
        + m(t.write_1h, p.write_1h)
        + m(t.read, p.read)
}

/// One assistant message out of a Claude Code transcript.
#[derive(Debug, Clone, PartialEq)]
pub struct Turn {
    /// The message id and the request id: a message is written once per
    /// content block, every copy carrying the message's final usage.
    pub id: String,
    /// The model key: its id, with [`FAST`] when it ran in fast mode.
    pub model: String,
    pub at_ms: u64,
    pub tokens: Tokens,
}

/// One transcript line, read as an assistant message's usage — `None` for
/// anything else (a user line, a tool result, an error's synthetic message).
pub fn claude_turn(line: &str) -> Option<Turn> {
    // Most lines are not assistant messages; skip them before parsing.
    if !line.contains("\"assistant\"") || !line.contains("\"usage\"") {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    if v.get("type")?.as_str()? != "assistant" {
        return None;
    }
    let m = v.get("message")?;
    let model = m.get("model")?.as_str()?;
    if model.starts_with('<') {
        return None;
    }
    let u = m.get("usage")?;
    let n = |v: &Value, k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
    let creation = u.get("cache_creation").filter(|c| c.is_object());
    let (write_5m, write_1h) = match creation {
        Some(c) => (n(c, "ephemeral_5m_input_tokens"), n(c, "ephemeral_1h_input_tokens")),
        None => (n(u, "cache_creation_input_tokens"), 0),
    };
    let tokens = Tokens {
        input: n(u, "input_tokens"),
        output: n(u, "output_tokens"),
        write_5m,
        write_1h,
        read: n(u, "cache_read_input_tokens"),
    };
    let fast = u.get("speed").and_then(Value::as_str) == Some("fast");
    let id = format!(
        "{}|{}",
        m.get("id").and_then(Value::as_str).unwrap_or(""),
        v.get("requestId").and_then(Value::as_str).unwrap_or("")
    );
    let at_ms =
        v.get("timestamp").and_then(Value::as_str).and_then(crate::usage::rfc3339_ms).unwrap_or(0);
    let model = if fast { format!("{model}{FAST}") } else { model.to_string() };
    Some(Turn { id, model, at_ms, tokens })
}

/// A turn's count as a session's mod reports it (T-581): `turn.complete`'s
/// `usage`, the engine's own sum of the turn's requests in the API's
/// spelling, and the model that answered. The engine says neither how long a
/// cache write lives nor whether the turn ran fast, so a write reads as
/// 5-minute here, as an older transcript's does; the ledger moves it by what
/// the conversation's transcript says (`Ledger::count_mod`).
pub fn mod_turn(usage: &Value) -> Option<(String, Tokens)> {
    let model = usage.get("model")?.as_str()?;
    if model.is_empty() || model.starts_with('<') || model.len() > 128 {
        return None;
    }
    let n = |k: &str| usage.get(k).and_then(Value::as_u64).unwrap_or(0);
    let tokens = Tokens {
        input: n("input_tokens"),
        output: n("output_tokens"),
        write_5m: n("cache_creation_input_tokens"),
        write_1h: 0,
        read: n("cache_read_input_tokens"),
    };
    Some((model.to_string(), tokens))
}

/// What one Codex rollout line says about tokens.
#[derive(Debug, Clone, PartialEq)]
pub enum CodexLine {
    /// The model the next turns run on (`turn_context`).
    Model(String),
    /// The thread's running totals so far (`token_count`), and when.
    Totals { at_ms: u64, tokens: Tokens },
}

/// One Codex rollout line, or `None`. Codex counts cached input inside
/// `input_tokens`; it is moved to `read` here, the way it is billed.
pub fn codex_line(line: &str) -> Option<CodexLine> {
    if !line.contains("turn_context") && !line.contains("token_count") {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    let p = v.get("payload")?;
    match v.get("type")?.as_str()? {
        "turn_context" => {
            p.get("model").and_then(Value::as_str).map(|m| CodexLine::Model(m.into()))
        }
        "event_msg" if p.get("type")?.as_str()? == "token_count" => {
            let t = p.get("info")?.get("total_token_usage")?;
            let n = |k: &str| t.get(k).and_then(Value::as_u64).unwrap_or(0);
            let cached = n("cached_input_tokens");
            Some(CodexLine::Totals {
                at_ms: v
                    .get("timestamp")
                    .and_then(Value::as_str)
                    .and_then(crate::usage::rfc3339_ms)
                    .unwrap_or(0),
                tokens: Tokens {
                    input: n("input_tokens").saturating_sub(cached),
                    output: n("output_tokens"),
                    read: cached,
                    ..Tokens::default()
                },
            })
        }
        _ => None,
    }
}

/// The growth from one set of running totals to the next: a rollout's
/// totals only rise, so a fall means a new thread started counting.
pub fn delta(prev: &Tokens, now: &Tokens) -> Tokens {
    if now.input < prev.input || now.output < prev.output || now.read < prev.read {
        return *now;
    }
    Tokens {
        input: now.input - prev.input,
        output: now.output - prev.output,
        write_5m: now.write_5m.saturating_sub(prev.write_5m),
        write_1h: now.write_1h.saturating_sub(prev.write_1h),
        read: now.read - prev.read,
    }
}

/// One ticket's account, as the snapshot carries it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TicketCost {
    pub ticket: ulid::Ulid,
    /// Every priced token, at list price.
    #[serde(default)]
    pub usd: f64,
    /// Every token counted, priced or not.
    #[serde(default)]
    pub tokens: u64,
    /// Tokens of models this build has no price for.
    #[serde(default)]
    pub unpriced: u64,
    /// The priced part of the last 24 hours, 7 days and 30 days.
    #[serde(default)]
    pub day: f64,
    #[serde(default)]
    pub week: f64,
    #[serde(default)]
    pub month: f64,
}

impl TicketCost {
    /// Fold one bucket of `model`'s tokens, from the hour `hour` (unix
    /// hours), into the account as of `now_ms`.
    pub fn fold(&mut self, model: &str, t: &Tokens, hour: u64, now_ms: u64) {
        self.fold_at(price(model).as_ref(), t, hour, now_ms);
    }

    /// `fold` with the model's price looked up once by the caller (T-688),
    /// `None` for an unpriced model.
    pub fn fold_at(&mut self, price: Option<&Price>, t: &Tokens, hour: u64, now_ms: u64) {
        self.tokens += t.total();
        let Some(usd) = price.map(|p| usd_at(p, t)) else {
            self.unpriced += t.total();
            return;
        };
        self.usd += usd;
        let age_h = (now_ms / 3_600_000).saturating_sub(hour);
        if age_h < 24 {
            self.day += usd;
        }
        if age_h < 24 * 7 {
            self.week += usd;
        }
        if age_h < 24 * 30 {
            self.month += usd;
        }
    }
}

/// A dollar figure as the dialog and the ticket page say it: `$4.20`,
/// `$1,240`, `<$0.01`.
pub fn usd_word(usd: f64) -> String {
    if usd < 0.005 {
        return "<$0.01".into();
    }
    if usd < 1000.0 {
        return format!("${usd:.2}");
    }
    let whole = usd.round() as u64;
    let s = whole.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    format!("${out}")
}

/// A dollar figure as a card's corner says it, in a few cells: `<$1`, `$4`,
/// `$140`, `$1.2k`.
pub fn usd_corner(usd: f64) -> String {
    if usd < 1.0 {
        "<$1".into()
    } else if usd < 999.5 {
        format!("${usd:.0}")
    } else {
        format!("${:.1}k", usd / 1000.0)
    }
}

/// A token count in a word: `640`, `48k`, `1.9M`.
pub fn tokens_word(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{}k", n / 1000),
        _ => format!("{:.1}M", n as f64 / 1_000_000.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mod_turn_is_the_engines_four_counts_and_its_model() {
        let usage = serde_json::json!({
            "input_tokens": 3, "output_tokens": 40, "cache_read_input_tokens": 900,
            "cache_creation_input_tokens": 120, "model": "claude-sonnet-5-5"
        });
        let (model, t) = mod_turn(&usage).unwrap();
        assert_eq!(model, "claude-sonnet-5-5");
        assert_eq!(t, Tokens { input: 3, output: 40, write_5m: 120, write_1h: 0, read: 900 });
        // No model, a synthetic one or no object: nothing to count.
        assert!(mod_turn(&serde_json::json!({"input_tokens": 3})).is_none());
        assert!(mod_turn(&serde_json::json!({"model": "<synthetic>"})).is_none());
        assert!(mod_turn(&serde_json::json!(null)).is_none());
    }

    #[test]
    fn prices_match_the_published_table_and_the_longest_id_wins() {
        let fable = price("claude-fable-5-1").unwrap();
        assert_eq!(
            (fable.input, fable.write_5m, fable.write_1h, fable.read, fable.output),
            (10.0, 12.5, 20.0, 0.25, 50.0)
        );
        let opus = price("claude-opus-5-5").unwrap();
        assert_eq!((opus.input, opus.write_1h, opus.read, opus.output), (4.0, 8.0, 0.20, 20.0));
        assert_eq!(price("claude-opus-5").unwrap().input, 5.0, "not Opus 5.5's");
        assert_eq!(price("claude-opus-4-20250514").unwrap().input, 15.0);
        assert_eq!(price("claude-sonnet-4-5-20250929").unwrap().output, 15.0);
        assert_eq!(price("claude-haiku-4-5-20251001").unwrap().read, 0.10);
        assert_eq!(price("anthropic.claude-opus-5-5").unwrap().input, 4.0);
        assert_eq!(price("claude-opus-4-5@20251101").unwrap().input, 5.0);
        assert_eq!(price("claude-opus-5-5:fast").unwrap().output, 40.0, "fast bills double");
        assert_eq!(price("gpt-6-astra"), None);
        assert_eq!(price("claude-opus-55"), None, "a prefix is not an id");
    }

    #[test]
    fn a_transcript_message_is_read_once_with_its_cache_split() {
        let line = r#"{"type":"assistant","requestId":"req_1","timestamp":"2026-10-01T16:04:00.000Z","message":{"id":"msg_1","model":"claude-opus-5-5","usage":{"input_tokens":2,"output_tokens":286,"cache_creation_input_tokens":33858,"cache_read_input_tokens":28746,"cache_creation":{"ephemeral_1h_input_tokens":33858,"ephemeral_5m_input_tokens":0}}}}"#;
        let t = claude_turn(line).unwrap();
        assert_eq!(t.id, "msg_1|req_1");
        assert_eq!(t.model, "claude-opus-5-5");
        assert_eq!(t.at_ms, 1_790_870_640_000);
        assert_eq!(
            t.tokens,
            Tokens { input: 2, output: 286, write_5m: 0, write_1h: 33858, read: 28746 }
        );
        // 2×4 + 286×20 + 33858×8 + 28746×0.2, per million.
        let want = (2.0 * 4.0 + 286.0 * 20.0 + 33858.0 * 8.0 + 28746.0 * 0.2) / 1e6;
        assert!((usd(&t.model, &t.tokens).unwrap() - want).abs() < 1e-12);
        // An older usage without the split counts its writes as 5-minute ones.
        let old = r#"{"type":"assistant","message":{"id":"m","model":"claude-sonnet-4-5","usage":{"input_tokens":1,"output_tokens":1,"cache_creation_input_tokens":10}}}"#;
        assert_eq!(claude_turn(old).unwrap().tokens.write_5m, 10);
        let fast = r#"{"type":"assistant","message":{"id":"m","model":"claude-opus-5-5","usage":{"input_tokens":1,"speed":"fast"}}}"#;
        assert_eq!(claude_turn(fast).unwrap().model, "claude-opus-5-5:fast");
        assert!(claude_turn(r#"{"type":"user","message":{"usage":{}}}"#).is_none());
        let synthetic =
            r#"{"type":"assistant","message":{"model":"<synthetic>","usage":{"input_tokens":0}}}"#;
        assert!(claude_turn(synthetic).is_none());
    }

    #[test]
    fn codex_totals_move_cached_input_to_reads_and_deltas_restart() {
        let model = r#"{"timestamp":"2026-09-24T09:26:14.228Z","type":"turn_context","payload":{"model":"gpt-6-astra"}}"#;
        assert_eq!(codex_line(model), Some(CodexLine::Model("gpt-6-astra".into())));
        let totals = r#"{"timestamp":"2026-09-24T09:26:14.228Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":2711816,"cached_input_tokens":2499712,"output_tokens":12965,"reasoning_output_tokens":3223,"total_tokens":2724781}}}}"#;
        let Some(CodexLine::Totals { tokens, at_ms }) = codex_line(totals) else { panic!() };
        assert_eq!(at_ms, 1_790_241_974_228);
        assert_eq!((tokens.input, tokens.read, tokens.output), (212_104, 2_499_712, 12_965));
        let later = Tokens { input: 212_200, read: 2_500_000, output: 13_000, ..Tokens::default() };
        assert_eq!(
            delta(&tokens, &later),
            Tokens { input: 96, read: 288, output: 35, ..Tokens::default() }
        );
        let fresh = Tokens { input: 5, ..Tokens::default() };
        assert_eq!(delta(&tokens, &fresh), fresh, "a fall is a new count");
    }

    #[test]
    fn a_ticket_account_folds_windows_and_leaves_the_unpriced_unpriced() {
        let now = 1_790_870_640_000;
        let hour = now / 3_600_000;
        let mut c = TicketCost::default();
        let t = Tokens { output: 1_000_000, ..Tokens::default() };
        c.fold("claude-opus-5-5", &t, hour, now);
        c.fold("claude-opus-5-5", &t, hour - 30, now);
        c.fold("gpt-6-astra", &t, hour, now);
        assert_eq!((c.usd, c.day, c.week, c.month), (40.0, 20.0, 40.0, 40.0));
        assert_eq!((c.tokens, c.unpriced), (3_000_000, 1_000_000));
    }

    #[test]
    fn the_words() {
        assert_eq!(usd_word(4.2), "$4.20");
        assert_eq!(usd_word(1240.4), "$1,240");
        assert_eq!(usd_word(0.001), "<$0.01");
        assert_eq!(usd_corner(0.4), "<$1");
        assert_eq!(usd_corner(4.4), "$4");
        assert_eq!(usd_corner(140.0), "$140");
        assert_eq!(usd_corner(1234.0), "$1.2k");
        assert_eq!(tokens_word(48_500), "48k");
        assert_eq!(tokens_word(1_900_000), "1.9M");
    }
}
