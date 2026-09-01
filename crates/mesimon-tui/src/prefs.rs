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

use crate::theme::{Flavor, Ground};

pub(crate) const SCHEMA: u64 = 1;

pub(crate) struct Prefs {
    pub dark: Flavor,
    pub light: Flavor,
    /// The document as loaded, so a save keeps what it does not understand.
    doc: Map<String, Value>,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { dark: Flavor::Graphite, light: Flavor::Chalk, doc: Map::new() }
    }
}

impl Prefs {
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
        self.doc.insert(slot_key(g).into(), Value::from(f.name()));
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
        Value::Object(doc).to_string() + "\n"
    }
}

fn slot_key(g: Ground) -> &'static str {
    match g {
        Ground::Dark => "dark",
        Ground::Light => "light",
    }
}

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

pub(crate) fn load(path: &Path) -> Loaded {
    let defaults = || Loaded { prefs: Prefs::default(), write_barred: false, notice: None };
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return defaults(),
        Err(_) => return unreadable(),
    };
    let Ok(Value::Object(doc)) = serde_json::from_str::<Value>(&text) else {
        return unreadable();
    };
    let schema = doc.get("schema_version").and_then(Value::as_u64).unwrap_or(0);
    let slot = |key: &str, fallback: Flavor| {
        doc.get(key).and_then(Value::as_str).and_then(Flavor::from_name).unwrap_or(fallback)
    };
    let prefs =
        Prefs { dark: slot("dark", Flavor::Graphite), light: slot("light", Flavor::Chalk), doc };
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
    let loaded = prefs_path().map(|p| load(&p)).unwrap_or_else(|| Loaded {
        prefs: Prefs::default(),
        write_barred: false,
        notice: None,
    });
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
