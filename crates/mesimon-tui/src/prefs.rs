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
use mesimon_core::snooze::Weekday;

use crate::theme::{Flavor, Ground};

pub(crate) const SCHEMA: u64 = 1;

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
            notify: false,
            notify_done: true,
            notify_focused: false,
            notify_sound_needs_you: Sound::Glass,
            notify_sound_done: Sound::Tink,
            doc: Map::new(),
        }
    }
}

const SNOOZE_KEY: &str = "snooze_needs_you";
const WEEK_START_KEY: &str = "week_start";
const MERGE_TRAIN_KEY: &str = "merge_train";
const MERGE_TRAIN_NOTICE_KEY: &str = "merge_train_notice";
const STATUS_TOP_KEY: &str = "status_line_top";
const NOTIFY_KEY: &str = "notify";
const NOTIFY_DONE_KEY: &str = "notify_done";
const NOTIFY_FOCUSED_KEY: &str = "notify_focused";
const NOTIFY_SOUND_NEEDS_YOU_KEY: &str = "notify_sound_needs_you";
const NOTIFY_SOUND_DONE_KEY: &str = "notify_sound_done";

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
        doc.insert(NOTIFY_KEY.into(), Value::from(self.notify));
        doc.insert(NOTIFY_DONE_KEY.into(), Value::from(self.notify_done));
        doc.insert(NOTIFY_FOCUSED_KEY.into(), Value::from(self.notify_focused));
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
    let notify = doc.get(NOTIFY_KEY).and_then(Value::as_bool).unwrap_or(false);
    let notify_done = doc.get(NOTIFY_DONE_KEY).and_then(Value::as_bool).unwrap_or(true);
    let notify_focused = doc.get(NOTIFY_FOCUSED_KEY).and_then(Value::as_bool).unwrap_or(false);
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
        notify,
        notify_done,
        notify_focused,
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
        prefs.notify = true;
        prefs.notify_done = false;
        prefs.notify_focused = true;
        prefs.set_sound_needs_you(Sound::Hero);
        prefs.set_sound_done(Sound::Off);
        save(&p, &prefs).unwrap();
        let l = load(&p);
        assert!(l.prefs.notify);
        assert!(!l.prefs.notify_done);
        assert!(l.prefs.notify_focused);
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
}
