# mesimon — pre-planning brief

A terminal UI that orchestrates many coding-agent sessions behind a kanban board, backed by a
daemon. **19 documents, ~282,000 words.** This file is the front door: what each document decides,
where to start, what to distrust, and what only the author can answer.

> **Demoted 2026-08-31 — this corpus is research, not authority.** It was written before any
> code existed, and the code has since overtaken it. The code and its tests are the spec;
> `STALE-MAP.md` is the design record. Read everything below for its measurements and reasoning,
> never for its conclusions — starting with the next paragraph, which describes an authority
> ladder that no longer applies.

**`00-DECISIONS.md` is binding, and it has been amended TWICE since the 18 section documents were
written.** Read its two amendment blocks first — ***Corrections to earlier decisions from the
synthesis pass*** and ***Second-pass corrections*** — because **both override anything above them,
including D0–D32**, and **no section document knows about `D32` (multi-principal boards) or about
the second-pass block at all.** Where a section disagrees with `00`, `00` wins. Where `00` is
silent, the section that *owns* the topic wins:

`02` daemon and wire protocol · `04` keybindings and modes · `06` glyphs, colours, palette ·
`07` board layout, card anatomy, screens · `11` the session state enum · `12` worktree placement
and git cost · `13` the `.mesimon/` tree · `14` performance numbers · `15` MCP, permissions and
the threat model.

> **Update 2026-08-29 (planning session):** a third amendment block — **D34** — now exists in
> `00-DECISIONS.md` and settles `[ASK] 6` (tmux backend for v0.1, normative architecture in
> **`19-tmux-backend-v01.md`**), adoption-in-v0.1, digits, Inbox transport, undo, search, the
> review column, multi-project, and the not-committed `.mesimon/`. **`STALE-MAP.md`** maps every
> amendment (D33 + D34 + second-pass) to the section text it supersedes — check it before trusting
> any section below.

---

## Read this first — a planning agent's path

| # | read | why this one, in this order |
|---|---|---|
| 1 | **`00-DECISIONS.md`** | The only binding file: **D0–D32**, two amendment blocks, 23 `[ASK]`s. Nothing else makes sense without D0 (every number came off one machine), D6 (the "no magic" checklist) and D28 (fixed vs configurable). **Start at the amendment blocks near the end, then read upward.** |
| 2 | **`01-product-frame.md`** | The competitive field, why the funded tier of this category died in 2026, and the v0.1 cut with a phase plan. Tells you what *not* to build. Also where the corrected D2 measurement lives. **Predates D32** — its moat argument is now D32a's, one level deeper. |
| 3 | **`17-stack-evaluation.md`** | Makes D4 (Rust + ratatui + `alacritty_terminal`) auditable instead of asserted, and lists the spikes that must run **before** product code. |
| 4 | **`18-failure-catalog.md`** | 76 merged blockers and 84 abnormal states, each assigned to exactly one surface. The shape of the work nobody estimates. Read before believing any schedule. |
| 5 | **`02-architecture.md`** | Process topology, wire protocol, and why one PTY master per supervisor is forced rather than chosen. Everything from `03` onward assumes it. **D32c invariants 1–4 land here** and are not yet written into it. |

After those five, read by need: UI → `07`, `06`, `04`. Agent plumbing → `09`, `11`, `10`.
Data and git → `13`, `12`. Shipping → `16`. Anything team-facing → **D32**, then `15`.

---

## The 19 documents

