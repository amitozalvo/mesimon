//! The keymap as data (04 §2.16), and the single source every hint renders from.
//!
//! Three things used to drift independently: the `match` arms in `app.rs`, the
//! footer literals in `ui/`, and doc 04's tables. This module makes that
//! impossible by construction:
//!
//! 1. `resolve(scope, key, ctx)` is the ONLY way a keypress becomes a [`Verb`],
//!    and the TUI dispatches on `Verb` with an exhaustive match — so a binding
//!    that exists here but is unhandled is a compile error, and a handler for a
//!    verb that is not in the table cannot be reached.
//! 2. Every binding carries an availability predicate over [`Ctx`]. The same
//!    predicate gates the key AND the hint, so a key that is hinted always
//!    works and a key that works is always hinted. "Can't move when no ticket
//!    is selected" is one line here, not two places that agree by luck.
//! 3. `footer(scope, ctx, width)` and `overlay(scope, ctx)` both read this
//!    table. There are no hint literals left in `ui/`.
//!
//! The validators in the test module enforce 04 §2.0's mechanical rules: the
//! legacy-floor atom set, the banned atoms, and "no key bound in both a scope
//! and its parent" (rule 3 — with no override escape hatch, because the
//! shipped keymap does not need one).

use std::fmt;

/// One key atom on 04 §2.0's legacy floor. Deliberately NOT crossterm's
/// `KeyCode`: core stays free of the input stack, and the TUI does the one
/// conversion at the edge (`tui/src/keys.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// Printable ASCII. An uppercase letter IS the Shift+letter atom.
    Char(char),
    /// `ctrl+<a-z>` and `ctrl+]`.
    Ctrl(char),
    Enter,
    /// The ONE atom off the legacy floor: terminals without the kitty
    /// disambiguate tier report it as a bare `Enter`. Every binding that uses
    /// it is gated on `Ctx::rich_keys`, so on a terminal that cannot tell the
    /// two apart the key is unbound and unhinted rather than quietly wrong
    /// (`shift_enter_is_inert_without_rich_keys`).
    ShiftEnter,
    Esc,
    Tab,
    BackTab,
    Space,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Key::Char(c) => write!(f, "{c}"),
            Key::Ctrl(c) => write!(f, "^{}", c.to_ascii_uppercase()),
            Key::Enter => write!(f, "enter"),
            Key::ShiftEnter => write!(f, "shift+enter"),
            Key::Esc => write!(f, "esc"),
            Key::Tab => write!(f, "tab"),
            Key::BackTab => write!(f, "shift+tab"),
            Key::Space => write!(f, "space"),
            Key::Backspace => write!(f, "backspace"),
            Key::Delete => write!(f, "delete"),
            Key::Left => write!(f, "←"),
            Key::Right => write!(f, "→"),
            Key::Up => write!(f, "↑"),
            Key::Down => write!(f, "↓"),
            Key::Home => write!(f, "home"),
            Key::End => write!(f, "end"),
            Key::PageUp => write!(f, "pgup"),
            Key::PageDown => write!(f, "pgdn"),
        }
    }
}

/// Which keymap owns the frame. `Global` is the parent of every screen scope;
/// `Input` is a barrier that inherits nothing (04 §2.0 rule 3), and so is
/// `DiffView`, which is the second half of the `z` chord.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Global,
    Board,
    Ticket,
    Diff,
    /// After `z` in the diff viewer — a chord tail, not a screen.
    DiffView,
    /// After `d` on the board or ticket screen — a chord tail, not a screen.
    /// Deleting takes two deliberate presses (`d d`), so a stray `d` on a
    /// card costs nothing.
    DeleteChord,
    /// After `a`, when `a` would archive. Same reason as the delete chord: a
    /// card leaving the board is worth one deliberate second press. Restoring
    /// does NOT come through here — undoing a mistake must never be harder
    /// than making it.
    ArchiveChord,
    /// After `^t` on the board, the ticket screen, or inside the composer —
    /// a chord tail, not a screen. `^t` rather than `t` because this must
    /// work while a title is being typed, and a Ctrl-letter is the only
    /// legacy-floor atom that a text field cannot swallow (`keys.rs` matches
    /// Ctrl before the printable arm). `Ctrl+<digit>` is a banned atom
    /// (`no_banned_atoms`) and its one whitelisted spelling `Ctrl+5` is
    /// already `Back`, so the digits live one press inside this tail.
    TagChord,
    Move,
    /// The Esc menu: everything that acts on the board as a whole, plus the
    /// two lists that are not the board. Board-wide actions deliberately have
    /// no key of their own — they are rare, they are not about the selection,
    /// and a menu row can say what it will do in full words.
    Menu,
    Drawer,
    Archived,
    /// Scope barrier: owns every key, inherits nothing.
    Input,
}

impl Scope {
    /// The scope a key falls through to when this one does not bind it.
    pub fn parent(self) -> Option<Scope> {
        match self {
            Scope::Board
            | Scope::Ticket
            | Scope::Diff
            | Scope::Move
            | Scope::Menu
            | Scope::Drawer
            | Scope::Archived => Some(Scope::Global),
            Scope::Global
            | Scope::DiffView
            | Scope::DeleteChord
            | Scope::ArchiveChord
            | Scope::TagChord
            | Scope::Input => None,
        }
    }

    /// The word the footer wears.
    pub fn word(self) -> &'static str {
        match self {
            Scope::Global => "MESIMON",
            Scope::Board => "BOARD",
            Scope::Ticket => "TICKET",
            Scope::Diff | Scope::DiffView => "DIFF",
            Scope::DeleteChord => "DELETE",
            Scope::ArchiveChord => "ARCHIVE",
            Scope::TagChord => "TAG",
            Scope::Move => "MOVE",
            Scope::Menu => "MENU",
            Scope::Drawer => "EXTERNAL",
            Scope::Archived => "ARCHIVED",
            Scope::Input => "INPUT",
        }
    }
}

/// Every action the keymap can name. The TUI matches this exhaustively, which
/// is what proves the table and the handler agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    // ---- global ----
    Help,
    NextAttention,
    PrevAttention,
    Reload,
    Redraw,
    Suspend,
    // ---- navigation (target resolved by scope) ----
    CursorLeft,
    CursorRight,
    CursorUp,
    CursorDown,
    First,
    Last,
    /// Enter: act on the selection. Board = get me working, ticket = focus,
    /// drawer = adopt + resume, archived = open.
    Act,
    /// Leave this screen. Board = quit mesimon.
    Back,
    Quit,
    // ---- board ----
    OpenTicket,
    TicketScreen,
    Rename,
    /// `d` — arms the delete chord; the next `d` deletes, `D` discards too.
    DeletePrefix,
    Delete,
    DeleteDiscard,
    Undo,
    /// `>` / `<` — the handler reads which one off the key.
    Grab,
    Peek,
    /// Esc on the board — opens the menu below.
    Menu,
    ExternalDrawer,
    ArchivedList,
    // ---- sessions ----
    Claude,
    Shell,
    ClaudeNew,
    ShellNew,
    Sleep,
    SleepAllDone,
    Pin,
    // ---- ticket lifecycle ----
    /// `a` — arms the archive chord, or restores at once when the ticket is
    /// already archived.
    ArchivePrefix,
    Archive,
    ArchiveAllDone,
    /// `^t` — open the tag tail. Works on the board, the ticket screen, and
    /// inside the composer.
    TagPrefix,
    /// Move the picker cursor. The direction is read off the key, the way
    /// the board's `hjkl` motions are.
    TagLeft,
    TagRight,
    TagUp,
    TagDown,
    /// A digit: jump to that group's row, and step along it on a repeat.
    TagGroup,
    /// Put the cell's tag on the ticket (or take it off again).
    TagToggle,
    /// Cycle the cell's tag through the tint ramp.
    TagColor,
    /// Rename the cell's tag.
    TagRename,
    /// Delete the cell's tag from the registry and every ticket. Two presses.
    TagForget,
    /// Leave the picker.
    TagDone,
    Merge,
    OpenDiff,
    // ---- diff ----
    ScrollDown,
    ScrollUp,
    PageDown,
    PageUp,
    NextFile,
    PrevFile,
    Refresh,
    ViewPrefix,
    Density,
    SwapPanes,
    WorktreeShell,
    // ---- move ----
    Drop,
    Cancel,
    /// `1`–`9` — the handler reads the digit off the key.
    DropColumn,
    // ---- drawer ----
    AdoptObserve,
    // ---- input ----
    Save,
    /// Shift+Enter in the composer: save AND start claude on the new ticket
    /// with the title as its first prompt, submitted.
    SaveStart,
    CycleWorkspace,
    EditLeft,
    EditRight,
    EditWordLeft,
    EditWordRight,
    EditHome,
    EditEnd,
    EditBackspace,
    EditDelete,
    EditDeleteWord,
    EditKillToStart,
}

/// 04 §2.0's legend. `Grace` actions land in the undo band; `Arm` actions name
/// what the next press does and only then perform it (the `m` idiom).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Plain,
    Grace,
    Arm,
}

/// Overlay grouping — the `?` sections, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    Navigate,
    Ticket,
    Sessions,
    Worktree,
    View,
    App,
}

impl Group {
    pub fn title(self) -> &'static str {
        match self {
            Group::Navigate => "MOVE AROUND",
            Group::Ticket => "TICKETS",
            Group::Sessions => "SESSIONS",
            Group::Worktree => "BRANCH",
            Group::View => "VIEW",
            Group::App => "APP",
        }
    }

    pub const ALL: [Group; 6] =
        [Group::Navigate, Group::Ticket, Group::Sessions, Group::Worktree, Group::View, Group::App];
}

