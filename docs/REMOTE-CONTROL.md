# Remote Control (browser preview)

Remote Control (Mesophon) lets your own browser **view tickets, preview an agent’s
output, send a prompt, answer supported Claude dialogs, file new tickets, move, rename
and tag tickets, start an agent on a ticket, and read and edit a ticket's notes**. It works on
desktop and phone.
Debug builds include Remote Control automatically. In release builds, set
`MESIMON_MESOPHON=1` when starting Mesimon. Open **Esc → Sharing → Remote Control**, sign in to
your relay, and enable this board.

Signing in registers this Mac with the relay once; the sign-in rows are the relay's address,
your display name and an **access code**. The address is prefilled with the hosted relay,
`relay.mesimon.dev:443`, which needs no pin; a self-hosted relay replaces it with its own
address, a space and the pin that relay printed when it started. A relay that gates registration (the hosted one does)
refuses a sign-in without one and says so under `Sign in`; a self-hosted relay ignores an empty
field. A code is either one a friend minted for you, which never runs out, or a license key
from a purchase, which lasts as long as the subscription. A code is used at most as many times
as it was minted for. Once signed in, **Enter a code** under your name takes a later code: a
renewal, or a friend's code on a Mac that signed in before the relay had a gate. When a Mac's
access runs out it keeps reading every board it belongs to, and a paired phone keeps its live
view, but edits stay on the Mac (the sharing dialog reads `LAPSED`) and a ticket filed from a
phone while the Mac is away is refused, until a code is entered. Choose **Pair a browser** and scan the QR code the
dialog shows with your phone’s camera: the page opens with the single-use code filled
in, and **Connect** pairs. Or open the displayed browser address and type the code. The
code works once, within ten minutes. The QR appears when the terminal has room for it
(beside the dialog from about 110 columns, under it from about 40 rows).

## Before you pair

Each board requires explicit enablement and pairing, including private boards.
Enabling Remote Control does not share a board with teammates. The host must stay awake
and its board daemon must be running. This preview does not yet provide editing a
ticket's other details, stopping agents, interactive terminals, or starting stopped
daemons.
For a browser on this Mac, set the relay’s `WEB_ORIGIN=http://localhost:8444`
and publish port 8444 on loopback only. Run `mesimon mesophon setup` to check the
connection and open the browser; there is no certificate or Keychain setup.
`mesimon mesophon setup --check` checks without opening a browser. A phone or
another computer needs a reachable HTTPS relay with a browser-trusted certificate;
`localhost` always means the device running the browser. The relay's deployment
guide ships with the relay.

## On your home screen

The page installs like an app: **Add to Home Screen** in Settings where the browser
offers it (Chrome on Android, Edge), or in Safari the Share sheet’s **Add to Home
Screen**. On iPhone and iPad the Home Screen app keeps its own pairing, separate from
Safari’s, so pair it from the Home Screen app itself.

Once the page has loaded while online, it opens with no signal too: the board as it was
last seen, your Sent list, and new tickets, which wait in the browser with a clock. It
opens no connection while it is offline. When the relay is reachable again, the page
reloads itself (unless the new-ticket sheet is open), and the tickets that waited go out.
While the relay answers, the page is always the relay’s current one.

## Finding your way

**Now** puts what needs you first: a permission request or a single-choice
question can be answered right on its card, and anything else opens the ticket.
Below it are the agents that are working, then the idle ones. A ticket is the same
card in Now and on the Board: its title, its tags and key, how long the agent has been
in its state and, while live, the step it is on (a tool call, in mono) or the first
line of its latest reply. In Now the card also names the ticket’s column. **Board**
shows every ticket by column: one column at a time on a phone, all of them side by
side on a wide screen. The status pill says whether the board is live. When it is not, the
page says which hop is out of reach (this browser, the relay, or your terminal),
keeps showing the board as it last saw it, marked as not live, and disables
answers and prompts until the terminal is back.

## Filing a ticket

**New ticket** (the floating button on a phone, the header button on a wider screen, or
**Add to** at the foot of a board column) opens a sheet with a title, optional details
(Markdown, up to 32 KiB, saved as the ticket’s description), a column and the board’s
own tags, one per group. **Sent** lists the tickets this browser filed, and its bar
files a ticket from a title alone.

A ticket goes out whether or not your terminal is reachable, and its ticks say where it is:

- **A clock**: sealed and saved in this browser, because this browser or the relay is
  out of reach. It goes out by itself when they are back, reloads included.
- **One tick**: sealed at the relay, waiting for your terminal. Until the terminal
  collects it you can **Unsend** it (the relay deletes it) or **Edit** it (taken back,
  then sent again as you change it).
- **Two ticks** and the new key: on your board, with **Open** to go to it. Only your
  terminal can seal this answer, so the relay cannot claim a ticket landed.
- **Teal ticks**: picked up. You opened the ticket at your terminal, or an agent started
  on it; Sent says which and when. The browser learns it the next time it is live with
  your terminal.

Now lists the tickets on their way under **Waiting to land**, and Board shows them as
dashed cards in their columns. A ticket from the browser lands quietly: no agent starts
on it. It is filed as yours and lands exactly once, however often it is delivered, and
a ticket you delete is not brought back by a late copy. Written against a column or a
tag the board has since lost, it lands in the default column, without that tag. A
terminal that has never been live with this version keeps no tickets for later, and
the sheet says so.

## Moving, renaming and tagging a ticket

