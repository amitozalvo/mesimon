# crates/mesimon-tui

Loaded when a session works under this directory; the root `CLAUDE.md` still applies.

**A paste is one event, and only a text field takes it.** `App::tick` routes `Event::Paste` to
`App::on_paste`; `EditBuffer::paste` flattens to one line (newlines become spaces) under the
field's byte `limit` — the same number the daemon caps at. A new text field passes its limit to
`EditBuffer::new` and joins `App::text_field`, or the layout pause (T-458) reads its Hebrew as
a stray key. `TextArea::paste` is the multi-line counterpart: newlines kept, CRLF
normalised, same limit.

## Traps that were measured

- **tmux keeps one XTVERSION reply per client and types the next one.** Its own attach-time
  query takes iTerm2's `ESC P >|iTerm2 …`; a pane that asks again through a passthrough gets
  the reply back as M-P plus text (T-488). `extended-keys` stays `on`, never `always`: snacks.nvim
  routes its probe around tmux only under `on`, and the root `S-Enter` bind carries Shift+Enter
  to the panes that never asked.
- **iTerm2 does not repaint a cell whose only change is its underline colour** (3.6, T-696): a
  whole line rewritten shows the new colour, one cell rewritten keeps the old. `quiet.rs` writes
  such a cell twice, bare then underlined, inside the frame's synchronized update.
- **A kitty key-release report outlives the keypress, and tmux types it.** Under the pushed
  flags a release is `CSI code;mods:3 u`; tmux 3.6a's CSI-u parser stops at the `:` and passes
  it to the pane as text. `restore_terminal` pops the flags and then fences on a
  cursor-position reply before the attach (`settle_key_reports`), so nothing reported under the
  flags reaches the tmux client.
