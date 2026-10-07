# Changelog

Mesimon is in beta. A release that changes a state-file format says so in its
notes, and the old file is preserved.

These notes describe each version at the time of release. They are available
in the Esc menu under `Release notes` and on GitHub.

## v0.1.0-beta.1 — 2026-10-06

### Added

- **mesimon is in beta.**
- **Remote Control is in every build.** Press Esc and open `Remote Control`;
  it no longer needs `MESIMON_MESOPHON=1` at startup.
- **The board header's `remote` mark opens Remote Control.** `k` above the
  top row reaches the header, `h` and `l` select the mark, and Enter opens the
  dialog. Esc from it goes back to the mark.
- **Remote Control can forget one board on a phone.** Open the board picker
  under the board's name, press the bin beside a board, then `Forget`. The
  other boards stay paired. A board whose computer still runs keeps listing
  the phone until you revoke it there.
- **Settings › Notifications has a `Delivered by` row.** `mesimon` posts the
  banner with the mascot; `your terminal` lets iTerm2, kitty, WezTerm or
  Ghostty post it instead, so a managed Mac has no app of mesimon's to ask
  about. In other terminals `your terminal` shows no banner; sounds play
  either way.
- **The Esc menu offers `Restart the private tmux server`** when macOS has
  cut the board's sessions off from the folder the repository is in. The
  advisory row says so, `mesimon doctor` prints a `private server` line with
  the same repair, and the restart parks every session and wakes it back.

### Changed

- **`c` copies the ticket's id; press it again for its title, and a third
  time for both.** It works on the board and on a ticket page, and no longer
  starts an agent: Enter on a ticket you just created starts one, and on a
  ticket page Enter on `+ agent session` starts one and Enter on a sleeping
  session wakes it.
- **The Esc menu's `Sharing` row is `Remote Control`,** and it opens Remote
  Control's dialog directly: `Sign in`, `Enable`, `Pair a browser` and your
  paired devices are its rows, with `Signed in as …` above them. The footer
  reads `REMOTE CONTROL` there, and `mesimon mesophon setup` names the same
  path.
- **On macOS the private tmux server now answers to macOS for itself.** The
  first agent you start after this update may ask once whether `mesimon-tmux`
  may access your Documents, Desktop or Downloads folder; allow it.
- **An open board asks the daemon for one snapshot per change, not three.**
  Notifications and keep-awake read the board the screen already has; they
  fetch their own only while you are inside an agent pane, a terminal or an
  editor.

### Fixed

- **Agents no longer die at launch with `Operation not permitted`** days
  after the terminal that first opened the board has quit, and agents in
  worktrees can commit again.

## v0.1.0-alpha.40 — 2026-10-05

### Added

- **Remote Control's New ticket sheet takes pictures in Details.** Paste a
  picture or use the picture bar, as in a note. The pictures are kept with the
  ticket once it is created.
- **Remote Control's Now page lists tickets created in the last hour under
  `Recently created`**, newest first, while no agent works on them.

### Changed

- **In Remote Control's Sent, a ticket that landed shows as its board card,**
  with its current column, tags and agent. An archived ticket reads
  `Archived`; one no longer on the board keeps the words it was sent with.

### Fixed

- **On a Claude Team or Enterprise account, the ticket page shows the agent's
  conversation again.** It read "it left no transcript", and the ticket's
  token usage was not counted.
- **A ticket moves on when its agent finishes a turn while mesimon restarts
  or updates.** It could stay in IN PROGRESS after the turn had ended.
- **On the phone, Back closes a ticket that opened by itself** (at launch,
  after a board switch or from a notification) instead of leaving Remote
  Control.

## v0.1.0-alpha.39 — 2026-10-05

### Added

- **Remote Control picks the agent tier when it starts an agent or sends a
  prompt.** The start sheet lists the board's tiers with the ticket's own
  selected, and the composer offers them beside Queue and Steer. A running
  agent switches tier when it next goes idle, and the prompt waits until then.
- **Remote Control shows whether a ticket has its own worktree, and can set
  its workspace.** The ticket page shows the branch; the card sheet's
  Workspace choice sets Shared checkout or Own worktree while the ticket can
  still change it, and says why once it cannot.
- **The board's header reads `remote` while Remote Control is on, and
  `remote offline` while the board cannot reach the relay.** Paired phones
  cannot see the board while it reads `remote offline`.

### Changed

- **Remote Control signs in with a license key.** `Sign in` (Esc → Sharing →
  Remote Control) opens a `LICENSE KEY` dialog with `Get a license key` under
  the field; once signed in, `Signed in as …` opens it again with `New license
  key` and `Sign out`. The `Access code` and `Enter a code` rows are gone.
- **Remote Control shows one `Reconnecting` line with a spinner while the
  board is out of reach**, in place of a banner that switched between
  connecting and disconnected every few seconds.
- **On the phone, a press on a ticket's title renames it;** the pencil is
  gone. In Sent, a landed ticket opens from its bubble, and the Open link is
  gone.

### Fixed

- **On a Claude Team or Enterprise account, cards follow the agent from its
  first prompt.** Claude Code there keeps its hook events from a person's
  plugins, so cards read `starting up` and `brief not sent` while the agent
  worked, and Shift+Enter sent the brief a second time. The plugin now reports
  those sessions from Claude Code's own events and still carries prompts,
  answers and the board's tools, one hook reports permission requests so the
  phone and the terminal can both answer them, a brief the agent already has
  is never sent again, and `mesimon doctor` says when this applies.
- **Remote Control reconnects by itself after the Mac sleeps.** The board
  could stay out of reach of every paired phone until mesimon restarted.
- **A restart while the merge train waits for a rebase keeps it waiting.** It
  had asked the next REVIEW ticket to rebase onto the same tip.
- **A card no longer stays on a question for fifteen minutes after the agent
  finished its turn.** This happened when the question never appeared in the
  agent's conversation.

## v0.1.0-alpha.38 — 2026-10-03

### Added

- **A `.mesimon-worktree-init.sh` at your repository's root runs in each new
  worktree before its agent starts**, so a worktree can begin with its build
  cache seeded or its dependencies installed. The agent starts anyway; the
  ticket page reads `init script running`, then `init failed ∙ exit 2` or
  `init timed out` (after ten minutes) beside the branch if it went wrong.
- **Remote Control reads the agent's conversation** on the ticket page: your
  prompts, the agent's replies and progress notes, and one line per tool
  call, formatted as the desk formats them. Scroll up for older pages; Raw in
  the panel's corner shows the screen as before.
- **Remote Control can add pictures to a note**, picked or pasted, and keeps
  them with the ticket as the desk does.
- **Remote Control marks the ticket that wears the crown** with `♛` before
  its title, and a ticket the crown changed in the last hour says what it did
  (`♛ moved · 2m`).
- **On the phone, a swipe across the Board steps to the next or previous
  column**, and a swipe dismisses a banner or a toast.

### Changed

- **Appearance opens on this board's settings**, and its Theme row picks one
  theme for dark and light terminals by default (Tab still picks each
  separately). The `follow the OS appearance` row is gone: two different themes
  are what makes the board follow the OS. Notifications is now its own row in
  Settings.
- **Remote Control's ticket page lists only the latest note past two;** All
  opens every note in a sheet. The description has no heading, and a press
  opens it whole.

### Fixed

- **A permission request stays answerable from the phone until it is
  answered**, including after the page slept and reconnected. It had lost its
  buttons after about a minute while the ticket still read needs-you.
- **A phone adding pictures can no longer make the desk's own picture saves
  fail.**

## v0.1.0-alpha.37 — 2026-10-03

### Added

- **The crown answers a worker's question and accepts its plan.** A question
  or a plan from an agent the crown started wakes the crown, which answers it,
  including a dialog of up to four questions and questions with several
  choices; the card reads `♛ answered` or `♛ accepted plan`. Questions about
  secrets, permissions, sign-in, anything destructive or beyond the ticket's
  brief are left to you, and so is every question from an agent you started.
  `Settings › Agents › Crown mode` set to `supervised` keeps every question
  and plan for you.
