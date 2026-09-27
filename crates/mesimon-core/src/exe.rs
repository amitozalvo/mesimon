//! Where THIS binary is, answered once and kept.
//!
//! Every path that names our own executable — the `U` reload's exec, the
//! update watch, the hook set every spawned claude embeds, the daemon respawn,
//! the bundled tmux's sibling lookup, `doctor`'s `binary` line — used to ask
//! `std::env::current_exe()` at the moment it needed it. On macOS that is the
//! path the process was started by and it never changes. On Linux it is
//! `readlink /proc/self/exe`, and the kernel answers for the INODE: the moment
//! `install.sh` (or the release checker) renames a new file over ours, the
//! answer becomes `/home/x/.local/bin/mesimon (deleted)` — a name nothing on
//! disk has. The reload watched the real path, saw the new binary land, and
//! then exec'd the deleted one: `exec of the new binary failed: No such file
//! or directory` (T-280, WSL, 2026-09-06). A daemon that outlived an install
//! would have written the same dead name into every new session's hooks.
//!
//! So: the path is resolved on the FIRST call and cached for the life of the
//! process (both the TUI and the daemon ask at startup), and a ` (deleted)`
//! suffix is stripped in any case — the file that name points at now is the
//! new binary, which is exactly what a reload wants to exec and a hook set
//! wants to name. A test seam (`MESIMON_HOOK_BIN`, `MESIMON_DAEMON_BIN`) still
//! outranks it at each caller; this is only the fallback they share.
//!
//! A Homebrew keg is the same trap one directory up (T-463). Linux answers
//! with the keg's own path, `<prefix>/Cellar/mesimon/<version>/bin/mesimon`,
//! and `brew upgrade` deletes that directory once the next version is linked.
//! So a keg path is taken through `<prefix>/opt/mesimon`, the link brew keeps
//! pointing at the linked keg — see [`keg_opt_path`]. macOS answers with the
//! `bin/` link the shell ran, which survives an upgrade as it is.

use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

static OWN: OnceLock<Option<PathBuf>> = OnceLock::new();

/// This process's executable, resolved once. `Err` only when the OS could not
/// say at the first ask — the same failure `std::env::current_exe` reports.
pub fn current_exe() -> io::Result<PathBuf> {
    OWN.get_or_init(|| std::env::current_exe().ok().map(|p| through_opt(undeleted(&p))))
        .clone()
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "the path of this binary is unknown")
        })
}

/// Linux's `/proc/self/exe` answers `<path> (deleted)` once the inode we run
/// from was unlinked — after an install renamed a new file over it. The path
/// under the suffix is where the new binary now is.
pub fn undeleted(p: &Path) -> PathBuf {
    const SUFFIX: &str = " (deleted)";
    match p.to_str() {
        Some(s) if s.ends_with(SUFFIX) => PathBuf::from(&s[..s.len() - SUFFIX.len()]),
        _ => p.to_path_buf(),
    }
}

/// A keg path taken through brew's `opt/` link, when that link names this
/// very file. A keg run by its own path while another version is linked keeps
/// its own path: the link would name a different binary.
fn through_opt(p: PathBuf) -> PathBuf {
    let same = |a: &Path, b: &Path| match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    match keg_opt_path(&p) {
        Some(opt) if same(&opt, &p) => opt,
        _ => p,
    }
}

