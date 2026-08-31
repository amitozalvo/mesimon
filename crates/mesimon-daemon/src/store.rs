//! Board persistence. Repo side (D12 layout, uncommitted per D34.9):
//!   .mesimon/board/columns.toml
//!   .mesimon/board/tickets/<SHORT-KEY>/ticket.toml   (ULID never in a path — D24)
//! Runtime side (D33b): sessions.json in the state dir.
//!
//! Files mesimon authors are rewritten atomically (temp + rename). mesimon never
//! round-trip-rewrites a user's file (D26) — spec.md etc. are opaque blobs.

use std::path::Path;

use anyhow::Result;
use mesimon_core::board::{Board, Column, SessionRecord, Ticket};
use mesimon_core::command::Notice;
use serde::{Deserialize, Serialize};

use crate::paths::Paths;

/// On-disk schema stamps (16 §6.2). Four state files, four independent
/// counters — a ticket change must not force a sessions migration. Absent is
/// read as 1; newer than ours refuses THAT file and bars writes to it.
/// v2 added the tag registry. Bumped rather than defaulted on purpose: at v1
/// an older build would read the file, ignore `tags`, and DROP the whole
/// registry on its next write. The stamp makes it refuse the file and bar its
/// writes instead — 16 §6.2's rule that a newer file is left untouched rather
/// than silently downgraded.
pub const COLUMNS_SCHEMA: u32 = 2;
pub const TICKET_SCHEMA: u32 = 1;
pub const SESSIONS_SCHEMA: u32 = 1;

fn schema_v1() -> u32 {
    1
}