- **The crown merges a worker's branch where auto merge will not.** When auto
  merge is off, the ticket was taken off it with `t`, or its column does not
  merge, the crown merges the branch itself (the same fast-forward `m` makes)
  and the worker is told; the card reads `♛ merged`. The crown is told who
  merges each branch: auto merge, itself, or you. Under `supervised` it raises
  its hand and leaves the merge to you.
- **The crown's words can reach a working agent at once**, the way your own
  `now` and `immediately` sends do, instead of waiting for the agent's turn to
  end. Not while the agent is at a dialog.
- **Send a prompt `immediately` to a working Claude agent.** Shift+Tab on the
  ticket page's composer cycles `now`, `queued` and `immediately`.
  `immediately` uses Claude Code's own send-now: the agent reads your words
  before its running command ends, and the command keeps running in the
  background. Codex has no such send.
- **Remote Control answers a dialog of several questions, or a several-choice
  question, from the ticket's card.** Every question is drawn, and one Submit
  sends them all. A question that mixes ticks with typed words is still
  answered in the pane.
- **Tiers take a description** (`Settings › Agents › Tiers`, up to 300
  bytes): when to use the tier, in your words. The crown reads them to pick a
  tier for each ticket it files or starts and says which it chose in the
  brief. A ticket you started keeps your tier.
- **The crown can read a picture on another ticket.**

### Changed

- **On Claude Code 2.1.287 or newer, mesimon talks to Claude through a plugin
  instead of typing into its pane.** Prompts, briefs and answers from the
  phone or the crown reach Claude whole, as your own words, the moment they
  are sent; a prompt sent during a turn waits and runs as its own turn; a
  question's answer lands straight in the dialog. Nothing is installed in
  your Claude Code configuration: mesimon keeps the plugin in its own state
  directory and passes it to each session it starts. Older Claude Code and
  Codex work as before, and if Claude Code refuses the plugin, sessions start
  the old way on their own; `mesimon doctor` says which way sessions start.
  Still typed: slash commands, a plan's accept (Enter on its first row) and a
  question refused from the phone (Escape). Sessions started by an earlier
  build switch at their next sleep and wake.
- **A ticket's cost on the plugin comes from Claude Code's own count of each
  turn**, subagents included, and the quota line's 5-hour and weekly windows
  refresh at the end of every turn.
- **One `Crown mode` row replaces `Crown sends its asks` and `Crown answers
  questions`.** `Settings › Agents › Crown mode` is `autonomous` by default,
  on new and existing boards: the crown's asks reach the agents it started
  without your `^y`, and it answers their questions and accepts their plans.
  `supervised` keeps all of that for you. The crown's rows now sit under a
  `Crown` heading.
- **The crown archives tickets only if you allow it.** `Settings › Agents ›
  Crown archives tickets` is off by default; while off, the crown moves a
  finished ticket to DONE and its merged worktree stays until you archive it.
- **The crown chooses each ticket's workspace on purpose**, a worktree or the
  shared checkout, and is refused the shared checkout while another ticket's
  agent holds it. It can also wake an agent it parked (`♛ woken`).
- **The crown is woken more precisely:** when a worker finishes a turn with
  nothing to merge; after a merge, only once the worker has read the merged
  notice; and after a daemon restart, for what happened while the daemon was
  down (the line says `after a restart`).
- **A worker idle with background tasks is left to its work.** Your words and
  the crown's still reach it, the card reads `idle ∙ 3 tasks ∙ 2h`, and after
  30 minutes the crown is told once. Nothing merges, parks or ends that
  worker by time.
- **Pressing `m` on a ticket whose agent the crown started tells the agent in
  the same step**, where `Auto merge tells the agent after a merge` is on:
  the dialog closes with `merged ∙ its agent was told` and offers no second
  press. A ticket whose agent you started keeps the two presses.
- **New machine defaults: the terminal tab, notifications and following the
  OS appearance are on.** The tab title reads `mesimon ∙ <project>`; iTerm2
  also colours the whole tab while a ticket needs you and shows the theme
  colour, the subtitle and the mascot icon. The progress ring stays off. A
  machine that already saved its settings keeps them; each row is in
  `Settings › Terminal`, `Notifications` and `Appearance`.
- **Auto merge is on by default** on a new install. `Settings › Behaviour ›
  Auto merge` turns it off; a machine that already saved it off keeps it off.
- **The board-wide `Sleep idle agents` row is removed.** A column's own `Agent
  behaviour › Sleep idle agents` is now the only idle timer.
- **The ticket page carries `♛` before its title** when it wears the crown,
  and no longer explains the crown in its rows or its footer.
- **The ticket page shows its cost only while the cards show cost** (`$`).
- **Remote Control's Now page lists a stopped agent for an hour**, under
  "Recently idle".

### Fixed

- **A new agent's brief is sent only once Claude's input box is on screen.**
  Several agents started at once could get a garbled or unsent brief. If the
  box does not appear within 30 s, the card reads `brief not sent`;
  Shift+Enter on the ticket sends it again.
- **A blank Enter on a ticket whose agent never took a prompt sends the
  brief.** The ask field reads `send the brief`. A blank Enter anywhere else
  says `nothing to send` instead of staying silent.
- **Words queued for an agent stopped on a question are held**, and the card
  says what holds them. `^y` no longer sends them into the question.
- **A phone's question answer reads "Answered." only once Claude confirms
  it**; otherwise it says the keys were sent but not confirmed, and keeps its
  buttons for a retry. Wrapped option labels are read correctly.
- **A queued phone prompt no longer blocks answering that agent's question.**
  Steer and Send now are refused while the agent waits on a dialog, and the
  queued row says who queued it and what holds it.
- **A question asked alongside other tool calls stays answerable** until its
  own call ends.
- **The transcript preview shows the agent's newest words**, including the
  progress notes a Claude 5 model writes between tool calls; it ran one reply
  behind the pane. Private thinking is never shown.
- **A raised hand comes down when the question behind it is answered in the
  pane.** A hand raised with no dialog behind it still stays until you open
  the ticket or send the next prompt.
- **macOS banners wear the mascot on a Mac without terminal-notifier.** A Mac
  that later installs terminal-notifier lists a second "Mesimon" under
  Notifications settings.
- **The write guard also refuses `NotebookEdit`** under `.mesimon` and the
  state directory.
- **Settings' "this board" scope (`b`) no longer carries into Terminal and
  Usage.**

## v0.1.0-alpha.36 — 2026-10-01

### Added

- **The board shows your plan's quota when a window nears its limit.** The
  row above the keys reads, for example, `claude 5h 86% resets 17:50`, using
  the numbers Claude Code's `/usage` and Codex's `/status` report; it stays
  grey and is silent while every window is calm. `Esc › Usage` shows every
  window, its reset time, the reading's age and why a provider has none; `r`
  reads again. `Settings › Usage` chooses near a limit (default), every
  window, each provider's headline, or nothing, plus which windows, when to
  name a reset and which providers to read. For Claude the board runs a
  short `claude -p` that sends no prompt, spends no tokens and saves no
  session. Readings happen only while a board is open, at most once a minute
  after a turn ends or a limit is hit, otherwise every 15 minutes, and are
  shared by every board in `~/.local/state/mesimon/usage.json`.
  `mesimon doctor` prints the last reading.
- **Each ticket shows an estimate of what its agents cost at API prices.**
  The ticket page's facts line reads `∙ $4.20 at API prices`; subagents
  count toward their ticket. A plan subscriber does not pay this amount; it
  is what the same tokens would cost on the API. `$` on the board switches
  every card's corner between its age and its cost (also `Settings › Usage`).
  `Esc › Usage` lists the board's last 24 hours, 7 days and 30 days and its
  costliest tickets; Enter opens one. Codex tickets show tokens instead of a
  price. Counting starts with this version: an older ticket counts only what
  its current sessions' transcripts hold.
- **Tiers can be reordered in Settings.** In the tiers list, `J`/`K` (or
  `Alt+Up`/`Alt+Down`) move the tier under the cursor. A board's own tiers
  are saved in the board; your machine's tiers in `tiers.toml`.

### Changed

- **`Ctrl+N` cycles only the tiers you made, in list order.** Once you have
  tiers, the built-in `claude` and `codex` no longer appear between them; the
  default stays in the ring when it is a built-in, so you can return to it.
  With no tiers of your own, the ring is still `claude` and `codex`. The
  open card names the default tier for a second after `Ctrl+N` lands on it;
  any other tier is named whenever the card is open.

### Fixed

- **Archiving many tickets no longer freezes the board.** Removing their
  worktrees took 4 to 13 seconds and could time out the board's requests; it
  now runs in the background.

## v0.1.0-alpha.35 — 2026-10-01

### Added

- **A column can put its idle agents to sleep.** In a column's settings,
  `Agent behaviour › Sleep idle agents` cycles off, 1, 5, 15 and 60 minutes
  (off by default). A Claude agent on a ticket in that column is put to sleep
  once it has sat idle at its prompt for that long, a woken agent that was
  never prompted included; its conversation is kept and `c` wakes it. An agent
  doing background work or watching a monitor is left alone, and so is one
  whose pane you are typing in. The board-wide timer in `Settings › Agents`
  still applies; whichever timer is due first wins. The check runs every ten
  seconds.
- **The crown can sleep the agents it started, and a sleeping agent frees its
  seat.** A new `sleep_agent` tool, for the crown only, parks an idle agent
  the crown started (conversation kept, `c` wakes it). An agent a person
  started is refused. The crown's start budget now counts only the agents it
  started that are awake, so a parked worker makes room for another start
  without an archive. The crown is also told, on its receipts and on its own
  ticket, that the board wakes it when a worker delivers, merges, answers or
  raises its hand, so it has no reason to poll.
- **The crown can deliver its own asks, if you let it.** `Settings › Agents ›
  Crown sends its asks` (off by default, per board) lets the words the crown
  sends with `ask_agent` reach an agent the crown started as soon as that
  agent is idle, without your `Ctrl+Y`. An ask to an agent a person started
  still waits for `Ctrl+Y`. Words for a sleeping agent wake it and need a free
  seat in the budget. Turning the setting off, or uncrowning, holds whatever
  has not been sent. The card reads `♛ sent` at delivery and the feed logs
  `ask_agent_sent`. With this on, one agent's words reach another agent's
  turn with no person between them.
- **The crown's actions play as lightning.** When the crown moves, starts,
  parks, archives, files or asks on a ticket, a bolt runs from the crown's
  card to that ticket and the title lights for two seconds with the word for
  what was done. A moved card leaves a faded copy where it was while the bolt
  plays, and a worker's delivery or merge strikes the crown's card in return.
  `Settings › Appearance › Crown's actions: lightning / still` turns the
  motion off, per machine. The mono theme draws no bolt.
