//! The per-machine preferences file: `~/.local/state/mesimon/prefs.json`.
//!
//! Two slots — the theme for a DARK terminal and the theme for a LIGHT one —
//! because that is how the author already runs their editor (one scheme when
//! macOS is dark, another when it is light) and because the terminal's
//! OSC 11 answer is the only fact the board has about where it is being
//! read. The watch keeps flipping between the two picks; a pick sets the
//! slot the terminal currently reports.
//!
//! It sits at the state ROOT beside `update-check.json`: one binary per
//! machine, so one preference per machine, and README promise 1 already
//! covers `~/.local/state/mesimon/` where `~/.config/` would be a new write
//! (and would collide with promise 2, "no config mutation").
//!
//! **A preference, not a cache** — the inverse of the stamp's rule. A file
//! written by a newer mesimon is READ where it can be and never overwritten
//! (its writes are barred for the session, like the four state files); an
//! unreadable one falls to the defaults with a notice, and the next pick
//! rewrites it. Saving MERGES into the loaded document, so a name this build
//! does not know in the other slot survives a pick in this one.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use mesimon_core::notify::Sound;
use mesimon_core::prefs::PrefKey;
use mesimon_core::snooze::Weekday;

use crate::theme::{Flavor, Ground};

pub(crate) const SCHEMA: u64 = 1;

#[derive(Clone)]
pub(crate) struct Prefs {
    pub dark: Flavor,
    pub light: Flavor,
    /// A ticket back from a snooze wears needs-you until looked at (T-74).
    /// On by default; the Esc menu's row flips it. Same file, no schema
    /// move: an absent key reads as the default and a save keeps it.
    pub snooze_needs_you: bool,
    /// The day a week starts on — what the snooze ring's last rung, `next
    /// Monday 9:00`, means by "next week". Monday by default; the Settings
    /// row cycles Monday → Sunday → Saturday. Same file, no schema move.
    pub week_start: Weekday,
    /// The merge train (2026-09-04): while every claude on the board is idle,
    /// mesimon fast-forwards finished REVIEW branches and asks idle agents
    /// whose branch fell behind to rebase + test. OFF by default — it prompts
    /// an agent with no per-press gesture, and this row is the consent. The
    /// TUI pushes it to the daemon; the daemon never reads this file.
    pub merge_train: bool,
    /// After a train merge, paste the merged notice into that agent (starts
    /// a turn). On by default; only meaningful while the train is on.
    pub merge_train_notice: bool,
    /// The private tmux server's status line sits at the TOP of an agent's
    /// pane (T-264) — where the board's own header was — instead of tmux's
    /// default bottom. Off by default; the TUI pushes it to the daemon, which
    /// owns the server and never reads this file.
    pub status_top: bool,
    /// Hold this machine awake while an agent is mid-turn (T-288). OFF by
    /// default and deliberately, for `notify`'s reason one level up: changing
    /// what a machine does about power is a thing the user asks for, never a
    /// thing an update starts doing. Held only while a board is open, and
    /// only where `caffeine::find` found a rung.
    pub keep_awake: bool,
    /// The board says it out loud (T-282): an OS banner and a sound when an
    /// agent needs you. OFF by default and deliberately — the board is quiet
    /// on purpose, and a channel out is a thing the user asks for, never a
    /// thing an update starts doing. Nothing else in this group is read
    /// while it is off.
    pub notify: bool,
    /// Also when a turn finishes (`Idle{EndTurn}`), not only when an agent is
    /// blocked. On by default: it is the half that answers "can I go do
    /// something else", and the needs-you half is on already.
    pub notify_done: bool,
    /// Show the banner even while the board's own terminal has focus. Off by
    /// default — the card is already saying it in the one saturated colour,
    /// and a banner over the board it duplicates is noise. The SOUND plays
    /// either way; this row is only about the banner.
    pub notify_focused: bool,
    /// Say it even about the ticket whose agent pane you are attached to
    /// (T-292). Off by default: what happened in that pane is on the screen
    /// in front of you, so a banner points at nothing and the chime is for a
    /// turn you just watched land. Unlike [`Prefs::notify_focused`] this row
    /// governs the SOUND too — on the board a chime still says "go look",
    /// and inside the pane there is nowhere to go.
    pub notify_in_pane: bool,
    /// May a banner QUOTE the agent — its last line, and a raised hand's own
    /// sentence (T-292)? On by default: the words are why the banner was
    /// worth sending, and the ticket alone is what T-292 was filed against.
    /// Off is for a machine whose lock screen other people see: the board,
    /// the ticket's key and its title still go, and so does mesimon's own
    /// reason word, which is from a fixed set and describes no work.
    pub notify_words: bool,
    /// The sound for a blocked agent, and the sound for a turn landing. Two,
    /// so the difference is audible without looking. `Sound::Off` is a rung
    /// of each ring, so either can be silenced on its own.
    pub notify_sound_needs_you: Sound,
    pub notify_sound_done: Sound,
    /// The document as loaded, so a save keeps what it does not understand.
    doc: Map<String, Value>,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            dark: Flavor::Graphite,
            light: Flavor::Chalk,
            snooze_needs_you: true,
            week_start: Weekday::Monday,
            merge_train: false,
            merge_train_notice: true,
            status_top: false,
            keep_awake: false,
            notify: false,
            notify_done: true,
            notify_focused: false,
            notify_in_pane: false,
            notify_words: true,
            notify_sound_needs_you: Sound::Glass,
            notify_sound_done: Sound::Tink,
            doc: Map::new(),
        }
    }
}

