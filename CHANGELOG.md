# Changelog

Mesimon is in alpha. Releases may change state-file formats; when a format
changes, the old file is preserved.

These notes describe each version at the time of release. They are available
in the Esc menu under `Release notes` and on GitHub.

## v0.1.0-alpha.19 — 2026-09-10

### Added

- **Run Codex sessions alongside Claude Code.** Choose the provider for new
  sessions in Settings → Agents. Existing and sleeping sessions retain their
  original provider, and each ticket has one live agent seat. Codex uses its
  native CLI authentication and runtime; compatibility was measured against
  codex-cli 0.153.4.
- **Keep your machine awake while an agent works.** Enable the option in
  Settings → Behaviour; it is off by default and also works while attached.
  A coffee indicator beside the ticket count shows an active hold, and a
  dimmed moon shows idle. Select the indicator and press Enter to open its
  setting. Waiting for user action or closing the board releases the hold.
- **Duplicate a board ticket with `yy`.**

### Changed

- **Settings are grouped by purpose.** Application and column settings have
  clearer sections, labels, and contextual hints, with independent sleep and
  archive offers.
- **Jump half a page with `{` and `}`.** Diff navigation also supports
  `Ctrl+]` to return to the previous screen.
- **Board tags are easier to see.** Overflow counts replace faded partial
  cards at the edges of a column.

### Fixed

- **Agent status and recovery follow more native session events.** Claude
  compaction, wakeups, and waiting states have improved detection; Codex
  sessions support recovery with conservative handling of uncertain state.
- **Upgrade older running daemons with explicit handover.** Protocol-1
  daemons can be handed over when you request an upgrade.
- **Navigation keeps focus and movement predictable.** Held-key navigation,
  header focus, cursor styling, and diff paging are corrected. Moving a ticket
  to an adjacent column requires matching double presses and retains focus
  in the source column.

## v0.1.0-alpha.18 — 2026-09-08

### Added

- **Get desktop notifications and sounds when an agent needs you or finishes.**
  Enable them in Esc → Settings → Notifications; they are off by default and
  work while the board is open, including while attached to an agent. Choose
  separate sounds, control notifications while focused, and turn off quoting
  agent replies. Supported macOS notification helpers can bring you back to
  the board when you click a banner.
- **Agents can mark a ticket as needing your decision after a turn ends.**
  The `raise_hand` tool leaves a visible reason on the ticket and pauses its
  automatic merge until you acknowledge it or answer the agent.
- **Start Claude from the ticket's `+ claude session` row.** Select it and
  press Enter. Its preview shows the workspace and starting behavior; sessions
  without readable replies now explain whether they are starting, waiting,
  working, or missing a conversation to resume.
- **Choose a ticket's workspace with `Shift+Tab` on the board or ticket page.**
  Switch between the shared checkout and an individual worktree while no
  worktree or live session locks the choice. Parked sessions no longer prevent
  changing the workspace for the next new session.
- **Sort a column by tags.** Select `by tag` in the column's `Sort now` row.
  The order follows the tag picker, and later card moves remain manual.

### Changed

- **`Ctrl+K` includes links from the agent's latest reply.** The board and
  ticket page list the ticket's notes first, followed by its latest agent
  reply, including replies from parked sessions.
- **The board's top row can receive keyboard focus.** Press `k` from a column
  header to reach it and select its available actions.
- **Ticket shell shortcuts are disabled by default.** Set
  `MESIMON_TICKET_SHELLS=1` to restore them. The persistent terminal opened with
  `!` remains available.
- **The installer, session previews, and supported notifications show the
  Mesimon mascot.** macOS may ask for notification permission for Mesimon's
  separate notification identity.

### Fixed

- **Blocked automatic merges explain why the checkout refused them.** The
  merge train retries when the checkout becomes clean.
- **Answering an agent clears its needs-you marker even without a typed
  prompt.** This includes continuing through an in-pane permission response.
- **Sleeping a session no longer shows the misleading `resume may lose
  context` warning.** A missing conversation is explained in the session preview.
- **Terminal history scrolls one line per mouse-wheel event.** Applications
  that handle their own mouse input keep receiving it.