- **A held ask says it waits on you.** On the cursor card, an ask that waits
  for you reads `T-544 asks ∙ you send` or `agent asked ∙ you send`, with
  `^y send ∙ ^u take back` on the row under it. The ticket page's row reads
  the same.
- **Remote Control renames, moves and tags a ticket.** On the ticket page,
  press the title to rename it (Enter saves, Escape puts it back) and press
  the line under it to pick a column and tags. On a desktop's Board a card
  drags to another column or to another slot in its own. A move obeys the
  DONE gate, and a tag must be one the board already has. Needs a relay
  updated to this release; the hosted relay is.
- **Remote Control reads and edits a ticket's notes.** The ticket page shows
  the description under the title and the other notes as rows. A note opens
  in place, with Previous and Next; Edit opens a sheet, and a phone can add,
  edit and delete notes, never the description. A saved note offers
  `Tell claude` while the agent is awake. A note changed at the terminal
  since you opened it is not overwritten: the page offers `Keep theirs` or
  `Save mine`. Notes you opened stay readable while your terminal is away,
  and an edit made then waits at the relay and lands once. Needs a relay
  updated to this release.

### Changed

- **The `?` overlay lays its key groups side by side.** Each column is as
  wide as its widest line, so a 120-column board shows the overlay in 14 rows
  where it took 20. The `> <` row now reads `move card, stay on original
  column`.
- **A pending archive fades the card.** After the first `a`, the card
  alternates between faded and normal until the second `a` or a cancelling
  key; the ticket page's title row fades the same way. The mono theme holds
  still.
- **A woken agent that has not run yet wears the idle ring** (`◦`) on its
  card, where before it looked like a ticket with no agent.
- **Remote Control's Now and Board draw the same card.** Now's cards carry
  tags and the agent's line as the Board's do, and a needs-you card says
  `needs approval`, `has a plan`, `has a question` or `needs you` on both.
- **Remote Control's settings open in a dialog** from a gear at the sidebar's
  foot; the connection hops are inside it, and the connection pill opens it
  on a phone. The Sent page's tick legend is gone (each tick keeps its word
  as a title), and `Started from this browser` sits on the ticket page's tag
  line.

### Fixed

