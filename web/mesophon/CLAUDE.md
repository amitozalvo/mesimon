# web/mesophon

Loaded when a session works under this directory; the root `CLAUDE.md` still applies.

**Mesophon (`web/mesophon`) has no build step.** The relay serves it as it is, under a CSP with
no inline style or script: styling is classes in `style.css`, and Preact/htm are vendored plain
modules with no bare imports (`vendor/README.md`). Its view subscribes to `store.js` in a layout
effect, never after paint, or a fast boot's emit is lost and the page freezes at "Starting…".
A paired phone's writes are `Action::FileTicket` into a column, `Action::StartAgent` on a ticket
(T-498, under `authorize_execution`), the card edits `MoveTicket`/`RenameTicket`/`TagTicket`/
`ChooseWorkspace`/`ArchiveTicket` (T-530, T-642, T-737; each is `Mutate` for every other principal, so `place_ticket`
asks `MoveTicket` of every mover) and `Annotate` on a ticket's notes (T-532): never widen `Paired` to `Mutate`, which also
reaches `PromptColumn`, merges and deletes, and a filed ticket starts nothing by itself. Every
side of the control socket drops a peer on a frame it cannot parse, so mail frames go only where
they were asked for: a host sends `Collect` after `ControlMail` answered, and the relay sends
`Mail` only then.
