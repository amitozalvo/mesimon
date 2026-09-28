//! The OS's light/dark appearance, asked on a thread (T-485).
//!
//! The board used to learn its ground by asking the TERMINAL (`OSC 11`),
//! and the live re-ask was the bug: every query was a write to the tty and a
//! timed read back, and a reply that came in after the budget landed on
//! stdin as keystrokes — `1 1 ; r …` walked the keymap, tagged a card twice
//! and opened rename with the colour in the field. Twice shipped, twice
//! leaked, and the watch went behind `MESIMON_GROUND_WATCH=1` and then away.
//!
//! This is the road that cannot do that. The OS is asked, never the
//! terminal: a subprocess on a thread of its own, whose answer arrives on a
//! channel, and nothing is ever written to or read from the terminal. The
//! cost is the one the design record named — a terminal pinned to a dark
//! profile does not follow the OS — which is why the watch is OPT-IN
//! (`Prefs::follow_os`, the Settings row) and the row says so: you turn it
//! on because your terminal follows the OS.
//!
//! The one-shot startup query to the terminal stays for a board that does
//! not follow the OS; a board that does skips it (the OS answered, and a
//! second exposure buys nothing).
//!
//! Rungs: macOS reads `AppleInterfaceStyle` through `defaults` (the key is
//! absent while the appearance is light — that is how macOS spells light).
//! Linux asks the XDG desktop portal for `color-scheme` (the cross-desktop
//! answer; `gdbus`, then `busctl`) and falls back to GNOME's own setting.
//! A missing tool is the next rung; a rung that hangs is bounded and
//! killed. Anything else is `None`: no opinion, the ground stays.

use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use crate::theme::Ground;

/// How often the OS is re-asked. Slow on purpose: an appearance flip is a
/// once-a-day event, and each ask is a short subprocess.
pub(crate) const EVERY: Duration = Duration::from_secs(3);

/// The longest one rung may take before it is killed. A stalled `defaults`
/// or a dbus with nobody home must not hold the thread — or, for the
/// startup ask, the launch.
const BUDGET: Duration = Duration::from_secs(1);

/// What asks the OS. A plain fn so a test can hand the watch an answer of
/// its own; `App` carries it as a field set by `lib.rs::run`, never by
/// `App::new`, so no test spawns a subprocess by accident.
pub(crate) type Probe = fn() -> Option<Ground>;

