//! Explicit board copies use the local desktop clipboard. SSH must never
//! write the remote machine's clipboard; it uses an unacknowledged OSC 52
//! request instead. No terminal or tmux configuration is changed.

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, PartialEq, Eq)]
struct Native {
    program: PathBuf,
    args: Vec<&'static str>,
    utf16: bool,
}

fn select(
    remote: bool,
    mac: bool,
    wsl: bool,
    wayland: bool,
    x11: bool,
    which: impl Fn(&str) -> Option<PathBuf>,
) -> Option<Native> {
    if remote {
        return None;
    }
    let candidates: &[(&str, &[&str], bool)] = if mac {
        &[("pbcopy", &[], false)]
    } else if wsl {
        &[("clip.exe", &[], true)]
    } else if wayland {
        &[("wl-copy", &[], false)]
    } else if x11 {
        &[
            ("xclip", &["-selection", "clipboard"], false),
            ("xsel", &["--clipboard", "--input"], false),
        ]
    } else {
        &[]
    };
    candidates.iter().find_map(|(name, args, utf16)| {
        which(name).map(|program| Native { program, args: args.to_vec(), utf16: *utf16 })
    })
}

fn find() -> Option<Native> {
    select(
        ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"].iter().any(|v| std::env::var_os(v).is_some()),
        cfg!(target_os = "macos"),
        crate::opener::is_wsl(),
        std::env::var_os("WAYLAND_DISPLAY").is_some(),
        std::env::var_os("DISPLAY").is_some(),
        |name| {
            crate::opener::which_on_path(name).or_else(|| {
                let path = match name {
                    "pbcopy" => PathBuf::from("/usr/bin/pbcopy"),
                    "clip.exe" => PathBuf::from("/mnt/c/Windows/System32/clip.exe"),
                    _ => return None,
                };
                path.is_file().then_some(path)
            })
        },
    )
}

fn payload(text: &str, utf16: bool) -> Vec<u8> {
    if utf16 {
        // clip.exe recognizes the BOM, preserving non-ASCII URLs and paths.
        std::iter::once(0xfeff_u16).chain(text.encode_utf16()).flat_map(u16::to_le_bytes).collect()
    } else {
        text.as_bytes().to_vec()
    }
}

fn native_copy(native: &Native, text: &str, timeout: Duration) -> io::Result<()> {
    let mut child = Command::new(&native.program)
        .args(&native.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let mut input = child.stdin.take().expect("piped stdin");
    let bytes = payload(text, native.utf16);
    // A helper that stops reading must not block the board in write_all.
    let writer = match std::thread::Builder::new()
        .name("mesimon-copy".into())
        .spawn(move || input.write_all(&bytes))
    {
        Ok(writer) => writer,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return Err(io::Error::other(format!("clipboard helper exited with {status}")));
                }
                if writer.is_finished() {
                    return writer
                        .join()
                        .map_err(|_| io::Error::other("clipboard writer failed"))?;
                }
            }
            Ok(None) => {}
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(io::ErrorKind::TimedOut, "clipboard helper timed out"));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn status(subject: &str, result: io::Result<bool>) -> String {
    match result {
        Ok(true) => format!("{subject} copied"),
        Ok(false) => {
            "copy requested from terminal ∙ if paste is empty, select the text to copy".into()
        }
        Err(error) => format!("could not copy {subject}: {error} ∙ select the text to copy"),
    }
}

pub(crate) fn copy_status(subject: &str, text: &str) -> String {
    let result = match find() {
        Some(native) => native_copy(&native, text, Duration::from_secs(2)).map(|()| true),
        None => crate::osc::copy_to_clipboard(text).map(|()| false),
    };
    status(subject, result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_the_users_desktop_and_never_the_ssh_hosts() {
        let all = |name: &str| Some(PathBuf::from(name));
        assert_eq!(select(true, true, true, true, true, all), None);
        for (mac, wsl, wayland, x11, expected) in [
            (true, false, false, false, "pbcopy"),
            (false, true, true, true, "clip.exe"),
            (false, false, true, true, "wl-copy"),
            (false, false, false, true, "xclip"),
        ] {
            assert_eq!(
                select(false, mac, wsl, wayland, x11, all).unwrap().program,
                PathBuf::from(expected)
            );
        }
        assert_eq!(select(false, false, false, false, false, all), None);
        assert_eq!(select(false, true, false, false, false, |_| None), None);
        let xsel =
            select(false, false, false, false, true, |name| (name == "xsel").then(|| name.into()))
                .unwrap();
        assert_eq!(xsel.args, ["--clipboard", "--input"]);
    }

    #[test]
    fn preserves_unicode_and_does_not_add_a_newline() {
        let text = "https://example.org/שלום?q='$(echo nope)'";
        assert_eq!(payload(text, false), text.as_bytes());
        let bytes = payload(text, true);
        assert_eq!(&bytes[..2], &[0xff, 0xfe]);
        let units: Vec<_> =
            bytes[2..].chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
        assert_eq!(String::from_utf16(&units).unwrap(), text);
    }

    #[test]
    fn only_a_native_success_claims_a_copy() {
        assert_eq!(status("link", Ok(true)), "link copied");
        assert_eq!(status("brief", Ok(true)), "brief copied");
        assert!(!status("link", Ok(false)).contains("copied"));
        assert!(
            status("link", Err(io::Error::other("failed"))).contains("could not copy link: failed")
        );
    }

    #[test]
    fn helper_receives_literal_utf8_and_reports_failure() {
        let mut native = Native {
            program: "/bin/sh".into(),
            args: vec![
                "-c",
                "IFS= read -r value; test \"$value\" = 'https://example.org/שלום?x=$(nope)'",
            ],
            utf16: false,
        };
        native_copy(&native, "https://example.org/שלום?x=$(nope)", Duration::from_secs(2)).unwrap();
        native.args = vec!["-c", "exit 7"];
        assert!(native_copy(&native, "link", Duration::from_secs(2))
            .unwrap_err()
            .to_string()
            .contains("7"));
        native.program = "/nonexistent/mesimon-clipboard".into();
        assert_eq!(
            native_copy(&native, "link", Duration::from_secs(2)).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }

    #[test]
    fn a_helper_that_never_reads_is_killed_and_reaped() {
        let native = Native {
            program: "/bin/sh".into(),
            args: vec!["-c", "while :; do :; done"],
            utf16: false,
        };
        let start = Instant::now();
        let error =
            native_copy(&native, &"x".repeat(256 * 1024), Duration::from_millis(50)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}
