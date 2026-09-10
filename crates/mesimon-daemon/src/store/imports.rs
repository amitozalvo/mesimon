//! Durable local import acceptance. Called only by the daemon's sole writer.
//! The local adapter owns remote verification; this journal records an already
//! owner-authorized, inert operation, never a remotely asserted principal.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use mesimon_core::board::Ticket;
use mesimon_core::command::Notice;
use mesimon_core::content::{ImportOrigin, PreparedImport, TicketContent};
use mesimon_core::{authorize, Action, Decision, Principal, Resource};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::paths::{own_private_dir, Paths};

const SCHEMA: u32 = 1;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u32,
    origin: ImportOrigin,
    digest: [u8; 32],
    id: ulid::Ulid,
    key: String,
    column: String,
    state: State,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
enum State {
    Pending {
        ticket: Box<Ticket>,
        note_bodies: Vec<(ulid::Ulid, String)>,
    },
    /// Staged data was durable before this marker. Missing both locations is
    /// uncertain acceptance/deletion, never permission to reconstruct a ticket.
    Ready,
    /// Keep correlation and digest after deletion, but discard the extra bodies.
    Committed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Receipt {
    pub id: ulid::Ulid,
    pub key: String,
}

fn root(paths: &Paths) -> PathBuf {
    paths.board_dir.join("board/imports")
}

fn source_key(origin: &ImportOrigin) -> String {
    let mut hash = Sha256::new();
    hash.update(b"mesimon-local-import-origin-v1\0");
    hash.update(origin.source.to_bytes());
    hash.update(origin.item.to_bytes());
    format!("{:x}", hash.finalize())
}

fn record_path(paths: &Paths, origin: &ImportOrigin) -> PathBuf {
    root(paths).join("receipts").join(format!("{}.json", source_key(origin)))
}

fn stage_path(paths: &Paths, origin: &ImportOrigin) -> PathBuf {
    root(paths).join("staging").join(source_key(origin))
}

fn final_path(paths: &Paths, key: &str) -> PathBuf {
    paths.board_dir.join("board/tickets").join(key)
}

pub(crate) fn digest(
    origin: &ImportOrigin,
    column: &str,
    content: &TicketContent,
) -> Result<[u8; 32]> {
    let bytes = serde_json::to_vec(&("mesimon-local-import-content-v1", origin, column, content))?;
    Ok(Sha256::digest(bytes).into())
}

fn sync_dir(path: &Path) -> Result<()> {
    fs::File::open(path)?.sync_all().map_err(Into::into)
}

fn ensure_layout(paths: &Paths) -> Result<()> {
    for dir in [root(paths), root(paths).join("receipts"), root(paths).join("staging")] {
        own_private_dir(&dir)?;
        sync_dir(dir.parent().context("import directory has no parent")?)?;
    }
    Ok(())
}

fn write_record(paths: &Paths, record: &Record) -> Result<()> {
    let path = record_path(paths, &record.origin);
    super::write_atomic(&path, &serde_json::to_string(record)?, super::PRIVATE)?;
    // Acceptance needs a durable receipt, not just write_atomic's best-effort
    // parent sync. A failure is an uncertain result that an exact retry resolves.
    sync_dir(path.parent().context("receipt has no parent")?)
}

fn read_record(path: &Path) -> Result<Record> {
    let record: Record = serde_json::from_str(&fs::read_to_string(path)?)
        .context("import receipt is malformed; left untouched")?;
    if record.schema != SCHEMA {
        bail!("unsupported import receipt schema {}; left untouched", record.schema);
    }
    if record.id == ulid::Ulid::nil()
        || !record
            .key
            .strip_prefix(mesimon_core::board::KEY_PREFIX)
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        || path.file_name().and_then(|s| s.to_str())
            != Some(&format!("{}.json", source_key(&record.origin)))
    {
        bail!("import receipt identity is inconsistent; left untouched");
    }
    Ok(record)
}

fn read_materialized(dir: &Path, record: &Record) -> Result<Ticket> {
    let file: super::TicketFile = toml::from_str(&fs::read_to_string(dir.join("ticket.toml"))?)?;
    let ticket = file.ticket;
    if file.schema_version != super::TICKET_SCHEMA
        || ticket.id != record.id
        || ticket.short_key != record.key
        || ticket.import_origin.as_ref() != Some(&record.origin)
    {
        bail!("import ticket identity or schema does not match its receipt; left untouched");
    }
    Ok(ticket)
}

fn exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => Ok(true),
        Ok(_) => bail!("import directory is not a real directory; left untouched"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}

/// Finish one recorded operation. Recovery repeats this owner-authorized action,
/// rather than inventing authority from the unverified import origin.
fn finish(
    paths: &Paths,
    mut record: Record,
    checkpoint: &mut impl FnMut(&str) -> Result<()>,
) -> Result<Receipt> {
    if authorize(
        &Principal::Local,
        &Action::ImportContent,
        &Resource::Column { name: record.column.clone() },
    ) != Decision::Allow
    {
        bail!("recorded import is not authorized");
    }
    let stage = stage_path(paths, &record.origin);
    let destination = final_path(paths, &record.key);
    if let State::Pending { ticket, note_bodies } = &record.state {
        if exists(&destination)? {
            // Pending precedes the durable ready marker, so it cannot have
            // published this destination. Never overwrite an unrelated ticket.
            bail!("reserved import ticket destination is already occupied");
        }
        own_private_dir(&stage)?;
        own_private_dir(&stage.join("notes"))?;
        if ticket.id != record.id
            || ticket.short_key != record.key
            || ticket.import_origin.as_ref() != Some(&record.origin)
            || ticket.column != record.column
            || note_bodies.len() != ticket.notes.len()
            || !ticket.notes.iter().zip(note_bodies).all(|(meta, (id, _))| meta.id == *id)
        {
            bail!("pending import content is inconsistent; left untouched");
        }
        let content = TicketContent {
            title: ticket.title.clone(),
            notes: note_bodies.iter().map(|(_, body)| body.clone()).collect(),
        };
        content.validate()?;
        if digest(&record.origin, &record.column, &content)? != record.digest {
            bail!("pending import digest does not match; left untouched");
        }
        for (id, body) in note_bodies {
            super::write_atomic(
                &stage.join("notes").join(format!("{id}.md")),
                body,
                super::SHARED,
            )?;
        }
        sync_dir(&stage.join("notes"))?;
        checkpoint("notes_saved")?;
        let file =
            super::TicketFile { schema_version: super::TICKET_SCHEMA, ticket: (**ticket).clone() };
        super::write_atomic(
            &stage.join("ticket.toml"),
            &toml::to_string_pretty(&file)?,
            super::SHARED,
        )?;
        sync_dir(&stage)?;
        sync_dir(stage.parent().context("stage has no parent")?)?;
        record.state = State::Ready;
        write_record(paths, &record)?;
        checkpoint("ready_saved")?;
    }
    if matches!(record.state, State::Ready) {
        if exists(&destination)? {
            // Already published before an uncertain response. Preserve all
            // subsequent local edits and never reapply the initial note bodies.
            read_materialized(&destination, &record)?;
        } else if exists(&stage)? {
            let ticket = read_materialized(&stage, &record)?;
            let notes = ticket
                .notes
                .iter()
                .map(|note| fs::read_to_string(stage.join("notes").join(format!("{}.md", note.id))))
                .collect::<std::io::Result<Vec<_>>>()?;
            let content = TicketContent { title: ticket.title, notes };
            content.validate()?;
            if digest(&record.origin, &record.column, &content)? != record.digest {
                bail!("staged import digest does not match; left untouched");
            }
            fs::rename(&stage, &destination)?;
        } else {
            bail!("import acceptance is uncertain: staged and original ticket are absent; refusing to recreate it");
        }
        sync_dir(destination.parent().context("ticket has no parent")?)?;
        sync_dir(stage.parent().context("stage has no parent")?)?;
        checkpoint("published")?;
        record.state = State::Committed;
        write_record(paths, &record)?;
        checkpoint("committed_saved")?;
    }
    Ok(Receipt { id: record.id, key: record.key })
}

pub(crate) fn replay(
    paths: &Paths,
    origin: &ImportOrigin,
    column: &str,
    content: &TicketContent,
) -> Result<Option<Receipt>> {
    let path = record_path(paths, origin);
    let record = match read_record(&path) {
        Ok(record) => record,
        Err(e)
            if e.downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            return Ok(None)
        }
        Err(e) => return Err(e),
    };
    if record.origin != *origin || record.digest != digest(origin, column, content)? {
        bail!("import origin was already used with different content or destination");
    }
    finish(paths, record, &mut |_| Ok(())).map(Some)
}

