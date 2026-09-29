# Remote Control (browser preview)

Remote Control (Mesophon) lets your own browser **view tickets, preview an agent’s
output, send a prompt, answer supported Claude dialogs, and file new tickets**. It works
on desktop and phone.
Debug builds include Remote Control automatically. In release builds, set
`MESIMON_MESOPHON=1` when starting Mesimon. Open **Esc → Sharing → Remote Control**, sign in to
your relay, and enable this board. Choose **Pair a browser**, open the displayed
browser address, and enter its single-use code within ten minutes.

## Before you pair

Each board requires explicit enablement and pairing, including private boards.
Enabling Remote Control does not share a board with teammates. The host must stay awake
and its board daemon must be running. This preview does not yet provide editing
existing tickets, agent start/stop, interactive terminals, or starting stopped daemons.
For a browser on this Mac, set the relay’s `WEB_ORIGIN=http://localhost:8444`
and publish port 8444 on loopback only. Run `mesimon mesophon setup` to check the
connection and open the browser; there is no certificate or Keychain setup.
`mesimon mesophon setup --check` checks without opening a browser. A phone or
another computer needs a reachable HTTPS relay with a browser-trusted certificate;
`localhost` always means the device running the browser. The relay's deployment
guide ships with the relay.

## Finding your way

**Now** puts what needs you first: a permission request or a single-choice
question can be answered right on its card, and anything else opens the ticket.
Below it are the agents that are working, then the idle ones. **Board** shows every
ticket by column: one column at a time on a phone, all of them side by side on a
wide screen. The status pill says whether the board is live. When it is not, the
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

Now lists the tickets on their way under **Waiting to land**, and Board shows them as
dashed cards in their columns. A ticket from the browser lands quietly: no agent starts
on it. It is filed as yours and lands exactly once, however often it is delivered, and
a ticket you delete is not brought back by a late copy. Written against a column or a
tag the board has since lost, it lands in the default column, without that tag. A
terminal that has never been live with this version keeps no tickets for later, and
the sheet says so.

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
tickets it filed most recently from the relay, with their keys. A ticket filed from the
relay also records that id in its own `ticket.toml`. Pairing
secrets and delivery receipts live in memory. The browser stores device keys,
credentials, remembered grants and, per grant, the last board it saw and the tickets
it sent, in IndexedDB. The remembered board holds ticket keys, titles, columns (with
their descriptions), the board’s tag names and agent states; never output, prompt or
queued text, tool input or dialog content. The Sent list keeps every ticket still on
its way (sealed, while it is only in this browser) and up to 50 settled ones: title,
column, tags, key and status, and the details only of a ticket that has not landed.
Revocation and **Forget this browser** delete both. Output and unsent prompts remain in memory. The relay routes encrypted content and stores routing metadata for
Mesophon, plus the tickets that wait for an away terminal and the terminal’s answers:
sealed, with the board, device and ticket ids, deleted after 30 days. Browser assets are served by the relay, so the relay’s web
deployment is part of the browser client’s trust boundary.
