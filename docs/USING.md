# Using mesimon

The [README](../README.md) gets you to a first agent. This page is the rest.

- [Requirements](#requirements)
- [Installing](#installing)
- [Updating](#updating)
- [On the board](#on-the-board)
- [Descriptions, notes and links](#descriptions-notes-and-links)
- [Keyboard layouts and terminals](#keyboard-layouts-and-terminals)
- [Agents and follow-ups](#agents-and-follow-ups)
- [Sleeping idle agents](#sleeping-idle-agents)
- [Keeping the machine awake](#keeping-the-machine-awake)
- [Pictures in notes](#pictures-in-notes)
- [What your agents can see](#what-your-agents-can-see)
- [Stopping everything](#stopping-everything)
- [Investigating agent state](#investigating-agent-state)

## Requirements

- **macOS on Apple Silicon, or Linux on x86_64 / aarch64 — WSL2 included.** The published
  builds are `aarch64-apple-darwin` and a static-musl pair for Linux, so one binary runs on any
  distro. The code is Unix-only by design (unix sockets, tmux): on Windows, WSL2 is the way in,
  and keep the repo in the Linux filesystem, not under `/mnt/c` (`mesimon doctor` says so too).
- **git**. On macOS tmux is *not* required — mesimon ships its own, installed as `mesimon-tmux`
  so it never shadows yours. On Linux, install the distro's (`sudo apt install tmux`, 3.3 or
  newer; `mesimon doctor` names the floor).
- **Claude Code or Codex** on your `PATH`, authenticated through its native CLI.
  Codex runtime integration is tested against `codex-cli 0.153.4`; `mesimon doctor`
  reports installed versions and the measured compatibility boundary.

## Installing

```sh
curl -fsSL https://mesimon.dev/install.sh | sh
mesimon doctor    # confirm the environment
```

Or with Homebrew, on macOS (Apple Silicon) or Linux:

```sh
brew install amitozalvo/tap/mesimon
```

The formula installs the same published binaries, and on macOS the same bundled tmux as
`mesimon-tmux`. Homebrew updates it: `brew upgrade mesimon`.

Binaries are published from
[amitozalvo/mesimon-releases](https://github.com/amitozalvo/mesimon-releases), a public repo that
carries releases and nothing else — so installing needs no GitHub account and no login.
`install.sh` verifies the checksum, runs the binary once before installing it, and tells you
exactly what to fix if anything is missing.

Working on mesimon itself? `cargo install --git https://github.com/amitozalvo/mesimon --locked
mesimon` builds it from source (Rust 1.88+).

## Updating

**A board tells you.** Every half hour it asks the releases repo whether a newer version is out,
and when there is one the header says so: `◦ v0.1.0-alpha.5 available (esc)`. Esc opens the menu,
`Install v0.1.0-alpha.5` downloads it and checks it against the published checksum, and then the
offer becomes the one below — nothing restarts until you press `U`.

Nothing about that is automatic except the question. mesimon never installs a version you did not
ask for, and never restarts a board you did not tell it to.

**And it tells you what changed.** Esc → `Release notes` opens every version's notes, newest
first, with `this build` on the one you are running. They are the binary's own (`CHANGELOG.md`,
compiled in), so the page reads offline. Opening it also asks, right then, whether a newer version
is out; if one is, the header chip above appears.

**Or ask from a shell.** `mesimon update` checks now and, when a newer version is out, downloads
it, checks it against the published checksum and puts it in place of the binary you ran. It
restarts nothing: an open board offers `U`. `mesimon update --check` only asks.

- `MESIMON_NO_UPDATE_CHECK=1` turns the board's check off. `mesimon update` still works when you
  run it yourself. `mesimon doctor` prints whether the check is on, when it last answered and what
  it heard.
- Development builds never check. Only the binary `ci/release.sh` cuts is stamped to, so a
  `cargo build` board makes no request and can never have its binary replaced by a download.

**Installed with Homebrew?** Then `brew upgrade mesimon` updates it, and mesimon never replaces a
binary Homebrew installed. The board still says when a newer version is out; its menu row reads
`Upgrade to v0.1.0-alpha.5 with brew` and copies `brew upgrade mesimon` for you to run.
`mesimon update` prints the same command instead of downloading. After the upgrade an open board
offers `U`, as below.

**Or re-run the install line**, which is still the whole procedure and the only one on a machine
where the check is off.

- If a board is open, it notices the new binary and shows `◦ update ready (U ∙ esc)`. `U`
  restarts it in place.
- If no board is open, the next one you start notices that the background daemon is running older
  code and restarts it for you, before drawing anything. Your sessions survive: they live on the
  tmux server, which the daemon does not own.

## On the board

```sh
cd <a git repo>
mesimon
```

On the board: `o` adds a ticket, `space` opens its page, `enter` goes to its agent (or opens the
page when it has none), `HJKL` moves the card (`>` or `<` twice does too), `p` shows each agent's
latest reply under its card, `q` quits.

`/` searches. Type any part of a ticket's title, its key, its column or a tag it wears and the
list narrows as you go; `ctrl-n` / `ctrl-p` walk it, `enter` puts the cursor on that card, `esc`
leaves the board where it was. Archived tickets are in the list, ranked under every live one and
marked; `tab` takes them out again.

![Typing csv into the search picker narrows fifteen tickets to one, and enter puts the cursor on its card](../assets/demo/search.gif)

<sub>Three letters narrow fifteen tickets to one, and `enter` scrolls the board to its card.</sub>

In the new-ticket composer, `shift-tab` cycles between the shared checkout and a dedicated
worktree; that choice locks once a session exists. On a ticket page: `c` starts an agent session,
`!` opens a terminal in the ticket's worktree or checkout, and `enter` steps into a live session:
its own terminal takes over your whole screen. `ctrl-]` (or `ctrl-5`) steps back out to where you
were. `v` shows the diff once there is a worktree, and `m` on the branch line merges it:
fast-forward only, so mesimon never mints a merge commit.

What you step into is the agent itself: Claude Code or Codex exactly as you run it without
mesimon, with the same prompt, the same slash commands and the same permission dialogs. Type to
it, answer its questions, interrupt it: mesimon never sits between you and it.

![From a ticket page into the agent's own terminal, a message typed to it, and back to the board](../assets/demo/agent.gif)

<sub>`enter` steps into the agent's own terminal, where you type to it as you always do, and
`ctrl-]` steps back out while it keeps working.</sub>

The board is deliberately quiet — exactly one saturated colour exists, and it means *this
session is waiting on you*.

`x` sleeps a ticket's sessions and wakes them again. When finished agents sit idle in DONE, `X`
sleeps them all. The Esc menu archives finished tickets and lists the archive. **Closing the board
does not stop your agents** — that is the point of the daemon.

The footer names the main keys for whatever you are looking at; `?` opens the complete key
reference for the current screen.

## Descriptions, notes and links

![Writing a ticket's description with tab, opening its page, and opening a link from its agent's note with ctrl-k](../assets/demo/ticket-page.gif)

<sub>`tab` adds two lines to the brief, `enter` opens the page with the note its agent left, and
`ctrl-k` opens one of that note's links.</sub>

`tab` on a card opens its description, the brief its agent starts from. `ctrl-s` saves it and
closes the editor, and `ctrl-g` opens the text in your own editor instead. The description is a
ticket's first note. Every other note is listed under NOTES on the ticket page, whether you
wrote it with `N` or the ticket's agent wrote it; `j` and `k` walk down to a note and show it.
Notes are markdown.

`ctrl-k`, on the board or on a ticket page, lists every link in the ticket's notes and in its
agent's latest reply: web addresses, files in the ticket's worktree or checkout (with a line
number when one is written), other tickets, and pasted pictures. `enter` opens one: a web
address in your browser, a text file in your editor, another ticket by moving to it. `c` copies
it instead.

## Keyboard layouts and terminals

On Hebrew and other layouts that mirror the bracket keys, `ctrl-]` arrives as Esc — which
interrupts the agent instead of detaching. `ctrl-5` is bound for exactly that and works on any
layout; in iTerm2 you can also fix the keystroke itself, leaving Escape alone: Keys → Key
Bindings → `ctrl-]` → Send Hex Code → `0x1d`.

Every key is an English letter, so on a layout whose letters are not Latin (Hebrew, Russian,
Greek, Arabic) a letter pressed on the board is no key at all. mesimon pauses instead of
guessing: the footer names the layout, and every character key is ignored until you switch to
English and press a letter, which then acts. `esc` dismisses the pause. Arrows, `enter`, digits
and the `ctrl` keys keep working, and text fields take any language.

If `shift-enter` does nothing and `?` does not list it, the terminal never reported the key:
mesimon asks for it through the kitty keyboard protocol and leaves the key unbound where the
answer is no. On iTerm2 the usual cause is a key binding on Shift+Enter — Claude Code's
`/terminal-setup` installs one that sends a plain newline. Delete the `⇧↩` row under Keys → Key
Bindings and Profiles → Keys, then start mesimon again, in an iTerm2 tab rather than inside your
own tmux.

## Agents and follow-ups

Choose the provider for new sessions with **Settings › Agents › Default tier**: `claude` runs
Claude Code and `codex` runs Codex. Claude Code is the initial default. Switching providers
leaves existing sessions, including sleeping ones, with their original provider. Accepted queued
starts keep their choice.
Each ticket has one live agent seat across both providers.

Follow-ups default to **Queue**: they wait for the current turn to end, including
approval and question stops. Choose **Steer** in **Settings › Behaviour › Follow-ups**
to send immediately by default, or toggle a composer with Shift+Tab. A queued
prompt appears on the ticket page; Ctrl+Y sends it now and Ctrl+U takes it back
for editing. Remote Control defaults to Queue and offers the same two actions.
The queue holds one prompt per ticket in memory; daemon restarts discard it.

## The crown: one agent runs the board

Press `ctrl-o` on a ticket to crown it. Its agent can then work on every other ticket through its
board tools: move, retitle, tag and archive them, write their notes, set their workspace, start an
agent on one, and leave words for another ticket's agent, which wait on that card until you send
them. Each edit is checked against the ticket as the agent last read it, and the card it lands on
lights with what was done (`♛ moved`, `♛ tagged`, `♛ started`). One ticket wears the crown at a
time. Only you can give it, and `ctrl-o` on the crowned card takes it back.

![Crowning a working agent's ticket with ctrl-o: its agent files two tickets, moves one, tags one and starts an agent on another](../assets/demo/crown.gif)

<sub>The crowned agent files two tickets, moves one back to TODO, tags one and starts an agent
on another; the moved, tagged and started cards light as they change.</sub>

The crown starts at most three agents at once; **Settings › Agents** changes the number or turns
starting off. A sleeping agent still holds its seat until its ticket is archived. A ticket the
crown started can never be crowned itself. Crowning types nothing into the agent's conversation: the crowned agent learns
it through its tools. When an agent it started delivers, answers what it asked, or raises its
hand, one sentence saying so is pasted into the crown's session. So is the merge of that agent's
worktree branch, whether `m`, the merge train or your own `git merge` made it. The crown is told
this when it reads its own ticket and in every `start_agent` and `ask_agent` receipt, so it has
nothing to poll: a background monitor it runs makes its session read as busy, and the wake waits
until the monitor ends.

## Sleeping idle agents

**Settings › Agents › Sleep idle agents** optionally sleeps finished agent sessions after
15, 30, 60 or 120 idle minutes. It is off by default and applies to this board, even with the
TUI closed. The daemon checks every five minutes, measuring from when the turn finishes.
Running turns, background work and sessions needing attention stay awake. Wake resumes the
same conversation. The board's `park_after_minutes` setting accepts any whole number of
minutes; `0` disables it.

## Light and dark themes

The board has two theme slots, one for a dark terminal and one for a light one. At launch it
asks the terminal which it is and wears that slot's theme. **Settings › Appearance › Theme**
opens on the state you are in; Tab switches the pick to the other state, then to both, so
both slots can be set without changing the terminal. Moving the cursor previews the theme
on the board behind the picker.

**Settings › Appearance › Follow the OS appearance** makes the board switch between the two
themes as macOS or your Linux desktop switches between light and dark, while the board is
open. It is off by default. It asks the OS, never the terminal, so turn it on only if your
terminal follows the OS appearance too: a terminal pinned to one profile would otherwise be
painted for the wrong background. `MESIMON_THEME=<name>` pins a theme for a launch and the
switch does nothing until a pick in the menu lifts the pin. `mesimon doctor`'s `theme` line
says which slot the OS would choose now.

## Keeping the machine awake

**Settings › Behaviour › Keep this machine awake** prevents sleep while an agent works,
including while you are attached to its pane. It is off by default. While enabled, a fixed-width
indicator beside the board's ticket count shows emoji-style `☕️` when preventing sleep and a dimmed crescent `☾`
when enabled but idle (`@` / `z` in ASCII mode).
The indicator disappears when the setting is disabled; activity changes do not shift the title.
From a column header, press Up / `k` to reach the board header, then Right / `l` to select
the indicator beside the ticket count. Enter opens its setting, selected and ready to toggle.
Left / `h` returns to repository status; Down / `j` returns to the column.
A turn waiting for user action releases the hold, as does closing the board.
On macOS the display and closed-lid behavior are unchanged. Linux requires `systemd-inhibit`;
its sleep lock can also block explicit suspend requests, depending on desktop policy.
`mesimon doctor` describes the available backend. WSL support remains opt-in and unverified.

`MESIMON_CAFFEINATE=off` overrides the setting. `MESIMON_CAFFEINATE=caffeinate` (or its absolute
path) uses a guarded macOS subprocess. Any other custom program named by this variable must
release its hold and terminate its children on stdin EOF; that contract is necessary for
cleanup after Mesimon is killed.

## Pictures in notes

Pictures can be pasted into a ticket note or new-ticket description with **Ctrl+V**
while the body is focused. Each appears as `[Image #N]`; save keeps the PNG with the
ticket, and the ticket's links menu (**Ctrl+K** on the board or the ticket page) opens it in
your desktop image viewer.
The ticket's agent can read pictures through `read_attachment`. Clipboard text still
pastes as text. Local macOS, X11, and Wayland desktops are supported (Wayland needs
clipboard data-control support); SSH and WSL image paste are not supported.
Pictures are limited to 10 MiB and 25 megapixels each, with 50 MiB of pending
pictures per draft. Discarding a draft discards its new pictures. Shared boards
currently synchronize the note text only: a picture absent on another machine is
reported as unavailable there.

## What your agents can see

An agent session mesimon starts gets the scoped board tools shown by `mesimon doctor --mcp`, so it
knows which ticket it is on, can read the ticket's description and notes, write notes of its own,
move its own card, put one of your tags on it, and file a new ticket for work it found outside its
scope (the new card has no session; you decide what happens to it). The tags are yours: an agent
picks from the ones you made in the picker and cannot add, rename, recolour or delete one.
They arrive on the command line and are installed nowhere: no `.mcp.json`, no `~/.claude.json`,
no `settings.local.json`, no plugin. A session you start yourself never sees them, and your own
MCP servers still load alongside.

There is no tool, at any tier, to kill a session, delete a ticket, merge a branch, or read a
session, a transcript or a cost. Those commands are refused by the daemon, not merely absent from
the tool list. Starting an agent, and archiving or renaming another ticket, belong to the ticket
wearing [the crown](#the-crown-one-agent-runs-the-board) alone;
[promise 3](PROMISES.md#3-zero-prompt-injection) says how the crown works.

Claude's `Edit`/`Write` and Codex's structured `apply_patch` writes into `.mesimon/` and
mesimon's state directory are refused too — which is why a note, a markdown file under
`.mesimon/`, reaches an agent through a tool and not through `Write`; the tool stamps who wrote
it. Its shell is not: `sed -i` into those paths still
works, because mesimon does not hook shell commands.

`mesimon doctor --mcp` prints all of it — the exact flag, every tool description, the token cost,
and what mesimon deliberately does not send.

## Stopping everything

Agent sessions cost real tokens and keep running after the board closes. To stop all of them for a
repo:

```sh
mesimon doctor                     # shows the daemon and the project key
pkill -f "mesimon daemon"          # stops the daemon (sessions survive this)
tmux -S /tmp/mesimon-$(id -u)/<project key>/tmux.sock kill-server   # stops the agents
```

Inside the board, `x` sleeps one ticket's sessions and `X` sleeps the finished agents in DONE,
which is the gentler version.

## Investigating agent state

`mesimon state explain [session-prefix] --repo <repo>` shows the current state,
confidence, recent inference decisions and board-movement decisions. The
compatibility manifests it ran against are `docs/claude-compatibility.json` and
`docs/codex-compatibility.json`.
