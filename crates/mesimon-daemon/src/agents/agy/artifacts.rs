//! agy's artifact panel identifies entries by filename. A decision also binds
//! the inode revision read for the card and a fresh projection nonce.
use super::*;
use rustix::fs::{open, openat, Mode, OFlags};
use std::fs::File;
use std::io::Read;
use std::os::unix::fs::MetadataExt;

const PLAN: &str = "implementation_plan.md";
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

fn read_note(file: &File) -> Option<String> {
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
    (!text.trim().is_empty()).then_some(text)
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
    rec.agy_reviewing
        && rec.agy_plan_request.as_deref() == Some(request)
        && rec.agy_conversation_id.as_deref() == Some(lead)
        && conversation(&home().join("brain"), lead)
            .and_then(|dir| regular(&dir, name))
            .and_then(|file| revision(&file))
            .as_deref()
            == Some(stamp)
}

pub(super) fn answered(rec: &mut SessionRecord, input: &str) {
    let Some(name) =
        input.strip_prefix("[Approved] ").or_else(|| input.strip_prefix("[Rejected] "))
    else {
        return;
    };
    if !rec.agy_pending_artifacts.iter().any(|n| n == name) {
        return;
    }
    rec.agy_pending_artifacts.retain(|n| n != name);
    let stamp = rec
        .agy_plan_request
        .as_deref()
        .and_then(parts)
        .filter(|(_, _, current)| *current == name)
        .map(|(_, stamp, _)| stamp.to_owned())
        .or_else(|| {
            let dir = conversation(&home().join("brain"), rec.agy_conversation_id.as_deref()?)?;
            revision(&regular(&dir, name)?)
        });
    if let Some(stamp) = stamp {
        rec.agy_answered_artifacts.insert(name.into(), stamp);
    }
}

/// Recover the canonical plan and feedback sidecars agy leaves beside the
/// other artifacts. Historical, already answered revisions stay answered.
fn discover(dir: &File) -> Vec<String> {
    let Ok(entries) = rustix::fs::Dir::read_from(dir) else { return Vec::new() };
    let mut names = Vec::new();
    let mut plan_has_sidecar = false;
    for entry in entries.flatten() {
        let Ok(sidecar) = entry.file_name().to_str() else { continue };
        let Some(name) = sidecar.strip_suffix(".md.metadata.json") else { continue };
        plan_has_sidecar |= sidecar == "implementation_plan.md.metadata.json";
        let mut bytes = Vec::new();
        let Some(file) = regular(dir, sidecar) else { continue };
        if file.take(16 * 1024).read_to_end(&mut bytes).is_err() {
            continue;
        }
        let Ok(meta) = serde_json::from_slice::<Value>(&bytes) else { continue };
        if meta["userFacing"] == true && meta["requestFeedback"] == true {
            names.push(format!("{name}.md"));
        }
    }
    names.sort();
    if !plan_has_sidecar && regular(dir, PLAN).is_some() {
        names.push(PLAN.into());
    }
    names
}

pub(super) fn observe(rec: &mut SessionRecord, brain: &Path, result: &mut HookObservation) {
    let Some(lead) = rec.agy_conversation_id.clone() else { return };
    let Some(dir) = conversation(brain, &lead) else {
        rec.agy_reviewing = false;
        result.metadata_changed |= rec.agy_plan_request.take().is_some();
        return;
    };
    if rec.agy_pending_artifacts.is_empty() {
        rec.agy_pending_artifacts = discover(&dir)
            .into_iter()
            .filter(|name| {
                let answered = rec.agy_answered_artifacts.get(name);
                answered.is_none()
                    || regular(&dir, name).and_then(|file| revision(&file)).as_ref() != answered
            })
            .collect();
    }
    for name in &rec.agy_pending_artifacts {
        let Some(file) = regular(&dir, name) else { continue };
        let Some(stamp) = revision(&file) else { continue };
        if rec.agy_answered_artifacts.get(name) == Some(&stamp) {
            continue;
        }
        let reason = if name == PLAN { Reason::Plan } else { Reason::Question };
        if rec.agy_reviewing
            && rec.agy_plan_request.as_deref().and_then(parts)
                == Some((lead.as_str(), stamp.as_str(), name.as_str()))
        {
            result.signal = Some(Signal::Attention { reason });
            return;
        }
        let Some(text) = read_note(&file) else { continue };
        if revision(&file).as_ref() != Some(&stamp) {
            continue;
        }
        let nonce = uuid::Uuid::new_v4();
        rec.agy_plan_request = Some(format!("agy-plan/{lead}/{nonce}:{stamp}/{name}"));
        rec.agy_reviewing = true;
        result.metadata_changed = true;
        if name == PLAN {
            result.plan = Some(text);
        } else {
            result.review_artifact = Some(text);
        }
        result.signal = Some(Signal::Attention { reason });
        return;
    }
    rec.agy_reviewing = false;
    result.metadata_changed |= rec.agy_plan_request.take().is_some();
}
