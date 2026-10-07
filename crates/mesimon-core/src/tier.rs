//! Agent tiers (T-443): a name for how a ticket's agent launches — which
//! provider, which model, how hard it thinks.
//!
//! Two layers, the settings shape (T-361): the MACHINE's tiers (`tiers.toml`
//! beside every board's state dir, one list for every repo) and a BOARD's
//! (`[[tiers]]` in `columns.toml`). A board entry with a machine tier's id
//! overrides it in place; a board entry with an id of its own is appended.
//! The default tier resolves board → machine → the `claude` built-in.
//!
//! Two built-ins, `claude` and `codex`, are never stored: each is "that
//! provider at the CLI's own defaults" — no `--model`, no `--effort` — which
//! is what every launch was before tiers existed. The cycle always offers
//! them; the editor never lists them.
//!
//! A ticket names its tier by ID, never by name, so renaming a machine tier
//! does not orphan another board's tickets. An id that no longer resolves (a
//! deleted tier, a teammate's tier on a shared board) reads as the default.
//!
//! Pure: every resolver takes the two layers and returns a value. The daemon
//! (at launch) and the TUI (on the card, in the cycle) call the same ones.

use serde::{Deserialize, Serialize};

use crate::board::{AgentProvider, Board, SessionKind};

/// The built-in tiers' ids — and their names, which are the same word.
pub const CLAUDE: &str = "claude";
pub const CODEX: &str = "codex";

/// A tier name is a word on a card: short, one token, no spaces.
pub const NAME_MAX: usize = 16;
/// A model rides argv and a Codex `-c` TOML string; this is generous for
/// `claude-opus-5-5[1m]` and bounded for both.
pub const MODEL_MAX: usize = 64;
/// A tier's description (T-584), in bytes: a sentence or two on when to use
/// it, the column description's bound, since `list_board` carries every
/// tier's to every agent that reads the board.
pub const DESCRIPTION_MAX: usize = 300;

/// How hard the agent thinks. `Default` passes nothing and leaves the CLI's
/// own choice. Each provider accepts its own subset ([`Effort::ring`]):
/// `claude --effort` takes low…max (2.1.280), Codex's
/// `model_reasoning_effort` minimal…ultra (codex-cli 0.155).
///
/// `Unknown` is a word a newer build wrote into the shared machine file;
/// it launches as `Default` rather than failing the whole file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effort {
    #[default]
    Default,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
    Ultra,
    #[serde(other)]
    Unknown,
}

impl Effort {
    /// The levels a provider takes, `Default` first. The editor steps over
    /// this ring; a level outside it never reaches argv.
    pub fn ring(provider: AgentProvider) -> &'static [Effort] {
        use Effort::*;
        match provider {
            AgentProvider::ClaudeCode => &[Default, Low, Medium, High, Xhigh, Max],
            AgentProvider::Codex => &[Default, Minimal, Low, Medium, High, Xhigh, Max, Ultra],
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
            Self::Ultra => "ultra",
            Self::Unknown => "unknown",
        }
    }

    pub fn is_default(&self) -> bool {
        *self == Self::Default
    }

    /// The next (or previous) level on `provider`'s ring, wrapping. A level
    /// the ring lacks steps to the ring's first.
    pub fn step(self, provider: AgentProvider, forward: bool) -> Effort {
        let ring = Self::ring(provider);
        let Some(i) = ring.iter().position(|e| *e == self) else {
            return ring[0];
        };
        let n = ring.len();
        ring[if forward { (i + 1) % n } else { (i + n - 1) % n }]
    }
}

/// One tier. `model` empty and `effort` `Default` are "the CLI decides".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tier {
    /// A ULID string for a tier a person made; `claude`/`codex` for the
    /// built-ins. The reference a ticket stores.
    pub id: String,
    pub name: String,
    pub provider: AgentProvider,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(default, skip_serializing_if = "Effort::is_default")]
    pub effort: Effort,
    /// When to use this tier, in the person's own words (T-584): "docs,
    /// renames, one-file fixes". The crown reads it in `list_board` to pick
    /// a tier for a ticket it files or starts; nothing in mesimon reads it
    /// as a rule. Each layer's entry carries its own, so a board's version
    /// of a machine tier keeps the board's words. An older build drops it,
    /// which widens nothing, so no schema moves.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

impl Tier {
    /// The built-in for a provider: its id is its name, and it passes nothing.
    pub fn builtin(provider: AgentProvider) -> Tier {
        let id = builtin_id(provider);
        Tier {
            id: id.into(),
            name: id.into(),
            provider,
            model: String::new(),
            effort: Effort::Default,
            description: String::new(),
        }
    }

