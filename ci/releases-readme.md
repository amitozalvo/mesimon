# mesimon — releases

Prebuilt binaries for **mesimon**, a terminal kanban board that runs your coding agents. Like
Kubernetes is to containers, mesimon is to Claude Code and Codex. This repository carries
releases only. The source, and the full documentation, are at
[amitozalvo/mesimon](https://github.com/amitozalvo/mesimon).

**Alpha.** It works and it is used every day, and it will change under you.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/amitozalvo/mesimon-releases/main/install.sh | sh
```

Or with Homebrew:

```sh
brew install amitozalvo/tap/mesimon
```

Then:

```sh
mesimon doctor            # checks your setup and prints fixes; it changes nothing
cd <a git repo> && mesimon
```

You need macOS on Apple Silicon or Linux (x86_64 or aarch64, WSL2 included), git, and Claude
Code or Codex, signed in. On Linux you also need tmux 3.3 or newer; on macOS mesimon brings its
own. [Requirements in detail](https://github.com/amitozalvo/mesimon/blob/main/docs/USING.md#requirements).

Re-running the install line is how you update. With Homebrew, `brew upgrade mesimon`.

## Three promises

1. **A strict write allowlist.** mesimon writes to a short list of paths, every one named, and
   nowhere else: never your shell rc, your git config, your agent configuration or your tmux
   config.
2. **No config mutation.** `mesimon doctor` prints fixes for you to apply; it never applies one
   itself.
3. **Zero prompt injection.** mesimon adds, removes and reorders no token of your conversation.
   The board tools it gives its agents, and a brief you can switch on, are shown in full.

[The promises in full](https://github.com/amitozalvo/mesimon/blob/main/docs/PROMISES.md), with
every path mesimon writes.

## Stopping everything

Agents keep running after the board closes. Inside the board, `x` sleeps one ticket's sessions
and `X` sleeps the finished agents in DONE. To stop every agent in a repo,
[Stopping everything](https://github.com/amitozalvo/mesimon/blob/main/docs/USING.md#stopping-everything)
gives the two commands.

## Feedback

Issues on this repo are read. `mesimon doctor` output is the single most useful thing to include:
it is ASCII-only and redacts your home directory, so it can be pasted.

## License

Apache-2.0.
