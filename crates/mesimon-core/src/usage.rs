//! Subscription quota (T-327): how much of each plan is left, in the
//! provider's own numbers.
//!
//! Two providers report it, each in its own shape, and this module turns both
//! into one [`Reading`]: Claude Code's `get_usage` control request (the rows
//! its `/usage` screen draws) and Codex's rate-limit snapshot (an app-server
//! `account/rateLimits/*` answer, or a rollout's `token_count` event). Every
//! percentage here is the provider's; mesimon estimates none of them.
//!
//! Claude grades its own rows (`severity`) and names its own headline
//! (`is_active`), and its schema tells a client never to grade a row itself,
//! so both are carried verbatim. Codex sends neither: [`grade`] reads a
//! severity off the percentage, and the fullest window is its headline. The
//! same goes for an older Claude Code that answers without the `limits` rows.
//!
//! [`Cadence`] decides when the daemon reads again: only while a board asked,
//! and then on the events that move a quota (a turn ending, a rate-limit
//! stop, a window rolling over), with a slow clock under them.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Whose plan a reading is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Claude,
    Codex,
}

impl Provider {
    pub const ALL: [Provider; 2] = [Provider::Claude, Provider::Codex];

    /// The word the line and the dialog lead a provider's group with.
    pub fn word(self) -> &'static str {
        match self {
            Provider::Claude => "claude",
            Provider::Codex => "codex",
        }
    }
}

/// How close a window is to its limit. Claude's is the server's own reading;
/// Codex's is [`grade`]'s.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    #[default]
    Normal,
    Warning,
    Critical,
}

/// What a window measures, classified on the server's kind — never on its
/// label, which is the server's to change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowKind {
    /// The rolling five-hour window.
    Session,
    /// The weekly window over every model.
    Weekly,
    /// A window scoped to one model (`Fable`, `Spark`).
    Model,
    /// Anything else a provider sends (a monthly window, a new meter): in the
    /// dialog always, on the line only when it warns.
    #[default]
    Other,
}

/// One quota window, ready to draw.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Window {
    pub kind: WindowKind,
    /// The line's word: `5h`, `week`, `Fable`.
    pub label: String,
    /// The dialog's words: `5-hour`, `week, all models`, `week, Fable`.
    pub long: String,
    /// Share of the window used, 0–100 as sent.
    pub percent: f64,
    /// When the window starts over, unix ms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at_ms: Option<u64>,
    #[serde(default)]
    pub severity: Severity,
    /// The provider's single-number pick: Claude's `is_active`, Codex's
    /// fullest window.
    #[serde(default)]
    pub headline: bool,
    /// The window's length, where it is known — the pace's denominator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length_mins: Option<u32>,
}

impl Window {
    /// The percentage as the line and the dialog print it.
    pub fn percent_word(&self) -> String {
        format!("{}%", self.percent.round().clamp(0.0, 999.0) as u32)
    }
}

/// What one provider reported, and when.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Reading {
    /// Unix ms of the answer.
    pub read_at_ms: u64,
    /// The plan word as the provider sends it: `max`, `pro`, `prolite`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    #[serde(default)]
    pub windows: Vec<Window>,
}

impl Reading {
    /// The one window a single number should show: the most severe, then the
    /// provider's own pick, then the fullest.
    pub fn headline(&self) -> Option<&Window> {
        self.windows
            .iter()
            .max_by(|a, b| rank(a).partial_cmp(&rank(b)).unwrap_or(std::cmp::Ordering::Equal))
    }

    /// The soonest moment a window starts over: past it, the reading
    /// describes a window that no longer exists.
    pub fn next_reset_ms(&self) -> Option<u64> {
        self.windows.iter().filter_map(|w| w.resets_at_ms).min()
    }

    pub fn worst(&self) -> Severity {
        self.windows.iter().map(|w| w.severity).max().unwrap_or_default()
    }
}

fn rank(w: &Window) -> (Severity, bool, f64) {
    (w.severity, w.headline, w.percent)
}

/// Why a provider has no reading to show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "detail")]
pub enum Problem {
    /// No plan limits apply: an API key, Bedrock or Vertex.
    NoPlan,
    /// Not signed in, or the sign-in expired.
    SignedOut,
    /// The provider's CLI is not on `PATH`.
    Missing,
    /// This version of the CLI cannot report usage.
    Unsupported,
    /// Anything else, in a few words.
    Failed(String),
}

impl Problem {
    /// What the Usage dialog says in place of the windows.
    pub fn words(&self, p: Provider) -> String {
        match (self, p) {
            (Problem::NoPlan, _) => {
                "no plan limits on this sign-in (an API key or a cloud provider)".into()
            }
            (Problem::SignedOut, Provider::Claude) => {
                "signed out ∙ run claude and /login, then r".into()
            }
            (Problem::SignedOut, Provider::Codex) => {
                "sign-in expired ∙ run codex login in a shell, then r".into()
            }
            (Problem::Missing, _) => format!("{} is not installed on this PATH", p.word()),
            (Problem::Unsupported, _) => {
                format!("this {} cannot report usage ∙ update it", p.word())
            }
            (Problem::Failed(why), _) => why.clone(),
        }
    }