    pub fn is_builtin(&self) -> bool {
        is_builtin_id(&self.id)
    }

    /// The model to pass, if any — and only one [`check_model`] accepts, so
    /// a hand-edited file cannot put a flag into argv.
    pub fn model_arg(&self) -> Option<&str> {
        (!self.model.is_empty() && check_model(&self.model).is_ok()).then_some(self.model.as_str())
    }

    /// The effort to pass, if any: a level on this provider's ring, never
    /// `Default` or `Unknown`.
    pub fn effort_arg(&self) -> Option<&'static str> {
        (!matches!(self.effort, Effort::Default | Effort::Unknown)
            && Effort::ring(self.provider).contains(&self.effort))
        .then(|| self.effort.word())
    }

    /// `Claude Code ∙ opus ∙ high` — the words a row or a status line
    /// spells a tier's launch in. Defaults are said as such, so a built-in
    /// reads `Claude Code ∙ its own model`.
    pub fn summary(&self) -> String {
        let mut parts = vec![self.provider.label().to_string()];
        match self.model_arg() {
            Some(m) => parts.push(m.to_string()),
            None if self.effort_arg().is_none() => parts.push("its own model".into()),
            None => {}
        }
        if let Some(e) = self.effort_arg() {
            parts.push(e.to_string());
        }
        parts.join(" ∙ ")
    }
}

pub fn builtin_id(provider: AgentProvider) -> &'static str {
    match provider {
        AgentProvider::ClaudeCode => CLAUDE,
        AgentProvider::Codex => CODEX,
    }
}

pub fn is_builtin_id(id: &str) -> bool {
    id == CLAUDE || id == CODEX
}

/// The machine layer: `tiers.toml` in the state root, every board's.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineTiers {
    /// A scalar, so it sits before `[[tiers]]` on disk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_tier: Option<String>,
    #[serde(default)]
    pub tiers: Vec<Tier>,
}

/// Which layer a settings scope writes. Also what a resolved tier says it
/// came from, beside [`Source`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TierScope {
    Machine,
    Board,
}

/// Where a resolved tier came from — the provenance word a board-scope row
/// wears.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Builtin,
    Machine,
    /// This board's own tier.
    Board,
    /// This board's entry for a machine tier's id.
    Override,
}

impl Source {
    pub fn word(self) -> &'static str {
        match self {
            Self::Builtin => "built in",
            Self::Machine => "machine",
            Self::Board => "this board",
            Self::Override => "this board ∙ overrides machine",
        }
    }
}

/// The two layers, read together. Borrowed for one question; cheap to make.
#[derive(Clone, Copy)]
pub struct Book<'a> {
    pub machine: &'a MachineTiers,
    pub board: &'a Board,
}

impl<'a> Book<'a> {
    pub fn new(machine: &'a MachineTiers, board: &'a Board) -> Self {
        Self { machine, board }
    }

    /// The tiers a person made, resolved: machine tiers in their order (a
    /// board override in its place), then the board's own. No built-ins.
    pub fn custom(&self) -> Vec<(Tier, Source)> {
        let mut out: Vec<(Tier, Source)> = self
            .machine
            .tiers
            .iter()
            .filter(|t| !t.is_builtin())
            .map(|t| match self.board.tiers.iter().find(|b| b.id == t.id) {
                Some(over) => (over.clone(), Source::Override),
                None => (t.clone(), Source::Machine),
            })
            .collect();
        for t in &self.board.tiers {
            if !t.is_builtin() && !out.iter().any(|(o, _)| o.id == t.id) {
                out.push((t.clone(), Source::Board));
            }
        }
        out
    }

    /// Every tier a ticket may name: the two built-ins, then [`Self::custom`].
    pub fn all(&self) -> Vec<Tier> {
        let mut out =
            vec![Tier::builtin(AgentProvider::ClaudeCode), Tier::builtin(AgentProvider::Codex)];
        out.extend(self.custom().into_iter().map(|(t, _)| t));
        out
    }

    pub fn get(&self, id: &str) -> Option<Tier> {
        if id == CLAUDE {
            return Some(Tier::builtin(AgentProvider::ClaudeCode));
        }
        if id == CODEX {
            return Some(Tier::builtin(AgentProvider::Codex));
        }
        self.custom().into_iter().map(|(t, _)| t).find(|t| t.id == id)
    }

    pub fn source(&self, id: &str) -> Option<Source> {
        if is_builtin_id(id) {
            return Some(Source::Builtin);
        }
        self.custom().into_iter().find(|(t, _)| t.id == id).map(|(_, s)| s)
    }