pub(crate) fn commit(paths: &Paths, prepared: PreparedImport) -> Result<Receipt> {
    commit_with_checkpoint(paths, prepared, &mut |_| Ok(()))
}

fn commit_with_checkpoint(
    paths: &Paths,
    prepared: PreparedImport,
    checkpoint: &mut impl FnMut(&str) -> Result<()>,
) -> Result<Receipt> {
    let origin = prepared.ticket.import_origin.clone().context("prepared import has no origin")?;
    let content = TicketContent {
        title: prepared.ticket.title.clone(),
        notes: prepared.note_bodies.iter().map(|(_, body)| body.clone()).collect(),
    };
    content.validate()?;
    ensure_layout(paths)?;
    let path = record_path(paths, &origin);
    if fs::symlink_metadata(&path).is_ok() {
        bail!("import receipt already exists; replay it instead");
    }
    let record = Record {
        schema: SCHEMA,
        digest: digest(&origin, &prepared.ticket.column, &content)?,
        id: prepared.ticket.id,
        key: prepared.ticket.short_key.clone(),
        column: prepared.ticket.column.clone(),
        origin,
        state: State::Pending {
            ticket: Box::new(prepared.ticket),
            note_bodies: prepared.note_bodies,
        },
    };
    write_record(paths, &record)?;
    checkpoint("pending_saved")?;
    finish(paths, record, checkpoint)
}

