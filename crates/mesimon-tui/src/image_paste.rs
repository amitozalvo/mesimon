//! Desktop clipboard reads and the image portion of an editor draft.
use anyhow::{Context, Result};
use mesimon_core::attachment::{self, DRAFT_MAX_BYTES, MAX_BYTES, MAX_PIXELS};
use std::io::{self, Cursor, Read};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};
use ulid::Ulid;

const WSL_POWERSHELL: &str = "/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe";
const WSL_HELPER_TIMEOUT: Duration = Duration::from_secs(2);
const WSL_HELPER_MAX_OUTPUT: usize = 16 * 1024 * 1024;
const POWERSHELL_IMAGE: &str = concat!(
    "$ErrorActionPreference='Stop'; ",
    "Add-Type -AssemblyName System.Windows.Forms; ",
    "$image=[Windows.Forms.Clipboard]::GetImage(); ",
    "if ($null -eq $image) { $text=[Windows.Forms.Clipboard]::GetText(); ",
    "if ([string]::IsNullOrEmpty($text)) { exit 2 }; ",
    "[Console]::Out.Write('TEXT:'+[Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($text))) } ",
    "else { if ([long]$image.Width * [long]$image.Height -gt 25000000) { exit 3 }; ",
    "$stream=New-Object System.IO.MemoryStream; ",
    "try { $image.Save($stream,[System.Drawing.Imaging.ImageFormat]::Png); ",
    "if ($stream.Length -gt 10485760) { exit 4 }; ",
    "[Console]::Out.Write('PNG:'+[Convert]::ToBase64String($stream.ToArray())) } ",
    "finally { $stream.Dispose(); $image.Dispose() } }"
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backend {
    Native,
    WslHost,
}

fn select_backend(ssh: bool, wsl: bool) -> Result<Backend> {
    anyhow::ensure!(!ssh, "image paste is unavailable over SSH; terminal text paste still works");
    Ok(if wsl { Backend::WslHost } else { Backend::Native })
}

#[derive(Debug, Clone, PartialEq)]
pub struct DraftImage {
    pub id: Ulid,
    pub number: usize,
    pub png: Option<Arc<[u8]>>,
}

pub enum Pasted {
    Text(String),
    Image(Vec<u8>),
}

pub struct Pending {
    pub started: std::time::Instant,
    pub timed_out: bool,
    pub editor: Ulid,
    pub body: crate::text::TextArea,
    pub result: mpsc::Receiver<Result<Pasted, String>>,
}

pub fn start(editor: Ulid, body: crate::text::TextArea) -> Result<Pending> {
    let backend = select_backend(
        ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"]
            .iter()
            .any(|key| std::env::var_os(key).is_some()),
        crate::opener::is_wsl(),
    )?;
    let (tx, result) = mpsc::channel();
    std::thread::Builder::new().name("mesimon-paste".into()).spawn(move || {
        let _ = tx.send(read(backend).map_err(|e| format!("{e:#}")));
    })?;
    Ok(Pending { editor, body, result, started: std::time::Instant::now(), timed_out: false })
}

fn read(backend: Backend) -> Result<Pasted> {
    if backend == Backend::WslHost {
        return read_wsl_host_clipboard();
    }
    let mut clipboard = arboard::Clipboard::new().context("desktop clipboard unavailable")?;
    match clipboard.get_image() {
        Ok(image) => Ok(Pasted::Image(encode(image)?)),
        Err(arboard::Error::ContentNotAvailable) => {
            let text =
                clipboard.get_text().context("clipboard has no supported picture or text")?;
            anyhow::ensure!(!text.is_empty(), "clipboard is empty");
            Ok(Pasted::Text(text))
        }
        Err(e) => Err(e).context("could not read clipboard picture"),
    }
}

fn powershell_program() -> Result<PathBuf> {
    crate::opener::which_on_path("powershell.exe")
        .or_else(|| {
            let path = PathBuf::from(WSL_POWERSHELL);
            path.is_file().then_some(path)
        })
        .context("Windows PowerShell was not found; check WSL Windows interop")
}

fn read_wsl_host_clipboard() -> Result<Pasted> {
    let mut command = Command::new(powershell_program()?);
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-STA",
        "-Command",
        POWERSHELL_IMAGE,
    ]);
    let output = bounded_output(command, WSL_HELPER_TIMEOUT)
        .context("could not read the Windows clipboard through WSL interop")?;
    anyhow::ensure!(
        output.status.success(),
        "Windows clipboard read failed (PowerShell exit {}){}",
        output.status.code().unwrap_or(-1),
        if output.status.code() == Some(2) {
            ": the Windows clipboard has no picture or text"
        } else if output.status.code() == Some(3) {
            ": picture exceeds 25 megapixels"
        } else if output.status.code() == Some(4) {
            ": picture exceeds 10 MiB"
        } else {
            ""
        }
    );
    decode_wsl_output(&output.stdout)
}

