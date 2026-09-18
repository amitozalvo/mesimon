//! Evidence for a human-authorized recovery after unverified process cleanup.
//! These checks exclude known native owners, not escaped descendants. The
//! caller owns authorization, pane absence, and acknowledgment of that risk.

use std::io::{Read, Seek, SeekFrom};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use mesimon_core::board::{SessionKind, SessionRecord};

use super::{RuntimeConfig, Snapshot};
use crate::paths::Paths;

const ARTIFACT_LIMIT: u64 = 256 * 1024;

fn regular_file(path: &Path) -> std::io::Result<std::fs::File> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other("Recovery evidence is not a regular file"));
    }
    Ok(file)
}

/// `read_bounded`, except that a file which is not there answers `None`
/// rather than refusing. Every other failure is still a refusal: an
/// unreadable artefact is lost evidence, and lost evidence never means done.
fn read_bounded_optional(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match regular_file(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        _ => read_bounded(path).map(Some),
    }
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    regular_file(path)
        .and_then(|file| file.take(ARTIFACT_LIMIT + 1).read_to_end(&mut bytes))
        .map_err(|error| format!("Cannot inspect Codex recovery evidence: {error}"))?;
    if bytes.len() as u64 > ARTIFACT_LIMIT {
        return Err("Codex recovery evidence exceeds its inspection bound".into());
    }
    Ok(bytes)
}

fn evidence(
    paths: &Paths,
    record: &SessionRecord,
) -> Result<(PathBuf, RuntimeConfig, Snapshot), String> {
    let config_path = paths.hooks_dir().join(format!("{}.codex.json", record.id));
    let config: RuntimeConfig = serde_json::from_slice(&read_bounded(&config_path)?)
        .map_err(|_| "Invalid Codex recovery configuration")?;
    let snapshot: Snapshot =
        serde_json::from_slice(&read_bounded(&super::snapshot_path(paths, record.id))?)
            .map_err(|_| "Invalid Codex recovery snapshot")?;
    validate_evidence(paths, record, &config, Some(&snapshot))?;
    Ok((config_path, config, snapshot))
}

/// The same evidence, minus a snapshot that is simply GONE (T-405). The
/// snapshot, the two endpoints and their `.app-server.log` all live in the
/// runtime dir under `/tmp`, which a reboot or a tmp sweep empties wholesale.
/// Its absence therefore proves MORE absence, not less: nothing can listen on
/// a socket that is not there. What the ownership rung actually needs is the
/// config — validated against the record, as always, which is what makes its
/// endpoint paths this session's — and every other rung below still runs.
/// A snapshot that is present is still read and still must match; only
/// `NotFound` is forgiven, because only `NotFound` is evidence. The callers
/// that consume the snapshot's CONTENT (`recovery_launch_target`,
/// `retain_unverified_cleanup`) keep asking for the whole of it.
fn ownership_evidence(
    paths: &Paths,
    record: &SessionRecord,
) -> Result<(PathBuf, RuntimeConfig), String> {
    let config_path = paths.hooks_dir().join(format!("{}.codex.json", record.id));
    let config: RuntimeConfig = serde_json::from_slice(&read_bounded(&config_path)?)
        .map_err(|_| "Invalid Codex recovery configuration")?;
    let snapshot: Option<Snapshot> =
        match read_bounded_optional(&super::snapshot_path(paths, record.id))? {
            Some(bytes) => Some(
                serde_json::from_slice(&bytes).map_err(|_| "Invalid Codex recovery snapshot")?,
            ),
            None => None,
        };
    validate_evidence(paths, record, &config, snapshot.as_ref())?;
    Ok((config_path, config))
}

