//! Agent tiers in the TUI (T-443): `^n` on a ticket and in a field, the
//! words the board says a tier in, and the Settings row and the two dialogs
//! that edit the machine's tiers and a board's.
//!
//! Every answer about which tier a ticket runs on comes from
//! `mesimon_core::tier::Book` over the snapshot's two layers — the same
//! resolver the daemon launches with — so the card never names a tier the
//! launch would not use.

use super::*;
use mesimon_core::board::{AgentProvider, SessionRecord};
use mesimon_core::text::scrub_cells;
use mesimon_core::tier::{self, Book, Effort, ModelLists, ModelStopKind, Source, Tier, TierScope};
use std::io::Read;
use std::path::PathBuf;

/// One row of the tiers list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TierRow {
    Tier(Tier, Source),
    New,
}

/// One row of a tier's page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TierField {
    Name,
    Provider,
    Model,
    Effort,
    /// When to use it, in the person's words (T-584): what the crown reads
    /// to pick a tier for a ticket it files or starts.
    Description,
    /// Delete the tier — or, for this board's own version of a machine
    /// tier, drop the version and inherit the machine's again.
    Remove,
}

/// Where Claude Code and Codex keep the model lists their pickers show
/// (T-951): Claude Code's home (`census::claude_home`) and Codex's
/// (`codex_home`). Read when a tier page opens, never in `App::new`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelHomes {
    pub claude: PathBuf,
    pub codex: PathBuf,
}

/// Codex's cache is ~400 KB; a file past this is not a list we read.
const MODEL_FILE_MAX: u64 = 2 * 1024 * 1024;
/// Claude Code keeps one catalog per account it signed in as.
const CATALOGS_MAX: usize = 16;

fn read_capped(path: &std::path::Path) -> Option<String> {
    let mut text = String::new();
    std::fs::File::open(path).ok()?.take(MODEL_FILE_MAX).read_to_string(&mut text).ok()?;
    Some(text)
}

impl ModelHomes {
    /// Both lists as the providers left them: Claude Code's account
    /// catalogs (`cache/model-catalog/*-cc.json`, 2.1.310) and Codex's
    /// `models_cache.json`. A missing or unreadable file is no list.
    pub fn read(&self) -> ModelLists {
        let dir = self.claude.join("cache").join("model-catalog");
        let catalogs: Vec<String> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with("-cc.json"))
            .take(CATALOGS_MAX)
            .filter_map(|e| read_capped(&e.path()))
            .collect();
        let catalogs: Vec<&str> = catalogs.iter().map(String::as_str).collect();
        ModelLists {
            claude: tier::claude_models(&catalogs),
            codex: read_capped(&self.codex.join("models_cache.json"))
                .and_then(|t| tier::codex_models(&t)),
        }
    }
}

/// The tier id a record launched on; a record from before tiers ran its
/// provider's built-in.
fn launched(rec: &SessionRecord) -> String {
    match (rec.tier.is_empty(), rec.kind.provider()) {
        (true, Some(p)) => tier::builtin_id(p).to_string(),
        _ => rec.tier.clone(),
    }
}

/// Between turns, the way the daemon's `session_at_rest` reads it: a stop
/// on a limit switches at once too (T-808).
fn between_turns(rec: &SessionRecord) -> bool {
    mesimon_core::quiet::at_rest(rec)
}

