//! Board persistence. Repo side (D12 layout, uncommitted per D34.9):
//!   .mesimon/board/columns.toml
//!   .mesimon/board/tickets/<SHORT-KEY>/ticket.toml   (ULID never in a path — D24)
//! Runtime side (D33b): sessions.json in the state dir.
//!
//! Files mesimon authors are rewritten atomically (temp + rename). mesimon never
//! round-trip-rewrites a user's file (D26) — spec.md etc. are opaque blobs.

use std::path::Path;

use anyhow::Result;
use mesimon_core::board::{AgentProvider, Board, Column, SessionRecord, Ticket, KEY_PREFIX};
use mesimon_core::command::Notice;
use mesimon_core::prompts::PromptSet;
use mesimon_core::tier::MachineTiers;
use serde::{Deserialize, Serialize};

use crate::paths::Paths;

pub(crate) mod imports;

/// On-disk schema stamps (16 §6.2). Four state files, four independent
/// counters — a ticket change must not force a sessions migration. Absent is
/// read as 1; newer than ours refuses THAT file and bars writes to it.
/// v2 added the tag registry. Bumped rather than defaulted on purpose: at v1
/// an older build would read the file, ignore `tags`, and DROP the whole
/// registry on its next write. The stamp makes it refuse the file and bar its
/// writes instead — 16 §6.2's rule that a newer file is left untouched rather
/// than silently downgraded.
/// v3 (T-217) added `mcp_tools`. Bumped for the same reason once more, and
/// the reason is sharper here than it was for `tags_seeded`, which rode a
/// serde default with no bump at all: a build that drops `tags_seeded` only
/// re-offers three tags, but a build that drops `mcp_tools = false` hands
/// every agent on the board its tools back after the user took them away.
/// A consent flag may not be lost by a downgrade. `claude_md_ignored` rides
/// the same bump.
/// v4 (T-117) put the automations on the columns: every `[[columns]]` table
/// may carry `on_working`, `on_done`, `train`, `requires_merge`, `reclaim`,
/// `claude_mode`, `agent_tools`, `auto_run`, `workspace`, `collapsed`. A
/// bump for the `mcp_tools` reason: a v3 build reading the file would drop
/// `agent_tools = "read"` and `claude_mode = "plan"` on its next write and
/// every claude spawned there would get the full tier and the user's own
/// mode back — a widening. Loading a v3 file seeds the four template columns
/// with what they DID (`Board::seed_template_settings`) and stamps 4; a v4
/// file is never re-seeded, so a rule removed by hand stays removed.
/// v5 adds the project provider. Older builds must not silently drop Codex
/// selection and start a different provider on the next request.
/// v6 (T-443) adds the board's agent tiers (`[[tiers]]`) and its default
/// tier — the tags reason once more: a v5 build would read the file, ignore
/// the registry and drop it on its next write, and a board whose default
/// was a Codex tier would start Claude.
pub const COLUMNS_SCHEMA: u32 = 6;
/// v2 added `[[notes]]`, on the columns file's reasoning: at v1 an older
/// build would read the ticket, ignore the array, and on its next
/// `save_ticket` drop every note's metadata while the files stayed behind
/// as orphans. v3 (T-74) added the snooze — `archived.until`/`needs_you`
/// and `woke_at` — for the same reason: a v2 build would drop the deadline
/// on its next save and the ticket would sleep forever. The cost is the
/// same trade: a v3 file is NOT loaded by a v2 build (a notice, and the
/// ticket is absent there until the newer build is back). v4 (T-227) added
/// `manual_merge`, the ticket's opt-out from the merge train — the
/// `mcp_tools` argument: a v3 build would drop it on its next save and the
/// train, re-armed, would merge a branch the user had taken off it.
/// v5 adds the non-toggleable execution policy. Older readers must refuse
/// the file instead of dropping the restriction and putting it on the train.
/// v6 adds import provenance, which also imposes an execution floor. A v5
/// reader must not discard its correlation or weaken a partially imported ticket.
/// v7 (T-443) adds the ticket's agent tier — the v5-provider reason: a v6
/// build would drop a Codex tier pick and start Claude on the ticket.
pub const TICKET_SCHEMA: u32 = 7;
/// `tiers.toml`, the machine's agent tiers (T-443) — one file under the
/// state root that every board's daemon reads and writes. Its own counter,
/// on the same doctrine: newer than ours is left untouched and not written.
pub const MACHINE_TIERS_SCHEMA: u32 = 1;
/// v2 adds Codex session kinds, exact thread identity and observation holds.
/// Older readers must refuse before decoding an unfamiliar session kind,
/// rather than quarantine the file and forget ownership of its live panes.
pub const SESSIONS_SCHEMA: u32 = 2;

fn schema_v1() -> u32 {
    1
}

/// `ColumnsFile::mcp_tools` defaults ON — see the field.
fn default_crown_budget() -> u8 {
    mesimon_core::board::DEFAULT_CROWN_BUDGET
}

fn yes() -> bool {
    true
}

#[derive(Serialize, Deserialize, Default)]
struct ColumnsFile {
    /// MUST be first: it is a scalar and `columns` serializes as `[[columns]]`,
    /// an array of tables. Any scalar after a table is a TOML serialize error.
    #[serde(default = "schema_v1")]
    schema_version: u32,
    next_key: u64,
    #[serde(default)]
    agent_provider: AgentProvider,
    #[serde(default)]
    park_after_minutes: u32,
    /// The crown's spawn budget (`Board::crown_budget`, T-412). A scalar,
    /// so it sits here; absent — every file before the field — means the
    /// default of three, and no bump: a build that drops it falls back to
    /// that same number, and the count it caps is in `sessions.json`.
    #[serde(default = "default_crown_budget")]
    crown_budget: u8,
    /// Whether the starter tags were offered (`Board::tags_seeded`). A scalar,
    /// so it sits here, before the tables. Absent on every file written
    /// before 2026-09-04, which is what makes an existing board's first load
    /// on this build the offer.
    #[serde(default)]
    tags_seeded: bool,
    /// Whether this board hands its sessions the MCP tool surface
    /// (`Board::mcp_tools`, T-217). Another scalar, so it sits up here with
    /// the others. Its default is TRUE, which is why it names a function
    /// rather than riding `bool`'s own default: a file written before this
    /// field existed means "on", not "the user turned the tools off".
    #[serde(default = "yes")]
    mcp_tools: bool,
    /// The agent-brief offer was answered "never" (`Board::claude_md_ignored`;
    /// the key keeps T-217's name so nobody is re-asked).
    #[serde(default)]
    claude_md_ignored: bool,
    /// The agent brief is on (`Board::system_prompt`, T-224). Off by default
    /// and a plain default with no schema bump: a build that drops it sends
    /// LESS to the model, the safe direction — see the field on `Board`.
    #[serde(default)]
    system_prompt: bool,
    /// Where an agent's `create_ticket` lands with no column named
    /// (`Board::default_column`, T-279). A scalar, so it sits here before
    /// the tables; absent — every file written before it — means the first
    /// column, the old behaviour, and a plain default with no bump: a build
    /// that drops it narrows nothing an agent gets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_column: Option<String>,
    #[serde(default)]
    follow_up_mode: mesimon_core::board::FollowUpMode,
    /// The three agent-prompt templates (`Board::prompts`, T-353), one
    /// scalar each rather than one `[prompts]` table: a table here could be
    /// followed by no scalar, and `columns` and `tags` already own that
    /// ground. Absent — every file before the field, and every board that
    /// never edited one — means mesimon's own words, which are in the binary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prompt_rebase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prompt_merged: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prompt_note_updated: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prompt_crown_wake: Option<String>,
    /// The crowned ticket (`Board::crown`, T-411), a ULID. A scalar, so it
    /// sits here before the tables; absent — every file before the field —
    /// means no crown, and a plain default with no bump: a build that drops
    /// it takes authority away from an agent, never hands any out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    crown: Option<ulid::Ulid>,
    /// The board's default agent tier (`Board::default_tier`, v6), a tier
    /// id. A scalar, so it sits here before the tables.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_tier: Option<String>,
    columns: Vec<Column>,
    /// The tag registry (v2). Another array of tables, so it may follow
    /// `columns` but must stay after every scalar.
    #[serde(default)]
    tags: Vec<mesimon_core::board::Tag>,
    /// The board's agent tiers (v6): its own and its overrides of the
    /// machine's. An array of tables, after the others.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tiers: Vec<mesimon_core::tier::Tier>,
}

/// `tiers.toml` with its stamp, the `TicketFile` shape.
#[derive(Serialize, Deserialize, Default)]
struct MachineTiersFile {
    /// First, a scalar, for the `[[tiers]]` reason.
    #[serde(default = "schema_v1")]
    schema_version: u32,
    #[serde(flatten)]
    tiers: MachineTiers,
}

/// `ticket.toml` with its schema stamp. The stamp lives here rather than on
/// `Ticket` so it stays a disk concern: off the wire, out of `mesimon-core`,
/// and out of every `Ticket { .. }` literal and rendering fixture.
#[derive(Serialize, Deserialize)]
struct TicketFile {
    /// MUST be first, and a scalar: `[archived]` is a TOML table and any
    /// scalar serialized after a table errors (`archived_table_roundtrips`).
    #[serde(default = "schema_v1")]
    schema_version: u32,
    #[serde(flatten)]
    ticket: Ticket,
}

/// `sessions.json`. The legacy shape is a bare array, which is why `load`
/// sniffs for `[` rather than reaching for `#[serde(untagged)]`.
#[derive(Serialize, Deserialize)]
struct SessionsFile {
    schema_version: u32,
    sessions: Vec<SessionRecord>,
}

/// State-dir files: argv (with `--resume <id>`), socket and transcript paths.
pub(crate) const PRIVATE: u32 = 0o600;
/// Board files inside the repo: the user's data, at the umask like any file.
pub(crate) const SHARED: u32 = 0o644;

/// `pub` for one outside caller: the TUI's per-machine `prefs.json`
/// (`mesimon-tui/src/prefs.rs`), which wants the same crash-safety and no
/// second copy of it.
pub fn write_atomic(path: &Path, content: &str, mode: u32) -> Result<()> {
    write_atomic_bytes(path, content.as_bytes(), mode)
}

pub fn write_atomic_bytes(path: &Path, content: &[u8], mode: u32) -> Result<()> {
    // 13 §13.9.1: temp + fsync + rename + directory fsync. Without the fsync
    // a crash between write and rename leaves a truncated file — precisely
    // the malformed input `load` now has to quarantine, so this is the
    // cheapest way to stop manufacturing them.
    //
    // Doc 13 measured ~170 µs and said "NOT F_FULLFSYNC" — but Rust's
    // `sync_all` IS `fcntl(F_FULLFSYNC)` on macOS, and that is what this
    // costs here: ~3 ms per call against ~0.1 ms for a bare `fsync(2)`,
    // ~8 ms for the whole shape (measured 2026-09-05, T-216). A price paid
    // on every `save_ticket` / `save_sessions`, on the writer thread; not
    // the hang T-216 was (that was 53 git forks), and left as it is —
    // the barrier is what makes the rename mean something on a power loss.
    //
    // `with_extension` REPLACES the extension, so the temp is `sessions.tmp`,
    // not `sessions.json.tmp` — deliberate: `load` reads exact names and the
    // flock singleton means there is never a second writer, and it keeps the
    // temp out of any `*.json`/`*.toml` glob.
    let tmp = path.with_extension("tmp");
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(mode)
            .open(&tmp)?;
        // `mode` only applies at creation; a leftover temp keeps its old bits.
        f.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(mode))?;
        std::io::Write::write_all(&mut f, content)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    if let Some(dir) = path.parent() {
        // Best-effort: the rename is already atomic for readers; this only
        // hardens it against power loss, and some filesystems refuse it.
        let _ = std::fs::File::open(dir).and_then(|d| d.sync_all());
    }
    Ok(())
}

