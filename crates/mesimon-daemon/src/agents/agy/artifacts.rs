//! agy's artifact panel identifies entries by filename. A decision also binds
//! the inode revision read for the card and a fresh projection nonce.
use super::*;
use rustix::fs::{open, openat, Mode, OFlags};
use std::fs::File;
use std::io::Read;
use std::os::unix::fs::MetadataExt;

pub(crate) const PLAN: &str = "implementation_plan.md";
const FLAGS: OFlags =
    OFlags::RDONLY.union(OFlags::NOFOLLOW).union(OFlags::CLOEXEC).union(OFlags::NONBLOCK);

/// Bind a user-facing feedback request to a Markdown file directly in the lead's
/// artifact directory. The tool's inline content is never the plan note.
pub(super) fn written(frame: &HookFrame, record: &SessionRecord) -> Option<String> {
    let lead = frame_conversation(frame)?;
    if record.agy_conversation_id.as_deref() != Some(lead) {
        return None;
    }
    let call = &frame.payload["toolCall"];
    let args = &call["args"];
    let file = match call["name"].as_str()? {
        "write_file" => args["Path"].as_str()?,
        "write_to_file" => args["TargetFile"].as_str()?,
        _ => return None,
    };
    let metadata = &args["ArtifactMetadata"];
    if metadata["UserFacing"] != true || metadata["RequestFeedback"] != true {
        return None;
    }
    let dir = Path::new(frame.payload["artifactDirectoryPath"].as_str()?);
    let path = Path::new(file);
    let name = path.file_name()?.to_str()?;
    let brain = home().join("brain");
    if !mesimon_core::attachment::valid_basename(name)
        || !name.ends_with(".md")
        || path.parent()? != dir
        || (dir != brain.join(lead) && dir != brain.canonicalize().ok()?.join(lead))
    {
        return None;
    }
    Some(name.into())
}

fn conversation(brain: &Path, lead: &str) -> Option<File> {
    let brain = open(brain.canonicalize().ok()?, FLAGS | OFlags::DIRECTORY, Mode::empty()).ok()?;
    Some(File::from(
        openat(brain, conversation_id(lead)?, FLAGS | OFlags::DIRECTORY, Mode::empty()).ok()?,
    ))
}

fn regular(dir: &File, name: &str) -> Option<File> {
    if !mesimon_core::attachment::valid_basename(name) {
        return None;
    }
    let file = File::from(openat(dir, name, FLAGS, Mode::empty()).ok()?);
    file.metadata().ok()?.is_file().then_some(file)
}

fn revision(file: &File) -> Option<String> {
    let m = file.metadata().ok()?;
    Some(format!(
        "{}-{}-{}-{}-{}-{}-{}",
        m.dev(),
        m.ino(),
        m.len(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec()
    ))
}

fn read_note(file: &File) -> Option<(String, bool)> {
    use mesimon_core::{
        board::{sanitize_note, NOTE_MAX_BYTES},
        text::cap_bytes,
    };
    let size = file.metadata().ok()?.len();
    let mut bytes = Vec::new();
    file.take(NOTE_MAX_BYTES as u64).read_to_end(&mut bytes).ok()?;
    let mut text = sanitize_note(&String::from_utf8_lossy(&bytes));
    if size > NOTE_MAX_BYTES as u64 {
        let marker = format!("\n\n[Preview truncated · full artifact: {size} bytes]");
        text.truncate(cap_bytes(&text, NOTE_MAX_BYTES - marker.len()).len());
        text.push_str(&marker);
    }
    let exact = size == bytes.len() as u64 && bytes == text.as_bytes();
    (!text.trim().is_empty()).then_some((text, exact))
}

fn parts(request: &str) -> Option<(&str, &str, &str)> {
    let rest = request.strip_prefix("agy-plan/")?;
    let (lead, rest) = rest.split_once('/')?;
    let (version, name) = rest.split_once('/')?;
    let (_, stamp) = version.split_once(':')?;
    Some((lead, stamp, name))
}

/// Recheck the actual file even when its next hook or title has not arrived.
pub(crate) fn current(rec: &SessionRecord, request: &str) -> bool {
    let Some((lead, stamp, name)) = parts(request) else { return false };
    rec.agy_review_count > 0
        && rec.agy_plan_request.as_deref() == Some(request)
        && rec.agy_conversation_id.as_deref() == Some(lead)
        && conversation(&home().join("brain"), lead)
            .and_then(|dir| regular(&dir, name))
            .and_then(|file| revision(&file))
            .as_deref()
            == Some(stamp)
}

/// A new invocation or an empty idle title retires the whole previous review.
pub(super) fn end_review(rec: &mut SessionRecord) -> bool {
    let changed = rec.agy_review_count != 0 || rec.agy_plan_request.is_some();
    rec.agy_review_count = 0;
    rec.agy_plan_request = None;
    rec.agy_artifact = AgyArtifact::Unseen;
    changed
}

pub(super) fn observe(
    rec: &mut SessionRecord,
    count: u64,
    brain: &Path,
    result: &mut HookObservation,
) {
    result.metadata_changed |= rec.agy_review_count != count;
    rec.agy_review_count = count;
    result.signal = Some(Signal::Attention { reason: Reason::Plan });
    if count != 1 {
        rec.agy_artifact = AgyArtifact::PaneOnly;
    }
    let candidate = (|| {
        let lead =
            rec.agy_conversation_id.as_deref().filter(|_| rec.agy_artifact == AgyArtifact::Plan)?;
        let file = regular(&conversation(brain, lead)?, PLAN)?;
        let stamp = revision(&file)?;
        let (note, exact) = read_note(&file)?;
        if revision(&file)? != stamp {
            return None;
        }
        result.plan = Some(note);
        if !exact {
            return None;
        }
        let old = rec
            .agy_plan_request
            .as_ref()
            .filter(|r| parts(r) == Some((lead, stamp.as_str(), PLAN)));
        Some(
            old.cloned().unwrap_or_else(|| {
                format!("agy-plan/{lead}/{}:{stamp}/{PLAN}", uuid::Uuid::new_v4())
            }),
        )
    })();
    result.agy_candidate = candidate;
}

pub(crate) const IN_PANE: &str = "review in agy's pane";

pub(crate) fn receipt(input: &str) -> Option<(bool, &str)> {
    let input = input.trim_end_matches(['\r', '\n']);
    input
        .strip_prefix("[Approved] ")
        .map(|name| (true, name))
        .or_else(|| input.strip_prefix("[Rejected] ").map(|name| (false, name)))
}
