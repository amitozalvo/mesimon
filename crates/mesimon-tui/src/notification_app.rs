//! Mesimon's own notification identity on macOS, in one of two bundles.
//!
//! macOS 26 ignores terminal-notifier 2's private `-appIcon` override. A
//! signed bundle with its own icon works; no sender spoofing is involved.
//! Where terminal-notifier is installed, that bundle is a private copy of it.
//! Where it is not, the `osascript` rung's banner would wear Script Editor's
//! icon (T-605), so the same script is compiled by `osacompile` into a
//! Mesimon applet instead. Discovery is read-only. Preparation runs only for
//! an actual banner.

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

const EXECUTABLE: &str = "Contents/MacOS/terminal-notifier";
const INFO: &str = "Contents/Info.plist";
const ICON: &str = "Contents/Resources/Mesimon.icns";
const APPLET: &str = "Contents/MacOS/applet";

/// The applet's bundle id. Not terminal-notifier copy's: two bundles under
/// one id would leave LaunchServices to pick which one it is.
const APPLET_ID: &str = "io.mesimon.notifications.applet";

/// The applet's whole program. The words ride argv as in the `osascript`
/// rung, and each arrives behind a `=` that is stripped here: AppKit reads a
/// launch argument that begins with `-` as a defaults key, so no word may
/// begin with one. Two words are title and body, three put a subtitle
/// between them.
pub(crate) const APPLET_SCRIPT: &str = r#"use AppleScript version "2.4"
use framework "Foundation"
use scripting additions
on word_of(w)
	if length of w > 1 then return text 2 thru -1 of w
	return ""
