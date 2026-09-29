# Remote Control browser

This plain ES-module client uses the M1 Wasm crypto and pairing/snapshot/preview/
prompt/receipt protocol, including main’s queued follow-ups. Build generated assets with `ci/build-mesophon.sh`.
There is no build step for the page itself: Preact and htm are vendored as plain
modules (`vendor/README.md`), fonts are self-hosted (`fonts/`), and every style
is in `style.css`, because the relay's CSP allows no inline style or script.
The relay's image clones this repository from GitHub; to try unpushed changes in
a local relay, build its image from this checkout with `ci/build-local-image.sh`
in `mesimon-relay`.

- `connection.js`: authentication, request correlation, deadlines and reconnect.
- `identity.js`: atomic IndexedDB device identity, remembered board selection,
  and per grant the last board it saw (titles, columns, tags and agent states
  only) and the tickets this browser sent it.
- `board.js`: bounded board projection, Now's groups, filters and list position.
- `sessions.js`: drafts, delivery receipts and reading position keyed by board,
  ticket and session. Drafts exist only in this tab and are lost on reload.
- `sent.js`: the Sent list, per board: a filed ticket's status (a clock while
  sealed in this browser, one tick at the relay, two once landed; unsent,
  unknown or refused), its key once landed, and what survives a reload.
- `mailbox.js`: the browser's own relay socket for the mailbox, apart from the
  host channel, open whether or not the host is reachable.
- `store.js`: application state and every action on it; the view renders from it.
- `shell.js`, `lists.js`, `detail.js`, `dialogs.js`, `compose.js`: the view
  (pairing, sidebar, Now, Board and Sent, the ticket, permission/question/plan
  cards, the New ticket sheet and Sent's bar). Host text is always a text node.
- `shin.js`, `icons.js`: the mascot from `assets/mascot/shin.txt` and inline icons.
- `app.js`: boot and window-level events.

`npm ci && npm test` runs state regressions and Chromium/WebKit UX tests using
controlled M1/M2 replies. The latter start a loopback HTTP fixture and write ignored
screenshots to `test-results/`. Playwright's Chromium and WebKit must be installed.
The fixture deliberately replaces crypto; it does not validate the relay or Wasm.

`browser.test.js` runs only inside the supervised Rust acceptance fixture, which
lives with the relay in its own repository (`mesimon-relay`, checked out beside
this one). Build Mesimon and Wasm assets first. Set `PLAYWRIGHT_BROWSERS_PATH` to
your installed browser cache because the supervised fixture uses a private home
directory. Then, from the relay repository, use the bounded runner and disposable
database:

```sh
python3 -B ../mesimon/ci/test-run.py -- python3 -B relay/tests/run_postgres.py -- \
  cargo test -p mesimon-relay --test mesophon -- --ignored --test-threads=1
```

This exercises real pairing, encrypted preview, exactly-once prompt delivery,
automatic restoration and revocation over local HTTP and fixture HTTPS. The
PostgreSQL wrapper also accepts `--postgres-bin /path/to/bin` before `--` for a
private native cluster.

The UI does not infer activity timestamps or permission details. Now groups
agents by the host's own state word: needs you, working, idle. A needs-you card
answers a permission or a single-choice question in place; anything else opens
the ticket. Output shows the last received time of a periodic, bounded 50-line
window. Scrolling up freezes that window locally; Jump to latest resumes
following. Reconnect queries receipts but never replays input. The composer
defaults to Queue, with an explicit Steer choice; queued prompts offer Send now
and Take back. Returned text stays bound to its original session and cannot
overwrite a newer draft silently. Explicit authenticated revocation clears
protected state, including the remembered board; the relay's generic error
cannot distinguish a sleeping host from a removed grant.

While the host is out of reach the page says which hop failed (this browser,
the relay, or the terminal) and keeps showing the last board, marked as not
live, with actions disabled. A cold start shows the board remembered from the
last live snapshot.

Physical-phone acceptance still requires a reachable relay with browser-trusted
HTTPS. Check pairing, software keyboard open, rotation, background/resume, reading
position, and send-once behavior on an actual iPhone/Android browser. Desktop
browser viewport tests are useful coverage but do not complete that acceptance.

M2 adds permission/question/plan cards and `awareness.js` for connected-browser
alerts. Payloads come from the daemon and render as text, never HTML. Only
single-choice/single-text questions and the measured plan menu have remote key
mappings; other shapes require a local answer. The daemon checks each selection
before sending the next key. The handshake advertises M2 features so new browsers
can continue using older M1 hosts without sending unknown operations.

Awareness travels through the existing authenticated encrypted channel. Ordinary
alerts are suppressed while any paired browser foregrounds the ticket; presence
expires after 15 seconds without renewal. In-page alerts work without system
notification support; each links to its originating ticket. Browser notification permission is
requested only by the alerts button. Closed-browser Web Push is not implemented.

Tickets from the browser use the `create` op, advertised as the `create`
feature so a browser never sends it to an older host. The daemon files the
ticket through its one writer as `Action::FileTicket`, into a column the board
has and with tags it already has, and never starts an agent on it. The answer
is a `created` receipt with the new key; a lost answer is recovered with
`status` on reconnect, never by sending the ticket again. The snapshot carries
what the sheet offers: `default_column`, `column_descriptions` and
`allowed_tags` (name, group and the TUI's tint index).

While the host is away, tickets go through the relay's mailbox. The Wasm seals
each one for the pinned host (`Browser.mail`: a fresh key wrapped to the host
and the body under it), the page keeps the envelope in IndexedDB until the
relay has it, and the mailbox socket deposits it (`deposit` → `deposited`, the
one tick). Unsend and Edit use `withdraw`, which the relay grants only before
the host has been handed the envelope. The host collects its mail once it is
back, files each envelope at most once, and answers with a receipt sealed to
this browser, which `Browser.receipt` opens with the pinned host key; a
receipt that does not open is ignored, whoever sent it. `sync` asks what
became of the envelopes at the relay after a reload or a reconnect. A host that
advertises `mailbox` takes every ticket this way, live or not.
