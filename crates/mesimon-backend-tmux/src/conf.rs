//! The private server's tmux.conf — every line justified by docs/19 or a spike.

/// Render the conf. `pane_died_cmd` is the daemon's notify command (spike T-7:
/// the `pane-died` hook is the ONLY timely death signal — control mode is
/// silent on pane death); `None` renders the hook-less M1 conf (unit tests).
pub fn render(pane_died_cmd: Option<&str>) -> String {
    // Spike references: T-2 (update-environment), T-6 (extended-keys, focus-events),
    // T-7 (remain-on-exit + pane-died), T-10 (clipboard/passthrough containment).
    // C-5 needs its own bind: `extended-keys always` negotiates CSI-u with the outer
    // terminal, so Ctrl+5 arrives as a distinct key (CSI 53;5u), not the legacy 0x1D
    // alias docs/04 §2.14 rung 1b assumed. It is the layout-independent unfocus key
    // (digits don't move on non-US layouts; Ctrl+physical-] sends Esc on Hebrew).
    let mut conf = r##"# mesimon private tmux server — generated, do not edit (docs/19-tmux-backend-v01.md)
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
set -g status-right " Ctrl+]/^5 back  "
set -g status-right-length 20
set -g window-status-format ""
set -g window-status-current-format ""
"##
    .to_string();
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
fn copy_pipe_cmd() -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        Some("pbcopy")
    } else {
        None
    }
}

/// Copy-mode bindings `(table, key, pipe_cmd)` that end a selection through
/// `copy_pipe_cmd`. Rendered into the conf for fresh servers AND issued as
/// live commands (`TmuxBackend::install_copy_bindings`) — a running server
/// never re-reads `-f`. Mouse drag-end plus the two keyboard copy keys;
/// `copy-pipe-and-cancel` matches the default bindings' cancel behavior.
pub fn copy_pipe_bindings() -> Vec<(&'static str, &'static str, &'static str)> {
    let Some(pipe) = copy_pipe_cmd() else {
        return Vec::new();
    };
    vec![
        ("copy-mode", "MouseDragEnd1Pane", pipe),
        ("copy-mode-vi", "MouseDragEnd1Pane", pipe),
        ("copy-mode", "Enter", pipe),
        ("copy-mode-vi", "y", pipe),
    ]
}

/// The `run-shell` command the `pane-died` hook executes: one invocation of
/// the mesimon binary's hook path, session identified by sid16, exit status
/// carried in `--reason`. `-b` keeps the notify off tmux's own thread.
pub fn pane_died_cmd(hook_bin: &str, hook_sock: &str) -> String {
    format!(
        r##"run-shell -b '"{hook_bin}" hook --sock "{hook_sock}" --session "#{{session_name}}" --event PaneDied --reason "#{{pane_dead_status}}"'"##
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hookless_render_has_no_hook() {
        assert!(!render(None).contains("pane-died"));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn render_pipes_copy_to_the_clipboard_without_osc52() {
        let conf = render(None);
        assert!(conf.contains(r#"copy-pipe-and-cancel "pbcopy""#));
        assert!(conf.contains("bind-key -T copy-mode-vi MouseDragEnd1Pane"));
        // T-10 containment must survive the clipboard fix.
        assert!(conf.contains("set-clipboard off"));
    }

    #[test]
    fn hooked_render_carries_the_notify() {
        let cmd = pane_died_cmd("/abs/mesimon", "/tmp/m/hook.sock");
        let conf = render(Some(&cmd));
        assert!(conf.contains("set-hook -g pane-died"));
        assert!(conf.contains(r#""/abs/mesimon" hook"#));
        assert!(conf.contains("#{session_name}"));
        assert!(conf.contains("#{pane_dead_status}"));
    }
}