On a ticket's page, press its **title** to write over it: **Enter** or **Save** sends the
new title, **Escape** or **Cancel** puts the old one back. Press the **line under the title**
(the ticket's tags and column) to open a sheet with the board's columns and tags. A column
moves the ticket there, to the end of that column. A tag goes on and replaces the ticket's
tag in the same group; pressing a tag the ticket wears takes it off. Each press reaches your
board as you make it, and **Done** closes the sheet. On a desktop's Board you can also drag
a card to another column, or to another place in its own.

These edits follow your terminal's rules. A column that needs the work merged refuses an
unmerged ticket, and the sheet says why. Only the board's own tags are offered: new tags are
made at your terminal. The edits need your terminal live; while it is out of reach the title
and the line are plain text. A terminal older than this version offers none of them.

## Starting an agent

A ticket with no agent offers **Start agent**, on its page and on its card on the Board.
It starts what **Shift+Enter** starts at your terminal: the agent the board’s tiers give
that ticket, with the ticket’s title and description as its first prompt, in the
ticket’s workspace. The button needs your terminal live, and is disabled while it is
out of reach. Its receipt uses the same ticks: a clock while your terminal starts it
(and cuts the ticket’s worktree, when it has one), then two ticks once the agent has
taken its first prompt. If the answer is lost, the browser asks what became of it and
never starts it again.

Your terminal refuses a start, and the page says why, when the ticket already has an
agent (a sleeping one included, which you wake at your terminal), when one is already
starting or a prompt for it is queued at your terminal, and when the ticket came from
outside the board (an import, or a teammate on a shared board): those start only at your
terminal. An agent you start from the browser is yours like any other, and revoking the
browser leaves it running. A terminal older than this version offers no button.

## Sending a prompt

A prompt targets the session shown when you send it. **Submitted** means delivery
to the agent’s input, not completion of its work. Waiting Codex prompts retain the
paired device’s authority and are cancelled if access is revoked or the target
changes. Reconnects never resubmit prompts automatically. If a delivery result
cannot be recovered, the browser shows **outcome unknown**; check the agent before
sending again. Prompt text is limited to 4096 UTF-8 bytes; previews show the last
50 lines, and oversized responses are rejected.

## Permissions

Claude permission requests can be approved once or denied while their remote
window is open. Claude’s local dialog remains available while the hook waits;
unanswered requests leave its native permission flow unchanged.
The write-protection gate remains deny-only; remote approval never installs rules
or changes the permission mode. Existing sessions need refreshed hook settings
before they offer remote approvals.

## Questions and plans

Single-choice questions, single-line free-text answers, and plan accept/reject
use verified native dialog selections. Plans are accepted with manual edit
approval. Unrecognized, multiple-question, and multi-select forms require a local
answer; uncertain delivery is never retried automatically. “Decision sent” and
“Answer keys sent” confirm transport, not tool execution or completion.

## Notes

A ticket's page shows its description under the title, then its other notes as rows. A row
opens the note; **Previous** and **Next** walk the ticket's notes in order. **Edit** opens the
whole note in a sheet; **+ Note** adds one; **Delete** removes a note (never the
description) after a second press. A save names the version of the note it was opened at: if
the note changed at your terminal since, nothing is written, your words stay, and the note
offers **Keep theirs** or **Save mine**. After a save, **Tell claude** (or codex) points an
awake agent at the note with the same sentence the desk's second `^s` sends.

Notes you have opened stay readable in this browser while your terminal is out of reach,
marked with when they were read. An edit made then waits at the relay, sealed, with one tick,
and lands when the terminal is back; until then **Unsend** takes it back. Pictures in a note
are named, not shown.

## Alerts

Connected-browser alerts carry encrypted per-ticket phase changes. They never
announce completion on prompt submission and are silenced while a paired browser
is foregrounded on that ticket. Updates appear in the page with a ticket link.
Enable optional system notifications through
**Enable connected-browser alerts**. This version requires the browser to remain
connected; it does not provide closed-browser Web Push or mobile live activities.
Older M1 hosts remain usable through capability negotiation.

## Revoking access

Select a paired device in the Remote Control dialog and press Enter twice to revoke it.
Disabling Remote Control removes every grant for this board; re-enabling requires new
pairing. **Forget this browser** removes the browser’s local identity and remembered
boards; revoke on the host to remove the corresponding grants too.

## What is stored where

The daemon stores `mesophon.json` (mode 0600) in its existing per-board state
directory, containing the opaque board identity, the device grants and the ids of the
tickets it filed most recently from the relay, with their keys, and of the note edits it
answered most recently, with their answers. A ticket filed from the
relay also records that id in its own `ticket.toml`, and, once it is picked up, when and
how (`[picked]`: opened at the desk or an agent started). Pairing
secrets and delivery receipts live in memory. The browser stores device keys,
credentials, remembered grants and, per grant, the last board it saw, the tickets
it sent, and the notes it read and the note edits it sent, in IndexedDB. The remembered board holds ticket keys, titles, columns (with
their descriptions), tags, the board’s tag names, agent states and when each agent
entered its state; never output, an agent’s step or reply line, prompt or queued text,
tool input or dialog content. The Sent list keeps every ticket still on its way (sealed,
while it is only in this browser) and up to 50 settled ones: title, column, tags, key,
status and when it was picked up, and the details only of a ticket that has not landed.
The notes kept are those of the 40 tickets whose notes were read most recently: each note's
name, author and time, and the bodies read; a note edit is kept, with its words, until the
terminal answers it. Revocation and **Forget this browser** delete all three. The page’s service worker keeps a
copy of the page’s own files (HTML, scripts, styles, fonts, icons and the Wasm module)
in the browser’s Cache Storage, and nothing of the board. Output and unsent prompts remain in memory. The relay routes encrypted content and stores routing metadata for
Mesophon, plus the tickets and note edits that wait for an away terminal and the terminal’s answers:
sealed, with the board, device and ticket ids, deleted after 30 days. Browser assets are served by the relay, so the relay’s web
deployment is part of the browser client’s trust boundary.
