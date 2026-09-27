# The README demo

`../demo.gif` is recorded, not drawn. To record it again after a change it shows:

```sh
cargo build --release -p mesimon
assets/demo/record.sh
```

This needs [VHS](https://github.com/charmbracelet/vhs) (`brew install vhs`), python3 and tmux.
One take lasts about 40 seconds and writes `assets/demo.gif`.

What each file does:

- **`record.sh`** builds a throwaway sandbox under `/tmp`: its own `HOME`, and a small git repo
  copied from `project/`. It starts the release binary's daemon there, seeds the board, runs
  the tape, then stops the daemon and removes everything, including the daemon's private tmux.
- **`seed.py`** creates the tickets over the daemon's socket, using the same wire protocol as
  the TUI. It also starts the agent that is already at work when the recording opens.
- **`stub-claude.py`** stands in for `claude` through `MESIMON_CLAUDE_BIN`. The daemon launches
  it with claude's own argv and pastes the prompt into its pane. It reports back the way claude
  does: through the hook commands in its settings file, and through the `mesimon mcp` shim for
  `raise_hand`. It writes a transcript, and the ticket page's preview reads it. Only the model is
  scripted.
- **`kitty-term.py`** puts a pty between VHS and the board. VHS's terminal (xterm.js) does not
  answer the kitty keyboard query, so on that terminal the board correctly turns Shift+Enter
  off. The shim answers the query and delivers the tape's `Alt+Enter` as the kitty report for
  Shift+Enter. Every other byte passes through unchanged.
- **`demo.tape`** is the take itself. Wherever the agent's timing matters, the tape waits for
  the screen to change instead of sleeping for a fixed time.