// The JSON keys are `PrefKey::name()`, one list for both files (T-361).
const SNOOZE_KEY: &str = PrefKey::SnoozeNeedsYou.name();
const WEEK_START_KEY: &str = PrefKey::WeekStart.name();
const MERGE_TRAIN_KEY: &str = PrefKey::MergeTrain.name();
const MERGE_TRAIN_NOTICE_KEY: &str = PrefKey::MergeTrainNotice.name();
const STATUS_TOP_KEY: &str = PrefKey::StatusTop.name();
const KEEP_AWAKE_KEY: &str = PrefKey::KeepAwake.name();
const NOTIFY_KEY: &str = PrefKey::Notify.name();
const NOTIFY_DONE_KEY: &str = PrefKey::NotifyDone.name();
const NOTIFY_FOCUSED_KEY: &str = PrefKey::NotifyFocused.name();
const NOTIFY_IN_PANE_KEY: &str = PrefKey::NotifyInPane.name();
const NOTIFY_WORDS_KEY: &str = PrefKey::NotifyWords.name();
const NOTIFY_SOUND_NEEDS_YOU_KEY: &str = PrefKey::NotifySoundNeedsYou.name();
const NOTIFY_SOUND_DONE_KEY: &str = PrefKey::NotifySoundDone.name();

impl Prefs {
    // The three bools are plain fields: `body()` writes every one on each
    // save. The week's day has a setter because its write is conditional —
    // a foreign day in the file survives until a pick replaces it.
    pub(crate) fn set_week_start(&mut self, day: Weekday) {
        self.week_start = day;
        self.doc.insert(WEEK_START_KEY.into(), Value::from(day.key()));
    }

    /// The two sound names take the week's shape rather than the bools': a
    /// name from a newer build's ring survives in the file until a pick here
    /// replaces it.
    pub(crate) fn set_sound_needs_you(&mut self, s: Sound) {
        self.notify_sound_needs_you = s;
        self.doc.insert(NOTIFY_SOUND_NEEDS_YOU_KEY.into(), Value::from(s.key()));
    }

    pub(crate) fn set_sound_done(&mut self, s: Sound) {
        self.notify_sound_done = s;
        self.doc.insert(NOTIFY_SOUND_DONE_KEY.into(), Value::from(s.key()));
    }

    pub(crate) fn for_ground(&self, g: Ground) -> Flavor {
        match g {
            Ground::Dark => self.dark,
            Ground::Light => self.light,
        }
    }

    pub(crate) fn set(&mut self, g: Ground, f: Flavor) {
        match g {
            Ground::Dark => self.dark = f,
            Ground::Light => self.light = f,
        }
        // A pick replaces whatever the slot held, a name from a newer build
        // included — this is the one write that outranks it.
        self.doc.insert(g.word().into(), Value::from(f.name()));
    }

    /// The machine's value overlaid with what one board sets (T-361). The
    /// result is a VIEW: it is what every reader reads and it is never
    /// saved — the machine file gets the machine copy, the board file the
    /// board's — so the `doc` it carries is the machine's, unused.
    pub(crate) fn overlay(&self, board: &BoardPrefs) -> Prefs {
        let mut p = self.clone();
        if let Some(f) = board.flavor(Ground::Dark) {
            p.dark = f;
        }
        if let Some(f) = board.flavor(Ground::Light) {
            p.light = f;
        }
        for (key, slot) in [
            (PrefKey::SnoozeNeedsYou, &mut p.snooze_needs_you),
            (PrefKey::MergeTrain, &mut p.merge_train),
            (PrefKey::MergeTrainNotice, &mut p.merge_train_notice),
            (PrefKey::KeepAwake, &mut p.keep_awake),
            (PrefKey::Notify, &mut p.notify),
            (PrefKey::NotifyDone, &mut p.notify_done),
            (PrefKey::NotifyFocused, &mut p.notify_focused),
            (PrefKey::NotifyInPane, &mut p.notify_in_pane),
            (PrefKey::NotifyWords, &mut p.notify_words),
        ] {
            if let Some(v) = board.bool(key) {
                *slot = v;
            }
        }
        if let Some(v) = board.sound(PrefKey::NotifySoundNeedsYou) {
            p.notify_sound_needs_you = v;
        }
        if let Some(v) = board.sound(PrefKey::NotifySoundDone) {
            p.notify_sound_done = v;
        }
        p
    }

