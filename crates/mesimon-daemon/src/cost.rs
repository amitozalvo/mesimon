//! `costs.json` (T-327): the tokens each ticket's agents have spent, read off
//! their transcripts as they grow.
//!
//! A ticket's account outlives its sessions: a record forgets a conversation
//! at `/clear` and goes when the ticket's agent is replaced, so the ledger
//! keeps every transcript a ticket's sessions ever pointed at (`learn`), with
//! how far it has been read, and the tokens by hour and model. The hour is
//! what lets the Usage dialog say the last 24 hours, 7 days and 30 days
//! without a time zone. Claude's subagent transcripts
//! (`<session>/subagents/agent-*.jsonl`) are found beside the session's own
//! and counted to the same ticket. What a ticket held before this file
//! existed is counted from the transcripts its live records still name.
//!
//! A Claude session whose mod reports its turns (T-581) is counted from
//! those reports instead: the engine's own sum of each turn's requests,
//! `turn.complete`'s `usage`. Its conversation is fenced at the first main
//! turn the mod reports, and from that moment the transcript tail still reads
//! it, into the ticket's `check` and never its hours, so the two counts stand
//! side by side (`Ledger::disagreements`, doctor's `costs` line) and neither
//! is added twice. The tail stays the count for the hook set's sessions,
//! Codex, and whatever a conversation held before its fence.
//!
//! The reading runs on a worker (`scan`): a transcript is megabytes, and the
//! writer only folds the counts back in. The file follows the other state
//! files' contract: its own `schema_version`, a newer build's bytes left
//! untouched with writes barred, an unparseable file quarantined.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufRead, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::Result;
use mesimon_core::command::Notice;
use mesimon_core::cost::{claude_turn, codex_line, delta, CodexLine, TicketCost, Tokens, FAST};
use serde::{Deserialize, Serialize};

use crate::paths::Paths;

pub const COSTS_SCHEMA: u32 = 1;

/// The most a pass reads of one transcript; the rest waits for the next.
const READ_CAP: u64 = 32 << 20;
/// Message ids remembered per conversation: a message's lines are adjacent.
const RECENT: usize = 64;

/// How far one transcript has been read.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Cursor {
    /// A Codex rollout, rather than a Claude transcript.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub codex: bool,
    #[serde(default)]
    pub offset: u64,
    /// The last Claude message ids counted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent: Vec<String>,
    /// Codex: the running totals last read, and the model the turns run on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub totals: Option<Tokens>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Unix ms from which the session's mod counts this conversation's turns
    /// (T-581): a transcript line stamped at or after it is the check's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by_mod_since: Option<u64>,
    /// `pr create` tool calls whose result the pass has not reached yet
    /// (T-739): the `opened here` fact is read off the result, which is
    /// the next line or the next pass.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_create: Vec<String>,
    /// `gh pr` / `glab mr` acts whose result the pass has not reached yet
    /// (T-859), as `<id> <entry>`: the `acted on here` fact is read off the
    /// result, which says whether the act went through.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_act: Vec<String>,
    /// Every `pr create` the transcript holds is in its ticket's `pr_urls`
    /// (T-810): read from its start by a build that looks for them, by the
    /// pass that learned it or by the backfill. A transcript the cursor had
    /// passed before T-739 looked is not, and the switch to pull requests
    /// reads it once (`backfill`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub backfilled: bool,
}

/// The two counts of a ticket's mod-reported turns (T-581), in tokens: the
/// mod's, which are the ticket's, and the transcript tail's of the same
/// conversations past their fence, which are only compared.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    #[serde(default, rename = "mod", skip_serializing_if = "is_zero")]
    pub by_mod: u64,
    #[serde(default, rename = "tail", skip_serializing_if = "is_zero")]
    pub by_tail: u64,
}

impl Check {
    fn is_empty(&self) -> bool {
        *self == Check::default()
    }
}

/// What the tail last read of a ticket's Claude cache writes and speed: the
/// mod's count says neither, and both are a session's settings, not a turn's.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hint {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub writes_1h: Option<bool>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fast: bool,
}

