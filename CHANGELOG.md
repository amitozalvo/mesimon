# Changelog

Versions are `0.1.0-alpha.N` until the walking skeleton is something a stranger
can rely on. Alphas can and will change state-file formats; when they do, the
old file is preserved, never overwritten.

Each release is a `## <tag> — <date>` heading, newest first. The file is compiled
into the binary and shown by the Esc menu's "Release notes" row (`core/src/relnotes.rs`
parses it; `cargo ut` checks every heading), and `ci/release.sh` lifts the tag's
section out of it for the GitHub release body.

## v0.1.0-alpha.13 — 2026-09-04

- **`z` snoozes a ticket.** On a card or its page, `z` arms a ring — `1h · 4h · tomorrow 9:00 ·
  next Monday 9:00` — and a second `z` walks it; Enter snoozes, Esc or any stray key cancels.
  The armed card opens with the preset on its own row and blinks until you confirm. A snooze IS
  an archive with a deadline: the ticket leaves the board, the ARCHIVED row reads `wakes in 3h`,
  and `a`/`u` bring it back early. It returns at the TOP of its column with its age restarted
  and, unless you turn that off in Settings, lit as needing you — the mark comes off when a
  keypress leaves your cursor on it. A ticket whose agent has finished its turn is put to sleep
  on the way out; one still working holds the ticket on the board and nothing is touched.

