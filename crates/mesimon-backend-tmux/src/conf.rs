//! The private server's tmux.conf — every line justified by docs/19 or a spike.

/// Render the conf. `status_text` is the FOCUSED mode line (docs/19 §2.3).
pub fn render() -> String {
    // Spike references: T-2 (update-environment), T-6 (extended-keys, focus-events),
    // T-7 (remain-on-exit + pane-died), T-10 (clipboard/passthrough containment).
    r##"# mesimon private tmux server — generated, do not edit (docs/19-tmux-backend-v01.md)
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
set -g status-left "  FOCUSED "
set -g status-left-length 20
set -g status-right " Ctrl+] board  "
set -g status-right-length 20
set -g window-status-format ""
set -g window-status-current-format "#{session_name}"
"##
    .to_string()
}
