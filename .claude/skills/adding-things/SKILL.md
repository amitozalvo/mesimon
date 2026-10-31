---
name: adding-things
description: The recipes for adding to mesimon — the complete set of places a change touches for a wire command, a key binding, a column setting, a launch flag, a theme, a machine pref (a prefs.json key), something the mouse can pick, a card or ticket-page visual, the shin mascot, or an e2e test. Load before adding any of these.
---

# Adding things: the recipes

From `AGENTS.md`, whose rules still apply.

Each list is the complete set of places a change touches. The compiler finds most of them (the
exhaustive matches are the mechanism); this is the rest. **If a step is missing here, add it
here.**

**A wire command** (something the TUI or an agent asks the daemon to do):
1. `core/src/command.rs`: the `Command` variant — its serde `snake_case` name is its wire name
   and its activity-feed name — then `Command::meta()`: read or mutate, logged or not, which
   ticket. Exhaustive; a new variant does not compile until classified.
2. `core/src/mcp.rs::agent_allows`: may an agent send it? Exhaustive, no `_` arm. Almost always no.
3. `daemon/src/server.rs::handle`: the dispatch arm, calling a `Daemon` method. Only
   `place_ticket` moves a ticket.
4. The sender: a `Verb` in the TUI (next recipe) or a tool in `mesimon/src/mcp.rs`.
5. An e2e if it touches a pane or the disk.

**A key binding:**
1. `core/src/keymap.rs`: a `Binding` in that scope's list (keys, `avail` over `Ctx`, `hint`,
   `mutates`) and a `Verb`. A fact `Ctx` lacks is a field on it — `Ctx` derives `Default`, so
   that is the struct plus `App::ctx()` in `tui/src/app.rs`, nothing else. A new `Scope` also
   goes in `Scope::ALL`, in `parent()`, `word()`, `bindings()` and in `scope_list_is_complete`'s
   `index()`, or it is validated by nothing; a BARRIER scope (parent `None`, every key its own)
   joins the exemption list in `q_pops_and_help_is_everywhere` — `?` is text in a text field.
2. `App::dispatch` in `tui/src/app.rs`: the arm. The match is exhaustive, so the compiler
   points at it.
3. `cargo ut` runs the keymap validators. A hint that shows on the board changes the board
   goldens — remint, then review the diff by eye.

**A column setting:**
1. The field on `core/src/board.rs::ColumnSettings` — `#[serde(default)]`,
   `skip_serializing_if` its default. It reaches disk inside the column's `[[columns]]` table
   and the TUI on the snapshot for free. If its default is today's behaviour, no schema bump;
   if an older build *dropping* it would widen what a spawn or an agent gets, bump
   `COLUMNS_SCHEMA` (`daemon/src/store.rs`; the doctrine is on the constant).
2. `board::template_settings` if the four template columns should carry it — the one place a
   column name is read as a literal.
3. The reader (`automove`, `train::plan`, `place_ticket`'s gate, `agents/claude.rs::flags`,
   `agent_tools_for`, `reclaim_columns`) reads the ticket's column through `Board::column`,
   never a name. `Board::set_column_settings` validates cross-column references.
4. `ColumnSettings::summary` (doctor's `columns` line, the dialog's details) and a `MenuItem` in
   `keymap::COLUMN_ITEMS` with its `Ctx::col_*` word, filled in `App::ctx()`.

**A launch flag** (a column's, a tier's — anything a spawn and a wake must both carry): a
`LaunchContext` field filled in `Daemon::launch_context`, then `agents/claude.rs::flags` AND
its `resume` owned-pair list (a wake replays the stored argv), and `agents/codex/mod.rs::prepare`.
A tier is `core/src/tier.rs`; which one a ticket launches on is `tier::Book`, never a field read.