## v0.1.0-alpha.17 — 2026-09-06

### Added

- **Open a persistent terminal with `!`.** From the board, it opens in the
  project's checkout. From a ticket page or branch diff, it opens in the ticket's
  attached worktree. Use `Ctrl+]` or `Ctrl+5` to return to Mesimon; commands keep
  running while detached, even if the board or daemon closes. `exit` closes the
  shell. This replaces the diff viewer's foreground shell.
- **Choose a default column for agent-created tickets.** Set `Default column`
  in Settings to choose where `create_ticket` puts tickets when no column is
  specified. The choice follows column renames; deleting that column resets it
  to the first column.

### Changed

- **Archiving a merged ticket removes its worktree.** Mesimon also deletes the
  branch if Git permits it; a squash-merged branch may be kept. Unmerged work
  and worktrees with session panes are kept, and snoozing does not remove the
  worktree. Starting or waking an agent after restoring the ticket recreates
  its worktree.

### Fixed

- **Collapsed columns stay collapsed on launch and refresh.** Mesimon places
  the cursor in the nearest expanded column when needed. Moving into a collapsed
  column yourself still expands it.
- **Reloading after a Linux update works.** Pressing `U` after installing a new
  binary no longer fails with `No such file or directory`.

## v0.1.0-alpha.16 — 2026-09-06

### Added

- **Configure automation for each column.** Select its header and press Enter
  to set moves on agent start or finish, merge requirements, merge-train
  participation, and idle-session sleep offers. The same dialog sets the default
  workspace, collapsed state, Claude permission mode, agent tool access
  (`off`, `read`, `annotate`, or `full`), and whether new tickets start Claude.
- **Manage columns from their headers.** Use `r` to rename, `HJKL` to reorder,
  `d d` to delete an empty column, and `O` to add one. Renaming preserves the
  column's tickets. Empty columns remain selectable.
- **Place the tmux status bar at the top.** The setting applies immediately
  and is saved for future sessions.

### Changed

- **Mesimon recognizes branches merged through upstream pull requests.**
  This includes squash merges whose changes are already present upstream.
  When `origin/main` exists, fetch to update the status; fetching remains
  opt-in. Completed branches show as merged, can enter DONE, and are no longer
  offered unnecessary rebases.
- **Queued prompts follow board order.** Prompts waiting for a shared checkout
  run in column order, then from top to bottom. Moving a card changes its place
  in the queue.
- **Detach shortcuts use consistent labels** in the tmux status bar and footer.

### Fixed

- **Agents remain marked as working after a daemon restart during a tool call.**
  A long-running tool previously could leave the card incorrectly marked as idle.

## v0.1.0-alpha.15 — 2026-09-05

### Added

- **Open links from a ticket with `Ctrl+K`.** On the board or ticket page, the
  dialog lists URLs, ticket keys, and existing files referenced in the notes.
  Use `j`/`k` to select, Enter to open, or `c` to copy. `Ctrl+Shift+K` opens the
  first link directly. Text files open in `$VISUAL` or `$EDITOR`, including a
  referenced line number; URLs and other files use the system opener or
  `MESIMON_OPEN`.
- **Show tags and the latest agent reply on every card with `P`.** Press `P`
  again to show them only on the selected card. Pressing `p` to hide the selected
  card's preview hides all previews. The ticket page's previous `P` shortcut for
  keeping a session awake has been removed.
- **Tickets show which agent created them.** The ticket page includes the
  author and source ticket for tickets filed through `create_ticket`.
- **Daemon logs include startup, shutdown, and slow operations.** Read
  `<state>/daemon.log` to investigate a stalled board. `mesimon doctor -v`
  reports the last shutdown reason.

### Fixed

- **Board actions no longer wait for periodic worktree checks.** In the measured
  test with a dozen worktrees, the worst input stall fell from 481 ms to 57 ms.
- **Completed turns no longer appear interrupted while a Stop hook is delayed.**
  Mesimon detects the completed reply and moves the ticket to REVIEW without
  waiting for the hook.
