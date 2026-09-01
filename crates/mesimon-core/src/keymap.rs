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
    /// Option/Alt held on a direction. `alt+h` and `alt+←` are the SAME atom,
    /// the way the two spellings of `ctrl+]` are — the keymap wants a
    /// direction, and the terminal may spell it either way.
    ///
    /// The second family off the legacy floor, and it is admitted on a
    /// different clause from [`Key::ShiftEnter`]. That atom is *ambiguous*: a
    /// terminal that cannot report it sends a plain `Enter`, which is another
    /// verb, so it had to be gated on `Ctx::rich_keys` to keep from acting
    /// wrongly. Alt is merely *absent* — a terminal that swallows Option
    /// (macOS Terminal composes `˙` for `⌥h`; iTerm2 needs `Option Key Sends:
    /// Esc+`) delivers no atom at all, and the key is inert rather than wrong.
    /// Inert is affordable exactly while no capability stands behind the
    /// atom: every move these four make, a legacy-floor spelling on the same
    /// screen also makes — `> <` on the board, `HJKL` in the tag picker,
    /// where the two spellings share one binding. They buy speed, never
    /// capability (`alt_is_admitted_only_for_a_nudge`).
    AltLeft,
    AltRight,
    AltUp,
    AltDown,
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
            Key::AltLeft => write!(f, "alt+←"),
            Key::AltRight => write!(f, "alt+→"),
            Key::AltUp => write!(f, "alt+↑"),
            Key::AltDown => write!(f, "alt+↓"),
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
    /// The theme picker, reached from a menu row: a list over the flavors
    /// whose cursor IS the preview (the board behind it repaints as the
    /// cursor moves), so Enter keeps and Esc puts the resting theme back.
    Theme,
    /// Scope barrier: owns every key, inherits nothing.
    Input,
}

impl Scope {
    /// Every scope, for the validators. Beside the enum so a new variant is
    /// added here in the same edit; `scope_list_is_complete` catches the one
    /// that is not.
    pub const ALL: [Scope; 14] = [
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
        Scope::Theme,
        Scope::Input,
    ];

