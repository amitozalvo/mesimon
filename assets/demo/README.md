# The README's clips

`../demo.gif` and the tour clips beside this file are recorded, not drawn. To record one again
after a change it shows:

```sh
cargo build --release -p mesimon
assets/demo/record.sh              # ../demo.gif, about 40 seconds
MESIMON_DEMO_KEY_FILE=~/.config/mesimon-demo.key \
  assets/demo/record.sh agent      # agent.gif, the real claude, about 12 seconds
assets/demo/record.sh search       # search.gif, about 8 seconds
assets/demo/record.sh crown        # crown.gif, about 13 seconds
assets/demo/record.sh ticket-page  # ticket-page.gif, about 11 seconds
assets/demo/record.sh reddit       # reddit.mp4 and reddit.gif, about 36 seconds, not in the repo
```

`MESIMON=target/debug/mesimon` in front of a line records with the debug build instead.

This needs [VHS](https://github.com/charmbracelet/vhs) (`brew install vhs`), python3 and tmux.
The argument names a tape in this directory; the tape's `Output` line says where its GIF goes.

The two Remote Control pictures, `remote.png` (the Now screen with a permission on its card)
and `remote-conversation.png` (a ticket page with the agent's conversation), are not tapes:
they are the real Remote Control page, driven on its own UX-test fixture with a seeded board,
no relay and no key. From `web/mesophon`, after `npm ci`:

```sh
npm run shot:remote                # writes both into this directory, 390 px at twice the density
```

`web/mesophon/remote-shot.mjs` holds the board it seeds; the README, mesimon.dev and its
Remote Control page (`ci/site.sh` copies them) all show these two files.

The README's logo, `../logo-dark.svg` and `../logo-light.svg`, is not recorded either: it is the
last frame of the intro (`crates/mesimon-tui/src/intro.rs`), in graphite for a dark page and
chalk for a light one. A test fails when the intro and the logo differ, and draws them again:

```sh
MESIMON_UPDATE_GOLDEN=1 cargo test -p mesimon-tui the_readme_logo
```

What each file does:

- **`record.sh`** builds a throwaway sandbox under `/tmp`: its own `HOME`, and a small git repo
  copied from `project/`. It starts the release binary's daemon there, seeds the tape's board,
  runs the tape, then stops the daemon and removes everything, including the daemon's private
  tmux. Its prefs turn the merge train off, so a finished branch waits for the take's `m`. It
  also sets one timing seam: the worktree flags refresh every 2 s instead of every
  10 s, so the merge offer appears soon after the agent finishes. The link opener is `true`,
  so a link a take opens reads `opening …` on the board and starts no browser. The `reddit`
  take alone keeps the merge train, whose merge is its ending, and runs with the `P` peek on,
  so every card shows its agent's latest words. A take that fails prints every agent pane
  before the sandbox goes: the stand-in's traceback is there.
- **`seed.py`** creates the tickets over the daemon's socket, using the same wire protocol as
  the TUI. It answers Setup first, as one person on one repository would, and files each
  ticket in TODO, then moves it where the tape wants it. `BOARDS` holds one ticket list per
  tape. It also starts the agent that is already at
  work when the recording opens, on the ticket `AT_WORK` names for the tape, and marks the
  first step into an agent as practised, so the practice screen does not open in a take. For
  `ticket-page` it then runs a second agent that writes a note on its ticket and stops, and puts
  that ticket back at the top of TODO. With the real claude it also answers claude's first-run
  screens, gives that agent a task long enough to outlast the take, and waits until claude has
  taken it (see below).
- **`stub-claude.py`** stands in for `claude` through `MESIMON_CLAUDE_BIN`. The daemon launches
  it with claude's own argv and pastes the prompt into its pane. It reports back the way claude
  does: through the hook commands in its settings file, and through the `mesimon mcp` shim for
  the board tools (`raise_hand` in the demo; `create_ticket`, `move_ticket`, `tag_ticket` and
  `start_agent` once crowned in the crown clip; `write_note` in the ticket-page clip). It writes
  a transcript, and the ticket page's preview reads it. Its pane has a prompt line that takes
  typed text, drawn the way the daemon reads Claude Code's composer (a rule, `❯ `, a rule),
  since a launch's words go only into a composer the daemon has seen; a line over it says it
  is a stand-in. Only the model is scripted, and what it
  does is keyed off its ticket's title. A script that fails leaves its traceback in the agent's
  pane.
- **`kitty-term.py`** puts a pty between VHS and the board. VHS's terminal (xterm.js) does not
  answer the kitty keyboard query, so on that terminal the board correctly turns Shift+Enter
  off. The shim answers the query and delivers the tape's `Alt+Enter` as the kitty report for
  Shift+Enter. Every other byte passes through unchanged.
- **`demo.tape`** is the README's main take: start an agent, answer it, merge its branch and
  tell the agent it merged.
  Wherever the agent's timing matters, the tape waits for the screen to change instead of
  sleeping for a fixed time.
- **`agent.tape`** is the tour's first clip: from a working agent's ticket page, step into the
  agent's own terminal, type to it, and step back to the board. It waits on the shape of the
  agent's screen, never its words, and every wait also matches the stand-in, so the tape runs
  with either. A busy claude holds a typed message until its current step ends, which can take
  most of a minute, so the take is cut from the queued message to the answer under it.
- **`search.tape`** is the search tour clip: fifteen tickets, `/`, `csv`, `enter`. Its board
  runs TODO off the bottom of the screen, so the card it finds starts out of view.
- **`crown.tape`** is the crown tour clip: `^o` crowns the agent at work, and the stub, once
  crowned, files two tickets, moves one, tags one and starts an agent on another.
- **`ticket-page.tape`** is the ticket-page tour clip: `tab` writes a card's description,
  `enter` opens its page, `j j` walks to the note its agent left, `^k` lists that note's links
  and `enter` opens one.
- **`reddit.tape`** is the clip for a Reddit post (T-1125), one take from the crown to the
  merge, all of it on the board: `^o` crowns the agent at work, it files, moves, tags and
  starts an agent on the demo ticket, that agent raises its hand, `k` puts the cursor on its
  card, Shift+Enter answers it there, and the merge train lands its branch. Larger cells than
  the README's clips, longer holds, an mp4 for Reddit's native video upload beside the gif.
  Its outputs are ignored by git.

## The agent clip is the real claude

`agent.gif` shows real Claude Code. The other four clips keep the stand-in, because they depend
on its timing: the demo's raised hand, the crown's tool calls, the note on the ticket page.
`record.sh` refuses a key file for any tape but `agent`.

`MESIMON_DEMO_KEY_FILE` names a file holding one secret and nothing else: a Console API key
(`sk-ant-api…`, sent as `ANTHROPIC_API_KEY`, billing that key's credits) or a token from
`claude setup-token` (`sk-ant-oat…`, sent as `CLAUDE_CODE_OAUTH_TOKEN`, using that
subscription). The API refuses a token sent as a key. The take runs on Sonnet unless `MESIMON_DEMO_MODEL` names another model. The model needs
claude's auto mode: without it (Haiku) claude asks before its first edit and the take stalls.

- **The key rides no command line.** A pane gets the environment the user's login shell exports:
  the daemon runs `$SHELL -l` from a clean environment and reads back what it exports. So
  `record.sh` writes the sandbox a `.profile` (mode 0600) that reads the key with the shell's own
  `read` builtin. The key then sits in the daemon's environment file (0600, under its runtime
  dir) and in claude's environment. Both files go with the sandbox.
- **Nothing of this machine's sign-in is borrowed.** The profile sets `CLAUDE_CONFIG_DIR` to the
  sandbox's `~/.claude`. That gives claude the sandbox's config, and a keychain entry name that no
  sign-in on the machine has made. `record.sh` exports the same variable, so a pane that starts
  before the shell's answer lands does not find your login either. `CLAUDE_CODE_TMPDIR` keeps
  claude's scratch files in the sandbox too.
- **claude's first-run screens are answered before it starts.** `seed.py` writes the sandbox's
  `.claude.json`: onboarding done, the dark theme, the key approved (claude keeps the key's last
  20 characters there), the auto-mode and fast-mode notices seen, and the agent's worktree
  trusted. It writes them once the daemon has finished probing claude for its road, since every
  claude that probe runs writes the same file. A screen left over would stall the take as a SETUP card, so `seed.py` waits for
  claude to take its prompt and stops the take with the pane's text if it does not.
- **Without a key file** the tape runs on the stand-in, free and the same every time, which is
  the way to rehearse it. It writes `agent.gif` as well; do not keep that one.