- **The crown is not woken for a delivery the merge train will land.** It
  hears once, at the merge. If the train gives the ticket up (a rebase that
  left it behind, a refused merge, a disarmed train, the ticket moved off the
  train's columns), the held delivery wakes the crown then. A worker on the
  shared checkout wakes the crown as before.
- **The header's memory chip no longer lands one column to the right** in
  iTerm2, where it could read `0190GiB` for 0.9 GiB after the `☕️` glyph.

## v0.1.0-alpha.34 — 2026-10-01

### Added

- **Nine more themes.** The theme picker adds void, nord, mocha (Catppuccin
  Mocha), tokyo (Tokyo Night), rose (Rosé Pine), gruvbox, ice (dark) and
  latte (Catppuccin Latte), gruvbox-light (light), for fifteen in all. The
  existing six are unchanged.
- **iTerm2's tab can take the theme's colour.** Turn it on in Settings ›
  Terminal › `Tab colour from the theme` (off by default, per machine). The
  tab is painted in the colour of the board's hint line and repainted when
  the theme changes. `Tab colour when needs you` still takes the tab while a
  ticket needs you, then hands it back.
- **The crown is woken when a worker it started is merged.** Merging that
  worker's worktree branch with `m`, the merge train or your own
  `git merge` pastes one line into the crown's session. A merge the crown
  was already told of is not repeated. Workers on the shared checkout have
  no branch and send no merge wake.

## v0.1.0-alpha.33 — 2026-09-30

### Added

- **Teams reaches the hosted relay on port 443.** The relay field is
  prefilled with `relay.mesimon.dev:443`, the port office networks let
  out, and the browser and the wire share it. A Mac that signed in before
  this release keeps using port 8443, which stays open until October 2027;
  sign out and in again to move to 443. A self-hosted relay keeps its own
  port and pin.
- **Access is renewed before the phone is refused.** Inside the last day of
  a hosted-relay grant, mesimon redeems its kept code by itself, so a Mac
  that only uses Remote Control is not refused once a billing period. If
  the relay refuses the phone's mail for a lapsed grant, mesimon renews
  once on that refusal too. A cancelled subscription is not retried after
  the relay says no. Needs a relay updated to this release; an older relay
  keeps the previous behaviour.

### Fixed

- **An archived ticket's sleeping agent no longer holds a crown seat.** The
  crown's `start_agent` refused with "3 of 3 crown-started agents are live"
  after all three tickets had been archived. A seat frees when its agent
  exits or its ticket is archived; restoring the ticket counts it again.

## v0.1.0-alpha.32 — 2026-09-30

### Added

- **The hosted relay needs an access code to sign in.** The sharing dialog
  (Esc → Sharing) has an `Access code` row beside the relay and the display
  name. Enter the code you were given, then `Sign in`. A relay that gates
  registration refuses a sign-in without one and says so under `Sign in`;
  a self-hosted relay ignores an empty field. A Mac that signed in before
  this release keeps its access and needs no code.
- **`Enter a code` under your name takes a later code**, for a renewal or a
  code given to a Mac that signed in earlier. When a Mac's access runs out
  the dialog's title reads `LAPSED`: every board it belongs to still reads
  and syncs in, edits stay on this Mac until a code is entered, a paired
  phone keeps its live view, and a ticket filed from the phone while the
  Mac is away is refused.
- **Remote Control's Start button asks for the first prompt.** Blank starts
  the agent on the ticket's title and description; on a sleeping agent the
  button reads `Wake agent` and wakes it, with the words if any. The
  button is on the ticket page only, no longer on Board cards.
- **The sidebar's board name is a board picker**: press it to switch between
  paired boards or pair another. The `Paired boards` section is gone.

### Changed

- **The relay field is prefilled with `relay.mesimon.dev`**, the hosted
  relay, which needs no pin. A self-hosted relay replaces it with its own
  address, a space and the pin it printed; `Ctrl+U` clears the field.
- **Signing out asks first.** Enter on `Signed in as …` shows `Sign out?`;
  a second Enter signs out, and any other key cancels.
- **The sharing dialog is shorter.** Signed in, the identity is one row; the
  relay and name fields show only while signed out. `Notes: included / kept
  here` is gone and notes always sync (a board already shared without notes
  keeps syncing that way). `Browser access` moved into the Remote Control
  dialog's title (`CONNECTED`, `DISCONNECTED` or `OFF`), and the open board
  no longer repeats under `OTHER BOARDS`. Remote Control is the first row
  under `THIS BOARD`, shown only while signed in.
- **Remote Control's ticket page is shorter**: the agent card under the title
  is gone (the shin beside the title carries the state), and the tags,
  column and key share one line. `+ Add to <column>` shows on hover or
  focus. On a desktop the ticket opens over the board and a press outside
  closes it.
- **Signing in to a relay from an older release**: a relay updated to this
  release answers three new refusals (code required, code invalid, access
  lapsed) that mesimon before alpha.32 reads as `invalid request`. Update
  to see the actual reason.

## v0.1.0-alpha.31 — 2026-09-29

### Added

- **Remote Control (Mesophon) can file a ticket from a paired browser.**
  `New ticket` takes a title, optional details, a column and the board's own
  tags. A filed ticket starts nothing by itself. Each ticket shows a clock
  while it is sealed in the browser, one tick once the relay holds it, and
  two ticks with its key once your terminal has filed it.
- **A ticket filed while your terminal is away waits at the relay** and is
  filed once, when the terminal is back. Until then the browser can unsend
  or edit it. The ticks turn teal when you open the ticket in mesimon or an
  agent starts on it.
- **A paired browser can start an agent on a ticket that has none**, with
  `Start agent` on the ticket page or a Board card. It starts what
  `Shift+Enter` starts, with the ticket's title and description as the
  first prompt. It is refused when the ticket already has an agent, a
  start is already on its way, or the ticket came from an import or a
  teammate.
- **Remote Control's dialog shows the pairing code as a QR code**, and
  scanning it fills the code in on the phone. The page can be added to the
  home screen and opens from a kept copy, with the remembered board, when
  the network is out.
- **Mesophon's Board shows each ticket's tags and each agent's current step
  and latest reply line.**

### Changed

- **Mesophon's output pane is drawn at the terminal pane's width**, and
  tags use the same coloured chips as the TUI. The sidebar folds to a rail
  on wide screens.
- **Remote Control's hosted address is `remote.mesimon.dev`** and the
  install line is `https://mesimon.dev/install.sh`.
- **The column auto-run setting is removed.**

### Fixed

- **An idle board no longer writes to the terminal**, so a background
  iTerm2 tab stops showing activity.

## v0.1.0-alpha.30 — 2026-09-29

### Added

- **A new Settings › Terminal section lets the terminal tab show the
  board's state.** Every row is off by default and set per machine.
  mesimon puts the terminal's own title and tab settings back when you quit,
  suspend with `Ctrl+Z` or reload with `U`.
  - `Tab title`: the project name, or `mesimon ∙ <project name>`. Two
    follow-up rows add a needs-you count (`2 need you ∙ <board>`) and show
    the open ticket's key and title while you are in its session.
  - `Tab progress ring`: spins while an agent works and turns red while
    one needs you, in terminals that support OSC 9;4.
  - `Tab colour when needs you` (iTerm2): colours the whole tab, or only
    the tab's dot, in the theme's attention colour.
  - `Tab subtitle` (iTerm2): counts the tickets that need you and the
    agents that are working.
  - `Tab icon` (iTerm2): shows the mesimon mascot, with an amber badge
    while any ticket needs you. Only the running session's copy of the
    profile changes; the saved iTerm2 profile does not.
- **The tab dot, subtitle and icon need the iTerm2 3.7 beta.** On older
  iTerm2 versions those rows say so and do nothing; the whole-tab colour
  works on iTerm2 3.6. The iTerm2 rows do nothing in other terminals or
  inside another tmux.
- **Notifications › Dock bounce when an agent needs you** (iTerm2) bounces
  the Dock icon once. Off by default; a board can override it.
- **On kitty, notifications use kitty's own notification escape** when
  `terminal-notifier` is not installed, and clicking one brings the kitty
  window forward. Not inside another tmux.
- **`mesimon doctor --verbose` has a `terminal` line** naming the
  terminal, its version and the tab settings in effect.

### Changed

- **Each board frame is drawn as one synchronized update**, so terminals
  that support it no longer show a half-drawn frame.

## v0.1.0-alpha.29 — 2026-09-28

### Added

- **The board can follow the OS light/dark appearance.** Turn it on in
  Settings › Appearance › `Follow the OS appearance` (off by default, per
  machine or per board). mesimon asks the OS, not the terminal, so no reply
  can be typed into the board. `mesimon doctor` says what the OS would pick.
- **One theme picker sets the dark theme, the light theme, or both.** It
  opens on the state the board is in; Tab moves to the other state, then to
  both, and Enter saves for the state shown.
- **Markdown tables are laid out in columns** in descriptions, notes,
  replies and the release notes, with the header in bold and each cell's
  markdown rendered. A table too wide for the page wraps its widest
  columns; a very narrow page shows each row as `header: value` lines.
- **More markdown renders:** task lists show `[ ]` and `[✓]`, quotes can
  hold lists, code and nested quotes, `***text***` is bold italic, and two
  trailing spaces, a trailing `\` or `<br>` end a line. A note last edited
  by a person keeps its line breaks.

### Fixed

- **A command the agent runs in the background keeps the card working.**
  Previously a subagent finishing could drop the card to idle while the
  command was still running.
- **nvim's snacks picker no longer receives stray text inside a mesimon
  pane under iTerm2.** Shift+Enter still reaches Claude Code and Codex.
- **The status line of a terminal opened with `!` from the board no longer
  shows an empty ticket segment.**

## v0.1.0-alpha.28 — 2026-09-28

### Added

- **mesimon is open source.** The code is public at
  https://github.com/amitozalvo/mesimon under Apache-2.0, with a `SECURITY.md`
  for private vulnerability reports. Binaries still install from
  `mesimon-releases`; nothing changes for an existing install.
- **Install with Homebrew: `brew install amitozalvo/tap/mesimon`.** It installs
  the same published binaries as the curl installer; on macOS the bundled
  `mesimon-tmux` is installed beside it, on Linux the distro's tmux is used.
  A Homebrew-installed mesimon does not replace its own binary on an update
  offer: the Esc menu row reads `Upgrade to <tag> with brew` and copies
  `brew upgrade mesimon`, and `mesimon doctor` says `installed by Homebrew`.
- **A column can carry a one-line description, and agents read it.** Set it
  in the column's settings dialog (the first row, up to 300 bytes).
  `list_board` and `get_ticket` return the descriptions, so an agent filing a
  ticket can tell BACKLOG from TODO by your words instead of guessing.
- **Shift+Enter on the ticket page asks the ticket's agent.** The same ask
  field the board opens on a card appears under the page's content, and
  Enter sends it without leaving the page.
- **The row above the footer names the card under the cursor** —
  `T-12 Title ∙ created 2d ago by T-9` — whenever no undo or notice is
  showing. A ticket an agent filed reads `by T-9`, the ticket that agent was
  working on, instead of `by agent on T-9`.
- **The README has a recorded demo and a tour.** One take of starting an
  agent, answering it and merging its branch, plus clips of search, the
  crown and the ticket page. The README is rewritten for a newcomer; the
  reference material moved to `docs/USING.md`, `docs/PROMISES.md` and
  `docs/REMOTE-CONTROL.md`.

### Changed

- **Archiving DONE in bulk (the header's X offer) now removes merged
  tickets' worktrees**, as archiving a single ticket already did. Earlier
  bulk archives left every worktree, and its build output of several GB, on
  disk; a sweep removes those too. Unmerged work keeps its worktree.
- **The crown is woken by what happened to a ticket, not by every turn its
  agents end:** a delivery (something new to merge), the answer to its own
  `ask_agent`, and a raised hand. Each wake line names the change.
- **The automatic-move fuse no longer trips on one agent's own turn
  cadence.** A blown fuse lapses on its own after two quiet minutes, and its
  notice names the ticket to move by hand to clear it sooner.
- **The crown's `start_agent` answers `status: "started"` or
  `"waiting_for_worktree"`** instead of `session_started: true/false`. A
  start parked while the worktree is being cut is not a refusal; refusals
  are errors.
- **ratatui 0.30; building from source needs Rust 1.88.** This closes the
  `lru` advisory (GHSA-rhfx-m35p-ff5j). Nothing visible changes.

### Fixed

- **The advisory row starts at the footer's left edge.** A long notice no
  longer pushes its `+N more` count off the row.

## v0.1.0-alpha.27 — 2026-09-26

### Added

- **The diff viewer marks the changed words of an edited line.** A removed
  line and the added line most like it are paired, and the words that
  differ are bold on a slightly stronger tint; the rest of the pair drops to
  regular weight. Lines with no partner, pairs more than 60% rewritten and
  lines over 2 KiB keep the whole-line colour.
- **The push / pull lists open a commit's diff.** On the checkout diff's
  push / pull view, `j`/`k` move through TO PUSH and TO PULL and `Enter`
  opens that commit's diff against its first parent; `q` returns to the
  list on the same row.
- **A board over several repositories lists push / pull per repository.**
  Each repository gets one row with its name, branch, state and when it was
  last fetched. Repositories with commits to push or pull come first, their
  commits underneath (`↑` to push, `↓` to pull); then those in sync (`✓`);
  then those with no upstream or a detached HEAD. A repository with no
  upstream is compared against the one remote branch with its name; no git
  config is written. The header sums every repository's arrows.
- **`f` on the push / pull view fetches every repository.** It fetches the
  board's branch and, on a multi-repository board, each repository compared
  with a remote, four at a time. Each row shows `fetching…`, then its fetch
  age or `fetch failed: <git's message>`. The periodic `MESIMON_GIT_FETCH`
  still fetches the root repository only.
- **A non-Latin keyboard layout pauses the board's keys.** Typing a Hebrew
  (or other non-Latin) letter on the board no longer does nothing, or the
  wrong thing through punctuation such as `/` and `.`. The footer names the
  layout and flashes on each dropped key; an English letter ends the pause
  and acts, and `Esc` dismisses it. Arrows, `Enter`, `Tab`, digits and
  `Ctrl` chords keep working. A Hebrew letter typed in a text field arms the
  pause before you return to the board. Known gap: if the layout changed
  outside mesimon, the first key can still arrive as `.` and repeat the last
  move.

### Changed

- **The mascot is an animated ש with one animation per agent state.** On
  the ticket page it stands over the empty seat and over a session with
  nothing to show, acting out waking, thinking, working, needs you, done,
  sleeping, failed and the other states; working types at a laptop and
  thinking fills a thought cloud. A small copy stands beside an agent's
  reply when the preview is at least 72 columns wide. Notification images
  and the installer's welcome use the new drawing.
- **The agents' `write_note` tool says an approved plan is already a
  note.** Agents were saving their plan a second time after mesimon had
  recorded it from the plan approval.

### Fixed

- **A refused `H`/`L` move leaves the cursor on the card.** When the DONE
  gate refused a nudge, the card stayed but the cursor moved to the next
  column. The card now shakes, the cursor stays on it, and `.` does not
  repeat the refused move.

## v0.1.0-alpha.26 — 2026-09-24

### Compatibility

- **`columns.toml` moves to schema 6 and ticket files to schema 7.** An
  older build leaves `columns.toml` untouched and does not write to it, and
  skips, with a notice, any ticket this version has saved until the newer
  build is back.

### Added

- **Agent tiers name how a ticket's agent launches: provider, model and
  effort.** Settings → Agents → `Tiers` lists them for this machine, or with
  `b` for this board only, and `Default tier` replaces the Provider row. The
  built-in `claude` and `codex` tiers pass no flags. `Ctrl+N` on the board or
  the ticket page moves a ticket to its next tier: an empty seat starts on
  it, a parked agent wakes on it, and a running agent is relaunched on the
  same conversation at its next idle, never mid-turn and never while you are
  in its pane. In the composer and the ask field, `Ctrl+N` picks the tier the
  words launch on. The ticket page's state row names the tier (`on coder ∙
  ^n next`), and a card waiting to switch says `switches to coder`. A seat
  cannot switch between Claude and Codex, and a model name the CLI does not
  accept fails at launch.
- **`mesimon update` installs the newest release from a shell.** It asks
  for the latest release and, when a newer one is out, downloads, verifies
  and installs it the way the Esc menu's Install row does, restarting
  nothing; an open board then offers `U`. `mesimon update --check` only
  asks. `MESIMON_NO_UPDATE_CHECK` does not silence the command.
- **Opening Release notes checks for a newer release at once** instead of
  waiting for the half-hour check. A newer release raises the header chip as
  before, and an answer less than 60 seconds old is reused.
- **`Tab` in the note editor opens the ticket's next note.** On a ticket
  with more than one note, `Tab` walks them in the rail's order and wraps to
  the description; the heading shows the position, for example `NOTE 2/3`.
  A note with unsaved changes stays open and the status line reads `unsaved
  ∙ ^s saves ∙ esc discards`. `Tab` never discards.

### Changed

- **The first attach shows an animated guide to the way back.** Before your
  first session opens, the practice pane shows a SESSION → BOARD diagram
  whose keycaps step through holding Ctrl and tapping 5, with `Ctrl+]` named
  as the alternative. Pressing the shortcut opens the session, as before;
  other keys are ignored on this screen.
- **Every refused card action shakes the card**, not only a refused
  archive. A move stopped by the DONE gate, `d d` over an unmerged branch,
  `z` on a ticket whose Claude agent is working, a workspace switch with an
  agent running, `c` on a taken seat, and a refused wake, resume, crown,
  manual merge, tag cycle or queued ask now shake the card as well as
  showing the reason in the status line. Refusals shown inside a dialog or
  a text field do not shake.
- **Crowning a ticket sweeps its title into the crown's colour.** After
  `Ctrl+O`, a wave crosses the title on the card and on the ticket page in
  1.3 s and leaves it in the crown's tint. The crowned card's title keeps
  that tint under the cursor. Below 24-bit colour a single highlighted cell
  walks the title; monochrome terminals show no motion.

### Fixed

- **A refused plan or question clears from the card.** After you answer an
  agent's plan with "No, keep planning", dismiss its question or deny a
  permission, the card kept reading `plan` or `question` until the agent's
  next dialog. It now clears on the agent's next tool call; a subagent's
  tool call does not clear it.
- **The External drawer no longer lists sessions mesimon started.** A
  conversation from a deleted ticket, one left behind by `/clear` or a fresh
  resume, and an exited session used to reappear there. mesimon now records
  every conversation it starts in a new state file, `started.json`, seeded
  on the first start from its existing launch records. Agent SDK runs, such
  as a plugin's review hook, are no longer listed. A conversation cleared
  with `/clear` before this version can still appear.
- **Scrolling the release notes and large diffs keeps up with the keys.**
  Both screens laid out their whole document on every frame while `j` was
  held or `{ }` paged; they now lay it out once per width and file.

## v0.1.0-alpha.25 — 2026-09-23

### Added

- **A worktree ticket on a multi-repository board gets one worktree per
  repository.** When the board's root is a folder that holds several git
  repositories, a ticket with the worktree workspace now gets a container
  under the state directory with one worktree per repository, all on the
  ticket's `msmn/` branch. `m` merges each repository in turn, fast-forward
  only; a refusal partway leaves the landed ones landed, and the next `m`
  continues. The ticket page names each repository with something to say,
  for example `api +3 ∙ web ✓`, and `get_ticket` carries one merge row per
  repository. The old refusal for a worktree ticket on such a board is gone.
  `worktrees.json` moves to schema 2; an older build leaves the file
  untouched and does not write to it.
- **Shift+Enter accepts an agent's plan from the board.** On a card whose
  agent is on its plan dialog, Shift+Enter opens the ask field at
  `accept plan`. A blank Enter presses the dialog's own default row
  (Claude Code's `Yes, and use auto mode`, Codex's `Yes, implement this
  plan`); words plus Enter accept the plan and paste the words into the
  turn as soon as the agent confirms it left the plan. An ask queued while
  the agent is still planning carries the same flag. A column's Shift+Enter
  field opens at `accept plan` when any of its seats can accept one, and
  accepts every one of them, one per shared checkout at a time. A queued
  ask that meets a question on the dialog is held for your `Ctrl+Y` rather
  than dropped, and `Ctrl+Y` into a dialog is refused.
- **Ctrl+P starts the agent in plan mode from the composer or the ask
  field.** The hint reads `plan mode` and toggles; the row says `∙ plan
  mode` while it is armed. The flag applies to the one launch the words end
  in: a fresh Claude session, a parked one being woken, or an idle live pane,
  which is parked and relaunched in plan mode (the feed records
  `plan_relaunch`). A working pane refuses the send-now form, and a queued
  ask waits for idle. Codex has no plan mode and is refused. The crown's
  `start_agent` and `ask_agent` tools take the same `plan` flag. `queue.json`
  moves to schema 2 to carry the flag.
- **Pressing `m` on the ticket page opens a merge dialog.** The dialog names
  what the merge will do and why to stay on the page, with its keys along the
  bottom edge. While git runs, a spinner turns in the dialog and keys are
  held; a merge that lands turns the dialog into the "tell the agent"
  question instead of closing it.
- **The crown is told when an agent it started finishes.** When a
  crown-started agent ends its turn or raises its hand and the crown is
  idle, one sentence is pasted into the crown's session. Finishes under a
  working crown are collected into one sentence, a person's queued ask on
  the crown goes first, and the hand's reason is not included. Uncrowning
  drops what was owed. The feed records `crown_wake`.
- **A refused archive shakes the card.** Pressing `a` on a ticket whose
  sessions are awake shakes the card on the board, or the title row on the
  ticket page, three times in colour-neutral motion, in addition to the
  status-line refusal.

### Changed

- **The tmux status line names the ticket by key.** The focused pane's
  breadcrumb reads `T-428 title` with the key in bold, matching the ticket
  page's chip; the title is still cut at 48 characters.
- **Every paste mesimon sends now waits for the agent's acknowledgement.**
  A `p` prompt, a manual merge request, a note's nudge and a queued ask sent
  by hand now expect the `UserPromptSubmit` hook (or Codex's new turn) like a
  ticket's first prompt, and the feed records the acknowledgement. A Claude
  record left with a pending prompt across a daemon restart is cleared at
  boot instead of holding its checkout.
- **Creating a ticket from the composer is one command.** Title, tags and
  description arrive together, so an auto-run agent starts on a card that
  already has its brief and tags. A failed create leaves no ticket behind.
  A board viewer can no longer create a ticket through the composer.

### Fixed

- **Three tickets behind one merge are asked to rebase once each, not six
  times.** The merge train held a ticket only until its git step cleared
  `needs rebase`, so while the agent's tests still ran it was asked again
  for the same base. The hold now lasts until the agent's turn ends.
- **A woken session no longer inherits its predecessor's death.** A wake
  reuses the session name and uuid, so the old process's `SessionEnd` and
  `pane-died` frames could land on the new record. Every hook frame now
  names its pane, and a death frame from another pane is dropped without
  asking tmux.
- **A transcript keeps being read after Claude Code moves it.** When a
  session changes its working directory through Claude Code's own worktree
  tool, the transcript file moves to another project directory; the card
  read "nothing to read in its transcript". The daemon now follows the
  file. A `cd` in Bash does not move it.
- **Adopting external sessions reads the whole transcript head.** A fixed
  8 KiB window cut the first real record mid-line on 230 of 550 transcripts
  measured (a pasted prompt or image can make it large), so those sessions
  had no working directory. The head is now read line by line under a 4 MiB
  cap. A resumed session's stale pid file also no longer hides the live one.
- **Provisioning a worktree on a multi-repository board no longer stalls
  the daemon.** The flag refresh ran on the daemon's main thread, one git
  round per repository (1.5 s on a 12-repository board); it now runs on the
  worker. A single-repository worktree's flags also refresh after a fetch
  without waiting for a page open.
- **The rebase ask on the card reads `waiting for rebase`**, and its note
  says the agent rebases first and `m` merges after.
- **Ask-field wording under a plan dialog.** The field's one stop is
  `accept plan`: Shift+Tab no longer cycles it to `queued`, and the grown
  field reads `accept the plan ∙ words go right after` instead of
  `ask agent`.
- **A squash-merged branch reads merged even when the base changed nearby.**
  The merged-by-patch check compared both sides with three lines of context,
  so a commit on the base within three lines of the branch's edit, landed
  before the squash, left the ticket reading unmerged and blocked DONE and
  delete. Only the changed lines are compared now.
- **`External sessions` in the Esc menu opens at once.** The scan of
  `~/.claude` transcripts ran on the daemon's main thread (1.2 s over 2,000
  files), holding every client and hook while the board waited. It runs on a
  worker now: the drawer opens on the last answer, shows that it is
  scanning, and refreshes when the scan lands. Long lists scroll, with the
  position shown in the title.
- **A quiet pane with a tool call in flight no longer reads `interrupted`.**
  A long test run inside a tool wrote nothing to the pane for a minute, and
  the silence probe marked the turn interrupted. The probe now checks the
  transcript first and holds while a tool call has no result yet. A session
  with no transcript path is still judged on silence alone.

## v0.1.0-alpha.24 — 2026-09-22

### Added

- **A crowned agent can start agents on other tickets.** The new `start_agent`
  tool (crown only) starts the board's agent on another ticket the way
  Shift+Enter does: the title and description become its first prompt, and a
  worktree ticket gets its worktree. A crowned agent may have at most 3 such
  agents running or sleeping at once; `Settings › Agents` cycles the number for
  the current board, `crown_budget` in `columns.toml` holds it, and `0` turns
  the tool off. A ticket started this way cannot itself be crowned. The card
  shows `♛ started`, and the feed records `start_agent`.
- **A crowned agent can queue words for another ticket's agent.** The new
  `ask_agent` tool (crown only) parks the text on that ticket's card, marked
  `queued by T-N's agent`; nothing reaches the other agent until you press
  `Ctrl+Y` on the ticket page to send it, and `Ctrl+U` takes it back. One ask
  per ticket, and a daemon restart discards it. The feed records the ask but
  never the words.