    /// The scope a key falls through to when this one does not bind it.
    pub fn parent(self) -> Option<Scope> {
        match self {
            Scope::Board
            | Scope::Ticket
            | Scope::Diff
            | Scope::Move
            | Scope::Menu
            | Scope::Drawer
            | Scope::Archived
            | Scope::Theme => Some(Scope::Global),
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
            Scope::Theme => "THEME",
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
    /// A newer release is published: fetch it, verify it, and put it at our
    /// own path. The restart afterwards is still [`Verb::Reload`] — this
    /// verb only ever moves the file that verb's watch is already watching.
    InstallUpdate,
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
    /// `alt+hjkl` / `alt+<arrow>` — move the card one step that way, right
    /// now, with no ghost to aim and no Enter to commit. Same shape as
    /// [`Verb::Grab`]: the handler reads the direction off the key.
    Nudge,
    /// `.` — do the last action again on the card under the cursor. Move is
    /// the only action it repeats today; the verb is deliberately the general
    /// one, so the second repeatable action is a variant and a match arm.
    Repeat,
    Peek,
    /// Esc on the board — opens the menu below.
    Menu,
    ExternalDrawer,
    ArchivedList,
    /// Open the theme picker from the menu. No key of its own: a theme is
    /// picked once and lived with, the same argument that took `p` off the
    /// footer.
    ThemePick,
    // ---- sessions ----
    Claude,
    Shell,
    /// Shift+Enter on the board: open a one-line field on the selected card
    /// and put what is typed there in front of the ticket's live claude,
    /// submitted, without leaving the board. The composer's
    /// [`Verb::SaveStart`] is the same gesture one step earlier — there the
    /// ticket and the agent do not exist yet, so the press mints both and
    /// asks the title; here they do, so it only asks.
    Prompt,
    /// `S` on the ticket page: a second shell beside whatever is there.
    /// There is no Claude twin: a ticket holds ONE claude (2026-09-02), and a
    /// second seat is a shell — see STALE-MAP "One claude per ticket".
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
    /// Re-read the user's shell startup files, so the environment new panes
    /// get is the one their terminal would give them.
    ReloadShellEnv,
    /// `^t` — open the tag tail. Works on the board, the ticket screen, and
    /// inside the composer.
    TagPrefix,
    /// Move the picker cursor. The direction is read off the key, the way
    /// the board's `hjkl` motions are.
    TagLeft,
    TagRight,
    TagUp,
    TagDown,
    /// Take the tag under the cursor with you: along its axis, which is the
    /// order the row draws and the digit cycles, or onto the axis above or
    /// below. The picker's grid nudged the way the board's is.
    TagCarryLeft,
    TagCarryRight,
    TagCarryUp,
    TagCarryDown,
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
    /// A digit on the board or the ticket screen: step the selected ticket
    /// along that group's tags without opening the picker. The same meaning
    /// the digit already has inside `^t`, minus the chord.
    TagCycle,
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
    /// with the title as its first prompt, submitted. On a ticket that
    /// already has an agent the same key is [`Verb::Prompt`] instead, and
    /// while a prompt field is open this verb sends it — one key, one
    /// sentence: ask claude, stay on the board.
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
    /// `↑` in a prompt field: the previous thing this board asked an agent,
    /// oldest at the far end. The draft under the cursor is kept, not lost.
    HistoryPrev,
    /// `↓` walks the other way, and one step past the newest ask puts the
    /// kept draft back — the field returns to what was being written.
    HistoryNext,
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
#[derive(Debug, Clone, Default)]
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
    /// One of those claude sessions holds a PANE — so there is a box a
    /// prompt can land in. Narrower than `ticket_has_claude`, which counts a
    /// `Sleeping` session: parked is live, but it has no process to type at.
    pub ticket_promptable: bool,
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
    /// `.` has an action to do again AND doing it here would change
    /// something. Both halves matter: repeating a move onto a card that is
    /// already in that column is not a repeat, it is a shuffle.
    pub can_repeat: bool,
    /// What `.` would do again — "move again". Same shape as `undo_word`,
    /// and for the same reason: a hint is a `&'static str`, so the word comes
    /// from a fixed set and the column name lives in the status line.
    pub repeat_word: &'static str,
    /// `alt+<direction>` has somewhere to send the card under the cursor —
    /// another column, or another row in this one. A board of one column
    /// holding one card offers the gesture nothing, and the hint goes with
    /// the key.
    pub can_nudge: bool,
    pub bulk_sleep: usize,
    /// What sleeping those sessions would hand back. The suggestion does not
    /// gate on it — the menu row's detail spends it as the payoff word.
    pub bulk_sleep_bytes: u64,
    pub bulk_archive: usize,
    pub has_archived: bool,
    pub peek_on: bool,
    /// The live theme's id (`Flavor::name`), for the menu row's label.
    pub theme_name: &'static str,
    /// Its one-line blurb, for the row's detail.
    pub theme_blurb: &'static str,
    /// Which slot a pick would set: `"dark"` or `"light"`, the ground the
    /// terminal currently reports.
    pub theme_slot_word: &'static str,
    /// `MESIMON_THEME` is pinning the live theme; a pick still saves.
    pub theme_pinned: bool,
    /// The ticket page's preview zone holds more rows than it can show, so
    /// there is somewhere to page to. Measured by the last draw (the zone's
    /// height is a fact of the frame, not of the board), which is also what
    /// keeps `{ }` inert — and unhinted — under a reply that fits.
    pub preview_scrolls: bool,
    pub update_ready: bool,
    /// A newer release than this build is published, and taking the offer
    /// downloads it. Never true beside `update_ready`: a binary already
    /// waiting on disk is a restart, not a second download.
    pub release_available: bool,
    /// The tag it would fetch — `v0.1.0-alpha.5`. Both the chip and the row
    /// name it, because "an update is available" with no version is a claim
    /// you cannot look up, decline, or report a bug against.
    pub release_tag: String,
    /// A shell startup file has changed since the environment mesimon is
    /// handing to new panes was captured.
    pub shell_env_stale: bool,
    /// Reading the shell environment failed, so panes are getting the fallback.
    /// Offered on the same row, because "ask again" is the same act.
    pub shell_env_failed: bool,
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
    /// The field is a prompt bound for a live agent, not a ticket title. It
    /// saves nothing and creates nothing, so every word the input scope
    /// spends on saving is wrong here — `enter` sends.
    pub prompting: bool,
    /// Something has been asked from this board before, so `↑` in a prompt
    /// field has somewhere to go. Gates the key AND its hint: a field with
    /// no history offers no history.
    pub prompt_history: bool,
    // ---- tags ----
    /// A tag name is being typed. While true every binding in the tag tail
    /// stands down, so the digits are text and not axis picks.
    pub tag_naming: bool,
    /// The cursor is on a real tag, not the `+ new` cell — so there is
    /// something to wear, recolour, rename or delete.
    pub tag_on_entry: bool,
    /// The cursor's tag is already on this ticket, so Enter takes it off.
    pub tag_worn: bool,
    /// At least one group holds at least one tag. The quick-cycle digits
    /// share a single binding, so `avail` cannot speak for one group — an
    /// empty board of tags is the only state where every digit is inert, and
    /// `^t` is still how the first tag gets made.
    pub tags_exist: bool,
    /// `d` is armed: the next `d` deletes that tag board-wide.
    pub tag_forget_armed: bool,
    // ---- terminal ----
    /// The terminal answered the kitty-protocol probe, so `Shift+Enter` is
    /// distinguishable from `Enter`. False on the legacy floor, where every
    /// binding on `Key::ShiftEnter` must stay inert and unhinted.
    pub rich_keys: bool,
}

/// The four `_word` fields are the hint's text when the verb is live, and
/// empty by default; the hint supplies the plain word then.
fn or<'a>(word: &'a str, fallback: &'a str) -> &'a str {
    if word.is_empty() {
        fallback
    } else {
        word
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

/// The digit row, in the order the groups are numbered: `0` addresses group
/// 10, the row it points at rather than the number it spells. One list, shared
/// by the picker's axis pick and the board's quick cycle, because those two
/// are the same gesture with and without the chord — and because
/// `Key::Char('1')` is "whichever key types a 1 on this layout", which the
/// shifted spellings `!@#$…` are not.
const DIGITS: &[Key] = &[
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
];

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
        hint: |c| or(c.undo_word, "undo"),
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
        // Shift on the Enter axis again, and the same sentence the composer's
        // Shift+Enter says: ask claude, and do not go anywhere. There Enter
        // saves and Shift+Enter saves AND asks; here Enter goes to the agent
        // and Shift+Enter asks it from where you are standing — the harder
        // press is always the one that gets a prompt in front of an agent
        // without spending the terminal on it.
        //
        // It sits next to `enter` in the footer on purpose: the two are one
        // choice (go there / ask from here), and reading them apart is what
        // made the gesture invisible before.
        keys: &[Key::ShiftEnter],
        verb: Verb::Prompt,
        show: "shift+enter",
        hint: |_| "ask claude",
        // A parked agent has no box to type into, and `rich_keys` is the
        // ShiftEnter clause: where the terminal spells this as a plain Enter
        // the key must be inert AND unhinted, or the press would focus the
        // pane instead of opening a field.
        avail: |c| c.ticket_promptable && c.rich_keys,
        class: Class::Plain,
        group: Group::Sessions,
        mutates: true,
        prio: 12,
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
        // Overlay-only on the board (`prio: 0`). Starting a session is the
        // ticket page's subject — `enter`/`space` one row up lead there and
        // the footer teaches those — so the board's cells go to what only the
        // board can do. The key still works from here for anyone who knows it.
        keys: &[Key::Char('c')],
        verb: Verb::Claude,
        show: "c",
        hint: |c| {
            // Live but paneless is exactly Sleeping: the press wakes the
            // parked conversation and attaches, so the hint says so.
            if c.ticket_has_claude && !c.ticket_promptable {
                "wake claude"
            } else if c.ticket_has_claude {
                "claude"
            } else {
                "start claude"
            }
        },
        avail: |c| c.has_ticket,
        class: Class::Plain,
        group: Group::Sessions,
        mutates: true,
        prio: 0,
    },
    Binding {
        // Overlay-only for the same reason as `c` above.
        keys: &[Key::Char('s')],
        verb: Verb::Shell,
        show: "s",
        hint: |_| "shell",
        avail: |c| c.has_ticket,
        class: Class::Plain,
        group: Group::Sessions,
        mutates: true,
        prio: 0,
    },
    Binding {
        // Overlay-only, and the two swapped places (author direction): the
        // footer now names the accelerator and `?` names the floor. `> <` is
        // still bound, still the spelling every terminal can reach, and still
        // the aiming gesture — it is the teaching of it that moved.
        keys: &[Key::Char('>'), Key::Char('<')],
        verb: Verb::Grab,
        show: "> <",
        // Named for what it adds, now that it shares the overlay with a key
        // that makes the same move: this one lifts a ghost you aim and can
        // still cancel. Two rows both reading "move card" told a reader which
        // keys exist and nothing about which to press.
        hint: |_| "move card, aiming",
        // The user's rule: no selection, no move — and no second column to
        // move to means the same thing.
        avail: |c| c.has_ticket && c.multi_column,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 0,
    },
    Binding {
        // The same move `> <` makes, minus the aiming: one press takes the
        // card one column over or one row along, and the cursor rides with
        // it. This is the one the footer teaches now — spelled `option`,
        // because that is what the key says on the machine this ships to, and
        // a hint names the key the hand is looking for. "now" left with `> <`:
        // it was drawing a contrast with the aiming gesture that the footer no
        // longer sets up, and moving the card IS the verb.
        //
        // `can_nudge` is the wider predicate on purpose — `> <` needs a second
        // column, a nudge also reorders inside one — so the footer offers it
        // on the one-column board where `> <` had nothing to say.
        keys: &[Key::AltLeft, Key::AltRight, Key::AltUp, Key::AltDown],
        verb: Verb::Nudge,
        show: "option+hjkl",
        hint: |_| "move card",
        avail: |c| c.can_nudge,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 60,
    },
    Binding {
        // Vim's `.`, and the same bargain: the aiming was the expensive part
        // of the last gesture, so the repeat spends no keys on it.
        //
        // Overlay-only. `.` is the one binding whose availability was already
        // its own advertisement — `can_repeat` is false until you have moved
        // something, so the footer entry could only ever appear *after* the
        // gesture it accelerates, to a hand that had just performed it. The
        // hint word still changes with what would repeat; `?` carries it.
        keys: &[Key::Char('.')],
        verb: Verb::Repeat,
        show: ".",
        hint: |c| or(c.repeat_word, "again"),
        avail: |c| c.has_ticket && c.can_repeat,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 0,
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
        // Overlay-only, with `c` and `s` above: the whole session group is
        // the ticket page's subject, and a footer that offers to sleep them
        // but not to start them was teaching half a gesture.
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
        prio: 0,
    },
    Binding {
        // The bulk sleep the header is already offering, one press instead of
        // esc + enter. Board-wide actions have no key as a rule (they are not
        // about the selection, and a menu row has room to say what it will
        // do) — this is the second exception, on the same terms as `U`:
        // overlay-only (`prio: 0`), so the footer never carries it and the
        // chip that names it is the only place it is taught. `Z` is zzz, not
        // shift-of-`x` — the retired `X` was that, and shift may not switch
        // verbs. Gated on the menu row's own predicate, so the key works
        // exactly when the offer stands.
        keys: &[Key::Char('Z')],
        verb: Verb::SleepAllDone,
        show: "Z",
        hint: |_| "sleep the agents in done",
        avail: |c| c.bulk_sleep > 0,
        class: Class::Plain,
        group: Group::Sessions,
        mutates: true,
        prio: 0,
    },
    Binding {
        // Overlay-only, and so is the ticket screen's. The composer's copy is
        // the one the footer names (`prio: 35`, gated on `Ctx::composing`),
        // because tagging is something you do once while the ticket is being
        // made — the moment the words are already in your head. After that it
        // is maintenance, and maintenance can be looked up.
        keys: &[Key::Ctrl('t')],
        verb: Verb::TagPrefix,
        show: "^t",
        hint: |_| "tags",
        avail: |c| c.has_ticket,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 0,
    },
    Binding {
        // The picker's digit, reached without the picker: one press steps the
        // selected card along that group's tags and off the end back to
        // untagged. Overlay-only (`prio: 0`) on the Nudge precedent — it is an
        // accelerator for `^t`, which the footer teaches one entry up, and the
        // footer's cells are better spent on verbs with no second spelling.
        //
        // Bare digits, not shift+digits: a terminal has no shift+digit atom
        // (`keys::to_key` hands Shift+1 over as `!`), and the ten shifted
        // symbols are a different physical key on every layout, where
        // `Key::Char('1')` is whichever key types a 1.
        keys: DIGITS,
        verb: Verb::TagCycle,
        show: "1-0",
        hint: |_| "cycle tag",
        avail: |c| c.has_ticket && c.tags_exist,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 0,
    },
    Binding {
        // Overlay-only: archiving is rare next to the verbs it was crowding,
        // it is two presses anyway, and the ticket page still offers it in
        // words. `d delete` next door stays hinted — that one is destructive,
        // and a key the footer never mentions is a key nobody expects to be.
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
        prio: 0,
    },
    Binding {
        // Overlay-only, and the whole card-destroying set went with it: the
        // board's footer is for moving and opening, and destroying a ticket
        // is done from the page that shows you what you are destroying. Both
        // presses of the chord still land from here.
        keys: &[Key::Char('d')],
        verb: Verb::DeletePrefix,
        show: "d",
        hint: |_| "delete",
        avail: |c| c.has_ticket,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: false,
        prio: 0,
    },
    Binding {
        // Overlay-only. The peek is a view preference, not a verb: it is set
        // once and lived with, and the board it changes is the evidence it
        // worked. A permanent cell teaching a toggle nobody presses twice is
        // the cell the footer could least afford.
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
        prio: 0,
    },
    Binding {
        // Overlay-only. The menu is not about the selection and the footer
        // now is; the header's suggestion chip already spells `(esc)` on the
        // occasions the menu has something to offer, and `?` names it the
        // rest of the time.
        keys: &[Key::Esc],
        verb: Verb::Menu,
        show: "esc",
        hint: |_| "menu",
        avail: always,
        class: Class::Plain,
        group: Group::App,
        mutates: false,
        prio: 0,
    },
    Binding {
        // Overlay-only, last of the pass. `q` and `^c` are the two spellings
        // of leaving that every terminal program has taught for decades, and
        // `?` still names them — the footer is for what this program does
        // that another one would not.
        keys: &[Key::Char('q'), Key::Ctrl('c')],
        verb: Verb::Quit,
        show: "q",
        hint: |_| "quit",
        avail: always,
        class: Class::Plain,
        group: Group::App,
        mutates: false,
        prio: 0,
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
        // The diff's page keys, on the preview zone: a long reply is read
        // here, not picked, and `jk` is already the rail's. Same verb, same
        // spelling, and available only while there is a further page — a
        // reply that fits offers nothing to turn.
        keys: &[Key::Char('}'), Key::Char('{'), Key::PageDown, Key::PageUp],
        verb: Verb::PageDown,
        show: "{ }",
        hint: |_| "page preview",
        avail: |c| c.preview_scrolls,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 15,
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
            // Live but paneless is exactly Sleeping: the press wakes the
            // parked conversation and attaches, so the hint says so.
            if c.ticket_has_claude && !c.ticket_promptable {
                "wake claude"
            } else if c.ticket_has_claude {
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
        // Three words, one verb: a corpse cannot be slept and a sleeper
        // cannot be dismissed, so the same key means the only thing it
        // could mean for the row under the cursor. `Enter` next door
        // switches on `sel_dead` the same way ("resume").
        hint: |c| {
            if c.sel_dead {
                "dismiss"
            } else if c.sel_sleeping {
                "wake"
            } else {
                "sleep"
            }
        },
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
        hint: |c| if c.merge_actionable { or(c.merge_word, "merge") } else { "" },
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
        // Overlay-only; the composer is where the footer teaches this. See the
        // board's copy.
        keys: &[Key::Ctrl('t')],
        verb: Verb::TagPrefix,
        show: "^t",
        hint: |_| "tags",
        avail: always,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 0,
    },
    Binding {
        // The board's quick cycle, on the screen that shows the same ticket.
        // `has_ticket` is what the board gates on; here the ticket is the
        // screen, so only the registry can be empty.
        keys: DIGITS,
        verb: Verb::TagCycle,
        show: "1-0",
        hint: |_| "cycle tag",
        avail: |c| c.tags_exist,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 0,
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
        // Hinted here, and only here: this is the screen that shows the
        // ticket's sessions and its worktree, so it is the screen where the
        // word "delete" means something specific. The board's copy is
        // overlay-only.
        keys: &[Key::Char('d')],
        verb: Verb::DeletePrefix,
        show: "d",
        hint: |_| "delete",
        avail: always,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: false,
        prio: 90,
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
        hint: |c| or(c.density_word, "context lines"),
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
        // The board's nudge, in the picker's grid: one press takes the tag
        // under the cursor one cell along its axis or one axis over, and the
        // cursor rides with it. Along the row it is order — what the row
        // draws and what a repeated digit walks. Across rows it is the axis
        // itself, and before this a tag created on the wrong one had no
        // repair: `d` is the only other way off an axis and it strips the tag
        // from every ticket on the way out.
        //
        // `HJKL` and the four Alt directions ride ONE binding on purpose.
        // Alt is admitted only where no capability stands behind it
        // (`alt_is_admitted_only_for_a_nudge`), and sharing the key list is
        // the strongest form of that: the accelerator cannot reach a move the
        // legacy floor does not already make, because it is the same entry.
        // Shift stays on its axis too — `hjkl` steps, `HJKL` steps carrying.
        keys: &[
            Key::Char('H'),
            Key::Char('J'),
            Key::Char('K'),
            Key::Char('L'),
            Key::AltLeft,
            Key::AltDown,
            Key::AltUp,
            Key::AltRight,
        ],
        verb: Verb::TagCarryLeft,
        show: "HJKL",
        hint: |_| "move tag",
        avail: |c| !c.tag_naming && c.tag_on_entry,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: true,
        prio: 15,
    },
    Binding {
        keys: DIGITS,
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

/// The theme picker's three shapes are the menu's. `Act` does not mutate:
/// nothing the daemon owns changes, and the file it writes is this
/// machine's own preference.
static THEME: &[Binding] = &[
    Binding {
        keys: &[Key::Char('j'), Key::Down, Key::Char('k'), Key::Up],
        verb: Verb::CursorDown,
        show: "jk",
        hint: |_| "preview",
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
        hint: |_| "keep",
        avail: always,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 20,
    },
    Binding {
        keys: &[Key::Char('q'), Key::Esc],
        verb: Verb::Back,
        show: "esc",
        hint: |_| "put it back",
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
        verb: Verb::InstallUpdate,
        label: |c| format!("Install {}", c.release_tag),
        // Says where it stops. The download is not the restart: a board that
        // swapped its own binary out from under a running session without
        // saying so would be the trespass `U` exists to avoid.
        detail: |_| "downloads and verifies it ∙ nothing restarts yet".into(),
        // The tag is half the row, so a flag without one is not an offer.
        avail: |c| c.release_available && !c.release_tag.is_empty(),
        key: "",
    },
    MenuItem {
        verb: Verb::ReloadShellEnv,
        label: |c| {
            if c.shell_env_failed {
                "Try the shell environment again".into()
            } else {
                "Reload the shell environment".into()
            }
        },
        // The chip already said what changed; the row says what taking it
        // reaches. A pane's environment is fixed at exec, so "live panes keep
        // theirs" is not a caveat to bury — it is the difference between this
        // working and the user concluding it did nothing.
        detail: |c| {
            if c.shell_env_failed {
                "your shell did not answer ∙ panes are on a fallback".into()
            } else {
                "new sessions and wakes get it ∙ live panes keep theirs".into()
            }
        },
        avail: |c| c.shell_env_stale || c.shell_env_failed,
        key: "",
    },
    MenuItem {
        verb: Verb::SleepAllDone,
        label: |c| format!("Sleep {} in done", plural(c.bulk_sleep, "agent")),
        detail: |c| match gib(c.bulk_sleep_bytes) {
            Some(g) => format!("frees ~{g:.1}GiB ∙ they wake where they left off"),
            None => "frees their memory ∙ they wake where they left off".into(),
        },
        avail: |c| c.bulk_sleep > 0,
        key: "Z",
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
    // Beside the peek: the two view preferences sit together. Never a
    // suggestion — a theme is not something worth doing right now.
    MenuItem {
        verb: Verb::ThemePick,
        label: |c| format!("Theme: {}", c.theme_name),
        detail: |c| {
            if c.theme_pinned {
                "pinned by MESIMON_THEME ∙ a pick here still saves for the next launch".into()
            } else {
                format!("{} ∙ for a {} terminal", c.theme_blurb, c.theme_slot_word)
            }
        },
        avail: always,
        key: "",
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
        // Below `Reload` because they are two stages of one story and the
        // later stage wins: a binary already on disk is reloaded, never
        // downloaded again.
        verb: Verb::InstallUpdate,
        headline: |c| format!("{} available", c.release_tag),
        key: "",
    },
    Suggestion {
        verb: Verb::ReloadShellEnv,
        // A failure and a change lead to the same act but are not the same
        // news, and the chip is the only place the difference gets said.
        headline: |c| {
            if c.shell_env_failed {
                "shell env unreadable".into()
            } else {
                "shell env changed".into()
            }
        },
        key: "",
    },
    Suggestion {
        verb: Verb::SleepAllDone,
        headline: |c| format!("sleep {}", plural(c.bulk_sleep, "agent")),
        key: "Z",
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
        // A prompt field saves nothing: there is no ticket being named and
        // nothing lands on the board. The word has to be the one the press
        // actually does, or the footer is teaching the wrong screen.
        hint: |c| if c.prompting { "send" } else { "save" },
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
        //
        // A prompt field is the third state, and there the two Enters agree:
        // the field was OPENED with Shift+Enter, so the finger is already
        // holding shift and the second press must land somewhere. Sending is
        // the only thing it could sanely mean — the alternative was a dead
        // key in the middle of the gesture that opened the field.
        keys: &[Key::ShiftEnter],
        verb: Verb::SaveStart,
        show: "shift+enter",
        // Silent while prompting: `enter send` one cell to the left already
        // says it, and two footer cells reading "send" teach nothing twice.
        // Bound but unhinted is the shape `space` and `> <` already use.
        hint: |c| if c.prompting { "" } else { "save + ask claude" },
        avail: |c| (c.composing || c.prompting) && c.rich_keys,
        class: Class::Plain,
        group: Group::Sessions,
        mutates: true,
        prio: 15,
    },
    Binding {
        // Tags while the title is still being typed. A Ctrl-letter is the
        // only legacy-floor atom a text field cannot swallow, which is the
        // whole reason the tag key is `^t` and not `t`.
        //
        // This is the ONE place the footer names `^t` — the board's copy and
        // the ticket screen's are overlay-only — so it has to survive the
        // truncation, and at prio 35 it did not: 120 cells ran out inside
        // `shift+tab`'s long hint one entry earlier and the tag key was never
        // once on screen. Ahead of the workspace toggle now, which is the
        // trade this makes.
        keys: &[Key::Ctrl('t')],
        verb: Verb::TagPrefix,
        show: "^t",
        hint: |_| "tags",
        avail: |c| c.composing,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 25,
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
        // The prompt field's own history, the way a shell's is: `↑` recalls
        // the previous ask, `↓` walks forward again, and one step past the
        // newest restores the draft that was under the cursor when the walk
        // began. Prompt-only — a title being composed has no "previous". The
        // hint stands only once there is something to recall, because a
        // hinted key that does nothing is the one thing the footer may not
        // teach.
        keys: &[Key::Up],
        verb: Verb::HistoryPrev,
        show: "↑",
        hint: |_| "earlier asks",
        avail: |c| c.prompting && c.prompt_history,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 40,
    },
    Binding {
        // Silent: `↑ earlier asks` one cell over already implies its opposite,
        // the shape `> <` and `shift+enter` use.
        keys: &[Key::Down],
        verb: Verb::HistoryNext,
        show: "↓",
        hint: |_| "",
        avail: |c| c.prompting && c.prompt_history,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 0,
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
        Scope::Theme => THEME,
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
        // One binding, two spellings of every direction: the shifted letter
        // every terminal can send, and the Alt atom the ones that can send it
        // do (`alt_is_admitted_only_for_a_nudge`).
        (Verb::TagCarryLeft | Verb::TagCarryRight | Verb::TagCarryUp | Verb::TagCarryDown, k) => {
            match k {
                Key::Char('H') | Key::AltLeft => Verb::TagCarryLeft,
                Key::Char('L') | Key::AltRight => Verb::TagCarryRight,
                Key::Char('K') | Key::AltUp => Verb::TagCarryUp,
                _ => Verb::TagCarryDown,
            }
        }
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

    /// `Scope::ALL` is what every validator below walks, so a scope missing
    /// from it is validated by nothing. The match is exhaustive: adding a
    /// variant fails to compile here, and the length check then fails until
    /// `ALL` names it too.
    #[test]
    fn scope_list_is_complete() {
        fn index(s: Scope) -> usize {
            match s {
                Scope::Global => 0,
                Scope::Board => 1,
                Scope::Ticket => 2,
                Scope::Diff => 3,
                Scope::DiffView => 4,
                Scope::DeleteChord => 5,
                Scope::ArchiveChord => 6,
                Scope::TagChord => 7,
                Scope::Move => 8,
                Scope::Menu => 9,
                Scope::Drawer => 10,
                Scope::Archived => 11,
                Scope::Theme => 12,
                Scope::Input => 13,
            }
        }
        for (i, s) in Scope::ALL.iter().enumerate() {
            assert_eq!(index(*s), i, "{s:?} is out of place in Scope::ALL");
        }
        let highest = Scope::ALL.iter().map(|s| index(*s)).max().unwrap_or(0);
        assert_eq!(Scope::ALL.len(), highest + 1);
    }

    /// 04 §2.0 rule 3: no key bound in both a scope and any ancestor, and none
    /// bound twice inside one scope. The shipped keymap declares no overrides,
    /// so this is absolute.
    #[test]
    fn no_key_bound_twice_in_a_chain() {
        for scope in Scope::ALL {
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

    /// 04 §2.0 rule 1: every atom is expressible on the legacy floor — the
    /// two exceptions are `Key::ShiftEnter` and the four Alt directions, and
    /// each one is held down by a test of its own below
    /// (`shift_enter_is_inert_without_rich_keys`,
    /// `alt_is_admitted_only_for_a_nudge`).
    #[test]
    fn every_atom_is_on_the_legacy_floor() {
        for scope in Scope::ALL {
            for b in bindings(scope) {
                for k in b.keys {
                    let ok = match k {
                        Key::Char(c) => c.is_ascii_graphic() || *c == ' ',
                        // ctrl+<a-z> plus the two ctrl+] spellings (0x1D
                        // arrives as Ctrl+5 on terminals without kitty).
                        Key::Ctrl(c) => c.is_ascii_lowercase() || *c == ']' || *c == '5',
                        Key::ShiftEnter => false,
                        Key::AltLeft | Key::AltRight | Key::AltUp | Key::AltDown => false,
                        _ => true,
                    };
                    assert!(
                        ok || OFF_FLOOR.contains(k),
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
        for scope in Scope::ALL {
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

    /// Shift+Enter says ONE sentence — "ask claude, and stay here" — and its
    /// three homes are that sentence at three moments: before the ticket
    /// exists (mint it, spawn, ask the title), on a ticket whose agent is
    /// already running (open a field), and inside that field (send). The atom
    /// is off the legacy floor, so what it buys has to be a single idea; this
    /// is the test that notices when a fourth home makes it two.
    #[test]
    fn shift_enter_asks_claude_at_every_stage() {
        let composing = Ctx { composing: true, rich_keys: true, ..Default::default() };
        let onboard = Ctx {
            has_ticket: true,
            ticket_has_claude: true,
            ticket_promptable: true,
            rich_keys: true,
            ..Default::default()
        };
        let prompting = Ctx { prompting: true, rich_keys: true, ..Default::default() };
        assert_eq!(resolve(Scope::Input, Key::ShiftEnter, &composing), Some(Verb::SaveStart));
        assert_eq!(resolve(Scope::Board, Key::ShiftEnter, &onboard), Some(Verb::Prompt));
        assert_eq!(resolve(Scope::Input, Key::ShiftEnter, &prompting), Some(Verb::SaveStart));
        // The board's press is hinted where it works…
        assert_eq!(
            hint_for(Scope::Board, Verb::Prompt, &onboard),
            Some(("shift+enter", "ask claude"))
        );
        // …and the field it opens says `send`, not `save`: nothing about a
        // prompt is a save, and the word is the only thing telling them apart.
        assert_eq!(hint_for(Scope::Input, Verb::Save, &prompting), Some(("enter", "send")));
        assert_eq!(hint_for(Scope::Input, Verb::Save, &composing), Some(("enter", "save")));
        // The second press is bound (the finger is still holding shift) and
        // deliberately unhinted — `enter send` beside it already teaches it.
        assert_eq!(hint_for(Scope::Input, Verb::SaveStart, &prompting), None);
        // A prompt field is not a composer: no workspace to cycle, no tags to
        // pick, because there is no ticket being made.
        assert_eq!(resolve(Scope::Input, Key::BackTab, &prompting), None);
        assert_eq!(resolve(Scope::Input, Key::Ctrl('t'), &prompting), None);
    }

    /// `↑`/`↓` in a prompt field walk what was asked before — and ONLY there,
    /// and only once there is something to walk. A composer has no earlier
    /// titles, and a field with an empty history must not hint a key that
    /// would do nothing.
    #[test]
    fn prompt_history_is_the_prompt_fields_and_needs_a_past() {
        let fresh = Ctx { prompting: true, ..Default::default() };
        let seasoned = Ctx { prompting: true, prompt_history: true, ..Default::default() };
        let composing = Ctx { composing: true, prompt_history: true, ..Default::default() };
        assert_eq!(resolve(Scope::Input, Key::Up, &fresh), None);
        assert_eq!(resolve(Scope::Input, Key::Down, &fresh), None);
        assert_eq!(hint_for(Scope::Input, Verb::HistoryPrev, &fresh), None);
        assert_eq!(resolve(Scope::Input, Key::Up, &seasoned), Some(Verb::HistoryPrev));
        assert_eq!(resolve(Scope::Input, Key::Down, &seasoned), Some(Verb::HistoryNext));
        assert_eq!(
            hint_for(Scope::Input, Verb::HistoryPrev, &seasoned),
            Some(("↑", "earlier asks"))
        );
        // `↓` is bound and silent: `↑`'s hint implies it.
        assert_eq!(hint_for(Scope::Input, Verb::HistoryNext, &seasoned), None);
        assert_eq!(resolve(Scope::Input, Key::Up, &composing), None);
    }

    /// A prompt needs a box to land in. `ticket_has_claude` counts a parked
    /// session — `Sleeping` is live — so gating on it would offer the key on
    /// a ticket with no process, and the press would reach a pane that is not
    /// there.
    #[test]
    fn prompting_needs_a_pane_not_merely_a_session() {
        let rich = |promptable| Ctx {
            has_ticket: true,
            ticket_has_claude: true,
            ticket_promptable: promptable,
            rich_keys: true,
            ..Default::default()
        };
        assert_eq!(resolve(Scope::Board, Key::ShiftEnter, &rich(true)), Some(Verb::Prompt));
        assert_eq!(resolve(Scope::Board, Key::ShiftEnter, &rich(false)), None);
        assert!(hint_for(Scope::Board, Verb::Prompt, &rich(false)).is_none());
        // And plain Enter is untouched either way: the two live side by side
        // in the footer, and only one of them spends the terminal.
        assert_eq!(resolve(Scope::Board, Key::Enter, &rich(true)), Some(Verb::Act));
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

    /// Every atom the legacy floor cannot spell. The list is short on
    /// purpose: each entry is answered by a named test, and a fifth entry is
    /// a decision, not an addition.
    const OFF_FLOOR: &[Key] =
        &[Key::ShiftEnter, Key::AltLeft, Key::AltRight, Key::AltUp, Key::AltDown];

    /// Alt's escape clause, and the reason it is not `rich_keys`. Shift+Enter
    /// had to be gated because a terminal that cannot report it does report
    /// something — a plain `Enter`, which saves without asking. Alt fails the
    /// other way: the terminal eats the modifier and NOTHING arrives, so the
    /// only cost is a hint for a key that does not press.
    ///
    /// What makes that affordable is that no CAPABILITY stands behind the
    /// atom, and THAT is the clause, not the count. So every Alt binding must
    /// be a nudge — it moves the thing under the cursor one step — and every
    /// one must hold a legacy-floor spelling of the same move bound beside it
    /// on the same screen: `> <` for the board's, and `HJKL` inside the
    /// picker's own key list, where the accelerator cannot drift from the
    /// floor because it IS the same entry. A third one needs the same two
    /// sentences, argued.
    ///
    /// The board's two swapped billing (2026-09-01, author direction): the
    /// FOOTER teaches `option+hjkl` and `> <` fell to the overlay. The floor
    /// spelling is still bound and `?` still names it, so the clause holds in
    /// its bound-beside-it form — but the "hinted" half is spent, and a
    /// terminal that eats the modifier now reads a footer whose move key does
    /// nothing and finds the working one only in `?`. That is the cost of
    /// this arrangement, recorded here because this is the test that would
    /// otherwise have quietly stopped guarding it.
    #[test]
    fn alt_is_admitted_only_for_a_nudge() {
        const ALT: &[Key] = &[Key::AltLeft, Key::AltRight, Key::AltUp, Key::AltDown];
        let mut found: Vec<(Scope, Verb)> = Vec::new();
        for scope in Scope::ALL {
            for b in bindings(scope) {
                if b.keys.iter().any(|k| ALT.contains(k)) {
                    found.push((scope, b.verb));
                    // All four or none: a direction left out is a key that
                    // resolves to nothing while its three neighbours work.
                    for k in ALT {
                        assert!(b.keys.contains(k), "{:?} in {scope:?} is missing {k:?}", b.verb);
                    }
                }
            }
        }
        assert_eq!(
            found,
            vec![(Scope::Board, Verb::Nudge), (Scope::TagChord, Verb::TagCarryLeft)],
            "an alt binding that is not one of the two nudges"
        );

        // The board's. The capability it accelerates is on the floor, bound
        // and spelled; the footer teaches the accelerator and the overlay
        // keeps the floor, which is the whole of what is left of the clause.
        let ctx =
            Ctx { has_ticket: true, multi_column: true, can_nudge: true, ..Default::default() };
        assert_eq!(resolve(Scope::Board, Key::Char('>'), &ctx), Some(Verb::Grab));
        assert_eq!(hint_for(Scope::Board, Verb::Grab, &ctx), Some(("> <", "move card, aiming")));
        assert_eq!(resolve(Scope::Board, Key::AltLeft, &ctx), Some(Verb::Nudge));
        assert_eq!(hint_for(Scope::Board, Verb::Nudge, &ctx), Some(("option+hjkl", "move card")));
        let shown: Vec<&str> = footer_items(Scope::Board, &ctx).iter().map(|b| b.show).collect();
        assert!(shown.contains(&"option+hjkl"), "the footer must name the move: {shown:?}");
        assert!(!shown.contains(&"> <"), "the floor spelling is overlay-only now: {shown:?}");
        assert!(
            overlay(Scope::Board, &ctx).iter().any(|(_, ks)| ks.iter().any(|(k, _)| *k == "> <")),
            "the floor spelling must survive in the complete answer"
        );
        // And the nudge is the board's alone: no fall-through from Global.
        assert_eq!(resolve(Scope::Ticket, Key::AltLeft, &ctx), None);
        // Nothing to send anywhere: inert, and unhinted with it.
        let alone = Ctx { can_nudge: false, ..ctx };
        assert_eq!(resolve(Scope::Board, Key::AltLeft, &alone), None);
        assert_eq!(hint_for(Scope::Board, Verb::Nudge, &alone), None);

        // The picker's. Both spellings reach the same verb, and the shifted
        // letter is the one the footer names.
        let tag = Ctx { has_ticket: true, tag_on_entry: true, ..Default::default() };
        for (k, v) in [
            (Key::Char('H'), Verb::TagCarryLeft),
            (Key::Char('L'), Verb::TagCarryRight),
            (Key::Char('K'), Verb::TagCarryUp),
            (Key::Char('J'), Verb::TagCarryDown),
            (Key::AltLeft, Verb::TagCarryLeft),
            (Key::AltRight, Verb::TagCarryRight),
            (Key::AltUp, Verb::TagCarryUp),
            (Key::AltDown, Verb::TagCarryDown),
        ] {
            assert_eq!(resolve(Scope::TagChord, k, &tag), Some(v), "{k:?}");
        }
        assert_eq!(hint_for(Scope::TagChord, Verb::TagCarryLeft, &tag), Some(("HJKL", "move tag")));
        // On the footer, in the cell `w 2nd tag beside` used to hold: the
        // picker's row runs the full width of a 120-column terminal, so this
        // one is here because the second-tag experiment ended, not for free.
        assert!(
            footer_items(Scope::TagChord, &tag).iter().any(|b| b.verb == Verb::TagCarryLeft),
            "the carry is the only tag mover and nothing on the footer says so"
        );
        let rows: Vec<&str> = overlay(Scope::TagChord, &tag)
            .into_iter()
            .flat_map(|(_, r)| r)
            .map(|(k, _)| k)
            .collect();
        assert!(rows.contains(&"HJKL"), "the picker's nudge is not in `?`: {rows:?}");
        // On the `+ new` cell there is no tag to carry; in a name field the
        // letters are text. Both spellings go down together, hint and all.
        let plus = Ctx { has_ticket: true, ..Default::default() };
        let naming = Ctx { tag_naming: true, ..tag };
        for c in [&plus, &naming] {
            assert_eq!(resolve(Scope::TagChord, Key::Char('H'), c), None);
            assert_eq!(resolve(Scope::TagChord, Key::AltLeft, c), None);
            assert_eq!(hint_for(Scope::TagChord, Verb::TagCarryLeft, c), None);
        }
    }

    /// 04 §2.0 rule 2, the part that bit us: no `ctrl+<digit>` other than the
    /// `Ctrl+5` that IS `ctrl+]` on legacy terminals. The rule used to ban
    /// Alt/Meta outright too — `Key::Alt` did not exist to be constructed —
    /// and `alt_is_admitted_only_for_a_nudge` is what replaced that ban:
    /// four atoms, two nudges, no capability behind either.
    #[test]
    fn no_banned_atoms() {
        for scope in Scope::ALL {
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

    /// The picker's digit, reached without the picker — and the reason it is
    /// a bare digit rather than the shift+digit that was asked for. A
    /// terminal has no shift+digit atom: `keys::to_key` hands Shift+1 over as
    /// `!`, so the binding would really be the ten symbols `!@#$%^&*()`,
    /// which are a different physical key on every layout (Hebrew swaps
    /// `(`/`)`, AZERTY types digits WITH shift) — and `!` is the diff
    /// screen's worktree shell already. `Key::Char('1')` is "whichever key types a 1 here",
    /// which is right everywhere by construction.
    #[test]
    fn digits_cycle_tags_without_the_picker() {
        let ctx = Ctx { has_ticket: true, tags_exist: true, ..Default::default() };
        for k in DIGITS {
            assert_eq!(resolve(Scope::Board, *k, &ctx), Some(Verb::TagCycle), "board {k:?}");
            assert_eq!(resolve(Scope::Ticket, *k, &ctx), Some(Verb::TagCycle), "ticket {k:?}");
            // The gesture it accelerates keeps the same key inside the chord.
            assert_eq!(resolve(Scope::TagChord, *k, &ctx), Some(Verb::TagGroup), "picker {k:?}");
        }
        // An empty registry is the one state where every digit is inert, and
        // it is unhinted with it — `^t` is still how the first tag gets made.
        let no_tags = Ctx { has_ticket: true, ..Default::default() };
        assert_eq!(resolve(Scope::Board, Key::Char('1'), &no_tags), None);
        assert_eq!(hint_for(Scope::Board, Verb::TagCycle, &no_tags), None);
        // No selection, nothing to tag.
        let no_card = Ctx { tags_exist: true, ..Default::default() };
        assert_eq!(resolve(Scope::Board, Key::Char('1'), &no_card), None);
        // The diff screen chains to Global, not to Ticket: digits stay unbound
        // there, and a text field owns its own.
        assert_eq!(resolve(Scope::Diff, Key::Char('1'), &ctx), None);
        let composing = Ctx { composing: true, tags_exist: true, ..Default::default() };
        assert_eq!(resolve(Scope::Input, Key::Char('1'), &composing), None);
        // The shifted spellings reach none of it, and `!` still opens a shell.
        for sym in "!@#$%^&*()".chars() {
            assert_ne!(resolve(Scope::Board, Key::Char(sym), &ctx), Some(Verb::TagCycle), "{sym}");
            assert_ne!(resolve(Scope::Ticket, Key::Char(sym), &ctx), Some(Verb::TagCycle), "{sym}");
        }
        // `!` is the diff screen's shell-in-worktree, and stays it: the atom
        // Shift+1 really produces was spoken for before this feature existed.
        let wt = Ctx { worktree_present: true, ..ctx.clone() };
        assert_eq!(resolve(Scope::Diff, Key::Char('!'), &wt), Some(Verb::WorktreeShell));
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
        // Every picker key is inert: the digits and letters are text now,
        // and a shifted letter is a letter.
        for k in [
            Key::Char('1'),
            Key::Char('h'),
            Key::Char('j'),
            Key::Char('H'),
            Key::Char('J'),
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
        for scope in Scope::ALL {
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
        // The menu and the quit left the FOOTER, not the board: both still
        // resolve and the overlay still names both, so the invariants these
        // lines used to guard are asserted where they now live. `?` is what
        // finds them, and `footer_always_keeps_the_help_tail` guards that.
        let named = |k: &str| {
            overlay(Scope::Board, &ctx).iter().any(|(_, ks)| ks.iter().any(|(s, _)| *s == k))
        };
        for gone in ["esc", "q"] {
            assert!(!shown.contains(&gone), "{gone} is overlay-only now: {shown:?}");
            assert!(named(gone), "the overlay is the complete answer and must name {gone}");
        }
        assert_eq!(resolve(Scope::Board, Key::Esc, &ctx), Some(Verb::Menu));
        assert_eq!(resolve(Scope::Board, Key::Char('q'), &ctx), Some(Verb::Quit));
        assert_eq!(resolve(Scope::Board, Key::Ctrl('c'), &ctx), Some(Verb::Quit));
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

    /// `.` is bound to nothing until there is something to repeat, and its
    /// hint appears on exactly the same condition — the footer cannot offer a
    /// repeat of an action that was never taken.
    #[test]
    fn repeat_is_unbound_until_there_is_something_to_repeat() {
        let selected = Ctx { has_ticket: true, multi_column: true, ..Default::default() };
        assert_eq!(resolve(Scope::Board, Key::Char('.'), &selected), None);
        assert_eq!(hint_for(Scope::Board, Verb::Repeat, &selected), None);
        let armed = Ctx { can_repeat: true, repeat_word: "move again", ..selected };
        assert_eq!(resolve(Scope::Board, Key::Char('.'), &armed), Some(Verb::Repeat));
        assert_eq!(hint_for(Scope::Board, Verb::Repeat, &armed), Some((".", "move again")));
        // A repeat needs a card under the cursor like every other card verb.
        let no_card = Ctx { has_ticket: false, ..armed.clone() };
        assert_eq!(resolve(Scope::Board, Key::Char('.'), &no_card), None);
        // And it is the board's key alone: the ticket screen has no move to
        // repeat, so `.` must not reach it through the global scope.
        assert_eq!(resolve(Scope::Ticket, Key::Char('.'), &armed), None);
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

    /// `x` is one verb wearing three words, and the word has to be the one
    /// the row under the cursor can actually do. The dismiss case is the one
    /// that was missing: `x` on a corpse resolved to `Verb::Sleep`, hinted
    /// "sleep", and came back "only idle sessions sleep".
    #[test]
    fn x_says_what_it_will_do_to_the_row_under_it() {
        let live = Ctx { sel_session: true, ..Default::default() };
        let asleep = Ctx { sel_session: true, sel_sleeping: true, ..Default::default() };
        let corpse = Ctx { sel_session: true, sel_dead: true, ..Default::default() };
        for c in [&live, &asleep, &corpse] {
            assert_eq!(resolve(Scope::Ticket, Key::Char('x'), c), Some(Verb::Sleep));
        }
        assert_eq!(hint_for(Scope::Ticket, Verb::Sleep, &live), Some(("x", "sleep")));
        assert_eq!(hint_for(Scope::Ticket, Verb::Sleep, &asleep), Some(("x", "wake")));
        assert_eq!(hint_for(Scope::Ticket, Verb::Sleep, &corpse), Some(("x", "dismiss")));
        // `Enter` next door reads the same flag, so the two keys never
        // disagree about what the selected row is.
        assert_eq!(hint_for(Scope::Ticket, Verb::Act, &corpse), Some(("enter", "resume")));
        // Nothing is offered when nothing is selected.
        assert_eq!(hint_for(Scope::Ticket, Verb::Sleep, &Ctx::default()), None);
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
        for scope in Scope::ALL {
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
        for s in
            [Scope::Board, Scope::Ticket, Scope::Diff, Scope::Drawer, Scope::Menu, Scope::Theme]
        {
            assert_eq!(resolve(s, Key::Char('u'), &ctx), Some(Verb::Undo), "{s:?}");
        }
        let nothing = Ctx::default();
        assert_eq!(resolve(Scope::Board, Key::Char('u'), &nothing), None);
        // And it says which thing it would undo.
        let arch = Ctx { can_undo: true, undo_word: "undo archive", ..Default::default() };
        assert_eq!(hint_for(Scope::Board, Verb::Undo, &arch), Some(("u", "undo archive")));
    }

    /// Shift hardens or forces the same verb on the same target. It never
    /// switches verbs, and it is never how a board-wide action is reached —
    /// `Z` is not shift-of-`x` but its own atom (zzz), which is why the
    /// retired `X` stays retired.
    #[test]
    fn shift_stays_on_one_axis() {
        let t = Ctx { sel_session: true, ..Default::default() };
        assert_eq!(resolve(Scope::Ticket, Key::Char('c'), &t), Some(Verb::Claude));
        // `C` is gone: a ticket holds one claude, and the second seat is a
        // shell (STALE-MAP "One claude per ticket"). Shift on `c` is inert.
        assert_eq!(resolve(Scope::Ticket, Key::Char('C'), &t), None);
        assert_eq!(resolve(Scope::Ticket, Key::Char('s'), &t), Some(Verb::Shell));
        assert_eq!(resolve(Scope::Ticket, Key::Char('S'), &t), Some(Verb::ShellNew));
        // The picker's pair is the same bargain on a motion: `hjkl` steps,
        // `HJKL` steps carrying the tag. Same axis, same cell under the
        // cursor, harder.
        let tag = Ctx { has_ticket: true, tag_on_entry: true, ..Default::default() };
        assert_eq!(resolve(Scope::TagChord, Key::Char('l'), &tag), Some(Verb::TagRight));
        assert_eq!(resolve(Scope::TagChord, Key::Char('L'), &tag), Some(Verb::TagCarryRight));
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
        // The bulk sleep is the exception, and it is one the header teaches:
        // its own key, gated on the same predicate as the row and the chip,
        // and kept off the footer so the board still reads as the selection's.
        assert_eq!(resolve(Scope::Board, Key::Char('Z'), &full), Some(Verb::SleepAllDone));
        assert_eq!(resolve(Scope::Board, Key::Char('Z'), &Ctx::default()), None);
        assert_eq!(resolve(Scope::Ticket, Key::Char('Z'), &full), None, "board-only");
        assert!(
            !footer_items(Scope::Board, &full).iter().any(|b| b.show == "Z"),
            "the header chip carries this offer; the footer stays out of it"
        );
        assert_eq!(
            hint_for(Scope::Board, Verb::SleepAllDone, &full),
            Some(("Z", "sleep the agents in done"))
        );
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
            Verb::ThemePick,
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
        assert!(!quiet_verbs.contains(&Verb::InstallUpdate));
        assert!(!quiet_verbs.contains(&Verb::ReloadShellEnv));
        // And a flag with no tag behind it is not an offer either: the row
        // names the version, so half of it missing means there is no row.
        let stem: Vec<Verb> = menu_items(&Ctx { release_available: true, ..Default::default() })
            .iter()
            .map(|m| m.verb)
            .collect();
        assert!(!stem.contains(&Verb::InstallUpdate), "an offer with no version: {stem:?}");
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
            release_available: true,
            release_tag: "v0.1.0-alpha.5".into(),
            shell_env_stale: true,
            ..Default::default()
        };
        // Declared priority, in the header and at the top of the menu alike.
        let heads: Vec<String> = suggestions(&all).iter().map(|s| (s.headline)(&all)).collect();
        assert_eq!(
            heads,
            [
                "update ready",
                "v0.1.0-alpha.5 available",
                "shell env changed",
                "sleep 3 agents",
                "archive 2 tickets"
            ]
        );
        let one = Ctx { bulk_sleep: 1, bulk_archive: 1, ..Default::default() };
        let heads: Vec<String> = suggestions(&one).iter().map(|s| (s.headline)(&one)).collect();
        assert_eq!(heads, ["sleep 1 agent", "archive 1 ticket"], "counts of one read as one");
        let rows: Vec<Verb> = menu_items(&all).iter().map(|m| m.verb).collect();
        assert_eq!(
            &rows[..5],
            &[
                Verb::Reload,
                Verb::InstallUpdate,
                Verb::ReloadShellEnv,
                Verb::SleepAllDone,
                Verb::ArchiveAllDone
            ],
            "suggested rows must lead the menu, in suggestion order: {rows:?}"
        );
        // And each suggested row is offered exactly when its suggestion is.
        for ctx in [
            Ctx::default(),
            all.clone(),
            Ctx { update_ready: true, ..Default::default() },
            Ctx {
                release_available: true,
                release_tag: "v0.1.0-alpha.5".into(),
                ..Default::default()
            },
            Ctx { release_available: true, ..Default::default() },
            Ctx { shell_env_stale: true, ..Default::default() },
            Ctx { shell_env_failed: true, ..Default::default() },
        ] {
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
            release_available: true,
            release_tag: "v0.1.0-alpha.5".into(),
            shell_env_stale: true,
            theme_name: "graphite",
            theme_blurb: "dark, the default",
            theme_slot_word: "dark",
            ..Default::default()
        };
        for m in MENU_ITEMS {
            let label = (m.label)(&ctx);
            assert!(!label.is_empty(), "{:?} has no label", m.verb);
            // A label built from a Ctx value can end up a stem — "Install "
            // with nothing after it. The row would render, and would teach
            // the wrong thing, so the trailing space is the tell.
            assert_eq!(label.trim_end(), label, "{:?} has a label with nothing after it", m.verb);
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
        for scope in Scope::ALL {
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
        for s in
            [Scope::Ticket, Scope::Diff, Scope::Drawer, Scope::Archived, Scope::Menu, Scope::Theme]
        {
            assert_eq!(resolve(s, Key::Char('q'), &ctx), Some(Verb::Back), "{s:?}");
            assert_eq!(resolve(s, Key::Esc, &ctx), Some(Verb::Back), "{s:?}");
        }
        assert_eq!(resolve(Scope::Move, Key::Char('q'), &ctx), Some(Verb::Cancel));
        for s in Scope::ALL {
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