- **An ask can wait for a quiet checkout.** In the board's ask field, Shift+Tab flips `now` /
  `queued`: a queued ask is parked until no claude sharing the ticket's checkout is mid-turn,
  then pasted — so five "commit" asks in one checkout land one at a time instead of on top of
  each other. Offered on shared-checkout tickets only (a worktree's checkout is its own). The
  card wears a slow mark and, open, says `queued ∙ after T-12`; Shift+Enter reopens the words,
  a blank Enter drops them, and talking to the agent yourself while it waits drops them too.

- **A merge train, opt-in.** Settings has a `Merge train` row (off by default). While every
  claude on the board is idle, mesimon fast-forwards the first finished REVIEW branch, tells its
  agent (its own row, on by default), and asks one idle agent whose branch fell behind to rebase
  and test — once per move of main, fused at six asks in two hours. It runs only while the
  board that turned it on is open; `mesimon doctor` prints a `merge train` line. The cards say
  what is coming (`merge ∙ after T-3 +1`), and the header reads `∙ train`. A shell on a ticket no
  longer blocks `m`.

- **The board says where its checkout stands.** The header hangs `⎇ main ↑2 ↓1 ∙ 3 changed` off
  the breadcrumb: the branch you are on, commits due to push, commits due to pull, files changed.
  Fetching is yours to ask for — the Esc menu's `Fetch origin` row, or `MESIMON_GIT_FETCH=<minutes>`
  — and it writes nothing but the remote-tracking refs. The clause yields to a suggestion chip
  when the header is tight: the count goes first, then the branch name truncates; the arrows are
  never cut.

- **Shift+Enter wakes a sleeping claude and asks it.** The board's ask had a hole: a ticket whose
  claude was parked was neither an empty seat nor promptable, so the key did nothing and the road
  was `c`, wait, `q`, ask. The field opens there now (`wake + ask claude`) and the daemon wakes
  the agent on the way, holding your words until its first breath. Alongside it, a finished turn
  can no longer be lost: four asks in twenty seconds used to trip the attention machine's flap
  guard, which dropped the real `Stop` and left the spinner on a card that was done. What the
  agent states now commits through the guard; only guesses are held back.

- **A new board comes with three tags.** A board that has never had a tag opens with `BUG`,
  `FEATURE` and `CHANGE` on group 1, coloured rose, green and blue, so the first `^t` is a pick
  rather than a blank row. Offered once: a board with tags of its own is left alone, and
  forgetting the three does not bring them back on the next start.

- **`HJKL` moves the card.** The board's nudge was reachable only through `alt+hjkl`, a dead key
  on any terminal that eats the modifier, and the footer taught only that spelling. The shifted
  letters now carry the card the way `hjkl` walks the cursor — the arrangement the tag picker
  already had — and the footer names it.

- **`X` sleeps the done column's agents.** `z` took the letter the bulk sleep used to sit on, so
  the bulk sleep moved onto `x`'s own shift: same verb, wider target — the selection's sessions
  for `x`, DONE's agents for `X`. The header's chip says `(X ∙ esc)`.

- **The week starts on your day.** Settings has a `Week starts on Monday` row; Enter cycles
  Monday → Sunday → Saturday. It is what the snooze ring's last rung means by "next week":
  `z` walks `1h · 4h · tomorrow 9:00 · next Sunday 9:00` for a Sunday week. Saved in
  `prefs.json`; `mesimon doctor`'s `snooze` line names the day.

- **A reload waits for the daemon it stopped.** `U` on a busy machine could leave a dozen live
  sessions with no board: the shutdown got two seconds, the next daemon five, and a shutdown that
  flushes every pending settle outlasted both. The lock is the clock now, not a stopwatch — the
  reload waits for the old daemon to let go, and the client that finds one still holding on waits
  rather than racing it. And no daemon at launch is no longer fatal: the board opens empty with
  the reason in the advisory row and connects when one appears.

- **Smaller.** The theme picker's row detail is the blurb alone — the row's own ground tag
  already said which slot a theme belongs to.

## v0.1.0-alpha.12 — 2026-09-04

- **Release notes, in the board.** The Esc menu has a `Release notes` row that opens this file
  on a screen of its own: one band per release, the running build marked, the notes as rich
  text under each. `j`/`k` and `{ }` read, `n`/`N` step between releases, `q` returns. The
  notes ship inside the binary, so what a build shows is what it is.

- **`^s` saves and leaves; `^S` saves and hands the words to claude.** In the note editor `^s`
  writes the note and closes it (it used to need a second press). In the grown composer `^s`
  keeps the description and folds back into the one-line composer, where Enter mints. `^S`
  (ctrl+shift+s) is the bigger room's Shift+Enter: composing, it mints the ticket, writes the
  description and starts claude on the title; on a note it tells the ticket's running claude
  the note changed, or starts one if the seat is empty. A sleeping claude leaves it inert —
  `c` wakes it. `^S` needs a terminal that can spell it; a legacy terminal sends `^s`.

- **`^g` opens the note in your own editor.** `$VISUAL`, else `$EDITOR`, else `vi`, on the
  terminal mesimon hands back for the duration. What comes back is saved at once on a note, or
  dropped into the draft while composing. The hint names the program; `mesimon doctor` prints
  an `editor` line.

- **A finished agent's mark says whether you have read the reply.** The card wears the heavy
  `✔` in the calm register while its agent's last reply is one you have not been on the card
  for, and the thin grey `✓` the moment the cursor lands or the ticket page opens. No cell is
  spent: the signal is the glyph. A fresh board finds every reply unread.

- **Esc lands in seconds, and an interrupted turn wears `⊘`.** Interrupting claude used to
  leave the ticket at `running` for up to two minutes, or forever when the interrupt wrote no
  transcript record at all. Both spellings of the record are read now, and the recordless Esc
  is caught off Claude Code's own session file within about two seconds. The state the card
  lands in finally has a mark and a word: `⊘ interrupted`.

- **Your ask outranks your own park.** `<<` a card to TODO and then Shift+Enter, and it lands
  in IN PROGRESS. Automove refused that move as ping-pong against your drag; a prompt from the
  same hand is the newer intent and now supersedes it. An agent's move and the flap fuse keep
  their protection.

- **Two phosphors with white ink, a red delete flash, and Solarized light.** Amber and green
  stop painting body text in the glow: green is a hued ground and accent under white ink, amber
  its original ladder with a white base step. Arming a delete flashes the card red (the title
  row on the ticket page too), on every theme. Solarized light joins as the sixth theme, picked
  from the same menu row.

- **An approved plan lands as a note on Claude Code 2.1.259.** That build moved the plan from
  the tool's input to its response, and the note stopped being written. Both shapes are read.

- **`{ }` on the ticket page glides.** A page turn scrolls the preview over the same 180 ms the
  composer takes to grow, and a press mid-glide continues from where the eye is, so a held key
  is one continuous scroll.

- **Smaller.** Shift+Tab in the tag picker walks the tint ramp back, wrapping. The ticket rail
  no longer says `x wake` under a sleeper whose other keys already say it; `x` still wakes. The
  grown composer names Shift+Tab beside the workspace word rather than spelling it in the frame.

## v0.1.0-alpha.11 — 2026-09-03

- **An approved plan becomes a note on the ticket.** When you approve an agent's plan, mesimon
  writes it as a note under the ticket, authored by that agent. Re-planning revises the same
  note rather than adding another; a ticket that had no description gets the plan as one.

- **An agent can tag its ticket.** `tag_ticket` is the seventh tool: it wears one of the board's
  existing tags on the agent's own ticket, or takes it off. An agent never coins a tag — a name
  the picker has not seen is refused with a pointer at the list `get_ticket` now carries, and a
  name that lives on two axes is refused until the agent says which. `create_ticket` takes tags
  the same way, so a filed card can land already sorted.

- **`Tab` on a card opens its description.** The same dialog the composer grows into now opens
  out of any card on the board, holding that ticket's description; `Shift+Tab` in it still
  chooses the workspace while nothing has locked it. (`Tab` used to walk the needs-you cards; it
  was hinted and never pressed.)

- **The ticket page's head is a band.** Title, state line and description sit together on the
  elevated surface, edge to edge, with the tag chips and the rail under them. The state line reads
  as a sentence at every age, a long branch name is cut rather than clipped, and the rail no longer
  offers `c` beside a claude it already lists.

- **Shift+Enter in the editor is a newline.** In a description or a note it used to save and ask
  claude, so the press that wanted a blank line minted a ticket and started an agent. `^s` saves;
  the board's Shift+Enter on the card still asks. `^]` closes the editor too, as it does everywhere
  else.

- **Smaller.** A worktree's branch and directory are named by a handful of words from the title,
  not half a sentence cut mid-word. The board hints `space` for the ticket page where `enter` goes
  to the agent. A note survives delete + undo. The footer keeps the screen's keys on the left and
  `esc menu` / `? keys` on the right, and each floating dialog carries its own keys in its frame.

## v0.1.0-alpha.10 — 2026-09-03

- **The bigger composer is a dialog on the board, not a panel across it.** `Tab` from the
  one-line composer still grows out of the card you are writing, but what it grows into is now
  the card itself, bigger: the same surface, the same coloured bar down its left edge wearing the
  tags you picked, the title on the first row and the column it lands in named under it. The
  dialog covers whole columns rather than floating over cut-off cards, so the board stays
  readable on both sides of it. Esc, `^s`, Shift+Enter, `^t` and Shift+Tab work as before.

## v0.1.0-alpha.9 — 2026-09-03

- **A ticket has a description and notes, and they are markdown files.** Every ticket can carry
  prose now: the description shows on the ticket page under the title, and any number of notes
  sit in the rail beside the sessions. They are real markdown files under the ticket, so an
  editor, a `grep` and a `git diff` all still work on them. `n` opens the note under the cursor,
  `N` starts a new one, `^s` saves, and a second `^s` on a saved note tells the ticket's agent
  it changed.

- **The composer grows into a full editor with `Tab`.** A one-line title is often not enough to
  say what a ticket is. `Tab` in the composer opens a multi-line editor for the description —
  as a panel over the board that grows out of the card you are about to create, so it is clear
  which card you are writing. The context line names the column, the workspace and the tags it
  will be minted with.

- **Shift+Enter on a ticket with no agent starts one on the title.** It already minted-and-asked
  from the composer and asked a running agent from the board; the one gap was a ticket that had
  been sitting in TODO. It now spawns claude there and submits the title, the same thing the
  composer does, without leaving the board. A sleeping agent is not an empty seat — `c` still
  wakes it.

- **An agent can file a ticket.** `create_ticket` is a sixth tool for the sessions mesimon
  starts: an agent that finds work outside its ticket's scope can file it instead of doing it
  or losing it. The new card lands in the first column (or one you name) with no session on it,
  and nothing an agent can call starts one — you decide what happens to it. `mesimon doctor
  --mcp` prints the tool verbatim, as it does the other five.

## v0.1.0-alpha.8 — 2026-09-02

- **Pasting several lines no longer saves the first one and types the rest.** The composer, a
  rename and the ask field took a paste as keystrokes, so a newline was an Enter: the first line
  became the ticket and the lines after it walked the board as keys. A paste is now one event.
  It goes into whichever text field is open, flattened to one line (newlines become spaces),
  and with no field open it does nothing at all.

- **A title has a ceiling, and a big paste says when it hit it.** Titles are capped at 2 KB,
  tags and asks keep their existing caps, and every field enforces the same number the daemon
  does. A paste past the limit is cut at a character boundary and the status line says
  `paste trimmed ∙ a title holds at most 2 KB`; typing past it is inert.

- **A sleeping ticket's colour block is no longer extra-muted.** The bar had three loudnesses
  and the quietest, for a parked ticket, read as washed out. There are two now: the cursor card
  at full strength, every other card one step down. The glyph is what says asleep.

## v0.1.0-alpha.7 — 2026-09-02

- **The board no longer re-asks the terminal whether it is light or dark.** The 3-second
  re-query behind live theme switching could land its reply on stdin as keystrokes, and the
  alpha.6 guard did not catch every shape: a reply still opened rename on a ticket with its
  bytes in the title. The periodic query is off; the launch still asks once, so the theme
  picker still lands on the right slot. Live repaint on an OS appearance flip is gone with it
  — relaunch, or pick from the Esc menu. `MESIMON_GROUND_WATCH=1` re-arms it.

## v0.1.0-alpha.6 — 2026-09-02

- **Five themes, picked from the Esc menu.** Graphite and chalk are joined by blue (navy and
  gold, the Borland look), amber and green (one phosphor each, a P3 and a P1 monitor). The
  `Theme:` row opens a picker whose cursor IS the preview — the board repaints as you move,
  Enter keeps, Esc puts the old one back. The choice is saved per ground, one theme for a dark
  terminal and one for a light one (`~/.local/state/mesimon/prefs.json`), so an OS appearance
  flip still lands on a theme you chose. `MESIMON_THEME` accepts every name and still pins the
  launch; `mesimon doctor` prints a `theme` line. Every palette passes the same colour law —
  one saturated colour on the board, reserved for needs-you — extended to a ground that is a
  colour and to a single-hue phosphor.

- **One claude per ticket.** A ticket now holds one Claude session and its second seat is a
  shell: `c` on a parked claude wakes it instead of starting another, and `C` is gone. Every
  place that picks "the" agent of a ticket assumed one, and with two the automove ping-ponged
  the column between their turns.

- **`{` `}` (or `pgup`/`pgdn`) page the ticket page's preview.** Hinted only while the zone
  overflows; a shell's tail keeps following its bottom until you scroll it, and returns to
  following when you scroll back down.

- **A late reply to the colour query can no longer type into the board.** A terminal that
  answers the light/dark probe after the 150 ms budget lands the answer on stdin as keystrokes;
  it is recognised and discarded before the keymap sees it.

- **The peek shows what the agent was actually asked.** A harness task notification (a
  backgrounded build finishing, a subagent reporting) is no longer shown as a prompt with the
  agent "thinking" under it.

- **A sleeping agent's `z` recedes with its bar**: the glyph walks the same dim ladder the tag
  stripe already does, so a parked ticket reads as parked from either.

## v0.1.0-alpha.5 — 2026-09-02

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

## v0.1.0-alpha.4 — 2026-09-01

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

## v0.1.0-alpha.3 — 2026-09-01

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

## v0.1.0-alpha.2 — 2026-09-01

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

## v0.1.0-alpha.1 — 2026-09-01

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
