# Security policy

mesimon's product promises are security promises: a strict write allowlist, no configuration
mutation, and zero prompt injection (README, "Three promises"; in full in
[`docs/PROMISES.md`](docs/PROMISES.md)). A way to make mesimon break any
of them — write outside the allowlist, change a config it says it never touches, or add, remove
or reorder a token of an agent's conversation — is a security bug. So is anything that lets
another user on the machine reach the agent tool socket or a pane's environment file, or that
lets an agent send a board command its tier denies.

## Reporting

Report privately through GitHub's private vulnerability reporting on this repository
(**Security → Report a vulnerability**). Do not open a public issue for something exploitable.
You will get an acknowledgement within a week. Fixes ship in the next release and are noted in
`CHANGELOG.md`. There is no bug bounty.

## Scope

In scope: the `mesimon` binary and everything it spawns — the daemon, the private tmux server,
and the `hook`, `gate` and `mcp` subcommands that run inside an agent's turn — plus the
installer (`install.sh`), the site that serves it (`mesimon.dev`) and the update path.

Out of scope: the coding agents themselves (Claude Code, Codex) and the tmux mesimon bundles on
macOS — report those upstream — and anything that requires an already-compromised user account
on the machine.

Only the latest release is supported.