- **`get_ticket` now tells an agent what the board's automatic moves will
  do.** The reply carries the current column's on-working and on-done rules
  under `automove`, and the `move_ticket` text says the board itself moves a
  ticket when a turn starts or ends. This is guidance in the tool text, not
  enforcement.

### Changed

- **The `create_ticket` tool text and the agent brief say a filed ticket is
  worked by its own session.** An agent that files a sibling ticket is told
  not to build it in the current session. A brief you already accepted is not
  offered again.
- **A headless daemon no longer runs `git status` every 10 seconds.** The
  sample runs only while a board is open or a merge is waiting to retry, and
  the first board to open after a quiet stretch gets one fresh sample.
- **Drawing the board and the ticket page does less work per frame.** The
  frame now builds its key-availability context once instead of three to six
  times, and a card's peek statistics are reused for a quarter second. The
  checkout list reads at most 16 MiB of file content per listing for its
  line-count badges; a larger checkout shows no badge on the rows past the
  budget.

### Fixed

- **Queued agent starts and wakes now survive a daemon restart.** Starting a
  column with Shift+Enter parks the tickets that share a checkout; when the
  daemon restarted, every parked start after the first was lost with no line
  in the feed. The queue is now saved to `queue.json` under the state
  directory and restored at boot, and the feed records each
  `queued_start_restored`. A queued follow-up prompt is still memory-only.
