//! The private server's tmux.conf — every line justified by docs/19 or a spike.

/// The status line's right-hand hint: the two detach keys and the word. Spelled
/// the way the board's own footer spells a control key (`keymap`: `Key::Ctrl(c)`
/// renders `^c`, so `^k`, `^t`, `^]`) — one vocabulary on both sides of the
/// handover; it read `Ctrl+]/^5` for a while, two spellings in ten cells
/// (T-261). Rendered into the conf for fresh servers and pushed live by
/// `TmuxBackend::set_status_left` for the ones already running, from this one
/// place, so the two cannot drift apart again.
pub const STATUS_RIGHT: &str = " ^]/^5 back  ";

/// One line per wheel event: tmux's default five-line step amplifies trackpad
/// gestures. Shared by fresh configs and live-server upgrades. Keep pane
/// selection and leave root-table mouse forwarding to applications intact.
pub const SCROLL_BINDINGS: [(&str, &str, &str); 4] = [
    ("copy-mode", "WheelUpPane", "select-pane; send-keys -N 1 -X scroll-up"),
    ("copy-mode", "WheelDownPane", "select-pane; send-keys -N 1 -X scroll-down"),
    ("copy-mode-vi", "WheelUpPane", "select-pane; send-keys -N 1 -X scroll-up"),
    ("copy-mode-vi", "WheelDownPane", "select-pane; send-keys -N 1 -X scroll-down"),
];

/// The `status-position` word for a preference: tmux's own two values.
pub fn status_position(top: bool) -> &'static str {
    if top {
        "top"
    } else {
        "bottom"
    }
}

/// Render the conf. `pane_died_cmd` is the daemon's notify command (spike T-7:
/// the `pane-died` hook is the ONLY timely death signal — control mode is
/// silent on pane death); `None` renders the hook-less M1 conf (unit tests).
/// `status_top` is the Settings preference (T-264): where the status line
/// sits over an agent's pane. Rendered for FRESH servers and pushed live by
/// `TmuxBackend::set_status_position` for running ones.
pub fn render(pane_died_cmd: Option<&str>, status_top: bool) -> String {
    // Spike references: T-2 (update-environment), T-6 (extended-keys, focus-events),
    // T-7 (remain-on-exit + pane-died), T-10 (clipboard/passthrough containment).
    // C-5 needs its own bind: `extended-keys always` negotiates CSI-u with the outer
    // terminal, so Ctrl+5 arrives as a distinct key (CSI 53;5u), not the legacy 0x1D
    // alias docs/04 §2.14 rung 1b assumed. It is the layout-independent unfocus key
    // (digits don't move on non-US layouts; Ctrl+physical-] sends Esc on Hebrew).
    let mut conf =
        r##"# mesimon private tmux server — generated, do not edit (docs/19-tmux-backend-v01.md)
set -g prefix None
unbind-key -a
bind-key -n C-] detach-client
bind-key -n C-5 detach-client
set -g remain-on-exit on
set -g update-environment ""
set -g set-clipboard off
set -g allow-passthrough off
set -g extended-keys always
set -g focus-events on
set -g mouse on
set -g history-limit 50000
set -g window-size latest
set -g default-terminal "tmux-256color"
set -g escape-time 10
set -g status-style "reverse"
set -g status-left " mesimon "
set -g status-left-length 120
set -g status-right "@STATUS_RIGHT@"
set -g status-right-length 20
set -g status-position @STATUS_POSITION@
set -g window-status-format ""
set -g window-status-current-format ""
"##
        .replace("@STATUS_RIGHT@", STATUS_RIGHT)
        .replace("@STATUS_POSITION@", status_position(status_top));
    for (table, key, command) in SCROLL_BINDINGS {
        conf.push_str(&format!("bind-key -T {table} {key} {{ {command} }}\n"));
    }
    for (table, key, pipe) in copy_pipe_bindings() {
        conf.push_str(&format!(
            "bind-key -T {table} {key} send-keys -X copy-pipe-and-cancel \"{pipe}\"\n"
        ));
    }
    if let Some(cmd) = pane_died_cmd {
        // Brace literal keeps the nested quoting sane (tmux ≥ 3.1).
        conf.push_str(&format!("set-hook -g pane-died {{ {cmd} }}\n"));
    }
    conf
}

