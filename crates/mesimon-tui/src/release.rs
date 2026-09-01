//! The release checker: does a newer mesimon exist, and would you like it?
//!
//! This module stops one step short of the thing it is named after. It asks
//! the dist repo whether a newer tag is published, and — only when the user
//! takes the offer — downloads it, verifies the published checksum, proves the
//! binary runs, and swaps it in at our own path. It never restarts anything.
//! That is `update.rs`'s job and always was: the swap changes our own exe's
//! mtime, the watch already looking at that mtime raises `update ready`, and
//! `U` is still the only thing that restarts a board. Two halves that already
//! existed, joined by a file move.
//!
//! **A development build never gets here.** The channel is stamped at build
//! time by `build.rs` and reads `release` only when `ci/release.sh` set
//! `MESIMON_RELEASE`, so a `cargo run` board makes no request, writes no
//! stamp, and cannot have its `target/` binary replaced by a download. On top
//! of that the exe path is checked for a `target` component, which is the
//! belt to that brace and is lifted by nothing — `MESIMON_UPDATE_CHECK=1`
//! forces a dev build past the CHANNEL gate only, so exercising the real path
//! means copying the binary out of the build tree first, which is what an
//! install is anyway.
//!
//! `MESIMON_NO_UPDATE_CHECK=1` is the opt-out for a shipped build, and it is
//! read before anything else: no stamp, no thread, no request.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use semver::Version;
use sha2::{Digest, Sha256};

/// Where the binaries are published. The source repo is private; this one
/// carries releases and nothing else, which is why the check needs no token
/// and no GitHub account (README, "Install").
const DIST_REPO: &str = "amitozalvo/mesimon-releases";

/// Six hours between questions. An alpha moves in days, not minutes, and the
/// stamp is shared by every board on this machine — so this is the rate for
/// the user, not for the process.
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// A request that did not answer is retried sooner than the full cadence, but
/// not soon enough to be a poll: a laptop that opened a board on a plane
/// should notice the network came back the same afternoon.
const RETRY_AFTER: Duration = Duration::from_secs(30 * 60);
/// Never during the first paint. Startup already spawns a daemon, probes the
/// terminal twice and loads a board; the least urgent thing mesimon does can
/// wait three seconds.
const FIRST_CHECK_AFTER: Duration = Duration::from_secs(3);
const POLL_EVERY: Duration = Duration::from_secs(1);

const API_TIMEOUT_SECS: &str = "10";
const DOWNLOAD_TIMEOUT_SECS: &str = "180";

const STAMP_SCHEMA: u64 = 1;

/// The one target the release channel publishes. `None` anywhere else, which
/// makes the whole module inert rather than making it guess at an asset name
/// that was never uploaded.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const TARGET: Option<&str> = Some("aarch64-apple-darwin");
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
const TARGET: Option<&str> = None;

/// What the offer is doing. `Quiet` is both "nothing has answered yet" and
/// "we are current" on purpose — neither of those is news, and a board that
/// said "up to date" in its header would be spending the one chip slot on the
/// absence of a thing.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Stage {
    Quiet,
    /// A newer tag is published and taking the offer would fetch it.
    Available(String),
    /// The fetch is running. The offer comes DOWN while it does — the same
    /// discipline the shell-env reload follows, so a slow download never
    /// leaves a chip standing as if the press had missed.
    Installing(String),
    /// It landed at our own path. From here `update.rs` owns the story.
    Installed(String),
}

/// One worker's whole result. Every worker sends exactly one of these, which
/// is what lets `working` be a plain bool.
enum Outcome {
    /// The newest published tag, or `None` when the request did not answer.
    Latest(Option<String>),
    Installed(String),
    Failed(String),
}

/// Everything the checker needs, resolved once. Its absence IS the "this
/// build never checks" answer, so there is no second flag to keep in step.
struct Eligible {
    /// The binary to replace — our own path, the one `update.rs` watches and
    /// the one `install.sh` writes.
    exe: PathBuf,
    /// The shared stamp — see [`stamp_path`].
    stamp: PathBuf,
    /// Where a download is unpacked, inside the 0700 runtime dir. A
    /// half-fetched tarball is not state and a reboot should take it.
    stage_dir: PathBuf,
}

