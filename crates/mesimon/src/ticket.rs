//! `mesimon ticket create` (T-693): file a ticket from the shell, the way
//! the composer does, and print its key.
//!
//! This is mesimon's whole half of any "file a ticket from X" integration —
//! a chat command's bridge, a launcher, a git hook, a cron. The script that
//! calls it is the person's own and lives outside this repository; mesimon
//! enables it and provides none of it. The repository's daemon does the work
//! as it does for the board, under the person's own principal (same uid,
//! same trust the TUI has), and is started when none is running, so a ticket
//! lands with the board closed.
use std::io::Read;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use mesimon_core::board::{sanitize_tag, Board, TagRef};
use mesimon_core::command::{Command, Response};
use mesimon_tui::client::{Client, Transport};

const USAGE: &str = "usage: mesimon ticket create --column <name> --title <text> \
[--repo <path>] [--note <markdown> | --note -] [--tag <name>]...\n\
Files a ticket on the board of <path> (default: the current directory) and prints its key.\n\
--note is the description; `-` reads it from stdin. --tag names a tag the board already has.";

/// What the command line asked for, before the board is consulted.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Create {
    repo: Option<PathBuf>,
    column: String,
    title: String,
    /// `None` is no description; `Some("-")` is stdin.
    note: Option<String>,
    tags: Vec<String>,
}