fn validate_evidence(
    paths: &Paths,
    record: &SessionRecord,
    config: &RuntimeConfig,
    snapshot: Option<&Snapshot>,
) -> Result<(), String> {
    let generation = record.codex_generation.ok_or("Codex recovery has no recorded generation")?;
    let stem = format!("cdx-{}-{:08x}", &record.id.simple().to_string()[..16], generation as u32);
    if record.kind != SessionKind::Codex
        || generation == 0
        || config.session != record.id
        || snapshot.is_some_and(|s| s.session != record.id)
        || config.generation != generation
        || snapshot.is_some_and(|s| s.generation != generation)
        || config.snapshot_path != super::snapshot_path(paths, record.id)
        || config.preview_path != super::preview_path(paths, record.id)
        || config.upstream_socket != paths.rt_dir.join(format!("{stem}-up.sock"))
        || config.proxy_socket != paths.rt_dir.join(format!("{stem}-ui.sock"))
    {
        return Err("Codex recovery evidence does not match the original session generation".into());
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryLaunchTarget {
    Exact(String),
    RetryStartup,
}

/// Native evidence decides whether there is an exact conversation to resume.
/// Only a positively recorded preselection launch can retry an empty startup.
pub fn recovery_launch_target(
    paths: &Paths,
    record: &SessionRecord,
) -> Result<RecoveryLaunchTarget, String> {
    let (_, config, snapshot) = evidence(paths, record)?;
    launch_target(record, &config, &snapshot)
}

fn launch_target(
    record: &SessionRecord,
    config: &RuntimeConfig,
    snapshot: &Snapshot,
) -> Result<RecoveryLaunchTarget, String> {
    if let Some(identity) = snapshot
        .thread_id
        .as_ref()
        .or(record.codex_thread_id.as_ref())
        .or(config.resume.as_ref())
        .filter(|id| !id.is_empty())
    {
        return Ok(RecoveryLaunchTarget::Exact(identity.clone()));
    }
    if snapshot.launch_phase == super::LaunchPhase::BeforeSelection
        && config.resume.is_none()
        && record.codex_thread_id.is_none()
        && snapshot.thread_id.is_none()
        && snapshot.turn_id.is_none()
        && snapshot.history_path.is_none()
        && record.transcript_path.is_none()
    {
        return Ok(RecoveryLaunchTarget::RetryStartup);
    }
    Err("Codex conversation identity is unavailable and startup may have created history; refusing a fresh conversation".into())
}

/// Excludes listeners without waiting on a full Unix socket accept backlog.
pub fn recovery_endpoint_absent(path: &Path) -> Result<(), String> {
    let bytes = path.as_os_str().as_bytes();
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.contains(&0) || bytes.len() >= address.sun_path.len() {
        return Err("Codex recovery endpoint cannot be inspected".into());
    }
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    #[cfg(target_os = "macos")]
    {
        address.sun_len = std::mem::size_of_val(&address) as u8;
    }
    for (target, source) in address.sun_path.iter_mut().zip(bytes) {
        *target = *source as libc::c_char;
    }
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err("Codex recovery cannot inspect endpoint listeners".into());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) } < 0 {
        return Err("Codex recovery cannot bound endpoint inspection".into());
    }
    let result = unsafe {
        libc::connect(
            fd.as_raw_fd(),
            (&address as *const libc::sockaddr_un).cast(),
            std::mem::size_of_val(&address) as libc::socklen_t,
        )
    };
    if result < 0
        && matches!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ENOENT | libc::ECONNREFUSED)
        )
    {
        Ok(())
    } else {
        Err(format!(
            "An old Codex endpoint is still live or its absence cannot be verified: {}",
            if result == 0 {
                "connected".into()
            } else {
                std::io::Error::last_os_error().to_string()
            }
        ))
    }
}