pub struct ReleaseWatch {
    eligible: Option<Eligible>,
    stage: Stage,
    /// One line for the status bar, taken once by the app.
    note: Option<String>,
    next_check: Option<Instant>,
    last_poll: Instant,
    /// A worker thread is out. Cleared by its one `Outcome`.
    working: bool,
    tx: Sender<Outcome>,
    rx: Receiver<Outcome>,
}

impl ReleaseWatch {
    pub fn new(repo_root: &Path) -> Self {
        let (tx, rx) = mpsc::channel();
        let mut w = Self {
            eligible: None,
            stage: Stage::Quiet,
            note: None,
            next_check: None,
            last_poll: Instant::now(),
            working: false,
            tx,
            rx,
        };
        let Some(eligible) = eligibility(repo_root) else {
            return w;
        };
        // The cached answer is why a second board opened this afternoon shows
        // the offer without a second request.
        let stamp = read_stamp(&eligible.stamp);
        if let Some(tag) = stamp.as_ref().and_then(|s| s.latest.clone()) {
            if is_newer(&tag) {
                w.stage = Stage::Available(tag);
            }
        }
        let due_in = stamp
            .as_ref()
            .map(|s| CHECK_EVERY.saturating_sub(s.age()))
            .unwrap_or(Duration::ZERO)
            .max(FIRST_CHECK_AFTER);
        w.next_check = Some(Instant::now() + due_in);
        w.eligible = Some(eligible);
        w
    }

    /// Rate-limited poll. True when something on screen would change.
    pub fn tick(&mut self) -> bool {
        let mut dirty = false;
        // Disconnected cannot happen — we hold the sender for the life of the
        // watch — so draining until empty drains everything.
        while let Ok(o) = self.rx.try_recv() {
            self.absorb(o);
            dirty = true;
        }
        // Never while a download is out: a check cannot overrule it anyway
        // (see `absorb`), and not asking is cheaper than reasoning about it.
        if self.eligible.is_none()
            || self.working
            || matches!(self.stage, Stage::Installing(_))
            || self.last_poll.elapsed() < POLL_EVERY
        {
            return dirty;
        }
        self.last_poll = Instant::now();
        if self.next_check.is_some_and(|t| Instant::now() >= t) {
            self.next_check = Some(Instant::now() + CHECK_EVERY);
            self.spawn(|| Outcome::Latest(fetch_latest_tag()));
        }
        dirty
    }

    /// Take the offer: fetch, verify, and put the new binary at our own path.
    /// Returns the tag it started on, or `None` when there is nothing to take
    /// — which the menu row's own `avail` already rules out.
    pub fn begin_install(&mut self) -> Option<String> {
        let Stage::Available(tag) = &self.stage else {
            return None;
        };
        let tag = tag.clone();
        let e = self.eligible.as_ref()?;
        let (exe, stage_dir) = (e.exe.clone(), e.stage_dir.clone());
        self.stage = Stage::Installing(tag.clone());
        let t = tag.clone();
        self.spawn(move || match install(&t, &exe, &stage_dir) {
            Ok(()) => Outcome::Installed(t),
            Err(e) => Outcome::Failed(e),
        });
        Some(tag)
    }

    /// A newer release is published and nothing is in flight.
    pub fn available(&self) -> bool {
        matches!(self.stage, Stage::Available(_))
    }

    /// The tag the offer names. Empty when there is no offer — the menu row
    /// and the chip both refuse to render on an empty one.
    pub fn tag(&self) -> String {
        match &self.stage {
            Stage::Available(t) | Stage::Installing(t) | Stage::Installed(t) => t.clone(),
            Stage::Quiet => String::new(),
        }
    }

    /// One line for the status bar, said once.
    pub fn take_note(&mut self) -> Option<String> {
        self.note.take()
    }

    fn spawn(&mut self, work: impl FnOnce() -> Outcome + Send + 'static) {
        let tx = self.tx.clone();
        self.working = true;
        std::thread::spawn(move || {
            let _ = tx.send(work());
        });
    }

