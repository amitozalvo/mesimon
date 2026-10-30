# crates/mesimon/tests

Loaded when a session works under this directory; `AGENTS.md` is the shared contract.
See `docs/DEVELOPMENT.md` (Test infrastructure) for the supervisor and fixture details.

**Test seams.** `MESIMON_CLAUDE_BIN` (stub agent binary), `MESIMON_HOOK_BIN` (hook binary for the
pane-died notify — required in e2e, because the in-process daemon's `current_exe()` is the test
binary), `MESIMON_REQUIRE_TMUX` (turns the e2e tmux skip into a hard failure — set in CI, because
a runner without tmux otherwise reports a green suite that ran almost nothing), `MESIMON_CI`
(relaxes two wall-clock budgets in `hook_e2e`), `MESIMON_NO_DAEMON_RESTART`, `MESIMON_FAKE_BUILD`
(manufacture a stale or newer build in `Hello`), `MESIMON_DAEMON_BIN` (what `spawn_detached`
respawns; required by `restart_skew_e2e`, the only test driving the real `Client::connect`),
`MESIMON_DAEMON_EXE` (the file the daemon stamps at start and re-reads for `list_board`'s
`daemon`, T-941, so an e2e moves a stand-in and never the shared binary),
`MESIMON_CLAUDE_HOME` (census root for fabricated `~/.claude` trees), `MESIMON_COMPOSER_WAIT_MS`
(how long launch words wait for Claude's composer), `MESIMON_SLEEP_MIN_AGE_MS`,
`MESIMON_LINGER_MS` (how long a crown-started worker sits idle with background tasks before the
crown is told, T-599), `MESIMON_ASYNC_HOOK_CAP_MS` (the ten-minute cap on a held Stop's
unaccounted child processes and rewake grace, T-1044), `MESIMON_PARK_STALE_MS` (the park clock that demotes a silent
`Idle{Background}`, ten minutes, T-944), `MESIMON_PERSON_IDLE_MS` (how long after tmux last
recorded activity from a client attached to the crown's pane it holds the crown's wake, ten
minutes, T-945),
`MESIMON_CODEX_CLEANUP_STALE_MS` (how long an unconfirmed Codex cleanup may own a live ticket's
checkout before the sweep goes looking for its runtime), `MESIMON_WORKTREE_INIT_MS` (how long
the repo's `.mesimon-worktree-init.sh` may run in a fresh worktree before it is killed, T-614),
`MESIMON_PANE_QUIET_MS`, `MESIMON_NO_UPDATE_CHECK`, `MESIMON_UPDATE_CHECK`,
`MESIMON_SERVER_GUARD_TICKS`, `MESIMON_WT_REFRESH_TICKS` (the slow bucket: worktree flags, merge
train, CLAUDE.md sample, checkout git sample), `MESIMON_NO_TAG_SEED`,
`MESIMON_TICKET_SHELLS`, `MESIMON_IDLE_SNAKE_MS` (how long the board waits for a key before its
snake comes out, T-870; an hour), `MESIMON_UPDATE_GOLDEN`, `MESIMON_MOD_DIR` (T-573's research seam: a Claude
Code mod folder loaded with `--plugin-dir` in place of the laid one; the spike mod and its driver
live in `crates/mesimon-daemon/mod-spike/`, measured in STALE-MAP, shipped to nobody),
`MESIMON_MOD_BRIDGE_WAIT_MS` (how long a mod launch's words wait for its bridge before the paste
road, T-575), `MESIMON_CLAUDE_ROAD` (`hooks|mod|auto`, a test seam over the `auto` every launch asks; `TestFixture` always
sets it, `hooks` unless the test process has `MESIMON_TEST_ROAD=mod`, the mod road's pass that
wraps every stub with `tests/common/fake_claude_mod.py` and twins every `hook_send`),
`MESIMON_REQUIRE_CLAUDE` (turns `mod_plugin`'s skip without `claude` into a failure; the release
sets it; a mods-off refusal stays a named skip), `MESIMON_RIG_NO_FLAGS` (the rig's `--flags-off`:
every launch gets `DISABLE_GROWTHBOOK=1`; never a user's), `MESIMON_TEST_FAIL_ENTER` (a file
path: while it exists, a pane's Enter fails after its paste went in, T-938),
`MESIMON_TEST_LAUNCH_BARRIER` (a file path: while it exists, a launch's paste edge waits, at most
10 s, after writing `<path>.reached`, T-987), `MESIMON_SAMPLE_DELAY_MS`
(how long a `sample_agent` read waits before its capture and again before it lands, T-973),
`MESIMON_RECHECK_MS` (how long the restart's look waits for a worker the crown's ledger names to
settle before letting it go, a minute, T-1004), `MESIMON_RECORD_TRACE` (a file path: the daemon
appends every frame its hook socket takes and every line a session's transcript gains, stamped,
T-1014; `ci/rig.py --record` sets it and cuts each case into a fixture under `traces/`, which
`common::trace` replays).

The e2e pattern is an in-process daemon thread + real tmux + the real built binary via
`env!("CARGO_BIN_EXE_mesimon")`, which is only available in `crates/mesimon/tests/`.

## Traps that were measured

- **macOS 14 answers `lseek` on a tty with 0, so bash 3.2's `read` (its `/bin/sh`) loses
  lines** (T-1019): it reads the tty in chunks and drops every line after the first that one
  `read()` brought, so a stub's `while read` loses words that arrive in one burst. A stub reads
  through `cat |` (bash reads a pipe a byte at a time); macOS 27 and Linux say `Illegal seek`.
  Ubuntu's `/bin/sh` is dash, which has no `read -t`.