/// What `load` found, beyond the board itself.
pub struct Loaded {
    pub board: Board,
    /// Standing advisories: a file quarantined, a file newer than this build.
    pub notices: Vec<Notice>,
    /// Writing `columns.toml` would destroy bytes we could not read — a newer
    /// file, or a quarantine rename that failed. Nothing may persist it.
    pub columns_write_barred: bool,
    pub sessions_write_barred: bool,
}

/// Move a file mesimon cannot read out of the way, preserving every byte.
///
/// Rename, not copy: a copy leaves the bad bytes at the canonical path, so the
/// next start quarantines them again, and the first save overwrites the
/// original anyway — the silent loss this exists to prevent. Same directory,
/// so the rename is intra-filesystem and can never fail EXDEV (`.mesimon/` may
/// live on a different device from `~/.local/state`). Epoch-millis suffix:
/// repeated failures never collide and the listing sorts by when it happened.
/// The version is deliberately NOT in the name — the version is often exactly
/// what could not be read.
pub fn quarantine(path: &Path) -> Option<std::path::PathBuf> {
    let ms = mesimon_core::clock::now_ms();
    let stem = path.file_name()?.to_string_lossy().into_owned();
    // Millisecond resolution is not enough on its own: two files quarantined
    // in the same millisecond would land on one name and the second rename
    // would DESTROY the first's bytes — the exact loss this function exists
    // to prevent. Probe for a free name (the flock singleton means there is
    // never a second writer racing us).
    let mut dest = path.with_file_name(format!("{stem}.quarantine-{ms}"));
    for n in 1..1000 {
        if !dest.exists() {
            break;
        }
        dest = path.with_file_name(format!("{stem}.quarantine-{ms}-{n}"));
    }
    if dest.exists() {
        return None; // pathological; the caller bars writes rather than clobber
    }
    std::fs::rename(path, &dest).ok().map(|_| dest)
}

/// 16 §6.2's `KT-C002`: `<<<<<<< ` at column 0 is an unresolved merge conflict
/// and says so — never a TOML syntax error pointing at line 43. `.mesimon/` is
/// git-excluded, but a board can still arrive through a copied checkout.
fn conflict_marker_line(text: &str) -> Option<usize> {
    text.lines().position(|l| l.starts_with("<<<<<<< ") || l.starts_with(">>>>>>> ")).map(|i| i + 1)
}

/// What to do with a file whose stamp we managed to read.
enum Verdict {
    /// Version equals ours, or is absent (16 §6.2: "absent -> treat as 1").
    Load,
    /// Newer than this build. 16 §6.2: refuse THAT file, do not guess. The
    /// bytes are valid — leave them alone and bar writes, or the next save
    /// silently downgrades a file written by a newer mesimon.
    Newer(u32),
}

fn verdict(found: u32, ours: u32) -> Verdict {
    // Older loads: every field defaults, and the one migration that has to
    // DO something — columns v3 → v4 — is `load_columns`'s, keyed on the
    // stamp it found (16 §6.2).
    if found > ours {
        Verdict::Newer(found)
    } else {
        Verdict::Load
    }
}

fn future_notice(path: &Path, found: u32, ours: u32) -> Notice {
    Notice::new(
        "future_version",
        format!(
            "{} was written by a newer mesimon (schema {found}, this build reads {ours}) — \
             left untouched and not written to",
            short_name(path)
        ),
    )
    .with_path(path.display())
}

fn quarantine_notice(path: &Path, moved: Option<&Path>, detail: String) -> Notice {
    let text = match moved {
        Some(m) => format!(
            "{} could not be read — kept as {} and started from a default",
            short_name(path),
            short_name(m)
        ),
        None => format!(
            "{} could not be read, and could not be moved aside — not written to",
            short_name(path)
        ),
    };
    Notice::new("quarantined", text).with_path(path.display()).with_detail(detail)
}

/// The last two path components — enough to identify the file in one line of
/// board chrome without spilling the user's home directory into it.
fn short_name(p: &Path) -> String {
    let mut it = p.components().rev().take(2).collect::<Vec<_>>();
    it.reverse();
    it.iter().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

/// After a lost `columns.toml`, `next_key` is gone — and `mint_ticket` derives
/// `short_key` from it while `save_ticket` does `create_dir_all` over the
/// result. Starting from 0 means the next ticket mints `T-1` and overwrites an
/// existing `T-1/ticket.toml`. Recover a floor from what is on disk, counting
/// directories whose `ticket.toml` we could NOT parse: those keys are taken
/// too, and reusing one is exactly the data loss being prevented.
fn recover_next_key(tickets_dir: &Path, parsed: &[Ticket]) -> u64 {
    let from_dirs = std::fs::read_dir(tickets_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            e.file_name().to_string_lossy().strip_prefix(KEY_PREFIX)?.parse::<u64>().ok()
        })
        .max()
        .unwrap_or(0);
    let from_parsed = parsed
        .iter()
        .filter_map(|t| t.short_key.strip_prefix(KEY_PREFIX)?.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    from_dirs.max(from_parsed)
}

/// A ticket whose column vanished with `columns.toml` would be invisible:
/// `column_tickets` matches by name, and v0.1 has no `Unfiled` (13:427). Put
/// the column back so the work stays on the board.
fn readd_missing_columns(board: &mut Board) {
    let missing: Vec<String> = {
        let mut seen: Vec<String> = board.columns.iter().map(|c| c.name.clone()).collect();
        let mut out = Vec::new();
        for t in &board.tickets {
            if !seen.iter().any(|n| n == &t.column) {
                seen.push(t.column.clone());
                out.push(t.column.clone());
            }
        }
        out
    };
    let mut prev = board.sorted_columns().last().map(|c| c.order.clone()).unwrap_or_default();
    for name in missing {
        let order = mesimon_core::fracindex::between(&prev, "");
        board.columns.push(Column::new(name, order.clone()));
        prev = order;
    }
}

/// Load `columns.toml`. Returns the board skeleton, whether writes to that
/// file are barred, whether the on-disk state was lost (so `next_key` must
/// be recovered from the ticket directories), and whether a migration
/// changed the board (so the caller writes it back at the new stamp).
fn load_columns(cols_path: &Path, notices: &mut Vec<Notice>) -> (Board, bool, bool, bool) {
    let defaults = || Board::with_default_columns();
    if !cols_path.is_file() {
        return (defaults(), false, false, false); // fresh repo; caller writes it out
    }
    let text = match std::fs::read_to_string(cols_path) {
        Ok(t) => t,
        Err(e) => {
            // Unreadable is not unparseable: leave it and bar writes.
            notices.push(
                Notice::new(
                    "quarantined",
                    format!("{} could not be opened", short_name(cols_path)),
                )
                .with_path(cols_path.display())
                .with_detail(e.to_string()),
            );
            return (defaults(), true, true, false);
        }
    };

    let fault = if let Some(line) = conflict_marker_line(&text) {
        Some(format!("unresolved merge conflict at line {line}"))
    } else {
        // Pass 1 keeps the parser's span for the common case (a truncated
        // write, a hand-edit typo); pass 2 keeps the field path for a shape
        // error. One from_str into an untagged enum would erase both.
        match toml::from_str::<toml::Value>(&text) {
            Err(e) => Some(e.to_string()),
            Ok(v) => {
                let found = v.get("schema_version").and_then(|s| s.as_integer()).unwrap_or(1);
                match verdict(found as u32, COLUMNS_SCHEMA) {
                    Verdict::Newer(n) => {
                        notices.push(future_notice(cols_path, n, COLUMNS_SCHEMA));
                        return (defaults(), true, true, false);
                    }
                    Verdict::Load => match v.try_into::<ColumnsFile>() {
                        Ok(cf) => {
                            let mut b = Board {
                                columns: cf.columns,
                                next_key: cf.next_key,
                                agent_provider: cf.agent_provider,
                                park_after_minutes: cf.park_after_minutes,
                                crown_budget: cf.crown_budget,
                                tags: cf.tags,
                                tags_seeded: cf.tags_seeded,
                                mcp_tools: cf.mcp_tools,
                                claude_md_ignored: cf.claude_md_ignored,
                                system_prompt: cf.system_prompt,
                                default_column: cf.default_column,
                                follow_up_mode: cf.follow_up_mode,
                                prompts: PromptSet {
                                    rebase: cf.prompt_rebase,
                                    merged: cf.prompt_merged,
                                    note_updated: cf.prompt_note_updated,
                                    crown_wake: cf.prompt_crown_wake,
                                },
                                crown: cf.crown,
                                tiers: cf.tiers,
                                default_tier: cf.default_tier,
                                ..Default::default()
                            };
                            // v3 → v4 (T-117): the template columns get the
                            // rules they had as literals, once, on the way
                            // to the new stamp.
                            let migrated = found < 4 && b.seed_template_settings();
                            return (b, false, false, migrated);
                        }
                        Err(e) => Some(e.to_string()),
                    },
                }
            }
        }
    };

    let detail = fault.unwrap_or_default();
    let moved = quarantine(cols_path);
    let barred = moved.is_none(); // could not move it: never write over it
    notices.push(quarantine_notice(cols_path, moved.as_deref(), detail));
    (defaults(), barred, true, false)
}

/// Load one `ticket.toml`. `None` excludes just that ticket from the board
/// (13 §13.10.3) — the others are untouched. A ticket absent from
/// `board.tickets` is never passed to `save_ticket`, so it is self-barring.
fn load_ticket(tp: &Path, notices: &mut Vec<Notice>) -> Option<Ticket> {
    let text = std::fs::read_to_string(tp).ok()?;
    let fault = if let Some(line) = conflict_marker_line(&text) {
        format!("unresolved merge conflict at line {line}")
    } else {
        match toml::from_str::<toml::Value>(&text) {
            Err(e) => e.to_string(),
            Ok(v) => {
                let found = v.get("schema_version").and_then(|s| s.as_integer()).unwrap_or(1);
                match verdict(found as u32, TICKET_SCHEMA) {
                    Verdict::Newer(n) => {
                        notices.push(future_notice(tp, n, TICKET_SCHEMA));
                        return None;
                    }
                    Verdict::Load => match v.try_into::<TicketFile>() {
                        Ok(tf) => return Some(tf.ticket),
                        Err(e) => e.to_string(),
                    },
                }
            }
        }
    };
    let moved = quarantine(tp);
    notices.push(quarantine_notice(tp, moved.as_deref(), fault));
    None
}

/// Load `sessions.json`. Returns the records and whether writes are barred.
fn load_sessions(sf: &Path, notices: &mut Vec<Notice>) -> (Vec<SessionRecord>, bool) {
    if !sf.is_file() {
        return (Vec::new(), false);
    }
    let text = match std::fs::read_to_string(sf) {
        Ok(t) => t,
        Err(e) => {
            notices.push(
                Notice::new("quarantined", format!("{} could not be opened", short_name(sf)))
                    .with_path(sf.display())
                    .with_detail(e.to_string()),
            );
            return (Vec::new(), true);
        }
    };

    let fault = match serde_json::from_str::<serde_json::Value>(&text) {
        // Pass 1: a syntax error here carries "line N column M".
        Err(e) => e.to_string(),
        Ok(v) => {
            // Shape sniff, not `#[serde(untagged)]`: untagged collapses every
            // failure into "did not match any variant", losing the line and
            // column 13 §13.10.3 requires. A legacy file is a bare array.
            if v.is_array() {
                match serde_json::from_value::<Vec<SessionRecord>>(v) {
                    Ok(recs) => return (recs, false),
                    Err(e) => e.to_string(),
                }
            } else {
                let found = v.get("schema_version").and_then(|s| s.as_u64()).unwrap_or(1) as u32;
                match verdict(found, SESSIONS_SCHEMA) {
                    Verdict::Newer(n) => {
                        notices.push(future_notice(sf, n, SESSIONS_SCHEMA));
                        return (Vec::new(), true);
                    }
                    Verdict::Load => match serde_json::from_value::<SessionsFile>(v) {
                        Ok(f) => return (f.sessions, false),
                        Err(e) => e.to_string(),
                    },
                }
            }
        }
    };

    let moved = quarantine(sf);
    let barred = moved.is_none();
    // Sessions are re-derivable: the panes are still on the tmux server.
    notices.push(
        quarantine_notice(sf, moved.as_deref(), fault)
            .with_detail("live panes survive on the tmux server; mesimon re-adopts what it can"),
    );
    (Vec::new(), barred)
}

/// Whether a board with no tags is given the starters at load.
/// `MESIMON_NO_TAG_SEED=1` is a test seam: the e2es that build a vocabulary
/// from nothing set it, so their registry starts empty.
fn seed_tags_wanted() -> bool {
    std::env::var_os("MESIMON_NO_TAG_SEED").is_none()
}

/// Load the board. A parse failure is a NOTICE, never an error: the daemon
/// must come up. `Err` is reserved for genuine environment faults.
pub fn load(paths: &Paths) -> Result<Loaded> {
    load_with(paths, seed_tags_wanted())
}

/// `load`, with the starter-tag offer decided by the caller (the tests, so
/// none of them has to touch the process environment).
pub fn load_with(paths: &Paths, seed_tags: bool) -> Result<Loaded> {
    let mut notices = Vec::new();
    let cols_path = paths.board_dir.join("board/columns.toml");
    let fresh = !cols_path.is_file();
    let (mut board, columns_write_barred, columns_lost, migrated) =
        load_columns(&cols_path, &mut notices);

    let tickets_dir = paths.board_dir.join("board/tickets");
    if tickets_dir.is_dir() {
        for entry in std::fs::read_dir(&tickets_dir)? {
            let entry = entry?;
            let tp = entry.path().join("ticket.toml");
            if tp.is_file() {
                if let Some(t) = load_ticket(&tp, &mut notices) {
                    board.tickets.push(t);
                }
            }
        }
    }

    if columns_lost {
        board.next_key = recover_next_key(&tickets_dir, &board.tickets);
        readd_missing_columns(&mut board);
    }
    // The starter offer: a board that never had a tag gets `STARTER_TAGS`
    // once, and a board with a vocabulary of its own is stamped as needing
    // none. Never onto a barred file — the stamp would be lost with the
    // write, and a quarantined board is not the moment to add to it.
    let seeded = seed_tags && !columns_write_barred && board.seed_starter_tags();
    // Write the defaults out only when the path is actually free: a fresh
    // repo, or a quarantine that succeeded in moving the bad file aside.
    if (fresh || columns_lost || seeded || migrated) && !columns_write_barred {
        save_columns(paths, &board)?;
    }

    let (sessions, sessions_write_barred) = load_sessions(&paths.sessions_file(), &mut notices);
    board.sessions = sessions;

    Ok(Loaded { board, notices, columns_write_barred, sessions_write_barred })
}

/// Everything `doctor` reads off `columns.toml`, from ONE parse (T-247): the
/// board's switches, the agent prompts, the default column and the columns
/// themselves. An unreadable file answers the shipped defaults, each in its
/// safe direction — the tools ON (doctor reporting "off" for a repo that
/// never said so would be worse than saying nothing), the brief OFF
/// (reporting a system-prompt line on for a repo that never asked for one
/// would be the worse mistake), mesimon's own prompt words, no default
/// column, and no columns at all: doctor never creates a board.
///
/// `load` is not an option for this: it seeds the starter tags and writes
/// `columns.toml` out for a repo that has none, and `mesimon doctor` runs
/// against arbitrary directories and may not create a board to answer a
/// question about one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnsScalars {
    pub agent_provider: AgentProvider,
    /// `Board::default_tier` and `Board::tiers` (T-443): the board's layer.
    pub default_tier: Option<String>,
    pub tiers: Vec<mesimon_core::tier::Tier>,
    /// `Board::crown_budget` (T-412).
    pub crown_budget: u8,
    /// `Board::mcp_tools` (T-217).
    pub mcp_tools: bool,
    /// `Board::system_prompt` (T-224).
    pub system_prompt: bool,
    /// `Board::default_column` (T-279): the chosen name while the file still
    /// lists that column, else `None`, which doctor reads as the first
    /// column — the daemon's own `landing_column` answer, so the two agree.
    pub default_column: Option<String>,
    /// `Board::prompts` (T-353).
    pub prompts: PromptSet,
    /// The columns in board order (T-117), `None` where there is no board to
    /// speak of. A v3 file answers with the template rules the daemon would
    /// seed, so doctor and the board agree.
    pub columns: Option<Vec<Column>>,
}