    fn absorb(&mut self, o: Outcome) {
        self.working = false;
        match o {
            Outcome::Latest(Some(tag)) => {
                if let Some(e) = &self.eligible {
                    write_stamp(&e.stamp, &tag);
                }
                // A check never overrules a download in flight or one that
                // landed: those are further along the same story.
                if matches!(self.stage, Stage::Quiet | Stage::Available(_)) {
                    self.stage = if is_newer(&tag) { Stage::Available(tag) } else { Stage::Quiet };
                }
            }
            // The request did not answer. The stamp is deliberately NOT
            // written: it records when we last HEARD, so a week offline must
            // not read back as a week of successful checks.
            Outcome::Latest(None) => self.next_check = Some(Instant::now() + RETRY_AFTER),
            Outcome::Installed(tag) => {
                self.note = Some(format!("{tag} is installed ∙ U restarts on it"));
                self.stage = Stage::Installed(tag);
            }
            Outcome::Failed(msg) => {
                self.note = Some(format!("update failed: {msg}"));
                // The offer comes back. A download that fell over is a reason
                // to try again, not a reason to stop saying a newer build is
                // out there — and `install.sh` is still the other way in.
                if let Stage::Installing(tag) = std::mem::replace(&mut self.stage, Stage::Quiet) {
                    self.stage = Stage::Available(tag);
                }
            }
        }
    }

    /// Render tests need the offer without a network and without a release
    /// build to make it in.
    #[cfg(test)]
    pub(crate) fn force_available(&mut self, tag: &str) {
        self.stage = Stage::Available(tag.to_string());
    }
}

// ---- eligibility -----------------------------------------------------------

fn eligibility(repo_root: &Path) -> Option<Eligible> {
    if why_off().is_some() {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let paths = mesimon_daemon::Paths::for_repo(repo_root).ok()?;
    Some(Eligible { exe, stamp: stamp_path()?, stage_dir: paths.rt_dir.join("update") })
}

/// Why this build does not check, or `None` when it does. One function, so
/// the doctor line and the checker itself cannot disagree about the answer —
/// and so a reason that is missing from the diagnosis is a reason that cannot
/// exist.
fn why_off() -> Option<&'static str> {
    if std::env::var_os("MESIMON_NO_UPDATE_CHECK").is_some() {
        return Some("MESIMON_NO_UPDATE_CHECK is set");
    }
    if !checking_channel() {
        return Some("this build was not cut by ci/release.sh");
    }
    if TARGET.is_none() {
        return Some("no release is published for this platform");
    }
    // The one guard nothing lifts. A binary still inside a build tree is a
    // development build wearing whatever stamp it was given, and replacing it
    // with a download would destroy work — so the answer here is no, even
    // when the channel gate was forced open on purpose.
    if std::env::current_exe().is_ok_and(|e| in_build_tree(&e)) {
        return Some("this binary is inside a build tree");
    }
    if stamp_path().is_none() {
        return Some("HOME is unset");
    }
    None
}

/// `~/.local/state/mesimon/update-check.json` — at the state ROOT, not under a
/// project key. The binary is one per machine, so asking once per repo would
/// be asking the same question N times; keying it off `HOME` rather than
/// `Paths` is also what lets `mesimon doctor` answer without a repo.
fn stamp_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".local/state/mesimon/update-check.json"))
}

/// The `mesimon doctor` line. A checker that quietly does nothing is
/// indistinguishable from one that is broken, so this is where the reason is
/// spelled — including the reasons that are the point (a dev build).
pub fn doctor_line() -> String {
    if let Some(why) = why_off() {
        return format!("off ∙ {why}");
    }
    let Some(stamp) = stamp_path().and_then(|p| read_stamp(&p)) else {
        return format!("on ∙ never asked ∙ {DIST_REPO}");
    };
    let ago = ago(stamp.age());
    match stamp.latest {
        Some(t) if is_newer(&t) => format!("on ∙ {t} is out ∙ asked {ago}"),
        Some(t) => format!("on ∙ {t} is the newest ∙ asked {ago}"),
        None => format!("on ∙ asked {ago}"),
    }
}

fn ago(d: Duration) -> String {
    let m = d.as_secs() / 60;
    match m {
        0 => "just now".into(),
        1..=59 => format!("{m}m ago"),
        60..=1439 => format!("{}h ago", m / 60),
        _ => format!("{}d ago", m / 1440),
    }
}

