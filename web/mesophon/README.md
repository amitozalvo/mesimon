# Remote Control browser

This plain ES-module client uses the M1 Wasm crypto and pairing/snapshot/preview/
prompt/receipt protocol, including main’s queued follow-ups. Build generated assets with `ci/build-mesophon.sh`.

- `connection.js`: authentication, request correlation, deadlines and reconnect.
- `identity.js`: atomic IndexedDB device identity and remembered board selection.
- `board.js`: bounded board projection, filters and list position.
- `sessions.js`: drafts, delivery receipts and reading position keyed by board,
  ticket and session. Drafts exist only in this tab and are lost on reload.
- `view.js`: list/detail DOM rendering. Host text is always text content.
- `app.js`: event wiring and application coordination.

`npm ci && npm test` runs state regressions and Chromium/WebKit UX tests using
controlled M1 replies. The latter start a loopback HTTP fixture and write ignored
screenshots to `test-results/`. Playwright's Chromium and WebKit must be installed.
The fixture deliberately replaces crypto; it does not validate the relay or Wasm.

`browser.test.js` runs only inside the supervised Rust acceptance fixture. Build
Mesimon and Wasm assets first. Set `PLAYWRIGHT_BROWSERS_PATH` to your installed
browser cache because the supervised fixture uses a private home directory. Then
use the bounded runner and disposable database:

```sh
python3 -B ci/test-run.py -- python3 -B team/relay/tests/run_postgres.py -- \
  cargo test -p mesimon-relay --test mesophon -- --ignored --test-threads=1
```

This exercises real pairing, encrypted preview, exactly-once prompt delivery,
automatic restoration and revocation over local HTTP and fixture HTTPS. The
PostgreSQL wrapper also accepts `--postgres-bin /path/to/bin` before `--` for a
private native cluster.

The UI does not infer activity timestamps or permission details. Output shows the
last received time of a periodic, bounded 50-line window. Scrolling up freezes
that window locally; Jump to latest resumes following. Reconnect queries receipts
but never replays input. The composer defaults to Queue, with an explicit Steer
choice; queued prompts offer Send now and Take back. Returned text stays bound to
its original session and cannot overwrite a newer draft silently. Explicit authenticated revocation clears protected state;
the relay's generic error cannot distinguish a sleeping host from a removed grant.

Physical-phone acceptance still requires a reachable relay with browser-trusted
HTTPS. Check pairing, software keyboard open, rotation, background/resume, reading
position, and send-once behavior on an actual iPhone/Android browser. Desktop
browser viewport tests are useful coverage but do not complete that acceptance.