impl App {
    /// The two layers, for one question.
    pub(crate) fn tiers(&self) -> Book<'_> {
        Book::new(&self.machine_tiers, &self.board)
    }

    /// The layer the Settings dialogs write: `b` flips it.
    pub(crate) fn tier_scope(&self) -> TierScope {
        if self.settings_board_scope {
            TierScope::Board
        } else {
            TierScope::Machine
        }
    }

    /// Which agent `c` (and every spawn road) means on this ticket: the
    /// seat's own where it holds one, else the one the ticket's tier starts.
    pub(crate) fn agent_kind_for(&self, ticket: ulid::Ulid) -> SessionKind {
        self.tiers().seat_provider(ticket).session_kind()
    }

    pub(crate) fn tier_name(&self, id: &str) -> String {
        self.tiers().get(id).map(|t| t.name).unwrap_or_else(|| id.to_string())
    }

    // ------------------------------------------------------------ the cycle

    /// Whether `^n` has somewhere to go right now: the open field's ring, or
    /// the subject ticket's.
    pub(crate) fn tier_cycle_ctx(&self) -> bool {
        let book = self.tiers();
        match &self.mode {
            Mode::Input { purpose: InputPurpose::Create { .. }, .. }
            | Mode::Editor(Editor { purpose: EditorPurpose::Compose { .. }, .. }) => {
                book.cycle(None).len() > 1
            }
            Mode::Input {
                purpose: InputPurpose::Prompt { target: AskTarget::Ticket(t), .. },
                ..
            }
            | Mode::Editor(Editor {
                purpose: EditorPurpose::Ask { target: AskTarget::Ticket(t), .. },
                ..
            }) => book.ring(*t).len() > 1,
            Mode::Input { .. } | Mode::Editor(_) => false,
            _ => self.subject().is_some_and(|t| book.ring(t).len() > 1),
        }
    }

    /// `^n` on the board or the ticket page: the ticket's next tier, at
    /// once. The status says what the press did to the seat.
    pub(crate) fn cycle_tier(&mut self) -> Result<()> {
        // The tier as the daemon has it (T-812 review): a second press
        // before the first's board lands steps from the first's answer.
        if !self.catch_up()? {
            return Ok(());
        }
        let Some(id) = self.subject() else { return Ok(()) };
        let book = self.tiers();
        let current = book.of_ticket(id).id;
        let Some(next) = book.next_after(id, &current) else { return Ok(()) };
        let words = self.tier_press_words(id, &next);
        match self.req(Command::SetTicketTier { id, tier: Some(next.id.clone()) }) {
            Response::Err { message } => self.status = message,
            _ => {
                self.refresh_soon()?;
                self.status = words;
                self.tier_flash = Some((id, Instant::now()));
            }
        }
        Ok(())
    }

    /// What `^n` did, in the seat's words: where there is no agent yet, it
    /// starts on the tier; a parked one wakes on it; a live one switches at
    /// its idle — or already runs it.
    fn tier_press_words(&self, ticket: ulid::Ulid, to: &Tier) -> String {
        let key = self.board.ticket(ticket).map(|t| t.short_key.clone()).unwrap_or_default();
        let head = format!("{key} ∙ {}", to.name);
        let word = keymap::AGENT_WORD;
        match self.board.live_agent(ticket) {
            None => format!("{head} ∙ its {word} starts on {}", to.summary()),
            Some(rec) if rec.state == SessionState::Sleeping => {
                format!("{head} ∙ wakes on {}", to.summary())
            }
            Some(rec) if launched(rec) == to.id => format!("{head} ∙ already running on it"),
            Some(rec) if between_turns(rec) => format!("{head} ∙ switches in a moment"),
            Some(_) => format!("{head} ∙ switches when the {word} is idle"),
        }
    }

    /// `^n` in a text field: the pick steps along the field's ring, and
    /// comes back to `None` on the tier the launch would run anyway — so a
    /// full turn of the ring leaves the draft as it was.
    pub(super) fn step_field_tier(
        &mut self,
        ticket: Option<ulid::Ulid>,
        pick: &mut Option<String>,
    ) {
        let book = self.tiers();
        let (ring, base) = match ticket {
            Some(t) => (book.ring(t), book.of_ticket(t).id),
            None => (book.cycle(None), book.default_tier().id),
        };
        if ring.len() < 2 {
            return;
        }
        let at = pick.clone().unwrap_or_else(|| base.clone());
        let i = ring.iter().position(|t| t.id == at).map_or(0, |i| (i + 1) % ring.len());
        let next = ring[i].clone();
        *pick = (next.id != base).then(|| next.id.clone());
        self.status = format!("{} ∙ {}", next.name, next.summary());
    }

    /// The word a field's row wears for its pick: the tier's name, where it
    /// is not what the ticket — or a new ticket — runs on anyway.
    pub(crate) fn field_tier_word(
        &self,
        ticket: Option<ulid::Ulid>,
        pick: Option<&str>,
    ) -> Option<String> {
        let pick = pick?;
        let book = self.tiers();
        let base = match ticket {
            Some(t) => book.of_ticket(t).id,
            None => book.default_tier().id,
        };
        (pick != base).then(|| self.tier_name(pick))
    }

    /// Whether a person made any tier: until they do, the board does not
    /// spell `claude` on every ticket it shows.
    fn tiers_in_use(&self) -> bool {
        !self.tiers().custom().is_empty()
    }

    /// The open card's tier word, beside its key: where the ticket's tier
    /// is not the default, which is what every card without one runs — and
    /// for a moment after `^n`, whatever it landed on, so a stop on the
    /// default is seen without a card at rest spelling it (T-562, user:
    /// "show default for a second after clicking ^n, so it won't be verbose
    /// when not needed").
    pub(crate) fn card_tier_word(&self, ticket: ulid::Ulid) -> Option<String> {
        let book = self.tiers();
        let want = book.of_ticket(ticket);
        (want.id != book.default_tier().id || self.tier_flashing(ticket)).then_some(want.name)
    }

    /// Is `^n`'s reveal still up on this ticket's card?
    pub(crate) fn tier_flashing(&self, ticket: ulid::Ulid) -> bool {
        self.tier_flash.is_some_and(|(id, at)| id == ticket && at.elapsed() < TIER_FLASH)
    }

    /// The card's "what mesimon does next" row while a switch is owed.
    pub(crate) fn tier_owed_row(&self, ticket: ulid::Ulid) -> Option<String> {
        let rec = self.board.live_agent(ticket)?;
        let want = self.tiers().of_ticket(ticket).name;
        if rec.tier_wake {
            return Some(format!("switching to {want}"));
        }
        if !(rec.tier_owed && rec.state.has_pane()) {
            return None;
        }
        Some(if self.ticket_queued(ticket) {
            "switches ∙ then asks".to_string()
        } else {
            format!("switches to {want}")
        })
    }

    /// The ticket page's tier clause on the state row: the tier it runs,
    /// and while a switch is owed, from what to what.
    pub(crate) fn tier_clause(&self, ticket: ulid::Ulid) -> Option<String> {
        let book = self.tiers();
        let want = book.of_ticket(ticket);
        let seat = self.board.live_agent(ticket);
        if seat.is_some_and(|r| r.tier_wake) {
            return Some(format!(" ∙ switching to {}", want.name));
        }
        if let Some(rec) = seat.filter(|r| r.state.has_pane() && !r.tier.is_empty()) {
            let from = launched(rec);
            if from != want.id {
                let from = self.tier_name(&from);
                return Some(if rec.tier_owed {
                    format!(" ∙ on {from} → {} at idle", want.name)
                } else {
                    format!(" ∙ on {from} ∙ {} from its next launch", want.name)
                });
            }
        }
        (self.tiers_in_use() || want.id != book.default_tier().id)
            .then(|| format!(" ∙ on {}", want.name))
    }

    /// The empty seat's preview clause: which tier a start would run.
    pub(crate) fn seat_tier_clause(&self, ticket: ulid::Ulid) -> Option<String> {
        let book = self.tiers();
        let want = book.of_ticket(ticket);
        (self.tiers_in_use() || want.id != book.default_tier().id)
            .then(|| format!("on {}", want.name))
    }

    // ------------------------------------------------------------ Settings

    /// The Default tier row's ring in the dialog's scope: the machine's is
    /// the built-ins and its own tiers; a board's starts with `inherit`.
    fn default_ring(&self) -> (Vec<Option<Tier>>, Option<String>) {
        let book = self.tiers();
        let builtins =
            [Tier::builtin(AgentProvider::ClaudeCode), Tier::builtin(AgentProvider::Codex)];
        if self.settings_board_scope {
            let mut ring: Vec<Option<Tier>> = vec![None];
            ring.extend(builtins.into_iter().map(Some));
            ring.extend(book.custom().into_iter().map(|(t, _)| Some(t)));
            (ring, book.board_default().map(|t| t.id))
        } else {
            let mut ring: Vec<Option<Tier>> = builtins.into_iter().map(Some).collect();
            ring.extend(
                self.machine_tiers.tiers.iter().filter(|t| !t.is_builtin()).cloned().map(Some),
            );
            (ring, Some(book.machine_default().id))
        }
    }

    fn default_next(&self) -> Option<Tier> {
        let (ring, current) = self.default_ring();
        let at = ring.iter().position(|t| t.as_ref().map(|t| &t.id) == current.as_ref());
        let i = at.map_or(0, |i| (i + 1) % ring.len());
        ring[i].clone()
    }

    /// Enter on the Default tier row: the next stop of the scope's ring.
    pub(super) fn cycle_default_tier(&mut self) -> Result<()> {
        let scope = self.tier_scope();
        let next = self.default_next();
        let id = next.as_ref().map(|t| t.id.clone());
        match self.req(Command::SetDefaultTier { scope, id }) {
            Response::Err { message } => self.status = message,
            _ => {
                self.refresh()?;
                // The machine's pick under this board's own (T-751): the
                // row stays on the board's, and the status says why.
                let kept = self
                    .tiers()
                    .board_default()
                    .filter(|_| matches!(scope, TierScope::Machine))
                    .map(|t| t.name);
                self.status = match (next, kept) {
                    (Some(t), Some(kept)) => {
                        format!("machine's default tier ∙ {} ∙ this board keeps {kept}", t.name)
                    }
                    (Some(t), None) => format!(
                        "default tier ∙ {} ∙ a ticket that picked none starts on {}",
                        t.name,
                        t.summary()
                    ),
                    (None, _) => format!(
                        "this board inherits the machine's default ∙ {}",
                        self.tiers().machine_default().name
                    ),
                };
            }
        }
        Ok(())
    }

    /// Everything the tier rows and keys read off `Ctx`.
    pub(super) fn fill_tier_ctx(&self, c: &mut Ctx) {
        let book = self.tiers();
        let default = book.default_tier();
        let customs = book.custom();
        c.agent_provider = default.provider;
        c.claude_unused = default.provider != AgentProvider::ClaudeCode
            && !customs.iter().any(|(t, _)| t.provider == AgentProvider::ClaudeCode);
        c.codex_in_use = default.provider == AgentProvider::Codex
            || customs.iter().any(|(t, _)| t.provider == AgentProvider::Codex);
        c.tier_cycle = self.tier_cycle_ctx();
        // What runs on this board, in either scope (T-751): the scope says
        // what Enter edits, and the detail carries the other layer.
        c.tier_default = default.name.clone();
        c.tier_default_summary = default.summary();
        c.tier_default_here = book.board_default().is_some();
        c.tier_machine_default = book.machine_default().name;
        c.tier_names = self
            .tier_rows()
            .into_iter()
            .filter_map(|r| match r {
                TierRow::Tier(t, _) => Some(t.name),
                TierRow::New => None,
            })
            .collect::<Vec<_>>()
            .join(" ");
        match &self.mode {
            Mode::Tiers { idx, naming: None } => {
                c.tiers_enter_word = match self.tier_rows().get(*idx) {
                    Some(TierRow::Tier(..)) => "edit",
                    Some(TierRow::New) => "new tier",
                    None => "",
                };
                c.tier_can_nudge = self.tier_slot(*idx).is_some();
            }
            Mode::TierEdit { idx, field: None, armed, .. } => {
                let field = self.tier_fields().get(*idx).copied();
                let model_steps = self.model_ring().len() > 1;
                c.tier_on_step = match field {
                    Some(TierField::Provider | TierField::Effort) => true,
                    Some(TierField::Model) => model_steps,
                    _ => false,
                };
                c.tier_on_model = field == Some(TierField::Model);
                c.tier_edit_enter_word = match field {
                    Some(TierField::Name | TierField::Description) => "edit",
                    Some(TierField::Provider) => "switch",
                    Some(TierField::Model) if model_steps => "next",
                    Some(TierField::Model) => "",
                    Some(TierField::Effort) => "next",
                    Some(TierField::Remove) if *armed => "confirm",
                    Some(TierField::Remove) => match self.edited_tier().map(|(_, s)| s) {
                        Some(Source::Override) => "revert",
                        _ => "delete",
                    },
                    None => "",
                };
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------ the list

    /// The tiers list's rows in the dialog's scope, then the row that makes
    /// one: the machine's own tiers, or this board's view of them.
    pub(crate) fn tier_rows(&self) -> Vec<TierRow> {
        let mut rows: Vec<TierRow> = if self.settings_board_scope {
            self.tiers().custom().into_iter().map(|(t, s)| TierRow::Tier(t, s)).collect()
        } else {
            self.machine_tiers
                .tiers
                .iter()
                .filter(|t| !t.is_builtin())
                .map(|t| TierRow::Tier(t.clone(), Source::Machine))
                .collect()
        };
        rows.push(TierRow::New);
        rows
    }

    /// The tier on list row `idx` as its scope orders it (T-562): the tier,
    /// its slot, and how many the scope orders. `None` on a row the scope
    /// does not order — `+ new tier`, a machine tier seen from a board — or
    /// where it orders one alone, which has nowhere to go.
    fn tier_slot(&self, idx: usize) -> Option<(Tier, usize, usize)> {
        let Some(TierRow::Tier(t, _)) = self.tier_rows().get(idx).cloned() else { return None };
        let ordered = self.tiers().ordered(self.tier_scope());
        let at = ordered.iter().position(|id| *id == t.id)?;
        (ordered.len() > 1).then_some((t, at, ordered.len()))
    }

    /// `JK` (and Alt up/down) on the list (T-562): the board's nudge — the
    /// tier under the cursor one step along its scope's order, and the
    /// cursor rides with it. An edge press stays put, as a card's does.
    pub(super) fn nudge_tier(&mut self, key: Key) -> Result<()> {
        let Mode::Tiers { idx, naming: None } = self.mode else { return Ok(()) };
        let Some((t, at, n)) = self.tier_slot(idx) else { return Ok(()) };
        let down = matches!(key, Key::Char('J') | Key::AltDown);
        let to = if down { Some(at + 1).filter(|&to| to < n) } else { at.checked_sub(1) };
        let Some(to) = to else { return Ok(()) };
        let scope = self.tier_scope();
        match self.req(Command::MoveTier { scope, id: t.id.clone(), to_index: to }) {
            Response::Err { message } => self.status = message,
            _ => {
                self.refresh()?;
                let idx = self
                    .tier_rows()
                    .iter()
                    .position(|r| matches!(r, TierRow::Tier(x, _) if x.id == t.id))
                    .unwrap_or(idx);
                self.mode = Mode::Tiers { idx, naming: None };
                self.status = format!("moved {} {}", t.name, if down { "down" } else { "up" });
            }
        }
        Ok(())
    }

    /// A list row's label and detail.
    pub(crate) fn tier_row_words(&self, row: &TierRow) -> (String, String) {
        let board = self.settings_board_scope;
        match row {
            TierRow::Tier(t, source) => {
                let default_id = if board {
                    self.tiers().default_tier().id
                } else {
                    self.tiers().machine_default().id
                };
                let mark = if t.id == default_id { " ∙ default" } else { "" };
                let label = format!("{:<w$}  {}{mark}", t.name, t.summary(), w = tier::NAME_MAX);
                let detail = if board {
                    match source {
                        Source::Machine => {
                            "machine ∙ an edit here makes this board's own version".to_string()
                        }
                        other => other.word().to_string(),
                    }
                } else {
                    "every board".to_string()
                };
                (label, detail)
            }
            TierRow::New => (
                "+ new tier".to_string(),
                if board {
                    "a tier for this board alone".into()
                } else {
                    "a tier for every board".into()
                },
            ),
        }
    }

    /// Enter on the list: a tier opens its page, the last row names a new one.
    pub(super) fn tiers_act(&mut self) -> Result<()> {
        let Mode::Tiers { idx, naming: None } = self.mode else { return Ok(()) };
        match self.tier_rows().get(idx).cloned() {
            Some(TierRow::Tier(t, _)) => self.open_tier_page(t.id, 0),
            Some(TierRow::New) => {
                self.mode = Mode::Tiers { idx, naming: Some(EditBuffer::new(tier::NAME_MAX)) };
                self.status = "a short name ∙ letters, digits, - and _".into();
            }
            None => {}
        }
        Ok(())
    }

    /// A tier's page, on row `idx`. The providers' model lists are read
    /// again here, so a list the CLI refreshed meanwhile is the one the ring
    /// steps, and the tier's model word is the one the page keeps.
    fn open_tier_page(&mut self, id: String, idx: usize) {
        if let Some(homes) = &self.model_homes {
            self.tier_models = homes.read();
        }
        self.mode = Mode::TierEdit { id, idx, field: None, armed: false };
        self.tier_kept_model = self.edited_tier().map(|(t, _)| t.model).unwrap_or_default();
    }

    /// The page's Model ring: `default` first, the word no list offers —
    /// the tier's own, else the one the page opened with or was typed —
    /// then the provider's list.
    pub(crate) fn model_ring(&self) -> Vec<tier::ModelStop> {
        let Some((t, _)) = self.edited_tier() else { return Vec::new() };
        let list = self.tier_models.of(t.provider);
        let listed = |w: &str| list.is_some_and(|l| l.iter().any(|m| m.model == w));
        let saved = if t.model.is_empty() || listed(&t.model) {
            self.tier_kept_model.as_str()
        } else {
            t.model.as_str()
        };
        tier::model_ring(list, saved)
    }

    /// `i` on the Model row (T-951): a name typed in place, for a model the
    /// provider's list does not offer. The daemon's `check_model` guards it.
    pub(super) fn type_tier_model(&mut self) {
        let Mode::TierEdit { id, idx, field: None, .. } = self.mode.clone() else { return };
        if self.tier_fields().get(idx) != Some(&TierField::Model) {
            return;
        }
        let Some((t, _)) = self.edited_tier() else { return };
        let buf = EditBuffer::from_text(t.model, tier::MODEL_MAX);
        self.mode = Mode::TierEdit { id, idx, field: Some(buf), armed: false };
    }

    /// A tier name or a model being typed: the raw key edits the buffer,
    /// then Enter saves and Esc drops it, against `Scope::Input`.
    pub(super) fn key_tier_field(&mut self, code: KeyCode, mods: KeyModifiers) -> Result<()> {
        match &mut self.mode {
            Mode::Tiers { naming: Some(buf), .. } | Mode::TierEdit { field: Some(buf), .. } => {
                edit_buffer_key(buf, code, mods);
            }
            _ => return Ok(()),
        }
        let ctx = self.ctx();
        let verb = crate::keys::to_key_text(code, mods)
            .and_then(|k| keymap::resolve(Scope::Input, k, &ctx));
        match verb {
            Some(Verb::Save | Verb::SaveStart) => self.commit_tier_field(),
            Some(Verb::Cancel) => {
                match &mut self.mode {
                    Mode::Tiers { naming, .. } => *naming = None,
                    Mode::TierEdit { field, .. } => *field = None,
                    _ => {}
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn commit_tier_field(&mut self) -> Result<()> {
        match self.mode.clone() {
            Mode::Tiers { idx, naming: Some(buf) } => {
                let name = buf.as_str().trim().to_string();
                if name.is_empty() {
                    self.mode = Mode::Tiers { idx, naming: None };
                    return Ok(());
                }
                // A new tier starts as its scope's default provider at the
                // CLI's own model and effort: the page that opens next is
                // where it becomes something.
                let book = self.tiers();
                let provider = if self.settings_board_scope {
                    book.default_tier().provider
                } else {
                    book.machine_default().provider
                };
                let t = Tier {
                    id: ulid::Ulid::new().to_string(),
                    name,
                    provider,
                    model: String::new(),
                    effort: Effort::Default,
                    description: String::new(),
                };
                let scope = self.tier_scope();
                match self.req(Command::SaveTier { scope, tier: t.clone() }) {
                    Response::Err { message } => self.status = message,
                    _ => {
                        self.refresh()?;
                        self.status =
                            format!("{} made ∙ now its provider, model and effort", t.name);
                        self.open_tier_page(t.id, 1);
                    }
                }
            }
            Mode::TierEdit { id, idx, field: Some(buf), .. } => {
                let Some((mut t, _)) = self.edited_tier() else {
                    self.mode = Mode::TierEdit { id, idx, field: None, armed: false };
                    return Ok(());
                };
                let text = buf.as_str().trim().to_string();
                let field = self.tier_fields().get(idx).copied();
                match field {
                    Some(TierField::Name) => t.name = text.clone(),
                    Some(TierField::Model) => t.model = text.clone(),
                    Some(TierField::Description) => t.description = text.clone(),
                    _ => {}
                }
                // A refused name (`check_model`) leaves the field open with
                // the daemon's words, and never joins the ring.
                if self.save_edited(t)? {
                    if field == Some(TierField::Model) {
                        self.tier_kept_model = text;
                    }
                    self.mode = Mode::TierEdit { id, idx, field: None, armed: false };
                }
            }
            _ => {}
        }
        Ok(())
    }

    // ------------------------------------------------------------ the page

    /// The tier the page is about, as the dialog's scope sees it, and where
    /// it comes from.
    pub(crate) fn edited_tier(&self) -> Option<(Tier, Source)> {
        let Mode::TierEdit { id, .. } = &self.mode else { return None };
        if self.settings_board_scope {
            let book = self.tiers();
            Some((book.get(id)?, book.source(id)?))
        } else {
            self.machine_tiers
                .tiers
                .iter()
                .find(|t| &t.id == id)
                .map(|t| (t.clone(), Source::Machine))
        }
    }

    /// The page's rows: the five settings, and the removal where there is
    /// one to make — a machine tier seen from a board has nothing to drop.
    pub(crate) fn tier_fields(&self) -> Vec<TierField> {
        let mut fields = vec![
            TierField::Name,
            TierField::Provider,
            TierField::Model,
            TierField::Effort,
            TierField::Description,
        ];
        let removable = match self.edited_tier() {
            Some((_, Source::Machine)) => !self.settings_board_scope,
            Some(_) => true,
            None => false,
        };
        if removable {
            fields.push(TierField::Remove);
        }
        fields
    }

    /// A page row's label and detail.
    pub(crate) fn tier_field_words(&self, field: TierField, armed: bool) -> (String, String) {
        let Some((t, source)) = self.edited_tier() else { return (String::new(), String::new()) };
        let claude = t.provider == AgentProvider::ClaudeCode;
        match field {
            TierField::Name => (
                format!("Name: {}", t.name),
                "tickets keep their tier by id, whatever it is called".into(),
            ),
            TierField::Provider => (
                format!("Provider: {}", t.provider.label()),
                "h l switches ∙ a model is its provider's own, so it is cleared".into(),
            ),
            TierField::Model => self.model_words(&t),
            TierField::Effort => (
                format!("Effort: {}", t.effort.word()),
                if claude {
                    "claude --effort ∙ h l steps ∙ default leaves Claude Code's own".into()
                } else {
                    "codex model_reasoning_effort ∙ h l steps ∙ default leaves Codex's own".into()
                },
            ),
            TierField::Description => (
                if t.description.is_empty() {
                    "Description: none".into()
                } else {
                    format!("Description: {}", scrub_cells(&t.description, false))
                },
                "when to use it, in your words ∙ the crown picks by it".into(),
            ),
            TierField::Remove => {
                let (label, what) = if source == Source::Override {
                    ("Use the machine's", "drops this board's version ∙ the machine's comes back")
                } else {
                    (
                        "Delete tier",
                        "tickets on it go back to the default ∙ running agents keep theirs",
                    )
                };
                if armed {
                    (format!("{label} ∙ enter again confirms"), what.into())
                } else {
                    (label.into(), what.into())
                }
            }
        }
    }

    /// The Model row's label and detail: the stop the tier is on, and the
    /// one fact its word lacks — the provider's name for it, or where the
    /// list came from.
    fn model_words(&self, t: &Tier) -> (String, String) {
        let label = t.provider.label();
        let list = self.tier_models.of(t.provider);
        let no_list = || format!("no model list from {}", label.to_lowercase());
        // `i`'s field: what the name is handed to.
        let typing = matches!(self.mode, Mode::TierEdit { idx, field: Some(_), .. }
            if self.tier_fields().get(idx) == Some(&TierField::Model));
        if typing {
            let takes = match t.provider {
                AgentProvider::ClaudeCode => "claude --model",
                AgentProvider::Codex => "codex -c model=…",
            };
            return (String::new(), format!("any model {takes} takes ∙ kept as saved"));
        }
        let stop = self.model_ring().into_iter().find(|s| s.model == t.model);
        match stop.map(|s| s.kind) {
            Some(ModelStopKind::Listed(m)) => {
                let detail = match (m.name.is_empty(), m.about.is_empty()) {
                    (true, _) => m.about,
                    (false, true) => m.name,
                    (false, false) => format!("{} ∙ {}", m.name, m.about),
                };
                (format!("Model: {}", t.model), detail)
            }
            Some(ModelStopKind::Saved) => (
                format!("Model: {} (saved)", scrub_cells(&t.model, false)),
                match list {
                    Some(_) => format!("not on {label}'s list ∙ kept until another is picked"),
                    None => format!("{} ∙ kept until another is picked", no_list()),
                },
            ),
            _ => (
                "Model: default".into(),
                match list {
                    Some(l) if l.len() == 1 => {
                        format!("{label} picks its own ∙ 1 model on its list")
                    }
                    Some(l) => format!("{label} picks its own ∙ {} models on its list", l.len()),
                    None => format!("{} ∙ default", no_list()),
                },
            ),
        }
    }

    /// Enter on the page: a field opens in place, a step steps, the removal
    /// arms and then acts.
    pub(super) fn tier_edit_act(&mut self) -> Result<()> {
        let Mode::TierEdit { id, idx, field: None, armed } = self.mode.clone() else {
            return Ok(());
        };
        let Some((t, source)) = self.edited_tier() else { return Ok(()) };
        match self.tier_fields().get(idx).copied() {
            Some(TierField::Name) => {
                let buf = EditBuffer::from_text(t.name, tier::NAME_MAX);
                self.mode = Mode::TierEdit { id, idx, field: Some(buf), armed: false };
            }
            Some(TierField::Description) => {
                let buf = EditBuffer::from_text(t.description, tier::DESCRIPTION_MAX);
                self.mode = Mode::TierEdit { id, idx, field: Some(buf), armed: false };
            }
            Some(TierField::Provider | TierField::Model | TierField::Effort) => {
                self.tier_edit_step(true)?
            }
            Some(TierField::Remove) if !armed => {
                self.mode = Mode::TierEdit { id, idx, field: None, armed: true };
            }
            Some(TierField::Remove) => {
                let scope = self.tier_scope();
                let name = t.name.clone();
                match self.req(Command::DeleteTier { scope, id: id.clone() }) {
                    Response::Err { message } => self.status = message,
                    _ => {
                        self.refresh()?;
                        self.status = if source == Source::Override {
                            format!("{name} ∙ the machine's again")
                        } else {
                            format!("{name} deleted")
                        };
                        let idx = self
                            .tier_rows()
                            .iter()
                            .position(|r| matches!(r, TierRow::Tier(t, _) if t.id == id))
                            .unwrap_or(0);
                        self.mode = Mode::Tiers { idx, naming: None };
                    }
                }
            }
            None => {}
        }
        Ok(())
    }

    /// `h`/`l` (and Enter) on the provider, the model or the effort.
    pub(super) fn tier_edit_step(&mut self, forward: bool) -> Result<()> {
        let Mode::TierEdit { idx, .. } = self.mode else { return Ok(()) };
        let Some((mut t, _)) = self.edited_tier() else { return Ok(()) };
        match self.tier_fields().get(idx) {
            Some(TierField::Provider) => {
                t.provider = t.provider.next();
                t.model.clear();
                self.tier_kept_model.clear();
                if !Effort::ring(t.provider).contains(&t.effort) {
                    t.effort = Effort::Default;
                }
            }
            Some(TierField::Model) => {
                let ring = self.model_ring();
                if ring.len() < 2 {
                    return Ok(());
                }
                t.model = tier::step_model(&ring, &t.model, forward);
            }
            Some(TierField::Effort) => t.effort = t.effort.step(t.provider, forward),
            _ => return Ok(()),
        }
        self.save_edited(t)?;
        Ok(())
    }

    /// Save the page's tier into the dialog's scope. A board-scope save of a
    /// machine tier IS the board's version of it. True when it saved.
    fn save_edited(&mut self, t: Tier) -> Result<bool> {
        let scope = self.tier_scope();
        match self.req(Command::SaveTier { scope, tier: t }) {
            Response::Err { message } => {
                self.status = message;
                Ok(false)
            }
            _ => {
                self.refresh()?;
                Ok(true)
            }
        }
    }
}