    /// What this copy holds for one key, as a word: `on`/`off`, a theme's
    /// name, a sound's name, a day. The row's detail quotes it as "the
    /// machine's".
    pub(crate) fn word(&self, key: PrefKey) -> &'static str {
        let onoff = |b: bool| if b { "on" } else { "off" };
        match key {
            PrefKey::Dark => self.dark.name(),
            PrefKey::Light => self.light.name(),
            PrefKey::SnoozeNeedsYou => onoff(self.snooze_needs_you),
            PrefKey::WeekStart => self.week_start.name(),
            PrefKey::MergeTrain => onoff(self.merge_train),
            PrefKey::MergeTrainNotice => onoff(self.merge_train_notice),
            PrefKey::StatusTop => {
                if self.status_top {
                    "top"
                } else {
                    "bottom"
                }
            }
            PrefKey::KeepAwake => onoff(self.keep_awake),
            PrefKey::Notify => onoff(self.notify),
            PrefKey::NotifyDone => onoff(self.notify_done),
            PrefKey::NotifyFocused => onoff(self.notify_focused),
            PrefKey::NotifyInPane => onoff(self.notify_in_pane),
            PrefKey::NotifyWords => onoff(self.notify_words),
            PrefKey::NotifySoundNeedsYou => self.notify_sound_needs_you.name(),
            PrefKey::NotifySoundDone => self.notify_sound_done.name(),
        }
    }

    fn body(&self) -> String {
        let mut doc = self.doc.clone();
        doc.insert("schema_version".into(), Value::from(SCHEMA));
        for (key, f) in [("dark", self.dark), ("light", self.light)] {
            // A name this build does not know is a newer build's pick for
            // that slot; the fallback it read as is not written over it.
            let foreign = doc
                .get(key)
                .and_then(Value::as_str)
                .is_some_and(|s| Flavor::from_name(s).is_none());
            if !foreign {
                doc.insert(key.into(), Value::from(f.name()));
            }
        }
        doc.insert(SNOOZE_KEY.into(), Value::from(self.snooze_needs_you));
        doc.insert(MERGE_TRAIN_KEY.into(), Value::from(self.merge_train));
        doc.insert(MERGE_TRAIN_NOTICE_KEY.into(), Value::from(self.merge_train_notice));
        doc.insert(STATUS_TOP_KEY.into(), Value::from(self.status_top));
        doc.insert(KEEP_AWAKE_KEY.into(), Value::from(self.keep_awake));
        doc.insert(NOTIFY_KEY.into(), Value::from(self.notify));
        doc.insert(NOTIFY_DONE_KEY.into(), Value::from(self.notify_done));
        doc.insert(NOTIFY_FOCUSED_KEY.into(), Value::from(self.notify_focused));
        doc.insert(NOTIFY_IN_PANE_KEY.into(), Value::from(self.notify_in_pane));
        doc.insert(NOTIFY_WORDS_KEY.into(), Value::from(self.notify_words));
        for (key, s) in [
            (NOTIFY_SOUND_NEEDS_YOU_KEY, self.notify_sound_needs_you),
            (NOTIFY_SOUND_DONE_KEY, self.notify_sound_done),
        ] {
            // A sound this build does not know is a newer build's pick; like
            // a foreign theme name, the default it read as is not written
            // over it.
            let foreign =
                doc.get(key).and_then(Value::as_str).is_some_and(|v| Sound::from_key(v).is_none());
            if !foreign {
                doc.insert(key.into(), Value::from(s.key()));
            }
        }
        // A day this build does not know is a newer build's; like a foreign
        // theme name, the default it read as is not written over it.
        let foreign = doc
            .get(WEEK_START_KEY)
            .and_then(Value::as_str)
            .is_some_and(|s| Weekday::from_key(s).is_none());
        if !foreign {
            doc.insert(WEEK_START_KEY.into(), Value::from(self.week_start.key()));
        }
        Value::Object(doc).to_string() + "\n"
    }
}

#[derive(Default)]
pub(crate) struct Loaded {
    pub prefs: Prefs,
    /// A newer build wrote it: read what is readable, write nothing back.
    pub write_barred: bool,
    /// One line for the status row at startup, or nothing.
    pub notice: Option<String>,
}

/// One board's overrides of the machine's preferences (T-361):
/// `~/.local/state/mesimon/<proj16>/prefs.json`, the TUI's second file.
/// SPARSE — it holds only the keys this board sets, and an absent key means
/// "inherit the machine's". Kept as the loaded document, like `Prefs::doc`,
/// so a key this build does not know survives a save; a value this build
/// cannot parse (a newer build's theme name, say) reads as inherit and is
/// not written over until a pick of that key replaces it. A machine-only key
/// (`PrefKey::board_overridable` false) in the file is ignored by `overlay`
/// and kept by `save`, in case a newer build made it overridable.
#[derive(Default, Clone)]
pub(crate) struct BoardPrefs {
    doc: Map<String, Value>,
}

pub(crate) const BOARD_SCHEMA: u64 = 1;

impl BoardPrefs {
    pub(crate) fn bool(&self, key: PrefKey) -> Option<bool> {
        if !key.board_overridable() {
            return None;
        }
        self.doc.get(key.name()).and_then(Value::as_bool)
    }

    pub(crate) fn sound(&self, key: PrefKey) -> Option<Sound> {
        if !key.board_overridable() {
            return None;
        }
        self.doc.get(key.name()).and_then(Value::as_str).and_then(Sound::from_key)
    }

