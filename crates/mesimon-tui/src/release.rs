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
//!
//! **A binary Homebrew installed is asked about, never replaced** (T-463).
//! The board still hears of a newer tag and says so, but taking the offer
//! names `brew upgrade mesimon` instead of downloading: a file swapped in at
//! brew's `bin/` link is one brew no longer tracks, and its next upgrade
//! refuses to link over it. The tap is bumped by the same `ci/release.sh`
//! run that publishes the tag, so the two answers agree.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use semver::Version;
use sha2::{Digest, Sha256};

/// Where the binaries are published. The source repo is private; this one
/// carries releases and nothing else, which is why the check needs no token
/// and no GitHub account (README, "Install").
const DIST_REPO: &str = "amitozalvo/mesimon-releases";

/// Half an hour between questions. The stamp is shared by every board on
/// this machine, so this is the rate for the USER, not the process — a dozen
/// open repos still make one request per interval, two an hour against
/// GitHub's sixty. Cheap enough that a tester told "alpha.5 is out" sees the
/// offer before the conversation is over, which is the whole point of asking.
/// A request that did not answer simply waits out the same interval: at this
/// cadence a separate retry clock would be the same number.
const CHECK_EVERY: Duration = Duration::from_secs(30 * 60);
/// Never during the first paint. Startup already spawns a daemon, probes the
/// terminal twice and loads a board; the least urgent thing mesimon does can
/// wait three seconds.
const FIRST_CHECK_AFTER: Duration = Duration::from_secs(3);
const POLL_EVERY: Duration = Duration::from_secs(1);
/// The floor under a check somebody asked for (T-445) — opening the release
/// notes. A stamp younger than this IS the answer, whichever board or
/// `mesimon update` wrote it: GitHub allows sixty unauthenticated requests an
/// hour to the whole machine, and a person paging in and out of the notes
/// should not be the one to spend them.
const ASK_AGAIN_AFTER: Duration = Duration::from_secs(60);

const API_TIMEOUT_SECS: &str = "10";
const DOWNLOAD_TIMEOUT_SECS: &str = "180";

const STAMP_SCHEMA: u64 = 1;

/// Every target the release channel publishes — `ci/release.sh`'s list, and
/// the names `install.sh` derives from `uname`. A unit test reads both files,
/// so the three copies cannot drift. `TARGET` is the one THIS binary is, and
/// `None` anywhere else, which makes the whole module inert rather than making
/// it guess at an asset name that was never uploaded. Linux is the musl pair
/// only: a `cargo install` from source is a glibc build, and a source build
/// is a dev build besides, so it never gets here.
pub(crate) const PUBLISHED: &[&str] =
    &["aarch64-apple-darwin", "x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl"];
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const TARGET: Option<&str> = Some("aarch64-apple-darwin");
#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "musl"))]
const TARGET: Option<&str> = Some("x86_64-unknown-linux-musl");
#[cfg(all(target_os = "linux", target_arch = "aarch64", target_env = "musl"))]
const TARGET: Option<&str> = Some("aarch64-unknown-linux-musl");
#[cfg(not(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(
        target_os = "linux",
        target_env = "musl",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )
)))]
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
    /// Homebrew installed this binary: see [`by_homebrew`].
    brew: bool,
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
    /// The question itself — `fetch_latest_tag`, and a stand-in under test.
    fetch: fn() -> Option<String>,
    tx: Sender<Outcome>,
    rx: Receiver<Outcome>,
}