- **Taken-over external sessions can use agent tools.** Imported sessions that
  Mesimon has relaunched can use tools such as `get_ticket` and `move_ticket`,
  subject to their configured access level.
- **Reloading shows progress.** Pressing `U` displays `mesimon: reloading…`,
  followed by a connection message if reconnecting takes more than a second.
- **The ticket page header includes the ticket key**, for example `TICKET (T-12)`.
- **Empty queued prompts show `enter drops` in the input field.** The cancellation
  hint no longer gets cut off in the delivery row on narrow cards.

## v0.1.0-alpha.14 — 2026-09-05

### Changed

- **Agent brief is an opt-in setting for Mesimon-launched Claude sessions.**
  It adds a system-prompt instruction to read the ticket through `get_ticket`,
  replacing alpha.13's offer to edit `CLAUDE.md`. It is off by default. The
  Settings dialog previews the exact text before enabling it, offers copy and
  dismiss options, and writes nothing to repository files. Agent tools must be
  enabled; sleep and wake an existing session to apply a change.
- **Shift+Enter includes the ticket description in the first prompt.** Creating
  and starting a ticket this way submits both the title and description. Plain
  Enter still enters only the title for you to edit. The ticket page shows
  `description unread` if an agent has taken a turn without reading the description.

### Added

- **View changes across a directory of repositories.** The header counts
  repositories and changed files in the root and one directory level below it.
  Press `v` for a combined diff list labeled by repository. A repository root
  keeps its own branch status; a plain folder with one repository shows that
  repository's branch. Per-ticket worktrees are not supported for multi-repository
  workspaces, and the composer no longer offers them there.
- **Open the checkout diff from the board with `v`.** It includes staged,
  unstaged, and untracked files. From a worktree ticket page, `v` still opens
  that ticket's branch diff. Press `q` to return.
- **Exclude a ticket from automatic merging with `t`.** On the board or ticket
  page, this disables the ticket's automatic merge and rebase requests. Manual
  `m` still works; press `t` again to rejoin the merge train. Cards label queued
  automatic merges as `auto-merge` and show their order.
- **Disable agent tools per repository.** Turn off `Agent tools` in Settings
  to launch sessions without Mesimon's MCP tools. It is on by default. Sleep
  and wake existing sessions to apply the change.

### Fixed

- **Agent turns triggered by Claude Code's `!` commands show as working.**
  Mesimon detects the first tool call even when no prompt-submission event occurs.
- **Long Settings descriptions scroll when selected** so their full text can be read.
- **Merge-train status appears in Settings and on affected cards.** The board
  header no longer shows a label that suggests the train applies to every ticket.
- **The detach documentation includes `Ctrl+5`.** Use it on keyboard layouts
  where `Ctrl+]` sends Escape and interrupts the agent.

## v0.1.0-alpha.13 — 2026-09-04

### Changed

- **`X` sleeps agents in DONE; `z` is now snooze.** `x` still sleeps the selected
  sessions. Bulk sleep has moved from `z` to `X`.

### Added

- **Snooze a ticket with `z`.** Press `z` again to cycle through one hour, four
  hours, tomorrow at 09:00, or the next week's start at 09:00. Enter confirms;
  Esc cancels. The ticket is archived until then and returns at the top of its
  column, marked as needing attention unless disabled in Settings. `a` or `u`
  restores it early. Snoozing sleeps an idle agent and is refused while it works.
- **Queue prompts until a shared checkout is idle.** In the board's prompt
  field, Shift+Tab switches between immediate and queued delivery. Queued prompts
  wait until no Claude sharing that checkout is mid-turn. Shift+Enter reopens a
  queued prompt; submitting an empty field cancels it. Sending a prompt directly
  to the agent also cancels the waiting prompt.
- **Enable automatic merging with `Merge train` in Settings.** Off by default,
  it fast-forwards finished REVIEW branches while all Claude sessions are idle.
  It can notify the merged ticket's agent and ask an idle agent on an outdated
  branch to rebase and test. Rebase requests are limited to one per main-branch
  update and six in two hours. The train runs only while the enabling board is open.
