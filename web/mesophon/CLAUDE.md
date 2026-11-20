# web/mesophon

Loaded when a session works under this directory; the root `CLAUDE.md` still applies.

**Mesophon (`web/mesophon`) has no build step.** The relay serves it as it is, under a CSP with
no inline style or script: styling is classes in `style.css`, and Preact/htm are vendored plain
modules with no bare imports (`vendor/README.md`). Its view subscribes to `store.js` in a layout
effect, never after paint, or a fast boot's emit is lost and the page freezes at "Starting…".
A paired phone's writes are `Action::FileTicket` into a column, `Action::StartAgent` on a ticket
(T-498, under `authorize_execution`), the card edits `MoveTicket`/`RenameTicket`/`TagTicket`/
`ChooseWorkspace`/`ArchiveTicket` (T-530, T-642, T-737; each is `Mutate` for every other principal, so `place_ticket`
asks `MoveTicket` of every mover), `Annotate` on a ticket's notes (T-532), `ParkAgent` on the
session its card shows (T-1090, the desk's `x` behind the daemon's sleep gate), `CrownTicket` on a
ticket (T-1182, the desk's `^o`; the owner's alone, denied to every other principal), `PressKeys` on the
agent session a card names (T-1208, the Terminal tab's keys and Stop; the daemon's `control_target` refuses a
shell, a terminal or a dead pane; T-1218, the person's decision: at a dialog the phone answers on a card the
keys are refused, and at every other stop, `Unknown` and an expired card included, they go in and answer the
agent's own dialog, any option it offers; never beside input the daemon owes the pane, nor within 500 ms of
an Escape) and a stopped agent's `resume` under `StartAgent` (T-1208): never widen `Paired` to `Mutate`, which also
reaches `PromptColumn`, merges and deletes, and a filed ticket starts nothing by itself. Every
side of the control socket drops a peer on a frame it cannot parse, so mail frames go only where
they were asked for: a host sends `Collect` after `ControlMail` answered, and the relay sends
`Mail` only then.

## Browser checks

From this directory run `node --test state.test.js assets.test.js` for state and
packaging. Iterate on the affected browser flow in one engine and viewport:

```sh
MESOPHON_UX_FLOW=notes MESOPHON_UX_ENGINE=chromium MESOPHON_UX_SIZES=phone node ux.test.js
```

| Source area | Flow names for the inner loop |
| --- | --- |
| Board, lists, shell, awareness, drafts, settings and theme contrast | `navigation` |
| Filing, delivery and Sent | `ticket`, `mail-prompt` |
| Starting, model/tier choice, terminal keys and resume | `start`, `tier`, `terminal` |
| Ticket edits and drag, workspace, archive and crown | `edit`, `workspace`, `archive`, `crown` |
| Questions, notes and pictures, conversation, offline shelf | `batch`, `notes`, `chat`, `shelf` |
| Ticket tabs (tablet and phone) | `tabs` |
| Pairing, history, keyboard, service worker, rejected credentials, oversized board, forgetting a board (phone) | `pair-link`, `back`, `keyboard`, `kept`, `refused`, `big-board`, `forget` |

Filters accept comma-separated names. After a web change, run the full
`node ux.test.js` matrix once. A docs-only rebase needs no browser rerun.
Both direct and npm invocations use `ci/test-run.py` for the disk reserve,
deadline and child cleanup. Every selected engine/viewport gets its own server
and `test-results/<engine>/<size>/`. Set `MESOPHON_UX_JOBS=1` for serial timing.
The first selected navigation case sweeps all fifteen themes. Every navigation
case does so when `style.css` or `themes.js` differs from the branch point with
main, or when explicitly requested with `MESOPHON_UX_THEMES=all`.