    pub(crate) fn flavor(&self, g: Ground) -> Option<Flavor> {
        self.doc.get(g.word()).and_then(Value::as_str).and_then(Flavor::from_name)
    }

    /// Present AND readable by this build — what "set for this board" means.
    pub(crate) fn is_set(&self, key: PrefKey) -> bool {
        match key {
            PrefKey::Dark => self.flavor(Ground::Dark).is_some(),
            PrefKey::Light => self.flavor(Ground::Light).is_some(),
            PrefKey::NotifySoundNeedsYou | PrefKey::NotifySoundDone => self.sound(key).is_some(),
            PrefKey::WeekStart | PrefKey::StatusTop => false,
            _ => self.bool(key).is_some(),
        }
    }

    pub(crate) fn set_bool(&mut self, key: PrefKey, v: bool) {
        debug_assert!(key.board_overridable() && key.is_bool());
        self.doc.insert(key.name().into(), Value::from(v));
    }

    pub(crate) fn set_sound(&mut self, key: PrefKey, s: Sound) {
        debug_assert!(matches!(key, PrefKey::NotifySoundNeedsYou | PrefKey::NotifySoundDone));
        self.doc.insert(key.name().into(), Value::from(s.key()));
    }

    pub(crate) fn set_flavor(&mut self, g: Ground, f: Flavor) {
        self.doc.insert(g.word().into(), Value::from(f.name()));
    }

    /// Back to inheriting: the key leaves the file.
    pub(crate) fn clear(&mut self, key: PrefKey) {
        self.doc.remove(key.name());
    }

    pub(crate) fn overridden(&self) -> Vec<PrefKey> {
        PrefKey::ALL.iter().copied().filter(|k| self.is_set(*k)).collect()
    }

    fn body(&self) -> String {
        let mut doc = self.doc.clone();
        doc.insert("schema_version".into(), Value::from(BOARD_SCHEMA));
        Value::Object(doc).to_string() + "\n"
    }
}

#[derive(Default)]
pub(crate) struct LoadedBoard {
    pub prefs: BoardPrefs,
    pub write_barred: bool,
    pub notice: Option<String>,
}

/// Where this repo's overrides live, or `None` where the state dir cannot
/// be derived (the repo path does not resolve).
pub(crate) fn board_prefs_path(repo: &Path) -> Option<PathBuf> {
    mesimon_daemon::Paths::for_repo(repo).ok().map(|p| p.prefs_file())
}

pub(crate) fn load_board(path: &Path) -> LoadedBoard {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return LoadedBoard::default(),
        Err(_) => return board_unreadable(),
    };
    let Ok(Value::Object(doc)) = serde_json::from_str::<Value>(&text) else {
        return board_unreadable();
    };
    let schema = doc.get("schema_version").and_then(Value::as_u64).unwrap_or(0);
    let prefs = BoardPrefs { doc };
    if schema > BOARD_SCHEMA {
        return LoadedBoard {
            prefs,
            write_barred: true,
            notice: Some(
                "this board's prefs.json was written by a newer mesimon ∙ board picks last this session only"
                    .into(),
            ),
        };
    }
    LoadedBoard { prefs, write_barred: false, notice: None }
}

fn board_unreadable() -> LoadedBoard {
    LoadedBoard {
        prefs: BoardPrefs::default(),
        write_barred: false,
        notice: Some(
            "this board's prefs.json is unreadable ∙ its overrides are off until the next pick rewrites it"
                .into(),
        ),
    }
}

pub(crate) fn save_board(path: &Path, prefs: &BoardPrefs) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    mesimon_daemon::store::write_atomic(path, &prefs.body(), 0o644)
}

/// `mesimon doctor`'s `board prefs` line (T-361): which machine preferences
/// this repo's board overrides, and what the machine holds for each.
pub fn board_doctor_line(repo: &Path) -> String {
    let Some(path) = board_prefs_path(repo) else {
        return "none (the repo path does not resolve)".into();
    };
    let loaded = load_board(&path);
    let set = loaded.prefs.overridden();
    let mut line = if set.is_empty() {
        "none ∙ Settings ∙ b sets one for this board".to_string()
    } else {
        let machine = load_home().prefs;
        let resolved = machine.overlay(&loaded.prefs);
        let ours: Vec<String> =
            set.iter().map(|k| format!("{}: {}", k.name(), resolved.word(*k))).collect();
        let theirs: Vec<&str> = set.iter().map(|k| machine.word(*k)).collect();
        format!("{} ∙ (machine: {})", ours.join(" ∙ "), theirs.join(", "))
    };
    if let Some(n) = loaded.notice {
        line = format!("{line} ∙ {n}");
    }
    line
}

/// `~/.local/state/mesimon/prefs.json` — keyed off `HOME` rather than
/// `Paths`, like the update stamp, so `mesimon doctor` can answer without a
/// repo.
pub(crate) fn prefs_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".local/state/mesimon/prefs.json"))
}

/// The file where it lives, or the defaults where there is none — what
/// every `doctor` line starts from.
pub(crate) fn load_home() -> Loaded {
    prefs_path().map(|p| load(&p)).unwrap_or_default()
}

