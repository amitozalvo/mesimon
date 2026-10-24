# Release watch

Run `python3 -B ci/release-watch.py` when either installed harness changes. Read the
Markdown digest, then run it with `--mark-read` to acknowledge the installed versions.
The crown files the research ticket; this script only reads upstream and prints.
It starts no agent session, installs nothing, and changes no harness configuration.

`--mark-read` is the only persistent write:
`~/.local/state/mesimon/release-watch/release-watch.json` holds each provider's version and `read_at` date, plus the filename of a SHA-256-addressed
snapshot in that same state directory. That JSON snapshot contains the exact
generated TypeScript files and published API references. The snapshot is written before the cursor is atomically
replaced. Old snapshots may be removed after no checkpoint refers to them. Keep
the cursor and captures local; never commit upstream declarations or documentation.
Ordinary runs generate types in a temporary directory that is removed on exit. A failed fetch, missing version in both sources,
downgrade, or version change during capture fails without advancing the cursor.
Claude versions found only in the changelog or only in GitHub releases print their
own sections, with the missing source identified.

The initial Codex baseline is the 751-file default `generate-ts` output from
T-934, captured with 0.164.0 on 2026-10-24. A fresh generation on this ticket was
byte-for-byte identical. Generation intentionally uses the same default surface,
without `--experimental`; this does not claim to cover experimental-only RPCs.
Claude starts from the 2.1.309 measurement; the first review covers 2.1.310.

The upstream sources are [Claude's changelog](https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md),
[Claude releases](https://github.com/anthropics/claude-code/releases),
[Codex releases](https://github.com/openai/codex/releases),
[Claude mod declarations](https://github.com/anthropics/claude-code/blob/main/mods/types/claude-code.d.ts),
[Codex plugins](https://developers.openai.com/plugins/build/plugins), and
[Codex hooks](https://learn.chatgpt.com/docs/hooks). Captures retain upstream text
and generated headers. Codex is Apache-2.0; upstream references remain their authors' work.
The release list is paginated through the baseline tag; stable installs exclude
prereleases, preview installs include them. GitHub's unauthenticated rate limit may
require retrying later; incomplete reads are errors, not empty digests.

Published API references are moving documents, not installed-version evidence.
In the initial capture Claude's published types still say 2.1.277. The first ticket
note separately compares the installed 2.1.310 declarations with T-934's 2.1.309
capture. Never infer a removed installed API from that older public document.
A keyword match is a research lead; review the **Other** section too.

`python3 -B ci/release-watch.py --dry-run` reads only `fixtures/input.json` and
cannot mark anything read. The fixture is deliberately synthetic, independent of
installed binaries, dates, saved checkpoints, and network access. Run the offline
unit suite with `python3 -B ci/release_watch_test.py`.

`--claude-decls` runs a separate, network-free installed-binary comparison (Python
3.14's `compression.zstd`). After reviewing, add `--mark-read` to save its text,
version, date and hash in the local state's `claude-decls.json`, independently of
release reads. `--claude-decls --dry-run` uses synthetic declarations; an unsupported
binary, oversized frame or changed version fails without replacing the baseline.