fn bounded_output(mut command: Command, timeout: Duration) -> io::Result<Output> {
    command.stdout(Stdio::piped()).stderr(Stdio::null()).process_group(0);
    let mut child = command.spawn()?;
    let mut stdout = child.stdout.take().expect("stdout was piped");
    if let Err(error) = set_nonblocking(&stdout) {
        kill_and_reap(&mut child);
        return Err(error);
    }

    let deadline = Instant::now() + timeout;
    let mut output = Vec::new();
    let mut eof = false;
    loop {
        if !eof {
            match drain_stdout(&mut stdout, &mut output, deadline) {
                Ok(reached_eof) => eof = reached_eof,
                Err(error) => {
                    kill_and_reap(&mut child);
                    return Err(error);
                }
            }
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    if !eof {
                        kill_process_group(child.id());
                    }
                    return Ok(Output { status, stdout: output, stderr: Vec::new() });
                }
                if eof {
                    return Ok(Output { status, stdout: output, stderr: Vec::new() });
                }
            }
            Ok(None) => {}
            Err(error) => {
                kill_and_reap(&mut child);
                return Err(error);
            }
        }
        if Instant::now() >= deadline {
            kill_and_reap(&mut child);
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Windows clipboard helper timed out",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn set_nonblocking(stdout: &std::process::ChildStdout) -> io::Result<()> {
    let fd = stdout.as_raw_fd();
    // SAFETY: `fd` is the live stdout descriptor owned by `stdout`.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: same live descriptor; this only adds O_NONBLOCK to its flags.
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Drain only bytes already available on the nonblocking pipe. `true` means
/// EOF; `false` means the child or an inherited descriptor may write more.
fn drain_stdout(
    stdout: &mut std::process::ChildStdout,
    output: &mut Vec<u8>,
    deadline: Instant,
) -> io::Result<bool> {
    let mut buffer = [0; 8192];
    loop {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Windows clipboard helper timed out while reading output",
            ));
        }
        match stdout.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(n) => {
                if output.len().saturating_add(n) > WSL_HELPER_MAX_OUTPUT {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "clipboard helper output exceeds 16 MiB",
                    ));
                }
                output.extend_from_slice(&buffer[..n]);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
}

fn kill_and_reap(child: &mut Child) {
    kill_process_group(child.id());
    let _ = child.kill();
    let _ = child.wait();
}

fn kill_process_group(pid: u32) {
    // The command is placed in its own group before spawn, so this reaches
    // descendants that inherited stdout after the helper itself exited.
    // SAFETY: negative `pid` addresses only the child-owned process group.
    let _ = unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGKILL) };
}

fn decode_wsl_output(output: &[u8]) -> Result<Pasted> {
    let output = std::str::from_utf8(output)?.trim();
    if let Some(encoded) = output.strip_prefix("PNG:") {
        Ok(Pasted::Image(decode_wsl_png(encoded.as_bytes())?))
    } else if let Some(encoded) = output.strip_prefix("TEXT:") {
        Ok(Pasted::Text(decode_wsl_text(encoded.as_bytes())?))
    } else {
        anyhow::bail!("Windows clipboard returned an unknown format")
    }
}

fn decode_wsl_text(encoded: &[u8]) -> Result<String> {
    use base64::Engine as _;
    Ok(String::from_utf8(base64::engine::general_purpose::STANDARD.decode(encoded)?)?)
}