- **See Git status in the board header.** It shows the checkout branch, commits
  ahead or behind, and changed-file count. Fetch through `Fetch origin` in the
  Esc menu or set `MESIMON_GIT_FETCH=<minutes>` for periodic fetching.
- **Send a prompt to a sleeping Claude with Shift+Enter.** Mesimon wakes the
  session and delivers the prompt when it is ready.
- **New boards start with `BUG`, `FEATURE`, and `CHANGE` tags.** They are added
  once; existing tags are preserved and deleting the defaults does not recreate them.
- **Move cards with `HJKL`.** These shortcuts provide an alternative to
  `Alt+hjkl` for terminals that do not pass the Alt modifier through.
- **Choose the first day of the week in Settings.** Monday, Sunday, and Saturday
  are supported. The choice determines the next-week snooze date.

### Fixed

- **Reloading waits for the old daemon to shut down.** Slow shutdowns no longer
  leave sessions running without a connected board. If a daemon is unavailable
  at launch, the board displays the reason and connects when one becomes available.
- **Rapid consecutive prompts no longer leave a completed agent marked as working.**
- **A ticket's shell no longer blocks manual merging with `m`.**
- **Theme descriptions no longer repeat the dark/light category.**

## v0.1.0-alpha.12 — 2026-09-04

### Changed

- **`Ctrl+S` saves and closes the note editor in one press.** In the expanded
  ticket composer, it saves the description and returns to the title field.
- **`Ctrl+Shift+S` saves and sends to Claude.** In the composer, it creates the
  ticket and starts Claude on its title. In a note, it notifies a running Claude
  or starts one if absent. Wake a sleeping Claude with `c` first. This shortcut
  requires a terminal that distinguishes `Ctrl+Shift+S` from `Ctrl+S`.

### Added

- **Read release notes inside Mesimon.** Choose `Release notes` from the Esc
  menu. Use `j`/`k` or `{`/`}` to scroll, `n`/`N` to move between releases, and
  `q` to return. Notes are bundled with the binary and available offline.
- **Edit notes and descriptions in your own editor with `Ctrl+G`.** Mesimon uses
  `$VISUAL`, then `$EDITOR`, then `vi`. Returning saves an existing note or updates
  the draft description.
- **Cards distinguish unread and read agent replies.** `✔` marks an unread
  reply; selecting the card or opening the ticket page changes it to `✓`.
  Replies start unread when the board opens.
- **Solarized light is available in the theme picker.** Amber and green themes
  use white body text, and pending deletion is highlighted in red on every theme.
- **Ticket previews scroll smoothly with `{` and `}`.**

### Fixed

- **Interrupted Claude turns are detected in about two seconds.** Cards show
  `⊘ interrupted` even when Escape produces no transcript record.
- **Sending a prompt can move a ticket back to IN PROGRESS after a manual move.**
  Mesimon no longer rejects that move as conflicting with the earlier action.
- **Approved plans are saved as notes with Claude Code 2.1.259.** Mesimon supports
  the changed plan-response format as well as the earlier format.
- **Shift+Tab cycles tag colors backward and wraps.**
- **Editor and session hints are clearer.** The expanded composer places Shift+Tab
  beside the workspace choice; sleeping sessions no longer show a duplicate wake hint.

## v0.1.0-alpha.11 — 2026-09-03

### Changed

- **Shift+Enter inserts a newline in descriptions and notes.** It no longer
  saves and starts Claude from the editor. Use `Ctrl+S` to save; Shift+Enter on
  a board card still sends a prompt. `Ctrl+]` also closes the editor.
- **`Tab` on a card opens its description.** This replaces the shortcut for
  cycling through tickets needing attention. Shift+Tab in the editor changes
  the workspace while that choice is still unlocked.

### Added

- **Approved agent plans are saved as ticket notes.** Replanning updates the same
  note. If the ticket has no description, the plan also becomes its description.
- **Agents can apply existing tags.** `tag_ticket` adds or removes tags on the
  agent's ticket, and `create_ticket` accepts tags for new tickets. Agents cannot
  create tag definitions. Duplicate names require a tag group to identify them.

### Fixed