| # | decides | read it if you are | D32 lands here |
|---|---|---|---|
| **00** | Every binding decision **D0–D32**, both amendment blocks, the 23 `[ASK]`s | **everyone, first, in full** | **defines it** |
| **01** | Prior art, positioning, scope, the v0.1/v0.2 cut, phase plan | scoping, cutting, arguing about what ships | D32a restates the moat |
| **02** | Daemon topology, IPC, wire protocol, session lifecycle, multi-client — **owns the daemon and the protocol** | building the daemon or any client | **yes** — `principal` in the command envelope, `authorize()` on every mutation path, no ambient transport authority (D32c 1–3) |
| **03** | The embedded VT: modes, query answers, encoders, what is deliberately not implemented | touching `alacritty_terminal` or any byte reaching a PTY | — |
| **04** | The input floor and the complete keymap — **owns keybindings and modes** | binding any key, anywhere | — |
| **05** | Renderer correctness: cell buffer, terminal restore, width, colour tiers | writing the ratatui backend or debugging a shear | — |
| **06** | Themes, palette, contrast, glyph tiers — **owns glyphs, colours, palette** | choosing any colour or codepoint | — |
| **07** | Board arithmetic, card anatomy, MOVE, the inbox — **owns layout and screens** | drawing anything the user looks at | untrusted-origin ticket text needs a card treatment |
| **08** | The review/disposition screen: hunks, accept, selective checkout, land | building the second screen (gated on `[ASK]` 7) | — |
| **09** | The Claude Code surface: argv, env allowlist, hooks, transcripts, postures | integrating the flagship agent | — |
| **10** | The adapter abstraction, the capability model, the three-stage probe | adding any second vendor | — |
| **11** | Attention detection and the state machine — **owns the session state enum** | deciding what "needs you" means | — |
| **12** | Worktrees and git cost — **owns worktree placement and git cost** | provisioning, tearing down, costing a `git status` | — |
| **13** | The `.mesimon/` tree, schemas, undo, activity log, search — **owns the tree** | writing any file to disk | **yes** — `origin` + `untrusted_origin` on every ticket and text field, `subject` on the consent ledger (D32c 5, 7) |
| **14** | Budgets as CI acceptance criteria — **owns performance numbers** | quoting any latency, RSS or CPU figure | — |
| **15** | MCP surface, permission tiers, consent ledger, threat model — **owns MCP, permissions, the threat model** | exposing anything to an agent | **yes** — principal in the credential binding (4), cross-principal injection, the read+propose-only v0.2 ceiling, instant revocation (D32e) |
| **16** | Config format, layering, validators, testing, packaging, distribution | writing a schema, a test, or a release | **yes** — column permissions keyed by principal *class*, per-principal budgets and rate limits (D32c 6, 8) |
| **17** | Stack scoring, rejected emulators, the pre-code spike list | choosing or defending the stack | — |
| **18** | 76 blockers, 84 abnormal states, the correctness checklist | estimating, or about to declare something done | — |

---

## State of this brief

- **`00-DECISIONS.md` has been amended twice since the sections were written, and the sections do
  not know it.** *Corrections to earlier decisions from the synthesis pass* and *Second-pass
  corrections* **override every decision above them**. D32 (multi-principal boards) was added in the
  same pass and **no section document references it.** Treat any section that contradicts either
  block as stale, not as a conflict to adjudicate.
- **It is research-derived, not implementation-derived.** A 37-agent sweep plus adversarial,
  amendment and cross-document passes. No line of mesimon exists.
- **Every version number and API claim needs re-verification at implementation time.** The
  adversarial pass caught fabricated crate versions in two dimensions.
- **~120 hard numbers were measured on one machine** — Darwin 25.6 arm64, Claude Code 2.1.250,
  `LANG=he_IL.UTF-8`, no Rust toolchain. The PTY read cap, the ptmx ceiling, timer coalescing, the
  fd limits and every git timing are Darwin facts. **Re-measure on Linux before freezing the event
  loop.** Numbers are marked `[M]` measured, `[E]` estimate, `[V]` verified, `[U]` unverified — an
  unmarked number is a bug.
- **The author is not a representative user** (D0.3). Every default must be tested against a machine
  with an empty `~/.claude/`.
- **Two measurement conflicts are open, not closed** (`00`, *Open measurement conflicts*): RSS per
  session is 104 MiB or 164–217 MiB depending on which run you believe, and idle CPU is quoted as a
  point value where it is a 2.7× range. The first sets the product's headline concurrency number.
- **Validators V18–V21 / `04` rules 12–15 have never been run**, and `04` rule 3a is newer still.
  Do that before freezing the keymap.

---

## Open questions only the author can close

The `[ASK]` table in `00-DECISIONS.md` **runs to 23 items: 3 settled, 20 open.** Ordered here by how
early they block; numbers are `00`'s.