impl Default for ColumnsScalars {
    fn default() -> Self {
        Self {
            agent_provider: AgentProvider::default(),
            default_tier: None,
            tiers: Vec::new(),
            crown_budget: mesimon_core::board::DEFAULT_CROWN_BUDGET,
            mcp_tools: true,

            system_prompt: false,
            default_column: None,
            prompts: PromptSet::default(),
            columns: None,
        }
    }
}

pub fn read_columns_scalars(paths: &Paths) -> ColumnsScalars {
    let Some(cf) = read_columns_file(paths) else {
        return ColumnsScalars::default();
    };
    let default_column =
        cf.default_column.filter(|name| cf.columns.iter().any(|c| &c.name == name));
    let prompts = PromptSet {
        rebase: cf.prompt_rebase,
        merged: cf.prompt_merged,
        note_updated: cf.prompt_note_updated,
        crown_wake: cf.prompt_crown_wake,
    };
    let mut b = Board { columns: cf.columns, ..Default::default() };
    if cf.schema_version < 4 {
        b.seed_template_settings();
    }
    ColumnsScalars {
        agent_provider: cf.agent_provider,
        default_tier: cf.default_tier,
        tiers: cf.tiers,
        crown_budget: cf.crown_budget,
        mcp_tools: cf.mcp_tools,
        system_prompt: cf.system_prompt,
        default_column,
        prompts,
        columns: Some(b.sorted_columns().into_iter().cloned().collect()),
    }
}

/// `columns.toml` parsed, or `None` where it is missing or unreadable —
/// `read_columns_scalars` decides what that answers.
fn read_columns_file(paths: &Paths) -> Option<ColumnsFile> {
    let text = std::fs::read_to_string(paths.board_dir.join("board/columns.toml")).ok()?;
    toml::from_str::<ColumnsFile>(&text).ok()
}

pub fn save_columns(paths: &Paths, board: &Board) -> Result<()> {
    let cf = ColumnsFile {
        schema_version: COLUMNS_SCHEMA,
        next_key: board.next_key,
        agent_provider: board.agent_provider,
        park_after_minutes: board.park_after_minutes,
        crown_budget: board.crown_budget,
        tags_seeded: board.tags_seeded,
        mcp_tools: board.mcp_tools,
        claude_md_ignored: board.claude_md_ignored,
        system_prompt: board.system_prompt,
        default_column: board.default_column.clone(),
        follow_up_mode: board.follow_up_mode,
        prompt_rebase: board.prompts.rebase.clone(),
        prompt_merged: board.prompts.merged.clone(),
        prompt_note_updated: board.prompts.note_updated.clone(),
        prompt_crown_wake: board.prompts.crown_wake.clone(),
        crown: board.crown,
        default_tier: board.default_tier.clone(),
        columns: board.columns.clone(),
        tags: board.tags.clone(),
        tiers: board.tiers.clone(),
    };
    write_atomic(&paths.board_dir.join("board/columns.toml"), &toml::to_string_pretty(&cf)?, SHARED)
}

/// What reading `tiers.toml` found.
#[derive(Debug, Default)]
pub struct MachineTiersLoad {
    pub tiers: MachineTiers,
    /// A quarantine or a newer file, for the advisory row.
    pub notice: Option<Notice>,
    /// Writing would destroy bytes we could not read.
    pub barred: bool,
}

/// Read the machine's tiers. Missing is empty, not an error. A file that
/// does not parse is moved aside (every byte kept) and reads as empty; one
/// from a newer build is left alone and bars writes — `load_columns`'s
/// doctrine, for the one file every board shares.
pub fn load_machine_tiers(path: &Path) -> MachineTiersLoad {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return MachineTiersLoad::default(),
        Err(e) => {
            let notice =
                Notice::new("quarantined", format!("{} could not be opened", short_name(path)))
                    .with_path(path.display())
                    .with_detail(e.to_string());
            return MachineTiersLoad { notice: Some(notice), barred: true, ..Default::default() };
        }
    };
    let fault = match toml::from_str::<toml::Value>(&text) {
        Err(e) => e.to_string(),
        Ok(v) => {
            let found = v.get("schema_version").and_then(|s| s.as_integer()).unwrap_or(1);
            match verdict(found as u32, MACHINE_TIERS_SCHEMA) {
                Verdict::Newer(n) => {
                    return MachineTiersLoad {
                        notice: Some(future_notice(path, n, MACHINE_TIERS_SCHEMA)),
                        barred: true,
                        ..Default::default()
                    };
                }
                Verdict::Load => match v.try_into::<MachineTiersFile>() {
                    Ok(f) => return MachineTiersLoad { tiers: f.tiers, ..Default::default() },
                    Err(e) => e.to_string(),
                },
            }
        }
    };
    let moved = quarantine(path);
    MachineTiersLoad {
        barred: moved.is_none(),
        notice: Some(quarantine_notice(path, moved.as_deref(), fault)),
        ..Default::default()
    }
}

/// The machine's tiers for a READER that may change nothing — `doctor`:
/// no quarantine, no bar; a file that does not parse reads as `None`.
pub fn read_machine_tiers(path: &Path) -> Option<MachineTiers> {
    let text = std::fs::read_to_string(path).ok()?;
    toml::from_str::<MachineTiersFile>(&text).ok().map(|f| f.tiers)
}

/// Write the machine's tiers whole, private to the user like the rest of
/// the state root.
pub fn save_machine_tiers(path: &Path, tiers: &MachineTiers) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let f = MachineTiersFile { schema_version: MACHINE_TIERS_SCHEMA, tiers: tiers.clone() };
    write_atomic(path, &toml::to_string_pretty(&f)?, PRIVATE)
}

pub fn save_ticket(paths: &Paths, t: &Ticket) -> Result<()> {
    let dir = paths.board_dir.join("board/tickets").join(&t.short_key);
    std::fs::create_dir_all(&dir)?;
    let tf = TicketFile { schema_version: TICKET_SCHEMA, ticket: t.clone() };
    write_atomic(&dir.join("ticket.toml"), &toml::to_string_pretty(&tf)?, SHARED)
}

/// Where a ticket's note bodies live: `notes/<ULID>.md` beside `ticket.toml`.
/// The id is minted by the daemon, so the path is never built from a name
/// anybody chose (docs/15 §4.7).
fn note_path(paths: &Paths, short_key: &str, id: ulid::Ulid) -> std::path::PathBuf {
    paths.board_dir.join("board/tickets").join(short_key).join("notes").join(format!("{id}.md"))
}

/// Write one note body, whole. Not bar-gated, like `save_ticket`: a ticket
/// that could not be read is absent from the board and so self-bars.
pub fn save_note(paths: &Paths, short_key: &str, id: ulid::Ulid, text: &str) -> Result<()> {
    let path = note_path(paths, short_key, id);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    write_atomic(&path, text, SHARED)
}

pub fn read_note(paths: &Paths, short_key: &str, id: ulid::Ulid) -> std::io::Result<String> {
    std::fs::read_to_string(note_path(paths, short_key, id))
}