pub(crate) fn load(path: &Path) -> Loaded {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Loaded::default(),
        Err(_) => return unreadable(),
    };
    let Ok(Value::Object(doc)) = serde_json::from_str::<Value>(&text) else {
        return unreadable();
    };
    let schema = doc.get("schema_version").and_then(Value::as_u64).unwrap_or(0);
    let slot = |key: &str, fallback: Flavor| {
        doc.get(key).and_then(Value::as_str).and_then(Flavor::from_name).unwrap_or(fallback)
    };
    let snooze_needs_you = doc.get(SNOOZE_KEY).and_then(Value::as_bool).unwrap_or(true);
    let merge_train = doc.get(MERGE_TRAIN_KEY).and_then(Value::as_bool).unwrap_or(false);
    let merge_train_notice =
        doc.get(MERGE_TRAIN_NOTICE_KEY).and_then(Value::as_bool).unwrap_or(true);
    let status_top = doc.get(STATUS_TOP_KEY).and_then(Value::as_bool).unwrap_or(false);
    let keep_awake = doc.get(KEEP_AWAKE_KEY).and_then(Value::as_bool).unwrap_or(false);
    let notify = doc.get(NOTIFY_KEY).and_then(Value::as_bool).unwrap_or(false);
    let notify_done = doc.get(NOTIFY_DONE_KEY).and_then(Value::as_bool).unwrap_or(true);
    let notify_focused = doc.get(NOTIFY_FOCUSED_KEY).and_then(Value::as_bool).unwrap_or(false);
    let notify_in_pane = doc.get(NOTIFY_IN_PANE_KEY).and_then(Value::as_bool).unwrap_or(false);
    let notify_words = doc.get(NOTIFY_WORDS_KEY).and_then(Value::as_bool).unwrap_or(true);
    let sound = |key: &str, fallback: Sound| {
        doc.get(key).and_then(Value::as_str).and_then(Sound::from_key).unwrap_or(fallback)
    };
    let notify_sound_needs_you = sound(NOTIFY_SOUND_NEEDS_YOU_KEY, Sound::Glass);
    let notify_sound_done = sound(NOTIFY_SOUND_DONE_KEY, Sound::Tink);
    let week_start = doc
        .get(WEEK_START_KEY)
        .and_then(Value::as_str)
        .and_then(Weekday::from_key)
        .unwrap_or_default();
    let prefs = Prefs {
        dark: slot("dark", Flavor::Graphite),
        light: slot("light", Flavor::Chalk),
        snooze_needs_you,
        week_start,
        merge_train,
        merge_train_notice,
        status_top,
        keep_awake,
        notify,
        notify_done,
        notify_focused,
        notify_in_pane,
        notify_words,
        notify_sound_needs_you,
        notify_sound_done,
        doc,
    };
    if schema > SCHEMA {
        return Loaded {
            prefs,
            write_barred: true,
            notice: Some(
                "prefs.json was written by a newer mesimon ∙ picks last this session only".into(),
            ),
        };
    }
    Loaded { prefs, write_barred: false, notice: None }
}

fn unreadable() -> Loaded {
    Loaded {
        prefs: Prefs::default(),
        write_barred: false,
        notice: Some(
            "prefs.json is unreadable ∙ themes on defaults until the next pick rewrites it".into(),
        ),
    }
}

/// Temp + fsync + rename, the store's road. World-readable on purpose: two
/// theme names hold no secret, unlike the state dir's 0600 argv files.
pub(crate) fn save(path: &Path, prefs: &Prefs) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    mesimon_daemon::store::write_atomic(path, &prefs.body(), 0o644)
}

/// What `mesimon doctor` says. Never asks the terminal which ground it is
/// on: doctor runs in pipes, and an OSC 11 query there is exactly the tty
/// write the query-hygiene rules forbid.
pub fn doctor_line() -> String {
    let loaded = load_home();
    let mut line =
        format!("dark: {} ∙ light: {}", loaded.prefs.dark.name(), loaded.prefs.light.name());
    if let Some(f) = std::env::var("MESIMON_THEME").ok().as_deref().and_then(Flavor::from_name) {
        line = format!("pinned to {} by MESIMON_THEME ∙ {line}", f.name());
    }
    if let Some(n) = loaded.notice {
        line = format!("{line} ∙ {n}");
    }
    line
}

/// What `mesimon doctor` says about the snooze preferences (T-74): how a
/// woken ticket returns, and which day "next week" starts on.
/// `mesimon doctor`'s `merge train` line: on or off, and whether it tells
/// the agent after a merge. Fresh from the file, no daemon needed.
pub fn train_doctor_line() -> String {
    let loaded = load_home();
    let p = &loaded.prefs;
    if !p.merge_train {
        "off (Settings turns it on: merges quiet REVIEW branches, asks idle agents to rebase)"
            .into()
    } else if p.merge_train_notice {
        "on ∙ tells the agent after a merge ∙ armed only while a board is open".into()
    } else {
        "on ∙ silent after a merge ∙ armed only while a board is open".into()
    }
}

