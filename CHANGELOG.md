# Changelog

Versions are `0.1.0-alpha.N` until the walking skeleton is something a stranger
can rely on. Alphas can and will change state-file formats; when they do, the
old file is preserved, never overwritten.

## v0.1.0-alpha.2

- **mesimon ships its own tmux.** A fresh Mac now needs nothing installed —
  `brew install tmux` is gone from the instructions. It is a statically linked
  tmux 3.6a (libevent, ncursesw and utf8proc all static; only libSystem and
  libresolv dynamic), built by `ci/build-tmux.sh` from pinned, checksummed
  sources, and it reads macOS's own terminfo database so no data files travel
  with it.

  It installs as `mesimon-tmux`, deliberately not `tmux`: installing under the
  real name would put it on your PATH and shadow your own tmux, which is the
  exact trespass mesimon promises never to commit. Your tmux, its version and
  its config are untouched — mesimon has always run its agents on a private
  server with a generated conf, and now it owns that server's binary too.

  This also closes a bug class rather than just an install step: a tester on
  tmux 3.2a could hit behaviour the author could not reproduce on 3.6a. The
  release gate now runs the whole e2e suite against the bundled binary, so the
  tmux that ships is the tmux that was tested.

  `MESIMON_TMUX_BIN` overrides the choice; `mesimon doctor` reports which tmux
  is in play and where it came from.

## v0.1.0-alpha.1

The first build shared outside the author's machine.

### Survivability

- **A malformed state file no longer stops the daemon.** `columns.toml`,
  `ticket.toml`, `sessions.json` and `worktrees.json` each carry a
  `schema_version`, and each is handled independently: unreadable files are
  moved aside as `<name>.quarantine-<ms>` with every byte intact and the board
  comes up on a default; files written by a *newer* mesimon are left exactly
  where they are and never written over. Either way the board says so and
  `mesimon doctor` names the file. Previously any parse error killed the daemon
  at startup, after the socket was already bound, so the board just said
  "daemon did not come up" and the reason was buried in a log.
- **Losing `worktrees.json` no longer orphans your worktrees.** It used to load
  as an empty map on any parse error and then get overwritten, which stranded
  every real git worktree and `msmn/*` branch with nothing to rebuild from.
  Bindings are now reconstructed from `git worktree list` plus mesimon's own
  ownership markers, and worktree actions pause until that recovery verifies.
- **Recovering `next_key`.** A lost `columns.toml` reset ticket numbering to
  zero, so the next ticket minted `T-1` and wrote over the existing `T-1`.
  The highest key on disk is now recovered first, counting ticket directories
  that could not be parsed — their keys are taken too.
- Board files are written with an fsync before the rename (13 §13.9.1), so a
  crash mid-write stops producing the truncated files above.

### Updating

- **The daemon can no longer run older code than the board.** It reports its
  build in the handshake; a newer client shuts it down and brings it back on
  the new binary, silently, before drawing anything. Sessions are untouched —
  panes live on the private tmux server and records re-derive. Only daemons
  mesimon started itself are ever restarted, at most one attempt, and a client
  older than its daemon leaves it alone.
- Re-running `install.sh` is the update. An open board offers
  `update ready (U reloads)`.

### New

- **`mesimon doctor`** — checks environment, install path, tmux (and its 3.1
  floor), the `claude` binary, git, the running daemon's build, and any
  quarantined state files. Prints copy-pasteable fixes and applies none, ever.
  ASCII-only and `$HOME`-redacted, because its output belongs in bug reports.
- `mesimon --version` now carries the commit and build date.

### Project

- `ci/release.sh` is the release: clippy `-D warnings`, a duplicate-dependency
  drift gate, and the full suite with tmux **required** (a skipped e2e suite
  certifies nothing), then build, verify the code signature, run the packaged
  artifact, and upload a checksummed tarball. It refuses a dirty tree, a tag
  that is not HEAD, a tag that does not match the workspace version, and a tag
  that is not on origin.
- The build runs on the maintainer's machine rather than a GitHub runner:
  macOS runners bill at 10x on a private repo, and the only target shipped is
  the machine it is developed on. `.github/workflows/ci.yml` is kept for the
  clean-room check a laptop cannot give (fresh checkout, empty state dir) and
  runs on demand only.