fn decode_wsl_png(encoded: &[u8]) -> Result<Vec<u8>> {
    use base64::Engine as _;
    let encoded = std::str::from_utf8(encoded)?.trim();
    let png = base64::engine::general_purpose::STANDARD.decode(encoded)?;
    anyhow::ensure!(png.len() <= MAX_BYTES, "picture exceeds 10 MiB");
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_PIXELS as u32);
    limits.max_image_height = Some(MAX_PIXELS as u32);
    limits.max_alloc = Some(MAX_PIXELS * 4);
    let decoder = image::codecs::png::PngDecoder::with_limits(Cursor::new(png.as_slice()), limits)
        .context("Windows clipboard did not return a valid PNG image")?;
    use image::ImageDecoder as _;
    let (width, height) = decoder.dimensions();
    anyhow::ensure!(
        width > 0 && height > 0 && u64::from(width) * u64::from(height) <= MAX_PIXELS,
        "picture exceeds 25 megapixels"
    );
    image::DynamicImage::from_decoder(decoder)
        .context("Windows clipboard returned a corrupt PNG image")?;
    Ok(png)
}

fn encode(image: arboard::ImageData<'_>) -> Result<Vec<u8>> {
    let width = u32::try_from(image.width)?;
    let height = u32::try_from(image.height)?;
    anyhow::ensure!(
        width > 0 && height > 0 && u64::from(width) * u64::from(height) <= MAX_PIXELS,
        "picture exceeds 25 megapixels"
    );
    let rgba = image::RgbaImage::from_raw(width, height, image.bytes.into_owned())
        .context("invalid clipboard pixels")?;
    let mut png = Cursor::new(Vec::new());
    rgba.write_to(&mut png, image::ImageFormat::Png)?;
    let png = png.into_inner();
    anyhow::ensure!(png.len() <= MAX_BYTES, "picture exceeds 10 MiB");
    Ok(png)
}

/// Keep only the compact label in the text editor; IDs ride the draft.
/// Other Markdown is untouched, including labels a user chose themselves.
pub fn unpack(text: &str) -> (String, Vec<DraftImage>) {
    let mut body = text.to_string();
    let mut images = Vec::new();
    for link in mesimon_core::links::extract(text) {
        let mesimon_core::links::Found::Attachment(id) = link.target else { continue };
        let Some(label) = link.label else { continue };
        let Some(number) = label.strip_prefix("Image #").and_then(|n| n.parse().ok()) else {
            continue;
        };
        let marker = format!("[{label}]");
        body = body.replace(&format!("{marker}({})", attachment::target(id)), &marker);
        images.push(DraftImage { id, number, png: None });
    }
    (body, images)
}

pub fn marker(image: &DraftImage) -> String {
    format!("[Image #{}]", image.number)
}

pub fn pack(body: &str, images: &[DraftImage]) -> String {
    let mut text = body.to_string();
    for image in images {
        let marker = marker(image);
        let mut expanded = String::new();
        let mut end = 0;
        for (at, _) in text.match_indices(&marker) {
            let after = at + marker.len();
            expanded.push_str(&text[end..after]);
            // A user can paste Markdown too. Its explicit target remains
            // authoritative, even when its label matches a draft picture.
            if !text[after..].starts_with('(') {
                expanded.push_str(&format!("({})", attachment::target(image.id)));
            }
            end = after;
        }
        expanded.push_str(&text[end..]);
        text = expanded;
    }
    text
}

