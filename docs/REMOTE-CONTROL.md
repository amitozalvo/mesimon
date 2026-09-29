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
own tags, one per group. **Sent** lists the tickets this browser filed: a clock while
the answer is out, two ticks and the new key once the ticket is on the board, and
**Open** to go to it. Sent’s bar files a ticket from a title alone.

A ticket from the browser lands quietly: no agent starts on it. It is filed as
yours, into a column the board already has, and can only wear tags the board already
has. Sending needs the terminal live; while it is away the
sheet keeps what you wrote and **Send** waits. If the answer is lost on the way, the
browser asks for it when it reconnects and never files the ticket twice; after a
host restart it says **delivery unknown**, so check the board before sending again.
Hosts from before this version do not take tickets, and the sheet says so.

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
directory, containing only the opaque board identity and device grants. Pairing
secrets and delivery receipts live in memory. The browser stores device keys,
credentials, remembered grants and, per grant, the last board it saw and the tickets
it sent, in IndexedDB. The remembered board holds ticket keys, titles, columns (with
their descriptions), the board’s tag names and agent states; never output, prompt or
queued text, tool input or dialog content. The Sent list keeps up to 50 filed tickets:
title, column, tags, key and status, and the details only of a ticket that did not
land. Revocation and **Forget this browser** delete both. Output and unsent text remain in memory. The relay routes encrypted content and stores only routing
metadata for Mesophon. Browser assets are served by the relay, so the relay’s web
deployment is part of the browser client’s trust boundary.