    /// The machine's own default, ignoring the board.
    pub fn machine_default(&self) -> Tier {
        self.machine
            .default_tier
            .as_deref()
            .and_then(|id| self.get(id))
            .unwrap_or_else(|| Tier::builtin(AgentProvider::ClaudeCode))
    }

    /// The board's own default choice, if it made one. A board from before
    /// tiers that chose Codex as its provider made that choice
    /// (`Board::agent_provider`), and it still stands.
    pub fn board_default(&self) -> Option<Tier> {
        match self.board.default_tier.as_deref() {
            Some(id) => self.get(id),
            None if self.board.agent_provider == AgentProvider::Codex => {
                Some(Tier::builtin(AgentProvider::Codex))
            }
            None => None,
        }
    }

    /// What a ticket that picked nothing starts on: board, then machine,
    /// then `claude`. A default naming a tier that is gone falls through.
    pub fn default_tier(&self) -> Tier {
        self.board_default().unwrap_or_else(|| self.machine_default())
    }

    /// The tier a ticket wants: its own pick when that still resolves,
    /// otherwise the default.
    pub fn of_ticket(&self, ticket: ulid::Ulid) -> Tier {
        self.board
            .ticket(ticket)
            .and_then(|t| t.tier.as_deref())
            .and_then(|id| self.get(id))
            .unwrap_or_else(|| self.default_tier())
    }

    /// Whether a ticket's stored pick is one this machine cannot resolve —
    /// a deleted tier, or a teammate's. The card must not name it as if it
    /// did.
    pub fn dangles(&self, ticket: ulid::Ulid) -> bool {
        self.board
            .ticket(ticket)
            .and_then(|t| t.tier.as_deref())
            .is_some_and(|id| self.get(id).is_none())
    }

    /// The tier a launch of a `kind` session on this ticket runs on. A
    /// session's provider is fixed at its birth, so a ticket whose pick is
    /// another provider's (a tier edited since) launches on its own
    /// provider's built-in rather than on a model that CLI has never heard of.
    pub fn launch(&self, ticket: ulid::Ulid, kind: SessionKind) -> Tier {
        let want = self.of_ticket(ticket);
        match kind.provider() {
            Some(p) if p != want.provider => Tier::builtin(p),
            _ => want,
        }
    }

    /// Which agent a NEW seat on this ticket is — the ticket's tier's
    /// provider. The one question every "start an agent" road asks.
    pub fn start_provider(&self, ticket: ulid::Ulid) -> AgentProvider {
        self.of_ticket(ticket).provider
    }

    /// Which agent the ticket's NEXT turn runs on (T-685): the seat's own
    /// provider where it holds one — live, parked or still stopping, since a
    /// conversation cannot move between CLIs — else the one a start would
    /// be, [`Self::start_provider`]. The plan flag's two readers (the TUI's
    /// field and the daemon's refusal) and `c`'s kind ask this one question.
    pub fn seat_provider(&self, ticket: ulid::Ulid) -> AgentProvider {
        match self.board.live_agent(ticket).and_then(|s| s.kind.provider()) {
            Some(p) => p,
            None => self.start_provider(ticket),
        }
    }

    /// The tiers the ticket may cycle to. A ticket holding a seat (live,
    /// parked or still stopping) keeps its provider — a conversation cannot
    /// move between CLIs — so only that provider's tiers are offered; an
    /// empty seat offers every tier [`Self::cycle`] holds.
    pub fn ring(&self, ticket: ulid::Ulid) -> Vec<Tier> {
        self.cycle(self.board.live_agent(ticket).and_then(|s| s.kind.provider()))
    }

    /// What `^n` steps through, for a seat of `seat`'s provider or none: the
    /// tiers a person made, in the tiers list's order. The built-ins ride
    /// only where they must (T-562, user: "skip built-ins in ^n") — the
    /// default when it is one, so a pick can always come back to it, and
    /// both while nobody has made a tier.
    pub fn cycle(&self, seat: Option<AgentProvider>) -> Vec<Tier> {
        let custom: Vec<Tier> = self.custom().into_iter().map(|(t, _)| t).collect();
        let tiers = if custom.is_empty() {
            self.all()
        } else {
            let default = self.default_tier();
            let mut out = Vec::new();
            if default.is_builtin() {
                out.push(default);
            }
            out.extend(custom);
            out
        };
        tiers.into_iter().filter(|t| seat.is_none_or(|p| t.provider == p)).collect()
    }