    /// The short form, for the line's place in the menu row.
    pub fn short(&self) -> &'static str {
        match self {
            Problem::NoPlan => "no plan limits",
            Problem::SignedOut => "signed out",
            Problem::Missing => "not installed",
            Problem::Unsupported => "needs an update",
            Problem::Failed(_) => "could not read",
        }
    }

    /// A condition a retry an hour from now is as likely to fix as one now.
    pub fn is_lasting(&self) -> bool {
        !matches!(self, Problem::Failed(_))
    }
}

/// Where one provider stands: the last good reading (kept through a later
/// failure, since an old number with its age beats none) and the latest
/// attempt's verdict.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ProviderUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reading: Option<Reading>,
    /// Why the latest attempt read nothing; `None` once one has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<Problem>,
    /// Unix ms of the latest attempt, good or bad.
    #[serde(default)]
    pub tried_at_ms: u64,
}

impl ProviderUsage {
    /// A good reading landed: it replaces the last one and clears any problem.
    pub fn read(&mut self, reading: Reading) {
        self.tried_at_ms = self.tried_at_ms.max(reading.read_at_ms);
        self.reading = Some(reading);
        self.problem = None;
    }

    /// An attempt failed. A lasting problem (signed out, no plan) drops the
    /// old reading — its numbers describe a sign-in that no longer answers;
    /// a passing one keeps it, with its age.
    pub fn failed(&mut self, problem: Problem, now_ms: u64) {
        if problem.is_lasting() {
            self.reading = None;
        }
        self.problem = Some(problem);
        self.tried_at_ms = now_ms;
    }

    /// The newer of two accounts of one provider — what a daemon keeps when
    /// another board's daemon wrote the shared file.
    pub fn newer(self, other: ProviderUsage) -> ProviderUsage {
        if other.tried_at_ms > self.tried_at_ms {
            other
        } else {
            self
        }
    }
}

/// Which providers the attached boards want read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Wants {
    #[serde(default)]
    pub claude: bool,
    #[serde(default)]
    pub codex: bool,
}

impl Wants {
    pub fn get(self, p: Provider) -> bool {
        match p {
            Provider::Claude => self.claude,
            Provider::Codex => self.codex,
        }
    }

    pub fn any(self) -> bool {
        self.claude || self.codex
    }

    pub fn union(self, other: Wants) -> Wants {
        Wants { claude: self.claude || other.claude, codex: self.codex || other.codex }
    }
}

/// The snapshot's account of both providers.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub claude: ProviderUsage,
    #[serde(default)]
    pub codex: ProviderUsage,
    /// What the daemon reads for, as the attached boards asked — the TUI
    /// pushes its preference when this disagrees (a restarted daemon wants
    /// nothing until told).
    #[serde(default)]
    pub wants: Wants,
    /// Providers with a read in flight.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reading: Vec<Provider>,
}

impl Usage {
    pub fn get(&self, p: Provider) -> &ProviderUsage {
        match p {
            Provider::Claude => &self.claude,
            Provider::Codex => &self.codex,
        }
    }

    pub fn get_mut(&mut self, p: Provider) -> &mut ProviderUsage {
        match p {
            Provider::Claude => &mut self.claude,
            Provider::Codex => &mut self.codex,
        }
    }
}

/// Codex's severity, and an older Claude Code's: a warning from 80%, critical
/// from 95%.
pub fn grade(percent: f64) -> Severity {
    if percent >= 95.0 {
        Severity::Critical
    } else if percent >= 80.0 {
        Severity::Warning
    } else {
        Severity::Normal
    }
}

/// A server label, bounded: it is drawn on one line of a board.
fn bounded(s: &str) -> String {
    s.chars().take(32).collect()
}

/// Claude Code's answer to `get_usage` — the `response` inside a successful
/// `control_response`.
pub fn parse_claude(body: &Value, now_ms: u64) -> Result<Reading, Problem> {
    if body.get("rate_limits_available").and_then(Value::as_bool) == Some(false) {
        return Err(Problem::NoPlan);
    }
    let plan = body.get("subscription_type").and_then(Value::as_str).map(bounded);
    let limits = &body["rate_limits"];
    let windows = match limits.get("limits").and_then(Value::as_array) {
        Some(rows) => rows.iter().filter_map(claude_row).collect(),
        None => claude_fixed(limits),
    };
    Ok(Reading { read_at_ms: now_ms, plan, windows })
}

/// One of the server's `limits[]` rows, carried as sent.
fn claude_row(row: &Value) -> Option<Window> {
    let kind = row.get("kind")?.as_str()?;
    let percent = row.get("percent")?.as_f64()?;
    let scope = row["scope"]["model"]["display_name"]
        .as_str()
        .or_else(|| row["scope"]["surface"]["display_name"].as_str())
        .map(bounded);
    let (kind, label, long, length_mins) = match kind {
        "session" => (WindowKind::Session, "5h".to_string(), "5-hour".to_string(), Some(300)),
        "weekly_all" => {
            (WindowKind::Weekly, "week".into(), "week, all models".into(), Some(10_080))
        }
        "weekly_scoped" => {
            let name = scope.unwrap_or_else(|| "scoped".into());
            let long = format!("week, {name}");
            (WindowKind::Model, name, long, Some(10_080))
        }
        other => {
            let name = scope.unwrap_or_else(|| bounded(&other.replace('_', " ")));
            (WindowKind::Other, name.clone(), name, None)
        }
    };
    let severity = match row.get("severity").and_then(Value::as_str) {
        Some("warning") => Severity::Warning,
        Some("critical") => Severity::Critical,
        // An unknown word is not an alarm: the board says nothing it was
        // not told.
        _ => Severity::Normal,
    };
    Some(Window {
        kind,
        label,
        long,
        percent,
        resets_at_ms: row.get("resets_at").and_then(Value::as_str).and_then(rfc3339_ms),
        severity,
        headline: row.get("is_active").and_then(Value::as_bool).unwrap_or(false),
        length_mins,
    })
}