/// Does this build ask at all? The stamp is `build.rs`'s, and it says
/// `release` only for the build `ci/release.sh` publishes.
fn checking_channel() -> bool {
    env!("MESIMON_CHANNEL") == "release" || std::env::var_os("MESIMON_UPDATE_CHECK").is_some()
}

fn in_build_tree(exe: &Path) -> bool {
    exe.components().any(|c| c.as_os_str() == "target")
}

fn is_newer(tag: &str) -> bool {
    newer_than(tag, env!("CARGO_PKG_VERSION"))
}

/// Strictly newer than what is running, and nothing else.
///
/// Unparseable on either side orders nothing, so it offers nothing (D26 fails
/// closed), and equal is not an offer — which is also what keeps a deliberate
/// `install.sh --version` pin from being nagged back up, or down.
fn newer_than(tag: &str, ours: &str) -> bool {
    match (Version::parse(tag.trim_start_matches('v')), Version::parse(ours)) {
        (Ok(theirs), Ok(mine)) => theirs > mine,
        _ => false,
    }
}

// ---- the two requests ------------------------------------------------------

/// `curl`, not an HTTP client crate. mesimon makes exactly two GETs in its
/// whole life and both of them `install.sh` already makes; the alternative is
/// a TLS stack and its dependency tree linked into a binary that otherwise
/// touches no network at all. macOS ships curl, and the installer that put
/// this binary here required it.
fn curl(args: &[&str]) -> Option<std::process::Output> {
    let ua = format!("mesimon/{}", env!("CARGO_PKG_VERSION"));
    Command::new("curl")
        .args(["-fsSL", "--proto", "=https", "--proto-redir", "=https", "--tlsv1.2", "-A", &ua])
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
}

fn fetch_latest_tag() -> Option<String> {
    // NOT `/releases/latest` — that endpoint skips prereleases and every alpha
    // is one, so it answers 404 until the first stable build. The list
    // endpoint is newest-first and includes them. Same trap, same note, as
    // `install.sh`.
    let url = format!("https://api.github.com/repos/{DIST_REPO}/releases?per_page=5");
    let out =
        curl(&["--max-time", API_TIMEOUT_SECS, "-H", "Accept: application/vnd.github+json", &url])?;
    latest_tag(&String::from_utf8_lossy(&out.stdout))
}