/// What the screen can currently do. Every availability predicate and every
/// state-dependent hint word reads from this and nothing else, so the footer,
/// the `?` overlay and the key dispatch can never disagree about whether an
/// action applies.
#[derive(Debug, Clone)]
pub struct Ctx {
    // ---- board selection ----
    /// A card is under the cursor. Without it there is nothing to rename,
    /// move, delete, archive or start a session on.
    pub has_ticket: bool,
    /// More than one column exists — otherwise there is nowhere to move to.
    pub multi_column: bool,
    /// The selected ticket has at least one session record.
    pub ticket_has_sessions: bool,
    /// The selected ticket has a live claude session.
    pub ticket_has_claude: bool,
    /// At least one of the selected ticket's sessions is awake.
    pub ticket_awake: bool,
    /// The selected ticket is archived.
    pub ticket_archived: bool,
    /// A running or waiting claude — Enter would focus it rather than open
    /// the ticket page.
    pub ticket_hot: bool,
    // ---- board affordances ----
    pub can_undo: bool,
    /// What `u` would undo right now — "undo delete", "undo archive".
    pub undo_word: &'static str,
    pub bulk_sleep: usize,
    /// What sleeping those sessions would hand back. The suggestion does not
    /// gate on it — the menu row's detail spends it as the payoff word.
    pub bulk_sleep_bytes: u64,
    pub bulk_archive: usize,
    pub has_archived: bool,
    pub peek_on: bool,
    pub update_ready: bool,
    pub any_attention: bool,
    // ---- ticket screen ----
    /// The rail has a selected session.
    pub sel_session: bool,
    pub sel_sleeping: bool,
    pub sel_dead: bool,
    pub sel_pinned: bool,
    // ---- worktree ----
    pub has_worktree: bool,
    /// `m` would actually do something on the next press.
    pub merge_actionable: bool,
    pub merge_word: &'static str,
    // ---- diff ----
    pub two_pane: bool,
    pub worktree_present: bool,
    pub density_word: &'static str,
    // ---- text input ----
    /// The field is naming a NEW ticket, not renaming one. Only then is the
    /// workspace still open to change (it locks the moment work starts).
    pub composing: bool,
    // ---- tags ----
    /// A tag name is being typed. While true every binding in the tag tail
    /// stands down, so the digits are text and not axis picks.
    pub tag_naming: bool,
    /// The cursor is on a real tag, not the `+ new` cell — so there is
    /// something to wear, recolour, rename or delete.
    pub tag_on_entry: bool,
    /// The cursor's tag is already on this ticket, so Enter takes it off.
    pub tag_worn: bool,
    /// `d` is armed: the next `d` deletes that tag board-wide.
    pub tag_forget_armed: bool,
    // ---- terminal ----
    /// The terminal answered the kitty-protocol probe, so `Shift+Enter` is
    /// distinguishable from `Enter`. False on the legacy floor, where every
    /// binding on `Key::ShiftEnter` must stay inert and unhinted.
    pub rich_keys: bool,
}

impl Default for Ctx {
    /// The two word-valued fields default to real words, not `""` — a hint
    /// that renders empty would be a bound key with nothing to say, which
    /// `every_binding_is_spelled` rejects.
    fn default() -> Self {
        Self {
            has_ticket: false,
            multi_column: false,
            ticket_has_sessions: false,
            ticket_has_claude: false,
            ticket_awake: false,
            ticket_archived: false,
            ticket_hot: false,
            can_undo: false,
            undo_word: "undo",
            bulk_sleep: 0,
            bulk_sleep_bytes: 0,
            bulk_archive: 0,
            has_archived: false,
            peek_on: false,
            update_ready: false,
            any_attention: false,
            sel_session: false,
            sel_sleeping: false,
            sel_dead: false,
            sel_pinned: false,
            has_worktree: false,
            merge_actionable: false,
            merge_word: "merge",
            two_pane: false,
            worktree_present: false,
            density_word: "context lines",
            composing: false,
            tag_naming: false,
            tag_on_entry: false,
            tag_worn: false,
            tag_forget_armed: false,
            rich_keys: false,
        }
    }
}

/// A hint word, or `""` to stay silent while still being bound.
type Hint = fn(&Ctx) -> &'static str;
/// Whether the binding applies right now.
type Avail = fn(&Ctx) -> bool;

pub struct Binding {
    /// Every atom that triggers this verb, primary first.
    pub keys: &'static [Key],
    pub verb: Verb,
    /// How the key is spelled in hints — `"hjkl"`, `"> <"`, `"D"`. A chord
    /// prefix shows only itself (`d`, `a`): pressing it swaps the footer to
    /// the tail's own scope, which then names the key still to press. Nothing
    /// ever renders `d d`.
    pub show: &'static str,
    pub hint: Hint,
    pub avail: Avail,
    pub class: Class,
    pub group: Group,
    /// Does this action change board state? The `--observer` client (D22) is
    /// the `mutates == false` subset, generated from this field.
    pub mutates: bool,
    /// Footer priority: lower comes first, 0 means overlay-only. The footer
    /// fills to the terminal width in this order.
    pub prio: u8,
}

const fn always(_: &Ctx) -> bool {
    true
}

// ---------------------------------------------------------------------------
// The table
// ---------------------------------------------------------------------------

static GLOBAL: &[Binding] = &[
    Binding {
        keys: &[Key::Char('?')],
        verb: Verb::Help,
        show: "?",
        hint: |_| "keys",
        avail: always,
        class: Class::Plain,
        group: Group::App,
        mutates: false,
        prio: 0,
    },
    Binding {
        keys: &[Key::Tab],
        verb: Verb::NextAttention,
        show: "tab",
        hint: |_| "needs you",
        avail: |c| c.any_attention,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 20,
    },
    Binding {
        keys: &[Key::BackTab],
        verb: Verb::PrevAttention,
        show: "shift+tab",
        hint: |_| "previous needs you",
        avail: |c| c.any_attention,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 0,
    },
    Binding {
        keys: &[Key::Char('U')],
        verb: Verb::Reload,
        show: "U",
        // The header carries this offer; the footer stays out of it.
        hint: |_| "reload the new build",
        avail: |c| c.update_ready,
        class: Class::Plain,
        group: Group::App,
        mutates: false,
        prio: 0,
    },
    Binding {
        keys: &[Key::Char('u')],
        verb: Verb::Undo,
        show: "u",
        hint: |c| c.undo_word,
        avail: |c| c.can_undo,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 15,
    },
    Binding {
        keys: &[Key::Ctrl('l')],
        verb: Verb::Redraw,
        show: "^L",
        hint: |_| "redraw",
        avail: always,
        class: Class::Plain,
        group: Group::App,
        mutates: false,
        prio: 0,
    },
    Binding {
        keys: &[Key::Ctrl('z')],
        verb: Verb::Suspend,
        show: "^Z",
        hint: |_| "suspend mesimon",
        avail: always,
        class: Class::Plain,
        group: Group::App,
        mutates: false,
        prio: 0,
    },
];

static BOARD: &[Binding] = &[
    Binding {
        // Bare arrows are aliases of hjkl (04 §2.0's atom list, §2.4's table).
        // `directional()` reads the axis back off whichever atom arrived.
        keys: &[
            Key::Char('h'),
            Key::Left,
            Key::Char('l'),
            Key::Right,
            Key::Char('j'),
            Key::Down,
            Key::Char('k'),
            Key::Up,
        ],
        verb: Verb::CursorLeft, // placeholder; resolve() re-reads the key
        show: "hjkl",
        hint: |_| "move around",
        avail: always,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 0,
    },
    Binding {
        keys: &[Key::Char('g')],
        verb: Verb::First,
        show: "g",
        hint: |_| "top of column",
        avail: |c| c.has_ticket,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 0,
    },
    Binding {
        keys: &[Key::Char('G')],
        verb: Verb::Last,
        show: "G",
        hint: |_| "bottom of column",
        avail: |c| c.has_ticket,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 0,
    },
    Binding {
        keys: &[Key::Enter],
        verb: Verb::Act,
        show: "enter",
        // Enter is "get me working": it focuses a live agent when there is
        // one, and the hint says which it will be BEFORE the press.
        hint: |c| {
            if c.ticket_hot {
                "go to the agent"
            } else {
                "ticket page"
            }
        },
        avail: |c| c.has_ticket,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 10,
    },
    Binding {
        keys: &[Key::Space],
        verb: Verb::TicketScreen,
        show: "space",
        hint: |_| "ticket page, even past a live agent",
        avail: |c| c.has_ticket,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 0,
    },
    Binding {
        keys: &[Key::Char('o')],
        verb: Verb::OpenTicket,
        show: "o",
        hint: |_| "open ticket",
        avail: always,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 30,
    },
    Binding {
        keys: &[Key::Char('c')],
        verb: Verb::Claude,
        show: "c",
        hint: |c| {
            if c.ticket_has_claude {
                "claude"
            } else {
                "start claude"
            }
        },
        avail: |c| c.has_ticket,
        class: Class::Plain,
        group: Group::Sessions,
        mutates: true,
        prio: 40,
    },
    Binding {
        keys: &[Key::Char('s')],
        verb: Verb::Shell,
        show: "s",
        hint: |_| "shell",
        avail: |c| c.has_ticket,
        class: Class::Plain,
        group: Group::Sessions,
        mutates: true,
        prio: 50,
    },
    Binding {
        keys: &[Key::Char('>'), Key::Char('<')],
        verb: Verb::Grab,
        show: "> <",
        hint: |_| "move card",
        // The user's rule: no selection, no move — and no second column to
        // move to means the same thing.
        avail: |c| c.has_ticket && c.multi_column,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 60,
    },
    Binding {
        keys: &[Key::Char('r')],
        verb: Verb::Rename,
        show: "r",
        hint: |_| "rename",
        avail: |c| c.has_ticket,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 70,
    },
    Binding {
        keys: &[Key::Char('x')],
        verb: Verb::Sleep,
        show: "x",
        hint: |c| {
            if c.ticket_awake {
                "sleep sessions"
            } else {
                "wake sessions"
            }
        },
        avail: |c| c.has_ticket && c.ticket_has_sessions,
        class: Class::Plain,
        group: Group::Sessions,
        mutates: true,
        prio: 80,
    },
    Binding {
        keys: &[Key::Ctrl('t')],
        verb: Verb::TagPrefix,
        show: "^t",
        hint: |_| "tags",
        avail: |c| c.has_ticket,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 110,
    },
    Binding {
        keys: &[Key::Char('a')],
        verb: Verb::ArchivePrefix,
        show: "a",
        hint: |c| {
            if c.ticket_archived {
                "restore"
            } else {
                "archive"
            }
        },
        avail: |c| c.has_ticket,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 90,
    },
    Binding {
        keys: &[Key::Char('d')],
        verb: Verb::DeletePrefix,
        show: "d",
        hint: |_| "delete",
        avail: |c| c.has_ticket,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: false,
        prio: 100,
    },
    Binding {
        keys: &[Key::Char('p')],
        verb: Verb::Peek,
        show: "p",
        hint: |c| {
            if c.peek_on {
                "hide replies"
            } else {
                "show replies"
            }
        },
        avail: always,
        class: Class::Plain,
        group: Group::View,
        mutates: false,
        prio: 120,
    },
    Binding {
        keys: &[Key::Esc],
        verb: Verb::Menu,
        show: "esc",
        hint: |_| "menu",
        avail: always,
        class: Class::Plain,
        group: Group::App,
        mutates: false,
        prio: 200,
    },
    Binding {
        keys: &[Key::Char('q'), Key::Ctrl('c')],
        verb: Verb::Quit,
        show: "q",
        hint: |_| "quit",
        avail: always,
        class: Class::Plain,
        group: Group::App,
        mutates: false,
        prio: 250,
    },
];