**A theme:** a `static` `Palette` in `tui/src/theme.rs` (every profile, hand-authored — never
nearest-matched), a `Flavor` variant and its arms in `palette`/`name`/`blurb` (the compiler
finds them), a `Kind` for the law or a fourth clause argued in `test_chroma_law`. `cargo ut`
runs the laws over `Flavor::ALL`; a tag ring must skip the accent's hue band and the ground's.
The picker, the prefs file and the goldens need nothing — rows come from `Flavor::ALL` and
goldens are colourless.

**A machine pref** (a `prefs.json` key; T-361 lets a board override most of them):
1. `core/src/prefs.rs::PrefKey`: the variant, its `name()` (the JSON key), `label()`, and
   whether `board_overridable` — machine-only is for a key about the terminal or the person (and
   the machine-only list in `key_names_are_unique_and_the_terminal_keys_are_machine_only`); a
   named value joins `is_bool`'s exclusions.
2. `tui/src/prefs.rs`: the `Prefs` field, `load`, `body` (a named value gets the foreign-value
   clause), `word`, and `overlay` if a board may set it, else `BoardPrefs::is_set`'s `false`
   arm; a round-trip test.
3. The `Ctx` field and `App::ctx()`; the `MenuItem` in `SETTINGS_ITEMS`/`NOTIFY_ITEMS` (its
   `label` is the name alone), its place in `settings_items` and `SettingsSection::for_verb`,
   and its `keymap::value` arm — what the Settings page draws beside the name; the `Verb` and
   its arm through `set_pref`. `keymap::pref_key` names the row's key, and board scope then
   cycles it with no further code.
4. The push, if anything outside the TUI consumes it: a `push_*`/`reconcile_*` pair for the
   daemon, `From<&Prefs>` in `notifier.rs` for the notifier, `drive_caffeine` for power. A value
   that must hold with no board open (a kill switch) is read by the daemon from both files
   instead, at the moment it decides (`modroad::read_setting`, T-574).
5. A `doctor` line via `load_home()`.
**`App::prefs` is the resolved view** (machine under this board's overrides): a test seeds it
with `seed_pref`, never by assignment, and `save_prefs` writes `machine_prefs`.
**A row's `detail` says one fact the label lacks** (what the value means, what it costs, or why
the row is inert), never what Enter does and never the label again; empty is fine. Three
spellings name Enter: `∙ enter again confirms`, `{error} ∙ enter tries again`, `∙ enter copies
it` (`keymap::HINT_ENTER_WORDS`; `a_hint_never_explains_enter` reads the source, T-677).

**Something the mouse can pick** (T-716): record a `mouse::Target` over its rect with
`app.hits.borrow_mut().record` as it is drawn, and a `point_*` arm in `App::click` that presses
the key a keyboard would (`App::press`), never a second road. A key hint needs nothing: every
hint `chrome::hint_spans` spells is clickable wherever it is left whole on the frame.

**A card or ticket-page visual:** `tui/src/ui/card.rs` / `ticket.rs` / `tags.rs`, plus a golden
in `tui/src/ui/tests.rs`. The L1 law tests (`test_no_banned_sgr`, `test_no_drawn_structure`) and
the colour laws in `theme.rs` run in `cargo ut` and say what is wrong.

**The shin (the mascot):** pixels in `assets/mascot/shin.txt`, the engine in `tui/src/creature.rs`,
inks from `Theme::creature_ink`. A new `SessionState` does not compile until `Anim::of` gives it
an animation. The notification PNGs and the installer's welcome are the engine's goldens:
`MESIMON_UPDATE_GOLDEN=1 cargo test -p mesimon-tui creature`.

**An e2e test** (`crates/mesimon/tests/<name>_e2e.rs`): `mod common; use common::*;`, then
`let Some(h) = Harness::boot("name", Some(STUB)) else { return };` — the daemon, the private
tmux, the seams and the teardown are the harness's; a shell stub paints Claude's composer
first (`claude_stub`; `Harness::boot_bare` keeps it as written); `h.client("name")` speaks the
wire, `hook_send` runs the real hook binary, `wait_until` polls. Any other `MESIMON_*` seam
rides `Harness::boot_with_env`. `prompt_e2e.rs` is the exemplar.
