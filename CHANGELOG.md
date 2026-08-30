# Changelog

Versions are `0.1.0-alpha.N` until the walking skeleton is something a stranger
can rely on. Alphas can and will change state-file formats; when they do, the
old file is preserved, never overwritten.

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

- CI on macOS arm64: fmt, clippy (`-D warnings`), the full suite with tmux
  required, and a duplicate-dependency drift gate. Tagged releases build,
  verify the code signature, smoke-test the artifact, and publish a checksummed
  tarball.