- **Notes survive deleting and undoing a ticket.**
- **Worktree names use complete words from the ticket title.** Branch and
  directory names no longer end in a word cut in half.
- **The ticket header groups its title, state, description, and tags.** Long
  branch names truncate within the available space, and the session list omits
  a redundant Claude launch hint.
- **Keyboard hints match their context.** The board labels Space as the ticket
  page shortcut; dialogs show their own keys, while the footer keeps menu and
  help shortcuts on the right.

## v0.1.0-alpha.10 — 2026-09-03

- **The expanded ticket composer opens as a dialog.** Press Tab from the title
  field to edit the description. The dialog shows the ticket's tags and target
  column and aligns with column boundaries so adjacent cards stay readable.
  Existing editor shortcuts are unchanged.

## v0.1.0-alpha.9 — 2026-09-03

### Added

- **Tickets support descriptions and notes stored as Markdown files.** The
  description appears on the ticket page and notes appear beside sessions.
  Use `n` to open a selected note, `N` to create one, and `Ctrl+S` to save.
  Pressing `Ctrl+S` again on a saved note notifies the ticket's agent.
- **Expand the ticket composer with Tab to write a description.** The editor
  shows the target column, workspace, and selected tags.
- **Shift+Enter starts Claude on an existing ticket with no agent.** It submits
  the ticket title without leaving the board. Sleeping agents still need to be
  woken with `c`.
- **Agents can create tickets with `create_ticket`.** New tickets go to the
  first column or a specified column, with no session started. The tool is
  available to sessions launched by Mesimon.

## v0.1.0-alpha.8 — 2026-09-02

### Fixed

- **Multiline pastes no longer create a ticket or trigger board shortcuts.**
  Text fields treat a paste as one input and replace newlines with spaces.
  Pasting when no text field is open does nothing.
- **Text fields enforce their size limits before submission.** Titles allow
  up to 2 KB. Oversized pastes are trimmed at a character boundary, with a
  message explaining the limit; typing past the limit has no effect.
- **Sleeping tickets use the same color-bar brightness as other unselected
  tickets.** The sleep symbol continues to identify their state.

## v0.1.0-alpha.7 — 2026-09-02

- **Automatic detection of theme changes is disabled after launch.** Periodic
  terminal color responses could be mistaken for keyboard input and open the
  rename field. Mesimon still detects light or dark mode at startup. After an
  OS appearance change, restart Mesimon or choose a theme from the Esc menu.
  `MESIMON_GROUND_WATCH=1` re-enables periodic detection.

## v0.1.0-alpha.6 — 2026-09-02

### Changed

- **Each ticket has one Claude session and one shell slot.** Pressing `c` on
  a sleeping Claude wakes it. The `C` shortcut for starting a second Claude has
  been removed, preventing competing agents from repeatedly moving the same ticket.

### Added

- **Choose from five themes in the Esc menu.** Blue, amber, and green join
  graphite and chalk. Moving through the picker previews each theme; Enter
  saves and Esc cancels. Mesimon remembers separate choices for light and dark
  terminals. `MESIMON_THEME` can override the choice, and `mesimon doctor` reports it.
- **Page through ticket previews with `{`/`}` or Page Up/Page Down.** Shell
  output follows the bottom until you scroll away, and resumes following when
  you scroll back down.

### Fixed

- **Late responses to the terminal color query are filtered before keyboard
  handling.** This addresses responses arriving after the initial detection timeout.
- **Agent previews show the user's prompt instead of background task notifications.**
- **The sleep symbol dims with the ticket's color bar.**

## v0.1.0-alpha.5 — 2026-09-02

### Added

- **Linux and Windows through WSL2 are supported.** Static Linux builds are
  available for x86_64 and aarch64 through the existing installer and update
  flow. Install tmux from your Linux package manager; 3.3 or newer is recommended.
- **Copying from agent panes works with Linux and WSL clipboards.** Mesimon
  uses `clip.exe` on WSL, `wl-copy` on Wayland, or `xclip` on X11.
- **Recall earlier prompts with Up and Down in the prompt field.** History
  holds the last 50 prompts for the current board run and preserves your draft
  while you browse.
