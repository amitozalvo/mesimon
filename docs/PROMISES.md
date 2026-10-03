# The three promises

These are commitments, and a way to make mesimon break one is a security bug. The
[security policy](../SECURITY.md) says how to report it.

## 1. A strict write allowlist

On its own, mesimon writes only to `.mesimon/`, `$GIT_DIR/info/exclude`, the git worktrees and
`msmn/*` branches it created (and their git bookkeeping), its state dir under
`~/.local/state/mesimon/`, and its runtime dir at `/tmp/mesimon-<uid>/<project key>/` — the
sockets, provider observation snapshots/logs, the daemon lock, and the environment file every
pane is launched with. That last one is a copy of your login shell's environment, secrets and
all, so the runtime dir is 0700 and mesimon refuses it unless it owns it; those permissions are
also the only thing between another user on this machine and the agent tool socket, because
there is deliberately no token.

Two more, each only when you ask for it:

- **Remote-tracking refs and objects**, if you turn on periodic fetching
  (`MESIMON_GIT_FETCH=<minutes>`) or press `f` in the push / pull view (`v` on the board, then
  `tab`). That fetch never
  writes `FETCH_HEAD`, never runs `gc`, and never touches your branches.
- **mesimon's own binary**, and the `mesimon-tmux` beside it where one exists, if you take an
  update offer. The download is checked against its published checksum first and refused
  without one.

Never your shell rc, your git config, your `~/.claude/` or `~/.codex/` configuration, or your
tmux config. Native agents still own their conversations and native trust decisions.

Every path, and what mesimon keeps there, is in [What mesimon writes](#what-mesimon-writes).

## 2. No config mutation

`mesimon doctor` diagnoses and prints copy-pasteable fixes. It has no `--fix`.

## 3. Zero prompt injection

mesimon adds, removes and reorders exactly zero tokens of your conversation. It never prepends a
system prompt, never appends a reminder, never rewrites what you typed. When you ask it to start
an agent on a ticket and submit (Shift+Enter, or Start agent in Remote Control), what it submits
is the ticket's title followed by the ticket's description — both your own words, written for
that ticket, and nothing of mesimon's.

It does give the sessions it spawns scoped board tools — read and move its ticket, read and
write its notes, tag it from the tags you already made, and file a new ticket — so an agent
can see which ticket it is on and what it is about. One ticket per board can wear the
*crown* (`^o` on its card): its agent may then move, retitle, tag, annotate, set the
workspace of and start an agent on the other tickets, archive them where you turn on
*Settings › Agents › Crown archives tickets* (off by default), and put an idle agent it
started to sleep, each edit checked against the ticket as the agent last read it and lit on the card
as it happens. Starts are capped by a per-board budget (Settings → Agents, three by default),
a ticket the crown started can never itself be crowned, and an agent you started is yours
alone to sleep. Words the crown leaves for another ticket's agent wait on that card until you
send them, unless you turn on *Settings › Agents › Crown sends its asks* (off by default): then
words for an agent the crown started reach it once it is idle, and an agent you started still
waits for you. The crown also answers by default (*Settings › Agents › Crown answers
questions*, on; turn it off to keep every question and plan for yourself): it may answer a
question an agent it started stops on, typed into that agent's dialog, or accept a plan that
agent stops on, with one Enter on the plan dialog's first row (`♛ accepted plan`), each shown
on its card and in the activity feed. A plan it would change, a question about secrets, spend,
anything destructive or beyond the brief is raised to you, and every other stop, and every
question or plan from an agent you started, still waits for you. Only you can
crown a ticket; an agent that asks for it is told to ask you.
`mesimon doctor --mcp` prints the current tool registry verbatim.

And there is one line you can choose to add. The *agent brief* is off until you turn it on:
a seven-line paragraph in the system prompt of the agent sessions mesimon starts in this repo
— only those, never a session you started yourself — telling the agent to read its ticket
before it starts work. The dialog that offers it shows the exact text first, `mesimon doctor`
prints it, and *Settings › Agents › Agent brief* turns it off again. Together with the tool registry
it is everything mesimon adds to model input.

What the tools can and cannot do is in
[What your agents can see](USING.md#what-your-agents-can-see).

## What mesimon writes

| Path | What |
|---|---|
| `<repo>/.mesimon/` | Your board: columns, tickets, and durable content-import receipts/staging under `board/imports/`. Excluded via `$GIT_DIR/info/exclude`, never `.gitignore`. |
| `$GIT_DIR/info/exclude` | One line, so `.mesimon/` does not show up in `git status`. |
| `~/.local/state/mesimon/<project key>/` | Sessions, worktrees, hook settings, provider launch settings, normalized previews, logs, the token counts behind each ticket's estimated cost, and the private tmux server's conf. (Its socket is in the runtime dir below.) |
| `~/.local/state/mesimon/notifications/` | Notification mascot images and signed Mesimon copies of the installed macOS notification helper. |
| `~/.local/state/mesimon/team/` | Board sharing: this machine's identity on the relay (`device.toml`), and the roots of team boards you joined without a checkout, under `boards/`. Only after you sign in. |
| `~/.local/state/mesimon/update-check.json` | When the release check last answered, and what it heard. One per machine, not per repo. |
| `~/.local/state/mesimon/prefs.json` | Your theme picks, one for a dark terminal and one for a light one, and whether the board follows the OS's appearance. One per machine, not per repo. |
| `~/.local/state/mesimon/usage.json` | Your plan's quota as Claude Code and Codex last reported it: each window's percentage and reset time, and the plan's name. Shared by every board on the machine, written only while a board is open. |
| `~/.local/state/mesimon/usage.lock` | An empty lock file, so only one board's daemon reads the quota at a time. |
| `/tmp/mesimon-<uid>/<project key>/` | The daemon, hook and private-tmux sockets, the daemon lock, and the environment file panes are launched with. 0700, because that file holds your shell's environment. Gone on reboot. |
| Worktrees and `msmn/*` branches | Only ones it created, only for tickets you set to worktree mode. |
| mesimon's own binary | Replaced in place, only if you take an update offer, only after its published checksum verifies. |

Nothing else. If you ever find mesimon writing outside that list, that is a bug worth reporting
above all others.

The Teams relay is a separate server binary, in its own repository, with its own
database; nothing in this list is written by it, and nothing it stores is
readable to it.