| `[ASK]` | question | why it cannot be defaulted |
|---|---|---|
| **3** | **Platform matrix** — macOS-first / macOS+Linux / Windows | Decides IPC, PTY layer (ConPTY), fd assumptions, atomic writes. Retrofitting Windows into a unix-socket daemon is a rewrite. Blocks `02`, `12`, `13`, `14`, `17` and ~15 of `18`'s blockers. |
| **19** | **D8: does the write-allowlist promise gain a clause, or do the features go?** | ×4 sections — the strongest signal in the corpus. Sparse-checkout, land-simulation, repo-wide `info/exclude` and the measured 40% `core.fsmonitor` win all write outside `.mesimon/`. The feature **is** the write; there is no middle path. |
| **21** | **D9: does runtime `state/` stay inside the repo?** | `git clean -xdf` / `git stash -u` delete the socket, lock, index and scrollback **while the daemon keeps writing to unlinked inodes**. Unmitigated. Moving it changes a tree five documents specify. |
| **20** | **D10 vs D15: may the daemon ever return `Allow` on `PermissionRequest`?** | A contradiction *inside* `00`, worked around three ways. Deny-only makes the Inbox's flagship interaction a racy digit written into a PTY; allowing it unlocks board-level approval — the single biggest missed capability, and the one default that would make the tool unsafe to recommend. |
| ~~1~~ | ~~New name~~ **SETTLED** — the project is **`mesimon`** (Hebrew משימון, "the task instrument"). Verified free on crates.io, Homebrew, npm, PyPI and RubyGems; no product collision on GitHub. Re-check immediately before first publish. |
| ~~2~~ | ~~License~~ **SETTLED** — **Apache-2.0** for the core, recorded before reading the AGPL prior art, with a source-available paid team tier planned (D3a). Open items: the paid-tier license (25) and the repo boundary, which is **not** deferrable. |
| **4** | **Product intent** — personal tool / OSS / product | The corpus quietly assumes all three. The category leader shut down in 2026 for lack of a business model. |
| **7** | **Is review/disposition in v0.1?** | `01` and `08` stake the differentiation on it. If no, v0.1 re-cuts around the ticket store plus attention alone. |
| **6** | **Own emulator vs tmux backend** | The largest single fork in the plan (`17` marks its ≈40% cost claim `[E]`). Confirm only after the D4 spikes. |
| **23** | **RSS per session: 104 MiB or 164–217 MiB?** | Both measured on one machine, so one measures a different thing. Sets the daemon's hard live-session cap — 105 vs 217 MB doubles what a 16 GB machine allows. |
| **22** | **D16 vs D20: digits — attention rail or vim count prefix?** | Cannot be both. `3j` currently jumps to rail item 3 *then* moves down one. Wanting both makes the rail a D16 amendment, not a keymap tweak. |
| **5** | **Default column template — 4 or 6** | Answered from arithmetic as **4** (`07` §2.3, `18` §5.3); 6 spends two columns on spines. Confirm — it is what `16`'s `[init]` ships. |
| **8** | **RTL/Hebrew** — LTR-only-and-documented, or budget a bidi pass | `LANG=he_IL.UTF-8`, and no TUI toolkit does bidi-aware truncation, so the author's *own* titles truncate from the wrong end on day one. Touches every truncation site and the golden fixtures from the first commit. |
| **9** | **Issue-tracker sync in scope at all?** | Market-validated and the most dangerous untreated data path: externally-authored text through a prompt template into an agent with repo write access, unattended. ~1 month. |
| **11** | **Are stages in v0.1?** | `source = "ref"` is ~a day; `source = "command"` pulls in the whole D9 trust path. Split across versions, or ship neither. |
| **12** | **Does `Space b s` stage-regrouping ship in v0.1?** | The direct answer to "what is on prod right now", and a second board layout engine. |
| **10** | **`z` vs `za` — the permanent shape** | v0.1 defers `za` and ships density at `zz`. Taste call on the most-pressed non-navigation key. |
| **16** | **D32: is the unit of sharing a board, a column, or a saved filter?** | The design implies **column** (grants are keyed to columns), but "share this one ticket" needs a per-ticket ACL the model does not have. A second granularity later means re-keying every grant. |
| **17** | **D32: default grant expiry?** | The worked example assumes 90 days. Never-expiring grants to an unsandboxed daemon are the kind still live three jobs later. |
| **18** | **D32: network binding posture** — loopback + SSH-forward, or a tailnet bind | Public binds and tunnel services are refused outright. Determines whether v0.2 has a usable story for a team without Tailscale. |

**Settled 2026-08-28, kept for the trail:** `[ASK] 13` teammates see tickets and outcomes only,
never sessions — a privacy *floor*, enforced at the authorization layer, with no config key that
lifts it (D32h). `[ASK] 14` the teammate's client is MCP, with easy configuration as a hard
requirement — which forces Streamable HTTP over stdio and gives the v0.2 daemon a network listener
(D32i). `[ASK] 15` the eight v0.1 forward-compatibility invariants of D32c are accepted and binding.

**Raised by sections, not yet numbered in `00`:** a second adapter (`codex-cli`) in v0.1, since a
Claude-only release teaches the codebase that `Session` means `ClaudeSession` (`10`) · the repo-size
and disk budget nothing in D0–D32 costs, with 40 worktrees running to multiple GB (`12`, `14`) ·
whether `git fsmonitor--daemon` may exist at all, being the only live process mesimon would leave
outside its own perimeter (`14`) · accepting ~10 engineer-days of `17`'s S1–S8 spikes before any
product code · whether `nodeterm` — already installed in the author's environment, with global
hooks, worktree groups and kanban columns — changes the scope, since its hooks co-fire inside every
mesimon session.