end word_of
on run
	set argv to rest of ((current application's NSProcessInfo's processInfo's arguments()) as list)
	if (count of argv) is 3 then
		display notification word_of(item 3 of argv) with title word_of(item 1 of argv) subtitle word_of(item 2 of argv)
	else if (count of argv) is 2 then
		display notification word_of(item 2 of argv) with title word_of(item 1 of argv)
	end if
end run
"#;

/// Covers both a native bundle executable and Homebrew's prefix/bin wrapper.
pub(crate) fn discover(program: &Path) -> Option<PathBuf> {
    let program = program.canonicalize().ok()?;
    program.ancestors().skip(1).take(4).find_map(|dir| {
        [dir.to_path_buf(), dir.join("terminal-notifier.app")].into_iter().find(|app| {
            app.extension().is_some_and(|e| e == "app")
                && app.join(INFO).is_file()
                && app.join(EXECUTABLE).is_file()
        })
    })
}

pub(crate) fn prepare(home: &Path, source: &Path) -> io::Result<PathBuf> {
    prepare_with(home, source, seal)
}

struct File {
    relative: PathBuf,
    bytes: Vec<u8>,
    mode: u32,
}

/// Snapshot ordinary files only. In particular, copying a symlink and then
/// editing Info.plist through it must never rewrite the installed helper.
fn collect(root: &Path, relative: &Path, files: &mut Vec<File>) -> io::Result<()> {
    let path = root.join(relative);
    let meta = fs::symlink_metadata(&path)?;
    if meta.is_dir() {
        let mut entries = fs::read_dir(&path)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            collect(root, &relative.join(entry.file_name()), files)?;
        }
    } else if meta.is_file() {
        files.push(File {
            relative: relative.into(),
            bytes: fs::read(path)?,
            mode: meta.permissions().mode() & 0o777,
        });
    } else {
        return Err(io::Error::other("notification app contains a symlink or special file"));
    }
    Ok(())
}

fn prepare_with(
    home: &Path,
    source: &Path,
    seal: impl FnOnce(&Path) -> io::Result<()>,
) -> io::Result<PathBuf> {
    let mut files = Vec::new();
    collect(source, Path::new(""), &mut files)?;
    if !files.iter().any(|f| f.relative == Path::new(EXECUTABLE) && f.mode & 0o111 != 0)
        || !files.iter().any(|f| f.relative == Path::new(INFO))
    {
        return Err(io::Error::other("notification helper is not an executable app bundle"));
    }
    // Include every input, including resources and permissions. Concurrent
    // boards share a complete, immutable generation; an upgraded helper gets
    // a new generation without replacing a running notification process.
    let icon = crate::mascot::app_icon();
    let mut hash = Sha256::new();
    hash.update(b"mesimon-notification-app-v1\0");
    hash.update(&icon);
    for file in &files {
        hash.update(file.relative.as_os_str().as_encoded_bytes());
        hash.update([0]);
        hash.update(file.mode.to_be_bytes());
        hash.update((file.bytes.len() as u64).to_be_bytes());
        hash.update(&file.bytes);
    }
    let generation = home.join(format!("app-{:x}", hash.finalize()));
    publish(home, &generation, EXECUTABLE, &icon, |staged_app| {
        for file in files {
            let path = staged_app.join(file.relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&path, file.bytes)?;
            fs::set_permissions(path, fs::Permissions::from_mode(file.mode))?;
        }
        fs::create_dir_all(staged_app.join("Contents/Resources"))?;
        fs::write(staged_app.join(ICON), &icon)?;
        seal(staged_app)
    })
}

/// The applet for a Mac without terminal-notifier: [`APPLET_SCRIPT`]
/// compiled by the system's own `osacompile`, given the mascot and
/// Mesimon's name, kept out of the Dock and ad-hoc signed.
pub(crate) fn prepare_applet(home: &Path) -> io::Result<PathBuf> {
    prepare_applet_with(home, build_applet)
}

fn prepare_applet_with(
    home: &Path,
    build: impl FnOnce(&Path, &[u8]) -> io::Result<()>,
) -> io::Result<PathBuf> {
    let icon = crate::mascot::app_icon();
    let mut hash = Sha256::new();
    hash.update(b"mesimon-notification-applet-v1\0");
    hash.update(&icon);
    hash.update(APPLET_SCRIPT);
    let generation = home.join(format!("applet-{:x}", hash.finalize()));
    publish(home, &generation, APPLET, &icon, |staged_app| build(staged_app, &icon))
}

fn build_applet(app: &Path, icon: &[u8]) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let parent = app.parent().ok_or_else(|| io::Error::other("applet has no parent"))?;
    let source = parent.join("notify.applescript");
    fs::write(&source, APPLET_SCRIPT)?;
    run(Command::new("/usr/bin/osacompile").arg("-o").arg(app).arg(&source), deadline)?;
    fs::remove_file(source)?;
    // The compiled applet names its own icon twice: an asset catalog, which
    // outranks the icon file, and the icon file itself.
    let resources = app.join("Contents/Resources");
    for stock in ["Assets.car", "applet.icns"] {
        match fs::remove_file(resources.join(stock)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    fs::write(app.join(ICON), icon)?;
    let plist = app.join(INFO);
    let _ = run(
        Command::new("/usr/bin/plutil").args(["-remove", "CFBundleIconName"]).arg(&plist),
        deadline,
    );
    // An agent (no Dock icon, no menu bar) for the second it runs.
    run(
        Command::new("/usr/bin/plutil")
            .args(["-replace", "LSUIElement", "-bool", "true"])
            .arg(&plist),
        deadline,
    )?;
    seal_as(app, APPLET_ID, deadline)
}

/// Stage a bundle, then publish it whole as `generation`. A generation is
/// named by its inputs, so a published one is reused as it is, and two
/// boards building the same one at once converge on it.
fn publish(
    home: &Path,
    generation: &Path,
    executable: &str,
    icon: &[u8],
    build: impl FnOnce(&Path) -> io::Result<()>,
) -> io::Result<PathBuf> {
    let app = generation.join("Mesimon.app");
    let ready = || {
        fs::symlink_metadata(generation).is_ok_and(|m| m.is_dir())
            && app.join(executable).is_file()
            && fs::read(app.join(ICON)).is_ok_and(|bytes| bytes == icon)
            && generation.join("ready").is_file()
    };
    mesimon_daemon::paths::own_private_dir(home).map_err(io::Error::other)?;
    if ready() {
        return Ok(app.join(executable));
    }
    let stage = home.join(format!(".app-{}", uuid::Uuid::new_v4()));
    mesimon_daemon::paths::own_private_dir(&stage).map_err(io::Error::other)?;
    let result = (|| {
        build(&stage.join("Mesimon.app"))?;
        fs::write(stage.join("ready"), b"signed\n")?;
        match fs::rename(&stage, generation) {
            Ok(()) => Ok(app.join(executable)),
            // Another board may have published this same generation first.
            Err(_) if ready() => Ok(app.join(executable)),
            Err(e) => Err(e),
        }
    })();
    if stage.exists() {
        fs::remove_dir_all(&stage)?;
    }
    result
}

fn seal(app: &Path) -> io::Result<()> {
    seal_as(app, "io.mesimon.notifications", Instant::now() + Duration::from_secs(10))
}

fn seal_as(app: &Path, id: &str, deadline: Instant) -> io::Result<()> {
    for (key, value) in [
        ("CFBundleIdentifier", id),
        ("CFBundleName", "Mesimon"),
        ("CFBundleDisplayName", "Mesimon"),
        ("CFBundleIconFile", "Mesimon"),
    ] {
        run(
            Command::new("/usr/bin/plutil")
                .args(["-replace", key, "-string", value])
                .arg(app.join(INFO)),
            deadline,
        )?;
    }
    run(Command::new("/usr/bin/codesign").args(["--force", "--sign", "-"]).arg(app), deadline)?;
    run(Command::new("/usr/bin/codesign").args(["--verify", "--strict"]).arg(app), deadline)
}

/// Bound first-use setup; a stalled system utility may not hold the board's
/// notification worker forever. Every child is reaped, including on errors.
fn run(cmd: &mut Command, deadline: Instant) -> io::Result<()> {
    let program = cmd.get_program().to_string_lossy().into_owned();
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => return Err(io::Error::other(format!("{program}: {status}"))),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            result => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(result.err().unwrap_or_else(|| {
                    io::Error::new(io::ErrorKind::TimedOut, format!("{program} timed out"))
                }));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir()
                .join(format!("msmn-notification-app-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&root).unwrap();
            Self(root.canonicalize().unwrap())
        }
        fn source(&self) -> PathBuf {
            let source = self.0.join("source/terminal-notifier.app");
            fs::create_dir_all(source.join("Contents/MacOS")).unwrap();
            fs::write(source.join(EXECUTABLE), b"#!/bin/sh\nexit 0\n").unwrap();
            fs::set_permissions(source.join(EXECUTABLE), fs::Permissions::from_mode(0o755))
                .unwrap();
            fs::write(
                source.join(INFO),
                br#"<plist version="1.0"><dict>
                <key>CFBundleExecutable</key><string>terminal-notifier</string>
                <key>CFBundlePackageType</key><string>APPL</string>
            </dict></plist>"#,
            )
            .unwrap();
            source
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn discovery_handles_bundles_and_homebrew_without_preparing_anything() {
        let f = Fixture::new();
        let source = f.source();
        assert_eq!(discover(&source.join(EXECUTABLE)), Some(source.clone()));
        let wrapper = f.0.join("source/bin/terminal-notifier");
        fs::create_dir_all(wrapper.parent().unwrap()).unwrap();
        fs::write(&wrapper, "wrapper").unwrap();
        assert_eq!(discover(&wrapper), Some(source));
        assert_eq!(discover(&f.0.join("missing")), None);
        assert!(!f.0.join("notifications").exists());
    }

    #[test]
    fn preparation_is_private_reused_and_invalidated_by_a_helper_upgrade() {
        let f = Fixture::new();
        let source = f.source();
        let home = f.0.join("notifications");
        let before = fs::read(source.join(INFO)).unwrap();
        let first =
            prepare_with(&home, &source, |app| fs::write(app.join(INFO), "Mesimon identity"))
                .unwrap();
        assert!(first.starts_with(&home));
        assert_eq!(fs::read(source.join(INFO)).unwrap(), before);
        assert_eq!(fs::read(source.join(EXECUTABLE)).unwrap(), fs::read(&first).unwrap());
        assert_eq!(
            prepare_with(&home, &source, |_| panic!("must reuse a sealed app")).unwrap(),
            first
        );
        fs::write(source.join("Contents/new-resource"), b"new helper version").unwrap();
        let second = prepare_with(&home, &source, |_| Ok(())).unwrap();
        assert_ne!(first, second);
        assert!(first.exists(), "an in-flight old notification keeps its executable");
        assert_eq!(fs::metadata(home).unwrap().permissions().mode() & 0o777, 0o700);
    }

    #[test]
    fn a_failed_signature_never_publishes_a_bundle() {
        let f = Fixture::new();
        let source = f.source();
        let home = f.0.join("notifications");
        assert!(prepare_with(&home, &source, |_| Err(io::Error::other("sign failed"))).is_err());
        assert_eq!(fs::read_dir(&home).unwrap().count(), 0, "the failed stage is removed");
        assert!(prepare_with(&home, &source, |_| Ok(())).is_ok(), "a retry can recover");
    }

    #[test]
    fn a_source_symlink_cannot_redirect_a_bundle_write() {
        let f = Fixture::new();
        let source = f.source();
        let outside = f.0.join("untouched");
        fs::write(&outside, b"user data").unwrap();
        fs::remove_file(source.join(INFO)).unwrap();
        std::os::unix::fs::symlink(&outside, source.join(INFO)).unwrap();
        assert!(prepare_with(&f.0.join("notifications"), &source, |_| Ok(())).is_err());
        assert_eq!(fs::read(&outside).unwrap(), b"user data");
    }

    #[test]
    fn simultaneous_boards_publish_one_complete_generation() {
        let f = Fixture::new();
        let source = f.source();
        let home = f.0.join("notifications");
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            let build = || {
                prepare_with(&home, &source, |_| {
                    barrier.wait();
                    Ok(())
                })
                .unwrap()
            };
            let a = scope.spawn(build);
            let b = scope.spawn(build);
            assert_eq!(a.join().unwrap(), b.join().unwrap());
        });
        assert_eq!(fs::read_dir(home).unwrap().count(), 1);
    }

    #[test]
    fn a_stalled_setup_child_is_bounded_and_reaped() {
        let start = Instant::now();
        let error = run(Command::new("/bin/sleep").arg("10"), start + Duration::from_millis(30))
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn the_applet_is_reused_and_a_failed_build_publishes_nothing() {
        let f = Fixture::new();
        let home = f.0.join("notifications");
        assert!(prepare_applet_with(&home, |_, _| Err(io::Error::other("no osacompile"))).is_err());
        assert_eq!(fs::read_dir(&home).unwrap().count(), 0, "the failed stage is removed");
        let build = |app: &Path, icon: &[u8]| {
            fs::create_dir_all(app.join("Contents/MacOS"))?;
            fs::create_dir_all(app.join("Contents/Resources"))?;
            fs::write(app.join(APPLET), b"applet")?;
            fs::write(app.join(ICON), icon)
        };
        let first = prepare_applet_with(&home, build).unwrap();
        assert!(first.starts_with(&home) && first.ends_with(APPLET));
        assert_eq!(
            prepare_applet_with(&home, |_, _| panic!("must reuse a sealed applet")).unwrap(),
            first
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_real_osacompile_builds_a_mascot_applet_without_posting() {
        let f = Fixture::new();
        let program = prepare_applet(&f.0.join("notifications")).unwrap();
        let app = program.parent().unwrap().parent().unwrap().parent().unwrap();
        let plist = fs::read_to_string(app.join(INFO)).unwrap();
        assert!(plist.contains(APPLET_ID));
        assert!(plist.contains("LSUIElement"));
        assert!(!plist.contains("CFBundleIconName"), "an asset catalog name outranks the icns");
        assert!(!app.join("Contents/Resources/Assets.car").exists());
        assert_eq!(fs::read(app.join(ICON)).unwrap(), crate::mascot::app_icon());
        assert!(app.join("Contents/Resources/Scripts/main.scpt").is_file());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_real_macos_tools_seal_a_mascot_bundle_without_posting() {
        let f = Fixture::new();
        let source = f.source();
        // A real Mach-O for codesign, but no notification helper is launched.
        fs::copy("/usr/bin/true", source.join(EXECUTABLE)).unwrap();
        let program = prepare(&f.0.join("notifications"), &source).unwrap();
        let app = program.parent().unwrap().parent().unwrap().parent().unwrap();
        let plist = fs::read_to_string(app.join(INFO)).unwrap();
        assert!(plist.contains("io.mesimon.notifications"));
        assert!(plist.contains("CFBundleIconFile"));
        let icon = fs::read(app.join(ICON)).unwrap();
        assert_eq!(&icon[..4], b"icns");
        assert_eq!(u32::from_be_bytes(icon[4..8].try_into().unwrap()) as usize, icon.len());
        assert_eq!(&icon[8..12], b"ic08");
    }
}