- **`write_note` refuses a note over 32 KiB instead of silently cutting it.**
  The existing note keeps its text, a new note is not created, and the error
  names the submitted size and the limit in bytes. Plan notes captured from
  hooks are still cut at the limit.
- **A page into the diff, the preview or the release notes no longer
  carries into the next document you open.** Each reading position is kept
  per document, and the release notes now glide like the other two.

## v0.1.0-alpha.23 — 2026-09-21

### Added

- **Remote Control opens a board in your own browser, on desktop or phone
  (preview).** View tickets, preview an agent's output, send a prompt, approve
  or deny Claude permission requests, and answer supported Claude dialogs.
  Debug builds include it; release builds need `MESIMON_MESOPHON=1` at startup.
  Open `Esc › Sharing › Remote Control`, sign in to your relay, enable the
  board, then `Pair a browser` and enter the single-use code within ten
  minutes. Each board is enabled and paired separately, and enabling it shares
  nothing with teammates. The host must stay awake with its daemon running.
  `mesimon mesophon setup` checks the connection and opens the browser for a
  browser on the same Mac, with no certificate to install; a phone or another
  computer needs a reachable HTTPS relay. Existing agent sessions need
  refreshed hook settings before they offer remote approvals. The preview has
  no board editing, no agent start or stop, no interactive terminal, and
  cannot start a stopped daemon; alerts require the browser to stay connected.
  Select a paired device and press Enter twice to revoke it; disabling Remote
  Control removes every grant for that board.