/// `<prefix>/Cellar/<formula>/<version>/<rest>` as
/// `<prefix>/opt/<formula>/<rest>`, or `None` for a path inside no Homebrew
/// keg. Pure. A caller asking whether a binary is brew's canonicalizes first,
/// because on macOS the path a process was started by is the `bin/` link.
pub fn keg_opt_path(p: &Path) -> Option<PathBuf> {
    let parts: Vec<Component<'_>> = p.components().collect();
    let cellar = parts.iter().position(|c| c.as_os_str() == "Cellar")?;
    let [formula, _version, rest @ ..] = &parts[cellar + 1..] else { return None };
    if rest.is_empty() {
        return None;
    }
    let mut opt: PathBuf = parts[..cellar].iter().collect();
    opt.push("opt");
    opt.push(formula);
    opt.extend(rest);
    Some(opt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_replaced_binary_still_names_its_path() {
        assert_eq!(
            undeleted(Path::new("/home/x/.local/bin/mesimon (deleted)")),
            PathBuf::from("/home/x/.local/bin/mesimon")
        );
        assert_eq!(
            undeleted(Path::new("/usr/local/bin/mesimon")),
            PathBuf::from("/usr/local/bin/mesimon")
        );
        // Only the suffix goes, never a name that merely contains the word.
        assert_eq!(
            undeleted(Path::new("/tmp/(deleted)/mesimon")),
            PathBuf::from("/tmp/(deleted)/mesimon")
        );
    }

    /// The fix is only as wide as its callers. This walks the workspace source
    /// and asserts that no code line outside this module asks the OS directly —
    /// a raw `std::env::current_exe()` anywhere else is the Linux `(deleted)`
    /// trap coming back under a new name.
    #[test]
    fn no_source_line_asks_the_os_for_the_exe_directly() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .expect("workspace root")
            .join("crates");
        let mut seen_self = false;
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.flatten() {
                let path = e.path();
                if path.is_dir() {
                    // An e2e re-spawning its own test binary is not shipped
                    // code, and a test binary is never replaced under itself.
                    if path.file_name().is_some_and(|n| n == "tests") {
                        continue;
                    }
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|x| x.to_str()) != Some("rs") {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&path) else { continue };
                for (i, line) in text.lines().enumerate() {
                    let code = line.trim_start();
                    if code.starts_with("//") || code.starts_with('*') {
                        continue;
                    }
                    if !code.contains("std::env::current_exe") {
                        continue;
                    }
                    if path.ends_with("mesimon-core/src/exe.rs") {
                        seen_self = true;
                        continue;
                    }
                    panic!("{}:{}: asks std::env::current_exe directly; use mesimon_core::exe::current_exe", path.display(), i + 1);
                }
            }
        }
        assert!(seen_self, "the grep found no current_exe at all — it has stopped working");
    }

    #[test]
    fn a_keg_path_is_named_through_brews_opt_link() {
        assert_eq!(
            keg_opt_path(Path::new(
                "/home/linuxbrew/.linuxbrew/Cellar/mesimon/0.1.0-alpha.28/bin/mesimon"
            )),
            Some(PathBuf::from("/home/linuxbrew/.linuxbrew/opt/mesimon/bin/mesimon"))
        );
        assert_eq!(
            keg_opt_path(Path::new("/opt/homebrew/Cellar/mesimon/0.1.0-alpha.28/bin/mesimon-tmux")),
            Some(PathBuf::from("/opt/homebrew/opt/mesimon/bin/mesimon-tmux"))
        );
        // Not a keg: the link brew makes, install.sh's path, and a Cellar
        // path too short to hold a file inside a version.
        assert_eq!(keg_opt_path(Path::new("/opt/homebrew/bin/mesimon")), None);
        assert_eq!(keg_opt_path(Path::new("/Users/x/.local/bin/mesimon")), None);
        assert_eq!(keg_opt_path(Path::new("/opt/homebrew/Cellar/mesimon/0.1.0")), None);
    }

    /// The link is taken only when it names the same file, so a keg laid out
    /// on disk here stands in for brew's: `opt/mesimon` points at one version,
    /// and a binary run from the other version keeps its own path.
    #[test]
    fn the_opt_link_is_taken_only_for_the_linked_keg() {
        let root = std::env::temp_dir().join(format!("msmn-keg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for v in ["1.0.0", "2.0.0"] {
            let bin = root.join("Cellar/mesimon").join(v).join("bin");
            std::fs::create_dir_all(&bin).expect("keg");
            std::fs::write(bin.join("mesimon"), v).expect("binary");
        }
        std::fs::create_dir_all(root.join("opt")).expect("opt");
        std::os::unix::fs::symlink("../Cellar/mesimon/2.0.0", root.join("opt/mesimon"))
            .expect("link");

        let linked = root.join("Cellar/mesimon/2.0.0/bin/mesimon");
        assert_eq!(through_opt(linked), root.join("opt/mesimon/bin/mesimon"));
        let other = root.join("Cellar/mesimon/1.0.0/bin/mesimon");
        assert_eq!(through_opt(other.clone()), other);
        let plain = root.join("bin/mesimon");
        assert_eq!(through_opt(plain.clone()), plain);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_answer_is_a_file_that_exists_and_is_stable() {
        let first = current_exe().expect("a test binary knows its own path");
        assert!(first.is_file(), "{}", first.display());
        assert_eq!(current_exe().expect("cached"), first);
    }
}