/// The newest published tag in a GitHub releases listing. Drafts are skipped
/// belt-and-braces: an unauthenticated request is never shown one, but the
/// asset a draft names does not exist, so an offer built on it could only
/// fail.
fn latest_tag(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    v.as_array()?
        .iter()
        .find(|r| !r.get("draft").and_then(|d| d.as_bool()).unwrap_or(false))
        .and_then(|r| r.get("tag_name"))
        .and_then(|t| t.as_str())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

// ---- taking the offer ------------------------------------------------------

fn install(tag: &str, exe: &Path, stage_dir: &Path) -> Result<(), String> {
    let target = TARGET.ok_or("no published build for this platform")?;
    let name = format!("mesimon-{tag}-{target}");
    let asset = format!("{name}.tar.gz");
    let base = format!("https://github.com/{DIST_REPO}/releases/download/{tag}");

    let _ = std::fs::remove_dir_all(stage_dir);
    std::fs::create_dir_all(stage_dir).map_err(|e| format!("could not stage: {e}"))?;
    let tarball = stage_dir.join(&asset);

    curl(&[
        "--max-time",
        DOWNLOAD_TIMEOUT_SECS,
        "-o",
        &tarball.to_string_lossy(),
        &format!("{base}/{asset}"),
    ])
    .ok_or_else(|| format!("could not download {asset}"))?;

    // `install.sh` warns and carries on when no checksum is published, because
    // a person is watching it and can decide. Nothing is watching this, and
    // what it is about to overwrite is the binary you are running — so here
    // an absent checksum is a refusal.
    let sums = curl(&["--max-time", API_TIMEOUT_SECS, &format!("{base}/{asset}.sha256")])
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .ok_or("no checksum is published for that release")?;
    let want = checksum_field(&sums).ok_or("the published checksum could not be read")?;
    let got = sha256_file(&tarball)?;
    if !got.eq_ignore_ascii_case(want) {
        return Err("checksum mismatch — the download is not what the release names".into());
    }

    // The system tar, because `install.sh` unpacks the identical bytes with
    // the identical tool and the two must not be able to disagree.
    let ok = Command::new("tar")
        .arg("-xzf")
        .arg(&tarball)
        .arg("-C")
        .arg(stage_dir)
        .status()
        .map_err(|e| format!("tar: {e}"))?;
    if !ok.success() {
        return Err("the archive would not unpack".into());
    }

    let fresh = stage_dir.join(&name).join("mesimon");
    set_exec(&fresh)?;
    // Run it BEFORE it becomes our path. An arm64 binary with a broken
    // signature dies with "Killed: 9" and no explanation, and finding that
    // out here beats finding it out from a board that will not start.
    let v = Command::new(&fresh)
        .arg("--version")
        .output()
        .map_err(|e| format!("the downloaded binary would not run: {e}"))?;
    if !v.status.success() {
        return Err("the downloaded binary would not run".into());
    }

    // The bundled tmux rides along ONLY where the install already has one:
    // mesimon resolves a sibling `mesimon-tmux` ahead of PATH, so planting one
    // where the user never had it would silently change which tmux their
    // sessions run under.
    //
    // It lands BEFORE mesimon, so the two ways this can half-finish are both
    // benign. Failing here changes nothing at all. Failing on the line below
    // leaves a new tmux under an old mesimon — which still works, because that
    // mesimon uses the same sibling for the server AND the client, and the
    // version refusal is between a client and a server of different builds.
    // The other order would report a failure after replacing the binary the
    // user is running, and raise `update ready` in the same breath.
    let sibling = exe.with_file_name("mesimon-tmux");
    let fresh_tmux = stage_dir.join(&name).join("mesimon-tmux");
    if sibling.exists() && fresh_tmux.exists() {
        set_exec(&fresh_tmux)?;
        replace(&fresh_tmux, &sibling)?;
    }
    replace(&fresh, exe)?;
    let _ = std::fs::remove_dir_all(stage_dir);
    Ok(())
}

/// Land `src` on `dest` atomically. The temp file is deliberately in `dest`'s
/// own directory, so the rename is within one filesystem and cannot be a
/// copy-then-truncate — the invariant `install.sh` relies on too. Replacing
/// the directory entry of a RUNNING executable is safe on Unix: this process
/// keeps the inode it was started from, which is exactly why the restart is
/// still a separate, explicit act.
fn replace(src: &Path, dest: &Path) -> Result<(), String> {
    let tmp = dest.with_extension("new");
    std::fs::copy(src, &tmp).map_err(|e| format!("{} is not writable: {e}", dest.display()))?;
    set_exec(&tmp)?;
    std::fs::rename(&tmp, dest).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("could not replace {}: {e}", dest.display())
    })
}

fn set_exec(p: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("{}: {e}", p.display()))
}

