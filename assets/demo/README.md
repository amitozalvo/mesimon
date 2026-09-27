# The README's clips

`../demo.gif` and the tour clips beside this file are recorded, not drawn. To record one again
after a change it shows:

```sh
cargo build --release -p mesimon
assets/demo/record.sh           # ../demo.gif, about 40 seconds
assets/demo/record.sh search    # search.gif, about 8 seconds
assets/demo/record.sh crown     # crown.gif, about 13 seconds
```

This needs [VHS](https://github.com/charmbracelet/vhs) (`brew install vhs`), python3 and tmux.
The argument names a tape in this directory; the tape's `Output` line says where its GIF goes.

What each file does:

- **`record.sh`** builds a throwaway sandbox under `/tmp`: its own `HOME`, and a small git repo
  copied from `project/`. It starts the release binary's daemon there, seeds the tape's board,
  runs the tape, then stops the daemon and removes everything, including the daemon's private
  tmux. It also sets one timing seam: the worktree flags refresh every 2 s instead of every
  10 s, so the merge offer appears soon after the agent finishes.
- **`seed.py`** creates the tickets over the daemon's socket, using the same wire protocol as
  the TUI. `BOARDS` holds one ticket list per tape. It also starts the agent that is already at
  work when the recording opens, on the ticket `AT_WORK` names for the tape.
- **`stub-claude.py`** stands in for `claude` through `MESIMON_CLAUDE_BIN`. The daemon launches
  it with claude's own argv and pastes the prompt into its pane. It reports back the way claude
  does: through the hook commands in its settings file, and through the `mesimon mcp` shim for
  the board tools (`raise_hand` in the demo; `create_ticket`, `move_ticket`, `tag_ticket` and
  `start_agent` once crowned in the crown clip). It writes a transcript, and the ticket page's
  preview reads it. Only the model is scripted, and what it does is keyed off its ticket's
  title. A script that fails leaves its traceback in the agent's pane.
- **`kitty-term.py`** puts a pty between VHS and the board. VHS's terminal (xterm.js) does not
  answer the kitty keyboard query, so on that terminal the board correctly turns Shift+Enter
  off. The shim answers the query and delivers the tape's `Alt+Enter` as the kitty report for
  Shift+Enter. Every other byte passes through unchanged.
- **`demo.tape`** is the README's main take: start an agent, answer it, merge its branch.
  Wherever the agent's timing matters, the tape waits for the screen to change instead of
  sleeping for a fixed time.
- **`search.tape`** is the search tour clip: fifteen tickets, `/`, `csv`, `enter`. Its board
  runs TODO off the bottom of the screen, so the card it finds starts out of view.
- **`crown.tape`** is the crown tour clip: `^o` crowns the agent at work, and the stub, once
  crowned, files two tickets, moves one, tags one and starts an agent on another.