/// The daemon may add a recovered original to its in-memory board. A retained
/// receipt whose original was deleted intentionally has no materialized ticket.
pub(crate) fn materialized(paths: &Paths, receipt: &Receipt) -> Result<Option<Ticket>> {
    let destination = final_path(paths, &receipt.key);
    if !exists(&destination)? {
        return Ok(None);
    }
    let file: super::TicketFile =
        toml::from_str(&fs::read_to_string(destination.join("ticket.toml"))?)?;
    if file.schema_version != super::TICKET_SCHEMA || file.ticket.id != receipt.id {
        bail!("import original ticket identity or schema differs; left untouched");
    }
    Ok(Some(file.ticket))
}

/// Called during startup under the daemon singleton lock. Corrupt/newer journals
/// remain in place and bar retries for that origin; other local work stays usable.
pub(crate) fn recover(paths: &Paths) -> Vec<Notice> {
    let dir = root(paths).join("receipts");
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            return vec![Notice::new("import_recovery", "could not read local import receipts")
                .with_detail(e.to_string())]
        }
    };
    let mut notices = Vec::new();
    for entry in entries {
        let result = (|| -> Result<()> {
            let path = entry?.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                return Ok(());
            }
            let record = read_record(&path)?;
            if !matches!(record.state, State::Committed) {
                finish(paths, record, &mut |_| Ok(()))?;
            }
            Ok(())
        })();
        if let Err(e) = result {
            notices.push(
                Notice::new(
                    "import_recovery",
                    "a local import needs recovery; its receipt was left untouched",
                )
                .with_detail(e.to_string()),
            );
        }
    }
    notices
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesimon_core::board::{ExecutionPolicy, WorkspaceStrategy};
    use mesimon_core::content::ImportPlacement;

    struct Fixture {
        dir: PathBuf,
        paths: Paths,
    }
    impl Fixture {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("msmn-import-{}", ulid::Ulid::new()));
            let paths = Paths {
                repo_root: dir.clone(),
                proj16: "test".into(),
                rt_dir: dir.join("runtime"),
                state_dir: dir.join("state"),
                board_dir: dir.join(".mesimon"),
            };
            fs::create_dir_all(paths.board_dir.join("board/tickets")).unwrap();
            Self { dir, paths }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.dir).unwrap();
        }
    }
    fn origin() -> ImportOrigin {
        ImportOrigin { source: ulid::Ulid::from(10), item: ulid::Ulid::from(11) }
    }
    fn content() -> TicketContent {
        TicketContent {
            title: "Incoming question".into(),
            notes: vec!["# Context\nExact body\n".into(), "Second note".into()],
        }
    }
    fn prepared() -> PreparedImport {
        PreparedImport::prepare(
            &Principal::Local,
            content(),
            origin(),
            ImportPlacement {
                id: ulid::Ulid::from(1),
                short_key: "T-1".into(),
                column: "TODO".into(),
                order: "a0".into(),
                created_at: "@0".into(),
            },
            ulid::Ulid::new,
        )
        .unwrap()
    }
    fn assert_complete(paths: &Paths, receipt: &Receipt) {
        let ticket = materialized(paths, receipt).unwrap().unwrap();
        assert_eq!(ticket.import_origin, Some(origin()));
        assert_eq!(ticket.effective_execution_policy(), ExecutionPolicy::OwnerOnly);
        assert_eq!(ticket.workspace_strategy(), WorkspaceStrategy::Worktree);
        for (note, body) in ticket.notes.iter().zip(content().notes) {
            assert_eq!(super::super::read_note(paths, &ticket.short_key, note.id).unwrap(), body);
        }
    }

    #[test]
    fn interrupted_imports_publish_all_notes_atomically_and_recover_once() {
        for boundary in
            ["pending_saved", "notes_saved", "ready_saved", "published", "committed_saved"]
        {
            let f = Fixture::new();
            let mut hit = false;
            let result = commit_with_checkpoint(&f.paths, prepared(), &mut |at| {
                if at == boundary {
                    hit = true;
                    if matches!(at, "pending_saved" | "notes_saved" | "ready_saved") {
                        assert!(!final_path(&f.paths, "T-1").exists());
                        assert!(super::super::load(&f.paths)?.board.tickets.is_empty());
                    } else {
                        assert_complete(
                            &f.paths,
                            &Receipt { id: ulid::Ulid::from(1), key: "T-1".into() },
                        );
                    }
                    bail!("injected {at}");
                }
                Ok(())
            });
            assert!(hit && result.is_err(), "{boundary}");
            // Discard every in-memory candidate and recover solely from disk.
            assert!(recover(&f.paths).is_empty(), "{boundary}");
            let receipt = replay(&f.paths, &origin(), "TODO", &content()).unwrap().unwrap();
            assert_complete(&f.paths, &receipt);
            assert!(recover(&f.paths).is_empty());
            assert_eq!(super::super::load(&f.paths).unwrap().board.tickets.len(), 1);
            let record = read_record(&record_path(&f.paths, &origin())).unwrap();
            assert!(matches!(record.state, State::Committed));
            assert!(!fs::read_to_string(record_path(&f.paths, &origin()))
                .unwrap()
                .contains("Exact body"));
        }
    }

    #[test]
    fn retries_preserve_local_edits_and_deletion_and_reject_changed_requests() {
        let f = Fixture::new();
        let receipt = commit(&f.paths, prepared()).unwrap();
        let mut ticket = materialized(&f.paths, &receipt).unwrap().unwrap();
        ticket.title = "Owner edit".into();
        ticket.column = "RENAMED".into();
        super::super::save_ticket(&f.paths, &ticket).unwrap();
        super::super::save_note(&f.paths, &ticket.short_key, ticket.notes[0].id, "Private draft")
            .unwrap();
        assert_eq!(replay(&f.paths, &origin(), "TODO", &content()).unwrap(), Some(receipt.clone()));
        assert_eq!(materialized(&f.paths, &receipt).unwrap().unwrap().title, "Owner edit");
        assert_eq!(
            super::super::read_note(&f.paths, &ticket.short_key, ticket.notes[0].id).unwrap(),
            "Private draft"
        );
        let mut changed = content();
        changed.title = "Different".into();
        assert!(replay(&f.paths, &origin(), "TODO", &changed).is_err());
        assert!(replay(&f.paths, &origin(), "OTHER", &content()).is_err());
        fs::remove_dir_all(final_path(&f.paths, &receipt.key)).unwrap();
        assert!(recover(&f.paths).is_empty());
        assert_eq!(replay(&f.paths, &origin(), "TODO", &content()).unwrap(), Some(receipt.clone()));
        assert!(materialized(&f.paths, &receipt).unwrap().is_none());
    }

    #[test]
    fn occupied_destination_is_never_overwritten_or_mistaken_for_acceptance() {
        let f = Fixture::new();
        let mut occupied = prepared().ticket;
        occupied.id = ulid::Ulid::from(99);
        occupied.import_origin = None;
        occupied.title = "Local work".into();
        super::super::save_ticket(&f.paths, &occupied).unwrap();
        let before = fs::read(final_path(&f.paths, "T-1").join("ticket.toml")).unwrap();
        assert!(commit(&f.paths, prepared()).is_err());
        assert!(!recover(&f.paths).is_empty());
        assert!(replay(&f.paths, &origin(), "TODO", &content()).is_err());
        assert_eq!(fs::read(final_path(&f.paths, "T-1").join("ticket.toml")).unwrap(), before);
    }

    #[test]
    fn uncertain_ready_acceptance_and_corrupt_receipts_fail_closed() {
        let f = Fixture::new();
        assert!(commit_with_checkpoint(&f.paths, prepared(), &mut |at| {
            if at == "ready_saved" {
                bail!("interrupted");
            }
            Ok(())
        })
        .is_err());
        fs::remove_dir_all(stage_path(&f.paths, &origin())).unwrap();
        assert!(!recover(&f.paths).is_empty());
        assert!(replay(&f.paths, &origin(), "TODO", &content()).is_err());
        assert!(!final_path(&f.paths, "T-1").exists());
        let path = record_path(&f.paths, &origin());
        for damaged in ["{truncated", "{\"schema\":999}"] {
            fs::write(&path, damaged).unwrap();
            assert!(!recover(&f.paths).is_empty());
            assert!(replay(&f.paths, &origin(), "TODO", &content()).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), damaged);
        }
    }

    #[test]
    fn tampered_stage_never_publishes() {
        let f = Fixture::new();
        let candidate = prepared();
        let first_note = candidate.ticket.notes[0].id;
        assert!(commit_with_checkpoint(&f.paths, candidate, &mut |at| {
            if at == "ready_saved" {
                bail!("interrupted");
            }
            Ok(())
        })
        .is_err());
        fs::write(
            stage_path(&f.paths, &origin()).join("notes").join(format!("{first_note}.md")),
            "Unexpected replacement",
        )
        .unwrap();
        assert!(!recover(&f.paths).is_empty());
        assert!(!final_path(&f.paths, "T-1").exists());
    }
}