pub fn check_draft(images: &[DraftImage], bytes: usize) -> Result<()> {
    let total: usize = images.iter().filter_map(|i| i.png.as_ref()).map(|p| p.len()).sum();
    anyhow::ensure!(
        bytes <= MAX_BYTES && total + bytes <= DRAFT_MAX_BYTES,
        "pictures exceed the 50 MiB draft limit (10 MiB per picture)"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_selection_uses_the_windows_host_only_for_local_wsl() {
        assert_eq!(select_backend(false, false).unwrap(), Backend::Native);
        assert_eq!(select_backend(false, true).unwrap(), Backend::WslHost);
        assert!(select_backend(true, false).is_err());
        assert!(select_backend(true, true).is_err());
    }

    #[test]
    fn wsl_clipboard_base64_decodes_to_a_valid_bounded_png() {
        use base64::Engine as _;
        let png = encode(arboard::ImageData {
            width: 1,
            height: 1,
            bytes: std::borrow::Cow::Borrowed(&[1, 2, 3, 255]),
        })
        .unwrap();
        let encoded = base64::engine::general_purpose::STANDARD.encode(&png);
        let decoded = decode_wsl_png(format!("{encoded}\r\n").as_bytes()).unwrap();
        assert_eq!(image::load_from_memory(&decoded).unwrap().to_rgba8().as_raw(), &[1, 2, 3, 255]);
        assert_eq!(decode_wsl_text(b"aGVsbG8=").unwrap(), "hello");
        assert!(decode_wsl_text(b"//8=").is_err(), "invalid UTF-8 is refused");
        assert!(matches!(
            decode_wsl_output(format!("PNG:{encoded}").as_bytes()).unwrap(),
            Pasted::Image(_)
        ));
        assert!(
            matches!(decode_wsl_output(b"TEXT:aGVsbG8=").unwrap(), Pasted::Text(text) if text == "hello")
        );
        assert!(decode_wsl_output(b"unknown").is_err());
        assert!(decode_wsl_png(b"not base64").is_err());
        assert!(decode_wsl_png(&vec![b'A'; MAX_BYTES * 2]).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn bounded_output_times_out_and_kills_descendant_holding_stdout() {
        let pid_file = std::env::temp_dir().join(format!("mesimon-clipboard-{}.pid", Ulid::new()));
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 2 & echo $! > \"$1\"; exit 0", "mesimon-test-helper"]);
        command.arg(&pid_file);

        let started = Instant::now();
        let error = bounded_output(command, Duration::from_millis(100)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(1), "deadline included pipe draining");

        let pid: libc::pid_t = std::fs::read_to_string(&pid_file).unwrap().trim().parse().unwrap();
        let _ = std::fs::remove_file(pid_file);
        // The shell has exited, but this descendant inherited stdout. The
        // helper's process group cleanup must kill it as well.
        let reap_deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let result = unsafe { libc::kill(pid, 0) };
            if result == -1 && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
                break;
            }
            assert!(Instant::now() < reap_deadline, "stdout-holding descendant survived cleanup");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(unix)]
    #[test]
    fn bounded_output_keeps_successful_clipboard_decoding() {
        let mut command = Command::new("sh");
        command.args(["-c", "printf 'TEXT:aGVsbG8='"]);
        let output = bounded_output(command, Duration::from_secs(1)).unwrap();
        assert!(output.status.success());
        assert!(
            matches!(decode_wsl_output(&output.stdout).unwrap(), Pasted::Text(text) if text == "hello")
        );
    }

    #[test]
    fn packing_never_overwrites_an_explicit_markdown_target() {
        let image = DraftImage { id: Ulid::new(), number: 1, png: None };
        let text = "[Image #1](https://example.org/picture.png) and [Image #1]";
        assert_eq!(
            pack(text, std::slice::from_ref(&image)),
            format!(
                "[Image #1](https://example.org/picture.png) and [Image #1]({})",
                attachment::target(image.id)
            )
        );
    }

    #[test]
    fn editor_hides_ids_but_save_round_trips_them() {
        let id = Ulid::new();
        let markdown = format!("Before [Image #3]({}) after", attachment::target(id));
        let (plain, images) = unpack(&markdown);
        assert_eq!(plain, "Before [Image #3] after");
        assert_eq!(pack(&plain, &images), markdown);
        assert_eq!(images[0].id, id);
        assert!(images[0].png.is_none());
    }

    #[test]
    fn clipboard_pixels_become_png_and_invalid_dimensions_are_refused() {
        let png = encode(arboard::ImageData {
            width: 1,
            height: 1,
            bytes: std::borrow::Cow::Borrowed(&[1, 2, 3, 255]),
        })
        .unwrap();
        let decoded = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(decoded.as_raw(), &[1, 2, 3, 255]);
        assert!(encode(arboard::ImageData {
            width: 100_000,
            height: 100_000,
            bytes: std::borrow::Cow::Borrowed(&[])
        })
        .is_err());
        assert!(encode(arboard::ImageData {
            width: 1,
            height: 1,
            bytes: std::borrow::Cow::Borrowed(&[])
        })
        .is_err());
        assert!(check_draft(&[], MAX_BYTES + 1).is_err());
    }
}