/// A Claude Code that predates the `limits` rows: the fixed windows, graded
/// here, the fullest one the headline.
fn claude_fixed(limits: &Value) -> Vec<Window> {
    let mut out = Vec::new();
    let fixed: [(&str, WindowKind, &str, &str, u32); 4] = [
        ("five_hour", WindowKind::Session, "5h", "5-hour", 300),
        ("seven_day", WindowKind::Weekly, "week", "week, all models", 10_080),
        ("seven_day_opus", WindowKind::Model, "Opus", "week, Opus", 10_080),
        ("seven_day_sonnet", WindowKind::Model, "Sonnet", "week, Sonnet", 10_080),
    ];
    for (key, kind, label, long, mins) in fixed {
        let w = &limits[key];
        let Some(percent) = w.get("utilization").and_then(Value::as_f64) else { continue };
        out.push(Window {
            kind,
            label: label.into(),
            long: long.into(),
            percent,
            resets_at_ms: w.get("resets_at").and_then(Value::as_str).and_then(rfc3339_ms),
            severity: grade(percent),
            headline: false,
            length_mins: Some(mins),
        });
    }
    for m in limits.get("model_scoped").and_then(Value::as_array).into_iter().flatten() {
        let (Some(name), Some(percent)) = (
            m.get("display_name").and_then(Value::as_str),
            m.get("utilization").and_then(Value::as_f64),
        ) else {
            continue;
        };
        let name = bounded(name);
        out.push(Window {
            kind: WindowKind::Model,
            long: format!("week, {name}"),
            label: name,
            percent,
            resets_at_ms: m.get("resets_at").and_then(Value::as_str).and_then(rfc3339_ms),
            severity: grade(percent),
            headline: false,
            length_mins: Some(10_080),
        });
    }
    mark_fullest(&mut out);
    out
}

fn mark_fullest(windows: &mut [Window]) {
    let best = windows
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.percent.partial_cmp(&b.1.percent).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, _)| i);
    for (i, w) in windows.iter_mut().enumerate() {
        w.headline = Some(i) == best;
    }
}

/// The rate-limit windows a session's mod read off `$.session.usage()` at a
/// turn's end (T-581), merged into what is held: `five_hour` and
/// `seven_day` (each `percentUsed` and an ISO `resetsAt`) refresh the 5-hour
/// and the all-models week, and every window the mod does not see (a
/// model's week, the plan word, the server's headline pick) keeps the
/// probe's reading. The engine sends no severity, so a window keeps the
/// server's grade while it is the same window and is graded here once it
/// starts over, or when it is new. `None` when the answer names no window.
pub fn merge_mod(held: Option<&Reading>, limits: &Value, now_ms: u64) -> Option<Reading> {
    let mut out = held.cloned().unwrap_or_default();
    let fresh = held.is_none_or(|r| r.windows.is_empty());
    let mut moved = false;
    for row in limits.as_array().into_iter().flatten() {
        let (Some(kind), Some(percent)) = (
            row.get("kind").and_then(Value::as_str),
            row.get("percentUsed").and_then(Value::as_f64).filter(|p| p.is_finite()),
        ) else {
            continue;
        };
        let (kind, label, long, length_mins) = match kind {
            "five_hour" => (WindowKind::Session, "5h".to_string(), "5-hour".to_string(), Some(300)),
            "seven_day" => {
                (WindowKind::Weekly, "week".into(), "week, all models".into(), Some(10_080))
            }
            other => {
                let name = bounded(&other.replace('_', " "));
                (WindowKind::Other, name.clone(), name, None)
            }
        };
        let resets_at_ms = row.get("resetsAt").and_then(Value::as_str).and_then(rfc3339_ms);
        let same =
            |w: &&mut Window| w.kind == kind && (kind != WindowKind::Other || w.label == label);
        match out.windows.iter_mut().find(same) {
            Some(w) => {
                let started_over = resets_at_ms.is_some() && resets_at_ms != w.resets_at_ms;
                w.severity =
                    if started_over { grade(percent) } else { w.severity.max(grade(percent)) };
                w.percent = percent;
                w.resets_at_ms = resets_at_ms.or(w.resets_at_ms);
            }
            None => out.windows.push(Window {
                kind,
                label,
                long,
                percent,
                resets_at_ms,
                severity: grade(percent),
                headline: false,
                length_mins,
            }),
        }
        moved = true;
    }
    if !moved {
        return None;
    }
    if fresh {
        mark_fullest(&mut out.windows);
    }
    out.read_at_ms = now_ms;
    Some(out)
}