fn process_inventory() -> Result<String, String> {
    let mut child = Command::new("ps")
        .args(["-ww", "-axo", "pid=,uid=,stat=,args="])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Codex recovery cannot inspect process arguments")?;
    // Piped stdout is guaranteed by this spawn configuration.
    let mut stdout = child.stdout.take().expect("piped process inventory stdout");
    let fd = stdout.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    let mut success = false;
    let mut bytes = Vec::new();
    if flags >= 0 && unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } >= 0 {
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut chunk = [0; 8192];
        loop {
            match stdout.read(&mut chunk) {
                Ok(0) => match child.try_wait() {
                    Ok(Some(status)) => {
                        success = status.success();
                        break;
                    }
                    Ok(None) => {}
                    Err(_) => break,
                },
                Ok(size) => bytes.extend_from_slice(&chunk[..size]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => break,
            }
            if bytes.len() > 8 * 1024 * 1024 || Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    if !success {
        let _ = child.kill();
    }
    let _ = child.wait();
    if !success {
        return Err("Codex recovery process inspection failed or exceeded its bound".into());
    }
    String::from_utf8(bytes).map_err(|_| "Codex recovery process arguments are unreadable".into())
}

fn owners_absent(
    inventory: &str,
    needles: &[String],
    current_pid: u32,
    current_uid: u32,
) -> Result<(), String> {
    let mut saw_self = false;
    for line in inventory.lines().filter(|line| !line.trim().is_empty()) {
        let mut rest = line.trim();
        let mut field = || {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            let result = &rest[..end];
            rest = rest[end..].trim_start();
            result
        };
        let pid = field().parse::<u32>().map_err(|_| "Incomplete Codex process inventory")?;
        let uid = field().parse::<u32>().map_err(|_| "Incomplete Codex process inventory")?;
        let status = field();
        if status.is_empty() {
            return Err("Incomplete Codex process inventory".into());
        }
        saw_self |= pid == current_pid && uid == current_uid;
        if status.starts_with('Z') {
            continue;
        }
        if uid == current_uid && (rest.is_empty() || rest == "-" || rest == "?") {
            return Err(
                "A same-user process has unavailable arguments; Codex ownership is uncertain"
                    .into(),
            );
        }
        // Conservative substring matching accepts false positives rather than
        // assuming shell quoting or whitespace cannot be part of a path.
        if needles.iter().any(|needle| rest.contains(needle)) {
            return Err(format!("Old Codex native owner process {pid} is still present"));
        }
    }
    if !saw_self {
        return Err("Codex process inventory is incomplete".into());
    }
    Ok(())
}

/// Does not assert that unknown escaped descendants have stopped, and does not
/// modify the snapshot. Only an explicitly authorized caller may acknowledge
/// that separate uncertainty after this known-owner check succeeds.
pub fn recovery_owner_absent(paths: &Paths, record: &SessionRecord) -> Result<(), String> {
    let (config_path, config) = ownership_evidence(paths, record)?;
    let endpoints = [&config.proxy_socket, &config.upstream_socket];
    for endpoint in endpoints {
        recovery_endpoint_absent(endpoint)?;
    }
    let needles = [
        config_path.to_str().ok_or("Unreadable Codex configuration path")?.to_owned(),
        format!("unix://{}", config.proxy_socket.display()),
        format!("unix://{}", config.upstream_socket.display()),
    ];
    owners_absent(&process_inventory()?, &needles, std::process::id(), unsafe { libc::getuid() })?;
    for endpoint in endpoints {
        recovery_endpoint_absent(endpoint)?;
    }
    // A concurrent replacement must not inherit the older check's result.
    ownership_evidence(paths, record)?;
    Ok(())
}

/// Retains original provider evidence before a new launch replaces it. A new
/// archive is always used so repeated failed recovery attempts remain legible.
pub fn retain_unverified_cleanup(paths: &Paths, record: &SessionRecord) -> Result<PathBuf, String> {
    let (config_path, config, snapshot) = evidence(paths, record)?;
    let log_path = config.snapshot_path.with_extension("app-server.log");
    let log_tail = match regular_file(&log_path) {
        Ok(mut file) => {
            let length = file.metadata().map_err(|e| e.to_string())?.len();
            file.seek(SeekFrom::Start(length.saturating_sub(64 * 1024)))
                .map_err(|e| e.to_string())?;
            let mut bytes = Vec::new();
            file.take(64 * 1024).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
            Some(String::from_utf8_lossy(&bytes).into_owned())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("Cannot retain Codex recovery diagnostic: {error}")),
    };
    let path = paths.hooks_dir().join(format!(
        "{}.cleanup-{:016x}-{}.json",
        record.id,
        config.generation,
        uuid::Uuid::new_v4().simple()
    ));
    super::write_json(
        &path,
        &serde_json::json!({
            "unknown_descendants_may_remain": true,
            "config_path": config_path, "config": config, "snapshot": snapshot,
            "runtime_log_path": log_path, "runtime_log_tail": log_tail
        }),
    )
    .map_err(|error| error.to_string())?;
    Ok(path)
}

/// Restores only the old launch configuration after the caller's attempted
/// spawn failed. Both the archived identity and the exact prepared generation
/// must still match; a newer launch must never be overwritten by rollback.
pub fn restore_unverified_cleanup(
    paths: &Paths,
    record: &SessionRecord,
    archive: &Path,
    prepared_generation: u64,
) -> Result<(), String> {
    let old_generation = record.codex_generation.ok_or("Missing original Codex generation")?;
    let prefix = format!("{}.cleanup-{old_generation:016x}-", record.id);
    let valid_name = archive
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix(&prefix))
        .and_then(|name| name.strip_suffix(".json"))
        .is_some_and(|suffix| uuid::Uuid::parse_str(suffix).is_ok());
    if archive.parent() != Some(paths.hooks_dir().as_path())
        || !valid_name
        || prepared_generation == old_generation
    {
        return Err("Codex recovery rollback evidence is not the original owned archive".into());
    }
    let mut bytes = Vec::new();
    regular_file(archive)
        .and_then(|file| file.take(1024 * 1024 + 1).read_to_end(&mut bytes))
        .map_err(|e| e.to_string())?;
    if bytes.len() > 1024 * 1024 {
        return Err("Codex recovery archive exceeds its inspection bound".into());
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let config: RuntimeConfig =
        serde_json::from_value(value["config"].clone()).map_err(|e| e.to_string())?;
    let snapshot: Snapshot =
        serde_json::from_value(value["snapshot"].clone()).map_err(|e| e.to_string())?;
    validate_evidence(paths, record, &config, Some(&snapshot))?;
    let config_path = paths.hooks_dir().join(format!("{}.codex.json", record.id));
    let prepared: RuntimeConfig =
        serde_json::from_slice(&read_bounded(&config_path)?).map_err(|e| e.to_string())?;
    let mut expected_record = record.clone();
    expected_record.codex_generation = Some(prepared_generation);
    let mut expected_snapshot = snapshot.clone();
    expected_snapshot.generation = prepared_generation;
    validate_evidence(paths, &expected_record, &prepared, Some(&expected_snapshot))?;
    // A child that actually started despite a reported spawn error is not a
    // failed preparation to roll back. Leave all evidence held for inspection.
    let current_snapshot: Snapshot =
        serde_json::from_slice(&read_bounded(&config.snapshot_path)?).map_err(|e| e.to_string())?;
    validate_evidence(paths, record, &config, Some(&current_snapshot))?;
    super::write_json(&config_path, &config).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_inventory_rejects_each_native_owner_without_exposing_arguments() {
        let needles = [
            "/owned/session.codex.json".into(),
            "unix:///owned/ui.sock".into(),
            "unix:///owned/up.sock".into(),
        ];
        let baseline = "10 501 S daemon\n11 0 S launchd\n";
        assert!(owners_absent(baseline, &needles, 10, 501).is_ok());
        for argument in &needles {
            let inventory = format!("{baseline}20 501 S helper --config {argument} secret\n");
            let error = owners_absent(&inventory, &needles, 10, 501).unwrap_err();
            assert!(error.contains("20"));
            assert!(!error.contains("secret"));
        }
        assert!(owners_absent(
            &format!("{baseline}20 501 Z helper {}", needles[0]),
            &needles,
            10,
            501
        )
        .is_ok());
        for inventory in [
            "",
            "11 0 S launchd",
            "10 501 S",
            "10 501 S daemon\nbad",
            "10 501 S daemon\n20 501 S ?",
        ] {
            assert!(owners_absent(inventory, &needles, 10, 501).is_err(), "{inventory}");
        }
    }
    #[test]
    fn startup_retry_requires_positive_preselection_and_never_substitutes_existing_history() {
        let (_, mut record, mut config, mut snapshot) = fixture_records(Path::new("/synthetic"));
        assert!(launch_target(&record, &config, &snapshot).is_err());
        let mut legacy = serde_json::to_value(&snapshot).unwrap();
        legacy.as_object_mut().unwrap().remove("launch_phase");
        snapshot = serde_json::from_value(legacy).unwrap();
        assert_eq!(snapshot.launch_phase, super::super::LaunchPhase::Unknown);
        assert!(launch_target(&record, &config, &snapshot).is_err());
        snapshot.launch_phase = super::super::LaunchPhase::BeforeSelection;
        assert_eq!(
            launch_target(&record, &config, &snapshot).unwrap(),
            RecoveryLaunchTarget::RetryStartup
        );
        snapshot.launch_phase = super::super::LaunchPhase::SelectionPending;
        assert!(launch_target(&record, &config, &snapshot).is_err());
        config.resume = Some("existing-conversation".into());
        assert_eq!(
            launch_target(&record, &config, &snapshot).unwrap(),
            RecoveryLaunchTarget::Exact("existing-conversation".into())
        );
        config.resume = None;
        record.codex_thread_id = Some("persisted-conversation".into());
        assert_eq!(
            launch_target(&record, &config, &snapshot).unwrap(),
            RecoveryLaunchTarget::Exact("persisted-conversation".into())
        );
        snapshot.thread_id = Some("newly-selected-conversation".into());
        assert_eq!(
            launch_target(&record, &config, &snapshot).unwrap(),
            RecoveryLaunchTarget::Exact("newly-selected-conversation".into())
        );
        snapshot.thread_id = None;
        record.codex_thread_id = None;
        snapshot.launch_phase = super::super::LaunchPhase::BeforeSelection;
        snapshot.history_path = Some("/synthetic/existing.jsonl".into());
        assert!(launch_target(&record, &config, &snapshot).is_err());
    }

    fn fixture_records(root: &Path) -> (Paths, SessionRecord, RuntimeConfig, Snapshot) {
        use mesimon_core::board::SessionState;
        let paths = Paths {
            repo_root: "/repo".into(),
            proj16: "fixture".into(),
            rt_dir: root.join("runtime"),
            state_dir: root.join("state"),
            board_dir: "/repo/.mesimon".into(),
        };
        let id = uuid::Uuid::new_v4();
        let mut record = SessionRecord::new(
            id,
            SessionKind::Codex,
            ulid::Ulid::new(),
            vec![],
            "/repo".into(),
            SessionState::Sleeping,
        );
        record.codex_generation = Some(5);
        let stem = format!("cdx-{}-00000005", &id.simple().to_string()[..16]);
        let config = RuntimeConfig {
            session: id,
            generation: 5,
            cwd: "/repo".into(),
            executable: "codex".into(),
            upstream_socket: paths.rt_dir.join(format!("{stem}-up.sock")),
            proxy_socket: paths.rt_dir.join(format!("{stem}-ui.sock")),
            snapshot_path: super::super::snapshot_path(&paths, id),
            preview_path: super::super::preview_path(&paths, id),
            resume: None,
            config_flags: vec![],
            env: vec![],
        };
        let snapshot = Snapshot {
            launch_phase: super::super::LaunchPhase::Unknown,
            session: id,
            generation: 5,
            sequence: 1,
            heartbeat_ms: 0,
            thread_id: None,
            turn_id: None,
            state: SessionState::Sleeping,
            observation_hold: true,
            history_path: None,
            title: None,
            plan: None,
            plan_key: None,
            stopped: false,
        };
        (paths, record, config, snapshot)
    }

    #[test]
    fn recovery_requires_exact_original_generation_and_both_owned_endpoints() {
        let (paths, mut record, config, snapshot) = fixture_records(Path::new("/owned"));
        assert!(validate_evidence(&paths, &record, &config, Some(&snapshot)).is_ok());
        let mut bad = config.clone();
        bad.generation = 6;
        assert!(validate_evidence(&paths, &record, &bad, Some(&snapshot)).is_err());
        let mut bad = config.clone();
        bad.upstream_socket = "/elsewhere/up.sock".into();
        assert!(validate_evidence(&paths, &record, &bad, Some(&snapshot)).is_err());
        let mut bad = config.clone();
        bad.proxy_socket = "/elsewhere/ui.sock".into();
        assert!(validate_evidence(&paths, &record, &bad, Some(&snapshot)).is_err());
        let mut bad = snapshot.clone();
        bad.session = uuid::Uuid::new_v4();
        assert!(validate_evidence(&paths, &record, &config, Some(&bad)).is_err());
        let mut bad = snapshot.clone();
        bad.generation = 6;
        assert!(validate_evidence(&paths, &record, &config, Some(&bad)).is_err());
        record.codex_generation = None;
        assert!(validate_evidence(&paths, &record, &config, Some(&snapshot)).is_err());
    }

    /// T-405: a runtime dir emptied by a reboot or a tmp sweep takes the
    /// snapshot with it. The config still says which endpoints are this
    /// session's, and every other rung still runs, so a MISSING snapshot is
    /// forgiven where a wrong one is not — and the record's own generation
    /// is still what the config must match.
    #[test]
    fn a_missing_snapshot_is_forgiven_where_a_wrong_one_is_not() {
        let (paths, mut record, config, snapshot) = fixture_records(Path::new("/owned"));
        assert!(validate_evidence(&paths, &record, &config, None).is_ok());
        let mut wrong = snapshot.clone();
        wrong.generation = 6;
        assert!(validate_evidence(&paths, &record, &config, Some(&wrong)).is_err());
        let mut bad = config.clone();
        bad.generation = 6;
        assert!(validate_evidence(&paths, &record, &bad, None).is_err());
        record.codex_generation = None;
        assert!(validate_evidence(&paths, &record, &config, None).is_err());
    }
    #[test]
    fn endpoint_probe_refuses_live_listener_and_accepts_stale_socket() {
        struct Fixture(PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let fixture = Fixture(PathBuf::from(format!(
            "/tmp/msmn-cdx-recovery-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..12]
        )));
        std::fs::create_dir(&fixture.0).unwrap();
        let socket = fixture.0.join("native.sock");
        assert!(recovery_endpoint_absent(&socket).is_ok());
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        assert!(recovery_endpoint_absent(&socket).is_err());
        drop(listener);
        // Closing a listener with the probe queued can leave a transient
        // reset on macOS. A transient refusal is safe; retry only in the test.
        let deadline = Instant::now() + Duration::from_millis(250);
        loop {
            let result = recovery_endpoint_absent(&socket);
            if result.is_ok() {
                break;
            }
            assert!(Instant::now() < deadline, "stale socket: {result:?}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn actual_process_inventory_proves_self_visible_and_rejects_matching_owner() {
        let inventory = process_inventory().unwrap();
        let uid = unsafe { libc::getuid() };
        assert!(owners_absent(
            &inventory,
            &[format!("absent-{}", uuid::Uuid::new_v4())],
            std::process::id(),
            uid
        )
        .is_ok());
        let executable = mesimon_core::exe::current_exe().unwrap();
        let name = executable.file_name().unwrap().to_str().unwrap().to_owned();
        assert!(owners_absent(&inventory, &[name], std::process::id(), uid).is_err());
    }
    #[test]
    fn failed_spawn_rollback_preserves_unknown_snapshot_and_compares_prepared_generation() {
        use std::os::unix::fs::PermissionsExt;
        struct Fixture(PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let fixture = Fixture(
            std::env::temp_dir().join(format!("msmn-cdx-rollback-{}", uuid::Uuid::new_v4())),
        );
        let (paths, record, config, snapshot) = fixture_records(&fixture.0);
        let config_path = paths.hooks_dir().join(format!("{}.codex.json", record.id));
        super::super::write_json(&config_path, &config).unwrap();
        super::super::write_json(&config.snapshot_path, &snapshot).unwrap();
        let original_snapshot = std::fs::read(&config.snapshot_path).unwrap();
        let archive = retain_unverified_cleanup(&paths, &record).unwrap();
        assert_eq!(std::fs::metadata(&archive).unwrap().permissions().mode() & 0o777, 0o600);
        let archived: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&archive).unwrap()).unwrap();
        assert_eq!(archived["unknown_descendants_may_remain"], true);
        assert_eq!(archived["snapshot"]["stopped"], false);
        let mut prepared = config.clone();
        prepared.generation = 6;
        prepared.proxy_socket = PathBuf::from(
            config.proxy_socket.to_str().unwrap().replace("00000005-ui", "00000006-ui"),
        );
        prepared.upstream_socket = PathBuf::from(
            config.upstream_socket.to_str().unwrap().replace("00000005-up", "00000006-up"),
        );
        super::super::write_json(&config_path, &prepared).unwrap();
        let prepared_bytes = std::fs::read(&config_path).unwrap();
        assert!(restore_unverified_cleanup(&paths, &record, &archive, 7).is_err());
        assert_eq!(std::fs::read(&config_path).unwrap(), prepared_bytes);
        restore_unverified_cleanup(&paths, &record, &archive, 6).unwrap();
        let restored: RuntimeConfig =
            serde_json::from_slice(&std::fs::read(&config_path).unwrap()).unwrap();
        assert_eq!(restored.generation, 5);
        assert_eq!(std::fs::read(&config.snapshot_path).unwrap(), original_snapshot);
        super::super::write_json(&config_path, &prepared).unwrap();
        let mut started = snapshot.clone();
        started.generation = 6;
        super::super::write_json(&config.snapshot_path, &started).unwrap();
        assert!(restore_unverified_cleanup(&paths, &record, &archive, 6).is_err());
        assert_eq!(std::fs::read(&config_path).unwrap(), prepared_bytes);
    }
}
