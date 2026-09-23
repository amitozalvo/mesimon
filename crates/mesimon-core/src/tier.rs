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

    /// The tiers the ticket may cycle to. A ticket holding a seat (live,
    /// parked or still stopping) keeps its provider — a conversation cannot
    /// move between CLIs — so only that provider's tiers are offered; an
    /// empty seat offers every tier.
    pub fn ring(&self, ticket: ulid::Ulid) -> Vec<Tier> {
        let seat = self.board.live_agent(ticket).and_then(|s| s.kind.provider());
        self.all().into_iter().filter(|t| seat.is_none_or(|p| t.provider == p)).collect()
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
        Tier { id: id.into(), name: name.into(), provider, model: model.into(), effort }
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
            woke_at: None,
            manual_merge: false,
            execution_policy: Default::default(),
            tier: tier.map(str::to_string),
            import_origin: None,
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
        assert_eq!(ring(&board), ["claude", "codex", "A", "C"], "an empty seat offers everything");
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
        // A pick of another provider's tier (edited since) launches on the
        // seat's own built-in, never on a model that CLI does not know.
        board.tickets[0].tier = Some("C".into());
        let book = Book::new(&machine, &board);
        assert_eq!(book.launch(t, SessionKind::Claude).id, CLAUDE);
        assert_eq!(book.launch(t, SessionKind::Codex).id, "C");
        assert_eq!(book.start_provider(t), AgentProvider::Codex);
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