/// The command tmux pipes a finished copy-mode selection into, reaching the
/// system clipboard WITHOUT `set-clipboard on` — T-10's OSC-52 containment
/// stays: inner apps still can't touch the clipboard, only tmux's own copy
/// does, via a local pipe. None on platforms without a known clipboard tool
/// (defaults keep the selection in tmux's buffer, as before).
///
/// macOS is `pbcopy`. Linux is decided at runtime by [`linux_clipboard`],
/// because which clipboard a Linux box has is not a compile-time fact: under
/// WSL it is Windows' own, on a desktop it is Wayland's or X's, on a server
/// there is none.
fn copy_pipe_cmd() -> Option<String> {
    if cfg!(target_os = "macos") {
        return Some("pbcopy".into());
    }
    if cfg!(target_os = "linux") {
        let kernel = std::fs::read_to_string("/proc/version").unwrap_or_default();
        return linux_clipboard(
            &kernel,
            std::env::var_os("WAYLAND_DISPLAY").is_some(),
            std::env::var_os("DISPLAY").is_some(),
            |name| {
                crate::which_on_path(name).or_else(|| {
                    // WSL puts the Windows System32 on PATH by default, but a
                    // user who turned that off still has the drive mounted.
                    let fixed = std::path::PathBuf::from("/mnt/c/Windows/System32").join(name);
                    (name == "clip.exe" && fixed.is_file()).then_some(fixed)
                })
            },
        );
    }
    None
}

/// Which clipboard a Linux copy reaches, from three facts so the choice is a
/// pure function: a kernel string naming Microsoft is WSL, whose clipboard is
/// Windows' own (`clip.exe`, reachable through interop); a Wayland session is
/// `wl-copy`; an X session is `xclip`. Paths come back ABSOLUTE, because the
/// pipe runs under the tmux server's frozen environment (D29) and a bare name
/// would be looked up on whatever PATH the server was born with.
pub(crate) fn linux_clipboard(
    kernel: &str,
    wayland: bool,
    x11: bool,
    which: impl Fn(&str) -> Option<std::path::PathBuf>,
) -> Option<String> {
    if kernel.to_ascii_lowercase().contains("microsoft") {
        return which("clip.exe").map(|p| p.display().to_string());
    }
    if wayland {
        if let Some(p) = which("wl-copy") {
            return Some(p.display().to_string());
        }
    }
    if x11 {
        if let Some(p) = which("xclip") {
            return Some(format!("{} -in -selection clipboard", p.display()));
        }
    }
    None
}

/// Copy-mode bindings `(table, key, pipe_cmd)` that end a selection through
/// `copy_pipe_cmd`. Rendered into the conf for fresh servers AND issued as
/// live commands (`TmuxBackend::install_copy_bindings`) — a running server
/// never re-reads `-f`. Mouse drag-end plus the two keyboard copy keys;
/// `copy-pipe-and-cancel` matches the default bindings' cancel behavior.
pub fn copy_pipe_bindings() -> Vec<(&'static str, &'static str, String)> {
    let Some(pipe) = copy_pipe_cmd() else {
        return Vec::new();
    };
    vec![
        ("copy-mode", "MouseDragEnd1Pane", pipe.clone()),
        ("copy-mode-vi", "MouseDragEnd1Pane", pipe.clone()),
        ("copy-mode", "Enter", pipe.clone()),
        ("copy-mode-vi", "y", pipe),
    ]
}

/// The `run-shell` command the `pane-died` hook executes: one invocation of
/// the mesimon binary's hook path, session identified by sid16, exit status
/// carried in `--reason`, the dead pane's key in `--pane` (a wake reuses the
/// session name for its new pane; the key is what tells them apart, T-245).
/// `-b` keeps the notify off tmux's own thread.
pub fn pane_died_cmd(hook_bin: &str, hook_sock: &str) -> String {
    format!(
        r##"run-shell -b '"{hook_bin}" hook --sock "{hook_sock}" --session "#{{session_name}}" --event PaneDied --reason "#{{pane_dead_status}}" --pane "{PANE_KEY}"'"##
    )
}