/// `mesimon doctor`'s `status line` line (T-264): which side of an agent's
/// pane the private tmux server's bar sits on.
pub fn status_line_doctor_line() -> String {
    if load_home().prefs.status_top {
        "top of the pane (Settings moves it back to the bottom)".into()
    } else {
        "bottom of the pane, tmux's default (Settings moves it to the top)".into()
    }
}

pub fn snooze_doctor_line() -> String {
    let prefs = load_home().prefs;
    let back = if prefs.snooze_needs_you {
        "a woken ticket returns with needs-you"
    } else {
        "a woken ticket returns quietly"
    };
    format!("{back} ∙ the week starts on {} (Settings changes both)", prefs.week_start.name())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("msmn-prefs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("prefs.json")
    }

    #[test]
    fn a_missing_file_is_the_defaults_and_says_nothing() {
        let l = load(&scratch("missing"));
        assert_eq!((l.prefs.dark, l.prefs.light), (Flavor::Graphite, Flavor::Chalk));
        assert!(!l.write_barred);
        assert!(l.notice.is_none());
    }

    #[test]
    fn a_pick_round_trips() {
        let p = scratch("roundtrip");
        let mut prefs = Prefs::default();
        prefs.set(Ground::Light, Flavor::Blue);
        save(&p, &prefs).unwrap();
        let l = load(&p);
        assert_eq!(l.prefs.for_ground(Ground::Light), Flavor::Blue);
        assert_eq!(l.prefs.for_ground(Ground::Dark), Flavor::Graphite);
        assert!(l.notice.is_none());
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("\"schema_version\":1"));
        assert!(text.ends_with('\n'));
    }

    /// The notification group (T-282): every key round-trips, and the file
    /// says the defaults out loud so a hand edit has something to edit.
    #[test]
    fn the_notification_preferences_round_trip() {
        let p = scratch("notify");
        let mut prefs = Prefs::default();
        assert!(!prefs.notify, "off by default, and deliberately");
        assert!(prefs.notify_done);
        assert!(!prefs.notify_focused);
        assert!(!prefs.notify_in_pane, "quiet inside the agent's own pane by default");
        assert!(prefs.notify_words, "the agent's words are quoted by default");
        prefs.notify = true;
        prefs.notify_done = false;
        prefs.notify_focused = true;
        prefs.notify_in_pane = true;
        prefs.notify_words = false;
        prefs.set_sound_needs_you(Sound::Hero);
        prefs.set_sound_done(Sound::Off);
        save(&p, &prefs).unwrap();
        let l = load(&p);
        assert!(l.prefs.notify);
        assert!(!l.prefs.notify_done);
        assert!(l.prefs.notify_focused);
        assert!(l.prefs.notify_in_pane);
        assert!(!l.prefs.notify_words);
        assert_eq!(l.prefs.notify_sound_needs_you, Sound::Hero);
        assert_eq!(l.prefs.notify_sound_done, Sound::Off);
        assert!(l.notice.is_none());
    }

    /// A sound name this build does not know is a newer build's pick: it
    /// reads as the default and survives a save of something else — the
    /// week's day rule, and the theme slots' before it.
    #[test]
    fn an_unknown_sound_falls_back_and_survives_a_save() {
        let p = scratch("notify-unknown");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(
            &p,
            r#"{"schema_version":1,"notify_sound_needs_you":"Klaxon","notify_sound_done":"Tink"}"#,
        )
        .unwrap();
        let mut l = load(&p);
        assert_eq!(l.prefs.notify_sound_needs_you, Sound::Glass, "the default stands in");
        l.prefs.set_sound_done(Sound::Purr);
        save(&p, &l.prefs).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["notify_sound_needs_you"], "Klaxon", "not written over");
        assert_eq!(v["notify_sound_done"], "Purr", "and the pick landed");
    }

    /// A name this build does not know falls to that slot's default — and
    /// survives a save of the OTHER slot, because the file is theirs too.
    #[test]
    fn an_unknown_name_falls_back_and_survives_a_save() {
        let p = scratch("unknown");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, r#"{"schema_version":1,"dark":"sepia","light":"chalk","x":1}"#).unwrap();
        let mut l = load(&p);
        assert_eq!(l.prefs.dark, Flavor::Graphite);
        l.prefs.set(Ground::Light, Flavor::Amber);
        save(&p, &l.prefs).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        // The slot we did not pick keeps the name we did not know, the field
        // we never understood is still there, and the pick landed.
        assert_eq!(v["dark"], "sepia");
        assert_eq!(v["light"], "amber");
        assert_eq!(v["x"], 1);
        // Picking THAT slot is what replaces the foreign name.
        l.prefs.set(Ground::Dark, Flavor::Blue);
        save(&p, &l.prefs).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["dark"], "blue");
    }

    #[test]
    fn a_newer_schema_is_read_and_never_written() {
        let p = scratch("newer");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, r#"{"schema_version":9,"dark":"green","light":"blue"}"#).unwrap();
        let l = load(&p);
        assert!(l.write_barred);
        assert!(l.notice.as_deref().unwrap_or("").contains("newer"));
        assert_eq!((l.prefs.dark, l.prefs.light), (Flavor::Green, Flavor::Blue));
    }

    /// The snooze preference (T-74): absent reads as on, a flip round-trips,
    /// and a theme pick on an older-shaped file keeps what it found.
    #[test]
    fn the_snooze_preference_defaults_on_and_round_trips() {
        let p = scratch("snooze");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, r#"{"schema_version":1,"dark":"blue","light":"chalk"}"#).unwrap();
        let mut l = load(&p);
        assert!(l.prefs.snooze_needs_you, "absent is the default: on");
        l.prefs.snooze_needs_you = false;
        save(&p, &l.prefs).unwrap();
        let l = load(&p);
        assert!(!l.prefs.snooze_needs_you);
        assert_eq!(l.prefs.dark, Flavor::Blue, "the theme slots are untouched");
        // A later theme pick writes the flag it loaded, not the default.
        let mut l = l;
        l.prefs.set(Ground::Dark, Flavor::Amber);
        save(&p, &l.prefs).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["snooze_needs_you"], false);
        assert_eq!(v["dark"], "amber");
    }

    /// The merge train: absent is OFF (it prompts agents with no gesture)
    /// and the notice absent is ON; both round-trip and survive a theme pick.
    #[test]
    fn the_merge_train_defaults_off_and_round_trips() {
        let p = scratch("train");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, r#"{"schema_version":1,"dark":"blue","light":"chalk"}"#).unwrap();
        let mut l = load(&p);
        assert!(!l.prefs.merge_train, "absent is the default: off");
        assert!(l.prefs.merge_train_notice, "absent is the default: on");
        l.prefs.merge_train = true;
        l.prefs.merge_train_notice = false;
        save(&p, &l.prefs).unwrap();
        let mut l = load(&p);
        assert!(l.prefs.merge_train);
        assert!(!l.prefs.merge_train_notice);
        l.prefs.set(Ground::Dark, Flavor::Amber);
        save(&p, &l.prefs).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["merge_train"], true);
        assert_eq!(v["merge_train_notice"], false);
        assert_eq!(v["dark"], "amber");
    }

    /// The status line's side (T-264): absent is the bottom (tmux's own
    /// default), a flip round-trips and survives a theme pick.
    #[test]
    fn the_status_line_defaults_to_the_bottom_and_round_trips() {
        let p = scratch("statusline");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, r#"{"schema_version":1,"dark":"blue","light":"chalk"}"#).unwrap();
        let mut l = load(&p);
        assert!(!l.prefs.status_top, "absent is the default: bottom");
        l.prefs.status_top = true;
        save(&p, &l.prefs).unwrap();
        let mut l = load(&p);
        assert!(l.prefs.status_top);
        l.prefs.set(Ground::Dark, Flavor::Amber);
        save(&p, &l.prefs).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["status_line_top"], true);
        assert_eq!(v["dark"], "amber");
    }

    /// Keeping the machine awake (T-288): absent is off — a file written
    /// before the preference existed must not start changing what a machine
    /// does about power — and a pick survives a save of something else.
    #[test]
    fn keeping_the_machine_awake_defaults_to_off_and_round_trips() {
        let p = scratch("keepawake");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, r#"{"schema_version":1,"dark":"blue","light":"chalk"}"#).unwrap();
        let mut l = load(&p);
        assert!(!l.prefs.keep_awake, "absent is off");
        l.prefs.keep_awake = true;
        save(&p, &l.prefs).unwrap();
        let mut l = load(&p);
        assert!(l.prefs.keep_awake);
        l.prefs.set(Ground::Dark, Flavor::Amber);
        save(&p, &l.prefs).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["keep_awake"], true);
        assert_eq!(v["dark"], "amber");
    }

    /// The week-start preference: absent is Monday, a pick round-trips as
    /// its lower-case name, a day this build does not know falls to Monday
    /// and survives a save of something else.
    #[test]
    fn the_week_start_defaults_to_monday_and_round_trips() {
        let p = scratch("weekstart");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, r#"{"schema_version":1,"dark":"blue","light":"chalk"}"#).unwrap();
        let mut l = load(&p);
        assert_eq!(l.prefs.week_start, Weekday::Monday, "absent is the default");
        l.prefs.set_week_start(Weekday::Sunday);
        save(&p, &l.prefs).unwrap();
        let l = load(&p);
        assert_eq!(l.prefs.week_start, Weekday::Sunday);
        assert!(l.prefs.snooze_needs_you, "the neighbour is untouched");
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["week_start"], "sunday");
        // A foreign day reads as the default and a theme pick keeps it.
        std::fs::write(
            &p,
            r#"{"schema_version":1,"dark":"blue","light":"chalk","week_start":"wednesday"}"#,
        )
        .unwrap();
        let mut l = load(&p);
        assert_eq!(l.prefs.week_start, Weekday::Monday);
        l.prefs.set(Ground::Dark, Flavor::Amber);
        save(&p, &l.prefs).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["week_start"], "wednesday", "not written over");
        // Picking a day IS what replaces it.
        l.prefs.set_week_start(Weekday::Saturday);
        save(&p, &l.prefs).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["week_start"], "saturday");
    }

    #[test]
    fn garbage_is_the_defaults_with_a_notice() {
        let p = scratch("garbage");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "not json").unwrap();
        let l = load(&p);
        assert_eq!(l.prefs.dark, Flavor::Graphite);
        assert!(!l.write_barred, "garbage is rewritten by the next pick");
        assert!(l.notice.as_deref().unwrap_or("").contains("unreadable"));
    }

    /// One board's file overlays only what it sets (T-361): the machine's
    /// train on, the board's off, everything else the machine's — and the
    /// resolved view is what a reader reads, never what a save writes.
    #[test]
    fn a_board_file_overlays_only_what_it_sets() {
        let mut machine = Prefs { merge_train: true, notify: true, ..Default::default() };
        machine.set(Ground::Dark, Flavor::Blue);
        let mut board = BoardPrefs::default();
        board.set_bool(PrefKey::MergeTrain, false);
        board.set_flavor(Ground::Light, Flavor::Amber);
        board.set_sound(PrefKey::NotifySoundDone, Sound::Hero);
        let r = machine.overlay(&board);
        assert!(!r.merge_train, "the board's off wins");
        assert!(r.notify, "untouched keys are the machine's");
        assert_eq!(r.dark, Flavor::Blue);
        assert_eq!(r.light, Flavor::Amber);
        assert_eq!(r.notify_sound_done, Sound::Hero);
        assert_eq!(
            board.overridden(),
            [PrefKey::Light, PrefKey::MergeTrain, PrefKey::NotifySoundDone]
        );
        assert_eq!(r.word(PrefKey::MergeTrain), "off");
        assert_eq!(machine.word(PrefKey::MergeTrain), "on");
    }

    /// A board pick round-trips through its own file, and clearing it
    /// drops the key — absent IS inherit, so the file never spells it.
    #[test]
    fn a_board_pick_round_trips_and_clear_drops_the_key() {
        let p = scratch("board-roundtrip");
        let mut b = BoardPrefs::default();
        b.set_bool(PrefKey::KeepAwake, true);
        save_board(&p, &b).unwrap();
        let l = load_board(&p);
        assert_eq!(l.prefs.bool(PrefKey::KeepAwake), Some(true));
        assert!(l.notice.is_none() && !l.write_barred);
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("\"schema_version\":1"));
        assert!(text.ends_with('\n'));
        let mut b = l.prefs;
        b.clear(PrefKey::KeepAwake);
        save_board(&p, &b).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert!(v.get("keep_awake").is_none(), "{v}");
        assert!(load_board(&p).prefs.overridden().is_empty());
    }

    /// A value this build cannot read is a newer build's pick: it reads as
    /// INHERIT (the machine's value stands, one level up from the machine
    /// file's "the default stands") and survives a save of something else.
    #[test]
    fn a_foreign_board_value_reads_as_inherit_and_survives_a_save() {
        let p = scratch("board-foreign");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, r#"{"schema_version":1,"dark":"sepia","notify_sound_done":"Klaxon"}"#)
            .unwrap();
        let mut l = load_board(&p);
        assert!(!l.prefs.is_set(PrefKey::Dark));
        assert!(!l.prefs.is_set(PrefKey::NotifySoundDone));
        let machine = Prefs::default();
        assert_eq!(machine.overlay(&l.prefs).dark, Flavor::Graphite);
        l.prefs.set_bool(PrefKey::Notify, true);
        save_board(&p, &l.prefs).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["dark"], "sepia", "not written over");
        assert_eq!(v["notify_sound_done"], "Klaxon");
        assert_eq!(v["notify"], true);
    }

    #[test]
    fn a_newer_board_schema_is_read_and_never_written() {
        let p = scratch("board-newer");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, r#"{"schema_version":9,"merge_train":true}"#).unwrap();
        let l = load_board(&p);
        assert!(l.write_barred);
        assert!(l.notice.as_deref().unwrap_or("").contains("newer"));
        assert_eq!(l.prefs.bool(PrefKey::MergeTrain), Some(true));
    }

    /// The two machine-only keys in a board file (a newer build may have
    /// made them overridable) are ignored by the overlay and kept by a save.
    #[test]
    fn machine_only_keys_in_a_board_file_are_ignored_and_kept() {
        let p = scratch("board-machine-only");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, r#"{"schema_version":1,"status_line_top":true,"week_start":"sunday"}"#)
            .unwrap();
        let mut l = load_board(&p);
        assert!(l.prefs.overridden().is_empty());
        let r = Prefs::default().overlay(&l.prefs);
        assert!(!r.status_top);
        assert_eq!(r.week_start, Weekday::Monday);
        l.prefs.set_bool(PrefKey::Notify, true);
        save_board(&p, &l.prefs).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["status_line_top"], true);
        assert_eq!(v["week_start"], "sunday");
    }

    #[test]
    fn board_garbage_is_no_overrides_with_a_notice() {
        let p = scratch("board-garbage");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "not json").unwrap();
        let l = load_board(&p);
        assert!(l.prefs.overridden().is_empty());
        assert!(!l.write_barred);
        assert!(l.notice.as_deref().unwrap_or("").contains("unreadable"));
    }
}