impl Hint {
    fn is_empty(&self) -> bool {
        *self == Hint::default()
    }
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TicketLedger {
    /// Every transcript the ticket's agents held, by path.
    #[serde(default)]
    pub conversations: BTreeMap<String, Cursor>,
    /// Tokens by unix hour, then model key.
    #[serde(default)]
    pub hours: BTreeMap<u64, BTreeMap<String, Tokens>>,
    #[serde(default, skip_serializing_if = "Check::is_empty")]
    pub check: Check,
    #[serde(default, skip_serializing_if = "Hint::is_empty")]
    pub hint: Hint,
    /// The PR URLs a `pr create` run by one of the ticket's sessions
    /// printed (T-739): the `opened here` fact, kept here because the
    /// transcript is read once and the cursor never goes back.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pr_urls: Vec<String>,
    /// The PRs a `gh pr` / `glab mr` act run by one of the ticket's
    /// sessions was about (T-859), as `forge::pr_act` spells them: a URL,
    /// or a number on the repository the command named or on the ticket's
    /// own (`#42`, `o/r!7`). The `acted on here` fact, kept as `pr_urls`
    /// is.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pr_acted: Vec<String>,
}

/// How many opened URLs a ticket keeps: the newest. The acted entries
/// share the cap.
const PR_URLS_CAP: usize = 32;

/// `entry` joins `list` once, the oldest dropped past the cap: the fold of
/// both the opened URLs and the acted entries (T-859).
fn keep_named(list: &mut Vec<String>, entry: String) -> bool {
    if list.iter().any(|u| mesimon_core::forge::same_url(u, &entry)) {
        return false;
    }
    if list.len() >= PR_URLS_CAP {
        list.remove(0);
    }
    list.push(entry);
    true
}

impl TicketLedger {
    /// Where the mod's count of `path` starts, if it does: its own fence, or
    /// for a subagent's transcript its session's (`<s>/subagents/…` is
    /// `<s>.jsonl`'s).
    fn fence(&self, path: &str) -> Option<u64> {
        let own = self.conversations.get(path).and_then(|c| c.by_mod_since);
        own.or_else(|| {
            let (stem, _) = path.split_once("/subagents/")?;
            self.conversations.get(&format!("{stem}.jsonl"))?.by_mod_since
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Ledger {
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub tickets: BTreeMap<ulid::Ulid, TicketLedger>,
}

/// One transcript to read on the worker.
#[derive(Debug, Clone)]
pub struct Job {
    pub ticket: ulid::Ulid,
    pub path: String,
    pub cursor: Cursor,
}

/// What a read found: the cursor after it and the tokens it added, each
/// with its unix ms and model key.
#[derive(Debug, Clone, PartialEq)]
pub struct Done {
    pub ticket: ulid::Ulid,
    pub path: String,
    pub cursor: Cursor,
    pub added: Vec<(u64, String, Tokens)>,
    /// PR URLs the pass found in a `pr create`'s result (T-739).
    pub pr_urls: Vec<String>,
    /// The PRs the pass found an act was about (T-859).
    pub pr_acted: Vec<String>,
}

impl Ledger {
    /// A transcript a ticket's session points at. True when it is new. A
    /// session that no longer reports through a mod (`by_mod` false: a wake
    /// on an older Claude Code) hands its conversation back to the tail.
    pub fn learn(&mut self, ticket: ulid::Ulid, path: &str, codex: bool, by_mod: bool) -> bool {
        let t = self.tickets.entry(ticket).or_default();
        if let Some(c) = t.conversations.get_mut(path) {
            if !by_mod {
                c.by_mod_since = None;
            }
            return false;
        }
        // Read from its start, so every `pr create` in it is seen (T-810).
        let cursor = Cursor { codex, backfilled: true, ..Cursor::default() };
        t.conversations.insert(path.to_string(), cursor);
        true
    }

    /// One turn a session's mod counted (T-581), `path` its conversation and
    /// `subagent` whether a subagent's loop ran it. The first main turn
    /// fences the conversation and is not added: it ended before the fence,
    /// so its lines are the tail's; a subagent's turn before that is the
    /// tail's too. After it every turn is the ticket's, with the cache writes
    /// and the speed the tail last read. True when the ledger moved.
    pub fn count_mod(
        &mut self,
        ticket: ulid::Ulid,
        path: &str,
        subagent: bool,
        at_ms: u64,
        model: &str,
        tokens: Tokens,
    ) -> bool {
        let t = self.tickets.entry(ticket).or_default();
        if t.fence(path).is_none() {
            if subagent {
                return false;
            }
            let c = t.conversations.entry(path.to_string()).or_default();
            c.by_mod_since = Some(at_ms);
            return true;
        }
        let mut tokens = tokens;
        if t.hint.writes_1h == Some(true) {
            tokens.write_1h += std::mem::take(&mut tokens.write_5m);
        }
        let model = if t.hint.fast && !model.ends_with(FAST) {
            format!("{model}{FAST}")
        } else {
            model.to_string()
        };
        if tokens.is_empty() {
            return false;
        }
        t.check.by_mod += tokens.total();
        t.hours.entry(at_ms / 3_600_000).or_default().entry(model).or_default().add(&tokens);
        true
    }

    /// The tickets whose two counts differ by more than 2% (and 1,000
    /// tokens): the ticket, the mod's count and the transcript's. A turn in
    /// flight reads ahead on the transcript until its end is reported.
    pub fn disagreements(&self) -> Vec<(ulid::Ulid, u64, u64)> {
        self.tickets
            .iter()
            .filter(|(_, t)| {
                let (a, b) = (t.check.by_mod, t.check.by_tail);
                a.abs_diff(b) > 1_000 && a.abs_diff(b) * 50 > a.max(b)
            })
            .map(|(id, t)| (*id, t.check.by_mod, t.check.by_tail))
            .collect()
    }

    /// How many tickets the mod has counted.
    pub fn by_mod(&self) -> usize {
        self.tickets.values().filter(|t| t.check.by_mod > 0).count()
    }

    /// The transcripts of `tickets` never read whole for `pr create` (T-810,
    /// `Cursor::backfilled`), for the backfill's worker.
    pub fn backfill_jobs(&self, tickets: &HashSet<ulid::Ulid>) -> Vec<(ulid::Ulid, String)> {
        self.tickets
            .iter()
            .filter(|(id, _)| tickets.contains(id))
            .flat_map(|(id, t)| {
                t.conversations.iter().filter(|(_, c)| !c.backfilled).map(|(p, _)| (*id, p.clone()))
            })
            .collect()
    }

    /// The backfill's answers folded in (T-810): each transcript read is
    /// marked, and each URL its ticket lacked is kept as the oldest of its
    /// `pr_urls` while there is room under the cap, never pushing a newer one
    /// out. No cursor moves and no count changes. The URLs it added, per
    /// ticket.
    pub fn apply_backfill(&mut self, found: Vec<Backfilled>) -> Vec<(ulid::Ulid, String)> {
        let mut added = Vec::new();
        for b in found {
            let Some(t) = self.tickets.get_mut(&b.ticket) else { continue };
            if let Some(c) = t.conversations.get_mut(&b.path) {
                c.backfilled = true;
            }
            for url in b.urls {
                if t.pr_urls.len() >= PR_URLS_CAP
                    || t.pr_urls.iter().any(|u| mesimon_core::forge::same_url(u, &url))
                {
                    continue;
                }
                t.pr_urls.insert(0, url.clone());
                added.push((b.ticket, url));
            }
        }
        added
    }

    /// Every known transcript, for the worker to read what grew.
    pub fn jobs(&self) -> Vec<Job> {
        self.tickets
            .iter()
            .flat_map(|(ticket, t)| {
                t.conversations.iter().map(|(path, cursor)| Job {
                    ticket: *ticket,
                    path: path.clone(),
                    cursor: cursor.clone(),
                })
            })
            .collect()
    }

    /// Fold a pass back in. True when anything moved. A line past its
    /// conversation's fence goes to the check, never the hours; the fence is
    /// the ledger's, which a mod's report may have set while the pass ran.
    pub fn apply(&mut self, done: Vec<Done>) -> bool {
        let mut moved = false;
        for d in done {
            let t = self.tickets.entry(d.ticket).or_default();
            let mut cursor = d.cursor;
            cursor.by_mod_since = t.conversations.get(&d.path).and_then(|c| c.by_mod_since);
            // A backfill that landed while this pass was out stands (T-810).
            cursor.backfilled |= t.conversations.get(&d.path).is_some_and(|c| c.backfilled);
            let fence = t.fence(&d.path);
            if t.conversations.get(&d.path) != Some(&cursor) {
                t.conversations.insert(d.path, cursor.clone());
                moved = true;
            }
            for url in d.pr_urls {
                moved |= keep_named(&mut t.pr_urls, url);
            }
            for entry in d.pr_acted {
                moved |= keep_named(&mut t.pr_acted, entry);
            }
            for (at_ms, model, tokens) in d.added {
                if tokens.is_empty() {
                    continue;
                }
                if !cursor.codex {
                    if tokens.write_1h > 0 {
                        t.hint.writes_1h = Some(true);
                    } else if tokens.write_5m > 0 {
                        t.hint.writes_1h = Some(false);
                    }
                    t.hint.fast = model.ends_with(FAST);
                }
                if fence.is_some_and(|since| at_ms >= since) {
                    t.check.by_tail += tokens.total();
                } else {
                    t.hours
                        .entry(at_ms / 3_600_000)
                        .or_default()
                        .entry(model)
                        .or_default()
                        .add(&tokens);
                }
                moved = true;
            }
        }
        moved
    }

    /// The PR URLs a ticket's sessions opened (T-739).
    pub fn opened_urls(&self, ticket: ulid::Ulid) -> Vec<String> {
        self.tickets.get(&ticket).map(|t| t.pr_urls.clone()).unwrap_or_default()
    }

    /// The PRs a ticket's sessions acted on (T-859), as `forge::pr_act`
    /// spells them; `forge::acted_url` makes each an address.
    pub fn acted_entries(&self, ticket: ulid::Ulid) -> Vec<String> {
        self.tickets.get(&ticket).map(|t| t.pr_acted.clone()).unwrap_or_default()
    }

    /// Whether a ticket's sessions opened or acted on a PR a forge read
    /// still owes (T-750, T-797, T-859): a URL or an acted entry the ledger
    /// keeps, or one in a transcript's tail the pass has not reached, that
    /// `owed` says no read has held yet, `live` naming the transcripts its
    /// sessions point at now (a new one the ledger has not learned is read
    /// whole). `None` when a tail could not be read: the gate reads that as
    /// unknown, never as none opened.
    pub fn opened_unread(
        &self,
        ticket: ulid::Ulid,
        live: &[String],
        owed: impl Fn(&str) -> bool,
    ) -> Option<bool> {
        if self.opened_owed(ticket, &owed) {
            return Some(true);
        }
        urls_in_tails(&self.tails(ticket, live)).map(|urls| urls.iter().any(|u| owed(u)))
    }

    /// `opened_unread`'s ledger half alone (T-838): a URL the pass has
    /// kept that `owed` says no read has held yet, with no transcript read.
    /// The snapshot road's, where a tail its pass has not reached is a held
    /// line one pass late and not a file read per snapshot.
    pub fn opened_owed(&self, ticket: ulid::Ulid, owed: impl Fn(&str) -> bool) -> bool {
        self.tickets
            .get(&ticket)
            .is_some_and(|t| t.pr_urls.iter().chain(&t.pr_acted).any(|u| owed(u)))
    }

    /// A ticket's transcripts with how far the pass read each, `live` as
    /// `opened_unread` takes it: what a worker looks past for a `pr create`
    /// the ledger has not reached yet (T-761).
    pub fn tails(&self, ticket: ulid::Ulid, live: &[String]) -> Vec<(String, Cursor)> {
        let mut out: Vec<(String, Cursor)> = self
            .tickets
            .get(&ticket)
            .map(|t| t.conversations.iter().map(|(p, c)| (p.clone(), c.clone())).collect())
            .unwrap_or_default();
        for path in live {
            if !out.iter().any(|(p, _)| p == path) {
                out.push((path.clone(), Cursor::default()));
            }
        }
        out
    }

    /// Drop the tickets the board no longer has. True when any went.
    pub fn keep(&mut self, live: &HashSet<ulid::Ulid>) -> bool {
        let before = self.tickets.len();
        self.tickets.retain(|id, _| live.contains(id));
        self.tickets.len() != before
    }

    /// Each ticket's account as of `now_ms`, for the snapshot. A model's
    /// price is looked up once per view (T-688): every snapshot folds every
    /// (ticket, hour, model) bucket, and the lookup is a scan.
    pub fn view(&self, now_ms: u64) -> Vec<TicketCost> {
        let mut prices: HashMap<&str, Option<mesimon_core::cost::Price>> = HashMap::new();
        self.tickets
            .iter()
            .filter_map(|(ticket, t)| {
                let mut c = TicketCost { ticket: *ticket, ..TicketCost::default() };
                for (hour, models) in &t.hours {
                    for (model, tokens) in models {
                        let price = prices
                            .entry(model.as_str())
                            .or_insert_with(|| mesimon_core::cost::price(model));
                        c.fold_at(price.as_ref(), tokens, *hour, now_ms);
                    }
                }
                (c.tokens > 0).then_some(c)
            })
            .collect()
    }
}

/// Claude's subagent transcripts for a session transcript: `<session>.jsonl`
/// keeps them in `<session>/subagents/`.
fn subagents(path: &str) -> Vec<String> {
    if path.contains("/subagents/") {
        return Vec::new();
    }
    let Some(stem) = path.strip_suffix(".jsonl") else { return Vec::new() };
    let Ok(dir) = std::fs::read_dir(Path::new(stem).join("subagents")) else {
        return Vec::new();
    };
    let mut out: Vec<String> = dir
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .map(|p| p.display().to_string())
        .collect();
    out.sort();
    out
}

/// Read what grew in every transcript, and the subagent transcripts beside
/// them. Runs on a worker; `now_ms` dates a line that carries no stamp.
pub fn scan(jobs: Vec<Job>, now_ms: u64) -> Vec<Done> {
    let known: HashSet<String> = jobs.iter().map(|j| j.path.clone()).collect();
    let mut all = jobs;
    let found: Vec<Job> = all
        .iter()
        .filter(|j| !j.cursor.codex)
        .flat_map(|j| {
            subagents(&j.path).into_iter().map(|path| Job {
                ticket: j.ticket,
                path,
                cursor: Cursor { backfilled: true, ..Cursor::default() },
            })
        })
        .filter(|j| !known.contains(&j.path))
        .collect();
    all.extend(found);
    all.into_iter().filter_map(|j| read(j, now_ms)).collect()
}

/// One transcript from its cursor to the last whole line, or `None` when it
/// has not grown. A file shorter than the cursor was rewritten: it is read
/// again from the start.
fn read(job: Job, now_ms: u64) -> Option<Done> {
    let len = std::fs::metadata(&job.path).ok()?.len();
    let mut cursor = job.cursor;
    let new = cursor.offset == 0 && cursor.recent.is_empty() && cursor.totals.is_none();
    if len == cursor.offset && !new {
        return None;
    }
    if len < cursor.offset {
        cursor = Cursor { codex: cursor.codex, backfilled: true, ..Cursor::default() };
    }
    let mut file = std::fs::File::open(&job.path).ok()?;
    file.seek(SeekFrom::Start(cursor.offset)).ok()?;
    let mut buf = Vec::new();
    file.take(READ_CAP.min(len - cursor.offset)).read_to_end(&mut buf).ok()?;
    let Some(end) = buf.iter().rposition(|b| *b == b'\n') else {
        // Nothing whole yet; a new transcript is still worth recording.
        return new.then_some(Done {
            ticket: job.ticket,
            path: job.path,
            cursor,
            added: Vec::new(),
            pr_urls: Vec::new(),
            pr_acted: Vec::new(),
        });
    };
    let text = String::from_utf8_lossy(&buf[..end]);
    let mut added = Vec::new();
    let mut pr_urls = Vec::new();
    let mut pr_acted = Vec::new();
    for line in text.lines() {
        let named = named_on(line, &mut cursor.pending_create, &mut cursor.pending_act);
        pr_urls.extend(named.opened);
        pr_acted.extend(named.acted);
        if cursor.codex {
            match codex_line(line) {
                Some(CodexLine::Model(m)) => cursor.model = Some(m),
                Some(CodexLine::Totals { at_ms, tokens }) => {
                    let grew = delta(&cursor.totals.unwrap_or_default(), &tokens);
                    cursor.totals = Some(tokens);
                    let model = cursor.model.clone().unwrap_or_else(|| "codex".into());
                    added.push((stamp(at_ms, now_ms), model, grew));
                }
                None => {}
            }
        } else if let Some(turn) = claude_turn(line) {
            if cursor.recent.contains(&turn.id) {
                continue;
            }
            cursor.recent.push(turn.id);
            if cursor.recent.len() > RECENT {
                cursor.recent.remove(0);
            }
            added.push((stamp(turn.at_ms, now_ms), turn.model, turn.tokens));
        }
    }
    cursor.offset += end as u64 + 1;
    Some(Done { ticket: job.ticket, path: job.path, cursor, added, pr_urls, pr_acted })
}

/// The `opened here` fact (T-739) and the `acted on here` fact (T-859) on
/// one line: a `pr create` the agent ran and the URL its result printed, or
/// a `gh pr` / `glab mr` act and the PR it was about. Parsed only where
/// one can be there, so a line costs one parse, not two.
fn named_on(
    line: &str,
    pending: &mut Vec<String>,
    pending_act: &mut Vec<String>,
) -> mesimon_core::forge::Named {
    if line.contains("gh pr")
        || line.contains("glab mr")
        || line.contains("pr create")
        || line.contains("mr create")
        || pending.iter().any(|id| line.contains(id.as_str()))
        || pending_act.iter().any(|p| line.contains(mesimon_core::forge::act_id(p)))
    {
        return mesimon_core::forge::pr_line(line, pending, pending_act);
    }
    mesimon_core::forge::Named::default()
}

/// `named_on`'s opened half alone, where only the `pr create` is looked for.
fn opened_on(line: &str, pending: &mut Vec<String>) -> Vec<String> {
    named_on(line, pending, &mut Vec::new()).opened
}

/// One transcript the backfill read (T-810): the PR URLs its `pr create`
/// results printed.
#[derive(Debug, Clone, PartialEq)]
pub struct Backfilled {
    pub ticket: ulid::Ulid,
    pub path: String,
    pub urls: Vec<String>,
}

/// The backfill (T-810), on a worker: each transcript from its start, line
/// by line and one at a time, for the `pr create` URLs a cursor that passed
/// it before T-739 never looked for. Nothing is held whole, so a transcript
/// of any size is only a longer read. One that cannot be opened has none
/// to give, and is marked read all the same.
pub fn backfill(jobs: Vec<(ulid::Ulid, String)>) -> Vec<Backfilled> {
    jobs.into_iter()
        .map(|(ticket, path)| {
            let mut urls = Vec::new();
            if let Ok(file) = std::fs::File::open(&path) {
                let mut reader = std::io::BufReader::new(file);
                let mut pending = Vec::new();
                let mut buf = Vec::new();
                loop {
                    buf.clear();
                    match reader.read_until(b'\n', &mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            let line = String::from_utf8_lossy(&buf);
                            urls.extend(opened_on(line.trim_end(), &mut pending));
                        }
                    }
                }
            }
            Backfilled { ticket, path, urls }
        })
        .collect()
}

/// Whether a tail past the ledger's cursor holds a `pr create`'s result,
/// its subagents' transcripts included (read whole where the ledger has
/// none). `None` when a tail could not be read: unknown, never none.
pub fn opened_in_tails(tails: &[(String, Cursor)]) -> Option<bool> {
    urls_in_tails(tails).map(|urls| !urls.is_empty())
}

/// The PR URLs `pr create` results printed in the tails past the ledger's
/// cursors (`opened_in_tails`), every tail read. `None` when one could not
/// be read.
fn urls_in_tails(tails: &[(String, Cursor)]) -> Option<Vec<String>> {
    let mut all: Vec<(String, Cursor)> = tails.to_vec();
    for (path, _) in tails {
        for sub in subagents(path) {
            if !all.iter().any(|(p, _)| *p == sub) {
                all.push((sub, Cursor::default()));
            }
        }
    }
    let mut unread = false;
    let mut urls = Vec::new();
    for (path, cursor) in &all {
        match opened_in_tail(path, cursor) {
            Ok(mut found) => urls.append(&mut found),
            Err(()) => unread = true,
        }
    }
    (!unread).then_some(urls)
}

/// The PR URLs one transcript's tail past `cursor` printed as its session
/// opened them (T-750, T-797), read on the caller's thread and moving no
/// cursor: the pass still folds the same lines in. `Err` when it cannot be
/// read, or grew past what one pass reads; a file that is gone has nothing
/// new.
fn opened_in_tail(path: &str, cursor: &Cursor) -> std::result::Result<Vec<String>, ()> {
    let len = match std::fs::metadata(path) {
        Ok(m) => m.len(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err(()),
    };
    // Shorter than the cursor: rewritten, so read from the start.
    let (offset, mut pending, mut pending_act) = if len < cursor.offset {
        (0, Vec::new(), Vec::new())
    } else {
        (cursor.offset, cursor.pending_create.clone(), cursor.pending_act.clone())
    };
    if len == offset {
        return Ok(Vec::new());
    }
    if len - offset > READ_CAP {
        return Err(());
    }
    let mut file = std::fs::File::open(path).map_err(|_| ())?;
    file.seek(SeekFrom::Start(offset)).map_err(|_| ())?;
    let mut buf = Vec::new();
    file.take(len - offset).read_to_end(&mut buf).map_err(|_| ())?;
    let Some(end) = buf.iter().rposition(|b| *b == b'\n') else { return Ok(Vec::new()) };
    // The acted entries beside the opened URLs (T-859): a review in the
    // tail the pass has not reached is owed a read as a `pr create` is.
    Ok(String::from_utf8_lossy(&buf[..end])
        .lines()
        .flat_map(|line| {
            let named = named_on(line, &mut pending, &mut pending_act);
            named.opened.into_iter().chain(named.acted)
        })
        .collect())
}

/// A line's moment: its own stamp, or the pass's when it carries none.
fn stamp(at_ms: u64, now_ms: u64) -> u64 {
    if at_ms == 0 {
        now_ms
    } else {
        at_ms
    }
}

/// Startup loader: the ledger, any notices, and whether writes are barred.
pub fn load_or_recover(paths: &Paths) -> (Ledger, Vec<Notice>, bool) {
    let (ledger, notices, barred) = crate::store::load_versioned(
        &paths.costs_file(),
        COSTS_SCHEMA,
        "the tickets' costs",
        ", and counting starts again",
    );
    (ledger.unwrap_or_default(), notices, barred)
}

/// The ledger as it stands on disk, for a reader that writes nothing
/// (doctor): `None` when it is missing, unreadable or a newer build's.
pub fn read_only(paths: &Paths) -> Option<Ledger> {
    crate::store::parse_versioned(&std::fs::read_to_string(paths.costs_file()).ok()?, COSTS_SCHEMA)
        .ok()
}

/// `Ledger`'s shape with this build's schema, borrowed rather than cloned
/// (T-839): the ledger is megabytes and only its version changes.
pub fn save(paths: &Paths, ledger: &Ledger) -> Result<()> {
    #[derive(Serialize)]
    struct Borrowed<'a> {
        schema_version: u32,
        tickets: &'a BTreeMap<ulid::Ulid, TicketLedger>,
    }
    let body = Borrowed { schema_version: COSTS_SCHEMA, tickets: &ledger.tickets };
    crate::store::write_atomic(
        &paths.costs_file(),
        &serde_json::to_string(&body)?,
        crate::store::PRIVATE,
    )
}

/// Where a session's transcript is, if it is one this ledger reads.
pub fn transcript_of(record: &mesimon_core::board::SessionRecord) -> Option<(PathBuf, bool)> {
    use mesimon_core::board::SessionKind;
    let path = PathBuf::from(record.transcript_path.as_ref()?);
    match record.kind {
        SessionKind::Claude => Some((path, false)),
        SessionKind::Codex => Some((path, true)),
        SessionKind::Bash => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_790_870_640_000;

    fn line(id: &str, out: u64) -> String {
        format!(
            r#"{{"type":"assistant","requestId":"r","timestamp":"2026-10-01T16:04:00.000Z","message":{{"id":"{id}","model":"claude-opus-5-5","usage":{{"input_tokens":1,"output_tokens":{out}}}}}}}"#
        )
    }

    /// A message stamped `at` (`16:MM:SS`), with 1-hour cache writes.
    fn line_at(id: &str, out: u64, at: &str) -> String {
        format!(
            r#"{{"type":"assistant","requestId":"r","timestamp":"2026-10-01T{at}.000Z","message":{{"id":"{id}","model":"claude-opus-5-5","usage":{{"input_tokens":1,"output_tokens":{out},"cache_creation":{{"ephemeral_1h_input_tokens":10,"ephemeral_5m_input_tokens":0}}}}}}}}"#
        )
    }

    /// The mod's count (T-581) and the tail's stand side by side: the first
    /// main turn fences the conversation, the tail's lines past the fence go
    /// to the check, and every later reported turn is the ticket's.
    #[test]
    fn a_mods_turns_are_counted_once_and_the_tail_is_their_check() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("s.jsonl");
        let path = main.display().to_string();
        let ms = |hh_mm_ss: &str| {
            mesimon_core::usage::rfc3339_ms(&format!("2026-10-01T{hh_mm_ss}Z")).unwrap()
        };
        std::fs::write(&main, format!("{}\n", line_at("m1", 100, "16:00:00"))).unwrap();
        let t = ulid::Ulid::new();
        let mut ledger = Ledger::default();
        ledger.learn(t, &path, false, true);
        ledger.apply(scan(ledger.jobs(), NOW));
        assert_eq!(ledger.view(NOW)[0].tokens, 111, "before any report the tail counts");
        let turn = Tokens { input: 1, output: 100, write_5m: 10, ..Tokens::default() };
        // A subagent's turn before the fence is the tail's; the first main
        // turn sets the fence and adds nothing (its lines were read).
        assert!(!ledger.count_mod(t, &path, true, ms("16:00:01"), "claude-opus-5-5", turn));
        assert!(ledger.count_mod(t, &path, false, ms("16:00:02"), "claude-opus-5-5", turn));
        assert_eq!(ledger.view(NOW)[0].tokens, 111);
        // The next turn: the tail reads it into the check, the mod's report
        // into the hours, its writes at the hour the transcript showed.
        std::fs::create_dir_all(dir.path().join("s/subagents")).unwrap();
        std::fs::write(
            dir.path().join("s/subagents/agent-1.jsonl"),
            format!("{}\n", line_at("a1", 5, "16:00:03")),
        )
        .unwrap();
        let mut f = std::fs::OpenOptions::new().append(true).open(&main).unwrap();
        use std::io::Write;
        writeln!(f, "{}", line_at("m2", 100, "16:00:04")).unwrap();
        ledger.apply(scan(ledger.jobs(), NOW));
        assert!(ledger.count_mod(
            t,
            &path,
            true,
            ms("16:00:05"),
            "claude-opus-5-5",
            Tokens { input: 1, output: 5, write_5m: 10, ..Tokens::default() }
        ));
        assert!(ledger.count_mod(t, &path, false, ms("16:00:06"), "claude-opus-5-5", turn));
        let l = &ledger.tickets[&t];
        assert_eq!(l.check, Check { by_mod: 127, by_tail: 127 });
        assert_eq!(ledger.view(NOW)[0].tokens, 111 + 127, "each turn once");
        let writes: u64 = l.hours.values().flat_map(|m| m.values()).map(|t| t.write_5m).sum();
        assert_eq!(writes, 0, "the transcript said the writes live an hour");
        assert!(ledger.disagreements().is_empty());
        // A mod that went quiet leaves the two apart, and doctor says so.
        writeln!(f, "{}", line_at("m3", 5_000, "16:00:07")).unwrap();
        ledger.apply(scan(ledger.jobs(), NOW));
        assert_eq!(ledger.disagreements(), vec![(t, 127, 127 + 5_011)]);
        assert_eq!(ledger.by_mod(), 1);
        // A wake on an older Claude Code hands the conversation back.
        ledger.learn(t, &path, false, false);
        writeln!(f, "{}", line_at("m4", 100, "16:00:08")).unwrap();
        ledger.apply(scan(ledger.jobs(), NOW));
        assert_eq!(ledger.view(NOW)[0].tokens, 111 + 127 + 111);
        // The ledger round-trips with its check and its hint.
        let back: Ledger = serde_json::from_str(&serde_json::to_string(&ledger).unwrap()).unwrap();
        assert_eq!(back, ledger);
    }

    #[test]
    fn a_transcript_is_read_once_a_message_and_only_what_grew() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        let t = ulid::Ulid::new();
        // Two lines of one message (two content blocks), one user line, and a
        // half-written line the pass must leave for the next.
        let body = format!(
            "{}\n{}\n{{\"type\":\"user\"}}\n{}\n{{\"type\":\"assis",
            line("m1", 100),
            line("m1", 100),
            line("m2", 50)
        );
        std::fs::write(&path, &body).unwrap();
        let mut ledger = Ledger::default();
        assert!(ledger.learn(t, &path.display().to_string(), false, false));
        assert!(!ledger.learn(t, &path.display().to_string(), false, false));
        assert!(ledger.apply(scan(ledger.jobs(), NOW)));
        let c = &ledger.view(NOW)[0];
        assert_eq!(c.tokens, 152, "m1 once, m2 once");
        assert!((c.usd - (2.0 * 4.0 + 150.0 * 20.0) / 1e6).abs() < 1e-12);
        // Nothing grew: nothing read.
        assert!(scan(ledger.jobs(), NOW).is_empty());
        // The half line completes, and a third message lands.
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        use std::io::Write;
        writeln!(f, "tant\"}}").unwrap();
        writeln!(f, "{}", line("m3", 10)).unwrap();
        assert!(ledger.apply(scan(ledger.jobs(), NOW)));
        assert_eq!(ledger.view(NOW)[0].tokens, 163);
    }

    /// The `opened here` fact (T-739): a `pr create` and its result, read
    /// across two passes, land on the ticket's ledger once.
    #[test]
    fn a_pr_create_in_the_transcript_binds_its_url_to_the_ticket() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        let t = ulid::Ulid::new();
        let call = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_9","name":"Bash","input":{"command":"gh pr create --fill"}}]}}"#;
        std::fs::write(
            &path,
            format!(
                "{call}
"
            ),
        )
        .unwrap();
        let mut ledger = Ledger::default();
        ledger.learn(t, &path.display().to_string(), false, false);
        ledger.apply(scan(ledger.jobs(), NOW));
        assert!(ledger.opened_urls(t).is_empty(), "the result has not come");
        assert_eq!(
            ledger.tickets[&t].conversations.values().next().unwrap().pending_create,
            ["toolu_9"]
        );
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        use std::io::Write;
        writeln!(f, r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","tool_use_id":"toolu_9","content":"https://github.com/o/r/pull/42\n"}}]}}}}"#).unwrap();
        assert!(ledger.apply(scan(ledger.jobs(), NOW)));
        assert_eq!(ledger.opened_urls(t), ["https://github.com/o/r/pull/42"]);
        assert!(ledger.tickets[&t]
            .conversations
            .values()
            .next()
            .unwrap()
            .pending_create
            .is_empty());
        // Said again, it is kept once.
        writeln!(f, "{call}").unwrap();
        writeln!(f, r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","tool_use_id":"toolu_9","content":"https://github.com/o/r/pull/42/"}}]}}}}"#).unwrap();
        ledger.apply(scan(ledger.jobs(), NOW));
        assert_eq!(ledger.opened_urls(t).len(), 1);
        let text = serde_json::to_string(&ledger).unwrap();
        assert!(text.contains("pr_urls"), "the fact survives a restart: {text}");
    }

    /// The `acted on here` fact (T-859): a `gh pr review` and its empty
    /// result land the number on the ticket's ledger, kept apart from the
    /// opened URLs and surviving a restart; a tail the pass has not reached
    /// shows it as owed.
    #[test]
    fn an_act_on_a_pr_in_the_transcript_names_it_on_the_ticket() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        let t = ulid::Ulid::new();
        let call = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_7","name":"Bash","input":{"command":"gh pr review 43 --approve"}}]}}"#;
        std::fs::write(&path, format!("{call}\n")).unwrap();
        let mut ledger = Ledger::default();
        ledger.learn(t, &path.display().to_string(), false, false);
        ledger.apply(scan(ledger.jobs(), NOW));
        assert!(ledger.acted_entries(t).is_empty(), "the result has not come");
        assert_eq!(
            ledger.tickets[&t].conversations.values().next().unwrap().pending_act,
            ["toolu_7 #43"]
        );
        // Unread tail: the act is owed as a `pr create` is.
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        use std::io::Write;
        writeln!(f, r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","tool_use_id":"toolu_7","content":""}}]}}}}"#).unwrap();
        assert_eq!(ledger.opened_unread(t, &[], |e| e == "#43"), Some(true));
        assert!(ledger.apply(scan(ledger.jobs(), NOW)));
        assert_eq!(ledger.acted_entries(t), ["#43"]);
        assert!(ledger.opened_urls(t).is_empty(), "an act opened nothing");
        assert!(ledger.opened_owed(t, |e| e == "#43"));
        let text = serde_json::to_string(&ledger).unwrap();
        assert!(text.contains("pr_acted"), "the fact survives a restart: {text}");
        let back: Ledger = serde_json::from_str(&text).unwrap();
        assert_eq!(back.acted_entries(t), ["#43"]);
    }

    /// T-810: the backfill reads a transcript the cursor passed before
    /// anything looked for `pr create`, from its start and line by line:
    /// Claude's result as a JSON-escaped string and as text blocks, and
    /// Codex's `function_call_output`. Its URLs join the ticket's as the
    /// oldest, no cursor moves, every transcript read is marked, and a
    /// transcript learned now is born read.
    #[test]
    fn the_backfill_finds_the_pr_creates_a_passed_cursor_never_read() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join("c.jsonl");
        let codex = dir.path().join("x.jsonl");
        let lines = [
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"gh pr create --fill --head fix/x"}}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"opening it"}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"Creating pull request for fix/x into main\n\nhttps://github.com/o/r/pull/7\n"}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_2","name":"Bash","input":{"command":"gh pr create -t again"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_2","content":[{"type":"text","text":"https://github.com/o/r/pull/8\n"}]}]}}"#,
        ];
        std::fs::write(&claude, lines.join("\n") + "\n").unwrap();
        let fc = r#"{"type":"response_item","payload":{"type":"function_call","name":"shell","call_id":"c1","arguments":"{\"command\":[\"bash\",\"-lc\",\"glab mr create --fill --yes\"]}"}}"#;
        let fo = r#"{"type":"response_item","payload":{"type":"function_call_output","call_id":"c1","output":"https://gitlab.example.com/g/p/-/merge_requests/9\n"}}"#;
        std::fs::write(&codex, format!("{fc}\n{fo}")).unwrap();
        let t = ulid::Ulid::new();
        let (c, x) = (claude.display().to_string(), codex.display().to_string());
        let mut ledger = Ledger::default();
        // Passed by a cursor from before T-739: at the end, never looked.
        let passed = |codex: bool| Cursor { codex, offset: 1 << 20, ..Cursor::default() };
        let mine = ledger.tickets.entry(t).or_default();
        mine.conversations.insert(c.clone(), passed(false));
        mine.conversations.insert(x.clone(), passed(true));
        mine.pr_urls.push("https://github.com/o/r/pull/99".into());
        let other = ulid::Ulid::new();
        ledger.learn(other, &dir.path().join("new.jsonl").display().to_string(), false, false);
        let jobs = ledger.backfill_jobs(&HashSet::from([t, other]));
        assert_eq!(jobs.len(), 2, "a transcript learned now is born read: {jobs:?}");
        let found = backfill(jobs);
        let urls: Vec<&str> =
            found.iter().flat_map(|b| b.urls.iter().map(String::as_str)).collect();
        assert_eq!(
            urls,
            [
                "https://github.com/o/r/pull/7",
                "https://github.com/o/r/pull/8",
                "https://gitlab.example.com/g/p/-/merge_requests/9"
            ]
        );
        let before = ledger.tickets[&t].conversations.clone();
        let added = ledger.apply_backfill(found);
        assert_eq!(added.len(), 3);
        assert_eq!(
            ledger.opened_urls(t).last().map(String::as_str),
            Some("https://github.com/o/r/pull/99"),
            "the newest stays newest"
        );
        assert_eq!(ledger.opened_urls(t).len(), 4);
        for (path, c) in &ledger.tickets[&t].conversations {
            assert!(c.backfilled, "{path} marked");
            assert_eq!(c.offset, before[path].offset, "no cursor moves");
        }
        assert!(ledger.backfill_jobs(&HashSet::from([t])).is_empty(), "once");
        assert!(ledger.apply_backfill(backfill(vec![(t, c.clone())])).is_empty(), "kept once");
        // A pass that ran beside it keeps the mark.
        let stale = Done {
            ticket: t,
            path: c.clone(),
            cursor: passed(false),
            added: Vec::new(),
            pr_urls: Vec::new(),
            pr_acted: Vec::new(),
        };
        ledger.apply(vec![stale]);
        assert!(ledger.tickets[&t].conversations[&c].backfilled);
    }

    /// The gate's read (T-750): a `pr create` in the tail the pass has not
    /// reached is opened here, read now and moving no cursor; a transcript
    /// with none, or a ticket with no transcript, opened nothing.
    #[test]
    fn a_pr_opened_in_the_unread_tail_is_seen_before_the_pass() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        let p = path.display().to_string();
        let t = ulid::Ulid::new();
        let mut ledger = Ledger::default();
        let owed = |_: &str| true;
        assert_eq!(
            ledger.opened_unread(t, &[], owed),
            Some(false),
            "no transcript, nothing opened"
        );
        std::fs::write(&path, format!("{}\n", line("m1", 10))).unwrap();
        assert_eq!(
            ledger.opened_unread(t, std::slice::from_ref(&p), owed),
            Some(false),
            "a live one read whole"
        );
        ledger.learn(t, &p, false, false);
        ledger.apply(scan(ledger.jobs(), NOW));
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        use std::io::Write;
        writeln!(f, r#"{{"type":"assistant","message":{{"content":[{{"type":"tool_use","id":"toolu_7","name":"Bash","input":{{"command":"glab mr create --fill"}}}}]}}}}"#).unwrap();
        assert_eq!(ledger.opened_unread(t, &[], owed), Some(false), "the result has not come");
        writeln!(f, r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","tool_use_id":"toolu_7","content":"https://gitlab.com/g/p/-/merge_requests/3\n"}}]}}}}"#).unwrap();
        let before = ledger.clone();
        assert_eq!(ledger.opened_unread(t, &[], owed), Some(true));
        // A URL a read already holds is owed nothing (T-797), in the tail
        // and in the ledger alike.
        let read = |u: &str| !u.ends_with("merge_requests/3");
        assert_eq!(ledger.opened_unread(t, &[], read), Some(false));
        assert_eq!(ledger, before, "the gate's read moves no cursor");
        // The pass folds the same lines in; the ledger answers from then on.
        ledger.apply(scan(ledger.jobs(), NOW));
        assert_eq!(ledger.opened_urls(t).len(), 1);
        assert_eq!(ledger.opened_unread(t, &[], read), Some(false));
        std::fs::remove_file(&path).unwrap();
        assert_eq!(ledger.opened_unread(t, &[], owed), Some(true));
        // A transcript that is gone has nothing new.
        let u = ulid::Ulid::new();
        ledger.learn(u, &p, false, false);
        assert_eq!(ledger.opened_unread(u, &[], owed), Some(false));
    }

    #[test]
    fn subagents_count_to_the_ticket_and_codex_counts_growth() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("abc.jsonl");
        std::fs::write(&main, format!("{}\n", line("m1", 10))).unwrap();
        std::fs::create_dir_all(dir.path().join("abc/subagents")).unwrap();
        std::fs::write(
            dir.path().join("abc/subagents/agent-1.jsonl"),
            format!("{}\n", line("s1", 20)),
        )
        .unwrap();
        let rollout = dir.path().join("rollout.jsonl");
        std::fs::write(
            &rollout,
            concat!(
                r#"{"timestamp":"2026-10-01T16:00:00Z","type":"turn_context","payload":{"model":"gpt-6-astra"}}"#,
                "\n",
                r#"{"timestamp":"2026-10-01T16:01:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":60,"output_tokens":10}}}}"#,
                "\n",
                r#"{"timestamp":"2026-10-01T16:02:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":150,"cached_input_tokens":90,"output_tokens":20}}}}"#,
                "\n"
            ),
        )
        .unwrap();
        let (a, b) = (ulid::Ulid::new(), ulid::Ulid::new());
        let mut ledger = Ledger::default();
        ledger.learn(a, &main.display().to_string(), false, false);
        ledger.learn(b, &rollout.display().to_string(), true, false);
        ledger.apply(scan(ledger.jobs(), NOW));
        let view = ledger.view(NOW);
        let of = |t| view.iter().find(|c| c.ticket == t).unwrap();
        assert_eq!(of(a).tokens, 2 + 30, "the subagent's tokens are the ticket's");
        assert_eq!(ledger.tickets[&a].conversations.len(), 2, "and its transcript is learned");
        assert_eq!((of(b).tokens, of(b).unpriced), (170, 0));
        assert!((of(b).usd - 0.00169).abs() < 1e-12, "cached input is priced only once");
        // The ledger round-trips, and a board without the ticket drops it.
        let paths = {
            let repo = dir.path().join("repo");
            std::fs::create_dir_all(&repo).unwrap();
            let mut p = Paths::for_repo(&repo).unwrap();
            p.state_dir = dir.path().join("state");
            std::fs::create_dir_all(&p.state_dir).unwrap();
            p
        };
        save(&paths, &ledger).unwrap();
        // The borrowed body is the ledger's own shape (T-839): a field
        // added to `Ledger` and not to it fails here.
        let whole = Ledger { schema_version: COSTS_SCHEMA, ..ledger.clone() };
        assert_eq!(
            std::fs::read_to_string(paths.costs_file()).unwrap(),
            serde_json::to_string(&whole).unwrap()
        );
        let (back, notices, barred) = load_or_recover(&paths);
        assert!(notices.is_empty() && !barred);
        assert_eq!(back.tickets, ledger.tickets);
        let mut kept = back;
        assert!(kept.keep(&[a].into_iter().collect()));
        assert_eq!(kept.tickets.len(), 1);
        // A newer file is left alone and barred; a broken one is set aside.
        std::fs::write(paths.costs_file(), r#"{"schema_version":9}"#).unwrap();
        let (_, notices, barred) = load_or_recover(&paths);
        assert!(barred && notices[0].text.contains("newer mesimon"));
        std::fs::write(paths.costs_file(), "{not json").unwrap();
        let (fresh, notices, barred) = load_or_recover(&paths);
        assert!(!barred && fresh.tickets.is_empty() && notices[0].text.contains("set aside"));
    }
}