/// A field under either spelling: the app-server's camelCase or the
/// rollout's snake_case.
fn field<'a>(v: &'a Value, camel: &str, snake: &str) -> Option<&'a Value> {
    v.get(camel).or_else(|| v.get(snake)).filter(|x| !x.is_null())
}

/// Codex's rate limits — an `account/rateLimits/read` result, an
/// `account/rateLimits/updated` notification's params, or a rollout's
/// `token_count.rate_limits`. `None` when the value holds no snapshot.
pub fn parse_codex(v: &Value, now_ms: u64) -> Option<Reading> {
    let mut snaps: Vec<&Value> = Vec::new();
    if let Some(by) =
        field(v, "rateLimitsByLimitId", "rate_limits_by_limit_id").and_then(Value::as_object)
    {
        // The main bucket first, the model buckets after it by id.
        let mut ids: Vec<&String> = by.keys().collect();
        ids.sort_by_key(|id| (id.as_str() != "codex", id.to_string()));
        snaps.extend(ids.into_iter().filter_map(|id| by.get(id)).filter(|s| s.is_object()));
    }
    if snaps.is_empty() {
        let one = field(v, "rateLimits", "rate_limits").unwrap_or(v);
        if one.get("primary").is_some() || one.get("secondary").is_some() {
            snaps.push(one);
        }
    }
    if snaps.is_empty() {
        return None;
    }
    let mut plan = None;
    let mut windows = Vec::new();
    for snap in snaps {
        if plan.is_none() {
            plan = field(snap, "planType", "plan_type").and_then(Value::as_str).map(bounded);
        }
        let id = field(snap, "limitId", "limit_id").and_then(Value::as_str).unwrap_or("codex");
        let name = field(snap, "limitName", "limit_name").and_then(Value::as_str);
        let first = windows.len();
        for slot in ["primary", "secondary"] {
            if let Some(w) = snap.get(slot).filter(|w| w.is_object()) {
                if let Some(window) = codex_window(w, id, name, now_ms) {
                    windows.push(window);
                }
            }
        }
        // The provider said this bucket is at its limit: its fullest window
        // is critical whatever the number reads.
        if field(snap, "rateLimitReachedType", "rate_limit_reached_type").is_some() {
            if let Some(w) = windows[first..].iter_mut().max_by(|a, b| {
                a.percent.partial_cmp(&b.percent).unwrap_or(std::cmp::Ordering::Equal)
            }) {
                w.severity = Severity::Critical;
            }
        }
    }
    mark_fullest(&mut windows);
    Some(Reading { read_at_ms: now_ms, plan, windows })
}

fn codex_window(w: &Value, id: &str, name: Option<&str>, now_ms: u64) -> Option<Window> {
    let percent = field(w, "usedPercent", "used_percent").and_then(Value::as_f64)?;
    let mins = field(w, "windowDurationMins", "window_minutes")
        .and_then(Value::as_u64)
        .and_then(|m| u32::try_from(m).ok());
    let resets_at_ms = field(w, "resetsAt", "resets_at")
        .and_then(Value::as_u64)
        .map(|s| s.saturating_mul(1000))
        .or_else(|| {
            field(w, "resetsInSeconds", "resets_in_seconds")
                .and_then(Value::as_u64)
                .map(|s| now_ms + s.saturating_mul(1000))
        });
    let span = mins.map(span_word);
    let (kind, label, long) = if id == "codex" {
        match mins {
            Some(300) => (WindowKind::Session, "5h".to_string(), "5-hour".to_string()),
            Some(10_080) => (WindowKind::Weekly, "week".into(), "week, all models".into()),
            _ => {
                let s = span.clone().unwrap_or_else(|| "window".into());
                (WindowKind::Other, s.clone(), s)
            }
        }
    } else {
        let full = bounded(name.unwrap_or(id));
        let short = short_model(&full);
        match mins {
            Some(10_080) => (WindowKind::Model, short, format!("week, {full}")),
            Some(300) => (WindowKind::Model, format!("{short} 5h"), format!("5-hour, {full}")),
            _ => {
                let s = span.unwrap_or_else(|| "window".into());
                (WindowKind::Model, format!("{short} {s}"), format!("{s}, {full}"))
            }
        }
    };
    Some(Window {
        kind,
        label,
        long,
        percent,
        resets_at_ms,
        severity: grade(percent),
        headline: false,
        length_mins: mins,
    })
}

/// A window's length in a word: `day`, `week`, `month`, else hours or days.
fn span_word(mins: u32) -> String {
    match mins {
        1440 => "day".into(),
        10_080 => "week".into(),
        43_200 => "month".into(),
        m if m % 1440 == 0 => format!("{}d", m / 1440),
        m if m % 60 == 0 => format!("{}h", m / 60),
        m => format!("{m}m"),
    }
}

/// The part of a model bucket's name the line has room for:
/// `GPT-5.3-Codex-Spark` is `Spark` beside the word `codex`.
fn short_model(name: &str) -> String {
    let parts: Vec<&str> = name.split('-').filter(|p| !p.is_empty()).collect();
    match parts.last() {
        Some(last) if parts.len() >= 3 && !last.chars().all(|c| c.is_ascii_digit() || c == '.') => {
            (*last).to_string()
        }
        _ => name.to_string(),
    }
}

