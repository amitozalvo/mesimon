# Changelog

Versions are `0.1.0-alpha.N` until the walking skeleton is something a stranger
can rely on. Alphas can and will change state-file formats; when they do, the
old file is preserved, never overwritten.

## v0.1.0-alpha.5

- **Linux ships — and with it, Windows through WSL2.** Two static builds, `x86_64` and
  `aarch64`, cross-linked from the same Mac that cuts the macOS release, installed by the same
  `install.sh` (it reads `uname`), updated by the same in-board offer. tmux comes from your
  distro (3.3 or newer; `mesimon doctor` names the floor and the `apt` line) — the Linux
  package bundles none.

- **Every tmux before 3.6 rewrote a tab in `-F` output as `_`.** The suite had only ever run
  on 3.6a, the one brew and the bundle share. On Debian's 3.3a and Ubuntu's 3.2a and 3.4 the
  daemon's every snapshot parsed to nothing while the panes sat alive: a restart read every
  session as crashed, and an interrupted turn never read as idle. The separator is a printable
  `|` now, and the release gate runs the whole suite on a distro tmux in Docker before it will
  cut anything.

- **Copying from an agent pane reaches the clipboard on Linux too**: `clip.exe` under WSL,
  `wl-copy` on Wayland, `xclip` on X — decided when the server starts, not when the binary is
  built.

- **`↑`/`↓` in the prompt field recall what this board asked before.** The draft under the
  cursor is kept and comes back one step past the newest entry, the way a shell's history
  behaves. Fifty entries, in memory, per run — a recall aid; the transcript is the record.

- **The interrupt probe waits a full minute, not eight seconds.** A working pane was measured
  silent for up to ~50 s while a large tool input streamed, and the old threshold read that as
  an interrupted turn. The release check asks every half hour rather than every six.

- **`mesimon doctor` knows WSL**: it names it on the `os` line, warns when the repo sits on a
  Windows drive under `/mnt` (git across that boundary is an order of magnitude slower), and
  warns when `curl` is missing, since the update check is silently inert without it.

## v0.1.0-alpha.4

- **`alt`+direction moves a card.** Option plus `hjkl` or an arrow key nudges the
  selected card one step and takes the cursor with it — the gesture every editor
  and every list in the OS already binds. The board's only mover was `> <`, a
  ghost you aim and an `Enter` you commit, three keys deep for "this one goes
  right". `> <` is unchanged and stays the spelling that works everywhere:
  terminals that eat the Option modifier make the new key inert, never wrong.
  (On iTerm2, `⌥←`/`⌥→` are mapped to a word jump by default — the left Option
  key with `hjkl` or `↑↓` works out of the box.)

- **`.` does the last move again.** Triage is a run of the same gesture — *these
  four go to done* — and it now costs one key per card. `.` files the card under
  the cursor into the column the last move went to, and the cursor stays put, so
  the next card slides up under it and `. . .` files three without travelling
  back. The column is remembered by name, so a rename or a delete takes the key
  out of service instead of quietly re-aiming it.

- **A launching agent shows it.** Shift+Enter in the composer stays on the board
  — the card is how you watch the work land — and the card sat there with a
  title and no sign of life until the agent's first hook arrived. It now wears
  the working arc at a quarter speed for the whole launch: spawning is not a
  different thing from working, it is working that has not started, and the
  slowness is the message. A ticket waiting on its worktree gets the same mark,
  which is the longest wait on the board.

- **An axis holds ten tags, not five.** Five is a sample, not a vocabulary, and
  a user who wants six components on one group is not building a list. Ten is
  also the number of groups, so both numbers in the tag system are now the same
  number. Ten names do not fit a picker row on an 80-column terminal, so the row
  scrolls with the cursor instead of being cut off: the cell you are on is
  always drawn, and a `~` marks the side still holding tags.

## v0.1.0-alpha.3

- **Your shell environment reaches your agents.** An `export` you add to
  `~/.zshrc` now arrives in the next session mesimon spawns. It could not
  before: a Claude pane is exec'd directly by tmux, so no shell runs and no
  startup file is ever read on that path, and the nine variables mesimon passed
  through were the agent's whole world. They were frozen besides — tmux captures
  its environment when its server first starts, so a board left running for two
  days handed new panes a two-day-old `PATH`, and neither reopening mesimon nor
  sleeping the session could shake it loose.

  mesimon asks your login shell what your environment is now, the same way your
  terminal does, and hands that to every pane. It withholds a short fixed list
  (tmux's own plumbing, a description of somebody else's terminal, its own
  `MESIMON_*`) and passes the rest. Nothing is written to your startup files and
  nothing runs that your terminal does not already run.

  A live pane keeps the environment it was born with — no running process's
  environment can be changed from outside — so this reaches new sessions and
  woken ones. When a startup file changes the header offers
  `shell env changed (esc)`, and the Esc menu row says which sessions it will
  and will not touch. It is never automatic: re-reading means running your rc
  files, and doing that every time an editor saves is not mesimon's call.

- **Leaving Claude parks the session instead of burying it.** Ctrl+C, `/exit`
  and Ctrl+D end the process, not the conversation, so mesimon records that as
  sleeping and `x` wakes it — the same key that put a session to sleep in the
  first place. The card used to go to a dead mark, and the way back was an
  unlabelled `Enter` on the session rail that nobody found. The ticket also
  keeps its worktree while the session is away. A crash still reads as a crash:
  only a clean exit parks, and only when waking would actually work.

- **`/clear` no longer kills the card.** Clearing the conversation inside a
  living pane was recorded as the session ending, so the card stayed dead for
  the rest of the session while the agent went on working in it.

- **A session with nothing to resume starts fresh instead of refusing.** A
  session that ended before its first prompt has no conversation to come back
  to, and `Enter` answered "no transcript to resume" forever. It starts a new
  conversation in the same row now, and says that is what it did.

- **`x` on a dead session dismisses it.** The key was documented and had no
  handler — the press came back "only idle sessions sleep". The conversation is
  untouched; only the rail stops showing the row.

- **A card can be reordered inside its own column.** Dropping the move ghost
  back into the column it came from did nothing at all: mesimon reported success
  and put the card back where it was. Reaching an in-column reorder still means
  going out with `>` and back with `h` — `> <` is "move card" between columns,
  and a grab-in-place gesture is not built yet.

- **The ticket page gives the whole page to its preview.** A DOCUMENTS
  placeholder — two lines about a feature that does not exist yet — was holding
  the top of the page above the agent transcript and shell preview. Tags also
  read before the branch name on the ticket's header row now.

- **`^t` works on the ticket page.** The tag picker was bound there and drew
  nothing: the footer named the keys, the keys were live, and the grid was never
  on screen. Repeating a digit also wraps around its row now, instead of walking
  off the end and going dead.

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