    /// The tiers an agent may name for a ticket it files or starts (T-584),
    /// as `list_board` lists them: `^n`'s ring for an empty seat
    /// ([`Self::cycle`]) — the tiers a person made, in the tiers list's
    /// order, with the default first when it is a built-in. While nobody
    /// made a tier the ring is both built-ins, and only the default is
    /// offered: a built-in carries no words to pick it by, and the other
    /// one is a CLI this person may not run.
    pub fn offered(&self) -> Vec<Tier> {
        let mut tiers = self.cycle(None);
        if self.custom().is_empty() {
            let default = self.default_tier().id;
            tiers.retain(|t| t.id == default);
        }
        tiers
    }

    /// The tier an agent's `word` names among [`Self::offered`]: its id, or
    /// its name in any case (names are unique, and a ULID is no name). The
    /// refusal lists what it could have named.
    pub fn resolve_offered(&self, word: &str) -> Result<Tier, String> {
        let offered = self.offered();
        let found = offered
            .iter()
            .find(|t| t.id == word)
            .or_else(|| offered.iter().find(|t| t.name.eq_ignore_ascii_case(word)));
        match found {
            Some(t) => Ok(t.clone()),
            None => Err(format!(
                "no tier {word} on this board; list_board's tiers are {}",
                offered
                    .iter()
                    .map(|t| if t.id == t.name {
                        t.id.clone()
                    } else {
                        format!("{} ({})", t.id, t.name)
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    /// The tier after `from` on the ticket's ring, wrapping; `None` when the
    /// ring has nothing else to offer.
    pub fn next_after(&self, ticket: ulid::Ulid, from: &str) -> Option<Tier> {
        let ring = self.ring(ticket);
        if ring.len() < 2 {
            return None;
        }
        let at = ring.iter().position(|t| t.id == from);
        let next = match at {
            Some(i) => ring[(i + 1) % ring.len()].clone(),
            None => ring[0].clone(),
        };
        (next.id != from).then_some(next)
    }

    /// What a ticket STORES for a pick: `None` for the default (so it
    /// follows a later change of default), the id otherwise.
    pub fn stored_pick(&self, id: &str) -> Option<String> {
        (id != self.default_tier().id).then(|| id.to_string())
    }

    /// The ids `scope`'s layer orders (T-562), in its order: every tier the
    /// machine made, or a board's own. A board's version of a machine tier
    /// stands where the machine puts it, so the board has no say there.
    pub fn ordered(&self, scope: TierScope) -> Vec<String> {
        match scope {
            TierScope::Machine => self
                .machine
                .tiers
                .iter()
                .filter(|t| !t.is_builtin())
                .map(|t| t.id.clone())
                .collect(),
            TierScope::Board => self
                .custom()
                .into_iter()
                .filter(|(_, s)| *s == Source::Board)
                .map(|(t, _)| t.id)
                .collect(),
        }
    }
}

/// Move `id` to slot `to` among the entries of `list` that [`Book::ordered`]
/// names — one layer's tiers — clamped to the last slot. Every other entry
/// keeps its place. `None` when `id` is not one of them; otherwise whether
/// anything moved.
pub fn move_tier(list: &mut [Tier], ordered: &[String], id: &str, to: usize) -> Option<bool> {
    let slots: Vec<usize> = (0..list.len()).filter(|&i| ordered.contains(&list[i].id)).collect();
    let from = slots.iter().position(|&i| list[i].id == id)?;
    let to = to.min(slots.len() - 1);
    if from == to {
        return Some(false);
    }
    let mut owned: Vec<Tier> = slots.iter().map(|&i| list[i].clone()).collect();
    let t = owned.remove(from);
    owned.insert(to, t);
    for (i, t) in slots.into_iter().zip(owned) {
        list[i] = t;
    }
    Some(true)
}

/// A tier name a person typed: 1–16 characters of `[A-Za-z0-9_-]`, not a
/// built-in's, and not another resolved tier's (ignoring case). `except` is
/// the id being renamed, which may keep its own name.
pub fn check_name(name: &str, taken: &[Tier], except: Option<&str>) -> Result<(), String> {
    if name.is_empty() {
        return Err("a tier needs a name".into());
    }
    if name.chars().count() > NAME_MAX {
        return Err(format!("a tier name is at most {NAME_MAX} characters"));
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("a tier name is letters, digits, - and _".into());
    }
    if is_builtin_id(&name.to_ascii_lowercase()) {
        return Err(format!("{name} is a built-in tier"));
    }
    if taken.iter().any(|t| Some(t.id.as_str()) != except && t.name.eq_ignore_ascii_case(name)) {
        return Err(format!("there is already a tier called {name}"));
    }
    Ok(())
}

/// The daemon-side boundary for a tier's description (T-584): drawn in the
/// tiers list and read by the crown, so `scrub_cells` on one line (which
/// also removes everything `scrub_text` does), capped at
/// [`DESCRIPTION_MAX`]. Empty when blank once scrubbed.
pub fn sanitize_description(raw: &str) -> String {
    use crate::text::{cap_bytes, nonblank, scrub_cells};
    nonblank(cap_bytes(&scrub_cells(raw, false), DESCRIPTION_MAX)).unwrap_or_default()
}

/// A model a person typed. It reaches argv (`--model <m>`) and a Codex TOML
/// string (`-c model="<m>"`), so it is a narrow whitelist rather than a
/// sanitizer: `[A-Za-z0-9._:/@[]-]`, at most 64 bytes, never a leading `-`.
/// Empty is allowed and means the CLI's own model.
pub fn check_model(model: &str) -> Result<(), String> {
    if model.len() > MODEL_MAX {
        return Err(format!("a model name is at most {MODEL_MAX} characters"));
    }
    if model.starts_with('-') {
        return Err("a model name cannot start with -".into());
    }
    if !model.chars().all(|c| {
        c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '/' | '@' | '[' | ']' | '-')
    }) {
        return Err("a model name is letters, digits and . _ : / @ [ ] -".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{SessionRecord, SessionState, Ticket};

    fn tier(id: &str, name: &str, provider: AgentProvider, model: &str, effort: Effort) -> Tier {
        Tier {
            id: id.into(),
            name: name.into(),
            provider,
            model: model.into(),
            effort,
            description: String::new(),
        }
    }

    fn ticket(board: &mut Board, tier: Option<&str>) -> ulid::Ulid {
        let id = ulid::Ulid::new();
        board.tickets.push(Ticket {
            id,
            short_key: "T-1".into(),
            title: "t".into(),
            column: "TODO".into(),
            order: "a".into(),
            created_at: "@0".into(),
            created_by: String::new(),
            created_from: None,
            entered_at: None,
            previous_column: None,
            picked: None,
            woke_at: None,
            manual_merge: false,
            execution_policy: Default::default(),
            tier: tier.map(str::to_string),
            import_origin: None,
            envelope: None,
            raised: None,
            workspace: None,
            tags: Vec::new(),
            notes: Vec::new(),
            archived: None,
        });
        id
    }

    fn claude_code() -> AgentProvider {
        AgentProvider::ClaudeCode
    }

    /// T-562: each layer orders its own. The machine moves among its tiers;
    /// a board moves only its own, and its version of a machine tier is not
    /// one of them — it stands where the machine puts it.
    #[test]
    fn a_layer_reorders_only_the_tiers_it_owns() {
        let mut machine = MachineTiers {
            default_tier: None,
            tiers: vec![
                tier("A", "quick", claude_code(), "", Effort::Default),
                tier("B", "coder", claude_code(), "", Effort::Default),
                tier("C", "deep", claude_code(), "", Effort::Default),
            ],
        };
        let mut board = Board {
            tiers: vec![
                tier("D", "mine", claude_code(), "", Effort::Default),
                tier("B", "coder", claude_code(), "opus", Effort::Max),
                tier("E", "also", AgentProvider::Codex, "", Effort::Default),
            ],
            ..Board::default()
        };
        let names = |m: &MachineTiers, b: &Board| -> Vec<String> {
            Book::new(m, b).custom().into_iter().map(|(t, _)| t.name).collect()
        };
        assert_eq!(Book::new(&machine, &board).ordered(TierScope::Machine), ["A", "B", "C"]);
        assert_eq!(Book::new(&machine, &board).ordered(TierScope::Board), ["D", "E"]);

        // The machine's: the last to the top, clamped past the end, a move
        // to where it stands is no move.
        let ordered = Book::new(&machine, &board).ordered(TierScope::Machine);
        assert_eq!(move_tier(&mut machine.tiers, &ordered, "C", 0), Some(true));
        assert_eq!(names(&machine, &board), ["deep", "quick", "coder", "mine", "also"]);
        let ordered = Book::new(&machine, &board).ordered(TierScope::Machine);
        assert_eq!(move_tier(&mut machine.tiers, &ordered, "C", 9), Some(true));
        assert_eq!(names(&machine, &board), ["quick", "coder", "deep", "mine", "also"]);
        let ordered = Book::new(&machine, &board).ordered(TierScope::Machine);
        assert_eq!(move_tier(&mut machine.tiers, &ordered, "C", 2), Some(false));
        assert_eq!(move_tier(&mut machine.tiers, &ordered, "D", 0), None, "the board's own");

        // The board's: its own two swap, the override between them keeps its
        // entry, and the override is not the board's to move.
        let ordered = Book::new(&machine, &board).ordered(TierScope::Board);
        assert_eq!(move_tier(&mut board.tiers, &ordered, "E", 0), Some(true));
        assert_eq!(names(&machine, &board), ["quick", "coder", "deep", "also", "mine"]);
        assert_eq!(board.tiers[1].id, "B");
        let ordered = Book::new(&machine, &board).ordered(TierScope::Board);
        assert_eq!(move_tier(&mut board.tiers, &ordered, "B", 0), None, "the machine orders it");
    }

    /// T-584: a tier carries the person's words on when to use it. Each
    /// layer's entry carries its own, so a board's version of a machine
    /// tier keeps the board's words, and the resolver hands them on.
    #[test]
    fn a_description_rides_each_layer_and_a_board_version_keeps_its_own() {
        let words = |t: Tier, d: &str| Tier { description: d.into(), ..t };
        let machine = MachineTiers {
            default_tier: None,
            tiers: vec![
                words(tier("A", "quick", claude_code(), "sonnet", Effort::Low), "docs, renames"),
                words(tier("B", "deep", claude_code(), "opus", Effort::High), "refactors"),
            ],
        };
        let board = Board {
            tiers: vec![
                words(tier("B", "deep", claude_code(), "opus", Effort::Max), "the daemon's writer"),
                tier("C", "plain", claude_code(), "", Effort::Default),
            ],
            ..Board::default()
        };
        let book = Book::new(&machine, &board);
        assert_eq!(book.get("A").unwrap().description, "docs, renames");
        assert_eq!(book.get("B").unwrap().description, "the daemon's writer", "the board's own");
        assert_eq!(book.get("C").unwrap().description, "");
        assert_eq!(book.get(CLAUDE).unwrap().description, "", "a built-in has no words");

        // Empty is not written, and a file from before the field parses.
        let json = serde_json::to_string(&board.tiers[1]).unwrap();
        assert!(!json.contains("description"), "{json}");
        let old: Tier =
            serde_json::from_str(r#"{"id":"A","name":"x","provider":"claude_code"}"#).unwrap();
        assert_eq!(old.description, "");
        let back: MachineTiers =
            serde_json::from_str(&serde_json::to_string(&machine).unwrap()).unwrap();
        assert_eq!(back, machine);
    }

    /// T-584: what an agent may name — `^n`'s ring for an empty seat, and
    /// only the default while nobody made a tier — by id or by name, and
    /// the refusal lists every id it could have named.
    #[test]
    fn an_agent_names_an_offered_tier_by_id_or_name() {
        let none = MachineTiers::default();
        let ids = |m: &MachineTiers, b: &Board| -> Vec<String> {
            Book::new(m, b).offered().into_iter().map(|t| t.id).collect()
        };
        assert_eq!(ids(&none, &Board::default()), [CLAUDE], "no tiers: the default alone");
        let codex = Board { default_tier: Some(CODEX.into()), ..Board::default() };
        assert_eq!(ids(&none, &codex), [CODEX]);
        let machine = MachineTiers {
            default_tier: None,
            tiers: vec![
                tier("01QUICK", "quick", claude_code(), "sonnet", Effort::Low),
                tier("01DEEP", "deep", claude_code(), "opus", Effort::Max),
            ],
        };
        let board = Board::default();
        assert_eq!(ids(&machine, &board), [CLAUDE, "01QUICK", "01DEEP"]);
        let ours = MachineTiers { default_tier: Some("01DEEP".into()), ..machine.clone() };
        assert_eq!(ids(&ours, &board), ["01QUICK", "01DEEP"], "a default of one's own");

        let book = Book::new(&machine, &board);
        assert_eq!(book.resolve_offered("01DEEP").unwrap().name, "deep");
        assert_eq!(book.resolve_offered("Quick").unwrap().id, "01QUICK", "a name, any case");
        let refused = book.resolve_offered("codex").unwrap_err();
        assert_eq!(
            refused,
            "no tier codex on this board; list_board's tiers are claude, 01QUICK (quick), \
             01DEEP (deep)"
        );
    }

    #[test]
    fn a_description_is_one_scrubbed_line_under_the_cap() {
        assert_eq!(sanitize_description("  docs\tand\u{7} renames \n"), "docs and renames");
        assert_eq!(sanitize_description(" \u{1b} \n"), "", "blank once scrubbed");
        let long = sanitize_description(&"é".repeat(DESCRIPTION_MAX));
        assert!(long.len() <= DESCRIPTION_MAX && long.chars().all(|c| c == 'é'));
    }

    #[test]
    fn a_board_entry_overrides_in_place_and_board_tiers_append() {
        let machine = MachineTiers {
            default_tier: None,
            tiers: vec![
                tier("A", "quick", claude_code(), "sonnet", Effort::High),
                tier("B", "coder", claude_code(), "opus", Effort::Xhigh),
            ],
        };
        let board = Board {
            tiers: vec![
                tier("B", "coder", claude_code(), "opus", Effort::Max),
                tier("C", "reviewer", AgentProvider::Codex, "gpt-6-astra", Effort::High),
            ],
            ..Board::default()
        };
        let book = Book::new(&machine, &board);
        let custom = book.custom();
        let names: Vec<_> = custom.iter().map(|(t, s)| (t.name.as_str(), *s)).collect();
        assert_eq!(
            names,
            [("quick", Source::Machine), ("coder", Source::Override), ("reviewer", Source::Board)]
        );
        assert_eq!(custom[1].0.effort, Effort::Max, "the board's copy wins");
        let all: Vec<_> = book.all().into_iter().map(|t| t.id).collect();
        assert_eq!(all, ["claude", "codex", "A", "B", "C"]);
    }

    #[test]
    fn the_default_resolves_board_then_machine_then_claude() {
        let machine = MachineTiers {
            default_tier: Some("A".into()),
            tiers: vec![tier("A", "quick", claude_code(), "sonnet", Effort::High)],
        };
        let mut board = Board::default();
        assert_eq!(Book::new(&MachineTiers::default(), &board).default_tier().id, CLAUDE);
        assert_eq!(Book::new(&machine, &board).default_tier().id, "A");
        board.default_tier = Some(CODEX.into());
        assert_eq!(Book::new(&machine, &board).default_tier().id, CODEX);
        // A default naming a tier that is gone falls through, never errors.
        board.default_tier = Some("gone".into());
        assert_eq!(Book::new(&machine, &board).default_tier().id, "A");
        // A board from before tiers that chose Codex keeps Codex.
        let legacy = Board { agent_provider: AgentProvider::Codex, ..Board::default() };
        assert_eq!(Book::new(&machine, &legacy).default_tier().id, CODEX);
    }

    #[test]
    fn a_ticket_pick_resolves_and_a_dangling_one_reads_as_the_default() {
        let machine = MachineTiers {
            default_tier: None,
            tiers: vec![tier("A", "quick", claude_code(), "sonnet", Effort::High)],
        };
        let mut board = Board::default();
        let picked = ticket(&mut board, Some("A"));
        let plain = ticket(&mut board, None);
        let gone = ticket(&mut board, Some("gone"));
        let book = Book::new(&machine, &board);
        assert_eq!(book.of_ticket(picked).id, "A");
        assert_eq!(book.of_ticket(plain).id, CLAUDE);
        assert_eq!(book.of_ticket(gone).id, CLAUDE);
        assert!(book.dangles(gone));
        assert!(!book.dangles(picked) && !book.dangles(plain));
        assert_eq!(book.stored_pick(CLAUDE), None, "the default is stored as inherit");
        assert_eq!(book.stored_pick("A").as_deref(), Some("A"));
    }

    #[test]
    fn a_seat_keeps_its_provider_in_the_ring_and_at_launch() {
        let machine = MachineTiers {
            default_tier: None,
            tiers: vec![
                tier("A", "quick", claude_code(), "sonnet", Effort::High),
                tier("C", "reviewer", AgentProvider::Codex, "gpt-6-astra", Effort::High),
            ],
        };
        let mut board = Board::default();
        let t = ticket(&mut board, None);
        let ring = |b: &Board| -> Vec<String> {
            Book::new(&machine, b).ring(t).into_iter().map(|t| t.id).collect()
        };
        // The built-ins are not in the cycle once a person made tiers — but
        // the default is, while it is one, so a pick can come back to it.
        assert_eq!(ring(&board), ["claude", "A", "C"], "an empty seat offers every provider");
        board.sessions.push(SessionRecord::new(
            uuid::Uuid::new_v4(),
            SessionKind::Claude,
            t,
            vec![],
            "/".into(),
            SessionState::Sleeping,
        ));
        assert_eq!(ring(&board), ["claude", "A"], "a parked claude keeps claude's tiers");
        let book = Book::new(&machine, &board);
        assert_eq!(book.next_after(t, CLAUDE).unwrap().id, "A");
        assert_eq!(book.next_after(t, "A").unwrap().id, CLAUDE, "wraps");
        // A default of one's own leaves the built-ins out altogether, and the
        // cycle is the tiers list's order.
        let mut ours = machine.clone();
        ours.default_tier = Some("A".into());
        ours.tiers.insert(0, tier("B", "deep", claude_code(), "opus", Effort::Max));
        let ids = |b: &Board| -> Vec<String> {
            Book::new(&ours, b).ring(t).into_iter().map(|t| t.id).collect()
        };
        assert_eq!(ids(&board), ["B", "A"], "a parked claude: its provider's, in order");
        assert_eq!(ids(&Board { sessions: vec![], ..board.clone() }), ["B", "A", "C"]);
        // Nobody made a tier: the two built-ins are the whole cycle.
        let none = MachineTiers::default();
        assert_eq!(
            Book::new(&none, &Board::default())
                .cycle(None)
                .into_iter()
                .map(|t| t.id)
                .collect::<Vec<_>>(),
            [CLAUDE, CODEX]
        );
        // A pick of another provider's tier (edited since) launches on the
        // seat's own built-in, never on a model that CLI does not know.
        board.tickets[0].tier = Some("C".into());
        let book = Book::new(&machine, &board);
        assert_eq!(book.launch(t, SessionKind::Claude).id, CLAUDE);
        assert_eq!(book.launch(t, SessionKind::Codex).id, "C");
        assert_eq!(book.start_provider(t), AgentProvider::Codex);
        // The seat's provider (T-685): the parked claude's while it holds the
        // seat, the tier's once the seat is empty.
        assert_eq!(book.seat_provider(t), AgentProvider::ClaudeCode);
        let empty = Board { sessions: vec![], ..board.clone() };
        assert_eq!(Book::new(&machine, &empty).seat_provider(t), AgentProvider::Codex);
    }

    #[test]
    fn only_a_level_on_the_providers_ring_reaches_argv() {
        let t = tier("A", "quick", claude_code(), "sonnet", Effort::High);
        assert_eq!((t.model_arg(), t.effort_arg()), (Some("sonnet"), Some("high")));
        let ultra = tier("A", "x", claude_code(), "", Effort::Ultra);
        assert_eq!(ultra.effort_arg(), None, "claude has no ultra");
        let codex = tier("A", "x", AgentProvider::Codex, "", Effort::Ultra);
        assert_eq!(codex.effort_arg(), Some("ultra"));
        let bad = tier("A", "x", claude_code(), "--dangerously-skip-permissions", Effort::Default);
        assert_eq!(bad.model_arg(), None, "a hand-edited flag never reaches argv");
        assert_eq!(Tier::builtin(claude_code()).summary(), "Claude Code ∙ its own model");
        assert_eq!(t.summary(), "Claude Code ∙ sonnet ∙ high");
    }

    #[test]
    fn effort_steps_wrap_over_the_providers_ring() {
        assert_eq!(Effort::Max.step(claude_code(), true), Effort::Default);
        assert_eq!(Effort::Default.step(claude_code(), false), Effort::Max);
        assert_eq!(Effort::Ultra.step(claude_code(), true), Effort::Default, "off-ring → first");
        assert_eq!(Effort::Xhigh.step(AgentProvider::Codex, true), Effort::Max);
    }

    #[test]
    fn an_unknown_effort_word_parses_and_launches_as_the_default() {
        let t: Tier = serde_json::from_str(
            r#"{"id":"A","name":"x","provider":"claude_code","effort":"galaxy"}"#,
        )
        .unwrap();
        assert_eq!(t.effort, Effort::Unknown);
        assert_eq!(t.effort_arg(), None);
    }

    #[test]
    fn names_and_models_are_checked() {
        let taken = vec![tier("A", "quick", claude_code(), "", Effort::Default)];
        assert!(check_name("coder", &taken, None).is_ok());
        assert!(check_name("Quick", &taken, None).is_err(), "case-insensitive");
        assert!(check_name("quick", &taken, Some("A")).is_ok(), "a rename keeps its own");
        assert!(check_name("Claude", &taken, None).is_err());
        assert!(check_name("two words", &taken, None).is_err());
        assert!(check_name("", &taken, None).is_err());
        assert!(check_name(&"x".repeat(NAME_MAX + 1), &taken, None).is_err());
        assert!(check_model("").is_ok());
        assert!(check_model("claude-opus-5-5[1m]").is_ok());
        assert!(check_model("gpt-6-astra").is_ok());
        assert!(check_model("-x").is_err());
        assert!(check_model("a\"b").is_err());
        assert!(check_model("a b").is_err());
    }
}