/// Where a window is heading at its rate so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pace {
    /// The percentage the window ends at if the rate holds.
    pub projected: u32,
    /// When it fills, if it does before it resets — unix ms.
    pub full_at_ms: Option<u64>,
}

/// The dialog's experimental row: a straight line through the window's
/// start and now. `None` before a tenth of the window has gone (a line
/// through two points that close says nothing), after it reset, at or past
/// 100%, or where the window's length is unknown.
pub fn pace(w: &Window, now_ms: u64) -> Option<Pace> {
    let len_ms = u64::from(w.length_mins?) * 60_000;
    let reset = w.resets_at_ms?;
    if reset <= now_ms || w.percent >= 100.0 || len_ms == 0 {
        return None;
    }
    let start = reset.saturating_sub(len_ms);
    let gone = now_ms.saturating_sub(start);
    if gone.saturating_mul(10) < len_ms {
        return None;
    }
    let projected = w.percent * len_ms as f64 / gone as f64;
    let full_at_ms = (projected >= 100.0 && w.percent > 0.0)
        .then(|| start + (gone as f64 * 100.0 / w.percent) as u64)
        .filter(|at| *at < reset);
    Some(Pace { projected: projected.round().min(9_999.0) as u32, full_at_ms })
}

/// A second read at most this often, whatever asked.
pub const MIN_GAP_MS: u64 = 60_000;
/// A person's `r` at most this often.
pub const ASK_GAP_MS: u64 = 10_000;
/// A turn ended: read again once the reading is this old.
pub const AFTER_TURN_MS: u64 = 3 * 60_000;
/// Nothing happened: read again once the reading is this old.
pub const IDLE_MS: u64 = 15 * 60_000;
/// The first retry after a failure, doubling to [`BACKOFF_MAX_MS`].
pub const BACKOFF_FIRST_MS: u64 = 5 * 60_000;
pub const BACKOFF_MAX_MS: u64 = 60 * 60_000;

/// One provider's read schedule, per daemon. The events since the last
/// attempt are flags; [`Cadence::due`] reads them against the reading's age.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cadence {
    /// Unix ms of this daemon's last attempt (0: none yet).
    pub tried_at_ms: u64,
    /// Consecutive failed attempts.
    pub failures: u32,
    /// A lasting problem answered last (signed out, no plan): retry hourly.
    pub lasting: bool,
    /// A turn ended since the last attempt.
    pub turn_ended: bool,
    /// A session stopped on a rate limit since the last attempt.
    pub limited: bool,
    /// A person asked (the dialog's `r`).
    pub asked: bool,
}

impl Cadence {
    /// Should this daemon read now? `read_at_ms` is the reading's age as the
    /// shared file has it — another board's daemon may have read a minute ago,
    /// and that counts.
    pub fn due(&self, now_ms: u64, read_at_ms: Option<u64>, next_reset_ms: Option<u64>) -> bool {
        let since_try = now_ms.saturating_sub(self.tried_at_ms);
        if self.asked {
            return self.tried_at_ms == 0 || since_try >= ASK_GAP_MS;
        }
        let last = self.tried_at_ms.max(read_at_ms.unwrap_or(0));
        let since = now_ms.saturating_sub(last);
        if self.failures > 0 {
            let wait = if self.lasting {
                BACKOFF_MAX_MS
            } else {
                (BACKOFF_FIRST_MS << (self.failures - 1).min(4)).min(BACKOFF_MAX_MS)
            };
            return since_try >= wait;
        }
        if last == 0 {
            return true;
        }
        if since < MIN_GAP_MS {
            return false;
        }
        if self.limited {
            return true;
        }
        let age = read_at_ms.map_or(u64::MAX, |r| now_ms.saturating_sub(r));
        if self.turn_ended && age >= AFTER_TURN_MS {
            return true;
        }
        // A window rolled over after the reading was taken: its number is
        // about a window that no longer exists.
        if let (Some(reset), Some(read)) = (next_reset_ms, read_at_ms) {
            if read < reset && reset <= now_ms {
                return true;
            }
        }
        age >= IDLE_MS
    }

    /// An attempt is starting: the events it answers are spent.
    pub fn start(&mut self, now_ms: u64) {
        self.tried_at_ms = now_ms;
        self.turn_ended = false;
        self.limited = false;
        self.asked = false;
    }

    pub fn succeeded(&mut self) {
        self.failures = 0;
        self.lasting = false;
    }

    pub fn failed(&mut self, problem: &Problem) {
        self.failures = self.failures.saturating_add(1);
        self.lasting = problem.is_lasting();
    }
}