impl ReleaseWatch {
    /// A watch that has checked nothing and will check nothing until it is
    /// given an `Eligible` — which only `new` does, outside the tests.
    fn inert() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            eligible: None,
            stage: Stage::Quiet,
            note: None,
            next_check: None,
            last_poll: Instant::now(),
            working: false,
            fetch: fetch_latest_tag,
            tx,
            rx,
        }
    }

    pub fn new(repo_root: &Path) -> Self {
        let mut w = Self::inert();
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
        // (see `adopt`), and not asking is cheaper than reasoning about it.
        if self.eligible.is_none()
            || self.working
            || matches!(self.stage, Stage::Installing(_))
            || self.last_poll.elapsed() < POLL_EVERY
        {
            return dirty;
        }
        self.last_poll = Instant::now();
        if self.next_check.is_some_and(|t| Instant::now() >= t) {
            self.ask();
        }
        dirty
    }

    /// Ask now, off the half-hour clock (T-445). Opening the release notes is
    /// a person asking what is new, and an answer up to half an hour old is
    /// the wrong one to give them. It stays as quiet as the clock's: a newer
    /// tag raises the same chip, and being current says nothing.
    ///
    /// A stamp younger than [`ASK_AGAIN_AFTER`] is taken as the answer rather
    /// than asking again — which also picks up what another board, or
    /// `mesimon update`, heard a moment ago.
    pub fn check_now(&mut self) {
        let Some(e) = &self.eligible else { return };
        // Nothing to ask while a download is out or once one landed: the
        // answer could not change what happens next (see `adopt`).
        if self.working || matches!(self.stage, Stage::Installing(_) | Stage::Installed(_)) {
            return;
        }
        if let Some(stamp) = read_stamp(&e.stamp).filter(|s| s.age() < ASK_AGAIN_AFTER) {
            if let Some(tag) = stamp.latest {
                self.adopt(tag);
            }
            return;
        }
        self.ask();
    }

    /// Send the one worker that asks, and put the clock's next question a
    /// whole interval out — an answer asked for is as good as a scheduled one.
    fn ask(&mut self) {
        self.next_check = Some(Instant::now() + CHECK_EVERY);
        let fetch = self.fetch;
        self.spawn(move || Outcome::Latest(fetch()));
    }

    /// Take the offer: fetch, verify, and put the new binary at our own path.
    /// Returns the tag it started on, or `None` when there is nothing to take
    /// — which the menu row's own `avail` already rules out.
    pub fn begin_install(&mut self) -> Option<String> {
        let Stage::Available(tag) = &self.stage else {
            return None;
        };
        let tag = tag.clone();
        let e = self.eligible.as_ref().filter(|e| !e.brew)?;
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

    /// Homebrew installed this binary, so the offer is `brew upgrade
    /// mesimon` and `begin_install` takes nothing.
    pub fn by_brew(&self) -> bool {
        self.eligible.as_ref().is_some_and(|e| e.brew)
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
                self.adopt(tag);
            }
            // The request did not answer. The stamp is deliberately NOT
            // written: it records when we last HEARD, so a week offline must
            // not read back as a week of successful checks. The next attempt
            // is already on the clock (`tick` set it before spawning).
            Outcome::Latest(None) => {}
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

    /// Take a heard tag as the offer, or as its absence. A check never
    /// overrules a download in flight or one that landed: those are further
    /// along the same story.
    fn adopt(&mut self, tag: String) {
        if matches!(self.stage, Stage::Quiet | Stage::Available(_)) {
            self.stage = if is_newer(&tag) { Stage::Available(tag) } else { Stage::Quiet };
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
    let exe = mesimon_core::exe::current_exe().ok()?;
    let paths = mesimon_daemon::Paths::for_repo(repo_root).ok()?;
    Some(Eligible {
        exe,
        stamp: stamp_path()?,
        stage_dir: paths.rt_dir.join("update"),
        brew: by_homebrew(),
    })
}

/// Homebrew installed this binary: its real path is inside a keg. macOS
/// starts us by brew's `bin/` link and Linux by the keg's `opt/` link
/// (`exe::current_exe`), so the answer is read off the canonical path.
fn by_homebrew() -> bool {
    mesimon_core::exe::current_exe()
        .and_then(std::fs::canonicalize)
        .is_ok_and(|p| mesimon_core::exe::keg_opt_path(&p).is_some())
}

/// Why this build does not check, or `None` when it does. One function, so
/// the doctor line and the checker itself cannot disagree about the answer —
/// and so a reason that is missing from the diagnosis is a reason that cannot
/// exist.
fn why_off() -> Option<&'static str> {
    if std::env::var_os("MESIMON_NO_UPDATE_CHECK").is_some() {
        return Some("MESIMON_NO_UPDATE_CHECK is set");
    }
    why_unfit()
}

/// Why this BUILD cannot update itself, whoever asks. `mesimon update` answers
/// to this alone (T-445): the opt-out above silences the question a board asks
/// on its own, and a person who types the command is asking it themselves.
/// Every guard here still holds for them — above all the build-tree one.
fn why_unfit() -> Option<&'static str> {
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
    if mesimon_core::exe::current_exe().is_ok_and(|e| in_build_tree(&e)) {
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
    let line = match stamp_path().and_then(|p| read_stamp(&p)) {
        None => format!("on ∙ never asked ∙ {DIST_REPO}"),
        Some(stamp) => {
            let ago = ago(stamp.age());
            match stamp.latest {
                Some(t) if is_newer(&t) => format!("on ∙ {t} is out ∙ asked {ago}"),
                Some(t) => format!("on ∙ {t} is the newest ∙ asked {ago}"),
                None => format!("on ∙ asked {ago}"),
            }
        }
    };
    if by_homebrew() {
        format!("{line} ∙ installed by Homebrew, so brew upgrade mesimon updates it")
    } else {
        line
    }
}

/// `4m ago` / `2h ago` / `3d ago` — the menu's word for a moment behind us.
pub(crate) fn ago(d: Duration) -> String {
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
    let target =
        TARGET.filter(|t| PUBLISHED.contains(t)).ok_or("no published build for this platform")?;
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

// ---- the command -----------------------------------------------------------

/// `mesimon update [--check]` (T-445): ask now, and unless `--check`, take the
/// offer — the menu's `Install` row from a shell, the same fetch, checksum,
/// `--version` gate and atomic swap. It restarts nothing either: an open board
/// sees its exe's mtime move and offers `U`, and the next board started
/// restarts an older daemon on its own. Returns the process's exit code.
pub fn update_command(check_only: bool) -> i32 {
    let ours = format!("v{}", env!("CARGO_PKG_VERSION"));
    if let Some(why) = why_unfit() {
        eprintln!("mesimon update: {why} ∙ re-run the install line instead");
        return 1;
    }
    let Some(tag) = fetch_latest_tag() else {
        eprintln!("mesimon update: the releases repo did not answer (github.com, over curl)");
        return 1;
    };
    // Every board's next check, and the doctor, hear what this one did.
    if let Some(stamp) = stamp_path() {
        write_stamp(&stamp, &tag);
    }
    if !is_newer(&tag) {
        println!("{ours} is the newest");
        return 0;
    }
    let brew = by_homebrew();
    if check_only {
        let how = if brew { "brew upgrade mesimon" } else { "mesimon update" };
        println!("{tag} is out ∙ this is {ours} ∙ {how} installs it");
        return 0;
    }
    if brew {
        eprintln!(
            "mesimon update: {tag} is out ∙ Homebrew installed {ours} ∙ brew upgrade mesimon"
        );
        return 1;
    }
    let exe = match mesimon_core::exe::current_exe() {
        Ok(exe) => exe,
        Err(e) => {
            eprintln!("mesimon update: {e}");
            return 1;
        }
    };
    // No repo, so no project runtime dir: a per-process dir under the uid's
    // own, which is verified private first — a tarball staged anywhere others
    // can write could be swapped between its checksum and its unpack.
    let root = mesimon_daemon::paths::runtime_root();
    if let Err(e) = mesimon_daemon::paths::own_private_dir(&root) {
        eprintln!("mesimon update: {e:#}");
        return 1;
    }
    let stage_dir = root.join(format!("update-{}", std::process::id()));
    println!("downloading {tag} ∙ verifying the checksum");
    let landed = install(&tag, &exe, &stage_dir);
    let _ = std::fs::remove_dir_all(&stage_dir);
    match landed {
        Ok(()) => {
            println!(
                "{tag} is installed at {} ∙ an open board offers U to restart on it",
                exe.display()
            );
            0
        }
        Err(e) => {
            eprintln!("mesimon update failed: {e}");
            1
        }
    }
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

use mesimon_core::clock::now_ms;

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
        let mut w = ReleaseWatch::new(Path::new("."));
        assert!(w.eligible.is_none());
        assert!(!w.available());
        assert_eq!(w.tag(), "");
        // Opening the release notes on a dev build asks nothing either.
        w.check_now();
        assert!(!w.working, "an ineligible watch sent a worker");
    }

    /// And `mesimon update` refuses on the same ground before it asks — the
    /// opt-out is not what stops it, the build is.
    #[test]
    fn a_dev_build_refuses_the_command_before_it_asks() {
        assert!(why_unfit().is_some());
        assert_eq!(update_command(true), 1);
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("msmn-release-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("tmp");
        dir
    }

    /// What `new` builds on a release build, pointed at a scratch stamp and a
    /// stand-in for the request.
    fn eligible_watch(dir: &Path, fetch: fn() -> Option<String>) -> ReleaseWatch {
        let mut w = ReleaseWatch::inert();
        w.fetch = fetch;
        w.eligible = Some(Eligible {
            exe: dir.join("mesimon"),
            stamp: dir.join("update-check.json"),
            stage_dir: dir.join("update"),
            brew: false,
        });
        w
    }

    /// The worker's one answer, absorbed as `tick` would.
    fn settle(w: &mut ReleaseWatch) {
        let o = w.rx.recv_timeout(Duration::from_secs(5)).expect("the worker answers");
        w.absorb(o);
    }

    fn a_newer_one() -> Option<String> {
        Some("v99.0.0".into())
    }
    fn this_one() -> Option<String> {
        Some(format!("v{}", env!("CARGO_PKG_VERSION")))
    }
    fn another_newer_one() -> Option<String> {
        Some("v98.0.0".into())
    }

    /// Opening the notes asks at once, once, and the answer lands exactly as
    /// the clock's would: the offer, the stamp, and the clock pushed out.
    #[test]
    fn asking_now_asks_once_and_lands_like_the_clock() {
        let dir = scratch("now");
        let mut w = eligible_watch(&dir, a_newer_one);
        let before = Instant::now();
        w.check_now();
        assert!(w.working, "asked at once");
        w.check_now();
        settle(&mut w);
        assert!(
            w.rx.recv_timeout(Duration::from_millis(200)).is_err(),
            "a second press while the first was out sent a second worker"
        );
        assert!(w.available());
        assert_eq!(w.tag(), "v99.0.0");
        let stamp = read_stamp(&dir.join("update-check.json")).expect("the answer is stamped");
        assert_eq!(stamp.latest.as_deref(), Some("v99.0.0"));
        assert!(w.next_check.is_some_and(|t| t >= before + CHECK_EVERY), "the clock moved out");

        let _ = std::fs::remove_dir_all(dir);

        // Current is quiet, as it is on the clock.
        let dir = scratch("now-current");
        let mut w = eligible_watch(&dir, this_one);
        w.check_now();
        settle(&mut w);
        assert!(!w.available());
        assert_eq!(w.tag(), "");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A stamp inside the floor IS the answer — whoever wrote it — and an
    /// older one is asked past.
    #[test]
    fn a_fresh_stamp_is_the_answer_and_asks_nothing() {
        let dir = scratch("floor");
        let stamp = dir.join("update-check.json");
        write_stamp(&stamp, "v99.0.0");
        let mut w = eligible_watch(&dir, another_newer_one);
        w.check_now();
        assert!(!w.working, "asked inside the floor");
        assert_eq!(w.tag(), "v99.0.0", "the stamp's answer was not taken");

        let old = now_ms() - (ASK_AGAIN_AFTER.as_millis() as u64) - 1_000;
        std::fs::write(
            &stamp,
            format!(r#"{{"schema_version": 1, "checked_at_ms": {old}, "latest": "v99.0.0"}}"#),
        )
        .expect("write");
        w.check_now();
        assert!(w.working, "a stamp past the floor was not asked past");
        settle(&mut w);
        assert_eq!(w.tag(), "v98.0.0");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Never while a download is out or after one landed: the answer could
    /// not change what happens next.
    #[test]
    fn asking_now_waits_on_a_download() {
        let dir = scratch("installing");
        let mut w = eligible_watch(&dir, a_newer_one);
        for stage in [Stage::Installing("v99.0.0".into()), Stage::Installed("v99.0.0".into())] {
            w.stage = stage.clone();
            w.check_now();
            assert!(!w.working, "asked during {stage:?}");
            assert_eq!(w.stage, stage);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A binary Homebrew installed hears of a newer tag like any other, and
    /// taking the offer downloads nothing: the board says `brew upgrade
    /// mesimon` instead, and the offer stays up until brew has done it.
    #[test]
    fn a_brew_install_is_offered_and_never_downloaded() {
        let dir = scratch("brew");
        let mut w = eligible_watch(&dir, a_newer_one);
        if let Some(e) = w.eligible.as_mut() {
            e.brew = true;
        }
        w.check_now();
        settle(&mut w);
        assert!(w.available(), "a brew install was not told");
        assert!(w.by_brew());
        assert_eq!(w.begin_install(), None);
        assert!(!w.working, "a brew install sent a download");
        assert!(w.available(), "the offer came down with nothing taken");
        assert!(!dir.join("update").exists(), "staged a download");
        let _ = std::fs::remove_dir_all(dir);
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

    /// The target names live in three places — this list, the scripts that
    /// build and package them, and the installer that derives them from
    /// `uname` — and this is what keeps them one list. A name missing from
    /// the scripts is an asset the checker would ask for and never find.
    #[test]
    fn every_published_target_is_built_by_ci_and_installable() {
        let release_sh = include_str!("../../../ci/release.sh");
        let build_linux_sh = include_str!("../../../ci/build-linux.sh");
        let install_sh = include_str!("../../../install.sh");
        let formula = include_str!("../../../ci/homebrew/mesimon.rb");
        for t in PUBLISHED {
            assert!(
                release_sh.contains(t) || build_linux_sh.contains(t),
                "{t} is built by no script in ci/"
            );
            assert!(install_sh.contains(t), "install.sh cannot name the {t} asset");
            assert!(
                formula.contains(&format!("@SHA256:{t}@")),
                "the Homebrew formula has no checksum slot for {t}"
            );
        }
        if let Some(t) = TARGET {
            assert!(PUBLISHED.contains(&t), "this binary's target {t} is not a published one");
        }
    }
}