/// Ask the OS once, now. Bounded by `BUDGET` per rung.
pub(crate) fn probe() -> Option<Ground> {
    #[cfg(target_os = "macos")]
    {
        let out =
            run(Command::new("/usr/bin/defaults").args(["read", "-g", "AppleInterfaceStyle"]))?;
        parse_defaults(out.ok, &out.stdout, &out.stderr)
    }
    #[cfg(target_os = "linux")]
    {
        let portal =
            ["org.freedesktop.portal.Settings", "org.freedesktop.appearance", "color-scheme"];
        if let Some(out) = run(Command::new("gdbus").args([
            "call",
            "--session",
            "--timeout=1",
            "--dest",
            "org.freedesktop.portal.Desktop",
            "--object-path",
            "/org/freedesktop/portal/desktop",
            "--method",
            &format!("{}.Read", portal[0]),
            portal[1],
            portal[2],
        ])) {
            if out.ok {
                if let Some(g) = parse_portal(&out.stdout) {
                    return Some(g);
                }
            }
        }
        if let Some(out) = run(Command::new("busctl").args([
            "--user",
            "--timeout=1",
            "call",
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            portal[0],
            "Read",
            "ss",
            portal[1],
            portal[2],
        ])) {
            if out.ok {
                if let Some(g) = parse_portal(&out.stdout) {
                    return Some(g);
                }
            }
        }
        let out = run(Command::new("gsettings").args([
            "get",
            "org.gnome.desktop.interface",
            "color-scheme",
        ]))?;
        if out.ok {
            parse_gsettings(&out.stdout)
        } else {
            None
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

struct Out {
    ok: bool,
    stdout: String,
    stderr: String,
}

/// Spawn, wait at most `BUDGET`, reap. `None` when the tool is not there or
/// would not finish — both "no opinion", and the next rung's turn.
fn run(cmd: &mut Command) -> Option<Out> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("DBUS_STARTER_ADDRESS")
        .spawn()
        .ok()?;
    let deadline = Instant::now() + BUDGET;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    // The child has exited and its output is short (a word, a line), so it
    // sits whole in the pipe buffers; nothing here can block.
    let out = child.wait_with_output().ok()?;
    Some(Out {
        ok: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

/// `defaults read -g AppleInterfaceStyle`: `Dark` on success; on a light
/// appearance the key does not exist and the tool exits 1 saying so. Any
/// other failure (a sandbox, a broken cfprefsd) is no opinion.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_defaults(ok: bool, stdout: &str, stderr: &str) -> Option<Ground> {
    if ok {
        let word = stdout.trim();
        if word.eq_ignore_ascii_case("dark") {
            Some(Ground::Dark)
        } else if word.eq_ignore_ascii_case("light") {
            Some(Ground::Light)
        } else {
            None
        }
    } else if stderr.contains("does not exist") {
        Some(Ground::Light)
    } else {
        None
    }
}

/// The portal's `color-scheme`: `0` no preference, `1` prefer dark, `2`
/// prefer light. `gdbus` prints `(<<uint32 1>>,)` and `busctl` prints
/// `v v u 1`; the last number on the line is the answer either way.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_portal(stdout: &str) -> Option<Ground> {
    let last = stdout.split(|c: char| !c.is_ascii_digit()).rfind(|s| !s.is_empty())?;
    match last {
        "1" => Some(Ground::Dark),
        "2" => Some(Ground::Light),
        _ => None,
    }
}

/// GNOME's `color-scheme`: `'prefer-dark'`, `'prefer-light'` or `'default'`
/// (no opinion — GNOME 42's light is spelled `default`, so it cannot be
/// read as light without reading a themed dark shell as light too).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_gsettings(stdout: &str) -> Option<Ground> {
    let s = stdout.trim();
    if s.contains("prefer-dark") {
        Some(Ground::Dark)
    } else if s.contains("prefer-light") {
        Some(Ground::Light)
    } else {
        None
    }
}

/// The live half: a thread asking `probe` every `EVERY` and sending each
/// CHANGE of answer down a channel the board drains from `App::tick`. No
/// tty, no blocking read, no armed terminal state — dropping the handle
/// ends the thread on its next beat.
pub(crate) struct Watch {
    rx: Receiver<Ground>,
    /// Dropped with the handle; the thread's `recv_timeout` then returns
    /// `Disconnected` and it exits.
    _stop: Sender<()>,
}

impl Watch {
    /// Start asking. `None` if the thread could not be spawned — the board
    /// then keeps its launch ground, which is what "off" means too.
    pub(crate) fn start(probe: Probe, every: Duration) -> Option<Self> {
        let (tx, rx) = channel();
        let (stop, stop_rx) = channel::<()>();
        std::thread::Builder::new()
            .name("mesimon-appearance".into())
            .spawn(move || {
                let mut last = None;
                loop {
                    if let Some(g) = probe() {
                        if last != Some(g) {
                            last = Some(g);
                            if tx.send(g).is_err() {
                                return;
                            }
                        }
                    }
                    match stop_rx.recv_timeout(every) {
                        Err(RecvTimeoutError::Timeout) => {}
                        _ => return,
                    }
                }
            })
            .ok()?;
        Some(Self { rx, _stop: stop })
    }

    /// Whatever the thread has said since the last call, last answer wins.
    /// Never blocks.
    pub(crate) fn take(&self) -> Option<Ground> {
        let mut latest = None;
        while let Ok(g) = self.rx.try_recv() {
            latest = Some(g);
        }
        latest
    }

    /// A watch a test feeds by hand: no thread, no subprocess.
    #[cfg(test)]
    pub(crate) fn seeded() -> (Self, Sender<Ground>) {
        let (tx, rx) = channel();
        let (stop, _) = channel::<()>();
        (Self { rx, _stop: stop }, tx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU8, Ordering};

    #[test]
    fn defaults_spells_light_as_a_missing_key() {
        assert_eq!(parse_defaults(true, "Dark\n", ""), Some(Ground::Dark));
        assert_eq!(
            parse_defaults(
                false,
                "",
                "2026-09-28 The domain/default pair of (kCFPreferencesAnyApplication, \
                 AppleInterfaceStyle) does not exist\n"
            ),
            Some(Ground::Light)
        );
        assert_eq!(parse_defaults(false, "", "Could not connect to cfprefsd"), None);
        assert_eq!(parse_defaults(true, "Auto\n", ""), None);
    }

    #[test]
    fn the_portal_answer_is_its_last_number_in_either_tools_spelling() {
        assert_eq!(parse_portal("(<<uint32 1>>,)\n"), Some(Ground::Dark));
        assert_eq!(parse_portal("(<<uint32 2>>,)\n"), Some(Ground::Light));
        assert_eq!(parse_portal("(<<uint32 0>>,)\n"), None, "no preference is no opinion");
        assert_eq!(parse_portal("v v u 1\n"), Some(Ground::Dark));
        assert_eq!(parse_portal("v v u 2\n"), Some(Ground::Light));
        assert_eq!(parse_portal(""), None);
        assert_eq!(parse_portal("Error: no such interface"), None);
    }

    #[test]
    fn gnome_default_is_no_opinion() {
        assert_eq!(parse_gsettings("'prefer-dark'\n"), Some(Ground::Dark));
        assert_eq!(parse_gsettings("'prefer-light'\n"), Some(Ground::Light));
        assert_eq!(parse_gsettings("'default'\n"), None);
    }

    static ANSWER: AtomicU8 = AtomicU8::new(0);

    fn scripted() -> Option<Ground> {
        match ANSWER.load(Ordering::SeqCst) {
            1 => Some(Ground::Dark),
            2 => Some(Ground::Light),
            _ => None,
        }
    }

    /// The thread reports each CHANGE and nothing else, and a silence in
    /// between is not a change.
    #[test]
    fn the_watch_reports_changes_only_and_stops_with_its_handle() {
        ANSWER.store(1, Ordering::SeqCst);
        let w = Watch::start(scripted, Duration::from_millis(5)).expect("a thread");
        let wait = |w: &Watch| {
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                if let Some(g) = w.take() {
                    return Some(g);
                }
                if Instant::now() > deadline {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        };
        assert_eq!(wait(&w), Some(Ground::Dark), "the first answer is reported");
        ANSWER.store(0, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(w.take(), None, "no answer is not a change");
        ANSWER.store(2, Ordering::SeqCst);
        assert_eq!(wait(&w), Some(Ground::Light));
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(w.take(), None, "the same answer again is not a change");
        drop(w);
    }

    #[test]
    fn take_is_the_latest_and_never_blocks() {
        let (w, tx) = Watch::seeded();
        assert_eq!(w.take(), None);
        tx.send(Ground::Light).unwrap();
        tx.send(Ground::Dark).unwrap();
        assert_eq!(w.take(), Some(Ground::Dark));
        assert_eq!(w.take(), None);
    }
}