static TICKET: &[Binding] = &[
    Binding {
        // A vertical list takes ↓ ↑ and nothing sideways.
        keys: &[Key::Char('j'), Key::Down, Key::Char('k'), Key::Up],
        verb: Verb::CursorDown,
        show: "jk",
        hint: |_| "select session",
        avail: |c| c.ticket_has_sessions,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 10,
    },
    Binding {
        keys: &[Key::Enter],
        verb: Verb::Act,
        show: "enter",
        hint: |c| {
            if c.sel_dead {
                "resume"
            } else if c.sel_sleeping {
                "wake"
            } else {
                "focus"
            }
        },
        avail: |c| c.sel_session,
        class: Class::Plain,
        group: Group::Sessions,
        mutates: true,
        prio: 20,
    },
    Binding {
        keys: &[Key::Char('c')],
        verb: Verb::Claude,
        show: "c",
        hint: |c| {
            if c.ticket_has_claude {
                "claude"
            } else {
                "start claude"
            }
        },
        avail: always,
        class: Class::Plain,
        group: Group::Sessions,
        mutates: true,
        prio: 30,
    },
    Binding {
        keys: &[Key::Char('s')],
        verb: Verb::Shell,
        show: "s",
        hint: |_| "shell",
        avail: always,
        class: Class::Plain,
        group: Group::Sessions,
        mutates: true,
        prio: 40,
    },
    Binding {
        keys: &[Key::Char('C')],
        verb: Verb::ClaudeNew,
        show: "C",
        hint: |_| "another claude",
        avail: always,
        class: Class::Plain,
        group: Group::Sessions,
        mutates: true,
        prio: 0,
    },
    Binding {
        keys: &[Key::Char('S')],
        verb: Verb::ShellNew,
        show: "S",
        hint: |_| "another shell",
        avail: always,
        class: Class::Plain,
        group: Group::Sessions,
        mutates: true,
        prio: 0,
    },
    Binding {
        keys: &[Key::Char('x')],
        verb: Verb::Sleep,
        show: "x",
        hint: |c| if c.sel_sleeping { "wake" } else { "sleep" },
        avail: |c| c.sel_session,
        class: Class::Plain,
        group: Group::Sessions,
        mutates: true,
        prio: 50,
    },
    Binding {
        keys: &[Key::Char('P')],
        verb: Verb::Pin,
        show: "P",
        hint: |c| {
            if c.sel_pinned {
                "let it sleep"
            } else {
                "keep awake"
            }
        },
        avail: |c| c.sel_session,
        class: Class::Plain,
        group: Group::Sessions,
        mutates: true,
        prio: 0,
    },
    Binding {
        keys: &[Key::Char('v')],
        verb: Verb::OpenDiff,
        show: "v",
        hint: |_| "diff",
        avail: |c| c.has_worktree,
        class: Class::Plain,
        group: Group::Worktree,
        mutates: false,
        prio: 60,
    },
    Binding {
        keys: &[Key::Char('m')],
        verb: Verb::Merge,
        show: "m",
        // The identity line renders this next to the branch state it acts on,
        // which is why the footer never asks for it (prio 0).
        //
        // The one binding that stays live while unhinted. Everywhere else
        // "not available" means inert, because there is nothing useful to
        // say; here there is — "the agent is still working", "no branch on
        // this ticket" — and answering beats silence for the press that comes
        // from muscle memory. The invariant that matters is unbroken: a key
        // that is HINTED always works.
        hint: |c| if c.merge_actionable { c.merge_word } else { "" },
        avail: always,
        class: Class::Arm,
        group: Group::Worktree,
        mutates: true,
        prio: 0,
    },
    Binding {
        keys: &[Key::Char('r')],
        verb: Verb::Rename,
        show: "r",
        hint: |_| "rename",
        avail: always,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 70,
    },
    Binding {
        keys: &[Key::Ctrl('t')],
        verb: Verb::TagPrefix,
        show: "^t",
        hint: |_| "tags",
        avail: always,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 85,
    },
    Binding {
        keys: &[Key::Char('a')],
        verb: Verb::ArchivePrefix,
        show: "a",
        hint: |c| {
            if c.ticket_archived {
                "restore"
            } else {
                "archive"
            }
        },
        avail: always,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 0,
    },
    Binding {
        keys: &[Key::Char('d')],
        verb: Verb::DeletePrefix,
        show: "d",
        hint: |_| "delete",
        avail: always,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: false,
        prio: 0,
    },
    Binding {
        keys: &[Key::Char('q'), Key::Esc, Key::Ctrl(']'), Key::Ctrl('5')],
        verb: Verb::Back,
        show: "q",
        hint: |_| "board",
        avail: always,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 250,
    },
];

static DIFF: &[Binding] = &[
    Binding {
        // A vertical list takes ↓ ↑ and nothing sideways.
        keys: &[Key::Char('j'), Key::Down, Key::Char('k'), Key::Up],
        verb: Verb::ScrollDown,
        show: "jk",
        // The one place j/k does not move a selection. This screen is read,
        // not picked — so the reading keys stay under the fingers and the
        // hint says so out loud.
        hint: |_| "scroll",
        avail: always,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 10,
    },
    Binding {
        keys: &[Key::Char('n'), Key::Char('N')],
        verb: Verb::NextFile,
        show: "n N",
        hint: |_| "next / previous file",
        avail: always,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 20,
    },
    Binding {
        keys: &[Key::Char('}'), Key::Char('{'), Key::PageDown, Key::PageUp],
        verb: Verb::PageDown,
        show: "{ }",
        hint: |_| "page",
        avail: always,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 30,
    },
    Binding {
        keys: &[Key::Char('z')],
        verb: Verb::ViewPrefix,
        show: "z",
        hint: |_| "context, layout…",
        avail: always,
        class: Class::Plain,
        group: Group::View,
        mutates: false,
        prio: 35,
    },
    Binding {
        keys: &[Key::Char('R')],
        verb: Verb::Refresh,
        show: "R",
        hint: |_| "refresh",
        avail: always,
        class: Class::Plain,
        group: Group::View,
        mutates: false,
        prio: 40,
    },
    Binding {
        keys: &[Key::Char('!')],
        verb: Verb::WorktreeShell,
        show: "!",
        hint: |_| "shell here",
        avail: |c| c.worktree_present,
        class: Class::Plain,
        group: Group::Worktree,
        mutates: false,
        prio: 50,
    },
    Binding {
        keys: &[Key::Char('q'), Key::Esc],
        verb: Verb::Back,
        show: "q",
        hint: |_| "back",
        avail: always,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 250,
    },
];

static DIFF_VIEW: &[Binding] = &[
    Binding {
        keys: &[Key::Char('z')],
        verb: Verb::Density,
        show: "z",
        hint: |c| c.density_word,
        avail: always,
        class: Class::Plain,
        group: Group::View,
        mutates: false,
        prio: 10,
    },
    Binding {
        keys: &[Key::Char('s')],
        verb: Verb::SwapPanes,
        show: "s",
        hint: |_| "swap list / diff",
        avail: |c| !c.two_pane,
        class: Class::Plain,
        group: Group::View,
        mutates: false,
        prio: 20,
    },
];

/// The `d` chord tail. Nothing else is bound here: any other key cancels, so
/// the only way to delete is to mean it twice.
static DELETE: &[Binding] = &[
    Binding {
        keys: &[Key::Char('d')],
        verb: Verb::Delete,
        show: "d",
        hint: |_| "delete ticket",
        avail: always,
        class: Class::Grace,
        group: Group::Ticket,
        mutates: true,
        prio: 10,
    },
    Binding {
        keys: &[Key::Char('D')],
        verb: Verb::DeleteDiscard,
        show: "D",
        hint: |_| "delete + discard the branch",
        avail: |c| c.has_worktree,
        class: Class::Grace,
        group: Group::Worktree,
        mutates: true,
        prio: 20,
    },
];

/// The `a` chord tail, reached only when `a` would archive. Like the delete
/// chord, nothing else is bound here: any other key cancels.
static ARCHIVE: &[Binding] = &[Binding {
    keys: &[Key::Char('a')],
    verb: Verb::Archive,
    show: "a",
    hint: |_| "archive it",
    avail: always,
    class: Class::Grace,
    group: Group::Ticket,
    mutates: true,
    prio: 10,
}];

