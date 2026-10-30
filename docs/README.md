# docs/

For people using mesimon:

- [`USING.md`](USING.md) — the board, settings, keyboard layouts, pictures in notes, what your
  agents can see, updating, stopping everything, and investigating an agent's state.
- [`PROMISES.md`](PROMISES.md) — the three promises in full, and every path mesimon writes.
- [`REMOTE-CONTROL.md`](REMOTE-CONTROL.md) — the Remote Control browser preview.

For people working on mesimon:

- [`../AGENTS.md`](../AGENTS.md) — the shared development and verification contract.
- [`DEVELOPMENT.md`](DEVELOPMENT.md) — implementation details and measured traps; consult by area.
- `STALE-MAP.md` — the historical design record: search for relevant decisions, and append
  consequential changes briefly. Existing decisions can be reevaluated against current evidence.
- `ARCHITECTURE.md` — the long-form implementation narrative. Its milestone story lags the code;
  compare it with current code and intended product behavior.
- `state-scenarios/` — recorded agent-state scenarios replayed by the daemon's state tests
  (`crates/mesimon-daemon/src/state_replay.rs`).
- `claude-compatibility.json`, `codex-compatibility.json` — the agent compatibility manifests
  compiled into the binary; `mesimon state` reports which one it ran against.
- `release-notes/` — the writing guide for `CHANGELOG.md` entries.

The pre-code research corpus this directory once held — the numbered planning documents, the
proposals, the spikes and the agent-state research — is not part of the public repository. The
`07 §4.2`-style citations in the source and in `STALE-MAP.md` point into it; they are
provenance, not obligation.

## The repository

- `crates/` — the Rust workspace.
- `docs/` — current product commitments and historical implementation evidence.
- `web/mesophon` — the Remote Control browser client.
- The paid Teams relay is a separate, private repository; `crates/mesimon-team` is its Apache
  client.