/// Unix ms of an RFC 3339 stamp: `2026-10-01T17:50:00.286999+00:00`, `…Z`.
pub fn rfc3339_ms(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> { s.get(r)?.parse().ok() };
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    let mut rest = &s[19..];
    let mut ms = 0i64;
    if let Some(frac) = rest.strip_prefix('.') {
        let digits: String = frac.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() {
            return None;
        }
        let first3: String = digits.chars().chain("000".chars()).take(3).collect();
        ms = first3.parse().ok()?;
        rest = &frac[digits.len()..];
    }
    let offset_secs = match rest {
        "Z" | "z" | "" => 0,
        tz => {
            let sign = match tz.as_bytes().first()? {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let oh: i64 = tz.get(1..3)?.parse().ok()?;
            let om: i64 = tz.get(4..6).or_else(|| tz.get(3..5))?.parse().ok()?;
            sign * (oh * 3600 + om * 60)
        }
    };
    let days = days_from_civil(y, mo as u32, d as u32);
    let secs = days * 86_400 + h * 3600 + mi * 60 + sec - offset_secs;
    u64::try_from(secs * 1000 + ms).ok()
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Hinnant's algorithm).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The UTC wall clock of unix seconds, in `struct tm`'s conventions — the
/// clock a board without a time zone reads (tests and goldens).
pub fn utc_of(secs: u64) -> crate::snooze::LocalTime {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    crate::snooze::LocalTime {
        year: (y - 1900) as i32,
        mon: (m - 1) as i32,
        mday: d as i32,
        hour: (rem / 3600) as i32,
        min: ((rem % 3600) / 60) as i32,
        sec: (rem % 60) as i32,
        // 1970-01-01 was a Thursday.
        wday: ((days % 7 + 11) % 7) as i32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: u64 = 1_790_870_640_000; // 2026-10-01T16:04:00Z

    #[test]
    fn a_mods_windows_refresh_the_probes_and_keep_what_it_cannot_see() {
        let held = parse_claude(&claude_answer(), NOW - 600_000).unwrap();
        let week = held.windows.iter().find(|w| w.kind == WindowKind::Weekly).cloned();
        let limits = json!([
            {"kind": "five_hour", "percentUsed": 9.5, "resetsAt": "2026-10-01T20:00:00Z"},
            {"kind": "seven_day", "percentUsed": 81, "resetsAt": "2099-01-01T00:00:00Z"},
        ]);
        let got = merge_mod(Some(&held), &limits, NOW).unwrap();
        assert_eq!(got.read_at_ms, NOW);
        assert_eq!(got.plan, held.plan, "the plan word is the probe's");
        let five = got.windows.iter().find(|w| w.kind == WindowKind::Session).unwrap();
        assert_eq!((five.percent, five.label.as_str()), (9.5, "5h"));
        // Every window the mod does not name stands as the probe read it.
        for w in held
            .windows
            .iter()
            .filter(|w| !matches!(w.kind, WindowKind::Session | WindowKind::Weekly))
        {
            assert!(got.windows.contains(w), "{w:?} kept");
        }
        let w = got.windows.iter().find(|w| w.kind == WindowKind::Weekly).unwrap();
        assert_eq!(w.percent, 81.0);
        if let Some(old) = week {
            // A week that started over is graded here; the same one keeps the
            // server's grade unless it climbed past it.
            assert_eq!(
                w.severity,
                if old.resets_at_ms == w.resets_at_ms {
                    old.severity.max(Severity::Warning)
                } else {
                    Severity::Warning
                }
            );
        }
        // Nothing held: the mod's windows alone, graded, the fullest the headline.
        let alone = merge_mod(None, &limits, NOW).unwrap();
        assert_eq!(alone.windows.len(), 2);
        assert!(alone.windows.iter().any(|w| w.headline && w.percent == 81.0));
        // Off a subscription the list is empty: nothing to merge.
        assert!(merge_mod(Some(&held), &json!([]), NOW).is_none());
        assert!(merge_mod(None, &json!([{"kind": "five_hour"}]), NOW).is_none());
    }

    /// The author's `get_usage` answer on 2026-10-01, trimmed to what this
    /// module reads.
    fn claude_answer() -> Value {
        json!({
            "subscription_type": "max",
            "rate_limits_available": true,
            "rate_limits": {
                "five_hour": {"utilization": 14, "resets_at": "2026-10-01T17:50:00.286999+00:00"},
                "seven_day": {"utilization": 63, "resets_at": "2026-10-03T08:00:00.287021+00:00"},
                "seven_day_opus": null,
                "limits": [
                    {"kind": "session", "group": "session", "percent": 14,
                     "resets_at": "2026-10-01T17:50:00.286999+00:00", "severity": "normal",
                     "is_active": false, "scope": null},
                    {"kind": "weekly_all", "group": "weekly", "percent": 63,
                     "resets_at": "2026-10-03T08:00:00.287021+00:00", "severity": "normal",
                     "is_active": false, "scope": null},
                    {"kind": "weekly_scoped", "group": "weekly", "percent": 64,
                     "resets_at": "2026-10-03T08:00:00.287197+00:00", "severity": "normal",
                     "is_active": true, "scope": {"model": {"display_name": "Fable", "id": null}, "surface": null}}
                ],
                "model_scoped": [{"display_name": "Fable", "utilization": 64,
                                  "resets_at": "2026-10-03T08:00:00.287197+00:00"}]
            }
        })
    }

    #[test]
    fn claude_rows_are_carried_as_the_server_sent_them() {
        let r = parse_claude(&claude_answer(), NOW).unwrap();
        assert_eq!(r.plan.as_deref(), Some("max"));
        let labels: Vec<_> =
            r.windows.iter().map(|w| (w.label.as_str(), w.percent_word())).collect();
        assert_eq!(labels, [("5h", "14%".into()), ("week", "63%".into()), ("Fable", "64%".into())]);
        assert_eq!(r.windows[2].kind, WindowKind::Model);
        assert_eq!(r.windows[2].long, "week, Fable");
        assert_eq!(r.headline().map(|w| w.label.as_str()), Some("Fable"), "the server's pick");
        assert_eq!(r.windows[0].resets_at_ms, Some(1_790_877_000_286));
        assert_eq!(r.next_reset_ms(), Some(1_790_877_000_286));
        assert_eq!(r.worst(), Severity::Normal);
    }

    #[test]
    fn claude_severity_is_the_servers_and_outranks_its_headline() {
        let mut v = claude_answer();
        v["rate_limits"]["limits"][0]["percent"] = json!(86);
        v["rate_limits"]["limits"][0]["severity"] = json!("warning");
        let r = parse_claude(&v, NOW).unwrap();
        assert_eq!(r.windows[0].severity, Severity::Warning);
        assert_eq!(
            r.headline().map(|w| w.label.as_str()),
            Some("5h"),
            "a warning outranks the pick"
        );
        // A word this build does not know is no alarm.
        v["rate_limits"]["limits"][0]["severity"] = json!("puce");
        assert_eq!(parse_claude(&v, NOW).unwrap().windows[0].severity, Severity::Normal);
    }

    #[test]
    fn an_api_key_has_no_plan_and_an_older_cli_is_graded_here() {
        let v = json!({"rate_limits_available": false, "rate_limits": null});
        assert_eq!(parse_claude(&v, NOW), Err(Problem::NoPlan));
        let mut old = claude_answer();
        old["rate_limits"].as_object_mut().unwrap().remove("limits");
        old["rate_limits"]["five_hour"]["utilization"] = json!(91);
        let r = parse_claude(&old, NOW).unwrap();
        let got: Vec<_> = r.windows.iter().map(|w| (w.label.as_str(), w.severity)).collect();
        assert_eq!(
            got,
            [("5h", Severity::Warning), ("week", Severity::Normal), ("Fable", Severity::Normal)]
        );
        assert_eq!(r.headline().map(|w| w.label.as_str()), Some("5h"));
    }

    #[test]
    fn codex_reads_the_rollout_and_the_app_server_alike() {
        // A rollout `token_count` event's `rate_limits`, snake_case.
        let rollout = json!({
            "limit_id": "codex", "limit_name": null,
            "primary": {"used_percent": 1.0, "window_minutes": 10080, "resets_at": 1_790_801_958},
            "secondary": null, "plan_type": "prolite", "rate_limit_reached_type": null
        });
        let r = parse_codex(&rollout, NOW).unwrap();
        assert_eq!(r.plan.as_deref(), Some("prolite"));
        assert_eq!(r.windows.len(), 1);
        assert_eq!((r.windows[0].kind, r.windows[0].label.as_str()), (WindowKind::Weekly, "week"));
        assert_eq!(r.windows[0].resets_at_ms, Some(1_790_801_958_000));
        // The app-server's answer, camelCase, a model bucket beside the main one.
        let read = json!({
            "rateLimits": {"limitId": "codex"},
            "rateLimitsByLimitId": {
                "codex_bengalfox": {"limitId": "codex_bengalfox", "limitName": "GPT-5.3-Codex-Spark",
                    "primary": {"usedPercent": 4, "windowDurationMins": 10080, "resetsAt": 1_791_000_000}},
                "codex": {"limitId": "codex", "planType": "pro",
                    "primary": {"usedPercent": 38, "windowDurationMins": 300, "resetsAt": 1_790_880_000},
                    "secondary": {"usedPercent": 84, "windowDurationMins": 10080, "resetsAt": 1_791_000_000}}
            }
        });
        let r = parse_codex(&read, NOW).unwrap();
        let got: Vec<_> = r.windows.iter().map(|w| (w.label.as_str(), w.severity)).collect();
        assert_eq!(
            got,
            [("5h", Severity::Normal), ("week", Severity::Warning), ("Spark", Severity::Normal)]
        );
        assert_eq!(r.windows[2].long, "week, GPT-5.3-Codex-Spark");
        assert_eq!(r.headline().map(|w| w.label.as_str()), Some("week"), "graded here: 84% warns");
        // A notification's params carry one snapshot; a reached limit is critical.
        let note = json!({"rateLimits": {"limitId": "codex", "rateLimitReachedType": "primary",
            "primary": {"usedPercent": 71, "windowDurationMins": 300, "resetsAt": 1_790_880_000}}});
        let r = parse_codex(&note, NOW).unwrap();
        assert_eq!(r.windows[0].severity, Severity::Critical);
        assert!(parse_codex(&json!({"credits": {}}), NOW).is_none());
    }

    #[test]
    fn grades_and_spans() {
        assert_eq!(grade(79.9), Severity::Normal);
        assert_eq!(grade(80.0), Severity::Warning);
        assert_eq!(grade(95.0), Severity::Critical);
        assert_eq!(span_word(43_200), "month");
        assert_eq!(span_word(120), "2h");
        assert_eq!(short_model("GPT-5.3-Codex-Spark"), "Spark");
        assert_eq!(short_model("premium"), "premium");
        assert_eq!(short_model("gpt-5-2"), "gpt-5-2", "a version is not a name");
    }

    #[test]
    fn pace_is_a_straight_line_and_says_nothing_early() {
        let week = Window {
            percent: 63.0,
            resets_at_ms: Some(NOW + 40 * 3_600_000),
            length_mins: Some(10_080),
            ..Default::default()
        };
        // 128 of 168 hours gone, 63% used: on track for 83%.
        let p = pace(&week, NOW).unwrap();
        assert_eq!(p.projected, 83);
        assert_eq!(p.full_at_ms, None);
        let hot = Window { percent: 90.0, ..week.clone() };
        let p = pace(&hot, NOW).unwrap();
        assert!(
            p.projected > 100 && p.full_at_ms.is_some_and(|t| t > NOW && t < NOW + 40 * 3_600_000)
        );
        let early = Window { resets_at_ms: Some(NOW + 160 * 3_600_000), ..week.clone() };
        assert_eq!(pace(&early, NOW), None, "under a tenth of the week gone");
        assert_eq!(pace(&Window { length_mins: None, ..week }, NOW), None);
    }

    #[test]
    fn cadence_reads_on_events_and_backs_off() {
        let mut c = Cadence::default();
        assert!(c.due(NOW, None, None), "never read: read");
        c.start(NOW);
        c.succeeded();
        let read = Some(NOW);
        assert!(!c.due(NOW + 30_000, read, None), "the floor");
        assert!(!c.due(NOW + 2 * 60_000, read, None), "nothing happened");
        c.turn_ended = true;
        assert!(!c.due(NOW + 2 * 60_000, read, None), "too fresh for a turn");
        assert!(c.due(NOW + AFTER_TURN_MS, read, None));
        c.turn_ended = false;
        c.limited = true;
        assert!(c.due(NOW + MIN_GAP_MS, read, None), "a rate-limit stop reads at once");
        c.limited = false;
        assert!(c.due(NOW + 2 * 60_000, read, Some(NOW + 60_000)), "a window rolled over");
        assert!(c.due(NOW + IDLE_MS, read, None));
        // Another daemon's fresh reading counts: no read for this one.
        let fresh = Cadence::default();
        assert!(!fresh.due(NOW + 10_000, Some(NOW), None));
        // A person's r: at most every ten seconds.
        let mut asked = Cadence { asked: true, ..c.clone() };
        asked.tried_at_ms = NOW;
        assert!(!asked.due(NOW + 5_000, read, None));
        assert!(asked.due(NOW + ASK_GAP_MS, read, None));
        // Failures back off; a lasting one waits the hour.
        let mut f = Cadence::default();
        f.start(NOW);
        f.failed(&Problem::Failed("timeout".into()));
        assert!(!f.due(NOW + BACKOFF_FIRST_MS - 1, None, None));
        assert!(f.due(NOW + BACKOFF_FIRST_MS, None, None));
        f.failed(&Problem::SignedOut);
        assert!(!f.due(NOW + BACKOFF_MAX_MS - 1, None, None));
        assert!(f.due(NOW + BACKOFF_MAX_MS, None, None));
    }

    #[test]
    fn a_lasting_problem_drops_the_reading_and_a_passing_one_keeps_it() {
        let mut p = ProviderUsage::default();
        p.read(Reading { read_at_ms: NOW, ..Default::default() });
        p.failed(Problem::Failed("claude did not answer".into()), NOW + 1);
        assert!(p.reading.is_some());
        p.failed(Problem::SignedOut, NOW + 2);
        assert!(p.reading.is_none());
        assert_eq!(p.clone().newer(ProviderUsage::default()), p, "the later attempt wins");
    }

    #[test]
    fn dates_both_ways() {
        assert_eq!(rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(rfc3339_ms("2026-10-03T08:00:00.287021+00:00"), Some(1_791_014_400_287));
        assert_eq!(rfc3339_ms("2026-10-03T11:00:00+03:00"), Some(1_791_014_400_000));
        assert_eq!(rfc3339_ms("2026-02-30"), None);
        assert_eq!(rfc3339_ms("not a date at all"), None);
        let t = utc_of(1_791_014_400);
        assert_eq!(
            (t.year + 1900, t.mon + 1, t.mday, t.hour, t.min, t.wday),
            (2026, 10, 3, 8, 0, 6)
        );
        let leap = utc_of(rfc3339_ms("2024-02-29T23:59:59Z").unwrap() / 1000);
        assert_eq!((leap.mon + 1, leap.mday, leap.wday), (2, 29, 4));
    }

    #[test]
    fn the_snapshot_shape_round_trips_and_an_old_daemon_reads_as_nothing() {
        let mut u = Usage::default();
        u.claude.read(parse_claude(&claude_answer(), NOW).unwrap());
        u.codex.failed(Problem::SignedOut, NOW);
        u.wants = Wants { claude: true, codex: true };
        let back: Usage = serde_json::from_str(&serde_json::to_string(&u).unwrap()).unwrap();
        assert_eq!(back, u);
        let empty: Usage = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, Usage::default());
    }
}
