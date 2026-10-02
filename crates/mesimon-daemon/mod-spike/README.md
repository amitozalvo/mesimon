# mesimon-spike: T-573's research mod

A throwaway Claude Code mod (a plugin of function hooks, Claude Code ≥ 2.1.287)
that measures, on the real Claude Code, whether a mod can replace the ad hocs
mesimon built to integrate with it: the generated hook set and `mesimon hook`,
the paste road, the dialog scraping, the permission bridge, `mesimon gate`,
the plan accept, the MCP shim and the cost reader. The measurements and the
go/no-go are in `docs/STALE-MAP.md` under T-573; the migration plan is a note
on the ticket. **Nothing here ships**: the daemon loads it only when
`MESIMON_MOD_DIR` names this folder (a copy under the state dir), and nothing
else does.

Files:

- `.claude-plugin/plugin.json`, `hooks/hooks.json`, `hooks/register.ts` — the
  mod. `claude plugin validate .` reports what it hooks, calls and reads.
- `hooks/register.test.ts` — `claude plugin test .`, the hooks' shapes against
  the engine's own `$`.
- `wait.py` — the stand-in for `mesimon approve`'s wait (row 4): run by the mod inside
  `$.process.run` while a permission dialog is up, it waits for a decision file and prints it.
- `bridge.py` — the daemon→mod direction of the bridge: spawned once per
  session by the mod, it tails a spool directory and prints each command as a
  line the mod reads as a stream. (The mod→daemon direction is `$.process.run`
  of the real `mesimon hook` per event; `$.process.spawn`'s stdin is one string
  in 2.1.287.)
- `drive.py` — the measurement driver: each scenario runs the real `claude`
  (Haiku, `--setting-sources ""`, a scratch cwd, the trust dialog answered by
  key) in a private tmux server, sends prompts the way mesimon does, drops
  commands into the spool and waits on the mod's event log.
- `report.py` — prints a session's event log as a timeline.

To measure again (cents, on the person's own Claude login):

```sh
P=~/.local/state/mesimon/<proj16>/mod-spike
rsync -a --exclude .claude-plugin/types --exclude tsconfig.json ./ "$P"/
python3 drive.py run --mod "$P" --out /tmp/spike-out coverage submit submit_midturn ask permission permit gate plan_result plan_allow plan_native tools relay load
python3 report.py /tmp/spike-out/ask/main/log
```

To load it on a board for a look (the mod observes into `<state>/mod-log/<KEY>/`
and refuses writes under `.mesimon`; it submits nothing and relays nothing
unless its other `MESIMON_MOD_*` variables are set):

```sh
MESIMON_MOD_DIR="$P" cargo run -- daemon --repo <repo>
```
