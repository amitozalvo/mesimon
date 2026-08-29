# mesimon

**mesimon** (Hebrew משימון, "the task instrument" — pronounced me-si-**MON**) is a terminal kanban
board that orchestrates many coding-agent sessions: like Kubernetes is to containers, mesimon is to
Claude Code (and other agent CLIs). Tickets outlive sessions; columns carry policy; a per-repo
daemon keeps everything alive when the TUI closes.

**Status: pre-v0.1, under active development.** Nothing here is usable yet.

## Three promises

1. **A strict write allowlist.** mesimon writes only to `.mesimon/`, `$GIT_DIR/info/exclude`, git
   worktrees and branches it created, and its own state dir under `~/.local/state/mesimon/`.
   Never your shell rc, your git config, your `~/.claude/`, or your tmux config.
2. **No config mutation.** `mesimon doctor` diagnoses and prints copy-pasteable fixes. It has no
   `--fix`.
3. **Zero token injection.** mesimon adds, removes, and reorders exactly zero tokens of what any
   model receives, by default. Anything that would change model input is per-column, opt-in, and
   authored by you.

## Layout

- `docs/` — the research corpus and decision record. Start at `docs/00-DECISIONS.md`
  (binding, amended three times — read the amendment blocks first) and `docs/STALE-MAP.md`.
- `crates/` — the Rust workspace.
- `team/` — reserved for the future source-available team tier (see `docs/00-DECISIONS.md` D3a).

## License

Apache-2.0. See `LICENSE`, `NOTICE`, and `TRADEMARK.md`.
