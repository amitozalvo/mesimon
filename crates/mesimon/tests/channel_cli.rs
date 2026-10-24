//! `mesimon channel` (T-946), the real binary under a scratch HOME: an
//! explicit choice exits 0 only when that word is what `prefs.json` holds
//! afterwards. No daemon, no tmux.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("msmn-channel-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".local/state/mesimon")).expect("scratch home");
    dir
}

fn prefs(home: &Path) -> PathBuf {
    home.join(".local/state/mesimon/prefs.json")
}

fn channel(home: &Path, args: &[&str]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_mesimon"))
        .arg("channel")
        .args(args)
        .env_clear()
        .env("HOME", home)
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run mesimon channel");
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    (out.status.code().unwrap_or(-1), text)
}

fn on_disk(home: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(prefs(home)).expect("prefs.json")).expect("json")
}

/// The reproduction from the second read: `nightly` on disk, `channel
/// unstable` asked. It must exit 0 with `unstable` saved, and a key this
/// build does not know is kept beside it.
#[test]
fn an_explicit_choice_replaces_an_unfamiliar_word_and_reads_back() {
    let home = scratch("unfamiliar");
    std::fs::write(
        prefs(&home),
        r#"{"schema_version":1,"release_channel":"nightly","from_a_newer_build":7}"#,
    )
    .expect("seed");
    let (code, said) = channel(&home, &["unstable"]);
    assert_eq!(code, 0, "{said}");
    assert!(said.contains("follows unstable"), "{said}");
    let v = on_disk(&home);
    assert_eq!(v["release_channel"], "unstable", "{v}");
    assert_eq!(v["from_a_newer_build"], 7, "{v}");

    let (code, said) = channel(&home, &["stable"]);
    assert_eq!((code, on_disk(&home)["release_channel"].clone()), (0, "stable".into()), "{said}");
    // Asking changes nothing.
    let (code, said) = channel(&home, &[]);
    assert_eq!(code, 0);
    assert!(said.contains("follows stable"), "{said}");
    let _ = std::fs::remove_dir_all(home);
}

/// A file a newer build wrote is never written, and the command says so
/// with a failure, never a 0 for a choice that is not on disk.
#[test]
fn a_choice_that_cannot_be_saved_exits_non_zero() {
    let home = scratch("barred");
    let newer = r#"{"schema_version":99,"release_channel":"stable"}"#;
    std::fs::write(prefs(&home), newer).expect("seed");
    let (code, said) = channel(&home, &["unstable"]);
    assert_eq!(code, 1, "{said}");
    assert!(said.contains("newer mesimon"), "{said}");
    assert_eq!(std::fs::read_to_string(prefs(&home)).expect("read"), newer, "it was written");
    let (code, _) = channel(&home, &["nightly"]);
    assert_eq!(code, 2, "an unknown word is a usage error");
    let _ = std::fs::remove_dir_all(home);
}