/// The `^t` chord tail. Like the delete and archive chords this inherits
/// nothing, so a stray key cancels rather than acting — but unlike them it is
/// a place you stay: a digit picks an axis and advances it, and the tail
/// holds so the next digit can pick another axis without a second `^t`.
///
/// Every binding stands down while `tag_naming`, which is what lets the same
/// digits be text inside the name field one keystroke later.
static TAG: &[Binding] = &[
    Binding {
        keys: &[
            Key::Char('h'),
            Key::Char('j'),
            Key::Char('k'),
            Key::Char('l'),
            Key::Left,
            Key::Down,
            Key::Up,
            Key::Right,
        ],
        verb: Verb::TagLeft,
        show: "hjkl",
        hint: |_| "move",
        avail: |c| !c.tag_naming,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 10,
    },
    Binding {
        keys: &[
            Key::Char('1'),
            Key::Char('2'),
            Key::Char('3'),
            Key::Char('4'),
            Key::Char('5'),
            Key::Char('6'),
            Key::Char('7'),
            Key::Char('8'),
            Key::Char('9'),
            Key::Char('0'),
        ],
        verb: Verb::TagGroup,
        show: "1-0",
        hint: |_| "group",
        avail: |c| !c.tag_naming,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 20,
    },
    Binding {
        keys: &[Key::Enter],
        verb: Verb::TagToggle,
        show: "enter",
        hint: |c| {
            if c.tag_naming {
                "save"
            } else if !c.tag_on_entry {
                "new tag"
            } else if c.tag_worn {
                "remove"
            } else {
                "add"
            }
        },
        avail: always,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 30,
    },
    Binding {
        keys: &[Key::Tab],
        verb: Verb::TagColor,
        show: "tab",
        hint: |_| "colour",
        avail: |c| !c.tag_naming && c.tag_on_entry,
        class: Class::Plain,
        group: Group::View,
        mutates: true,
        prio: 40,
    },
    Binding {
        keys: &[Key::Char('r')],
        verb: Verb::TagRename,
        show: "r",
        hint: |_| "rename",
        avail: |c| !c.tag_naming && c.tag_on_entry,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 50,
    },
    Binding {
        // Deleting reaches every ticket wearing the tag, so it takes two
        // deliberate presses — the same grace the `d` and `a` chords give a
        // single card, for a change with a wider blast radius.
        keys: &[Key::Char('d')],
        verb: Verb::TagForget,
        show: "d",
        hint: |c| {
            // `show` already prints the key; the footer composes "d <hint>".
            if c.tag_forget_armed {
                "again to delete it everywhere"
            } else {
                "delete"
            }
        },
        avail: |c| !c.tag_naming && c.tag_on_entry,
        class: Class::Grace,
        group: Group::Ticket,
        mutates: true,
        prio: 60,
    },
    Binding {
        // One Esc binding, two meanings, because an atom may not appear twice
        // in a scope whatever its `avail`. Naming backs out of the field;
        // otherwise it closes the picker.
        keys: &[Key::Esc, Key::Ctrl('t')],
        verb: Verb::TagDone,
        show: "esc",
        hint: |c| {
            if c.tag_naming {
                "cancel"
            } else {
                "done"
            }
        },
        avail: always,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 250,
    },
];

static MOVE: &[Binding] = &[
    Binding {
        // Bare arrows are aliases of hjkl (04 §2.0's atom list, §2.4's table).
        // `directional()` reads the axis back off whichever atom arrived.
        keys: &[
            Key::Char('h'),
            Key::Left,
            Key::Char('l'),
            Key::Right,
            Key::Char('j'),
            Key::Down,
            Key::Char('k'),
            Key::Up,
        ],
        verb: Verb::CursorLeft,
        show: "hjkl",
        hint: |_| "place",
        avail: always,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 10,
    },
    Binding {
        keys: &[
            Key::Char('1'),
            Key::Char('2'),
            Key::Char('3'),
            Key::Char('4'),
            Key::Char('5'),
            Key::Char('6'),
            Key::Char('7'),
            Key::Char('8'),
            Key::Char('9'),
        ],
        verb: Verb::DropColumn,
        show: "1-9",
        hint: |_| "column",
        avail: |c| c.multi_column,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: true,
        prio: 20,
    },
    Binding {
        keys: &[Key::Char('>'), Key::Char('<')],
        verb: Verb::Grab,
        show: "> <",
        hint: |_| "shift column",
        avail: always,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 0,
    },
    Binding {
        keys: &[Key::Enter],
        verb: Verb::Drop,
        show: "enter",
        hint: |_| "drop",
        avail: always,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 30,
    },
    Binding {
        keys: &[Key::Esc, Key::Char('q')],
        verb: Verb::Cancel,
        show: "esc",
        hint: |_| "cancel",
        avail: always,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: false,
        prio: 250,
    },
];

static MENU: &[Binding] = &[
    Binding {
        // A vertical list takes ↓ ↑ and nothing sideways.
        keys: &[Key::Char('j'), Key::Down, Key::Char('k'), Key::Up],
        verb: Verb::CursorDown,
        show: "jk",
        hint: |_| "select",
        avail: always,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 10,
    },
    Binding {
        keys: &[Key::Enter],
        verb: Verb::Act,
        show: "enter",
        hint: |_| "choose",
        avail: always,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: true,
        prio: 20,
    },
    Binding {
        keys: &[Key::Char('q'), Key::Esc],
        verb: Verb::Back,
        show: "esc",
        hint: |_| "close",
        avail: always,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 250,
    },
];

/// One row of the Esc menu. A row is a [`Verb`], not a key — which is how the
/// menu can offer actions that have no binding at all (the two lists) beside
/// actions that do (peek), showing the key so the menu teaches it.
pub struct MenuItem {
    pub verb: Verb,
    /// Full words: the menu has room the footer does not, and the counts the
    /// header chip carries are spelled out here.
    pub label: fn(&Ctx) -> String,
    /// A one-line consequence, or `""`.
    pub detail: fn(&Ctx) -> String,
    pub avail: Avail,
    /// The key that also does this, for the row's right edge.
    pub key: &'static str,
}

static MENU_ITEMS: &[MenuItem] = &[
    MenuItem {
        verb: Verb::Reload,
        label: |_| "Restart on the new build".into(),
        detail: |_| "a newer mesimon is on disk ∙ sessions keep running".into(),
        avail: |c| c.update_ready,
        key: "U",
    },
    MenuItem {
        verb: Verb::SleepAllDone,
        label: |c| format!("Sleep {} in done", plural(c.bulk_sleep, "agent")),
        detail: |c| match gib(c.bulk_sleep_bytes) {
            Some(g) => format!("frees ~{g:.1}GiB ∙ they wake where they left off"),
            None => "frees their memory ∙ they wake where they left off".into(),
        },
        avail: |c| c.bulk_sleep > 0,
        key: "",
    },
    MenuItem {
        verb: Verb::ArchiveAllDone,
        label: |c| format!("Archive {} in done", plural(c.bulk_archive, "ticket")),
        detail: |_| "clears the column ∙ restore any of them later".into(),
        avail: |c| c.bulk_archive > 0,
        key: "",
    },
    MenuItem {
        verb: Verb::ExternalDrawer,
        label: |_| "External sessions".into(),
        detail: |_| "claude sessions in this repo that mesimon did not start".into(),
        avail: always,
        key: "",
    },
    MenuItem {
        verb: Verb::ArchivedList,
        label: |_| "Archived tickets".into(),
        detail: |_| "off the board, still here".into(),
        avail: |c| c.has_archived,
        key: "",
    },
    MenuItem {
        verb: Verb::Peek,
        label: |c| {
            if c.peek_on {
                "Hide agent replies".into()
            } else {
                "Show agent replies".into()
            }
        },
        detail: |_| "the latest reply under the selected card".into(),
        avail: always,
        key: "p",
    },
    MenuItem {
        verb: Verb::Help,
        label: |_| "All keys on this screen".into(),
        detail: |_| String::new(),
        avail: always,
        key: "?",
    },
    MenuItem {
        verb: Verb::Quit,
        label: |_| "Quit mesimon".into(),
        detail: |_| "sessions keep running".into(),
        avail: always,
        key: "q",
    },
];

/// `3 agents`, `1 agent` — a count and its noun. Every suggestion carries a
/// number, and `1 tickets` in the header would be the first thing seen.
fn plural(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// Whole GiB, or None below a tenth of one — a payoff that rounds to
/// `~0.0GiB` is not a payoff, so the detail says it in words instead.
fn gib(bytes: u64) -> Option<f64> {
    (bytes >= 107_374_182).then(|| bytes as f64 / (1024.0 * 1024.0 * 1024.0))
}

/// A standing offer: something worth doing right now, named in the header's
/// right-hand cluster and taken from the Esc menu. Highest priority first.
///
/// A suggestion is NOT an independent surface — it is a pointer at a menu row.
/// Its availability is the row's own `avail`, so a suggestion can never be
/// shown without the menu offering the thing it suggests, and `menu_items`
/// floats the suggested rows to the top in this same order. The `›` mark is
/// the visual language that joins the two: chip and row wear it, and nothing
/// else does.
pub struct Suggestion {
    pub verb: Verb,
    /// The header's short form, with its count. Lowercase — it is a clause in
    /// the header line, not a title.
    pub headline: fn(&Ctx) -> String,
    /// The key that also takes it, echoed in the chip because a key that works
    /// right now should not need a menu to be found. Empty for the rest.
    pub key: &'static str,
}

static SUGGESTIONS: &[Suggestion] = &[
    Suggestion { verb: Verb::Reload, headline: |_| "update ready".into(), key: "U" },
    Suggestion {
        verb: Verb::SleepAllDone,
        headline: |c| format!("sleep {}", plural(c.bulk_sleep, "agent")),
        key: "",
    },
    Suggestion {
        verb: Verb::ArchiveAllDone,
        headline: |c| format!("archive {}", plural(c.bulk_archive, "ticket")),
        key: "",
    },
];

/// The menu row a verb belongs to, if any.
fn menu_row(verb: Verb) -> Option<&'static MenuItem> {
    MENU_ITEMS.iter().find(|m| m.verb == verb)
}