- **`mesimon doctor` reports WSL-specific setup issues.** It warns about
  repositories on Windows-mounted drives under `/mnt` and about missing `curl`,
  which is required for update checks.

### Changed

- **Update checks run every 30 minutes.**

### Fixed

- **Sessions retain the correct status with tmux versions older than 3.6.**
  A parsing incompatibility could make live sessions appear crashed after a
  restart and prevent interrupted turns from appearing idle.
- **Long tool-input transfers are less likely to be mistaken for interruptions.**
  The inactivity threshold increased from eight seconds to one minute.

## v0.1.0-alpha.4 — 2026-09-01

### Added

- **Move a selected card one step with Alt plus `hjkl` or an arrow key.** The
  cursor follows the card. The existing `<` and `>` move controls remain
  available for terminals that do not pass the Alt modifier through.
- **Repeat the last column move with `.`.** The selected card moves to the
  previous destination column while the cursor stays in place for the next card.
  Renaming or deleting the destination disables the repeat action.
- **Cards show a progress indicator while agents or worktrees start.**
- **Each tag group supports ten tags**, increased from five. Wide tag rows
  scroll to keep the selected tag visible on narrow terminals.

## v0.1.0-alpha.3 — 2026-09-01

### Changed

- **New and woken sessions receive your current shell environment.** Mesimon
  reads your login-shell startup files and passes the environment to sessions,
  excluding internal tmux, terminal, and Mesimon variables. When startup files
  change, the board offers a manual refresh. Existing running sessions retain
  their environment; no shell configuration files are modified.
- **Exiting Claude normally puts the session to sleep.** After a clean exit
  with a resumable conversation, press `x` to wake it. The ticket retains its
  worktree. Crashes remain marked as crashes.

### Fixed

- **`/clear` no longer marks a running Claude session as ended.**
- **Sessions without a transcript can start fresh.** Resuming a session that
  ended before its first prompt starts a new conversation in the same session row.
- **`x` dismisses an ended session from the list.** Its conversation is preserved.
- **Cards can be reordered within their own column.** Open the move controls
  with `>`, return to the original column with `h`, and choose the new position.
- **The ticket preview has more space.** An unused documents placeholder has
  been removed, and the header lists tags before the branch name.
- **`Ctrl+T` displays the tag picker on the ticket page.** Repeated digit
  shortcuts also wrap within their tag group instead of stopping at its end.

## v0.1.0-alpha.2 — 2026-09-01

- **The macOS package includes tmux.** Installing tmux separately is no longer
  required. The bundled version, 3.6a, installs as `mesimon-tmux` and leaves your
  existing tmux installation and configuration unchanged. `MESIMON_TMUX_BIN`
  overrides the binary choice; `mesimon doctor` reports the selected binary.

## v0.1.0-alpha.1 — 2026-09-01

First public alpha release.

### Added

- **Diagnose setup problems with `mesimon doctor`.** It checks the environment,
  installation, tmux, Claude, Git, daemon version, and damaged state files.
  It prints suggested fixes without applying changes and redacts the home
  directory so the output can be shared in bug reports.
- **`mesimon --version` includes the commit and build date.**
- **Update by rerunning `install.sh`.** An open board offers `U` to reload
  after installation. Release packages include checksums.

### Fixed

- **Malformed state files no longer prevent startup.** Mesimon preserves them
  as `<name>.quarantine-<ms>` and loads defaults for the affected data. Files from
  a newer Mesimon version are left untouched. The board and `mesimon doctor`
  identify affected files.
- **Mesimon can recover worktree bindings after losing `worktrees.json`.**
  It reconstructs them from Git's worktree list and Mesimon ownership records,
  pausing worktree actions until recovery is verified.
- **Ticket numbering recovers from existing ticket directories.** Losing
  `columns.toml` no longer causes a new ticket to overwrite an existing ticket key.
- **Board-file writes are flushed before replacement** to reduce corruption
  if Mesimon stops during a write.
- **A newer board restarts an older Mesimon-managed daemon on the new binary.**
  Running agent panes are preserved. An older board leaves a newer daemon alone.