pub fn delete_note(paths: &Paths, short_key: &str, id: ulid::Ulid) -> Result<()> {
    match std::fs::remove_file(note_path(paths, short_key, id)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

pub fn delete_ticket_dir(paths: &Paths, short_key: &str) -> Result<()> {
    let dir = paths.board_dir.join("board/tickets").join(short_key);
    if dir.is_dir() {
        std::fs::remove_dir_all(dir)?;
    }
    Ok(())
}

pub fn save_sessions(paths: &Paths, board: &Board) -> Result<()> {
    let sf = SessionsFile { schema_version: SESSIONS_SCHEMA, sessions: board.sessions.clone() };
    write_atomic(&paths.sessions_file(), &serde_json::to_string_pretty(&sf)?, PRIVATE)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch repo + its Paths. `for_repo` keys the state dir off the
    /// canonical path, so each test gets its own sessions.json.
    fn scratch(name: &str) -> (std::path::PathBuf, Paths) {
        let dir = std::env::temp_dir().join(format!("msmn-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".mesimon/board/tickets")).unwrap();
        let paths = Paths::for_repo(&dir).unwrap();
        std::fs::create_dir_all(&paths.state_dir).unwrap();
        let _ = std::fs::remove_file(paths.sessions_file());
        (dir, paths)
    }

    fn cleanup(dir: &Path, paths: &Paths) {
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(&paths.state_dir);
    }

    fn write(p: &Path, body: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    fn ticket_body(key: &str, column: &str) -> String {
        format!(
            "id = \"01J8ZQ7VJ0000000000000000{}\"\nshort_key = \"{key}\"\n\
             title = \"t\"\ncolumn = \"{column}\"\norder = \"a0\"\n\
             created_at = \"@1788046350\"\n",
            &key[2..3]
        )
    }

    #[test]
    fn execution_policy_roundtrips_and_old_readers_refuse_its_schema() {
        use mesimon_core::board::ExecutionPolicy;
        let body = format!(
            "schema_version = 5\n{}execution_policy = \"owner_only\"\n",
            ticket_body("T-1", "TODO")
        );
        let parsed: TicketFile = toml::from_str(&body).unwrap();
        assert_eq!(parsed.ticket.execution_policy, ExecutionPolicy::OwnerOnly);
        assert!(!parsed.ticket.manual_merge);
        let encoded = toml::to_string_pretty(&parsed).unwrap();
        let back: TicketFile = toml::from_str(&encoded).unwrap();
        assert_eq!(back.ticket.execution_policy, ExecutionPolicy::OwnerOnly);
        assert!(matches!(verdict(parsed.schema_version, 4), Verdict::Newer(5)));
        let old: TicketFile = toml::from_str(&ticket_body("T-1", "TODO")).unwrap();
        assert_eq!(old.ticket.execution_policy, ExecutionPolicy::LocalAutomation);
        let unknown = body.replace("owner_only", "future_policy");
        assert!(toml::from_str::<TicketFile>(&unknown).is_err());
    }

    #[test]
    fn import_provenance_roundtrips_and_imposes_a_durable_floor() {
        use mesimon_core::board::ExecutionPolicy;
        use mesimon_core::content::{ImportOrigin, ImportPlacement, PreparedImport, TicketContent};
        let origin = ImportOrigin { source: ulid::Ulid::from(10), item: ulid::Ulid::from(11) };
        let prepared = PreparedImport::prepare(
            &mesimon_core::Principal::Local,
            TicketContent { title: "Incoming".into(), notes: vec!["Approved context".into()] },
            origin.clone(),
            ImportPlacement {
                id: ulid::Ulid::from(1),
                short_key: "T-1".into(),
                column: "TODO".into(),
                order: "a0".into(),
                created_at: "@0".into(),
            },
            || ulid::Ulid::from(2),
        )
        .unwrap();
        let file = TicketFile { schema_version: TICKET_SCHEMA, ticket: prepared.ticket };
        let text = toml::to_string_pretty(&file).unwrap();
        let back: TicketFile = toml::from_str(&text).unwrap();
        assert_eq!(back.ticket.import_origin, Some(origin.clone()));
        assert_eq!(back.ticket.notes.len(), 1);
        assert_eq!(back.ticket.effective_execution_policy(), ExecutionPolicy::OwnerOnly);
        assert!(matches!(verdict(back.schema_version, 5), Verdict::Newer(TICKET_SCHEMA)));
        // An incomplete local import must not turn into an automatic execution
        // merely because execution_policy was missing from its serialized form.
        let missing_policy = text.replace("execution_policy = \"owner_only\"\n", "");
        let partial: TicketFile = toml::from_str(&missing_policy).unwrap();
        assert!(partial.ticket.execution_policy.allows_automation());
        assert_eq!(partial.ticket.effective_execution_policy(), ExecutionPolicy::OwnerOnly);
        assert_eq!(partial.ticket.import_origin, Some(origin));
    }

    fn quarantined(dir: &Path, stem: &str) -> Vec<std::path::PathBuf> {
        std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                let n = p.file_name().unwrap().to_string_lossy().into_owned();
                n.starts_with(stem) && n.contains(".quarantine-")
            })
            .collect()
    }

    /// Today's columns.toml carries no stamp and must keep loading untouched
    /// (16 §6.2: "absent -> treat as 1"). This is the regression that matters
    /// most — every existing user has one of these.
    #[test]
    fn legacy_columns_without_schema_version_loads() {
        let (dir, paths) = scratch("legacycols");
        write(
            &dir.join(".mesimon/board/columns.toml"),
            "next_key = 4\n\n[[columns]]\nname = \"TODO\"\norder = \"a0\"\n",
        );
        let l = load(&paths).unwrap();
        assert!(l.notices.is_empty(), "{:?}", l.notices);
        assert_eq!(l.board.next_key, 4);
        assert_eq!(l.board.columns.len(), 1);
        assert!(!l.columns_write_barred);
        // Its lone TODO is seeded on the way to v4, and the rule pointing at
        // an IN PROGRESS this board does not have is pruned rather than
        // left to hit `no such column` on every edge.
        assert_eq!(l.board.columns[0].settings.on_working, None);
        assert!(std::fs::read_to_string(dir.join(".mesimon/board/columns.toml"))
            .unwrap()
            .contains(&format!("schema_version = {COLUMNS_SCHEMA}")));
        cleanup(&dir, &paths);
    }

    /// The v3 → v4 migration (T-117): the four template columns get the
    /// rules they carried as literals, a fifth gets nothing, the file is
    /// restamped with the rules INSIDE each `[[columns]]` table, and a second
    /// load changes nothing.
    #[test]
    fn v3_columns_file_migrates_to_v4_with_the_template_settings() {
        use mesimon_core::board::TrainReach;
        let (dir, paths) = scratch("migratecols");
        let cols = dir.join(".mesimon/board/columns.toml");
        write(
            &cols,
            "schema_version = 3\nnext_key = 7\nmcp_tools = false\n\n\
             [[columns]]\nname = \"TODO\"\norder = \"a\"\n\n\
             [[columns]]\nname = \"IN PROGRESS\"\norder = \"b\"\n\n\
             [[columns]]\nname = \"REVIEW\"\norder = \"c\"\n\n\
             [[columns]]\nname = \"DONE\"\norder = \"d\"\n\n\
             [[columns]]\nname = \"BACKLOG\"\norder = \"e\"\n",
        );
        let l = load(&paths).unwrap();
        assert!(l.notices.is_empty(), "{:?}", l.notices);
        let col = |n: &str| l.board.column(n).unwrap().settings.clone();
        assert_eq!(col("TODO").on_working.as_deref(), Some("IN PROGRESS"));
        assert_eq!(col("IN PROGRESS").on_done.as_deref(), Some("REVIEW"));
        assert_eq!(col("IN PROGRESS").train, TrainReach::Rebase);
        assert_eq!(col("REVIEW").on_working.as_deref(), Some("IN PROGRESS"));
        assert_eq!(col("REVIEW").train, TrainReach::Merge);
        assert!(col("DONE").requires_merge && col("DONE").reclaim);
        assert_eq!(col("BACKLOG"), Default::default());
        assert!(!l.board.mcp_tools, "the other scalars survive the migration");
        let text = std::fs::read_to_string(&cols).unwrap();
        assert!(text.contains(&format!("schema_version = {COLUMNS_SCHEMA}")), "{text}");
        let todo = text.find("name = \"TODO\"").unwrap();
        let next = text.find("name = \"IN PROGRESS\"").unwrap();
        let rule = text.find("on_working = \"IN PROGRESS\"").unwrap();
        assert!(todo < rule && rule < next, "the rule sits inside TODO's table:\n{text}");
        assert!(!text.contains("collapsed"), "a default is not written:\n{text}");
        // Idempotent: the stamp is what keeps a rule removed by hand removed.
        let again = load(&paths).unwrap();
        assert_eq!(again.board.columns, l.board.columns);
        let stripped = text.replace("on_working = \"IN PROGRESS\"\n", "");
        write(&cols, &stripped);
        let third = load(&paths).unwrap();
        assert_eq!(third.board.column("TODO").unwrap().settings.on_working, None);
        assert_eq!(third.board.column("REVIEW").unwrap().settings.on_working, None);
        cleanup(&dir, &paths);
    }

    /// A legacy sessions.json is a bare JSON array. The shape sniff must take
    /// it without a notice — reusing the M1 fixture shape verbatim.
    #[test]
    fn legacy_sessions_bare_array_loads() {
        let (dir, paths) = scratch("legacysess");
        write(
            &paths.sessions_file(),
            r#"[{"id":"3f2b8c1e-9a4d-4e6f-8b1a-2c3d4e5f6a7b","kind":"claude",
                "ticket":"01J8ZQ7VJ00000000000000000","argv":["claude"],"cwd":"/tmp",
                "state":{"state":"unknown"}}]"#,
        );
        let l = load(&paths).unwrap();
        assert!(l.notices.is_empty(), "{:?}", l.notices);
        assert_eq!(l.board.sessions.len(), 1);
        assert!(!l.sessions_write_barred);
        cleanup(&dir, &paths);
    }

    /// Save then load, for every file this module owns.
    #[test]
    fn versioned_roundtrip_reloads() {
        let (dir, paths) = scratch("roundtrip");
        let mut b = Board::with_default_columns();
        b.next_key = 9;
        save_columns(&paths, &b).unwrap();
        save_sessions(&paths, &b).unwrap();
        let l = load(&paths).unwrap();
        assert!(l.notices.is_empty(), "{:?}", l.notices);
        assert_eq!(l.board.next_key, 9);
        assert_eq!(l.board.columns.len(), b.columns.len());
        cleanup(&dir, &paths);
    }

    /// A truncated columns.toml must not stop the daemon: quarantine it, keep
    /// the bytes, come up on defaults, and say so with the parser's own span.
    #[test]
    fn columns_truncated_is_quarantined_not_fatal() {
        let (dir, paths) = scratch("truncated");
        let cols = dir.join(".mesimon/board/columns.toml");
        write(&cols, "next_key = 3\n[[columns]\nname = ");
        let l = load(&paths).unwrap();
        assert_eq!(l.notices.len(), 1, "{:?}", l.notices);
        assert_eq!(l.notices[0].kind, "quarantined");
        assert!(l.notices[0].detail.is_some(), "the parser's span must survive");
        assert!(!l.board.columns.is_empty(), "came up on defaults");
        assert_eq!(quarantined(&dir.join(".mesimon/board"), "columns.toml").len(), 1);
        cleanup(&dir, &paths);
    }

    /// A conflict marker is reported as a conflict, not as a syntax error
    /// pointing at whatever line the parser happened to choke on (16 §6.2).
    #[test]
    fn columns_conflict_markers_say_conflict() {
        let (dir, paths) = scratch("conflict");
        write(
            &dir.join(".mesimon/board/columns.toml"),
            "next_key = 3\n<<<<<<< HEAD\n[[columns]]\nname = \"A\"\norder = \"a0\"\n",
        );
        let l = load(&paths).unwrap();
        assert_eq!(l.notices.len(), 1);
        let d = l.notices[0].detail.clone().unwrap();
        assert!(d.contains("merge conflict"), "{d}");
        cleanup(&dir, &paths);
    }

    /// THE data-loss regression: losing columns.toml loses next_key, and
    /// mint_ticket derives short_key from it — without recovery the next
    /// ticket mints T-1 and save_ticket overwrites the existing T-7.
    #[test]
    fn columns_quarantine_recovers_next_key() {
        let (dir, paths) = scratch("nextkey");
        write(&dir.join(".mesimon/board/columns.toml"), "not = [valid");
        write(&dir.join(".mesimon/board/tickets/T-7/ticket.toml"), &ticket_body("T-7", "TODO"));
        // A directory we cannot parse still owns its key.
        write(&dir.join(".mesimon/board/tickets/T-12/ticket.toml"), "broken = [");
        let l = load(&paths).unwrap();
        assert_eq!(l.board.next_key, 12, "must clear every key on disk");
        cleanup(&dir, &paths);
    }

    /// A ticket in a column that only the lost file knew about must stay
    /// visible — v0.1 has no Unfiled column to park it in.
    #[test]
    fn columns_quarantine_keeps_referenced_columns() {
        let (dir, paths) = scratch("readd");
        write(&dir.join(".mesimon/board/columns.toml"), "not = [valid");
        write(&dir.join(".mesimon/board/tickets/T-2/ticket.toml"), &ticket_body("T-2", "SHIPPED"));
        let l = load(&paths).unwrap();
        assert!(
            l.board.columns.iter().any(|c| c.name == "SHIPPED"),
            "columns: {:?}",
            l.board.columns.iter().map(|c| &c.name).collect::<Vec<_>>()
        );
        assert_eq!(l.board.tickets.len(), 1);
        cleanup(&dir, &paths);
    }

    /// One bad ticket excludes exactly itself (13 §13.10.3) — the others load
    /// and are never touched.
    #[test]
    fn one_bad_ticket_excludes_only_itself() {
        let (dir, paths) = scratch("oneticket");
        let base = dir.join(".mesimon/board/tickets");
        write(&base.join("T-1/ticket.toml"), &ticket_body("T-1", "TODO"));
        write(&base.join("T-2/ticket.toml"), "id = [broken");
        write(&base.join("T-3/ticket.toml"), &ticket_body("T-3", "TODO"));
        let l = load(&paths).unwrap();
        assert_eq!(l.board.tickets.len(), 2);
        assert_eq!(l.notices.iter().filter(|n| n.kind == "quarantined").count(), 1);
        assert!(base.join("T-1/ticket.toml").is_file());
        assert!(base.join("T-3/ticket.toml").is_file());
        assert_eq!(quarantined(&base.join("T-2"), "ticket.toml").len(), 1);
        cleanup(&dir, &paths);
    }

    /// A malformed sessions.json costs the session records, not the daemon.
    #[test]
    fn sessions_truncated_is_quarantined() {
        let (dir, paths) = scratch("sesstrunc");
        write(&paths.sessions_file(), r#"[{"id":"3f2b8c1e-9a4d-4e"#);
        let l = load(&paths).unwrap();
        assert!(l.board.sessions.is_empty());
        assert_eq!(l.notices.len(), 1);
        assert_eq!(l.notices[0].kind, "quarantined");
        assert!(!l.sessions_write_barred, "the path is free again");
        assert_eq!(quarantined(&paths.state_dir, "sessions.json").len(), 1);
        cleanup(&dir, &paths);
    }

    /// A file a NEWER mesimon wrote is valid — refuse it, do not move it, and
    /// bar writes so this build cannot silently downgrade it (16 §6.2).
    #[test]
    fn future_version_is_refused_not_quarantined() {
        let (dir, paths) = scratch("future");
        let cols = dir.join(".mesimon/board/columns.toml");
        write(&cols, "schema_version = 99\nnext_key = 4\n");
        write(&paths.sessions_file(), r#"{"schema_version":99,"sessions":[]}"#);
        let l = load(&paths).unwrap();
        assert!(cols.is_file(), "the bytes must stay where they are");
        assert!(paths.sessions_file().is_file());
        assert!(quarantined(&dir.join(".mesimon/board"), "columns.toml").is_empty());
        assert!(l.columns_write_barred);
        assert!(l.sessions_write_barred);
        assert_eq!(l.notices.iter().filter(|n| n.kind == "future_version").count(), 2);
        assert!(l.notices[0].text.contains("99"), "{}", l.notices[0].text);
        cleanup(&dir, &paths);
    }

    #[test]
    fn provider_switch_roundtrips_without_changing_existing_session_providers() {
        use mesimon_core::board::{SessionKind, SessionState};
        let (dir, paths) = scratch("providers");
        let mut board = load_with(&paths, false).unwrap().board;
        let mut claude = SessionRecord::new(
            uuid::Uuid::new_v4(),
            SessionKind::Claude,
            ulid::Ulid::new(),
            vec!["claude".into()],
            dir.display().to_string(),
            SessionState::Sleeping,
        );
        claude.claude_session_id = Some(uuid::Uuid::new_v4());
        board.sessions.push(claude);
        board.agent_provider = AgentProvider::Codex;
        let mut codex = SessionRecord::new(
            uuid::Uuid::new_v4(),
            SessionKind::Codex,
            ulid::Ulid::new(),
            vec!["codex".into()],
            dir.display().to_string(),
            SessionState::Sleeping,
        );
        codex.codex_thread_id = Some("opaque-thread:exact-resume".into());
        codex.codex_turn_id = Some("opaque-turn:already-observed".into());
        codex.codex_generation = Some(17);
        codex.codex_observed_seq = 23;
        codex.agent_preview_path = Some("/owned/agent-preview.json".into());
        codex.observation_hold = false;
        board.sessions.push(codex);
        for provider in [AgentProvider::Codex, AgentProvider::ClaudeCode] {
            board.agent_provider = provider;
            save_columns(&paths, &board).unwrap();
            save_sessions(&paths, &board).unwrap();
            let loaded = load_with(&paths, false).unwrap();
            assert!(loaded.notices.is_empty(), "{:?}", loaded.notices);
            assert_eq!(loaded.board.agent_provider, provider);
            assert_eq!(loaded.board.sessions[0].kind, SessionKind::Claude);
            assert_eq!(loaded.board.sessions[1].kind, SessionKind::Codex);
            assert_eq!(
                loaded.board.sessions[0].claude_session_id,
                board.sessions[0].claude_session_id
            );
            assert_eq!(
                loaded.board.sessions[1].codex_thread_id.as_deref(),
                Some("opaque-thread:exact-resume")
            );
            assert!(!loaded.board.sessions[1].observation_hold);
            assert_eq!(loaded.board.sessions[1].codex_generation, Some(17));
            assert_eq!(loaded.board.sessions[1].codex_observed_seq, 23);
            assert_eq!(
                loaded.board.sessions[1].codex_turn_id.as_deref(),
                Some("opaque-turn:already-observed")
            );
            assert_eq!(
                loaded.board.sessions[1].agent_preview_path.as_deref(),
                Some("/owned/agent-preview.json")
            );
            assert!(loaded.board.sessions.iter().all(|s| s.state == SessionState::Sleeping));
        }
        // These stamps must trigger the refusal path in the preceding build,
        // before it attempts to decode the new session kind or loses selection.
        let columns: toml::Value = toml::from_str(
            &std::fs::read_to_string(paths.board_dir.join("board/columns.toml")).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            verdict(columns["schema_version"].as_integer().unwrap() as u32, 4),
            Verdict::Newer(COLUMNS_SCHEMA)
        ));
        let sessions: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(paths.sessions_file()).unwrap()).unwrap();
        assert!(matches!(
            verdict(sessions["schema_version"].as_u64().unwrap() as u32, 1),
            Verdict::Newer(2)
        ));
        cleanup(&dir, &paths);
    }

    #[test]
    fn legacy_provider_files_keep_claude_and_gain_stamps_only_when_saved() {
        use mesimon_core::board::{SessionKind, SessionState};
        let (dir, paths) = scratch("legacyprovider");
        let columns = paths.board_dir.join("board/columns.toml");
        let old_columns = "schema_version = 4\nnext_key = 1\ncolumns = []\n";
        write(&columns, old_columns);
        let rec = SessionRecord::new(
            uuid::Uuid::new_v4(),
            SessionKind::Claude,
            ulid::Ulid::new(),
            Vec::new(),
            dir.display().to_string(),
            SessionState::Sleeping,
        );
        let mut old_record = serde_json::to_value(rec).unwrap();
        let object = old_record.as_object_mut().unwrap();
        object.remove("codex_thread_id");
        object.remove("observation_hold");
        object.remove("codex_generation");
        object.remove("codex_observed_seq");
        object.remove("codex_turn_id");
        object.remove("agent_preview_path");
        object.remove("pending_prefill");
        object.remove("codex_submit_sent");
        for old_sessions in [
            serde_json::json!([old_record.clone()]),
            serde_json::json!({"schema_version": 1, "sessions": [old_record]}),
        ] {
            let old_text = old_sessions.to_string();
            write(&paths.sessions_file(), &old_text);
            let loaded = load_with(&paths, false).unwrap();
            assert!(loaded.notices.is_empty(), "{:?}", loaded.notices);
            assert_eq!(loaded.board.agent_provider, AgentProvider::ClaudeCode);
            assert_eq!(loaded.board.sessions[0].kind.provider(), Some(AgentProvider::ClaudeCode));
            assert_eq!(std::fs::read_to_string(&columns).unwrap(), old_columns);
            assert_eq!(std::fs::read_to_string(paths.sessions_file()).unwrap(), old_text);
        }
        cleanup(&dir, &paths);
    }

    #[test]
    fn future_provider_files_preserve_bytes_and_bar_writes() {
        let (dir, paths) = scratch("futureprovider");
        let columns = paths.board_dir.join("board/columns.toml");
        let column_text = format!(
            "schema_version = {}\nnext_key = 1\nagent_provider = \"future_agent\"\ncolumns = []\n",
            COLUMNS_SCHEMA + 1
        );
        let session_text = format!(
            "{{\"schema_version\":{},\"sessions\":[{{\"kind\":\"future_agent\"}}]}}",
            SESSIONS_SCHEMA + 1
        );
        write(&columns, &column_text);
        write(&paths.sessions_file(), &session_text);
        let loaded = load_with(&paths, false).unwrap();
        assert!(loaded.columns_write_barred && loaded.sessions_write_barred);
        assert_eq!(loaded.notices.iter().filter(|n| n.kind == "future_version").count(), 2);
        assert_eq!(std::fs::read_to_string(&columns).unwrap(), column_text);
        assert_eq!(std::fs::read_to_string(paths.sessions_file()).unwrap(), session_text);
        assert!(quarantined(&paths.state_dir, "sessions.json").is_empty());
        cleanup(&dir, &paths);
    }

    /// Loading twice must not quarantine twice or re-notice: the bad file is
    /// gone after the first pass, and nothing reads a quarantined name.
    #[test]
    fn quarantined_files_are_not_reloaded() {
        let (dir, paths) = scratch("twice");
        write(&dir.join(".mesimon/board/columns.toml"), "not = [valid");
        let first = load(&paths).unwrap();
        assert_eq!(first.notices.len(), 1);
        let second = load(&paths).unwrap();
        assert!(second.notices.is_empty(), "{:?}", second.notices);
        assert_eq!(quarantined(&dir.join(".mesimon/board"), "columns.toml").len(), 1);
        cleanup(&dir, &paths);
    }

    /// Repeated failures must never collide on one name.
    #[test]
    fn quarantine_names_do_not_collide() {
        let (dir, paths) = scratch("collide");
        let f = dir.join(".mesimon/board/columns.toml");
        write(&f, "a");
        let one = quarantine(&f).unwrap();
        write(&f, "b");
        let two = quarantine(&f).unwrap();
        assert_ne!(one, two, "same-millisecond quarantines must not share a name");
        assert_eq!(std::fs::read_to_string(&one).unwrap(), "a", "first file's bytes survive");
        assert_eq!(std::fs::read_to_string(&two).unwrap(), "b");
        cleanup(&dir, &paths);
    }

    /// A pre-M4 ticket.toml (the 6 original keys, no `workspace`) must keep
    /// parsing — load() hard-fails on parse errors, so defaults ARE the migration.
    #[test]
    fn m3_ticket_toml_parses() {
        let m3 = r#"
id = "01J8ZQ7VJ00000000000000000"
short_key = "T-3"
title = "old ticket"
column = "TODO"
order = "a0"
created_at = "@1788046350"
"#;
        let t: Ticket = toml::from_str(m3).unwrap();
        assert!(t.workspace.is_none());
        assert!(t.archived.is_none());
        assert_eq!(t.workspace_strategy(), mesimon_core::board::WorkspaceStrategy::SharedCheckout);
    }

    /// A pre-tags `ticket.toml` still parses: the `#[serde(default)]` IS the
    /// migration. The stakes are no longer a dead daemon — a missing default
    /// now quarantines the user's file — which is why this fixture exists for
    /// every field added to `Ticket`.
    #[test]
    fn pre_tags_ticket_toml_parses() {
        let m4 = r#"
id = "01J8ZQ7VJ00000000000000000"
short_key = "T-4"
title = "untagged"
column = "TODO"
order = "a0"
created_at = "@1788046350"
workspace = "worktree"
"#;
        let t: Ticket = toml::from_str(m4).unwrap();
        assert!(t.tags.is_empty());
        assert!(t.archived.is_none());
        assert!(t.notes.is_empty());
    }

    /// A pre-notes ticket (v1 with tags) still parses, and a note's own
    /// optional fields default: `name`/`rev`/`*_by` were all born together,
    /// but the fixture is cheap and the failure mode is a quarantined file.
    #[test]
    fn pre_notes_ticket_toml_parses() {
        let alpha8 = r#"
schema_version = 1
id = "01J8ZQ7VJ00000000000000000"
short_key = "T-4"
title = "tagged"
column = "TODO"
order = "a0"
created_at = "@1788046350"

[[tags]]
name = "BUG"
group = 1
"#;
        let f: TicketFile = toml::from_str(alpha8).unwrap();
        assert_eq!(f.ticket.tags.len(), 1);
        assert!(f.ticket.notes.is_empty());
        let sparse = r#"
id = "01J8ZQ7VJ00000000000000000"
short_key = "T-4"
title = "t"
column = "TODO"
order = "a0"
created_at = "@1"

[[notes]]
id = "01J8ZQ7VJ00000000000000001"
created_at = "@2"
"#;
        let t: Ticket = toml::from_str(sparse).unwrap();
        assert_eq!(t.previous_column, None, "old files have no remembered stay");
        assert_eq!(t.notes.len(), 1);
        assert_eq!(t.notes[0].rev, 0);
        assert_eq!(t.notes[0].name, "");
    }

    /// A v2 ticket — a plain `[archived]` with no deadline — still parses,
    /// and the snooze fields default to "not a snooze, not woke". The other
    /// direction: a snoozed ticket round-trips through the stamped wrapper
    /// with `woke_at` among the scalars and the deadline inside the table.
    #[test]
    fn pre_snooze_ticket_toml_parses() {
        let alpha12 = r#"
schema_version = 2
id = "01J8ZQ7VJ00000000000000000"
short_key = "T-4"
title = "parked"
column = "TODO"
order = "a0"
created_at = "@1788046350"
entered_at = "@1788046360"

[[tags]]
name = "BUG"
group = 1

[archived]
at = "@1788046400"
by = "local"
"#;
        let f: TicketFile = toml::from_str(alpha12).unwrap();
        assert_eq!(f.schema_version, 2);
        let a = f.ticket.archived.as_ref().expect("archived");
        assert_eq!((a.until.as_deref(), a.needs_you), (None, false));
        assert!(!f.ticket.is_woke());
        assert_eq!(f.ticket.snooze_until_secs(), None);

        let mut t = f.ticket.clone();
        t.woke_at = Some("@1788046500".into());
        t.archived = Some(mesimon_core::board::Archived {
            at: "@1788046400".into(),
            by: "local".into(),
            until: Some("@1788050000".into()),
            needs_you: true,
        });
        let f = TicketFile { schema_version: TICKET_SCHEMA, ticket: t.clone() };
        let s = toml::to_string_pretty(&f).unwrap();
        assert!(s.find("woke_at").unwrap() < s.find("[[tags]]").unwrap(), "{s}");
        assert!(s.find("[archived]").unwrap() < s.find("until").unwrap(), "{s}");
        let back: TicketFile = toml::from_str(&s).unwrap();
        assert_eq!(back.ticket.archived, t.archived);
        assert_eq!(back.ticket.snooze_until_secs(), Some(1788050000));
        assert!(back.ticket.is_woke());
        // Not a snooze: neither key is written, so the file reads as it did.
        let plain = TicketFile { schema_version: TICKET_SCHEMA, ticket: f.ticket.clone() };
        let mut plain = plain;
        plain.ticket.woke_at = None;
        plain.ticket.archived = Some(mesimon_core::board::Archived {
            at: "@1".into(),
            by: "local".into(),
            until: None,
            needs_you: false,
        });
        let s = toml::to_string_pretty(&plain).unwrap();
        assert!(!s.contains("until") && !s.contains("needs_you") && !s.contains("woke_at"), "{s}");
    }

    /// `[raised]` is a TABLE (T-107), so it belongs with the tables: after
    /// every scalar and before `[[tags]]`. A scalar serialized after a table
    /// errors outright, which is what makes the serializer the judge here.
    #[test]
    fn a_raised_hand_serializes_with_the_tables() {
        let t = Ticket {
            id: ulid::Ulid(11),
            short_key: "T-11".into(),
            title: "asked".into(),
            column: "REVIEW".into(),
            order: "a0".into(),
            created_at: "@0".into(),
            created_by: String::new(),
            created_from: None,
            entered_at: None,
            previous_column: Some(mesimon_core::board::ColumnStay {
                column: "IN PROGRESS".into(),
                seconds: 3661,
            }),
            woke_at: None,
            manual_merge: false,
            execution_policy: Default::default(),
            tier: None,
            import_origin: None,
            raised: Some(mesimon_core::board::Raised {
                at: "@1788046500".into(),
                by: "agent:00000000-0000-0000-0000-000000000000".into(),
                reason: "which auth provider?".into(),
            }),
            // A worktree strategy is a scalar and sits before it; a tag is a
            // table and sits after. Both present, so the order is real.
            workspace: Some(mesimon_core::board::WorkspaceStrategy::SharedCheckout),
            tags: vec![mesimon_core::board::TagRef { name: "BUG".into(), group: 1 }],
            notes: Vec::new(),
            archived: None,
        };
        let f = TicketFile { schema_version: TICKET_SCHEMA, ticket: t.clone() };
        let s = toml::to_string_pretty(&f).expect("a scalar after a table would error here");
        assert!(s.find("[raised]").unwrap() < s.find("[[tags]]").unwrap(), "{s}");
        let back: TicketFile = toml::from_str(&s).unwrap();
        assert_eq!(back.ticket.previous_column, t.previous_column);
        assert_eq!(back.ticket.raised, t.raised);
        assert!(back.ticket.hand_raised());
        // No hand, no key — an older build's file and this one agree.
        let mut plain = f;
        plain.ticket.raised = None;
        let s = toml::to_string_pretty(&plain).unwrap();
        assert!(!s.contains("raised"), "{s}");
    }

    /// `[[notes]]` is another array of tables: after `[[tags]]`, before
    /// `[archived]`. The serializer that writes the file is the judge.
    #[test]
    fn notes_serialize_after_tags_and_before_archived() {
        let mut t = Ticket {
            id: ulid::Ulid(9),
            short_key: "T-9".into(),
            title: "noted".into(),
            column: "DONE".into(),
            order: "a0".into(),
            created_at: "@0".into(),
            created_by: String::new(),
            created_from: None,
            entered_at: None,
            previous_column: None,
            woke_at: None,
            manual_merge: false,
            execution_policy: Default::default(),
            tier: None,
            import_origin: None,
            raised: None,
            workspace: None,
            tags: Vec::new(),
            notes: vec![mesimon_core::board::NoteMeta {
                id: ulid::Ulid(10),
                name: "Why".into(),
                rev: 3,
                created_at: "@1".into(),
                created_by: "local".into(),
                edited_at: "@2".into(),
                edited_by: "agent:00000000-0000-0000-0000-000000000000".into(),
            }],
            archived: Some(mesimon_core::board::Archived {
                at: "@3".into(),
                by: "local".into(),
                until: None,
                needs_you: false,
            }),
        };
        t.set_tag(1, Some("BUG".into()));
        let f = TicketFile { schema_version: TICKET_SCHEMA, ticket: t.clone() };
        let s = toml::to_string_pretty(&f).unwrap();
        let back: TicketFile = toml::from_str(&s).unwrap();
        assert_eq!(back.ticket.notes, t.notes);
        assert_eq!(back.ticket.tags, t.tags);
        assert_eq!(back.ticket.archived, t.archived);
        assert!(s.find("[[tags]]").unwrap() < s.find("[[notes]]").unwrap());
        assert!(s.find("[[notes]]").unwrap() < s.find("[archived]").unwrap());
    }

    /// Tags AND archived together, round-tripped through the serializer that
    /// actually writes the file. `[[tags]]` is an array of tables and
    /// `[archived]` is a table: tables may follow tables, but a scalar after
    /// either errors — so this is the test that catches `tags` being declared
    /// in the wrong place in the struct.
    #[test]
    fn tags_serialize_before_the_archived_table() {
        let mut t = Ticket {
            id: ulid::Ulid(9),
            short_key: "T-9".into(),
            title: "tagged".into(),
            column: "DONE".into(),
            order: "a0".into(),
            created_at: "@0".into(),
            created_by: String::new(),
            created_from: None,
            entered_at: None,
            previous_column: None,
            woke_at: None,
            manual_merge: false,
            execution_policy: Default::default(),
            tier: None,
            import_origin: None,
            raised: None,
            workspace: Some(mesimon_core::board::WorkspaceStrategy::Worktree),
            tags: Vec::new(),
            notes: Vec::new(),
            archived: Some(mesimon_core::board::Archived {
                at: "@1788046350".into(),
                by: "local".into(),
                until: None,
                needs_you: false,
            }),
        };
        t.set_tag(1, Some("BUG".into()));
        t.set_tag(2, Some("STAGING".into()));

        let s = toml::to_string_pretty(&t).unwrap();
        let back: Ticket = toml::from_str(&s).unwrap();
        assert_eq!(back.tags, t.tags);
        assert_eq!(back.archived, t.archived);
        // And the stamped wrapper the daemon actually writes.
        let f = TicketFile { schema_version: TICKET_SCHEMA, ticket: t.clone() };
        let s = toml::to_string_pretty(&f).unwrap();
        let back: TicketFile = toml::from_str(&s).unwrap();
        assert_eq!(back.ticket.tags, t.tags);
        assert_eq!(back.ticket.archived, t.archived);
    }

    /// A ticket with BOTH optional fields round-trips — `[archived]` is a
    /// table, so it must serialize last or to_string_pretty errors. This is
    /// the test that catches wrong struct field order.
    #[test]
    fn archived_table_roundtrips() {
        let t = Ticket {
            id: ulid::Ulid(8),
            short_key: "T-8".into(),
            title: "arch".into(),
            column: "DONE".into(),
            order: "a0".into(),
            created_at: "@0".into(),
            created_by: String::new(),
            created_from: None,
            entered_at: None,
            previous_column: None,
            woke_at: None,
            manual_merge: false,
            execution_policy: Default::default(),
            tier: None,
            import_origin: None,
            raised: None,
            workspace: Some(mesimon_core::board::WorkspaceStrategy::Worktree),
            tags: Vec::new(),
            notes: Vec::new(),
            archived: Some(mesimon_core::board::Archived {
                at: "@1788046350".into(),
                by: "local".into(),
                until: None,
                needs_you: false,
            }),
        };
        let s = toml::to_string_pretty(&t).unwrap();
        let back: Ticket = toml::from_str(&s).unwrap();
        assert_eq!(back.archived, t.archived);
        assert_eq!(back.column, "DONE");
    }

    /// A hand-written ticket.toml with the trailing `[archived]` table parses.
    #[test]
    fn archived_toml_parses() {
        let m5 = r#"
id = "01J8ZQ7VJ00000000000000000"
short_key = "T-9"
title = "archived ticket"
column = "REVIEW"
order = "a0"
created_at = "@1788046350"

[archived]
at = "@1788050000"
by = "local"
"#;
        let t: Ticket = toml::from_str(m5).unwrap();
        assert!(t.is_archived());
        assert_eq!(t.column, "REVIEW");
    }

    /// THE GATE on the `TicketFile { schema_version, #[serde(flatten)] ticket }`
    /// shape: `#[serde(flatten)]` serializes through a map, and toml errors on
    /// any scalar emitted after a table. If this fails, the stamp has to move
    /// onto `Ticket` itself as its first field.
    #[test]
    fn ticket_file_schema_precedes_archived_table() {
        let tf = TicketFile {
            schema_version: TICKET_SCHEMA,
            ticket: Ticket {
                id: ulid::Ulid(11),
                short_key: "T-11".into(),
                title: "stamped".into(),
                column: "DONE".into(),
                order: "a0".into(),
                created_at: "@0".into(),
                created_by: String::new(),
                created_from: None,
                entered_at: None,
                previous_column: None,
                woke_at: None,
                manual_merge: false,
                execution_policy: Default::default(),
                tier: None,
                import_origin: None,
                raised: None,
                workspace: Some(mesimon_core::board::WorkspaceStrategy::Worktree),
                tags: Vec::new(),
                notes: Vec::new(),
                archived: Some(mesimon_core::board::Archived {
                    at: "@1788050000".into(),
                    by: "local".into(),
                    until: None,
                    needs_you: false,
                }),
            },
        };
        let s = toml::to_string_pretty(&tf).unwrap();
        assert!(
            s.find("schema_version").unwrap() < s.find("[archived]").unwrap(),
            "stamp must precede the table:\n{s}"
        );
        let back: TicketFile = toml::from_str(&s).unwrap();
        assert_eq!(back.schema_version, TICKET_SCHEMA);
        assert_eq!(back.ticket.archived, tf.ticket.archived);
        assert_eq!(back.ticket.short_key, "T-11");
    }

    /// A v1 columns.toml (no registry) still loads, and the tag list defaults
    /// to empty at the file level — the starter offer is `load`'s, and it is
    /// what `tags_seeded` absent means: the offer is still owed.
    #[test]
    fn v1_columns_file_loads_without_a_registry() {
        let v1 = r#"
schema_version = 1
next_key = 7

[[columns]]
name = "TODO"
order = "a0"
"#;
        let cf: ColumnsFile = toml::from_str(v1).unwrap();
        assert_eq!(cf.schema_version, 1);
        assert_eq!(cf.next_key, 7);
        assert!(cf.tags.is_empty());
        assert!(!cf.tags_seeded);
    }

    fn starter_names(b: &Board) -> Vec<&str> {
        b.group_tags(mesimon_core::board::STARTER_GROUP)
    }

    /// A fresh board opens with the three starters on group 1, each with its
    /// hand-picked colour, and the file says the offer was made.
    #[test]
    fn a_fresh_board_gets_the_starter_tags_once() {
        let (dir, paths) = scratch("seedfresh");
        let l = load(&paths).unwrap();
        assert!(l.notices.is_empty(), "{:?}", l.notices);
        assert_eq!(starter_names(&l.board), vec!["BUG", "FEATURE", "CHANGE"]);
        for (name, color) in mesimon_core::board::STARTER_TAGS {
            let def = l.board.tag_def(mesimon_core::board::STARTER_GROUP, name).unwrap();
            assert_eq!(def.color, Some(color), "{name} carries a chosen colour, not a hash");
        }
        assert!(l.board.tags_seeded);
        let text = std::fs::read_to_string(dir.join(".mesimon/board/columns.toml")).unwrap();
        assert!(text.contains("tags_seeded = true"), "{text}");
        assert_eq!(text.matches("[[tags]]").count(), 3);

        // Forgetting all three is respected: the next load seeds nothing.
        let mut b = l.board.clone();
        b.tags.clear();
        save_columns(&paths, &b).unwrap();
        let again = load(&paths).unwrap();
        assert!(again.board.tags.is_empty(), "the starters must not come back");
        assert!(again.board.tags_seeded);
        cleanup(&dir, &paths);
    }

    /// A board written before the stamp existed: with a vocabulary of its own
    /// it is left alone (only stamped); with none it gets the offer.
    #[test]
    fn an_existing_board_is_offered_the_starters_only_when_it_has_no_tags() {
        let (dir, paths) = scratch("seedexisting");
        let cols = dir.join(".mesimon/board/columns.toml");
        write(
            &cols,
            r#"schema_version = 2
next_key = 4

[[columns]]
name = "TODO"
order = "a0"

[[tags]]
name = "OWN"
group = 1
"#,
        );
        let l = load(&paths).unwrap();
        assert_eq!(starter_names(&l.board), vec!["OWN"], "a vocabulary is never added to");
        assert!(l.board.tags_seeded);
        assert!(std::fs::read_to_string(&cols).unwrap().contains("tags_seeded = true"));

        write(
            &cols,
            r#"schema_version = 2
next_key = 4

[[columns]]
name = "TODO"
order = "a0"
"#,
        );
        let l = load(&paths).unwrap();
        assert_eq!(starter_names(&l.board), vec!["BUG", "FEATURE", "CHANGE"]);
        assert_eq!(l.board.next_key, 4, "the rest of the file is untouched");
        cleanup(&dir, &paths);
    }

    /// A file written before T-217 has neither switch. The agent tools must
    /// read as ON — the shipped behaviour — and the CLAUDE.md offer as
    /// unanswered. This is the whole reason `mcp_tools` names a serde default
    /// instead of riding `bool`'s.
    #[test]
    fn a_file_without_the_switches_reads_as_tools_on_and_never_ignored() {
        let (dir, paths) = scratch("t217compat");
        let cols = dir.join(".mesimon/board/columns.toml");
        write(
            &cols,
            r#"schema_version = 2
next_key = 4
tags_seeded = true

[[columns]]
name = "TODO"
order = "a0"
"#,
        );
        let l = load(&paths).unwrap();
        assert!(l.board.mcp_tools, "an absent switch is not a switch turned off");
        assert!(!l.board.claude_md_ignored);
        assert!(!l.board.system_prompt, "and an absent brief is a brief nobody turned on");
        assert!(!l.columns_write_barred, "a v2 file is still ours to write");
        cleanup(&dir, &paths);
    }

    /// And an unreadable file falls back to defaults, which must ALSO read as
    /// tools on: `Board`'s hand-written `Default` is what carries that, and a
    /// derived one would have made a corrupt file look like a user's choice.
    #[test]
    fn the_default_board_has_the_tools_on() {
        assert!(Board::default().mcp_tools);
        assert!(Board::with_default_columns().mcp_tools);
        assert!(!Board::default().claude_md_ignored);
        assert!(!Board::default().system_prompt, "the brief is opt-in");
    }

    /// The stamp survives the round trip, so "never ask again" is never asked
    /// again after a restart.
    #[test]
    fn the_switches_round_trip_through_the_file() {
        let (dir, paths) = scratch("t217trip");
        let mut l = load(&paths).unwrap();
        l.board.mcp_tools = false;
        l.board.claude_md_ignored = true;
        l.board.system_prompt = true;
        save_columns(&paths, &l.board).unwrap();
        let back = load(&paths).unwrap();
        assert!(!back.board.mcp_tools);
        assert!(back.board.claude_md_ignored);
        assert!(back.board.system_prompt, "the brief's consent survives a restart");
        cleanup(&dir, &paths);
    }

    /// The default column (T-279) rides the same file: absent until chosen,
    /// round-tripped once it is, and doctor's reader agrees with the board's
    /// `landing_column` — a name the file's columns no longer list is nobody's
    /// default.
    #[test]
    fn the_default_column_round_trips_and_is_absent_until_chosen() {
        let (dir, paths) = scratch("t279trip");
        let mut l = load(&paths).unwrap();
        save_columns(&paths, &l.board).unwrap();
        let text = std::fs::read_to_string(paths.board_dir.join("board/columns.toml")).unwrap();
        assert!(!text.contains("default_column"), "unchosen is unwritten:\n{text}");
        assert_eq!(read_columns_scalars(&paths).default_column, None);
        l.board.set_default_column(Some("REVIEW")).unwrap();
        save_columns(&paths, &l.board).unwrap();
        let back = load(&paths).unwrap();
        assert_eq!(back.board.default_column.as_deref(), Some("REVIEW"));
        assert_eq!(back.board.landing_column().as_deref(), Some("REVIEW"));
        assert_eq!(read_columns_scalars(&paths).default_column.as_deref(), Some("REVIEW"));
        // Written by hand to a column the board lacks: the board lands on the
        // first column and doctor says the same.
        let text = std::fs::read_to_string(paths.board_dir.join("board/columns.toml")).unwrap();
        std::fs::write(
            paths.board_dir.join("board/columns.toml"),
            text.replace("default_column = \"REVIEW\"", "default_column = \"GONE\""),
        )
        .unwrap();
        let back = load(&paths).unwrap();
        assert_eq!(back.board.landing_column().as_deref(), Some("TODO"));
        assert_eq!(read_columns_scalars(&paths).default_column, None);
        cleanup(&dir, &paths);
    }

    /// The seam the vocabulary e2es use: no offer, no stamp, no write.
    #[test]
    fn the_seed_can_be_declined_for_a_test() {
        let (dir, paths) = scratch("seedoff");
        let l = load_with(&paths, false).unwrap();
        assert!(l.board.tags.is_empty());
        assert!(!l.board.tags_seeded);
        cleanup(&dir, &paths);
    }

    #[test]
    fn inactivity_timeout_defaults_off_and_survives_reload() {
        let (dir, paths) = scratch("inactivity");
        let mut board = load_with(&paths, false).unwrap().board;
        assert_eq!(board.park_after_minutes, 0);
        for minutes in [17, u32::MAX, 0] {
            board.park_after_minutes = minutes;
            save_columns(&paths, &board).unwrap();
            assert_eq!(load_with(&paths, false).unwrap().board.park_after_minutes, minutes);
        }
        let path = paths.board_dir.join("board/columns.toml");
        let text = std::fs::read_to_string(&path).unwrap();
        let legacy = text
            .lines()
            .filter(|l| !l.starts_with("park_after_minutes"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(path, legacy).unwrap();
        assert_eq!(load_with(&paths, false).unwrap().board.park_after_minutes, 0);
        cleanup(&dir, &paths);
    }

    /// The registry round-trips through the serializer that writes the file.
    /// `[[columns]]` and `[[tags]]` are both arrays of tables, so they may
    /// follow each other — but a scalar after either is a TOML error, which
    /// is why `schema_version` and `next_key` are declared first.
    #[test]
    fn registry_roundtrips_after_the_columns_table() {
        let cf = ColumnsFile {
            schema_version: COLUMNS_SCHEMA,
            next_key: 3,
            agent_provider: AgentProvider::Codex,
            park_after_minutes: 30,
            crown_budget: 5,
            tags_seeded: true,
            mcp_tools: false,
            claude_md_ignored: true,
            system_prompt: true,
            default_column: Some("TODO".into()),
            follow_up_mode: mesimon_core::board::FollowUpMode::Steer,
            prompt_rebase: Some("catch {branch} up to {base}".into()),
            prompt_merged: None,
            prompt_note_updated: None,
            prompt_crown_wake: None,
            crown: Some(ulid::Ulid(7)),
            default_tier: Some("01TIER".into()),
            columns: vec![Column {
                name: "TODO".into(),
                order: "a0".into(),
                settings: mesimon_core::board::ColumnSettings {
                    description: Some("planned for this version".into()),
                    collapsed: true,
                    workspace: Some(mesimon_core::board::WorkspaceStrategy::Worktree),
                    claude_mode: mesimon_core::board::ClaudeMode::Plan,
                    codex_sandbox: mesimon_core::board::CodexSandbox::Inherit,
                    codex_approval: mesimon_core::board::CodexApproval::Inherit,
                    agent_tools: mesimon_core::board::AgentTools::Read,
                    auto_run: true,
                    on_working: Some("QA".into()),
                    on_done: None,
                    requires_merge: true,
                    reclaim: true,
                    offers: Some(mesimon_core::board::ColumnOffers::Sleep),
                    train: mesimon_core::board::TrainReach::Merge,
                },
            }],
            tags: vec![
                mesimon_core::board::Tag { name: "BUG".into(), group: 1, color: None },
                mesimon_core::board::Tag { name: "STAGING".into(), group: 2, color: Some(4) },
            ],
            tiers: vec![mesimon_core::tier::Tier {
                id: "01TIER".into(),
                name: "coder".into(),
                provider: AgentProvider::ClaudeCode,
                model: "opus".into(),
                effort: mesimon_core::tier::Effort::Xhigh,
            }],
        };
        let text = toml::to_string_pretty(&cf).unwrap();
        let back: ColumnsFile = toml::from_str(&text).unwrap();
        // T-117: the flattened settings round-trip inside the column's own
        // table, and a default (`on_done`) is not written.
        assert_eq!(back.columns, cf.columns);
        assert!(text.contains("claude_mode = \"plan\""), "{text}");
        assert!(text.contains("agent_tools = \"read\""), "{text}");
        assert!(text.contains("description = \"planned for this version\""), "{text}");
        assert!(!text.contains("on_done"), "{text}");
        assert_eq!(back.tags.len(), 2);
        assert_eq!(back.tags[0].name, "BUG");
        // An unchosen colour stays absent on disk and falls back to the
        // name's hash; a chosen one round-trips.
        assert_eq!(back.tags[0].color, None);
        assert!(!text.contains("color") || text.matches("color").count() == 1, "{text}");
        assert_eq!(back.tags[1].color, Some(4));
        assert_eq!(back.next_key, 3);
        // T-217's two scalars ride the same rule: declared before the tables,
        // and round-tripped rather than dropped.
        assert!(!back.mcp_tools);
        assert!(back.claude_md_ignored);
        assert!(back.system_prompt);
        assert_eq!(back.default_column.as_deref(), Some("TODO"));
        assert_eq!(back.follow_up_mode, mesimon_core::board::FollowUpMode::Steer);
        assert_eq!(back.agent_provider, AgentProvider::Codex);
        assert_eq!(back.park_after_minutes, 30);
        // T-443: the default tier is a scalar before the tables, and the
        // tiers an array of tables after them — a scalar after `[[tiers]]`
        // would be a serialize error.
        assert_eq!(back.default_tier.as_deref(), Some("01TIER"));
        assert_eq!(back.tiers, cf.tiers);
        assert!(text.find("default_tier").unwrap() < text.find("[[columns]]").unwrap());
        assert!(text.contains("[[tiers]]") && text.contains("effort = \"xhigh\""), "{text}");
        assert_eq!(back.crown_budget, 5);
        assert!(text.find("agent_provider").unwrap() < text.find("[[columns]]").unwrap());

        let scalars = text.find("mcp_tools").expect("mcp_tools on disk");
        assert!(
            text.find("system_prompt").expect("system_prompt on disk")
                < text.find("[[columns]]").unwrap()
        );
        assert!(
            text.find("default_column").expect("default_column on disk")
                < text.find("[[columns]]").unwrap()
        );
        // T-353's three ride the same rule, and an unwritten one stays
        // unwritten: absent means mesimon's own words, which are in the
        // binary and never on disk.
        assert_eq!(back.prompt_rebase.as_deref(), Some("catch {branch} up to {base}"));
        assert!(back.prompt_merged.is_none());
        assert!(!text.contains("prompt_merged"), "an unset template is unwritten:\n{text}");
        assert!(
            text.find("prompt_rebase").expect("prompt_rebase on disk")
                < text.find("[[columns]]").unwrap()
        );
        let table = text.find("[[columns]]").expect("the columns table");
        assert!(scalars < table, "a scalar after a table is a TOML error:\n{text}");
        // The stamp is what stops an older build silently dropping the
        // registry on its next write: at v1 it would parse, ignore `tags`,
        // and overwrite the file without them.
        assert_eq!(COLUMNS_SCHEMA, 6);
        assert!(matches!(verdict(1, COLUMNS_SCHEMA), Verdict::Load));
        assert!(matches!(verdict(3, COLUMNS_SCHEMA), Verdict::Load));
        assert!(matches!(verdict(COLUMNS_SCHEMA, 3), Verdict::Newer(6)));
    }

    /// Today's ticket.toml carries no stamp; it must read as schema 1 (16 §6.2
    /// "absent -> treat as 1"), through the wrapper that now does the reading.
    #[test]
    fn unstamped_ticket_file_reads_as_v1() {
        let legacy = r#"
id = "01J8ZQ7VJ00000000000000000"
short_key = "T-3"
title = "old ticket"
column = "TODO"
order = "a0"
created_at = "@1788046350"

[archived]
at = "@1788050000"
by = "local"
"#;
        let tf: TicketFile = toml::from_str(legacy).unwrap();
        assert_eq!(tf.schema_version, 1);
        assert_eq!(tf.ticket.short_key, "T-3");
        assert!(tf.ticket.is_archived());
    }

    /// A ticket WITH a workspace field round-trips through the TOML writer
    /// (Option field must serialize after the scalars or to_string_pretty errors).
    #[test]
    fn workspace_field_roundtrips() {
        let t = Ticket {
            id: ulid::Ulid(7),
            short_key: "T-7".into(),
            title: "wt".into(),
            column: "TODO".into(),
            order: "a0".into(),
            created_at: "@0".into(),
            created_by: String::new(),
            created_from: None,
            entered_at: None,
            previous_column: None,
            woke_at: None,
            manual_merge: false,
            execution_policy: Default::default(),
            tier: None,
            import_origin: None,
            raised: None,
            workspace: Some(mesimon_core::board::WorkspaceStrategy::Worktree),
            tags: Vec::new(),
            notes: Vec::new(),
            archived: None,
        };
        let s = toml::to_string_pretty(&t).unwrap();
        let back: Ticket = toml::from_str(&s).unwrap();
        assert_eq!(back.workspace, Some(mesimon_core::board::WorkspaceStrategy::Worktree));
    }

    /// `created_by` (T-253) rides the file as a scalar, is omitted while
    /// empty, and a file from before it reads back as unknown — not as a
    /// person, which is what the ticket page's silence on a human ticket
    /// would otherwise claim of every old one.
    #[test]
    fn created_by_roundtrips_and_an_old_file_reads_unknown() {
        let mut t = Ticket {
            id: ulid::Ulid(8),
            short_key: "T-8".into(),
            title: "filed".into(),
            column: "TODO".into(),
            order: "a0".into(),
            created_at: "@0".into(),
            created_by: String::new(),
            created_from: None,
            entered_at: None,
            previous_column: None,
            woke_at: None,
            manual_merge: false,
            execution_policy: Default::default(),
            tier: None,
            import_origin: None,
            raised: None,
            workspace: None,
            tags: Vec::new(),
            notes: Vec::new(),
            archived: None,
        };
        let old = toml::to_string_pretty(&t).unwrap();
        assert!(!old.contains("created_by"), "empty is omitted:\n{old}");
        let back: Ticket = toml::from_str(&old).unwrap();
        assert!(back.created_by.is_empty());
        assert!(!back.agent_created());

        assert!(!old.contains("created_from"), "absent is omitted:\n{old}");
        assert_eq!(back.created_from, None);

        t.created_by = "agent:00000000-0000-0000-0000-000000000000".into();
        t.created_from = Some(ulid::Ulid(241));
        let s = toml::to_string_pretty(&t).unwrap();
        let back: Ticket = toml::from_str(&s).unwrap();
        assert!(back.agent_created());
        assert_eq!(back.created_by, t.created_by);
        assert_eq!(back.created_from, Some(ulid::Ulid(241)));
    }

    /// T-443: the machine's tiers round-trip, a file that does not parse is
    /// moved aside with every byte kept, and one from a newer build is left
    /// alone and bars writes.
    #[test]
    fn machine_tiers_roundtrip_quarantine_and_bar() {
        use mesimon_core::tier::{Effort, Tier};
        let dir = std::env::temp_dir().join(format!("msmn-store-tiers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("tiers.toml");
        let empty = load_machine_tiers(&path);
        assert!(empty.tiers.tiers.is_empty() && empty.notice.is_none() && !empty.barred);

        let tiers = MachineTiers {
            default_tier: Some("01A".into()),
            tiers: vec![Tier {
                id: "01A".into(),
                name: "quick".into(),
                provider: AgentProvider::ClaudeCode,
                model: "sonnet".into(),
                effort: Effort::High,
            }],
        };
        save_machine_tiers(&path, &tiers).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("schema_version = 1\n"), "{text}");
        let back = load_machine_tiers(&path);
        assert_eq!(back.tiers, tiers);
        assert!(back.notice.is_none() && !back.barred);

        std::fs::write(&path, "schema_version = 1\n[[tiers]\n").unwrap();
        let bad = load_machine_tiers(&path);
        assert!(bad.tiers.tiers.is_empty() && !bad.barred);
        assert_eq!(bad.notice.as_ref().map(|n| n.kind.as_str()), Some("quarantined"));
        assert!(!path.exists(), "moved aside");
        let kept = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().starts_with("tiers.toml.quarantine-"));
        assert!(kept, "the bytes are kept");

        std::fs::write(&path, "schema_version = 99\n").unwrap();
        let newer = load_machine_tiers(&path);
        assert!(newer.barred);
        assert_eq!(newer.notice.as_ref().map(|n| n.kind.as_str()), Some("future_version"));
        assert!(path.exists(), "a newer file is left untouched");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T-443: a ticket's tier pick round-trips as a scalar, and a board's
    /// tiers and default tier survive a save and a load.
    #[test]
    fn a_tier_pick_and_the_board_tiers_survive_a_load() {
        let (dir, paths) = scratch("tier-pick");
        let body = format!("schema_version = 7\n{}tier = \"01A\"\n", ticket_body("T-1", "TODO"));
        write(&paths.board_dir.join("board/tickets/T-1/ticket.toml"), &body);
        let mut loaded = load_with(&paths, false).unwrap();
        assert_eq!(loaded.board.tickets[0].tier.as_deref(), Some("01A"));
        loaded.board.default_tier = Some("01A".into());
        loaded.board.tiers.push(mesimon_core::tier::Tier {
            id: "01A".into(),
            name: "quick".into(),
            provider: AgentProvider::Codex,
            model: String::new(),
            effort: mesimon_core::tier::Effort::Default,
        });
        save_columns(&paths, &loaded.board).unwrap();
        save_ticket(&paths, &loaded.board.tickets[0]).unwrap();
        let again = load_with(&paths, false).unwrap();
        assert_eq!(again.board.default_tier.as_deref(), Some("01A"));
        assert_eq!(again.board.tiers, loaded.board.tiers);
        assert_eq!(again.board.tickets[0].tier.as_deref(), Some("01A"));
        cleanup(&dir, &paths);
    }
}