/// The standing offers right now, highest priority first. Each one's
/// availability IS its menu row's — that is the whole connection.
pub fn suggestions(ctx: &Ctx) -> Vec<&'static Suggestion> {
    SUGGESTIONS.iter().filter(|s| menu_row(s.verb).is_some_and(|m| (m.avail)(ctx))).collect()
}

/// Is this menu row the thing a header chip is pointing at? The menu marks
/// those rows, so the chip and the row are visibly one offer.
pub fn is_suggested(verb: Verb, ctx: &Ctx) -> bool {
    suggestions(ctx).iter().any(|s| s.verb == verb)
}

/// The menu rows that apply right now.
pub fn menu_items(ctx: &Ctx) -> Vec<&'static MenuItem> {
    // Suggested rows float to the top, in suggestion order, so Esc followed by
    // Enter takes the highest-priority offer without a single motion key.
    let top: Vec<&'static MenuItem> =
        suggestions(ctx).into_iter().filter_map(|s| menu_row(s.verb)).collect();
    let mut rows = top.clone();
    rows.extend(
        MENU_ITEMS.iter().filter(|m| (m.avail)(ctx) && !top.iter().any(|t| t.verb == m.verb)),
    );
    rows
}

static DRAWER: &[Binding] = &[
    Binding {
        // A vertical list takes ↓ ↑ and nothing sideways.
        keys: &[Key::Char('j'), Key::Down, Key::Char('k'), Key::Up],
        verb: Verb::CursorDown,
        show: "jk",
        hint: |_| "select",
        avail: always,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 10,
    },
    Binding {
        keys: &[Key::Enter],
        verb: Verb::Act,
        show: "enter",
        hint: |_| "adopt + resume here",
        avail: always,
        class: Class::Plain,
        group: Group::Sessions,
        mutates: true,
        prio: 20,
    },
    Binding {
        keys: &[Key::Char('o')],
        verb: Verb::AdoptObserve,
        show: "o",
        hint: |_| "adopt, observe only",
        avail: always,
        class: Class::Plain,
        group: Group::Sessions,
        mutates: true,
        prio: 30,
    },
    Binding {
        keys: &[Key::Char('q'), Key::Esc, Key::Char('e')],
        verb: Verb::Back,
        show: "esc",
        hint: |_| "close",
        avail: always,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 250,
    },
];

static ARCHIVED: &[Binding] = &[
    Binding {
        // A vertical list takes ↓ ↑ and nothing sideways.
        keys: &[Key::Char('j'), Key::Down, Key::Char('k'), Key::Up],
        verb: Verb::CursorDown,
        show: "jk",
        hint: |_| "select",
        avail: always,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 10,
    },
    Binding {
        keys: &[Key::Enter],
        verb: Verb::Act,
        show: "enter",
        hint: |_| "open",
        avail: always,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 20,
    },
    Binding {
        keys: &[Key::Char('a')],
        verb: Verb::Archive,
        show: "a",
        hint: |_| "restore",
        avail: always,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 30,
    },
    Binding {
        keys: &[Key::Char('q'), Key::Esc, Key::Char('V')],
        verb: Verb::Back,
        show: "esc",
        hint: |_| "close",
        avail: always,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 250,
    },
];

static INPUT: &[Binding] = &[
    Binding {
        keys: &[Key::Enter],
        verb: Verb::Save,
        show: "enter",
        hint: |_| "save",
        avail: always,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 10,
    },
    Binding {
        // Shift on the save axis: the same target, one step further —
        // Enter mints the ticket, Shift+Enter mints it AND puts an agent on
        // it with the title as the prompt it has already been asked. Only
        // when composing (a rename has nothing to start) and only when the
        // terminal can tell this key from Enter at all.
        keys: &[Key::ShiftEnter],
        verb: Verb::SaveStart,
        show: "shift+enter",
        hint: |_| "save + ask claude",
        avail: |c| c.composing && c.rich_keys,
        class: Class::Plain,
        group: Group::Sessions,
        mutates: true,
        prio: 15,
    },
    Binding {
        // Tags while the title is still being typed. A Ctrl-letter is the
        // only legacy-floor atom a text field cannot swallow, which is the
        // whole reason the tag key is `^t` and not `t`.
        keys: &[Key::Ctrl('t')],
        verb: Verb::TagPrefix,
        show: "^t",
        hint: |_| "tags",
        avail: |c| c.composing,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 35,
    },
    Binding {
        keys: &[Key::Esc],
        verb: Verb::Cancel,
        show: "esc",
        hint: |_| "cancel",
        avail: always,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: false,
        prio: 20,
    },
    Binding {
        keys: &[Key::BackTab],
        verb: Verb::CycleWorkspace,
        show: "shift+tab",
        // Set at creation because the choice locks the moment a session or a
        // worktree exists — this is the only place it is ever open.
        hint: |_| "shared checkout / own worktree",
        avail: |c| c.composing,
        class: Class::Plain,
        group: Group::Worktree,
        mutates: false,
        prio: 30,
    },
    Binding {
        keys: &[Key::Left],
        verb: Verb::EditLeft,
        show: "←",
        hint: |_| "",
        avail: always,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 0,
    },
    Binding {
        keys: &[Key::Right],
        verb: Verb::EditRight,
        show: "→",
        hint: |_| "",
        avail: always,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 0,
    },
    Binding {
        keys: &[Key::Home, Key::Ctrl('a')],
        verb: Verb::EditHome,
        show: "^A",
        hint: |_| "",
        avail: always,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 0,
    },
    Binding {
        keys: &[Key::End, Key::Ctrl('e')],
        verb: Verb::EditEnd,
        show: "^E",
        hint: |_| "",
        avail: always,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 0,
    },
    Binding {
        keys: &[Key::Backspace],
        verb: Verb::EditBackspace,
        show: "backspace",
        hint: |_| "",
        avail: always,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: false,
        prio: 0,
    },
    Binding {
        keys: &[Key::Delete],
        verb: Verb::EditDelete,
        show: "delete",
        hint: |_| "",
        avail: always,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: false,
        prio: 0,
    },
    Binding {
        keys: &[Key::Ctrl('w'), Key::Ctrl('h')],
        verb: Verb::EditDeleteWord,
        show: "^W",
        hint: |_| "",
        avail: always,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: false,
        prio: 0,
    },
    Binding {
        keys: &[Key::Ctrl('u')],
        verb: Verb::EditKillToStart,
        show: "^U",
        hint: |_| "",
        avail: always,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: false,
        prio: 0,
    },
];

/// Every binding declared directly in `scope` (not its parent).
pub fn bindings(scope: Scope) -> &'static [Binding] {
    match scope {
        Scope::Global => GLOBAL,
        Scope::Board => BOARD,
        Scope::Ticket => TICKET,
        Scope::Diff => DIFF,
        Scope::DiffView => DIFF_VIEW,
        Scope::DeleteChord => DELETE,
        Scope::ArchiveChord => ARCHIVE,
        Scope::TagChord => TAG,
        Scope::Move => MOVE,
        Scope::Menu => MENU,
        Scope::Drawer => DRAWER,
        Scope::Archived => ARCHIVED,
        Scope::Input => INPUT,
    }
}

/// `scope` then its parent chain.
pub fn chain(scope: Scope) -> Vec<Scope> {
    let mut out = vec![scope];
    let mut s = scope;
    while let Some(p) = s.parent() {
        out.push(p);
        s = p;
    }
    out
}

/// The one path from a keypress to an action. Returns `None` when the key is
/// unbound here OR bound but unavailable — an unavailable key does nothing,
/// exactly as its absent hint promises.
pub fn resolve(scope: Scope, key: Key, ctx: &Ctx) -> Option<Verb> {
    for s in chain(scope) {
        for b in bindings(s) {
            if b.keys.contains(&key) && (b.avail)(ctx) {
                return Some(directional(b.verb, key));
            }
        }
    }
    None
}

/// Multi-key bindings whose verb depends on which atom arrived. Keeping this
/// here (rather than in the handler) means the table stays the whole truth.
fn directional(verb: Verb, key: Key) -> Verb {
    match (verb, key) {
        (Verb::CursorLeft | Verb::CursorRight | Verb::CursorUp | Verb::CursorDown, k) => match k {
            Key::Char('h') | Key::Left => Verb::CursorLeft,
            Key::Char('l') | Key::Right => Verb::CursorRight,
            Key::Char('k') | Key::Up => Verb::CursorUp,
            _ => Verb::CursorDown,
        },
        (Verb::TagLeft | Verb::TagRight | Verb::TagUp | Verb::TagDown, k) => match k {
            Key::Char('h') | Key::Left => Verb::TagLeft,
            Key::Char('l') | Key::Right => Verb::TagRight,
            Key::Char('k') | Key::Up => Verb::TagUp,
            _ => Verb::TagDown,
        },
        (Verb::ScrollDown, Key::Char('k') | Key::Up) => Verb::ScrollUp,
        (Verb::PageDown, Key::Char('{') | Key::PageUp) => Verb::PageUp,
        (Verb::NextFile, Key::Char('N')) => Verb::PrevFile,
        (v, _) => v,
    }
}

/// The available bindings of a scope chain, most footer-worthy first. Bindings
/// with `prio == 0` are overlay-only and never appear here.
pub fn footer_items(scope: Scope, ctx: &Ctx) -> Vec<&'static Binding> {
    let mut out: Vec<&'static Binding> = Vec::new();
    for s in chain(scope) {
        for b in bindings(s) {
            if b.prio > 0 && (b.avail)(ctx) && !(b.hint)(ctx).is_empty() {
                out.push(b);
            }
        }
    }
    out.sort_by_key(|b| b.prio);
    out
}

/// The footer line for this scope and state, filled to `width` in priority
/// order. `? keys` is always the tail — the one hint that is never crowded
/// out, because it is how everything else is found.
pub fn footer(scope: Scope, ctx: &Ctx, width: usize) -> String {
    const SEP: &str = " ∙ ";
    const TAIL: &str = "? keys";
    let budget = width.saturating_sub(TAIL.len() + SEP.len());
    let mut line = String::new();
    for b in footer_items(scope, ctx) {
        let item = format!("{} {}", b.show, (b.hint)(ctx));
        let add = if line.is_empty() { item.chars().count() } else { item.chars().count() + 3 };
        if line.chars().count() + add > budget {
            continue;
        }
        if !line.is_empty() {
            line.push_str(SEP);
        }
        line.push_str(&item);
    }
    if line.is_empty() {
        return TAIL.to_string();
    }
    format!("{line}{SEP}{TAIL}")
}

