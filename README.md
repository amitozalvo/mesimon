# mesimon

**mesimon** (Hebrew משימון, "the task instrument" — pronounced me-si-**MON**) is a terminal kanban
board that orchestrates many coding-agent sessions: like Kubernetes is to containers, mesimon is to
Claude Code (and other agent CLIs). Tickets outlive sessions; columns carry policy; a per-repo
daemon keeps everything alive when the TUI closes.

**Status: v0.1.0-alpha.1 — early, and shared with a small group for feedback.** It runs, it is
dogfooded daily, and it will change under you. See [TESTING.md](TESTING.md) for what is useful to
report.

## Three promises

1. **A strict write allowlist.** mesimon writes only to `.mesimon/`, `$GIT_DIR/info/exclude`, git
   worktrees and branches it created, and its own state dir under `~/.local/state/mesimon/`.
   Never your shell rc, your git config, your `~/.claude/`, or your tmux config.
2. **No config mutation.** `mesimon doctor` diagnoses and prints copy-pasteable fixes. It has no
   `--fix`.
3. **Zero token injection.** mesimon adds, removes, and reorders exactly zero tokens of what any
   model receives, by default. Anything that would change model input is per-column, opt-in, and
   authored by you.

## Requirements

- **macOS on Apple Silicon.** The published build is `aarch64-apple-darwin`. The code is
  Unix-only by design (unix sockets, tmux); Linux is buildable but untested and unshipped.
- **tmux 3.1+** — every agent runs in a pane on a *private* tmux server, never your own.
- **git**.
- **[Claude Code](https://claude.com/claude-code)** on your `PATH`, to spawn Claude sessions.
- A GitHub account with access to this repo, and `gh` (the repo is private).

## Install

```sh
brew install tmux gh          # if you do not have them
gh auth login                 # once
sh install.sh                 # installs to ~/.local/bin/mesimon
mesimon doctor                # confirm the environment
```

`install.sh` verifies the checksum, runs the binary once before installing it, and tells you
exactly what to fix if anything is missing.

Prefer building it yourself? `cargo install --git https://github.com/amitozalvo/mesimon --locked
mesimon` (Rust 1.85+).

## Updating

**Re-run `install.sh`.** That is the whole procedure.

- If a board is open, it notices the new binary and offers `update ready (U reloads)`. `U`
  restarts it in place.
- If no board is open, the next one you start notices that the background daemon is running older
  code and restarts it for you, before drawing anything. Your sessions survive: they live on the
  tmux server, which the daemon does not own.

## The first five minutes

```sh
cd <a git repo>
mesimon
```

On the board: `a` adds a ticket, `enter` opens it, `space` opens its page, `m` then `<`/`>` moves
it between columns, `p` peeks at a live pane, `q` quits.

On a ticket page: `c` starts a Claude session, `s` a shell, `enter` focuses a live one — that hands
your whole terminal over; detach with tmux's `ctrl-b d` and you are back on the board. `w` switches
the ticket between a shared checkout and its own worktree (locked once a session exists), `v` shows
the diff once there is a worktree, and the merge flow lives on the identity line: fast-forward only,
so mesimon never mints a merge commit.

`tab` jumps to whatever needs you. The board is deliberately quiet — exactly one saturated colour
exists, and it means *this session is waiting on you*.

`Z` parks idle sessions, `A` archives a finished ticket, `V` lists the archive. **Closing the board
does not stop your agents** — that is the point of the daemon.

The footer always names the keys for whatever you are looking at.

## Stopping everything

Agent sessions cost real tokens and keep running after the board closes. To stop all of them for a
repo:

```sh
mesimon doctor                     # shows the daemon and the project key
pkill -f "mesimon daemon"          # stops the daemon (sessions survive this)
tmux -S /tmp/mesimon-$(id -u)/<project key>/tmux.sock kill-server   # stops the agents
```

Inside the board, `Z` parks every idle session, which is the gentler version.

## What mesimon writes

| Path | What |
|---|---|
| `<repo>/.mesimon/` | Your board: columns and tickets. Excluded via `$GIT_DIR/info/exclude`, never `.gitignore`. |
| `$GIT_DIR/info/exclude` | One line, so `.mesimon/` does not show up in `git status`. |
| `~/.local/state/mesimon/<project key>/` | Sessions, worktrees, hook settings, logs, the private tmux socket and conf. |
| Worktrees and `msmn/*` branches | Only ones it created, only for tickets you set to worktree mode. |

Nothing else. If you ever find mesimon writing outside that list, that is a bug worth reporting
above all others.

## Layout

- `docs/` — a pre-code research corpus, kept for its measurements and reasoning. It is not the
  spec; the code is. `docs/STALE-MAP.md` is the design record: what was built, and why.
- `crates/` — the Rust workspace.
- `team/` — reserved for the future source-available team tier (see `docs/00-DECISIONS.md` D3a).

## License

Apache-2.0. See `LICENSE`, `NOTICE`, and `TRADEMARK.md`.
