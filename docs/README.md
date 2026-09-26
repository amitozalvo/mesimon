# docs/

- `STALE-MAP.md` — the design record: what shipped, what was refuted, and why. Append a block
  when you ship something (see `CLAUDE.md`, "Source of truth").
- `ARCHITECTURE.md` — the long-form implementation narrative. Its milestone story lags the code;
  the code and its tests are the spec.
- `state-scenarios/` — recorded agent-state scenarios replayed by the daemon's state tests
  (`crates/mesimon-daemon/src/state_replay.rs`).
- `claude-compatibility.json`, `codex-compatibility.json` — the agent compatibility manifests
  compiled into the binary; `mesimon state` reports which one it ran against.
- `release-notes/` — the writing guide for `CHANGELOG.md` entries.

The pre-code research corpus this directory once held — the numbered planning documents, the
proposals, the spikes and the agent-state research — is not part of the public repository. The
`07 §4.2`-style citations in the source and in `STALE-MAP.md` point into it; they are
provenance, not obligation.
