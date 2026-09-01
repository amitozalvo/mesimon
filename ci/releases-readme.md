# mesimon — releases

Prebuilt binaries for **mesimon**, a terminal kanban board that orchestrates
many coding-agent sessions. This repository carries releases only; the source
lives in a private repo and is not needed to run it.

**Status: early alpha, shared for feedback.** It works and is used daily by its
author. It will change under you.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/amitozalvo/mesimon-releases/main/install.sh | sh
```

Nothing else to install first.

Then:

```sh
mesimon doctor            # checks your environment, prints fixes, changes nothing
cd <a git repo> && mesimon
```

Re-running the install line is how you update.

## Requirements

- **macOS on Apple Silicon.** That is the only build published.
- **git**.
- **[Claude Code](https://claude.com/claude-code)** on your `PATH`, to spawn
  Claude sessions.

tmux is **not** a prerequisite: mesimon ships its own, installed alongside it as
`mesimon-tmux`. It never shadows or touches the tmux you already have — agents
run on a private server with a generated conf, exactly as before.

`mesimon doctor` checks all of these and tells you the exact line to fix any
that are missing.

## What it writes

- `<repo>/.mesimon/` — your board. Excluded from git via
  `$GIT_DIR/info/exclude`, never `.gitignore`.
- `$GIT_DIR/info/exclude` — one line, so the above does not show in `git status`.
- `~/.local/state/mesimon/` — sessions, worktrees, logs, its private tmux socket.
- Git worktrees and `msmn/*` branches it created, for tickets you put in
  worktree mode.

Nothing else. Not your shell rc, not your git config, not your `~/.claude/`,
not your tmux config.

## Stopping everything

Agents keep running after the board closes — that is the point of the daemon.
To stop them:

```sh
pkill -f "mesimon daemon"                                    # the daemon
tmux -S /tmp/mesimon-$(id -u)/<project key>/tmux.sock kill-server   # the agents
```

`mesimon doctor` prints the project key. Inside the board, `Z` parks every idle
session, which is the gentler version.

## Feedback

Issues on this repo are read. `mesimon doctor` output is the single most useful
thing to include — it is ASCII-only and redacts your home directory precisely
so it can be pasted.

## License

Apache-2.0.