/// A pane's identity across the server's life: `<server pid>:<pane id>`. The
/// pane id alone is unique per SERVER only — a fresh server hands out `%0`
/// again, and a wake that kills the last session restarts the server.
/// `TmuxBackend::spawn` returns it and the hook binary rebuilds the same
/// string from `TMUX` and `TMUX_PANE`, so the three must agree.
pub const PANE_KEY: &str = "#{pid}:#{pane_id}";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hookless_render_has_no_hook() {
        assert!(!render(None, false).contains("pane-died"));
    }

    /// The status line's side is the preference's (T-264): bottom, tmux's
    /// own default, unless Settings said top — and the conf always says
    /// which, so a fresh server never inherits a stale word.
    #[test]
    fn the_status_line_sits_where_the_preference_says() {
        assert!(render(None, false).contains("set -g status-position bottom\n"));
        assert!(render(None, true).contains("set -g status-position top\n"));
        assert_eq!(status_position(true), "top");
    }

    /// The detach hint spells its keys the way the board's footer does — a
    /// caret, never `Ctrl+` — and says the same thing on a fresh server's conf
    /// as on a live one (T-261).
    #[test]
    fn the_detach_hint_speaks_the_footers_language() {
        assert!(!STATUS_RIGHT.contains("Ctrl"), "{STATUS_RIGHT:?}");
        assert!(STATUS_RIGHT.contains("^]") && STATUS_RIGHT.contains("^5"));
        assert!(render(None, false).contains(&format!("set -g status-right \"{STATUS_RIGHT}\"")));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn render_pipes_copy_to_the_clipboard_without_osc52() {
        let conf = render(None, false);
        assert!(conf.contains(r#"copy-pipe-and-cancel "pbcopy""#));
        assert!(conf.contains("bind-key -T copy-mode-vi MouseDragEnd1Pane"));
        // T-10 containment must survive the clipboard fix.
        assert!(conf.contains("set-clipboard off"));
    }

    /// The Linux choice is a pure function of three facts, so every branch is
    /// pinned here without a Wayland session or a Windows drive in the room.
    #[test]
    fn a_linux_copy_finds_the_clipboard_it_has() {
        use std::path::PathBuf;
        let have = |names: &'static [&'static str]| {
            move |n: &str| names.contains(&n).then(|| PathBuf::from("/usr/bin").join(n))
        };
        let wsl = "Linux version 5.15.167.4-microsoft-standard-WSL2 (root@...)";
        let plain = "Linux version 6.8.0-45-generic (buildd@lcy02) ...";

        assert_eq!(
            linux_clipboard(wsl, false, false, have(&["clip.exe", "xclip"])).as_deref(),
            Some("/usr/bin/clip.exe"),
            "WSL is Windows' clipboard, whatever else is installed"
        );
        assert_eq!(linux_clipboard(wsl, false, false, have(&[])), None, "no interop, no pipe");
        assert_eq!(
            linux_clipboard(plain, true, true, have(&["wl-copy", "xclip"])).as_deref(),
            Some("/usr/bin/wl-copy"),
            "Wayland outranks X when both are up"
        );
        assert_eq!(
            linux_clipboard(plain, false, true, have(&["xclip"])).as_deref(),
            Some("/usr/bin/xclip -in -selection clipboard")
        );
        assert_eq!(
            linux_clipboard(plain, false, false, have(&["xclip"])),
            None,
            "no display: a server"
        );
        assert_eq!(
            linux_clipboard(plain, true, false, have(&["xclip"])),
            None,
            "wayland without wl-copy"
        );
    }

    #[test]
    fn hooked_render_carries_the_notify() {
        let cmd = pane_died_cmd("/abs/mesimon", "/tmp/m/hook.sock");
        let conf = render(Some(&cmd), false);
        assert!(conf.contains("set-hook -g pane-died"));
        assert!(conf.contains(r#""/abs/mesimon" hook"#));
        assert!(conf.contains("#{session_name}"));
        assert!(conf.contains("#{pane_dead_status}"));
        assert!(conf.contains(r##"--pane "#{pid}:#{pane_id}""##));
    }
}