pub fn run(args: &[String]) -> Result<()> {
    if matches!(args.first().map(String::as_str), None | Some("--help" | "-h")) {
        println!("{USAGE}");
        return Ok(());
    }
    let create = match parse(args) {
        Ok(create) => create,
        Err(complaint) => {
            eprintln!("{complaint}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    let repo = match create.repo {
        Some(repo) => repo,
        None => std::env::current_dir()?,
    };
    let text = match create.note.as_deref() {
        None => String::new(),
        Some("-") => {
            let mut text = String::new();
            std::io::stdin().read_to_string(&mut text).context("read the note from stdin")?;
            text
        }
        Some(text) => text.to_string(),
    };
    let mut client = Client::connect(&repo).context("connect to the daemon")?;
    let board = snapshot(&mut client)?;
    let column = column_of(&board, &create.column).map_err(|e| anyhow::anyhow!(e))?;
    let tags = tags_of(&board, &create.tags).map_err(|e| anyhow::anyhow!(e))?;
    let command = Command::CreateTicketWithNote {
        column,
        title: create.title,
        workspace: None,
        text,
        uploads: Vec::new(),
        tags,
        tier: None,
    };
    let id = match client.request(command)? {
        Response::Created { id } => id,
        Response::Err { message } => bail!("{message}"),
        other => bail!("unexpected answer to a create: {other:?}"),
    };
    // The receipt is the id; the key a person speaks is on the board.
    let key = snapshot(&mut client)?
        .ticket(id)
        .map(|t| t.short_key.clone())
        .with_context(|| format!("the daemon created {id} and then lost it"))?;
    println!("{key}");
    Ok(())
}

fn snapshot(client: &mut Client) -> Result<Board> {
    match client.request(Command::Snapshot)? {
        Response::Board { board, .. } => Ok(board),
        other => bail!("unexpected answer to a snapshot: {other:?}"),
    }
}

/// `create` and its flags. The complaint is for a person at a shell: what
/// was wrong, before the usage line.
fn parse(args: &[String]) -> Result<Create, String> {
    let (verb, flags) = args.split_first().ok_or_else(|| "a verb is needed".to_string())?;
    if verb != "create" {
        return Err(format!("unknown verb: {verb}"));
    }
    let mut create = Create {
        repo: None,
        column: String::new(),
        title: String::new(),
        note: None,
        tags: Vec::new(),
    };
    let mut it = flags.iter();
    while let Some(flag) = it.next() {
        let mut value = || it.next().ok_or_else(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--repo" => create.repo = Some(PathBuf::from(value()?)),
            "--column" => create.column = value()?.trim().to_string(),
            "--title" => create.title = value()?.trim().to_string(),
            "--note" => create.note = Some(value()?.to_string()),
            "--tag" => create.tags.push(value()?.to_string()),
            other => return Err(format!("unknown flag: {other}")),
        }
    }
    if create.column.is_empty() {
        return Err("--column is needed".into());
    }
    if create.title.is_empty() {
        return Err("--title is needed".into());
    }
    Ok(create)
}

/// The board's own spelling of a column named at the shell, case aside;
/// a miss lists the columns there are.
fn column_of(board: &Board, name: &str) -> Result<String, String> {
    board
        .columns
        .iter()
        .find(|c| c.name.eq_ignore_ascii_case(name))
        .map(|c| c.name.clone())
        .ok_or_else(|| {
            let names: Vec<&str> = board.columns.iter().map(|c| c.name.as_str()).collect();
            format!("no such column: {name} (this board has {})", names.join(", "))
        })
}

/// Tags by name, from the board's registry alone: a script never adds to
/// the vocabulary, so a typo is refused rather than worn.
fn tags_of(board: &Board, names: &[String]) -> Result<Vec<TagRef>, String> {
    names
        .iter()
        .map(|raw| {
            let name = sanitize_tag(raw).ok_or_else(|| "empty tag name".to_string())?;
            board
                .tags
                .iter()
                .find(|t| t.name.eq_ignore_ascii_case(&name))
                .map(|t| TagRef { name: t.name.clone(), group: t.group })
                .ok_or_else(|| {
                    let known: Vec<&str> = board.tags.iter().map(|t| t.name.as_str()).collect();
                    format!("no such tag: {name} (this board has {})", known.join(", "))
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesimon_core::board::Tag;

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn create_takes_its_flags_in_any_order_and_repeats_only_tag() {
        let c = parse(&a(&[
            "create",
            "--tag",
            "bug",
            "--title",
            " Flaky login ",
            "--note",
            "-",
            "--column",
            "todo",
            "--tag",
            "feature",
            "--repo",
            "/r",
        ]))
        .unwrap();
        assert_eq!(
            c,
            Create {
                repo: Some(PathBuf::from("/r")),
                column: "todo".into(),
                title: "Flaky login".into(),
                note: Some("-".into()),
                tags: a(&["bug", "feature"]),
            }
        );
        let c = parse(&a(&["create", "--column", "TODO", "--title", "t"])).unwrap();
        assert_eq!((c.repo, c.note, c.tags), (None, None, Vec::new()));
    }

    #[test]
    fn create_refuses_what_it_cannot_file() {
        let err = |v: &[&str]| parse(&a(v)).unwrap_err();
        assert_eq!(err(&["create", "--title", "t"]), "--column is needed");
        assert_eq!(err(&["create", "--column", "TODO", "--title", "  "]), "--title is needed");
        assert_eq!(err(&["create", "--column"]), "--column needs a value");
        assert_eq!(
            err(&["create", "--column", "TODO", "--title", "t", "--tier", "x"]),
            "unknown flag: --tier"
        );
        assert_eq!(err(&["delete", "--column", "TODO"]), "unknown verb: delete");
    }

    fn board() -> Board {
        let mut b = Board::default();
        for (i, name) in ["TODO", "IN PROGRESS", "DONE"].iter().enumerate() {
            b.columns.push(mesimon_core::board::Column::new(*name, i.to_string()));
        }
        b.tags.push(Tag { name: "FEATURE".into(), group: 1, color: None });
        b.tags.push(Tag { name: "QUESTION".into(), group: 2, color: None });
        b
    }

    #[test]
    fn a_column_is_matched_case_aside_and_a_miss_lists_them() {
        let b = board();
        assert_eq!(column_of(&b, "in progress").unwrap(), "IN PROGRESS");
        assert_eq!(
            column_of(&b, "later").unwrap_err(),
            "no such column: later (this board has TODO, IN PROGRESS, DONE)"
        );
    }

    #[test]
    fn a_tag_comes_from_the_registry_with_its_group_and_a_stranger_is_refused() {
        let b = board();
        assert_eq!(
            tags_of(&b, &a(&["feature", "QUESTION"])).unwrap(),
            vec![
                TagRef { name: "FEATURE".into(), group: 1 },
                TagRef { name: "QUESTION".into(), group: 2 }
            ]
        );
        assert_eq!(
            tags_of(&b, &a(&["BUG"])).unwrap_err(),
            "no such tag: BUG (this board has FEATURE, QUESTION)"
        );
        assert_eq!(tags_of(&b, &a(&["  "])).unwrap_err(), "empty tag name");
        assert!(tags_of(&b, &[]).unwrap().is_empty());
    }
}
