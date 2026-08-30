//! Board persistence. Repo side (D12 layout, uncommitted per D34.9):
//!   .mesimon/board/columns.toml
//!   .mesimon/board/tickets/<SHORT-KEY>/ticket.toml   (ULID never in a path — D24)
//! Runtime side (D33b): sessions.json in the state dir.
//!
//! Files mesimon authors are rewritten atomically (temp + rename). mesimon never
//! round-trip-rewrites a user's file (D26) — spec.md etc. are opaque blobs.

use std::path::Path;

use anyhow::{Context, Result};
use mesimon_core::board::{Board, Column, SessionRecord, Ticket};
use serde::{Deserialize, Serialize};

use crate::paths::Paths;

#[derive(Serialize, Deserialize, Default)]
struct ColumnsFile {
    next_key: u64,
    columns: Vec<Column>,
}

fn write_atomic(path: &Path, content: &str) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn load(paths: &Paths) -> Result<Board> {
    let cols_path = paths.board_dir.join("board/columns.toml");
    let mut board = if cols_path.exists() {
        let cf: ColumnsFile = toml::from_str(&std::fs::read_to_string(&cols_path)?)
            .with_context(|| format!("parse {}", cols_path.display()))?;
        Board { columns: cf.columns, next_key: cf.next_key, ..Default::default() }
    } else {
        let b = Board::with_default_columns();
        save_columns(paths, &b)?;
        b
    };

    let tickets_dir = paths.board_dir.join("board/tickets");
    if tickets_dir.is_dir() {
        for entry in std::fs::read_dir(&tickets_dir)? {
            let entry = entry?;
            let tp = entry.path().join("ticket.toml");
            if tp.is_file() {
                let t: Ticket = toml::from_str(&std::fs::read_to_string(&tp)?)
                    .with_context(|| format!("parse {}", tp.display()))?;
                board.tickets.push(t);
            }
        }
    }

    let sf = paths.sessions_file();
    if sf.is_file() {
        let recs: Vec<SessionRecord> = serde_json::from_str(&std::fs::read_to_string(&sf)?)
            .with_context(|| format!("parse {}", sf.display()))?;
        board.sessions = recs;
    }
    Ok(board)
}

pub fn save_columns(paths: &Paths, board: &Board) -> Result<()> {
    let cf = ColumnsFile { next_key: board.next_key, columns: board.columns.clone() };
    write_atomic(
        &paths.board_dir.join("board/columns.toml"),
        &toml::to_string_pretty(&cf)?,
    )
}

pub fn save_ticket(paths: &Paths, t: &Ticket) -> Result<()> {
    let dir = paths.board_dir.join("board/tickets").join(&t.short_key);
    std::fs::create_dir_all(&dir)?;
    write_atomic(&dir.join("ticket.toml"), &toml::to_string_pretty(t)?)
}

pub fn delete_ticket_dir(paths: &Paths, short_key: &str) -> Result<()> {
    let dir = paths.board_dir.join("board/tickets").join(short_key);
    if dir.is_dir() {
        std::fs::remove_dir_all(dir)?;
    }
    Ok(())
}

pub fn save_sessions(paths: &Paths, board: &Board) -> Result<()> {
    write_atomic(
        &paths.sessions_file(),
        &serde_json::to_string_pretty(&board.sessions)?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(
            t.workspace_strategy(),
            mesimon_core::board::WorkspaceStrategy::SharedCheckout
        );
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
            archived: None,
        };
        let s = toml::to_string_pretty(&t).unwrap();
        let back: Ticket = toml::from_str(&s).unwrap();
        assert_eq!(back.workspace, Some(mesimon_core::board::WorkspaceStrategy::Worktree));
    }
}