/// One binding's spelling and word, for the places that name a single key in
/// running text (an empty column's nudge, the ticket screen's branch line).
/// Returns `None` when the verb does not apply here — the caller then says
/// nothing rather than naming a key that would do nothing.
pub fn hint_for(scope: Scope, verb: Verb, ctx: &Ctx) -> Option<(&'static str, &'static str)> {
    for s in chain(scope) {
        for b in bindings(s) {
            if b.verb == verb && (b.avail)(ctx) {
                let hint = (b.hint)(ctx);
                if hint.is_empty() {
                    return None;
                }
                return Some((b.show, hint));
            }
        }
    }
    None
}

/// Everything available right now, grouped for the `?` overlay. Unlike the
/// footer this includes `prio == 0` bindings — the overlay is the complete
/// answer, which is the whole reason it exists.
pub fn overlay(scope: Scope, ctx: &Ctx) -> Vec<(Group, Vec<(&'static str, &'static str)>)> {
    let mut out: Vec<(Group, Vec<(&'static str, &'static str)>)> = Vec::new();
    for g in Group::ALL {
        let mut rows: Vec<(&'static str, &'static str)> = Vec::new();
        for s in chain(scope) {
            for b in bindings(s) {
                if b.group != g || !(b.avail)(ctx) {
                    continue;
                }
                let hint = (b.hint)(ctx);
                if hint.is_empty() {
                    continue;
                }
                rows.push((b.show, hint));
            }
        }
        if !rows.is_empty() {
            out.push((g, rows));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_SCOPES: [Scope; 13] = [
        Scope::Global,
        Scope::Board,
        Scope::Ticket,
        Scope::Diff,
        Scope::DiffView,
        Scope::DeleteChord,
        Scope::ArchiveChord,
        Scope::TagChord,
        Scope::Move,
        Scope::Menu,
        Scope::Drawer,
        Scope::Archived,
        Scope::Input,
    ];

    /// 04 §2.0 rule 3: no key bound in both a scope and any ancestor, and none
    /// bound twice inside one scope. The shipped keymap declares no overrides,
    /// so this is absolute.
    #[test]
    fn no_key_bound_twice_in_a_chain() {
        for scope in ALL_SCOPES {
            let mut seen: Vec<(Key, Scope)> = Vec::new();
            for s in chain(scope) {
                for b in bindings(s) {
                    for k in b.keys {
                        if let Some((_, other)) = seen.iter().find(|(sk, _)| sk == k) {
                            panic!("{k:?} bound twice reaching {scope:?}: {s:?} and {other:?}");
                        }
                        seen.push((*k, s));
                    }
                }
            }
        }
    }

    /// 04 §2.0 rule 1: every atom is expressible on the legacy floor —
    /// `Key::ShiftEnter` excepted, and only because the test below proves it
    /// binds nothing on a terminal that cannot report it.
    #[test]
    fn every_atom_is_on_the_legacy_floor() {
        for scope in ALL_SCOPES {
            for b in bindings(scope) {
                for k in b.keys {
                    let ok = match k {
                        Key::Char(c) => c.is_ascii_graphic() || *c == ' ',
                        // ctrl+<a-z> plus the two ctrl+] spellings (0x1D
                        // arrives as Ctrl+5 on terminals without kitty).
                        Key::Ctrl(c) => c.is_ascii_lowercase() || *c == ']' || *c == '5',
                        Key::ShiftEnter => false,
                        _ => true,
                    };
                    assert!(
                        ok || *k == Key::ShiftEnter,
                        "{k:?} in {scope:?} is not on the legacy floor"
                    );
                }
            }
        }
    }

    /// The legacy floor's escape clause, made load-bearing: an off-floor atom
    /// is only allowed because it disappears where the terminal cannot spell
    /// it. Without `rich_keys` every ShiftEnter binding must be unavailable —
    /// so the key is inert AND unhinted, and the terminal that reports it as a
    /// plain `Enter` gets exactly the plain `Enter` behaviour.
    #[test]
    fn shift_enter_is_inert_without_rich_keys() {
        let legacy = Ctx { composing: true, rich_keys: false, ..Default::default() };
        let rich = Ctx { composing: true, rich_keys: true, ..Default::default() };
        let mut found = false;
        for scope in ALL_SCOPES {
            for b in bindings(scope) {
                if b.keys.contains(&Key::ShiftEnter) {
                    found = true;
                    assert!(
                        !(b.avail)(&legacy),
                        "{:?} in {scope:?} offers ShiftEnter on the legacy floor",
                        b.verb
                    );
                }
            }
            assert_eq!(resolve(scope, Key::ShiftEnter, &legacy), None, "{scope:?}");
        }
        assert!(found, "no ShiftEnter binding left — drop the atom too");
        // And it does resolve where the terminal can spell it.
        assert_eq!(resolve(Scope::Input, Key::ShiftEnter, &rich), Some(Verb::SaveStart));
        // A rename is not a composition: nothing to start.
        let renaming = Ctx { composing: false, rich_keys: true, ..Default::default() };
        assert_eq!(resolve(Scope::Input, Key::ShiftEnter, &renaming), None);
    }

    /// Shift+Enter hardens Enter on the same target rather than switching
    /// verbs: plain Enter still saves, and it saves the SAME ticket.
    #[test]
    fn shift_enter_stays_on_the_save_axis() {
        let rich = Ctx { composing: true, rich_keys: true, ..Default::default() };
        assert_eq!(resolve(Scope::Input, Key::Enter, &rich), Some(Verb::Save));
        assert_eq!(
            hint_for(Scope::Input, Verb::SaveStart, &rich),
            Some(("shift+enter", "save + ask claude"))
        );
    }

    /// 04 §2.0 rule 2, the part that bit us: no Alt/Meta atom exists at all
    /// (there is no `Key::Alt` to construct), and no `ctrl+<digit>` other than
    /// the `Ctrl+5` that IS `ctrl+]` on legacy terminals.
    #[test]
    fn no_banned_atoms() {
        for scope in ALL_SCOPES {
            for b in bindings(scope) {
                for k in b.keys {
                    if let Key::Ctrl(c) = k {
                        assert!(
                            !c.is_ascii_digit() || *c == '5',
                            "ctrl+{c} in {scope:?} is a banned atom"
                        );
                    }
                }
            }
        }
    }

    /// The picker owns its scope: it inherits nothing, so a key it does not
    /// bind cancels rather than falling through to the board.
    #[test]
    fn tag_picker_is_a_barrier() {
        let ctx = Ctx { has_ticket: true, tag_on_entry: true, ..Default::default() };
        assert_eq!(resolve(Scope::Board, Key::Ctrl('t'), &ctx), Some(Verb::TagPrefix));
        assert_eq!(resolve(Scope::Ticket, Key::Ctrl('t'), &ctx), Some(Verb::TagPrefix));
        for (k, v) in [
            (Key::Char('h'), Verb::TagLeft),
            (Key::Char('l'), Verb::TagRight),
            (Key::Char('k'), Verb::TagUp),
            (Key::Char('j'), Verb::TagDown),
            (Key::Enter, Verb::TagToggle),
            (Key::Tab, Verb::TagColor),
            (Key::Char('r'), Verb::TagRename),
            (Key::Char('d'), Verb::TagForget),
            (Key::Esc, Verb::TagDone),
        ] {
            assert_eq!(resolve(Scope::TagChord, k, &ctx), Some(v), "{k:?}");
        }
        for d in ['0', '1', '5', '9'] {
            assert_eq!(resolve(Scope::TagChord, Key::Char(d), &ctx), Some(Verb::TagGroup), "{d}");
        }
        // Arrows alias the motions, as everywhere else.
        assert_eq!(resolve(Scope::TagChord, Key::Left, &ctx), Some(Verb::TagLeft));
        assert_eq!(resolve(Scope::TagChord, Key::Down, &ctx), Some(Verb::TagDown));
        // Nothing else binds.
        for k in [Key::Char('c'), Key::Char('?'), Key::Char('q'), Key::Char('x')] {
            assert_eq!(resolve(Scope::TagChord, k, &ctx), None, "{k:?}");
        }
        // On the `+ new` cell there is no tag to recolour, rename or delete.
        let empty = Ctx { has_ticket: true, ..Default::default() };
        for k in [Key::Tab, Key::Char('r'), Key::Char('d')] {
            assert_eq!(resolve(Scope::TagChord, k, &empty), None, "{k:?}");
        }
        assert_eq!(resolve(Scope::TagChord, Key::Enter, &empty), Some(Verb::TagToggle));
    }

    /// While a name is being typed the picker stands down and the field owns
    /// the keys — and the hints come from the TAG table, not from `INPUT`.
    /// Borrowing `Scope::Input` here leaked the composer's own hints
    /// ("shift+enter save + ask claude") into a field that does no such thing.
    #[test]
    fn naming_a_tag_shows_only_its_own_keys() {
        let naming =
            Ctx { has_ticket: true, tag_on_entry: true, tag_naming: true, ..Default::default() };
        // One atom, two meanings: the hint says which, and dispatch does it.
        assert_eq!(resolve(Scope::TagChord, Key::Enter, &naming), Some(Verb::TagToggle));
        assert_eq!(hint_for(Scope::TagChord, Verb::TagToggle, &naming), Some(("enter", "save")));
        assert_eq!(resolve(Scope::TagChord, Key::Esc, &naming), Some(Verb::TagDone));
        assert_eq!(hint_for(Scope::TagChord, Verb::TagDone, &naming), Some(("esc", "cancel")));
        // Every picker key is inert: the digits and letters are text now.
        for k in [
            Key::Char('1'),
            Key::Char('h'),
            Key::Char('j'),
            Key::Char('r'),
            Key::Char('d'),
            Key::Tab,
        ] {
            assert_eq!(resolve(Scope::TagChord, k, &naming), None, "{k:?}");
        }
        // And the composer's own hints are nowhere near this scope.
        let hints: Vec<&str> = overlay(Scope::TagChord, &naming)
            .into_iter()
            .flat_map(|(_, rows)| rows)
            .map(|(_, h)| h)
            .collect();
        assert!(!hints.iter().any(|h| h.contains("claude")), "{hints:?}");
        assert!(!hints.iter().any(|h| h.contains("worktree")), "{hints:?}");
    }

    /// `^t` reaches the composer. A bare letter could not: `INPUT` is a
    /// barrier that types anything it does not bind, and a Ctrl-letter is the
    /// only legacy-floor atom a text field cannot swallow. This is the whole
    /// reason the key is `^t`.
    #[test]
    fn tags_reach_the_composer() {
        let composing = Ctx { composing: true, ..Default::default() };
        assert_eq!(resolve(Scope::Input, Key::Ctrl('t'), &composing), Some(Verb::TagPrefix));
        // A rename is not a composition, but tags still make no sense there:
        // the gesture is for the ticket being made.
        let renaming = Ctx { composing: false, ..Default::default() };
        assert_eq!(resolve(Scope::Input, Key::Ctrl('t'), &renaming), None);
        // And the bare letter stays free for the field to type.
        assert_eq!(resolve(Scope::Input, Key::Char('t'), &composing), None);
    }

    /// Every binding is spelled and (unless deliberately silent) described.
    #[test]
    fn every_binding_is_spelled() {
        let ctx = Ctx::default();
        for scope in ALL_SCOPES {
            for b in bindings(scope) {
                assert!(!b.show.is_empty(), "{:?} in {scope:?} has no spelling", b.verb);
                assert!(!b.keys.is_empty(), "{:?} in {scope:?} binds nothing", b.verb);
                // A prio'd binding must say something, or the footer would
                // render a bare key with no meaning.
                if b.prio > 0 {
                    assert!(
                        !(b.hint)(&ctx).is_empty() || !(b.avail)(&ctx),
                        "{:?} in {scope:?} is in the footer with no hint",
                        b.verb
                    );
                }
            }
        }
    }

    /// The user's rule, encoded: with nothing selected, the board offers no
    /// verb that needs a selection. This is the test that would have caught
    /// "move" being hinted on an empty column.
    #[test]
    fn empty_board_hints_nothing_that_needs_a_ticket() {
        let ctx = Ctx { multi_column: true, ..Default::default() };
        let items = footer_items(Scope::Board, &ctx);
        let shown: Vec<&str> = items.iter().map(|b| b.show).collect();
        for absent in ["> <", "r", "d", "a", "c", "s", "x", "enter"] {
            assert!(!shown.contains(&absent), "{absent} hinted with no ticket selected: {shown:?}");
        }
        assert!(shown.contains(&"o"), "open-ticket must always be offered: {shown:?}");
        assert!(shown.contains(&"esc"), "the menu is always reachable: {shown:?}");
        assert!(shown.contains(&"q"));
    }

    /// And the other half: an unavailable key is inert, not merely unhinted.
    #[test]
    fn unavailable_keys_do_nothing() {
        let empty = Ctx { multi_column: true, ..Default::default() };
        for k in [Key::Char('>'), Key::Char('r'), Key::Char('d'), Key::Char('c')] {
            assert_eq!(resolve(Scope::Board, k, &empty), None, "{k:?} acted with no ticket");
        }
        let selected = Ctx { has_ticket: true, multi_column: true, ..Default::default() };
        assert_eq!(resolve(Scope::Board, Key::Char('>'), &selected), Some(Verb::Grab));
        // One column: nowhere to move to, so the grab stays inert.
        let one_col = Ctx { has_ticket: true, ..Default::default() };
        assert_eq!(resolve(Scope::Board, Key::Char('>'), &one_col), None);
    }

    /// The collisions this rework existed to remove. Each of these keys used
    /// to mean two different things depending on the screen.
    #[test]
    fn one_verb_one_key_across_screens() {
        let ctx = Ctx {
            has_ticket: true,
            ticket_has_sessions: true,
            sel_session: true,
            multi_column: true,
            has_worktree: true,
            merge_actionable: true,
            merge_word: "merge",
            ..Default::default()
        };
        for (key, verb) in [
            (Key::Char('c'), Verb::Claude),
            (Key::Char('s'), Verb::Shell),
            (Key::Char('r'), Verb::Rename),
            (Key::Char('d'), Verb::DeletePrefix),
            (Key::Char('a'), Verb::ArchivePrefix),
            (Key::Char('x'), Verb::Sleep),
        ] {
            assert_eq!(resolve(Scope::Board, key, &ctx), Some(verb), "board {key:?}");
            assert_eq!(resolve(Scope::Ticket, key, &ctx), Some(verb), "ticket {key:?}");
        }
        // `m` is merge and nothing else — the board grab is `>` / `<`.
        assert_eq!(resolve(Scope::Ticket, Key::Char('m'), &ctx), Some(Verb::Merge));
        assert_eq!(resolve(Scope::Board, Key::Char('m'), &ctx), None);
        // Merge stays pressable when it cannot merge, so it can say why — but
        // it is not hinted then, which is the invariant that matters.
        let quiet = Ctx { has_worktree: true, ..Default::default() };
        assert_eq!(resolve(Scope::Ticket, Key::Char('m'), &quiet), Some(Verb::Merge));
        assert_eq!(hint_for(Scope::Ticket, Verb::Merge, &quiet), None);
        assert!(hint_for(Scope::Ticket, Verb::Merge, &ctx).is_some());
        // Kill is gone entirely: `x` is the only way to stop a session, and it
        // is reversible.
        assert!(!bindings(Scope::Ticket)
            .iter()
            .any(|b| b.class == Class::Arm && b.verb == Verb::Sleep));
    }

    /// Deleting takes two deliberate presses, and the branch-discarding form
    /// is only reachable from inside the chord.
    #[test]
    fn delete_is_a_chord() {
        let ctx = Ctx { has_ticket: true, has_worktree: true, ..Default::default() };
        assert_eq!(resolve(Scope::Board, Key::Char('d'), &ctx), Some(Verb::DeletePrefix));
        assert_eq!(resolve(Scope::DeleteChord, Key::Char('d'), &ctx), Some(Verb::Delete));
        assert_eq!(resolve(Scope::DeleteChord, Key::Char('D'), &ctx), Some(Verb::DeleteDiscard));
        // No worktree, no discard form.
        let bare = Ctx { has_ticket: true, ..Default::default() };
        assert_eq!(resolve(Scope::DeleteChord, Key::Char('D'), &bare), None);
        // A stray key inside the chord resolves to nothing, so the handler
        // cancels rather than acting.
        assert_eq!(resolve(Scope::DeleteChord, Key::Char('x'), &ctx), None);
    }

    /// Archiving takes two presses; restoring takes one. The chord tail binds
    /// nothing but `a`, so any other key inside it cancels.
    #[test]
    fn archive_is_a_chord_but_restore_is_not() {
        let live = Ctx { has_ticket: true, ..Default::default() };
        assert_eq!(resolve(Scope::Board, Key::Char('a'), &live), Some(Verb::ArchivePrefix));
        assert_eq!(resolve(Scope::ArchiveChord, Key::Char('a'), &live), Some(Verb::Archive));
        assert_eq!(resolve(Scope::ArchiveChord, Key::Char('x'), &live), None);
        // Already archived: still the prefix verb, but the hint says restore
        // and the handler acts at once — one press to undo a mistake.
        let gone = Ctx { has_ticket: true, ticket_archived: true, ..Default::default() };
        assert_eq!(hint_for(Scope::Board, Verb::ArchivePrefix, &gone), Some(("a", "restore")));
        assert_eq!(hint_for(Scope::Board, Verb::ArchivePrefix, &live), Some(("a", "archive")));
    }

    /// A chord prefix advertises one key, never `d d` — pressing it swaps the
    /// footer to the tail, which names the key still to press.
    #[test]
    fn chord_prefixes_advertise_a_single_key() {
        for scope in ALL_SCOPES {
            for b in bindings(scope) {
                assert!(
                    !b.show.contains("d d")
                        && !b.show.contains("a a")
                        && !b.show.contains("z z")
                        && !b.show.contains("^t ^t")
                        && !b.show.contains("^t 1"),
                    "{:?} in {scope:?} spells a whole chord ({:?}) instead of one key",
                    b.verb,
                    b.show
                );
            }
        }
    }

    /// Undo is global: a delete from the ticket page is undoable wherever the
    /// user lands afterwards.
    #[test]
    fn undo_reaches_every_screen() {
        let ctx = Ctx { can_undo: true, ..Default::default() };
        for s in [Scope::Board, Scope::Ticket, Scope::Diff, Scope::Drawer, Scope::Menu] {
            assert_eq!(resolve(s, Key::Char('u'), &ctx), Some(Verb::Undo), "{s:?}");
        }
        let nothing = Ctx::default();
        assert_eq!(resolve(Scope::Board, Key::Char('u'), &nothing), None);
        // And it says which thing it would undo.
        let arch = Ctx { can_undo: true, undo_word: "undo archive", ..Default::default() };
        assert_eq!(hint_for(Scope::Board, Verb::Undo, &arch), Some(("u", "undo archive")));
    }

    /// Shift hardens or forces the same verb on the same target. It never
    /// switches verbs, and it is never how a board-wide action is reached.
    #[test]
    fn shift_stays_on_one_axis() {
        let t = Ctx { sel_session: true, ..Default::default() };
        assert_eq!(resolve(Scope::Ticket, Key::Char('c'), &t), Some(Verb::Claude));
        assert_eq!(resolve(Scope::Ticket, Key::Char('C'), &t), Some(Verb::ClaudeNew));
        assert_eq!(resolve(Scope::Ticket, Key::Char('s'), &t), Some(Verb::Shell));
        assert_eq!(resolve(Scope::Ticket, Key::Char('S'), &t), Some(Verb::ShellNew));
        // The old unrelated pairs are gone: no board key at all for the bulk
        // verbs, and none for the two lists.
        let full = Ctx {
            has_ticket: true,
            bulk_sleep: 3,
            bulk_archive: 3,
            has_archived: true,
            ..Default::default()
        };
        for retired in [Key::Char('X'), Key::Char('A'), Key::Char('V'), Key::Char('e')] {
            assert_eq!(resolve(Scope::Board, retired, &full), None, "{retired:?} is retired");
        }
    }

    /// Board-wide actions live in the menu, and the menu is one Esc away.
    #[test]
    fn menu_holds_the_board_wide_actions() {
        let ctx = Ctx {
            bulk_sleep: 2,
            bulk_archive: 2,
            has_archived: true,
            update_ready: true,
            ..Default::default()
        };
        assert_eq!(resolve(Scope::Board, Key::Esc, &ctx), Some(Verb::Menu));
        let verbs: Vec<Verb> = menu_items(&ctx).iter().map(|m| m.verb).collect();
        for v in [
            Verb::ExternalDrawer,
            Verb::ArchivedList,
            Verb::SleepAllDone,
            Verb::ArchiveAllDone,
            Verb::Help,
            Verb::Quit,
        ] {
            assert!(verbs.contains(&v), "{v:?} missing from the menu: {verbs:?}");
        }
        // Rows that do not apply stay out: nothing archived, nothing to sleep.
        let quiet = Ctx::default();
        let quiet_verbs: Vec<Verb> = menu_items(&quiet).iter().map(|m| m.verb).collect();
        assert!(!quiet_verbs.contains(&Verb::ArchivedList));
        assert!(!quiet_verbs.contains(&Verb::SleepAllDone));
        assert!(!quiet_verbs.contains(&Verb::Reload));
        // Every menu row that names a key must name one the keymap really has.
        for m in menu_items(&ctx) {
            if m.key.is_empty() {
                continue;
            }
            let found = chain(Scope::Board)
                .into_iter()
                .flat_map(bindings)
                .any(|b| b.show == m.key && b.verb == m.verb);
            assert!(found, "menu row {:?} names key {:?}, which is not bound", m.verb, m.key);
        }
    }

    /// A suggestion is a pointer at a menu row, and the menu is where it is
    /// taken. Enforced rather than reviewed: every suggestion names a verb the
    /// menu has, a suggestion is offered exactly when its row is, and the rows
    /// it points at sort first, in suggestion order. The header cannot offer
    /// something the menu will not do.
    #[test]
    fn every_suggestion_is_a_menu_row() {
        for s in SUGGESTIONS {
            assert!(
                MENU_ITEMS.iter().any(|m| m.verb == s.verb),
                "suggestion {:?} has no menu row to point at",
                s.verb
            );
        }
        let all = Ctx {
            bulk_sleep: 3,
            bulk_sleep_bytes: 3 << 30,
            bulk_archive: 2,
            has_archived: true,
            update_ready: true,
            ..Default::default()
        };
        // Declared priority, in the header and at the top of the menu alike.
        let heads: Vec<String> = suggestions(&all).iter().map(|s| (s.headline)(&all)).collect();
        assert_eq!(heads, ["update ready", "sleep 3 agents", "archive 2 tickets"]);
        let one = Ctx { bulk_sleep: 1, bulk_archive: 1, ..Default::default() };
        let heads: Vec<String> = suggestions(&one).iter().map(|s| (s.headline)(&one)).collect();
        assert_eq!(heads, ["sleep 1 agent", "archive 1 ticket"], "counts of one read as one");
        let rows: Vec<Verb> = menu_items(&all).iter().map(|m| m.verb).collect();
        assert_eq!(
            &rows[..3],
            &[Verb::Reload, Verb::SleepAllDone, Verb::ArchiveAllDone],
            "suggested rows must lead the menu, in suggestion order: {rows:?}"
        );
        // And each suggested row is offered exactly when its suggestion is.
        for ctx in [Ctx::default(), all.clone(), Ctx { update_ready: true, ..Default::default() }] {
            for m in MENU_ITEMS {
                let suggested = is_suggested(m.verb, &ctx);
                if suggested {
                    assert!(
                        menu_items(&ctx).iter().any(|r| r.verb == m.verb),
                        "{:?} is suggested but not in the menu",
                        m.verb
                    );
                }
                if SUGGESTIONS.iter().any(|s| s.verb == m.verb) {
                    assert_eq!(
                        suggested,
                        (m.avail)(&ctx),
                        "{:?} must be suggested exactly when its row applies",
                        m.verb
                    );
                }
            }
        }
        // Nothing to offer is the resting state: no chips, no marked rows.
        assert!(suggestions(&Ctx::default()).is_empty());
    }

    /// Every menu row says something, in both halves it fills. A row that
    /// renders empty words is a row that teaches nothing.
    #[test]
    fn every_menu_row_is_spelled() {
        let ctx = Ctx {
            bulk_sleep: 1,
            bulk_archive: 1,
            has_archived: true,
            update_ready: true,
            ..Default::default()
        };
        for m in MENU_ITEMS {
            assert!(!(m.label)(&ctx).is_empty(), "{:?} has no label", m.verb);
        }
        // The sleep payoff is spent in words when it would round to nothing,
        // and in GiB when it would not.
        let thin = Ctx { bulk_sleep: 1, ..Default::default() };
        let fat = Ctx { bulk_sleep: 1, bulk_sleep_bytes: 3 << 30, ..Default::default() };
        let sleep = MENU_ITEMS.iter().find(|m| m.verb == Verb::SleepAllDone).expect("row");
        assert!(!(sleep.detail)(&thin).contains("GiB"), "a ~0.0GiB payoff must not be claimed");
        assert!((sleep.detail)(&fat).contains("~3.0GiB"));
    }

    /// Bare arrows are aliases of the letter motions, everywhere the letters
    /// are bound (04 §2.0's atom list). This exists because the letters and
    /// the aliases were once written out by hand in each handler, and the
    /// move into the table dropped every arrow silently — nothing failed,
    /// the keys just stopped working.
    #[test]
    fn arrows_alias_the_letter_motions() {
        fn motion(v: Verb) -> bool {
            matches!(
                v,
                Verb::CursorLeft
                    | Verb::CursorRight
                    | Verb::CursorUp
                    | Verb::CursorDown
                    | Verb::ScrollUp
                    | Verb::ScrollDown
            )
        }
        let ctx = Ctx {
            has_ticket: true,
            multi_column: true,
            ticket_has_sessions: true,
            ..Default::default()
        };
        let mut checked = 0;
        for scope in ALL_SCOPES {
            if scope == Scope::Input {
                continue; // a text field's arrows move the cursor, not a list
            }
            for (letter, arrow) in
                [('h', Key::Left), ('l', Key::Right), ('k', Key::Up), ('j', Key::Down)]
            {
                let Some(v) = resolve(scope, Key::Char(letter), &ctx) else {
                    continue;
                };
                if !motion(v) {
                    continue;
                }
                assert_eq!(
                    resolve(scope, arrow, &ctx),
                    Some(v),
                    "{scope:?}: {letter} works but {arrow:?} does not"
                );
                checked += 1;
            }
        }
        // Guard the guard: if the motions are ever renamed out from under
        // this, it must fail rather than vacuously pass.
        assert!(checked >= 12, "only {checked} motion aliases checked — is `motion` stale?");
    }

    /// `q` pops one level and `?` is reachable from every screen — the two
    /// rules a new user needs before any other key matters.
    #[test]
    fn q_pops_and_help_is_everywhere() {
        let ctx = Ctx::default();
        assert_eq!(resolve(Scope::Board, Key::Char('q'), &ctx), Some(Verb::Quit));
        for s in [Scope::Ticket, Scope::Diff, Scope::Drawer, Scope::Archived, Scope::Menu] {
            assert_eq!(resolve(s, Key::Char('q'), &ctx), Some(Verb::Back), "{s:?}");
            assert_eq!(resolve(s, Key::Esc, &ctx), Some(Verb::Back), "{s:?}");
        }
        assert_eq!(resolve(Scope::Move, Key::Char('q'), &ctx), Some(Verb::Cancel));
        for s in ALL_SCOPES {
            if matches!(
                s,
                Scope::Input
                    | Scope::DiffView
                    | Scope::DeleteChord
                    | Scope::ArchiveChord
                    | Scope::TagChord
            ) {
                continue; // barrier scopes: four chord tails and a text field
            }
            assert_eq!(resolve(s, Key::Char('?'), &ctx), Some(Verb::Help), "{s:?}");
        }
    }

    /// The footer never crowds out the one hint that finds the others.
    #[test]
    fn footer_always_keeps_the_help_tail() {
        let ctx = Ctx {
            has_ticket: true,
            multi_column: true,
            ticket_has_sessions: true,
            can_undo: true,
            bulk_sleep: 3,
            bulk_archive: 3,
            any_attention: true,
            ..Default::default()
        };
        for width in [40usize, 60, 80, 100, 140, 200] {
            let line = footer(Scope::Board, &ctx, width);
            assert!(line.ends_with("? keys"), "width {width}: {line}");
            assert!(line.chars().count() <= width, "width {width} overflowed: {line}");
        }
    }

    /// The overlay is the complete answer: everything available, including the
    /// bindings the footer had no room for.
    #[test]
    fn overlay_covers_what_the_footer_drops() {
        let ctx = Ctx {
            has_ticket: true,
            multi_column: true,
            ticket_has_sessions: true,
            has_worktree: true,
            ..Default::default()
        };
        let rows: Vec<&str> =
            overlay(Scope::Board, &ctx).into_iter().flat_map(|(_, r)| r).map(|(k, _)| k).collect();
        for present in ["space", "g", "G", "?", "^L", "esc"] {
            assert!(rows.contains(&present), "{present} missing from the overlay: {rows:?}");
        }
    }
}