- **Pictures paste into a ticket note or a new ticket's description with
  `Ctrl+V`.** Each appears in the text as `[Image #N]`; saving keeps the PNG
  with the ticket, and the note's `Ctrl+K` links menu opens it in your image
  viewer. The ticket's agent can read them through the new `read_attachment`
  tool. Local macOS, X11 and Wayland desktops are supported; SSH and WSL are
  not. Limits are 10 MiB and 25 megapixels per picture and 50 MiB of pending
  pictures per draft. Discarding a draft discards its new pictures, and shared
  boards synchronize the note text only.
- **One ticket per board can wear the crown (`^o` on its card), letting its
  agent edit the other tickets.** A crowned agent may move, retitle, tag,
  annotate, archive and set the workspace of any ticket; each edit is checked
  against the ticket as that agent last read it, lights the touched card, and
  leaves a mark until you see it. Only you can crown a ticket — an agent that
  asks for the crown is told to ask you. Archiving or snoozing the crowned
  ticket by hand drops the crown.
- **`Settings › Agents › Sleep idle agents` sleeps finished agent sessions
  after 15, 30, 60 or 120 idle minutes.** It is off by default, applies to the
  current board, and keeps working with the TUI closed. Running turns,
  background work and sessions needing attention stay awake; waking resumes the
  same conversation. The board's `park_after_minutes` setting takes any whole
  number of minutes, and `0` disables it.
- **Follow-up prompts now wait for the agent's current turn to end.** `Queue`
  is the default and holds the prompt through approval and question stops; a
  queued prompt shows on the ticket page, where `Ctrl+Y` sends it now and
  `Ctrl+U` takes it back for editing. `Settings › Behaviour › Follow-ups`
  switches the default to `Steer`, and Shift+Tab flips a single composer. The
  queue holds one prompt per ticket in memory, and a daemon restart discards it.
- **An open card names its ticket's key.** `p` on the card under the cursor and
  `P` on every card now close the meta row with the short key, right-aligned
  under the age, with or without tags.

### Changed

- **Key hints and statuses say `agent` instead of `claude`.** `c` reads `start
  agent` and `wake agent`, Shift+Enter reads `ask agent`, the ask room's frame
  reads `ASK AGENT`, and `Settings › Agents` reads `Sleep idle agents`. Labels
  that identify one session — card and rail rows, the tmux breadcrumb, doctor's
  installation rows, the `Provider:` row — still name Claude or Codex.
- **Shift+Enter on a column header now starts an agent on the tickets with no
  session.** It previously skipped them. A blank Enter starts each empty seat on
  its own title and says nothing to the agents already seated; the receipt now
  reads, for example, `asked 2 ∙ started 3`. A column of pending tickets sharing
  the checkout opens at `queued` and drains one agent at a time.
- **The `create_ticket` tool text now asks agents to extend an existing
  ticket rather than file a near-duplicate.** It describes a ticket as a unit of
  work to pick up, points at `list_board` as the duplicate check, and says the
  older ticket wins. This is guidance in the tool description, not enforcement.
- **The README explains why `shift-enter` may do nothing in iTerm2.** Claude
  Code's `/terminal-setup` installs a `⇧↩` binding that sends a plain newline
  and defeats the key; delete that row under Keys → Key Bindings and restart
  mesimon.

### Fixed

- **A session that published an artifact no longer counts as working forever.**
  Claude Code lists an ambient artifact watch as a running monitor in every
  `Stop`, so a finished session read as monitoring and never reached REVIEW.
  Only a monitor whose task the session actually started counts now.
- **A session whose background work goes silent no longer reads as working for
  hours.** A background wait that gets no further signal for ten minutes now
  settles to idle, which is what the automatic column move needs.
- **Codex sessions that never reported `stopped` no longer hold the shared
  checkout.** Records left mid-stop counted as working, so queued asks on that
  board waited forever on cards showing nothing running. Such a record now keeps
  its place on the rail and releases the checkout and the agent seat after 60
  seconds. The check also survives a cleared `/tmp`.
- **Interrupting a Codex compaction no longer leaves the card running.** An
  interrupted turn now retires that turn's pending compactions and settles to
  interrupted, instead of waiting for a completion that never arrives.

## v0.1.0-alpha.22 — 2026-09-15

### Added

- **Shift+Enter on a column header asks every seated agent in that column
  the same prompt.** The ask field opens under the header; Enter pastes the
  words into every agent's pane, wakes parked agents with the words held,
  and skips tickets with no agent. The receipt reads
  `asked 3 ∙ queued 2 ∙ 1 without claude`. The batch never starts a session.
  The field opens at `queued` when any of those agents shares the checkout;
  Shift+Tab flips it to `now`, and tickets on their own worktree are always
  sent now.