/// `shasum -a 256` writes `<hex>  <name>`; the hash is the first field, and a
/// field that is not 64 hex digits is not a hash.
fn checksum_field(text: &str) -> Option<&str> {
    text.split_whitespace()
        .next()
        .filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn sha256_file(p: &Path) -> Result<String, String> {
    let bytes = std::fs::read(p).map_err(|e| format!("could not read the download: {e}"))?;
    let mut h = Sha256::new();
    h.update(&bytes);
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

// ---- the stamp -------------------------------------------------------------

/// When we last heard, and what we heard.
///
/// This is a CACHE and is treated as one: an unreadable file, or one written
/// by a newer build, is ignored and overwritten rather than quarantined. That
/// is the opposite of the four state files' rule and deliberately so — the
/// worst an ignored stamp costs is one extra HTTP request, where an ignored
/// `sessions.json` would cost the board.
struct Stamp {
    checked_at_ms: u64,
    latest: Option<String>,
}

impl Stamp {
    fn age(&self) -> Duration {
        Duration::from_millis(now_ms().saturating_sub(self.checked_at_ms))
    }
}

fn read_stamp(p: &Path) -> Option<Stamp> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()?;
    if v.get("schema_version").and_then(|s| s.as_u64()).unwrap_or(0) > STAMP_SCHEMA {
        return None;
    }
    Some(Stamp {
        checked_at_ms: v.get("checked_at_ms").and_then(|t| t.as_u64()).unwrap_or(0),
        latest: v.get("latest").and_then(|t| t.as_str()).map(str::to_string),
    })
}

fn write_stamp(p: &Path, latest: &str) {
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let body = serde_json::json!({
        "schema_version": STAMP_SCHEMA,
        "checked_at_ms": now_ms(),
        "latest": latest,
    });
    // Best effort throughout: a stamp that cannot be written costs one extra
    // request next time and nothing else, so it is never worth a notice.
    let _ = std::fs::write(p, body.to_string());
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Forward only. A tag that is older, equal, or unreadable is not an
    /// offer — the last of those is the D26 clause: an order we cannot
    /// establish is not an order we act on.
    #[test]
    fn only_a_strictly_newer_tag_is_an_offer() {
        assert!(newer_than("v0.1.0-alpha.5", "0.1.0-alpha.4"));
        assert!(newer_than("0.1.0-alpha.5", "0.1.0-alpha.4"), "the leading v is optional");
        assert!(newer_than("v0.1.0", "0.1.0-alpha.9"), "a release outranks its own prereleases");
        assert!(newer_than("v0.2.0", "0.1.0"));
        assert!(newer_than("v0.1.0-alpha.10", "0.1.0-alpha.9"), "numeric, not lexical");

        assert!(!newer_than("v0.1.0-alpha.4", "0.1.0-alpha.4"), "equal is not an offer");
        assert!(!newer_than("v0.1.0-alpha.3", "0.1.0-alpha.4"), "never a downgrade");
        assert!(!newer_than("v0.1.0-alpha.9", "0.1.0"), "nor back into a prerelease");
        assert!(!newer_than("nightly", "0.1.0"), "unorderable offers nothing");
        assert!(!newer_than("v0.1.0-alpha.5", "not-a-version"));
        assert!(!newer_than("", "0.1.0"));
    }

    #[test]
    fn the_newest_published_tag_is_the_first_non_draft() {
        let body = r#"[
            {"tag_name": "v0.1.0-alpha.9", "draft": true},
            {"tag_name": "v0.1.0-alpha.5", "draft": false, "prerelease": true},
            {"tag_name": "v0.1.0-alpha.4", "draft": false}
        ]"#;
        assert_eq!(latest_tag(body).as_deref(), Some("v0.1.0-alpha.5"));
        // A prerelease is the ONLY kind this project publishes, so the list
        // endpoint (not /releases/latest) is the one that can answer at all.
        assert_eq!(latest_tag(r#"[{"tag_name": "v9.9.9"}]"#).as_deref(), Some("v9.9.9"));
    }

    /// Nothing about a malformed answer is an offer. Rate-limit JSON, an HTML
    /// error page, an empty list and a nameless release all read the same.
    #[test]
    fn a_body_that_says_nothing_offers_nothing() {
        assert_eq!(latest_tag("[]"), None);
        assert_eq!(latest_tag(r#"{"message": "API rate limit exceeded"}"#), None);
        assert_eq!(latest_tag("<html>502</html>"), None);
        assert_eq!(latest_tag(r#"[{"draft": false}]"#), None);
        assert_eq!(latest_tag(r#"[{"tag_name": ""}]"#), None);
        assert_eq!(latest_tag(""), None);
    }

    #[test]
    fn the_checksum_is_the_first_field_and_must_look_like_one() {
        let hex = "a".repeat(64);
        assert_eq!(
            checksum_field(&format!("{hex}  mesimon-v0.1.0-aarch64-apple-darwin.tar.gz\n")),
            Some(hex.as_str())
        );
        assert_eq!(checksum_field(&hex), Some(hex.as_str()));
        // A 404 body, a short hash, a hash with a stray character: none of
        // these verify anything, and treating one as a hash would mean
        // comparing our real digest against nonsense and calling it a match
        // only by accident.
        assert_eq!(checksum_field("Not Found"), None);
        assert_eq!(checksum_field(&"a".repeat(63)), None);
        assert_eq!(checksum_field(&format!("{}z  x.tar.gz", "a".repeat(63))), None);
        assert_eq!(checksum_field(""), None);
    }

    /// The gate the whole module rests on: the test binary is a dev build, so
    /// the channel is closed and `eligibility` refuses before it can touch the
    /// network, the stamp, or a path.
    #[test]
    fn a_dev_build_is_never_eligible() {
        assert_eq!(env!("MESIMON_CHANNEL"), "dev", "a test binary is never a release build");
        assert!(!checking_channel());
        let w = ReleaseWatch::new(Path::new("."));
        assert!(w.eligible.is_none());
        assert!(!w.available());
        assert_eq!(w.tag(), "");
    }

    /// And the guard under the gate: a binary inside a build tree is refused
    /// whatever the channel says, because the alternative is a download
    /// landing on somebody's `cargo build` output.
    #[test]
    fn a_binary_in_a_build_tree_is_refused() {
        assert!(in_build_tree(Path::new("/Users/x/code/mesimon/target/debug/mesimon")));
        assert!(in_build_tree(Path::new(
            "/Users/x/code/mesimon/target/aarch64-apple-darwin/release/mesimon"
        )));
        assert!(!in_build_tree(Path::new("/Users/x/.local/bin/mesimon")));
        assert!(!in_build_tree(Path::new("/opt/homebrew/bin/mesimon")));
    }

    /// The download half, against the live release channel — the one part of
    /// this module no fixture can stand in for, and the part whose failure
    /// mode is "replaced the binary you are running with something else".
    /// Ignored by default (it needs the network and moves ~10 MB); run it
    /// deliberately after touching `install`:
    ///
    /// ```text
    /// cargo test -p mesimon-tui --lib -- --ignored the_download_half
    /// ```
    ///
    /// It installs the CURRENT version onto a scratch path, never over the
    /// real binary. What is proved is the sequence — asset naming, the
    /// checksum, the unpack, the `--version` gate and the atomic swap — not
    /// the ordering rule, which the unit tests above already hold. The
    /// scratch path has no `mesimon-tmux` beside it, so this also proves the
    /// sibling branch is skipped rather than planted.
    #[test]
    #[ignore = "needs the network and the live release channel"]
    fn the_download_half_works_against_the_real_release_channel() {
        let dir = std::env::temp_dir().join(format!("msmn-install-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("tmp");
        let exe = dir.join("mesimon");
        std::fs::write(&exe, b"#!/bin/sh\nexit 9\n").expect("placeholder");
        set_exec(&exe).expect("chmod");

        let tag = format!("v{}", env!("CARGO_PKG_VERSION"));
        install(&tag, &exe, &dir.join("stage")).expect("the published release installs");

        let out = Command::new(&exe).arg("--version").output().expect("run what landed");
        assert!(out.status.success(), "the installed binary would not run");
        let said = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(said.contains(env!("CARGO_PKG_VERSION")), "landed the wrong build: {said:?}");
        assert!(!exe.with_file_name("mesimon-tmux").exists(), "no sibling was planted");
        assert!(!dir.join("stage").exists(), "the staging dir is cleaned up");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_stamp_round_trips_and_a_newer_one_is_ignored() {
        let dir = std::env::temp_dir().join(format!("msmn-stamp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("tmp");
        let p = dir.join("update-check.json");

        write_stamp(&p, "v0.1.0-alpha.5");
        let s = read_stamp(&p).expect("just written");
        assert_eq!(s.latest.as_deref(), Some("v0.1.0-alpha.5"));
        assert!(s.age() < Duration::from_secs(5), "a fresh stamp is not due");

        // A file from a build that knows more than we do is not guessed at.
        std::fs::write(&p, r#"{"schema_version": 99, "latest": "v9.9.9"}"#).expect("write");
        assert!(read_stamp(&p).is_none());
        // Neither is a corrupt one — and neither costs more than one request.
        std::fs::write(&p, "{not json").expect("write");
        assert!(read_stamp(&p).is_none());
        assert!(read_stamp(&dir.join("nothing-here.json")).is_none());

        // An age with no timestamp reads as ancient, which is due — never as
        // fresh, which would be a check that silently stopped happening.
        std::fs::write(&p, r#"{"schema_version": 1, "latest": "v1.0.0"}"#).expect("write");
        assert!(read_stamp(&p).expect("read").age() > CHECK_EVERY);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