#[derive(Serialize, Deserialize, Default)]
struct ColumnsFile {
    /// MUST be first: it is a scalar and `columns` serializes as `[[columns]]`,
    /// an array of tables. Any scalar after a table is a TOML serialize error.
    #[serde(default = "schema_v1")]
    schema_version: u32,
    next_key: u64,
    columns: Vec<Column>,
    /// The tag registry (v2). Another array of tables, so it may follow
    /// `columns` but must stay after every scalar.
    #[serde(default)]
    tags: Vec<mesimon_core::board::Tag>,
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

pub(crate) fn write_atomic(path: &Path, content: &str) -> Result<()> {
    // 13 §13.9.1: temp + fsync + rename + directory fsync, measured at ~170 µs
    // total. Without the fsync a crash between write and rename leaves a
    // truncated file — precisely the malformed input `load` now has to
    // quarantine, so this is the cheapest way to stop manufacturing them.
    // NOT F_FULLFSYNC: doc 13 reserves that (2.09 ms, 51x) for boot_epoch.
    //
    // `with_extension` REPLACES the extension, so the temp is `sessions.tmp`,
    // not `sessions.json.tmp` — deliberate: `load` reads exact names and the
    // flock singleton means there is never a second writer, and it keeps the
    // temp out of any `*.json`/`*.toml` glob.
    let tmp = path.with_extension("tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        std::io::Write::write_all(&mut f, content.as_bytes())?;
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
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
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
    // `older` is unreachable while every counter is at 1; when a second
    // version lands, the migration chain goes here (16 §6.2).
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
        .filter_map(|e| e.file_name().to_string_lossy().strip_prefix("T-")?.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    let from_parsed = parsed
        .iter()
        .filter_map(|t| t.short_key.strip_prefix("T-")?.parse::<u64>().ok())
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
        board.columns.push(Column { name, order: order.clone() });
        prev = order;
    }
}

/// Load `columns.toml`. Returns the board skeleton, whether writes to that
/// file are barred, and whether the on-disk state was lost (so `next_key`
/// must be recovered from the ticket directories).
fn load_columns(cols_path: &Path, notices: &mut Vec<Notice>) -> (Board, bool, bool) {
    let defaults = || Board::with_default_columns();
    if !cols_path.is_file() {
        return (defaults(), false, false); // fresh repo; caller writes it out
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
            return (defaults(), true, true);
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
                        return (defaults(), true, true);
                    }
                    Verdict::Load => match v.try_into::<ColumnsFile>() {
                        Ok(cf) => {
                            let b = Board {
                                columns: cf.columns,
                                next_key: cf.next_key,
                                tags: cf.tags,
                                ..Default::default()
                            };
                            return (b, false, false);
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
    (defaults(), barred, true)
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

/// Load the board. A parse failure is a NOTICE, never an error: the daemon
/// must come up. `Err` is reserved for genuine environment faults.
pub fn load(paths: &Paths) -> Result<Loaded> {
    let mut notices = Vec::new();
    let cols_path = paths.board_dir.join("board/columns.toml");
    let fresh = !cols_path.is_file();
    let (mut board, columns_write_barred, columns_lost) = load_columns(&cols_path, &mut notices);

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
    // Write the defaults out only when the path is actually free: a fresh
    // repo, or a quarantine that succeeded in moving the bad file aside.
    if (fresh || columns_lost) && !columns_write_barred {
        save_columns(paths, &board)?;
    }

    let (sessions, sessions_write_barred) = load_sessions(&paths.sessions_file(), &mut notices);
    board.sessions = sessions;

    Ok(Loaded { board, notices, columns_write_barred, sessions_write_barred })
}

pub fn save_columns(paths: &Paths, board: &Board) -> Result<()> {
    let cf = ColumnsFile {
        schema_version: COLUMNS_SCHEMA,
        next_key: board.next_key,
        columns: board.columns.clone(),
        tags: board.tags.clone(),
    };
    write_atomic(&paths.board_dir.join("board/columns.toml"), &toml::to_string_pretty(&cf)?)
}

pub fn save_ticket(paths: &Paths, t: &Ticket) -> Result<()> {
    let dir = paths.board_dir.join("board/tickets").join(&t.short_key);
    std::fs::create_dir_all(&dir)?;
    let tf = TicketFile { schema_version: TICKET_SCHEMA, ticket: t.clone() };
    write_atomic(&dir.join("ticket.toml"), &toml::to_string_pretty(&tf)?)
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
    write_atomic(&paths.sessions_file(), &serde_json::to_string_pretty(&sf)?)
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
            workspace: Some(mesimon_core::board::WorkspaceStrategy::Worktree),
            tags: Vec::new(),
            archived: Some(mesimon_core::board::Archived {
                at: "@1788046350".into(),
                by: "local".into(),
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
            workspace: Some(mesimon_core::board::WorkspaceStrategy::Worktree),
            tags: Vec::new(),
            archived: Some(mesimon_core::board::Archived {
                at: "@1788046350".into(),
                by: "local".into(),
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
                workspace: Some(mesimon_core::board::WorkspaceStrategy::Worktree),
                tags: Vec::new(),
                archived: Some(mesimon_core::board::Archived {
                    at: "@1788050000".into(),
                    by: "local".into(),
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
    /// to empty. The board comes up; nothing is seeded.
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
            columns: vec![Column { name: "TODO".into(), order: "a0".into() }],
            tags: vec![
                mesimon_core::board::Tag { name: "BUG".into(), group: 1, color: None },
                mesimon_core::board::Tag { name: "STAGING".into(), group: 2, color: Some(4) },
            ],
        };
        let text = toml::to_string_pretty(&cf).unwrap();
        let back: ColumnsFile = toml::from_str(&text).unwrap();
        assert_eq!(back.tags.len(), 2);
        assert_eq!(back.tags[0].name, "BUG");
        // An unchosen colour stays absent on disk and falls back to the
        // name's hash; a chosen one round-trips.
        assert_eq!(back.tags[0].color, None);
        assert!(!text.contains("color") || text.matches("color").count() == 1, "{text}");
        assert_eq!(back.tags[1].color, Some(4));
        assert_eq!(back.next_key, 3);
        // The stamp is what stops an older build silently dropping the
        // registry on its next write: at v1 it would parse, ignore `tags`,
        // and overwrite the file without them.
        assert_eq!(COLUMNS_SCHEMA, 2);
        assert!(matches!(verdict(1, COLUMNS_SCHEMA), Verdict::Load));
        assert!(matches!(verdict(COLUMNS_SCHEMA, 1), Verdict::Newer(2)));
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
            workspace: Some(mesimon_core::board::WorkspaceStrategy::Worktree),
            tags: Vec::new(),
            archived: None,
        };
        let s = toml::to_string_pretty(&t).unwrap();
        let back: Ticket = toml::from_str(&s).unwrap();
        assert_eq!(back.workspace, Some(mesimon_core::board::WorkspaceStrategy::Worktree));
    }
}