- **`Tab` in the ask field grows it into the full composer.** Enter breaks a
  line, `Ctrl+S` sends (or queues, when the delivery row says `queued`),
  Shift+Tab flips now/queued where the field offers it, and Esc folds the
  text back into the one-line field. The frame names the destination
  (`ASK CLAUDE`, `ASK EVERY CODEX`).
- **A prompt can now contain line breaks.** A multi-line prompt keeps its
  lines in Claude's and Codex's input box and still submits as one turn.
  Prompt history keeps the lines. Prompt templates remain one line.

### Changed

- **Shift+Enter over a ticket with no agent opens the ask field instead of
  starting the agent on the ticket title.** Typed words become the agent's
  first prompt; a blank Enter still sends the title. Nothing spawns until
  Enter. The delivery row opens at `now` on a quiet shared checkout and at
  `queued` while another agent works in that checkout, with Shift+Tab
  flipping either; a worktree ticket opens at `now` with no toggle. The hint
  reads `start + ask claude`.

### Fixed

- **Sleeping a session and waking it immediately no longer asks for a
  confirm or leaves the woken session marked crashed.** Pressing `x` and
  then `x` on a ticket page while the agent was still running its exit hooks
  answered `running elsewhere (pid N)`; confirming killed that process under
  the freshly spawned pane, and its late exit marked the new session's record
  crashed. A session genuinely running elsewhere, and a genuine crash, are
  still reported.
- **Holding `k` on the first ticket of a column stops there** on terminals
  that speak the kitty keyboard protocol. The held key previously carried the
  cursor onto the column header and the board's top row; holding `↑` already
  stopped. A fresh `k` after a pause still steps onto the header.
- **Handing over to a session's pane no longer types stray characters such as
  `49;2:3u` at its first prompt** on terminals that speak the kitty keyboard
  protocol. Mesimon now waits for the terminal to confirm the protocol is off
  before attaching.

## v0.1.0-alpha.21 — 2026-09-13

### Added

- **A ticket's terminal (`!`) is adoptable and supports several shells.**
  Pressing `!` opens the ticket's own terminal (worktree or checkout); the
  ticket page shows it as a row under the sessions, and Enter arms it, then
  Enter again adopts it into a real shell session. After adoption, `!` opens
  a fresh terminal beside it, so a ticket can hold as many shells as it
  needs. A shell running a command spins its row and the card. `x` on a
  shell row closes that shell's pane outright; bulk sleep gestures still
  only park agents and leave shells alone.
- **One board can override selected machine preferences.** Press `b` in
  Settings to switch to THIS BOARD scope, where Enter cycles
  inherit/on/off per preference and the picker gains an inherit row.
  Status line side and week start remain machine-only. `doctor -v` prints
  a board-prefs line.
- **The reply row's peek setting (`p`/`P`) is remembered across launches**,
  stored as a machine preference (`peek`: off / cursor / all). The next
  board you open keeps the rung this one was left on; `doctor -v` gained a
  `replies` line.
- **The search picker (`/`) opens on recently viewed tickets** when nothing
  is typed yet, newest first under a "viewed recently" subtitle. Typing
  still searches the whole board as before.

### Changed

- **A collapsed column's count caps at 9**, with a superscript `+` on the
  row beneath it for ten or more. Previously a two-digit count pushed the
  column name down a row.
- **Claude Code's three read-only MCP tools (`get_ticket`, `list_board`,
  `read_note`) no longer prompt for approval in plan mode**, and are
  pre-approved by default so plan-mode sessions stop asking on every turn.
  Write tools still prompt as before.

### Fixed

- **A plan or question an agent is still waiting on no longer loses its
  needs-you mark after 15 minutes** if the transcript still shows it open.
  The wait now restates itself once a minute from the transcript tail; a
  wait nobody restates (a lost answer frame) still demotes at 15 minutes as
  before.
- **A Codex session that crashed before confirming cleanup no longer holds
  a deleted ticket's machine-awake lock forever.** The daemon now releases
  such orphaned records once ownership checks pass, retrying every 15
  seconds.

## v0.1.0-alpha.20 — 2026-09-11

### Added

- **Search the board with `/`.** Type any part of a title, short key, column
  or tag and the ranked list narrows as you type. `Ctrl+N`/`Ctrl+P` and the
  arrows walk it, Enter puts the board cursor on that card, Esc leaves the
  board where it was. Archived tickets are ranked below every live one and
  marked; `Tab` takes them out. fzf query syntax works (`'literal`,
  `^prefix`, `suffix$`, `!exclude`; words are ANDed).
- **See what a checkout has to push and pull.** On the checkout Git screen
  (`v`), `Tab` switches between uncommitted changes and an upstream
  comparison listing **To push** and **To pull** with short object IDs and
  subjects, newest first. Incoming commits come from already-fetched refs —
  the view never fetches, pushes or pulls. Each direction lists at most 100
  commits and says when it omitted more. Ticket branch diffs are unchanged.
- **Rewrite the sentences mesimon sends your agents.** Settings → Agents →
  Agent prompts holds the three: the rebase ask, the merged notice and the
  note-changed nudge. Each template is one line and may use `{branch}` and
  `{base}` (the two merge prompts) or `{note}` and `{id}` (the nudge); an
  unrecognised `{word}` is left as written. `Ctrl+U` empties a field, which
  restores mesimon's own wording. Templates are per repo, stored in
  `columns.toml`; agents cannot change them.
- **The ticket page remembers the previous column stay**, shown as
  `previously IN PROGRESS for 1h 1m`. Only visits longer than 60 seconds are
  recorded; a later long visit replaces the last one. Existing tickets start
  without history.
- **Description and note editors soft-wrap at the editor's width**, including
  the new-ticket composer. Up/Down and PageUp/PageDown follow visual rows.
  Stored text, explicit newlines and indentation are unchanged.

### Changed

- **Copy actions use the local clipboard.** Copying a board link or the agent
  brief now runs `pbcopy` on macOS, `clip.exe` on WSL, `wl-copy` on Wayland,
  or `xclip`/`xsel` on X11, and only says `link copied` / `brief copied` once
  the helper succeeded. SSH sessions and machines with no helper keep OSC 52
  and say `copy requested from terminal`, which the terminal may still
  refuse.
- **The ticket page's subtitle leads with tags**, and the workspace clause —
  strategy word, branch, merge state — moved to its own row below. A tagged
  ticket's branch name is no longer truncated to make room for chips.
- **The ticket page reads its description in one place.** With a note
  selected, the header band drops its excerpt and the reading zone's heading
  says `DESCRIPTION` on the first note and `NOTE` on the rest.
- **The merge train waits on fewer things.** It no longer holds for agents
  working in unrelated worktrees, and no longer requires five seconds of
  terminal silence — an idle agent whose prompt animates is no longer read as
  busy.
- **The breadcrumb separator is `›`.** The ASCII theme tier keeps `>`.

### Fixed

- **Idle session previews and the `p` peek show the agent's closing reply
  again.** They had been showing the user's own last prompt instead.
- **Branches with no commits are no longer asked to rebase.** The merge train
  spent a whole agent turn on a fresh worktree every time the base moved
  under it.
- **Merging from the ticket page says `merging N commit(s)…` while it runs**,
  instead of leaving the pre-confirmation frame on screen. Keys typed during
  that wait are discarded, so holding `m` no longer sends the merged notice
  to the agent and starts a turn.
- **A blocked merge no longer lets the train ask other tickets to rebase.**
  When a merge candidate cannot land — uncommitted changes in the main
  checkout, for example — the pending row names that ticket instead of
  promising `next`, and the rebases wait for it.
- **Codex sessions with a custom status line receive their first prompt.**
  Input readiness now reads the composer's own text cursor, so any status
  line configuration works, including none.
- **Option+Left and Option+Right move by word in text fields.** Terminal
  profiles that send `ESC b`/`ESC f` for those keys were ignored.
- **Closing a board while attached to a pane no longer strands the focus
  token.** Every later attach answered `another session is focused` until the
  daemon restarted.

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
