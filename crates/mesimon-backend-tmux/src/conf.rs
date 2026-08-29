//! The private server's tmux.conf — every line justified by docs/19 or a spike.

/// Render the conf. `pane_died_cmd` is the daemon's notify command (spike T-7:
/// the `pane-died` hook is the ONLY timely death signal — control mode is
/// silent on pane death); `None` renders the hook-less M1 conf (unit tests).
pub fn render(pane_died_cmd: Option<&str>) -> String {
    // Spike references: T-2 (update-environment), T-6 (extended-keys, focus-events),
    // T-7 (remain-on-exit + pane-died), T-10 (clipboard/passthrough containment).
    let mut conf = r##"# mesimon private tmux server — generated, do not edit (docs/19-tmux-backend-v01.md)
set -g prefix None
unbind-key -a
bind-key -n C-] detach-client
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
set -g status-right " Ctrl+] back  "
set -g status-right-length 20
set -g window-status-format ""
set -g window-status-current-format ""
"##
    .to_string();
    if let Some(cmd) = pane_died_cmd {
        // Brace literal keeps the nested quoting sane (tmux ≥ 3.1).
        conf.push_str(&format!("set-hook -g pane-died {{ {cmd} }}\n"));
    }
    conf
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
    fn hooked_render_carries_the_notify() {
        let cmd = pane_died_cmd("/abs/mesimon", "/tmp/m/hook.sock");
        let conf = render(Some(&cmd));
        assert!(conf.contains("set-hook -g pane-died"));
        assert!(conf.contains(r#""/abs/mesimon" hook"#));
        assert!(conf.contains("#{session_name}"));
        assert!(conf.contains("#{pane_dead_status}"));
    }
}
