# mesimon

**mesimon** (Hebrew משימון, "the task instrument" — pronounced me-si-**MON**) is a terminal kanban
board that runs your coding agents. Like Kubernetes is to containers, mesimon is to Claude Code
and Codex.

![Starting an agent from the board, answering it from the ticket page, and merging its branch](assets/demo.gif)

<sub>Start an agent with Shift+Enter on the board. When its card lights, open the ticket and
answer with Shift+Enter there, then merge its branch with `m`. Recorded with a scripted stand-in agent so the take is reproducible;
[`assets/demo/`](assets/demo) re-records it.</sub>

**Alpha.** It works and it is used every day, and it will change under you. Each version's
changes are in the [CHANGELOG](CHANGELOG.md).

## What it does

- **Start an agent on a ticket with one key.** Your ticket's title and description are its
  first prompt. Claude Code and Codex both work.
- **Each agent can work in its own worktree**, on its own branch, so several agents can change
  the same repo at once without touching each other's files.
- **The card lights when the agent needs you.** The board stays quiet while agents work, and a
  card moves to REVIEW on its own when its agent is done. It turns the one bright colour on the
  screen only when its agent asks you a question or waits on a permission.
- **Review and merge with one key.** `v` shows the branch's diff and `m` merges it,
  fast-forward only.
- **Agents keep running when you close the board.** A background process holds every session,
  and the next time you open the board everything is where you left it. Idle agents can sleep
  and wake into the same conversation.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/amitozalvo/mesimon-releases/main/install.sh | sh
```

Or with Homebrew:

```sh
brew install amitozalvo/tap/mesimon
```

Then run `mesimon doctor`. It checks your setup and prints fixes, and changes nothing.

You need macOS on Apple Silicon or Linux (x86_64 or aarch64, WSL2 included), git, and Claude
Code or Codex, signed in. On Linux you also need tmux 3.3 or newer; on macOS mesimon brings its
own. [Requirements in detail](docs/USING.md#requirements) covers the rest, and
[Updating](docs/USING.md#updating) says how new versions arrive.

## First run

1. `cd` into a git repository and run `mesimon`.
2. Press `o` and type a task as the title, for example *Add a --version flag*.
3. Press `shift+tab` so the line under the title reads `⎇ worktree`. The agent gets its own
   branch.
4. Press `shift+enter`. The ticket is saved, an agent starts on it, and its card spins while it
   works.
5. When the agent is done, its card moves to REVIEW. Press `space` on it to read what the agent
   said, then `m` twice to merge its branch.

If a card lights before then, its agent is asking you something. Press `shift+enter` on the
card to answer it, or `enter` to go into the agent's own terminal; `ctrl-]` brings you back.

`shift+enter` needs a terminal that reports it, such as iTerm2, Ghostty, kitty or WezTerm. If
`?` does not list it, press `enter` in step 4 to save the ticket, then `c` to start its agent,
and type the task into it.

`?` lists every key on the screen you are on. Closing the board does not stop your agents:
[Stopping everything](docs/USING.md#stopping-everything) says how to.

## Tour

**The ticket page.** `tab` on a card writes its description, and `ctrl-k` lists the links in
its notes.

![Writing a ticket's description with tab, opening its page, and opening a link from its agent's note with ctrl-k](assets/demo/ticket-page.gif)

<sub>`tab` adds two lines to the brief, `enter` opens the page with the note its agent left, and
`ctrl-k` opens one of that note's links.</sub>

**Search.** `/` finds any ticket by a few letters of its title, key, column or tag.

![Typing csv into the search picker narrows fifteen tickets to one, and enter puts the cursor on its card](assets/demo/search.gif)

<sub>Three letters narrow fifteen tickets to one, and `enter` scrolls the board to its card.</sub>

**The crown.** `ctrl-o` on a card lets its agent file, move, tag and start the other tickets.

![Crowning a working agent's ticket with ctrl-o: its agent files two tickets, moves one, tags one and starts an agent on another](assets/demo/crown.gif)

<sub>The crowned agent runs the board, and you keep the crown: `ctrl-o` on its card takes it
back. [More on the crown](docs/USING.md#the-crown-one-agent-runs-the-board).</sub>

## Three promises

1. **A strict write allowlist.** mesimon writes to a short list of paths, every one named, and
   nowhere else: never your shell rc, your git config, your agent configuration or your tmux
   config.
2. **No config mutation.** `mesimon doctor` prints fixes for you to apply; it never applies one
   itself.
3. **Zero prompt injection.** mesimon adds, removes and reorders no token of your conversation.
   The board tools it gives its agents, and a brief you can switch on, are shown in full.

[The promises in full](docs/PROMISES.md), with every path mesimon writes. If you find mesimon
breaking one, [report it](SECURITY.md).

## Learn more

- [Using mesimon](docs/USING.md): the board, settings, keyboard layouts, pictures in notes, what
  your agents can see, updating and stopping everything.
- [Remote Control](docs/REMOTE-CONTROL.md): a browser preview for reading and answering your
  agents from a phone or another computer.
- [What is useful to report](TESTING.md), and how to report a [security issue](SECURITY.md).
- [All the docs](docs/README.md), including the design record and the architecture.

## License

Apache-2.0. See [LICENSE](LICENSE), [NOTICE](NOTICE) and [TRADEMARK.md](TRADEMARK.md).
