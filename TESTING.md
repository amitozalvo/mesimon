# Trying mesimon, and what is useful to say back

Thanks for running this. It is an alpha shared with a handful of people; the
point is to find out where it is wrong, not to have it work perfectly.

## Before anything else

```sh
mesimon doctor
```

Paste that output into any report. It is ASCII-only and redacts your home
directory precisely so it can be pasted; it names your tmux and Claude Code
versions, the daemon's build, and anything mesimon could not read. Almost every
"it did nothing" turns out to be visible there.

## What is worth reporting

**Anything in this list, however small:**

- mesimon wrote somewhere outside the allowlist in
  [`docs/PROMISES.md`](docs/PROMISES.md). This is the most important possible
  bug — it is a product promise, not a preference.
- A session you did not start, or a session that survived something that should
  have stopped it. Cost and lifecycle are the whole reason this exists.
- The board says a session needs you and it does not, or the reverse. The
  attention state machine is the least-proven part of the system.
- Anything that made you lose work: a branch, a worktree, a ticket, a
  conversation.
- A board that will not open, or opens empty when it should not.
- The board looked wrong — misaligned, clipped, wrongly coloured, garbled after
  a resize. Say which terminal, at what size, and paste a screenshot; terminal
  differences are exactly what a single-author project cannot see.

**Also genuinely useful, and easy to leave unsaid:**

- The moment you had to guess what a key did, or guessed wrong.
- Anything you expected to be able to do and could not find.
- Where it felt slower than it should.
- Whether the thing is worth using at all. "I stopped opening it after Tuesday"
  is a finding, not a complaint — please say it.

## How to report

Open an issue on the repo. A title, what you did, what happened, and the
`mesimon doctor` output is plenty — no template to fill in.

If something is on fire and you need everything to stop:

```sh
pkill -f "mesimon daemon"
tmux -S /tmp/mesimon-$(id -u)/<project key>/tmux.sock kill-server
```

The project key is in `mesimon doctor` output. Killing the daemon does not kill
your agents; the second line does.

## Things that are already known

Save yourself the typing:

- **Published builds target Apple Silicon macOS and Linux on x86_64 or aarch64, including
  WSL2.** Intel macOS remains unshipped.
- **`?` opens the help screen.** The footer also names the keys for whatever you are
  looking at.
- **The docs corpus in `docs/` predates the code** and is not a user manual. It
  is a research record; large parts of it describe things that do not exist.
- **State files can change format between alphas.** When that happens mesimon
  preserves the old file and tells you, rather than migrating silently.
- Sessions mesimon did not spawn can be adopted, but stay observe-only until you
  take them over — they cannot report that they need you.
