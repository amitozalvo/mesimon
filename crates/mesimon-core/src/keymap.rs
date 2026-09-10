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

use crate::board::AgentProvider;

/// Compact provider names used in session actions and status lines.
pub fn agent_word(provider: AgentProvider) -> &'static str {
    match provider {
        AgentProvider::ClaudeCode => "claude",
        AgentProvider::Codex => "codex",
    }
}

fn agent_hint(c: &Ctx, claude: &'static str, codex: &'static str) -> &'static str {
    let provider =
        if c.composing || c.editor_composing { c.agent_provider } else { c.ticket_agent_provider };
    match provider {
        AgentProvider::ClaudeCode => claude,
        AgentProvider::Codex => codex,
    }
}

/// One key atom on 04 §2.0's legacy floor. Deliberately NOT crossterm's
/// `KeyCode`: core stays free of the input stack, and the TUI does the one
/// conversion at the edge (`tui/src/keys.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// Printable ASCII. An uppercase letter IS the Shift+letter atom.
    Char(char),
    /// `ctrl+<a-z>` and `ctrl+]`. An UPPERCASE letter is the ctrl+shift+letter
    /// atom, and it is off the legacy floor on [`Key::ShiftEnter`]'s clause:
    /// a terminal without the kitty tier sends the bare control byte, which
    /// is the lowercase atom and another verb, so every binding on one is
    /// gated on `Ctx::rich_keys` (`ambiguous_atoms_are_inert_without_rich_keys`).
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
            // The case IS the shift: `^s` and `^S` are two atoms.
            Key::Ctrl(c) => write!(f, "^{c}"),
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
    /// After `z` on the board or the ticket screen (T-74): a chord tail you
    /// STAY in while `z` walks the preset ring — `1h`, `4h`, `tomorrow
    /// 9:00`, `next Monday 9:00` — until Enter snoozes or Esc (or any stray
    /// key) cancels. A snooze is an archive with a deadline, so like the
    /// archive chord it costs a deliberate second press.
    SnoozeChord,
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
    /// The settings submenu, reached from the menu's `Settings` row: the
    /// preferences (theme, replies, how a snooze returns) in a list of
    /// their own, so the menu proper stays the list of things to DO. Esc
    /// pops back to the menu, on the row that opened it.
    Settings,
    /// The notifications list, one level under Settings (T-282). Five rows
    /// on the same surface with the same three shapes: whether the board
    /// says anything out loud, which of the two moments, and the two sounds.
    /// Its own door because `draw_list` sizes a dialog at two lines a row
    /// and does not scroll — Settings is already at the edge of a `MIN_H`
    /// terminal, and five more rows there would be five rows nobody can
    /// reach.
    Notifications,
    /// The agent-brief offer's confirm dialog (T-217, re-aimed at the system
    /// prompt by T-224): the text every claude mesimon starts would carry,
    /// shown verbatim, over the board. The one modal confirmation in mesimon
    /// — every other one is a chord tail or the `m` key's arm, and neither
    /// can show five lines of text. Enter and Esc are the list dialogs' own
    /// `Act`/`Back`; `c` and `i` are its two extra answers.
    Brief,
    /// The release notes, reached from a menu row: `CHANGELOG.md` compiled
    /// into the binary (`relnotes.rs`), one painted band per release, read
    /// top to bottom. A screen, not a dialog — it is the one document in
    /// mesimon that is only ever read, so it gets the whole terminal.
    Releases,
    /// The links dialog (T-256): what the ticket's notes point at — URLs,
    /// ticket keys, files — one row each, over the board or the ticket page.
    /// `^k` opens it; Enter opens the row. A list dialog like the archived
    /// one, and like it a list the board is not.
    Links,
    /// The column settings dialog (T-117): one column's every setting in a
    /// list of its own, reached from the column's header or the menu. The
    /// Settings list's shapes, plus `h`/`l` on its sort row. Its Name row
    /// is a text field while it is being typed in, and then the scope is
    /// `Input`.
    ColumnSettings,
    /// The board's own top row (T-305): `k` off a column header lands here,
    /// where Enter reads the section under the cursor and `j` goes back to
    /// the column. One section is focusable today — the checkout's git
    /// clause, whose Enter is the diff the board's `v` opens — so nothing
    /// walks sideways and `h`/`l` are unbound. The column header is
    /// `Ctx::col_header`, a different place a cursor can be.
    Header,
    /// Scope barrier: owns every key, inherits nothing.
    Input,
    /// The full-screen note editor (a title line over a multi-line markdown
    /// body): a second text barrier. Reached by `Tab` from the composer,
    /// where it keeps the composer's keys, and by `n`/`N` on a ticket.
    Editor,
}

impl Scope {
    /// Every scope, for the validators. Beside the enum so a new variant is
    /// added here in the same edit; `scope_list_is_complete` catches the one
    /// that is not.
    pub const ALL: [Scope; 23] = [
        Scope::Global,
        Scope::Board,
        Scope::Ticket,
        Scope::Diff,
        Scope::DiffView,
        Scope::DeleteChord,
        Scope::ArchiveChord,
        Scope::SnoozeChord,
        Scope::TagChord,
        Scope::Move,
        Scope::Menu,
        Scope::Drawer,
        Scope::Archived,
        Scope::Theme,
        Scope::Settings,
        Scope::Notifications,
        Scope::Brief,
        Scope::Releases,
        Scope::Links,
        Scope::ColumnSettings,
        Scope::Header,
        Scope::Input,
        Scope::Editor,
    ];

    /// The scope a key falls through to when this one does not bind it.
    pub fn parent(self) -> Option<Scope> {
        match self {
            Scope::Board
            | Scope::Ticket
            | Scope::Diff
            | Scope::Menu
            | Scope::Drawer
            | Scope::Archived
            | Scope::Theme
            | Scope::Settings
            | Scope::Notifications
            | Scope::Brief
            | Scope::Releases
            | Scope::Links
            | Scope::ColumnSettings
            | Scope::Header => Some(Scope::Global),
            Scope::Global
            | Scope::Move
            | Scope::DiffView
            | Scope::DeleteChord
            | Scope::ArchiveChord
            | Scope::SnoozeChord
            | Scope::TagChord
            | Scope::Input
            | Scope::Editor => None,
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
            Scope::SnoozeChord => "SNOOZE",
            Scope::TagChord => "TAG",
            Scope::Move => "MOVE",
            Scope::Menu => "MENU",
            Scope::Drawer => "EXTERNAL",
            Scope::Archived => "ARCHIVED",
            Scope::Theme => "THEME",
            Scope::Settings => "SETTINGS",
            Scope::Notifications => "NOTIFICATIONS",
            Scope::Brief => "AGENT BRIEF",
            Scope::Releases => "RELEASES",
            Scope::Links => "LINKS",
            Scope::ColumnSettings => "COLUMN",
            Scope::Header => "HEADER",
            Scope::Input => "INPUT",
            Scope::Editor => "EDIT",
        }
    }
}

/// Every action the keymap can name. The TUI matches this exhaustively, which
/// is what proves the table and the handler agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    SettingsAppearance,
    SettingsBehaviour,
    SettingsAgents,
    ColumnAgentBehaviour,
    // ---- global ----
    Help,
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
    /// drawer = adopt + resume, archived = open, header = read the section
    /// under the cursor (T-305: the git clause's diff).
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
    /// `P` on the board: `p` widened from the cursor card to every card
    /// (T-237). Shift never switches verbs — the same reply row, on every
    /// card at once — and a second `P` narrows back to the cursor card.
    PeekAll,
    /// Esc on the board — opens the menu below.
    Menu,
    ExternalDrawer,
    ArchivedList,
    /// `^k` on the board or the ticket page: the LINKS dialog over the
    /// ticket's notes (T-256). Nothing to list is a status line, never an
    /// empty dialog — the archived list's rule.
    Links,
    /// `^K`: the first of those links, opened with no dialog. Shift hardens
    /// the verb on one axis and never changes it — `^s`/`^S`'s shape — and
    /// on the legacy floor the press arrives as `^k`, the safe half.
    LinkFirst,
    /// `c` in the links dialog: the row's target to the clipboard (OSC 52,
    /// the brief dialog's road — write-only, so the dialog stays up and the
    /// status never claims success).
    LinkCopy,
    /// Open the settings submenu from the menu: the preferences, one level
    /// down, so a menu row is either an action or the door to the settings
    /// and never a toggle between actions.
    Settings,
    /// Open the theme picker from the settings submenu. No key of its own: a theme is
    /// picked once and lived with, the same argument that took `p` off the
    /// footer.
    ThemePick,
    // ---- columns (T-117) ----
    /// Open the column settings dialog on the cursor's column: Enter on a
    /// column header, or the menu's row.
    ColumnSettings,
    /// `O`: a new column after the cursor's, named first in the same dialog.
    AddColumn,
    /// The dialog's rows. Each is a toggle or a cycle that KEEPS the dialog
    /// open and relabels off the snapshot — the Settings list's rule — bar
    /// `ColumnName` (the name edited in place), `SortColumn` (one-shot) and
    /// `DeleteColumn` (armed by its first Enter).
    ColumnName,
    ColumnCollapse,
    SortColumn,
    ColumnWorkspace,
    ColumnClaudeMode,
    ColumnCodexSandbox,
    ColumnCodexApproval,
    ColumnTools,
    ColumnAutoRun,
    ColumnOnWorking,
    ColumnOnDone,
    ColumnRequiresMerge,
    ColumnReclaim,
    ColumnTrain,
    DeleteColumn,
    /// Open the release notes from the menu. Same argument, read even less
    /// often: what changed is a question for after an update, not a key.
    ReleaseNotes,
    // ---- sessions ----
    Agent,
    Shell,
    /// Shift+Enter on the board: open a one-line field on the selected card
    /// and put what is typed there in front of the ticket's live claude,
    /// submitted, without leaving the board. The composer's
    /// [`Verb::SaveStart`] is the same gesture one step earlier — there the
    /// ticket and the agent do not exist yet, so the press mints both and
    /// asks the title; here they do, so it only asks. On a ticket whose
    /// claude seat is EMPTY the press is the composer's second half over
    /// again (2026-09-03): start claude with the title as its first prompt,
    /// submitted, and stay — a ticket saved with plain Enter gets the same
    /// key later instead of a different one. Where another claude holds the
    /// checkout the same press opens the field at `queued` (T-294): the
    /// start is the loudest thing this key does, so it is the one that asks.
    Prompt,
    /// `S` on the ticket page: a second shell beside whatever is there.
    /// There is no Claude twin: a ticket holds ONE claude (2026-09-02), and a
    /// second seat is a shell — see STALE-MAP "One claude per ticket".
    ShellNew,
    Sleep,
    SleepAllDone,
    // ---- ticket lifecycle ----
    /// `a` — arms the archive chord, or restores at once when the ticket is
    /// already archived.
    ArchivePrefix,
    Archive,
    ArchiveAllDone,
    /// `z` — arms the snooze chord on the first preset (T-74).
    SnoozePrefix,
    /// `z` inside the chord — the next preset on the ring.
    SnoozeNext,
    /// Enter inside the chord — snooze until the preset's deadline.
    SnoozeConfirm,
    /// Esc inside the chord — never mind.
    SnoozeCancel,
    /// The menu row that flips whether a woken ticket returns lit
    /// (needs-you) or quietly; remembered in `prefs.json`.
    SnoozeQuiet,
    /// The Settings row that moves the private tmux server's status line
    /// between the bottom of an agent's pane and the top (T-264);
    /// remembered in `prefs.json`, pushed to the daemon, which owns the
    /// server.
    StatusLine,
    /// `t` — take the ticket off the merge train, or put it back (T-227).
    /// Flips `Ticket::manual_merge` through `Command::SetManualMerge`.
    ManualMerge,
    /// The Settings row that cycles the day a week starts on (Monday →
    /// Sunday → Saturday) — what the snooze ring's last rung means by "next
    /// week"; remembered in `prefs.json`.
    WeekStart,
    /// The Settings row that opens the notifications list (T-282) — a door,
    /// like Settings itself is a door in the menu.
    Notifications,
    /// Its five rows. The first is the master switch: off means the board
    /// says nothing out loud, and the other four are not offered.
    NotifyToggle,
    /// Whether a turn LANDING is one of the two moments, or only a blocked
    /// agent is.
    NotifyDone,
    /// The two sound rings, each cycled by Enter — and each PLAYS the sound
    /// it names as you walk it, the theme picker's rule that the cursor is
    /// the preview.
    NotifySoundNeedsYou,
    NotifySoundDone,
    /// Whether the banner also shows while the board's own terminal has
    /// focus. The sound plays either way; this row is only the banner.
    NotifyFocused,
    /// Whether anything is said about the ticket whose agent pane you are
    /// attached to (T-292). The one notification row that governs the SOUND
    /// as well: the pane already showed you, and there is nowhere to go look.
    NotifyInPane,
    /// Whether a banner may quote the AGENT (T-292) — its last line, and a
    /// raised hand's own sentence — or name only the ticket. mesimon's own
    /// reason word is not the agent's words and is never withheld.
    NotifyWords,
    /// The Settings row that turns the merge train on or off (2026-09-04):
    /// while every claude is idle, mesimon fast-forwards finished REVIEW
    /// branches and asks idle agents whose branch fell behind to rebase;
    /// remembered in `prefs.json`, pushed to the daemon, armed only while
    /// this board is open.
    MergeTrain,
    /// The Settings row under it: whether a train merge also pastes the
    /// merged notice into that agent.
    MergeTrainNotice,
    /// The Settings row that turns the agent tool surface on or off for this
    /// board (T-217). Board state, not a preference: it is per repo, it
    /// lives in `columns.toml`, and the daemon reads it at every spawn.
    McpTools,
    /// Project default for newly accepted sessions; existing seats retain theirs.
    AgentProvider,
    /// The Settings row under it (T-224): whether every claude mesimon
    /// starts on this board carries `brief::TEXT` in its system prompt.
    /// Board state like `McpTools`, and the switch the offer's dialog turns.
    SystemPrompt,
    /// The Settings row under it (T-279): the board's default column, where
    /// an agent's `create_ticket` lands a card that names no column. Enter
    /// cycles it through the columns in board order. Board state like
    /// `McpTools`, in `columns.toml`.
    DefaultColumn,
    /// The menu row that opens the agent-brief dialog (T-217/T-224): the
    /// text shown verbatim, with four ways out.
    BriefOffer,
    /// `c` in that dialog — the text on the screen to the terminal's
    /// clipboard, for a user who would rather put the words somewhere of
    /// their own. Writes nothing and stamps nothing, so the dialog stays
    /// open behind it.
    BriefCopy,
    /// `i` in that dialog — never offer it again. `mesimon doctor` still
    /// prints the brief and Settings still turns it on, which is what makes
    /// "never" affordable here.
    BriefIgnore,
    /// Re-read the user's shell startup files, so the environment new panes
    /// get is the one their terminal would give them.
    ReloadShellEnv,
    /// Fetch the checkout's upstream remote now (T-124) — the Esc menu's
    /// `Fetch origin` row. The header's `↓` only moves after a fetch, and
    /// the periodic one is opt-in, so this is the road most boards take.
    GitFetch,
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
    /// The same ramp, the other way (`shift+tab`): ten tints is a long walk
    /// back past the one just skipped.
    TagColorBack,
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
    HalfPageDown,
    HalfPageUp,
    NextFile,
    PrevFile,
    Refresh,
    ViewPrefix,
    Density,
    SwapPanes,
    /// `!` — the project's TERMINAL (T-273): the user's own shell on the
    /// private tmux server, persistent, in the checkout root — or in the
    /// ticket's worktree from a worktree ticket's page or branch diff. One
    /// verb on three screens; the screen says which directory.
    Terminal,
    // ---- move ----
    Cancel,
    // ---- drawer ----
    AdoptObserve,
    /// `n`: open the note the cursor means in the editor — the selected
    /// rail note, else the ticket's description, else a fresh one that
    /// becomes the description on save.
    NoteEdit,
    /// `N`: always a fresh note. Same axis as `n`, one step harder.
    NoteNew,
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
    /// `Tab` in the composer: grow it into the editor, title carried over,
    /// cursor in the description. On a board card: the same dialog, on the
    /// ticket's description.
    Describe,
    // ---- editor ----
    /// `^s`: save. Composing, the description is kept and the dialog folds
    /// back into the one-line composer it grew out of, where Enter mints. In
    /// a note editor it stays open, and a second press on a saved note tells
    /// the ticket's claude the note changed.
    EditorSave,
    /// `^S` (ctrl+shift+s), composing only: Shift on the editor's save axis
    /// — mint the ticket with its description AND start claude on it with the
    /// title submitted, staying on the board. The one-line composer's
    /// Shift+Enter, in the bigger room (2026-09-04, user request).
    EditorSaveStart,
    /// `enter`: in the title, move to the body; in the body, a newline.
    EditorNewline,
    EditorUp,
    EditorDown,
    /// `^g`: hand the body to the user's own editor (`$VISUAL`, `$EDITOR`,
    /// `vi`) in the terminal, and take back what it wrote — saved at once
    /// on a note, the way a `git commit` message is committed by the
    /// editor's write. The key is the one Claude Code teaches for exactly
    /// this ("open in external editor").
    EditorExternal,
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

/// The settings hierarchy; leaf rows keep their existing actions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SettingsSection {
    #[default]
    Root,
    Appearance,
    Behaviour,
    Agents,
}

impl SettingsSection {
    pub fn title(self) -> &'static str {
        match self {
            Self::Root => "SETTINGS",
            Self::Appearance => "APPEARANCE & NOTIFICATIONS",
            Self::Behaviour => "BEHAVIOUR",
            Self::Agents => "AGENTS",
        }
    }

    pub fn opener(self) -> Verb {
        match self {
            Self::Root => Verb::Settings,
            Self::Appearance => Verb::SettingsAppearance,
            Self::Behaviour => Verb::SettingsBehaviour,
            Self::Agents => Verb::SettingsAgents,
        }
    }

    pub fn for_verb(verb: Verb) -> Self {
        match verb {
            Verb::ThemePick | Verb::Notifications | Verb::StatusLine => Self::Appearance,
            Verb::MergeTrain
            | Verb::MergeTrainNotice
            | Verb::SnoozeQuiet
            | Verb::WeekStart
            | Verb::DefaultColumn => Self::Behaviour,
            Verb::SystemPrompt | Verb::McpTools | Verb::AgentProvider => Self::Agents,
            _ => Self::Root,
        }
    }
}

/// What the screen can currently do. Every availability predicate and every
/// state-dependent hint word reads from this and nothing else, so the footer,
/// the `?` overlay and the key dispatch can never disagree about whether an
/// action applies.
#[derive(Debug, Clone, Default)]
pub struct Ctx {
    pub settings_section: SettingsSection,
    pub agent_provider: AgentProvider,
    /// The ticket's existing provider, falling back to the project default.
    pub ticket_agent_provider: AgentProvider,
    pub column_agents: bool,
    pub col_naming: bool,
    pub col_offers_word: &'static str,
    // ---- board selection ----
    /// A card is under the cursor. Without it there is nothing to rename,
    /// move, delete, archive or start a session on.
    pub has_ticket: bool,
    /// More than one column exists — otherwise there is nowhere to move to.
    pub multi_column: bool,
    /// The selected ticket has at least one session record.
    pub ticket_has_sessions: bool,
    /// The selected ticket has a live claude session.
    pub ticket_has_agent: bool,
    /// One of those claude sessions holds a PANE — so there is a box a
    /// prompt can land in. Narrower than `ticket_has_agent`, which counts a
    /// `Sleeping` session: parked is live, but it has no process to type at.
    /// Live and not this is exactly Sleeping, which is how `c` knows to say
    /// `wake` and Shift+Enter to say `wake + ask` (2026-09-04).
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
    /// `P` is showing every card's reply, not only the cursor card's.
    pub peek_all: bool,
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
    // ---- the agent tier (T-217) ----
    /// This board hands its sessions the MCP tool surface. Board state, per
    /// repo — the Settings row's label and detail are the only readers.
    pub mcp_tools: bool,
    /// Sessions this board starts carry the agent brief in their system
    /// prompt (T-224). Board state, per repo — the Settings row's label and
    /// the offer read it.
    pub system_prompt: bool,
    /// Where an agent's `create_ticket` lands when it names no column
    /// (T-279): the chosen default while the board has it, else the first
    /// column — `Board::landing_column`'s word, so the row says what the
    /// daemon will do. Empty on a board with no columns.
    pub default_column: String,
    /// The brief is off, the repo's `CLAUDE.md` does not say it either, the
    /// tool it names is on, and the offer was not answered with "never". All
    /// four, because each one alone would offer noise.
    pub brief_offer: bool,
    // ---- the board's own checkout (T-124) ----
    /// There is a repository under the board at all — the checkout has been
    /// sampled. `v` on the board diffs its uncommitted work (T-221), and
    /// where there is no repository the key is inert rather than a message.
    pub git_repo: bool,
    /// The board sits on a WORKSPACE — a root with repositories nested one
    /// level under it (`RepoGit::repos`, T-225). A worktree there would be a
    /// worktree of the meta repo and none of the code, so the workspace
    /// choice is not offered, and the daemon refuses it besides.
    pub multi_repo: bool,
    /// The checkout's branch tracks a remote branch, so a fetch has
    /// somewhere to go. Gates the menu row: without an upstream there are no
    /// arrows on the header either.
    pub git_upstream: bool,
    /// That remote's name (`origin`), for the row's label.
    pub git_remote: String,
    /// A fetch is running now — the row stands down until it lands.
    pub git_fetching: bool,
    /// `MESIMON_GIT_FETCH` armed the periodic fetch; the row says so.
    pub git_fetch_on: bool,
    /// The row's detail, spelled by the app: `2 to push ∙ 1 to pull ∙ fetched
    /// 4m ago`. Words live here; the header carries the glyph form.
    pub git_fetch_note: String,
    /// The rail cursor is on a NOTE row, not a session.
    pub sel_note: bool,
    /// The subject ticket has somewhere `^k` could look: a description
    /// (`notes[0]`) or a claude whose transcript can be read (T-307). The
    /// bodies are not on the board, so this is the cheap proxy for "there
    /// might be a link" — one that holds none still gets a status line,
    /// never an empty dialog.
    pub ticket_linkable: bool,
    // ---- ticket screen ----
    /// The rail has a selected session.
    pub sel_session: bool,
    /// How many rows the ticket page's rail holds — the sessions, the
    /// `+ claude session` row when it stands, then every note. What `jk`
    /// gates on: a rail of one row is not a list to walk.
    pub ticket_rail_rows: usize,
    pub sel_sleeping: bool,
    pub sel_dead: bool,
    /// The rail cursor is on the `+ claude session` row (T-300) — the
    /// phantom row the rail carries while the ticket's claude seat is empty
    /// and it can still be filled. Enter there starts the session, which is
    /// why the row exists at all: the two spawn keys under an empty rail
    /// asked the reader to know which of `c` and `s` they wanted before
    /// they knew what either was.
    pub sel_new_agent: bool,
    /// A ticket may grow its own SHELL session (T-300). Off — the default —
    /// `s` and `S` on the ticket page and `s` on the board are inert and
    /// unhinted; the sessions a board already has are untouched, and
    /// `MESIMON_TICKET_SHELLS=1` opens the doors again. `!` is unaffected:
    /// the project's terminal is a place to stand, not a session of the
    /// ticket.
    pub ticket_shells: bool,
    // ---- worktree ----
    pub has_worktree: bool,
    /// The SUBJECT ticket's workspace is still open to change: no session
    /// and no worktree binding yet (the daemon's `set_workspace` lock,
    /// mirrored). Shift+Tab sets it then — on the board, on the ticket
    /// page, and in the description editor, which is where it lived until
    /// T-309. It closes for good the moment work starts on the ticket.
    pub workspace_open: bool,
    /// That ticket asks for a worktree of its own (`WorkspaceStrategy`,
    /// defaulted). The hint names where the press would leave it, the way
    /// `t`'s does — so this is what tells the two words apart.
    pub workspace_worktree: bool,
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
    /// The ask field's ticket is a shared-checkout ticket, so Shift+Tab can
    /// make the ask WAIT for the checkout to go quiet (2026-09-04). Never a
    /// worktree ticket: its checkout is its own. A pane is NOT required
    /// since T-294 — the delivery wakes a parked claude, or starts one.
    pub ask_queueable: bool,
    /// The ask field's toggle sits at `queued` — Enter parks the words.
    pub ask_queued: bool,
    /// A claude is mid-turn in the subject ticket's shared checkout, so a
    /// press that would START or WAKE a session there stops and asks first
    /// (T-294). The TUI's own read of `quiet::is_working`, and a HINT: it
    /// decides whether a field opens, never how the words are delivered.
    /// False on a worktree ticket — its checkout is its own.
    pub checkout_busy: bool,
    /// The subject ticket has an ask waiting (not yet pasted): Shift+Enter
    /// reopens the field on it, and a blank Enter there drops it.
    pub ticket_queued: bool,
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
    // ---- snooze ----
    /// The armed preset's label (`1h`, `tomorrow 9:00`…) while the snooze
    /// chord is up, so Enter's hint can name the pick; empty otherwise.
    /// Static because a hint is — the presets are a fixed ring.
    pub snooze_word: &'static str,
    /// A woken ticket returns lit (the preference; the menu row flips it).
    pub snooze_needs_you: bool,
    /// The tmux status line sits at the top of a pane (the preference; the
    /// Settings row flips it).
    pub status_top: bool,
    /// The day the week starts on — `snooze::Weekday::name()`, so the ring's
    /// last rung and the Settings row agree on the word. Empty in a bare
    /// `Ctx` (the row falls to a plain label); `App::ctx` always sets it.
    pub week_start_word: &'static str,
    /// The notification preferences (T-282), each one row's word. `notify`
    /// gates the other five: an off list is a single row.
    pub notify: bool,
    pub notify_done: bool,
    pub notify_focused: bool,
    pub notify_in_pane: bool,
    pub notify_words: bool,
    /// The two sound names — `notify::Sound::name()`, so the row, `doctor`
    /// and the ring agree on the spelling. Empty in a bare `Ctx`;
    /// `App::ctx` always sets them.
    pub notify_sound_needs_you: &'static str,
    pub notify_sound_done: &'static str,
    /// The merge train preference (the row's word), and whether the daemon
    /// says it is ARMED — the row's detail says `arming…` between the two.
    pub merge_train: bool,
    pub merge_train_notice: bool,
    pub merge_train_armed: bool,
    /// The train can reach the subject ticket: an attached worktree binding
    /// on it, and the train on (the preference, or the daemon saying it is
    /// armed). What makes `t` worth offering (T-227).
    pub train_reaches: bool,
    /// The subject ticket wears `manual_merge`: the train leaves it alone,
    /// and `t` puts it back on.
    pub manual_merge: bool,
    // ---- editor ----
    /// The note editor is up. Every editor binding is gated on it.
    pub editing: bool,
    /// The editor is composing a NEW ticket (title + description), so the
    /// composer's keys — workspace, tags — are live in it.
    pub editor_composing: bool,
    /// The cursor is in the body, not the title line.
    pub editor_body: bool,
    /// The editor holds changes not yet saved.
    pub editor_dirty: bool,
    /// The program `^g` hands the note's body to — the basename of
    /// `$VISUAL`, else `$EDITOR`, else `vi` — as the footer's word for it
    /// (`^g nvim`). Empty means no external editor is wired up (every test
    /// app, so a developer's own `$EDITOR` never reaches a golden), and the
    /// key is inert and unhinted.
    pub editor_word: &'static str,
    // ---- the column under the cursor (T-117) ----
    /// The board cursor rests on a column HEADER — `k` off the top card, or
    /// an empty column, whose only position is its header. No ticket is
    /// selected, so every ticket verb stands down, and four verbs take the
    /// column as their subject instead: Enter (its settings), `r` (rename),
    /// `HJKL` (move the column), `d` (delete). The hint word switches on
    /// this; the column NAME is not a hint (a hint is a `&'static str`) and
    /// goes in the status line and the dialog's title.
    pub col_header: bool,
    /// The column settings dialog's rows read the column they are on off
    /// these (a `MenuItem` label is a plain `fn(&Ctx)`, so the column is
    /// mirrored here rather than captured). `col_name` and the two rule
    /// targets are the user's own words, so they are `String`s; the rest are
    /// words from a fixed set.
    pub col_name: String,
    /// The dialog is on a column that does not exist yet (`O`): only the
    /// Name row stands.
    pub col_new: bool,
    /// The cursor is on the dialog's `Sort now` row, where `h`/`l` step the
    /// order and Enter runs it.
    pub col_on_sort: bool,
    pub col_sort_word: &'static str,
    pub col_collapsed: bool,
    pub col_workspace_word: &'static str,
    pub col_claude_mode_word: &'static str,
    pub col_codex_sandbox_word: &'static str,
    pub col_codex_approval_word: &'static str,
    /// What `inherit` resolves to — the user's own default mode, off the
    /// snapshot. Empty when unknown.
    pub col_inherit_mode: String,
    pub col_tools_word: &'static str,
    pub col_auto_run: bool,
    pub col_on_working: String,
    pub col_on_done: String,
    pub col_requires_merge: bool,
    pub col_reclaim: bool,
    pub col_train_word: &'static str,
    /// The dialog's Delete row was chosen once: the next Enter on it sends.
    pub col_delete_armed: bool,
    /// Live tickets in the dialog's column: a delete is refused while any.
    pub col_live: usize,
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

/// The workspace toggle's word on the board and the ticket page (T-309), and
/// the one place the two screens can agree. It names the DESTINATION — `t`'s
/// idiom — because the card's mark and the page's state row already say where
/// the ticket stands. EMPTY while the press cannot act: the choice is locked,
/// or the board is a workspace of repositories where a worktree of the root
/// would hold none of the code (T-225). The key stays live there so it can
/// say which; only the hint stands down.
fn workspace_hint(c: &Ctx) -> &'static str {
    if c.multi_repo || !c.workspace_open {
        ""
    } else if c.workspace_worktree {
        "shared checkout"
    } else {
        "own worktree"
    }
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
        // The right cluster's last word on every screen where `?` resolves
        // (`footer_split`); a barrier scope inherits nothing and so says
        // nothing — there `?` is text or a cancel.
        prio: 255,
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
        // one, and the hint says which it will be BEFORE the press. On a
        // column header it opens the column's settings (T-117).
        hint: |c| {
            if c.col_header {
                "column settings"
            } else if c.ticket_hot {
                "go to the agent"
            } else {
                "ticket page"
            }
        },
        avail: |c| c.has_ticket || c.col_header,
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
        // An empty seat gets the composer's sentence: the press starts
        // claude on the title, submitted, and stays. A ticket saved with
        // plain Enter is one press behind a Shift+Enter one, and this is
        // that press — unless another claude is working in the same
        // checkout (T-294), where the field opens at `queued` instead, so
        // the start waits its turn rather than becoming a second writer in
        // one index.
        hint: |c| {
            // A parked agent has no box to type into, and until 2026-09-04
            // that left the key inert there — `c`, wait, come back, ask.
            // Now the press opens the same field and the daemon wakes the
            // agent on the way (user: "ask claude on sleeping agent auto
            // wakes it for the user"); the hint says the extra thing it
            // does. Live but paneless is exactly Sleeping.
            if c.ticket_queued {
                "edit the queued ask"
            } else if c.ticket_has_agent && !c.ticket_promptable {
                agent_hint(c, "wake + ask claude", "wake + ask codex")
            } else if c.ticket_has_agent {
                agent_hint(c, "ask claude", "ask codex")
            } else if c.checkout_busy {
                // An empty seat on a checkout somebody else is working in
                // (T-294): the press opens the field instead of spawning,
                // so the start can wait its turn. The word names what the
                // key is for, and the field says now or queued.
                agent_hint(c, "start claude", "start codex")
            } else {
                agent_hint(c, "ask claude the title", "ask codex the title")
            }
        },
        // Every ticket, at every stage of its seat: empty (the title is the
        // prompt), parked (wake, then ask), paned (ask). `rich_keys` is the
        // ShiftEnter clause: where the terminal spells this as a plain Enter
        // the key must be inert AND unhinted, or the press would focus the
        // pane instead of opening a field.
        avail: |c| c.has_ticket && c.rich_keys,
        class: Class::Plain,
        group: Group::Sessions,
        mutates: true,
        prio: 12,
    },
    Binding {
        // Beside `enter` in the footer exactly when `enter` goes to the
        // agent instead (author 2026-09-03): that is when the page needs
        // its own key. On a cold ticket `enter` IS the page, so `space` is
        // silent there — bound, one more spelling of the same move, the way
        // `m` stays bound while unhinted.
        keys: &[Key::Space],
        verb: Verb::TicketScreen,
        show: "space",
        hint: |c| if c.ticket_hot { "ticket page" } else { "" },
        avail: |c| c.has_ticket,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 11,
    },
    Binding {
        keys: &[Key::Char('o')],
        verb: Verb::OpenTicket,
        show: "o",
        // "new", not "open": it mints a card. "open ticket" read as opening
        // the one under the cursor (T-158).
        hint: |_| "new ticket",
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
        verb: Verb::Agent,
        show: "c",
        hint: |c| {
            // Live but paneless is exactly Sleeping: the press wakes the
            // parked conversation and attaches, so the hint says so.
            if c.ticket_has_agent && !c.ticket_promptable {
                match c.ticket_agent_provider {
                    AgentProvider::ClaudeCode => "wake claude",
                    AgentProvider::Codex => "wake codex",
                }
            } else if c.ticket_has_agent {
                agent_word(c.ticket_agent_provider)
            } else {
                match c.agent_provider {
                    AgentProvider::ClaudeCode => "start claude",
                    AgentProvider::Codex => "start codex",
                }
            }
        },
        avail: |c| c.has_ticket,
        class: Class::Plain,
        group: Group::Sessions,
        mutates: true,
        prio: 0,
    },
    Binding {
        // Overlay-only for the same reason as `c` above, and gated with the
        // ticket page's own two (T-300): where a ticket may not grow a
        // shell, the board may not start one either.
        keys: &[Key::Char('s')],
        verb: Verb::Shell,
        show: "s",
        hint: |_| "shell",
        avail: |c| c.has_ticket && c.ticket_shells,
        class: Class::Plain,
        group: Group::Sessions,
        mutates: true,
        prio: 0,
    },
    Binding {
        // The footer names the immediate nudge; the overlay also teaches
        // the adjacent-column move with a cancellable preview.
        keys: &[Key::Char('>'), Key::Char('<')],
        verb: Verb::Grab,
        show: "> <",
        hint: |_| "move card, twice",
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
        // it. This is the one the footer teaches now. "now" left with `> <`:
        // it was drawing a contrast with the aiming gesture that the footer no
        // longer sets up, and moving the card IS the verb.
        //
        // `can_nudge` is the wider predicate on purpose — `> <` needs a second
        // column, a nudge also reorders inside one — so the footer offers it
        // on the one-column board where `> <` had nothing to say.
        //
        // `HJKL` joined the key list 2026-09-04 (user request), the picker's
        // arrangement: `hjkl` steps the cursor, `HJKL` steps it carrying the
        // card, and the four Alt atoms are the same entry, so the accelerator
        // cannot reach a move the floor does not make. The footer names the
        // shifted spelling now, because every terminal can send it — the
        // `option` one worked on the machine this ships to and read as a
        // dead key on the ones that eat the modifier.
        keys: &[
            Key::Char('H'),
            Key::Char('J'),
            Key::Char('K'),
            Key::Char('L'),
            Key::AltLeft,
            Key::AltRight,
            Key::AltUp,
            Key::AltDown,
        ],
        verb: Verb::Nudge,
        show: "HJKL",
        // The same entry moves the COLUMN when the cursor is on its header
        // (T-117): one nudge, the thing under the cursor, one step.
        hint: |c| if c.col_header { "move column" } else { "move card" },
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
        // `o` mints a card in the column; `O` mints a COLUMN after it (T-117)
        // — the same act one level wider, the way `x`/`X` widen the sleep.
        // The footer teaches this on column headers; the key and help entry
        // remain available anywhere on the board.
        keys: &[Key::Char('O')],
        verb: Verb::AddColumn,
        show: "O",
        hint: |_| "new column",
        avail: always,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 45,
    },
    Binding {
        keys: &[Key::Char('r')],
        verb: Verb::Rename,
        show: "r",
        hint: |c| if c.col_header { "rename column" } else { "rename" },
        avail: |c| c.has_ticket || c.col_header,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 70,
    },
    Binding {
        // The same verb the ticket page's `v` carries, on the screen whose
        // subject is the repository rather than a ticket: the board diffs the
        // CHECKOUT's uncommitted work (T-221). Which diff you get is answered
        // by which screen you pressed it on, never by what the cursor is over
        // — a worktree ticket's branch diff is still `space` then `v`.
        //
        // `Group::View` and not `Worktree`: the board has no other Worktree
        // binding, so one here would mint a `BRANCH` section in `?` over a
        // working-tree diff. `p show replies ∙ v diff` is what it is.
        //
        // Overlay-only (`prio: 0`), because the hint sits where it operates:
        // `chrome::git_clause` draws it beside the checkout's own `∙ 3
        // changed`, which is the thing it reads — the ticket rail's `c s x`
        // and the PREVIEW heading's `{ } page` are the same idiom (T-158).
        keys: &[Key::Char('v')],
        verb: Verb::OpenDiff,
        show: "v",
        hint: |_| "diff",
        avail: |c| c.git_repo,
        class: Class::Plain,
        group: Group::View,
        mutates: false,
        prio: 0,
    },
    Binding {
        // The project's terminal (T-273): a shell in the checkout, on the
        // private tmux server, that outlives the visit — `git fetch` and
        // `git push` without leaving the board or opening a tab. Overlay-only
        // on every screen (T-277, user: "keep only on ? help menu"): it drew
        // in the header's git clause beside `v diff` for a day, and a key
        // that is always there is what `?` lists. `always`: a board with no
        // repository still has a directory to stand in.
        keys: &[Key::Char('!')],
        verb: Verb::Terminal,
        show: "!",
        hint: |_| "terminal",
        avail: always,
        class: Class::Plain,
        group: Group::View,
        mutates: false,
        prio: 0,
    },
    Binding {
        // The train's per-ticket door (T-227, user: "let the user cancel it
        // per ticket easily"). One press takes the card off the train, the
        // next puts it back; the card's `auto-merge ∙ next` row becomes
        // `auto-merge ∙ off` and `m` is the only road for it. Hinted in the
        // footer, on both screens, exactly while the train can reach the
        // ticket or has been told not to: a key nobody is told about is not
        // an easy cancel, and the hint is itself the indication that mesimon
        // has plans for this branch. `t` is train; the user's "double esc"
        // could not be it, since Esc is the menu here and `back` on the
        // ticket page. A toggle, not a chord: both directions are visible on
        // the card and reversible with the same key.
        keys: &[Key::Char('t')],
        verb: Verb::ManualMerge,
        show: "t",
        hint: |c| if c.manual_merge { "auto-merge" } else { "merge by hand" },
        avail: |c| c.has_ticket && (c.train_reaches || c.manual_merge),
        class: Class::Plain,
        group: Group::Worktree,
        mutates: true,
        prio: 62,
    },
    Binding {
        // `Tab` on a card is the composer's `Tab` a ticket late: the card
        // grows into the description editor, the same dialog over the board
        // (2026-09-03, T-163). It took `needs you`'s key — the attention
        // walk had one hint in the footer and was never once pressed, and
        // the description is what the board's cursor most often wants next.
        // Hinted, unlike `n` under it: this is the gesture the footer
        // teaches, and `n` is the note axis the overlay keeps.
        keys: &[Key::Tab],
        verb: Verb::Describe,
        show: "tab",
        hint: |_| "describe",
        avail: |c| c.has_ticket,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 65,
    },
    Binding {
        // The composer's workspace pick, reachable for as long as it means
        // anything (T-309): a ticket that has not started yet can still be
        // told to take a worktree of its own, and until now the only way in
        // was the description editor. Same key, same toggle, one screen out.
        // Overlay-only — the board's footer is already at its width at 120
        // columns, and the card says which way the ticket is set (the branch
        // mark, dormant while nothing is cut yet).
        //
        // `m`'s shape, and for `m`'s reason: LIVE while unhinted, because a
        // locked ticket has something worth saying — "T-9 has a session" —
        // and the press that comes from muscle memory deserves it. Silence
        // is what the first cut shipped, and it read as a broken key
        // (user, 2026-09-07). The invariant is unbroken: a key that is
        // HINTED always works.
        keys: &[Key::BackTab],
        verb: Verb::CycleWorkspace,
        show: "shift+tab",
        hint: workspace_hint,
        avail: |c| c.has_ticket && !c.ticket_archived,
        class: Class::Plain,
        group: Group::Worktree,
        mutates: true,
        prio: 0,
    },
    Binding {
        // `n` opens the note the cursor means: the selected rail note on the
        // ticket page, else the description, else a fresh note that becomes
        // the description. Overlay-only here: the board's footer is for
        // moving and opening, the argument `a` and `d` make — and `tab`,
        // above, is the description's own hinted key.
        keys: &[Key::Char('n')],
        verb: Verb::NoteEdit,
        show: "n",
        hint: |c| if c.sel_note { "edit note" } else { "note" },
        avail: |c| c.has_ticket,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 0,
    },
    Binding {
        // Shift on the note axis: the same target, harder — always a NEW
        // note, never the one under the cursor. Overlay-only; `n` teaches
        // the axis.
        keys: &[Key::Char('N')],
        verb: Verb::NoteNew,
        show: "N",
        // Silent on the board (the shape `space` and `> <` use): the ticket
        // page's overlay teaches it, and the board's overlay at 30 rows had
        // exactly one row to spare — spending two pushed `p` off the end.
        hint: |_| "",
        avail: |c| c.has_ticket,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 0,
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
        // chip that names it is the only place it is taught. `X` IS
        // shift-of-`x` (2026-09-04, user): the same verb, sleep sessions,
        // widened from the selection to the done column — shift never
        // switches verbs, and the `Z` it replaced had become one, once `z`
        // snoozed the ticket beside it. Gated on the menu row's own
        // predicate, so the key works exactly when the offer stands.
        keys: &[Key::Char('X')],
        verb: Verb::SleepAllDone,
        show: "X",
        hint: |_| "sleep the finished agents",
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
        // The ticket's links (T-256): what its notes and its agent's latest
        // words point at, listed. A Ctrl-letter like `^t`, and overlay-only
        // like it — the footer is the selection's, and this is looked up. A
        // ticket with neither a note nor a transcript has nothing to list,
        // so the key is inert there; one whose words hold no link gets a
        // status line, not an empty dialog.
        keys: &[Key::Ctrl('k')],
        verb: Verb::Links,
        show: "^k",
        hint: |_| "links",
        avail: |c| c.has_ticket && c.ticket_linkable,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: false,
        prio: 0,
    },
    Binding {
        // Shift on the same axis: the first link, no dialog — the Jira
        // ticket a card mirrors, one press from the board. Off the legacy
        // floor on `^S`'s clause: a terminal without the kitty tier sends
        // the bare `^k`, which opens the dialog, and that is the safe half.
        keys: &[Key::Ctrl('K')],
        verb: Verb::LinkFirst,
        show: "^K",
        hint: |_| "open first link",
        avail: |c| c.has_ticket && c.ticket_linkable && c.rich_keys,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: false,
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
        // Overlay-only like `a`: a snooze IS an archive, with a deadline.
        // `z` is "zzz" beside `Z` (sleep the done agents) — two zzz's, one
        // for the card and one for its agents — and it is free here: the
        // only other `z` is the diff viewer's view prefix, a reading screen
        // with no ticket verbs, and neither is reachable from the other.
        keys: &[Key::Char('z')],
        verb: Verb::SnoozePrefix,
        show: "z",
        hint: |_| "snooze",
        avail: |c| c.has_ticket && !c.ticket_archived,
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
        hint: |c| if c.col_header { "delete column" } else { "delete" },
        avail: |c| c.has_ticket || c.col_header,
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
        // Overlay-only, like `p` and for the same reason (T-237, user: "no
        // need to hint this"). `P` IS shift-of-`p`: the same reply row,
        // widened from the cursor card to every card on the board; pressing
        // it again narrows back to the cursor card, never to nothing.
        keys: &[Key::Char('P')],
        verb: Verb::PeekAll,
        show: "P",
        hint: |c| {
            if c.peek_all {
                "replies on the cursor card only"
            } else {
                "show every reply"
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
        // In the footer's RIGHT cluster beside `? keys` (T-158): the app
        // keys stand apart from the card's, so the board's one door to the
        // board-wide actions is named without competing with the selection.
        prio: 254,
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
        // A vertical list takes ↓ ↑ and nothing sideways. Gated on the rail
        // having somewhere to go rather than on there being sessions
        // (T-300): the rail is sessions, then the `+ claude session` row,
        // then the notes, and a ticket with no session at all still holds
        // two rows to walk. One row is not a list, and the key is inert
        // there — which is what keeps the hint honest.
        keys: &[Key::Char('j'), Key::Down, Key::Char('k'), Key::Up],
        verb: Verb::CursorDown,
        show: "jk",
        hint: |c| if c.ticket_has_sessions { "select session" } else { "select row" },
        avail: |c| c.ticket_rail_rows > 1,
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
        // On the PREVIEW heading (T-158), beside the zone it pages.
        prio: 0,
    },
    Binding {
        keys: &[Key::Enter],
        verb: Verb::Act,
        show: "enter",
        hint: |c| {
            if c.sel_new_agent {
                match c.agent_provider {
                    AgentProvider::ClaudeCode => "start claude",
                    AgentProvider::Codex => "start codex",
                }
            } else if c.sel_note {
                "edit note"
            } else if c.sel_dead {
                "resume"
            } else if c.sel_sleeping {
                "wake"
            } else {
                "focus"
            }
        },
        avail: |c| c.sel_session || c.sel_note || c.sel_new_agent,
        class: Class::Plain,
        group: Group::Sessions,
        mutates: true,
        prio: 20,
    },
    Binding {
        keys: &[Key::Char('c')],
        verb: Verb::Agent,
        show: "c",
        hint: |c| {
            // Live but paneless is exactly Sleeping: the press wakes the
            // parked conversation and attaches, so the hint says so.
            // Otherwise SILENT — the key stays bound, the trailer under the
            // rail stops naming it. A claude that is up is a row already
            // listed, which `enter` on that row says (author 2026-09-03),
            // and an EMPTY seat is the `+ claude session` row, which says
            // the same thing about starting one (T-300). Both would be a
            // second spelling of a row the reader is looking at.
            if c.ticket_has_agent && !c.ticket_promptable {
                match c.ticket_agent_provider {
                    AgentProvider::ClaudeCode => "wake claude",
                    AgentProvider::Codex => "wake codex",
                }
            } else {
                ""
            }
        },
        avail: always,
        class: Class::Plain,
        group: Group::Sessions,
        mutates: true,
        // Under the rail (T-158): the sessions' keys sit under the sessions.
        prio: 0,
    },
    Binding {
        // Gated since T-300 (user: "keep the feature but gate it for now").
        // A ticket's own shell is whole — the rail lists it, the preview
        // reads its pane, `x` sleeps it — but a rail that offered `c` and
        // `s` side by side asked a first-time reader to choose between two
        // words before either had a meaning, and the shell is the one they
        // did not want. `MESIMON_TICKET_SHELLS=1` puts both keys back.
        keys: &[Key::Char('s')],
        verb: Verb::Shell,
        show: "s",
        hint: |_| "shell",
        avail: |c| c.ticket_shells,
        class: Class::Plain,
        group: Group::Sessions,
        mutates: true,
        // Under the rail (T-158).
        prio: 0,
    },
    Binding {
        keys: &[Key::Char('S')],
        verb: Verb::ShellNew,
        show: "S",
        hint: |_| "another shell",
        avail: |c| c.ticket_shells,
        class: Class::Plain,
        group: Group::Sessions,
        mutates: true,
        prio: 0,
    },
    Binding {
        // The project's terminal (T-273), from the ticket's page: in the
        // ticket's worktree when it has one — the directory that is hard to
        // reach — else the checkout. Not a session of the ticket (`s` is
        // that, and is why `!` stayed out of T-300's gate: a place to stand
        // is not a seat on the ticket): nothing joins the rail, and the
        // shell is the same one the board's `!` finds. Overlay-only (T-277):
        // the word still says which directory in `?`, the footer stays the
        // rail's.
        keys: &[Key::Char('!')],
        verb: Verb::Terminal,
        show: "!",
        hint: |c| if c.has_worktree { "terminal in worktree" } else { "terminal" },
        avail: always,
        class: Class::Plain,
        group: Group::Sessions,
        mutates: false,
        prio: 0,
    },
    Binding {
        keys: &[Key::Char('x')],
        verb: Verb::Sleep,
        show: "x",
        // Two words, one verb: a corpse cannot be slept and a sleeper
        // cannot be dismissed, so the same key means the only thing it
        // could mean for the row under the cursor. `Enter` next door
        // switches on `sel_dead` the same way ("resume"). On a SLEEPER the
        // key still wakes but says nothing (author 2026-09-04): `enter` on
        // the same row already reads "wake" and `c` under the rail "wake
        // claude", so a third spelling of one act was noise in the trailer.
        hint: |c| {
            if c.sel_dead {
                "dismiss"
            } else if c.sel_sleeping {
                ""
            } else {
                "sleep"
            }
        },
        avail: |c| c.sel_session,
        class: Class::Plain,
        group: Group::Sessions,
        mutates: true,
        // Under the rail (T-158): the row it acts on is right above it.
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
        // The board's `t`, on the ticket page (T-227): same verb, same
        // words, beside the `m` it hands the merge to.
        keys: &[Key::Char('t')],
        verb: Verb::ManualMerge,
        show: "t",
        hint: |c| if c.manual_merge { "auto-merge" } else { "merge by hand" },
        avail: |c| c.has_ticket && (c.train_reaches || c.manual_merge),
        class: Class::Plain,
        group: Group::Worktree,
        mutates: true,
        prio: 62,
    },
    Binding {
        // The board's shift+tab, on the ticket page (T-309): same verb, same
        // toggle. HINTED here where it is overlay-only on the board — the
        // ticket page's footer has the room and this page is where a ticket
        // is set up before anyone starts on it. The word is the DESTINATION,
        // `t`'s idiom above: where the press would leave the ticket, since
        // the state row beside it already says where it stands.
        keys: &[Key::BackTab],
        verb: Verb::CycleWorkspace,
        show: "shift+tab",
        hint: workspace_hint,
        avail: |c| c.has_ticket && !c.ticket_archived,
        class: Class::Plain,
        group: Group::Worktree,
        mutates: true,
        prio: 64,
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
        // `n` opens the note the cursor means: the selected rail note on the
        // ticket page, else the description, else a fresh note that becomes
        // the description.
        keys: &[Key::Char('n')],
        verb: Verb::NoteEdit,
        show: "n",
        // One word whether the description exists or not: at 120 columns
        // anything longer than `describe` is what the footer drops, and a
        // described ticket is the common case once this exists. `describe`
        // is true either way — the editor opens on what is there.
        hint: |c| if c.sel_note { "edit note" } else { "describe" },
        // After `d` (90) and before `q`: at 120 columns something has to
        // give, and it is the pop — every screen teaches `q` — never the
        // destructive key the page is where you are meant to find.
        avail: always,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 95,
    },
    Binding {
        // Shift on the note axis: the same target, harder — always a NEW
        // note, never the one under the cursor. Overlay-only; `n` teaches
        // the axis.
        keys: &[Key::Char('N')],
        verb: Verb::NoteNew,
        show: "N",
        hint: |_| "new note",
        avail: always,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 0,
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
        // The board's links key on the ticket's own page, overlay-only like
        // the board's. The state row named it while a fetched body held a
        // link until T-312; `?` is the one home now.
        keys: &[Key::Ctrl('k')],
        verb: Verb::Links,
        show: "^k",
        hint: |_| "links",
        avail: |c| c.ticket_linkable,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: false,
        prio: 0,
    },
    Binding {
        // See the board's copy.
        keys: &[Key::Ctrl('K')],
        verb: Verb::LinkFirst,
        show: "^K",
        hint: |_| "open first link",
        avail: |c| c.ticket_linkable && c.rich_keys,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: false,
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
        keys: &[Key::Char('z')],
        verb: Verb::SnoozePrefix,
        show: "z",
        hint: |_| "snooze",
        avail: |c| !c.ticket_archived,
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
        // Overlay-only: the BOARD chip in the header already says where `q`
        // goes, and the footer has better uses for the cell.
        keys: &[Key::Char('q'), Key::Esc, Key::Ctrl(']'), Key::Ctrl('5')],
        verb: Verb::Back,
        show: "q",
        hint: |_| "board",
        avail: always,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 0,
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
        prio: 0, // Beside the hunk pane.
    },
    Binding {
        keys: &[Key::Char('n'), Key::Char('N')],
        verb: Verb::NextFile,
        show: "n N",
        hint: |_| "file",
        avail: always,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 0, // Beside FILES, or the file heading in the single-pane view.
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
        prio: 0, // Beside the hunk pane.
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
        // The project's terminal (T-273) on the diff's own target: a branch
        // diff with its worktree present opens the worktree's, the checkout
        // diff the checkout's — the same directory the diff reads. Until
        // T-273 this was `$SHELL` in the foreground, gone on return.
        // Overlay-only since T-277, like the other two.
        keys: &[Key::Char('!')],
        verb: Verb::Terminal,
        show: "!",
        hint: |c| if c.worktree_present { "terminal in worktree" } else { "terminal" },
        avail: always,
        class: Class::Plain,
        group: Group::Worktree,
        mutates: false,
        prio: 0,
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

/// The release notes screen: read-only, so the diff's reading keys on the
/// diff's verbs — `App::dispatch` routes them on the screen. `n N` step by
/// release the way they step by file there; nothing here mutates.
static RELEASES: &[Binding] = &[
    Binding {
        keys: &[Key::Char('j'), Key::Down, Key::Char('k'), Key::Up],
        verb: Verb::ScrollDown,
        show: "jk",
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
        hint: |_| "next / previous release",
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

/// The `d` chord tail. Nothing else is bound here: any other key cancels, so
/// the only way to delete is to mean it twice.
static DELETE: &[Binding] = &[
    Binding {
        keys: &[Key::Char('d')],
        verb: Verb::Delete,
        show: "d",
        hint: |c| if c.col_header { "delete column" } else { "delete ticket" },
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

/// The `z` chord tail (T-74). A tail you stay in: `z` walks the preset
/// ring, Enter takes the pick, Esc leaves — and, like the other chords, a
/// key bound to nothing here cancels rather than acts.
static SNOOZE: &[Binding] = &[
    Binding {
        keys: &[Key::Char('z')],
        verb: Verb::SnoozeNext,
        show: "z",
        hint: |_| "next",
        avail: always,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: false,
        prio: 10,
    },
    Binding {
        keys: &[Key::Enter],
        verb: Verb::SnoozeConfirm,
        show: "enter",
        hint: |c| crate::snooze::hint_for_label(c.snooze_word),
        avail: always,
        class: Class::Grace,
        group: Group::Ticket,
        mutates: true,
        prio: 20,
    },
    Binding {
        keys: &[Key::Esc],
        verb: Verb::SnoozeCancel,
        show: "esc",
        hint: |_| "cancel",
        avail: always,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: false,
        prio: 30,
    },
];

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
        // `shift+tab` walks the ramp the other way — one binding, so the
        // footer spends no cell on it (`directional` tells them apart).
        keys: &[Key::Tab, Key::BackTab],
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
        keys: &[Key::Char('>'), Key::Char('<')],
        verb: Verb::Grab,
        show: "> <",
        hint: |_| "repeat to move",
        avail: always,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 10,
    },
    Binding {
        keys: &[Key::Esc, Key::Char('q')],
        verb: Verb::Cancel,
        show: "other key",
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

/// The settings submenu's three shapes are the menu's; only the last word
/// differs — Esc here goes BACK to the menu, not out of it.
static SETTINGS: &[Binding] = &[
    Binding {
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
        mutates: false,
        prio: 20,
    },
    Binding {
        keys: &[Key::Char('q'), Key::Esc],
        verb: Verb::Back,
        show: "esc",
        hint: |_| "back",
        avail: always,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 250,
    },
];

/// The CLAUDE.md dialog's four answers (T-217). Enter and Esc are the list
/// dialogs' own verbs, which is what keeps "one verb per key across screens"
/// true; `c` and `i` are the two this surface adds.
///
/// The three answers wear `Group::Sessions` — the drawer's precedent, where a
/// dialog's keys take the group of what they act on, and what these act on is
/// what every future session gets told. `Group::App` would have been the
/// tempting read and is structurally wrong: `footer_split` reserves it for the
/// right-hand cluster, so an answer in it lands in the FOOTER rather than in
/// the frame's own edge, which is where a dialog teaches itself.
///
/// `c` writes nothing and stamps nothing — copying is not evidence of pasting
/// — so it is the one key here that leaves the dialog standing. `i` is the
/// only way to answer "never", and it is affordable exactly because
/// `mesimon doctor` prints the brief whatever the stamp says and the Settings
/// row still turns it on.
static BRIEF: &[Binding] = &[
    Binding {
        keys: &[Key::Enter],
        verb: Verb::Act,
        show: "enter",
        // The switch's fate, not the key's: the dialog above already says
        // where the text goes, and this is the half the user is deciding.
        hint: |_| "turn on",
        avail: always,
        class: Class::Plain,
        group: Group::Sessions,
        mutates: true,
        prio: 10,
    },
    Binding {
        keys: &[Key::Char('c')],
        verb: Verb::BriefCopy,
        show: "c",
        hint: |_| "copy",
        avail: always,
        class: Class::Plain,
        group: Group::Sessions,
        mutates: false,
        prio: 20,
    },
    Binding {
        keys: &[Key::Char('i')],
        verb: Verb::BriefIgnore,
        show: "i",
        hint: |_| "never ask again",
        avail: always,
        class: Class::Plain,
        group: Group::Sessions,
        mutates: true,
        prio: 30,
    },
    Binding {
        keys: &[Key::Char('q'), Key::Esc],
        verb: Verb::Back,
        show: "esc",
        // Not "back": this dialog is a question, and Esc's answer is "not
        // now" — different from `i`'s "not ever", and it has to read so.
        hint: |_| "not now",
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
        label: |c| format!("Sleep {} on finished tickets", plural(c.bulk_sleep, "agent")),
        detail: |c| match gib(c.bulk_sleep_bytes) {
            Some(g) => format!("frees ~{g:.1}GiB ∙ they wake where they left off"),
            None => "frees their memory ∙ they wake where they left off".into(),
        },
        avail: |c| c.bulk_sleep > 0,
        key: "X",
    },
    MenuItem {
        verb: Verb::ArchiveAllDone,
        label: |c| {
            let noun = if c.bulk_archive == 1 { "ticket" } else { "tickets" };
            format!("Archive {} finished {noun}", c.bulk_archive)
        },
        detail: |_| "the columns that offer it ∙ restore any of them later".into(),
        avail: |c| c.bulk_archive > 0,
        key: "",
    },
    // The agent-brief offer (T-217, re-aimed T-224). Its row is the dialog's
    // door, so the detail says what the dialog will show rather than what it
    // will do — nothing is switched until the text is on the screen.
    MenuItem {
        verb: Verb::BriefOffer,
        // The outcome, not the mechanism: a system prompt is how, an agent
        // reading its ticket is what. The how belongs in the detail, one
        // line down, next to the reach (only sessions mesimon starts) and
        // the promise that nothing is switched blind.
        label: |_| "Tell agents to read the ticket".into(),
        detail: |_| "one line for agents mesimon starts ∙ you see it first".into(),
        avail: |c| c.brief_offer,
        key: "",
    },
    MenuItem {
        verb: Verb::ExternalDrawer,
        label: |_| "External sessions".into(),
        detail: |_| "agent sessions in this repo that mesimon did not start".into(),
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
    // The door to the preferences. Never a suggestion — a setting is not
    // something worth doing right now — and it names what is behind it, so
    // nobody opens it to find out.
    MenuItem {
        verb: Verb::Settings,
        label: |_| "Settings".into(),
        // Names what is behind the door, and fits the row: the detail's
        // budget is 56 cells, so the list is the interesting half rather than
        // all seven rows (it named four of six before this).
        detail: |_| "appearance & notifications, behaviour, agents".into(),
        avail: always,
        key: "",
    },
    // What this build is, in its own words. Never a
    // suggestion either — the header's update chip is the one that says a
    // NEWER one exists, and this row reads the notes the binary carries.
    MenuItem {
        verb: Verb::ReleaseNotes,
        label: |_| "Release notes".into(),
        detail: |_| "what changed in each version, newest first".into(),
        avail: always,
        key: "",
    },
    MenuItem {
        verb: Verb::Quit,
        label: |_| "Quit mesimon".into(),
        detail: |_| "sessions keep running".into(),
        avail: always,
        key: "q",
    },
];

/// The settings submenu's rows: every preference, and nothing that acts on
/// the board. A row here is a toggle or a picker, so choosing one keeps the
/// submenu open — the row relabels itself and the change is on the screen.
/// Rows are `MenuItem`s so the two lists draw through one function; none is
/// ever a suggestion (`every_suggestion_is_a_menu_row` holds them apart).
static SETTINGS_ITEMS: &[MenuItem] = &[
    MenuItem {
        verb: Verb::SettingsAppearance,
        label: |_| "Appearance & notifications".into(),
        detail: |_| "theme, notifications, status line".into(),
        avail: always,
        key: "",
    },
    MenuItem {
        verb: Verb::SettingsBehaviour,
        label: |_| "Behaviour".into(),
        detail: |_| "auto merge, snooze, week start, default column".into(),
        avail: always,
        key: "",
    },
    MenuItem {
        verb: Verb::SettingsAgents,
        label: |_| "Agents".into(),
        detail: |_| "provider, brief and tools".into(),
        avail: always,
        key: "",
    },
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
    // Where the tmux status line sits over an agent's pane (T-264). A
    // preference the daemon is told, like the train: it owns the server.
    // Notifications (T-282). A door, not a switch — five rows do not fit in
    // this list, and the list under it is where they say what they will do.
    // Third, with the other two preferences about what the board shows YOU:
    // `draw_list` does not scroll and this list already outruns a 20-row
    // terminal, so a door appended last would be the row nobody can reach.
    MenuItem {
        verb: Verb::Notifications,
        label: |c| {
            if c.notify {
                "Notifications: on".into()
            } else {
                "Notifications: off".into()
            }
        },
        detail: |c| {
            if c.notify {
                format!(
                    "a banner and a sound when an agent needs you ∙ {} ∙ enter opens them",
                    or(c.notify_sound_needs_you, "Glass")
                )
            } else {
                "the board says nothing outside its own window ∙ enter opens them".into()
            }
        },
        avail: always,
        key: "",
    },
    MenuItem {
        verb: Verb::StatusLine,
        label: |c| {
            if c.status_top {
                "Status line at the top".into()
            } else {
                "Status line at the bottom".into()
            }
        },
        detail: |c| {
            if c.status_top {
                "tmux's bar over an agent's pane ∙ enter moves it down".into()
            } else {
                "tmux's bar over an agent's pane ∙ enter moves it up".into()
            }
        },
        avail: always,
        key: "",
    },
    // How a snoozed ticket comes back.
    MenuItem {
        verb: Verb::SnoozeQuiet,
        label: |c| {
            if c.snooze_needs_you {
                "Snooze returns with needs-you".into()
            } else {
                "Snooze returns quietly".into()
            }
        },
        detail: |c| {
            if c.snooze_needs_you {
                "lit until you look at it ∙ enter makes it quiet".into()
            } else {
                "it just reappears ∙ enter lights it until you look".into()
            }
        },
        avail: always,
        key: "",
    },
    // Which day "next week" starts on: the snooze ring's last rung.
    MenuItem {
        verb: Verb::WeekStart,
        label: |c| format!("Week starts on {}", or(c.week_start_word, "Monday")),
        detail: |c| {
            format!(
                "z's last rung: next {} 9:00 ∙ enter cycles the day",
                or(c.week_start_word, "Monday")
            )
        },
        avail: always,
        key: "",
    },
    // The merge train (2026-09-04): the one standing consent for mesimon to
    // prompt an agent with no per-press gesture, which is why it is a
    // preference and off by default.
    MenuItem {
        verb: Verb::MergeTrain,
        label: |c| if c.merge_train { "Auto merge: on".into() } else { "Auto merge: off".into() },
        detail: |c| {
            if c.merge_train && c.merge_train_armed {
                "merges quiet REVIEW branches, asks idle agents to rebase ∙ armed while this board is open".into()
            } else if c.merge_train {
                "merges quiet REVIEW branches, asks idle agents to rebase ∙ arming…".into()
            } else {
                "mesimon merges and asks to rebase for you while the board is quiet ∙ enter turns it on".into()
            }
        },
        avail: always,
        key: "",
    },
    MenuItem {
        verb: Verb::MergeTrainNotice,
        label: |c| {
            if c.merge_train_notice {
                "Auto merge tells the agent after a merge".into()
            } else {
                "Auto merge stays silent after a merge".into()
            }
        },
        detail: |c| {
            if c.merge_train_notice {
                "pastes the merged notice into the agent ∙ starts a turn ∙ enter keeps it quiet"
                    .into()
            } else {
                "the card's ⎇✓ says it ∙ enter tells the agent too".into()
            }
        },
        avail: |c| c.merge_train,
        key: "",
    },
    // The agent tool surface (T-217). The one row here that is NOT a
    // preference: it is board state in `columns.toml`, per repo, because
    // "may agents on this board see their ticket" is a property of the
    // board — so it acts over the wire and reads back off the snapshot.
    MenuItem {
        verb: Verb::AgentProvider,
        label: |c| format!("Provider: {}", c.agent_provider.label()),
        detail: |c| {
            format!("new sessions only ∙ enter selects {}", c.agent_provider.next().label())
        },
        avail: always,
        key: "",
    },
    MenuItem {
        verb: Verb::McpTools,
        label: |c| {
            if c.mcp_tools {
                "Agent tools: on".into()
            } else {
                "Agent tools: off".into()
            }
        },
        // Off says the consequence, not the mechanism, and names the one
        // thing that surprises: a pane's argv is fixed at exec, so the
        // sessions already running keep whatever they were born with.
        detail: |c| {
            if c.mcp_tools {
                "the seven board tools every spawn carries ∙ enter takes them away".into()
            } else {
                "spawns carry no tools ∙ a session cannot see its ticket ∙ live panes keep theirs"
                    .into()
            }
        },
        avail: always,
        key: "",
    },
    // The agent brief (T-224): one sentence in the system prompt of every
    // claude mesimon starts here, telling it to read its ticket first. Board
    // state like the row above, off by default, and it needs the tools it
    // names — so the row says so rather than offering a switch that does
    // nothing.
    MenuItem {
        verb: Verb::SystemPrompt,
        label: |c| {
            if c.system_prompt {
                "Agent brief: on".into()
            } else {
                "Agent brief: off".into()
            }
        },
        detail: |c| {
            if !c.mcp_tools {
                "needs the agent tools on ∙ the brief names get_ticket".into()
            } else if c.system_prompt {
                "in every spawn's system prompt ∙ enter turns it off".into()
            } else {
                "one line: read the ticket first ∙ enter shows it first".into()
            }
        },
        avail: always,
        key: "",
    },
    // The default column (T-279): where an agent's `create_ticket` lands a
    // card that names no column. Board state like the two rows above, so it
    // acts over the wire and the row relabels itself off the snapshot. The
    // label names the column the daemon WILL use — the first column until
    // one is chosen — so an unset default never reads as "none".
    MenuItem {
        verb: Verb::DefaultColumn,
        label: |c| format!("Default column: {}", c.default_column),
        detail: |c| {
            if !c.mcp_tools {
                "needs the agent tools on ∙ create_ticket is one of them".into()
            } else {
                "where an agent's create_ticket lands unplaced ∙ enter cycles".into()
            }
        },
        // A board with no columns has nowhere to land: no row rather than a
        // label with nothing after the colon. (Delete refuses the last
        // column, so this is a snapshot that has not arrived yet.)
        avail: |c| !c.default_column.is_empty(),
        key: "",
    },
];

/// The notifications list, one level under Settings (T-282).
///
/// Five rows, each one idea, in the order somebody would meet them: whether
/// at all, then which moments, then what they sound like, then the one
/// exception about the banner. Rows two to five are gated on the first, so
/// the list is a single row until it is turned on — the `MergeTrainNotice`
/// shape, which is what keeps a preference list from offering settings for
/// a thing that is off.
pub static NOTIFY_ITEMS: &[MenuItem] = &[
    MenuItem {
        verb: Verb::NotifyToggle,
        label: |c| {
            if c.notify {
                "Notifications: on".into()
            } else {
                "Notifications: off".into()
            }
        },
        // Off names the reach, because a channel out of the board is a thing
        // to consent to and not a thing to discover afterwards.
        detail: |c| {
            if c.notify {
                "an OS banner and a sound ∙ only while this board is open ∙ enter turns them off"
                    .into()
            } else {
                "an OS banner and a sound when an agent needs you ∙ enter turns them on".into()
            }
        },
        avail: always,
        key: "",
    },
    MenuItem {
        verb: Verb::NotifyDone,
        label: |c| {
            if c.notify_done {
                "Also when a turn finishes".into()
            } else {
                "Only when an agent needs you".into()
            }
        },
        detail: |c| {
            if c.notify_done {
                "the card's ✔ ∙ enter keeps it to the blocked ones".into()
            } else {
                "a blocked agent only ∙ enter says a landed turn too".into()
            }
        },
        avail: |c| c.notify,
        key: "",
    },
    // What a banner actually SAYS (T-292), before what it sounds like: the
    // ticket's title and the agent's own line are the reason the channel was
    // worth opening, and a banner lands on a lock screen other people see.
    // The ticket is never withheld — that half is the feature.
    MenuItem {
        verb: Verb::NotifyWords,
        label: |c| {
            if c.notify_words {
                "The agent's words: quoted".into()
            } else {
                "The agent's words: withheld".into()
            }
        },
        detail: |c| {
            if c.notify_words {
                "its last line, on the banner ∙ enter names only the ticket".into()
            } else {
                "the ticket and the reason word only ∙ enter quotes it".into()
            }
        },
        avail: |c| c.notify,
        key: "",
    },
    MenuItem {
        verb: Verb::NotifySoundNeedsYou,
        label: |c| {
            format!("Sound when an agent needs you: {}", or(c.notify_sound_needs_you, "Glass"))
        },
        detail: |_| "enter cycles the ring and plays what it names".into(),
        avail: |c| c.notify,
        key: "",
    },
    MenuItem {
        verb: Verb::NotifySoundDone,
        label: |c| format!("Sound when a turn finishes: {}", or(c.notify_sound_done, "Tink")),
        detail: |_| "a quieter one, so the two are told apart without looking".into(),
        avail: |c| c.notify && c.notify_done,
        key: "",
    },
    MenuItem {
        verb: Verb::NotifyFocused,
        label: |c| {
            if c.notify_focused {
                "Banner while the board is focused: shown".into()
            } else {
                "Banner while the board is focused: quiet".into()
            }
        },
        detail: |c| {
            if c.notify_focused {
                "a banner over the board that already says it ∙ enter quiets it".into()
            } else {
                "the card already says so ∙ the sound plays either way ∙ enter shows it".into()
            }
        },
        avail: |c| c.notify,
        key: "",
    },
    // The same idea one level finer (T-292), and the one row that governs
    // the sound as well: on the board a chime still says "go look", and
    // inside the agent's own pane there is nowhere left to go.
    MenuItem {
        verb: Verb::NotifyInPane,
        label: |c| {
            if c.notify_in_pane {
                "Inside the agent's own pane: said anyway".into()
            } else {
                "Inside the agent's own pane: silent".into()
            }
        },
        detail: |c| {
            if c.notify_in_pane {
                "said about the very pane you are attached to ∙ enter quiets it".into()
            } else {
                "its own pane already showed you ∙ sound too ∙ enter says it anyway".into()
            }
        },
        avail: |c| c.notify,
        key: "",
    },
];

/// `3 agents`, `1 agent` — a count and its noun. Every suggestion carries a
/// number, and `1 tickets` in the header would be the first thing seen.
use crate::text::plural;

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
        // Under the two above and over the two below: a one-time setup nudge
        // is never more urgent than a running system's news, and it is worth
        // more than a tidy-up. It has no key of its own — the dialog is the
        // whole of it, and a dialog is not something to hang a letter off.
        verb: Verb::BriefOffer,
        // Says what taking it GETS you. "claude.md misses the ticket line"
        // shipped first and was cut (author: "doesn't indicate well"): it
        // named a file the reader has no reason to care about yet, spent its
        // width on mesimon's own jargon — "the ticket line" means nothing
        // until you have seen it — and "misses" reads as longing as easily as
        // absence. The file is named on the menu row this points at, and the
        // dialog shows it in full; the chip's one job is to be worth opening.
        headline: |_| "tell agents to read the ticket".into(),
        key: "",
    },
    Suggestion {
        verb: Verb::SleepAllDone,
        headline: |c| format!("sleep {}", plural(c.bulk_sleep, "agent")),
        key: "X",
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

/// The settings rows that apply right now — all of them, today, but the
/// filter is the menu's so a conditional preference costs nothing later.
pub fn settings_items(ctx: &Ctx) -> Vec<&'static MenuItem> {
    let verbs: &[Verb] = match ctx.settings_section {
        SettingsSection::Root => {
            &[Verb::SettingsAppearance, Verb::SettingsBehaviour, Verb::SettingsAgents]
        }
        SettingsSection::Appearance => &[Verb::ThemePick, Verb::Notifications, Verb::StatusLine],
        SettingsSection::Behaviour => &[
            Verb::MergeTrain,
            Verb::MergeTrainNotice,
            Verb::SnoozeQuiet,
            Verb::WeekStart,
            Verb::DefaultColumn,
        ],
        SettingsSection::Agents => &[Verb::AgentProvider, Verb::SystemPrompt, Verb::McpTools],
    };
    verbs
        .iter()
        .filter_map(|v| SETTINGS_ITEMS.iter().find(|m| m.verb == *v && (m.avail)(ctx)))
        .collect()
}

/// The notifications list's rows that apply right now (T-282).
pub fn notify_items(ctx: &Ctx) -> Vec<&'static MenuItem> {
    NOTIFY_ITEMS.iter().filter(|m| (m.avail)(ctx)).collect()
}

fn yes_no(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}

fn on_off(b: bool) -> &'static str {
    if b {
        "on"
    } else {
        "off"
    }
}

/// The column settings dialog's rows (T-117): one column's every setting,
/// every automation the daemon runs on a ticket there among them — this
/// list is what "no magic" means. `MenuItem`s, so `ui/menu.rs` draws them
/// and the menu laws read them; the column is mirrored onto `Ctx`'s `col_*`
/// fields. A new column (`O`) has only its Name row until it exists.
static COLUMN_ITEMS: &[MenuItem] = &[
    MenuItem {
        verb: Verb::ColumnName,
        label: |c| {
            if c.col_new || c.col_name.is_empty() {
                "Name".into()
            } else {
                format!("Name: {}", c.col_name)
            }
        },
        detail: |c| {
            if c.col_new {
                "type it ∙ enter adds the column after the cursor's".into()
            } else {
                "enter edits it in place ∙ every ticket in it follows".into()
            }
        },
        avail: |c| c.col_new || c.col_naming,
        key: "",
    },
    MenuItem {
        verb: Verb::ColumnCollapse,
        label: |c| format!("Collapsed: {}", yes_no(c.col_collapsed)),
        detail: |_| "a one-cell spine until the cursor enters it".into(),
        avail: |c| !c.col_new,
        key: "",
    },
    MenuItem {
        verb: Verb::SortColumn,
        label: |c| format!("Sort now: {}", or(c.col_sort_word, "newest first")),
        detail: |_| "h l pick the order ∙ enter sorts the cards once".into(),
        avail: |c| !c.col_new,
        key: "",
    },
    MenuItem {
        verb: Verb::ColumnWorkspace,
        label: |c| {
            format!("Workspace for new tickets: {}", or(c.col_workspace_word, "board default"))
        },
        detail: |_| "the composer starts here ∙ shift+tab still changes it".into(),
        avail: |c| !c.col_new && !c.multi_repo,
        key: "",
    },
    MenuItem {
        verb: Verb::ColumnAgentBehaviour,
        label: |_| "Agent behaviour".into(),
        detail: |_| "mode, tools, creation and turn transitions".into(),
        avail: |c| !c.col_new,
        key: "",
    },
    MenuItem {
        verb: Verb::ColumnClaudeMode,
        label: |c| {
            if c.col_claude_mode_word.is_empty() || c.col_claude_mode_word == "inherit" {
                format!("Mode: inherit ({})", or(&c.col_inherit_mode, "unset"))
            } else {
                format!("Mode: {}", c.col_claude_mode_word)
            }
        },
        detail: |_| "--permission-mode for a claude started here ∙ a wake picks a change up".into(),
        avail: |c| !c.col_new && c.agent_provider == AgentProvider::ClaudeCode,
        key: "",
    },
    MenuItem {
        verb: Verb::ColumnCodexSandbox,
        label: |c| format!("Codex sandbox: {}", or(c.col_codex_sandbox_word, "inherit")),
        detail: |_| {
            "native sandbox policy ∙ inherit keeps Codex configuration ∙ applies on launch/wake"
                .into()
        },
        avail: |c| !c.col_new && c.agent_provider == AgentProvider::Codex,
        key: "",
    },
    MenuItem {
        verb: Verb::ColumnCodexApproval,
        label: |c| format!("Codex approvals: {}", or(c.col_codex_approval_word, "inherit")),
        detail: |_| {
            "native approval policy ∙ never refuses requests requiring approval ∙ applies on launch/wake".into()
        },
        avail: |c| !c.col_new && c.agent_provider == AgentProvider::Codex,
        key: "",
    },
    MenuItem {
        verb: Verb::ColumnTools,
        label: |c| format!("Agent tools: {}", or(c.col_tools_word, "full")),
        detail: |c| {
            if !c.mcp_tools {
                "agent tools are off for this board (Settings)".into()
            } else {
                match c.col_tools_word {
                    "read only" => {
                        "get_ticket, list_board, read_note ∙ enforced on every call".into()
                    }
                    "notes + tags" => {
                        "read, plus write_note and tag_ticket ∙ no move, no create".into()
                    }
                    "off" => "no tools at all ∙ an agent here cannot see its ticket".into(),
                    _ => "every tool: move_ticket and create_ticket too".into(),
                }
            }
        },
        avail: |c| !c.col_new,
        key: "",
    },
    MenuItem {
        verb: Verb::ColumnAutoRun,
        label: |c| format!("Start agent on creation: {}", on_off(c.col_auto_run)),
        detail: |c| {
            format!(
                "a ticket you create here gets {} on its brief, submitted",
                agent_word(c.agent_provider)
            )
        },
        avail: |c| !c.col_new,
        key: "",
    },
    MenuItem {
        verb: Verb::ColumnOnWorking,
        label: |c| {
            if c.col_on_working.is_empty() {
                "When agent starts working: stay".into()
            } else {
                format!("When agent starts working: move to {}", c.col_on_working)
            }
        },
        detail: |_| "enter cycles the other columns, then stay".into(),
        avail: |c| !c.col_new,
        key: "",
    },
    MenuItem {
        verb: Verb::ColumnOnDone,
        label: |c| {
            if c.col_on_done.is_empty() {
                "When agent ends a turn: stay".into()
            } else {
                format!("When agent ends a turn: move to {}", c.col_on_done)
            }
        },
        detail: |_| "enter cycles the other columns, then stay".into(),
        avail: |c| !c.col_new,
        key: "",
    },
    MenuItem {
        verb: Verb::ColumnRequiresMerge,
        label: |c| format!("Entry needs a merged branch: {}", yes_no(c.col_requires_merge)),
        detail: |_| "a card with unmerged work is refused at the door".into(),
        avail: |c| !c.col_new,
        key: "",
    },
    MenuItem {
        verb: Verb::ColumnReclaim,
        label: |c| format!("Offer: {}", or(c.col_offers_word, "off")),
        detail: |_| "enter cycles off, sleep, archive, sleep + archive".into(),
        avail: |c| !c.col_new,
        key: "",
    },
    MenuItem {
        verb: Verb::ColumnTrain,
        label: |c| {
            format!(
                "Auto merge: {}",
                match c.col_train_word {
                    "auto-merge" => "on",
                    "rebase asks" => "rebase only",
                    _ => "off",
                }
            )
        },
        detail: |c| {
            if !c.merge_train {
                "needs auto merge on (Settings → Behaviour)".into()
            } else {
                match c.col_train_word {
                    "auto-merge" => {
                        "ff-merges finished branches here, asks idle agents to rebase".into()
                    }
                    "rebase asks" => "asks an idle agent here to rebase when the base moves".into(),
                    _ => "auto merge does not act here".into(),
                }
            }
        },
        avail: |c| !c.col_new,
        key: "",
    },
];

/// The column dialog's rows that apply right now.
pub fn column_items(ctx: &Ctx) -> Vec<&'static MenuItem> {
    COLUMN_ITEMS
        .iter()
        .filter(|m| {
            let agent = matches!(
                m.verb,
                Verb::ColumnClaudeMode
                    | Verb::ColumnCodexSandbox
                    | Verb::ColumnCodexApproval
                    | Verb::ColumnTools
                    | Verb::ColumnAutoRun
                    | Verb::ColumnOnWorking
                    | Verb::ColumnOnDone
            );
            (m.avail)(ctx)
                && if ctx.col_new || ctx.col_naming {
                    m.verb == Verb::ColumnName
                } else {
                    agent == ctx.column_agents
                }
        })
        .collect()
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

/// The links dialog (T-256): the archived list's shapes, plus `c copy`, plus
/// `^k` as a second spelling of `Back` so the key that opened it closes it
/// (the drawer's `e`, the archived list's `V`). `Act` does not mutate:
/// nothing the daemon owns changes when a link opens.
/// The column settings dialog (T-117): the Settings list's shapes on one
/// column — `jk` selects, Enter does the row (a toggle advances, the sort
/// runs, the delete arms then sends), Esc goes back — plus `h`/`l` on the
/// `Sort now` row only, where they step the order the next Enter will use.
/// The order is local to the dialog and costs no wire, and the two keys are
/// gated on the row so they are inert and unhinted everywhere else.
static COLUMN: &[Binding] = &[
    Binding {
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
        keys: &[Key::Char('h'), Key::Left, Key::Char('l'), Key::Right],
        verb: Verb::CursorLeft,
        show: "h l",
        hint: |_| "other order",
        avail: |c| c.col_on_sort,
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
            if c.col_on_sort {
                "sort now"
            } else if c.col_delete_armed {
                "delete it"
            } else {
                "choose"
            }
        },
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
        hint: |_| "back",
        avail: always,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 250,
    },
];

static LINKS: &[Binding] = &[
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
        keys: &[Key::Char('c')],
        verb: Verb::LinkCopy,
        show: "c",
        hint: |_| "copy",
        avail: always,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: false,
        prio: 30,
    },
    Binding {
        keys: &[Key::Char('q'), Key::Esc, Key::Ctrl('k')],
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

/// The board's own top row (T-305), reached by `k` off a column header. It is
/// a cursor position, not a screen: nothing is drawn over the board, the
/// cursor column keeps its painted band, and `j` walks straight back into it.
///
/// One section is focusable — the checkout's git clause (`chrome::git_clause`)
/// — so Enter IS the board's `v`, and the clause stops spelling that key
/// beside the count it reads: a section the cursor can stand on says what
/// Enter does in the footer, which is where the hint belongs once it is one
/// press away. `h`/`l` are unbound until a second section earns them, and so
/// is `k`: there is nothing above the top row.
static HEADER: &[Binding] = &[
    Binding {
        keys: &[Key::Char('j'), Key::Down],
        verb: Verb::CursorDown,
        show: "j",
        hint: |_| "back to the board",
        avail: always,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 10,
    },
    Binding {
        // The board's `v`, on the section that draws the count it opens.
        // Gated on the sample for the same reason `v` is: with no repository
        // under the board there is no clause to stand on.
        keys: &[Key::Enter],
        verb: Verb::Act,
        show: "enter",
        hint: |_| "diff",
        avail: |c| c.git_repo,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 20,
    },
    Binding {
        keys: &[Key::Char('q'), Key::Esc],
        verb: Verb::Back,
        show: "esc",
        hint: |_| "back",
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
        hint: |c| {
            if c.prompting {
                if c.ask_queued {
                    "queue"
                } else {
                    "send"
                }
            } else {
                "save"
            }
        },
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
        hint: |c| {
            if c.prompting {
                ""
            } else {
                agent_hint(c, "save + ask claude", "save + ask codex")
            }
        },
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
        // The composer grows: `Tab` opens the full editor with the title
        // carried over and the cursor in the description. Composer-only —
        // a rename has no description and a prompt is not a ticket. Behind
        // `^t` (25) on purpose: that key was moved ahead of `shift+tab` to
        // survive the 120-column cut, and a longer item ahead of it would
        // push it off again.
        keys: &[Key::Tab],
        verb: Verb::Describe,
        show: "tab",
        hint: |_| "describe",
        avail: |c| c.composing,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: false,
        prio: 28,
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
        // Set at creation because the choice locks the moment a session or
        // a worktree exists — this is the first place it is open, and since
        // T-309 the board and the ticket page keep offering it for as long
        // as it stays open. In the ASK field the same key cycles the
        // delivery instead — `now` / `queued` — on the row under the field
        // (2026-09-04); one binding, because a key is bound once per scope,
        // and one gesture: shift+tab is "the other way" for whatever the
        // field is about.
        hint: |c| if c.prompting { "now / queued" } else { "shared checkout / own worktree" },
        avail: |c| (c.composing && !c.multi_repo) || (c.prompting && c.ask_queueable),
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

/// The note editor: a text barrier like `Input`, with the composer's keys
/// alive only while it is composing a ticket. `Tab` is deliberately unbound
/// (a tab is not a body character; `sanitize_note` turns one into a space),
/// and `{ }` are text here — paging is `pgup`/`pgdn`.
static EDITOR: &[Binding] = &[
    Binding {
        // Save, and leave the dialog — either way (2026-09-04, user request:
        // "^s should just save, and exit the composer to go back to the
        // small composer" … "either way ^s saves and exits the dialog").
        // Composing, the description is KEPT and the dialog folds back into
        // the one-line composer it grew out of — the draft is the
        // composer's, and Enter or Shift+Enter there mints it, description
        // and all. On a note the body is written and the editor closes; a
        // clean one just closes. It stayed open before, with a second press
        // telling claude — that second press is `^S` now, beside it. Always
        // live: there is always something to keep or a dialog to leave.
        // `^s` is a legacy-floor atom; raw mode clears IXON, so the terminal
        // does not eat it as flow control.
        keys: &[Key::Ctrl('s')],
        verb: Verb::EditorSave,
        show: "^s",
        hint: |_| "save",
        avail: |c| c.editing,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 10,
    },
    Binding {
        // Shift on the save axis: `^s` saves and leaves, `^S` saves, leaves
        // AND puts the words in front of claude. Composing, that is the
        // one-line composer's Shift+Enter in the bigger room — mint, and
        // claude on the ticket with the title submitted. On a ticket that
        // exists it depends on who is there (2026-09-04, "if no claude
        // session in ticket, treat like new"): a claude with a pane is TOLD
        // the note changed (`NoteToAgent`, the old second press of `^s`),
        // an empty seat gets a claude started on the title exactly as a new
        // ticket would, and a Sleeping claude — which holds the seat and
        // has no pane to type at — leaves the key inert and unhinted, as the
        // board's Shift+Enter is there (`c` wakes it). Only where the
        // terminal can tell ctrl+shift+s from ctrl+s: on the legacy floor
        // the press ARRIVES as `^s`, which saves and leaves, and the finger
        // finds Shift+Enter on the small composer or the board. Same clause
        // as Shift+Enter's, and it buys the same one sentence.
        keys: &[Key::Ctrl('S')],
        verb: Verb::EditorSaveStart,
        show: "^S",
        // On a note the key follows who is on the ticket — the editor's
        // ticket IS the subject, so the board's own two facts say it: a
        // paned claude is told the note changed (`NoteToAgent`), a ticket
        // with NO claude gets one started on the title the way a composed
        // ticket would (2026-09-04, "if no claude session in ticket, treat
        // like new"), and a Sleeping one — live, no pane — leaves it inert.
        hint: |c| {
            if !c.editor_composing && c.ticket_promptable {
                agent_hint(c, "save + tell claude", "save + tell codex")
            } else {
                agent_hint(c, "save + ask claude", "save + ask codex")
            }
        },
        avail: |c| {
            c.editing
                && c.rich_keys
                && (c.editor_composing || c.ticket_promptable || !c.ticket_has_agent)
        },
        class: Class::Plain,
        group: Group::Sessions,
        mutates: true,
        prio: 12,
    },
    Binding {
        // Two presses when there is something to lose; the first says so.
        // `^]` (both spellings) is the same verb (2026-09-03, user request):
        // it is the pop-outward key everywhere else — tmux's detach, the
        // ticket page's way back to the board — and a note is the deepest
        // the board goes. On a Hebrew layout ctrl+physical-`]` sends a bare
        // Esc, which is THIS binding's other key, so the mirrored bracket
        // lands on the same verb rather than on nothing. Shown as `esc`.
        keys: &[Key::Esc, Key::Ctrl(']'), Key::Ctrl('5')],
        verb: Verb::Cancel,
        show: "esc",
        hint: |c| if c.editor_dirty { "discard" } else { "close" },
        avail: |c| c.editing,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: false,
        prio: 20,
    },
    Binding {
        // The body in the user's own editor (T-181, 2026-09-03: "vim editing
        // in notes / description"). `$VISUAL`, then `$EDITOR`, then `vi` —
        // git's ladder — and the hint names the program that will open, so
        // the footer reads `^g nvim`. On a note the text that comes back is
        // SAVED at once: leaving the editor is the commit, as it is for a
        // commit message, and a `^s` owed afterwards would be the one step
        // every `$EDITOR` integration the user knows does not ask for.
        // Composing, the body comes back into the draft and `^s` still
        // mints. `^g` is Claude Code's own key for "open in external
        // editor", so the finger already knows it, and it is a ctrl-letter,
        // which is the only floor atom a text field cannot swallow. Inert
        // where no editor word is set (every test app).
        keys: &[Key::Ctrl('g')],
        verb: Verb::EditorExternal,
        show: "^g",
        hint: |c| c.editor_word,
        avail: |c| c.editing && !c.editor_word.is_empty(),
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 22,
    },
    Binding {
        keys: &[Key::Ctrl('t')],
        verb: Verb::TagPrefix,
        show: "^t",
        hint: |_| "tags",
        avail: |c| c.editing && c.editor_composing,
        class: Class::Plain,
        group: Group::Ticket,
        mutates: true,
        prio: 25,
    },
    Binding {
        // Composing, the pick rides with the draft; on a ticket that exists
        // it is set on the daemon at once (`SetWorkspace`), and only while
        // nothing has locked it — the choice closes the moment a session or
        // a worktree exists, the same rule `set_workspace` refuses by. The
        // same `workspace_open` now carries the key on the board and the
        // ticket page (T-309); here it stays SILENT in the edge (author
        // 2026-09-04): the key is spelled on the context row beside the
        // pick it cycles (`editor::context_line`), the way the one-line
        // composer's card spells it, so the bottom edge said it a second
        // time. Bound, not hinted — the `c` precedent on the ticket page.
        keys: &[Key::BackTab],
        verb: Verb::CycleWorkspace,
        show: "shift+tab",
        hint: |_| "",
        avail: |c| c.editing && !c.multi_repo && (c.editor_composing || c.workspace_open),
        class: Class::Plain,
        group: Group::Worktree,
        mutates: true,
        prio: 0,
    },
    Binding {
        // In the title, Enter is the way down to the body (a title is one
        // line); in the body it is a newline, and says nothing — every
        // editor the user has ever used already taught it.
        keys: &[Key::Enter],
        verb: Verb::EditorNewline,
        show: "enter",
        hint: |c| if c.editor_body { "" } else { "to body" },
        avail: |c| c.editing,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 35,
    },
    Binding {
        // Shift+Enter is the SAME newline (2026-09-03, user request). The
        // editor is a body, and every chat-shaped box the user types into —
        // claude's own included — has taught the finger that Shift+Enter
        // breaks a line; here it briefly meant agent_hint(c, "save + ask claude", "save + ask codex") while
        // composing, and the press that wanted a blank line minted a ticket
        // and started an agent. That sentence still has its board home a
        // press after `^s` (an empty seat's Shift+Enter asks the title), so
        // nothing is lost and the atom stays on one idea. Gated on
        // `rich_keys` like every ShiftEnter binding; on the legacy floor the
        // key ARRIVES as `Enter`, which is the same verb — the one place the
        // degradation is exact. Unhinted: `enter` beside it already says it.
        keys: &[Key::ShiftEnter],
        verb: Verb::EditorNewline,
        show: "shift+enter",
        hint: |_| "",
        avail: |c| c.editing && c.rich_keys,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 0,
    },
    Binding {
        keys: &[Key::Up, Key::Down],
        verb: Verb::EditorDown,
        show: "↑↓",
        hint: |_| "",
        avail: |c| c.editing,
        class: Class::Plain,
        group: Group::Navigate,
        mutates: false,
        prio: 0,
    },
    Binding {
        keys: &[Key::PageUp, Key::PageDown],
        verb: Verb::PageDown,
        show: "pgup pgdn",
        hint: |_| "",
        avail: |c| c.editing,
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
        avail: |c| c.editing,
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
        avail: |c| c.editing,
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
        avail: |c| c.editing,
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
        avail: |c| c.editing,
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
        avail: |c| c.editing,
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
        avail: |c| c.editing,
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
        avail: |c| c.editing,
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
        avail: |c| c.editing,
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
        Scope::SnoozeChord => SNOOZE,
        Scope::TagChord => TAG,
        Scope::Move => MOVE,
        Scope::Menu => MENU,
        Scope::Drawer => DRAWER,
        Scope::Archived => ARCHIVED,
        Scope::Theme => THEME,
        // The same three shapes: a list dialog's keys are the list's, and
        // which list Enter is choosing in is the mode's to know, not the
        // keymap's.
        Scope::Settings | Scope::Notifications => SETTINGS,
        Scope::Brief => BRIEF,
        Scope::Releases => RELEASES,
        Scope::Links => LINKS,
        Scope::ColumnSettings => COLUMN,
        Scope::Header => HEADER,
        Scope::Input => INPUT,
        Scope::Editor => EDITOR,
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
        (Verb::TagColor, Key::BackTab) => Verb::TagColorBack,
        (Verb::ScrollDown, Key::Char('k') | Key::Up) => Verb::ScrollUp,
        (Verb::EditorDown, Key::Up) => Verb::EditorUp,
        // The displayed paging pair uses half pages; physical page keys
        // retain full pages, including inside the editor (T-318).
        (Verb::PageDown, Key::Char('}')) => Verb::HalfPageDown,
        (Verb::PageDown, Key::Char('{')) => Verb::HalfPageUp,
        (Verb::PageDown, Key::PageUp) => Verb::PageUp,
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
            if b.prio > 0
                && (b.avail)(ctx)
                && !(b.hint)(ctx).is_empty()
                && (b.verb != Verb::AddColumn || ctx.col_header)
            {
                out.push(b);
            }
        }
    }
    out.sort_by_key(|b| b.prio);
    out
}

/// The footer's two clusters: what this screen does (left, in priority
/// order) and the app-level keys (right — `Group::App`: `esc menu`,
/// `? keys`). The right cluster is reserved first and never crowded out,
/// because it is how everything else is found; a dialog's frame carries its
/// own left cluster and leaves the right one to the footer under it.
pub fn footer_split(scope: Scope, ctx: &Ctx) -> (Vec<&'static Binding>, Vec<&'static Binding>) {
    footer_items(scope, ctx).into_iter().partition(|b| b.group != Group::App)
}

/// The footer line for this scope and state, filled to `width`: the left
/// cluster in priority order, skipping what does not fit, then the right
/// cluster. `? keys` closes the line wherever `?` resolves — and only there:
/// inside a text field or a chord tail `?` is text or a cancel, and a hint
/// that names a key must be a key that works.
pub fn footer(scope: Scope, ctx: &Ctx, width: usize) -> String {
    const SEP: &str = " ∙ ";
    let (own, app) = footer_split(scope, ctx);
    let word = |b: &Binding| format!("{} {}", b.show, (b.hint)(ctx));
    let right = app.iter().map(|b| word(b)).collect::<Vec<_>>().join(SEP);
    let reserved = if right.is_empty() { 0 } else { right.chars().count() + 3 };
    let budget = width.saturating_sub(reserved);
    let mut line = String::new();
    for b in own {
        let item = word(b);
        let add = if line.is_empty() { item.chars().count() } else { item.chars().count() + 3 };
        if line.chars().count() + add > budget {
            continue;
        }
        if !line.is_empty() {
            line.push_str(SEP);
        }
        line.push_str(&item);
    }
    match (line.is_empty(), right.is_empty()) {
        (true, _) => right,
        (false, true) => line,
        (false, false) => format!("{line}{SEP}{right}"),
    }
}

/// One binding's spelling and word, for the places that name a single key in
/// running text (an empty column's nudge, the ticket screen's branch line).
/// Returns `None` when the verb does not apply here — the caller then says
/// nothing rather than naming a key that would do nothing.
pub fn hint_for(scope: Scope, verb: Verb, ctx: &Ctx) -> Option<(&'static str, &'static str)> {
    binding_for(scope, verb, ctx).map(|b| (b.show, (b.hint)(ctx)))
}

/// The binding behind `hint_for`, for a surface that spells several keys
/// through one span builder (the ticket rail's trailer rows). Same rule:
/// `None` when the verb does not apply here or has nothing to say.
pub fn binding_for(scope: Scope, verb: Verb, ctx: &Ctx) -> Option<&'static Binding> {
    for s in chain(scope) {
        for b in bindings(s) {
            if b.verb == verb && (b.avail)(ctx) {
                if (b.hint)(ctx).is_empty() {
                    return None;
                }
                return Some(b);
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

    #[test]
    fn agent_hints_distinguish_project_default_from_existing_seat() {
        let mut ctx = Ctx {
            agent_provider: AgentProvider::Codex,
            ticket_agent_provider: AgentProvider::Codex,
            has_ticket: true,
            rich_keys: true,
            ..Ctx::default()
        };
        assert_eq!(hint_for(Scope::Board, Verb::Agent, &ctx), Some(("c", "start codex")));
        assert_eq!(
            hint_for(Scope::Board, Verb::Prompt, &ctx),
            Some(("shift+enter", "ask codex the title"))
        );
        ctx.ticket_has_agent = true;
        ctx.ticket_agent_provider = AgentProvider::ClaudeCode;
        assert_eq!(hint_for(Scope::Board, Verb::Agent, &ctx), Some(("c", "wake claude")));
        ctx.agent_provider = AgentProvider::ClaudeCode;
        ctx.ticket_agent_provider = AgentProvider::Codex;
        assert_eq!(hint_for(Scope::Ticket, Verb::Agent, &ctx), Some(("c", "wake codex")));
        ctx.ticket_promptable = true;
        assert_eq!(hint_for(Scope::Board, Verb::Prompt, &ctx), Some(("shift+enter", "ask codex")));
        ctx.settings_section = SettingsSection::Agents;
        let provider =
            settings_items(&ctx).into_iter().find(|r| r.verb == Verb::AgentProvider).unwrap();
        assert_eq!((provider.label)(&ctx), "Provider: Claude Code");
        assert!((provider.detail)(&ctx).contains("new sessions only"));
    }

    /// `Scope::ALL` is what every validator below walks, so a scope missing
    /// from it is validated by nothing. The match is exhaustive: adding a
    /// variant fails to compile here, and the length check then fails until
    /// `ALL` names it too.
    #[test]
    fn pending_move_only_binds_confirmation_and_cancellation() {
        let ctx = Ctx::default();
        for key in [Key::Char('>'), Key::Char('<')] {
            assert_eq!(resolve(Scope::Move, key, &ctx), Some(Verb::Grab));
        }
        for key in [
            Key::Enter,
            Key::Left,
            Key::Right,
            Key::Up,
            Key::Down,
            Key::Char('1'),
            Key::Char('j'),
            Key::Char('?'),
            Key::Ctrl('c'),
        ] {
            assert_eq!(resolve(Scope::Move, key, &ctx), None);
        }
        assert_eq!(hint_for(Scope::Move, Verb::Grab, &ctx), Some(("> <", "repeat to move")));
        assert_eq!(Scope::Move.parent(), None);
    }

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
                Scope::SnoozeChord => 7,
                Scope::TagChord => 8,
                Scope::Move => 9,
                Scope::Menu => 10,
                Scope::Drawer => 11,
                Scope::Archived => 12,
                Scope::Theme => 13,
                Scope::Settings => 14,
                Scope::Notifications => 15,
                Scope::Brief => 16,
                Scope::Releases => 17,
                Scope::Links => 18,
                Scope::ColumnSettings => 19,
                Scope::Header => 20,
                Scope::Input => 21,
                Scope::Editor => 22,
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
        ambiguous_atoms_are_inert_without_rich_keys(Key::ShiftEnter);
        let rich = Ctx { composing: true, rich_keys: true, ..Default::default() };
        // And it does resolve where the terminal can spell it.
        assert_eq!(resolve(Scope::Input, Key::ShiftEnter, &rich), Some(Verb::SaveStart));
        // A rename is not a composition: nothing to start.
        let renaming = Ctx { composing: false, rich_keys: true, ..Default::default() };
        assert_eq!(resolve(Scope::Input, Key::ShiftEnter, &renaming), None);
    }

    /// The same clause, for the same reason, on the editor's `^S`: a terminal
    /// without the kitty tier sends the bare 0x13 for ctrl+shift+s, which is
    /// `^s` — keep the draft, another verb — so the shifted atom must be
    /// unbound and unhinted there, and the press degrades to exactly the
    /// unshifted key.
    #[test]
    fn ctrl_shift_s_is_inert_without_rich_keys() {
        ambiguous_atoms_are_inert_without_rich_keys(Key::Ctrl('S'));
        let rich =
            Ctx { editing: true, editor_composing: true, rich_keys: true, ..Default::default() };
        assert_eq!(resolve(Scope::Editor, Key::Ctrl('S'), &rich), Some(Verb::EditorSaveStart));
        assert_eq!(
            hint_for(Scope::Editor, Verb::EditorSaveStart, &rich),
            Some(("^S", "save + ask claude"))
        );
        // On a note the key follows who is on the ticket: a paned claude is
        // told, an empty seat gets one started ("treat like new"), and a
        // Sleeping claude — neither paned nor an empty seat — leaves it
        // inert, the board's Shift+Enter's rule.
        let paned = Ctx {
            editing: true,
            ticket_has_agent: true,
            ticket_promptable: true,
            rich_keys: true,
            ..Default::default()
        };
        assert_eq!(resolve(Scope::Editor, Key::Ctrl('S'), &paned), Some(Verb::EditorSaveStart));
        assert_eq!(
            hint_for(Scope::Editor, Verb::EditorSaveStart, &paned),
            Some(("^S", "save + tell claude"))
        );
        let empty = Ctx { editing: true, rich_keys: true, ..Default::default() };
        assert_eq!(resolve(Scope::Editor, Key::Ctrl('S'), &empty), Some(Verb::EditorSaveStart));
        assert_eq!(
            hint_for(Scope::Editor, Verb::EditorSaveStart, &empty),
            Some(("^S", "save + ask claude"))
        );
        let asleep =
            Ctx { editing: true, ticket_has_agent: true, rich_keys: true, ..Default::default() };
        assert_eq!(resolve(Scope::Editor, Key::Ctrl('S'), &asleep), None);
        assert_eq!(hint_for(Scope::Editor, Verb::EditorSaveStart, &asleep), None);
        // And the case is the atom: `^s` and `^S` never collapse in the name.
        assert_ne!(Key::Ctrl('s').to_string(), Key::Ctrl('S').to_string());
    }

    /// `^K` is `^k` hardened — the first link with no dialog — and on the
    /// legacy floor the press ARRIVES as `^k`, which opens the dialog: the
    /// same clause as `^S`, the same safe degradation. Both screens.
    #[test]
    fn ctrl_shift_k_is_inert_without_rich_keys() {
        ambiguous_atoms_are_inert_without_rich_keys(Key::Ctrl('K'));
        for scope in [Scope::Board, Scope::Ticket] {
            let rich = Ctx {
                has_ticket: true,
                ticket_linkable: true,
                rich_keys: true,
                ..Default::default()
            };
            assert_eq!(resolve(scope, Key::Ctrl('K'), &rich), Some(Verb::LinkFirst), "{scope:?}");
            assert_eq!(resolve(scope, Key::Ctrl('k'), &rich), Some(Verb::Links), "{scope:?}");
            assert_eq!(hint_for(scope, Verb::LinkFirst, &rich), Some(("^K", "open first link")));
            let legacy = Ctx { rich_keys: false, ..rich.clone() };
            assert_eq!(resolve(scope, Key::Ctrl('k'), &legacy), Some(Verb::Links), "{scope:?}");
            // No note and no transcript, nothing to list: both spellings inert.
            let bare = Ctx { has_ticket: true, rich_keys: true, ..Default::default() };
            assert_eq!(resolve(scope, Key::Ctrl('k'), &bare), None, "{scope:?}");
            assert_eq!(resolve(scope, Key::Ctrl('K'), &bare), None, "{scope:?}");
        }
        // Inside the dialog the key that opened it closes it.
        assert_eq!(resolve(Scope::Links, Key::Ctrl('k'), &Ctx::default()), Some(Verb::Back));
        assert_eq!(resolve(Scope::Links, Key::Char('c'), &Ctx::default()), Some(Verb::LinkCopy));
    }

    /// The ambiguity clause itself: every binding on `atom` must be
    /// unavailable — and the atom unresolvable — wherever `rich_keys` is off.
    fn ambiguous_atoms_are_inert_without_rich_keys(atom: Key) {
        let legacy = Ctx {
            composing: true,
            editing: true,
            editor_composing: true,
            editor_dirty: true,
            rich_keys: false,
            ..Default::default()
        };
        let mut found = false;
        for scope in Scope::ALL {
            for b in bindings(scope) {
                if b.keys.contains(&atom) {
                    found = true;
                    assert!(
                        !(b.avail)(&legacy),
                        "{:?} in {scope:?} offers {atom:?} on the legacy floor",
                        b.verb
                    );
                }
            }
            assert_eq!(resolve(scope, atom, &legacy), None, "{scope:?}");
        }
        assert!(found, "no {atom:?} binding left — drop the atom too");
    }

    /// Shift+Enter says ONE sentence — "ask claude, and stay here" — and its
    /// three homes are that sentence at three moments: before the ticket
    /// exists (mint it, spawn, ask the title), on a ticket whose agent is
    /// already running (open a field), and inside that field (send). The atom
    /// is off the legacy floor, so what it buys has to be a single idea; this
    /// is the test that notices when a fourth home makes it two.
    /// The ask waits only where waiting means something: a shared-checkout
    /// ticket with an awake claude. Its toggle renames Enter, and a ticket
    /// with an ask already waiting is offered the field on it (2026-09-04).
    #[test]
    fn the_ask_field_queues_only_on_a_shared_checkout() {
        let prompting = Ctx { prompting: true, rich_keys: true, ..Ctx::default() };
        assert_eq!(resolve(Scope::Input, Key::BackTab, &prompting), None, "a worktree ticket");
        assert_eq!(hint_for(Scope::Input, Verb::CycleWorkspace, &prompting), None);
        let shared = Ctx { ask_queueable: true, ..prompting.clone() };
        assert_eq!(resolve(Scope::Input, Key::BackTab, &shared), Some(Verb::CycleWorkspace));
        assert_eq!(
            hint_for(Scope::Input, Verb::CycleWorkspace, &shared),
            Some(("shift+tab", "now / queued")),
            "T-294: the seat does not enter into it — a shared checkout is \
             the whole gate, since the delivery can wake or start a claude"
        );
        assert_eq!(hint_for(Scope::Input, Verb::Save, &shared), Some(("enter", "send")));
        let queued = Ctx { ask_queued: true, ..shared.clone() };
        assert_eq!(hint_for(Scope::Input, Verb::Save, &queued), Some(("enter", "queue")));
        let onboard = Ctx {
            has_ticket: true,
            ticket_has_agent: true,
            ticket_promptable: true,
            rich_keys: true,
            ticket_queued: true,
            ..Ctx::default()
        };
        assert_eq!(
            hint_for(Scope::Board, Verb::Prompt, &onboard),
            Some(("shift+enter", "edit the queued ask"))
        );
    }

    #[test]
    fn shift_enter_asks_claude_at_every_stage() {
        let composing = Ctx { composing: true, rich_keys: true, ..Default::default() };
        let onboard = Ctx {
            has_ticket: true,
            ticket_has_agent: true,
            ticket_promptable: true,
            rich_keys: true,
            ..Default::default()
        };
        let prompting = Ctx { prompting: true, rich_keys: true, ..Default::default() };
        assert_eq!(resolve(Scope::Input, Key::ShiftEnter, &composing), Some(Verb::SaveStart));
        assert_eq!(resolve(Scope::Board, Key::ShiftEnter, &onboard), Some(Verb::Prompt));
        assert_eq!(resolve(Scope::Input, Key::ShiftEnter, &prompting), Some(Verb::SaveStart));
        // The editor the composer grows into is NOT a fourth home: there
        // Shift+Enter is a newline, composing or noting alike (2026-09-03),
        // because a body is where the finger expects it to break a line.
        // "Ask claude" from the editor is `^S` (2026-09-04: the composer's
        // Shift+Enter on the save key's own shift, gated the same way), and
        // the same key tells a ticket's claude about a note.
        let composing_full =
            Ctx { editing: true, editor_composing: true, rich_keys: true, ..Default::default() };
        let noting = Ctx { editing: true, rich_keys: true, ..Default::default() };
        assert_eq!(
            resolve(Scope::Editor, Key::ShiftEnter, &composing_full),
            Some(Verb::EditorNewline)
        );
        assert_eq!(resolve(Scope::Editor, Key::ShiftEnter, &noting), Some(Verb::EditorNewline));
        assert_eq!(
            resolve(Scope::Editor, Key::ShiftEnter, &composing_full),
            resolve(Scope::Editor, Key::Enter, &composing_full)
        );
        assert_eq!(hint_for(Scope::Editor, Verb::SaveStart, &composing_full), None);
        // And it is unhinted: `enter` beside it already teaches the verb.
        assert_eq!(
            hint_for(
                Scope::Editor,
                Verb::EditorNewline,
                &Ctx { editor_body: true, ..noting.clone() }
            ),
            None
        );
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
        // pick, because there is no ticket being made. Shift+Tab is bound
        // there only where the ask can WAIT (a shared-checkout ticket with a
        // pane), and then it cycles the delivery, not the workspace.
        assert_eq!(resolve(Scope::Input, Key::BackTab, &prompting), None);
        assert_eq!(resolve(Scope::Input, Key::Ctrl('t'), &prompting), None);
        let queueable = Ctx { ask_queueable: true, ..prompting.clone() };
        assert_eq!(resolve(Scope::Input, Key::BackTab, &queueable), Some(Verb::CycleWorkspace));
        assert_eq!(
            hint_for(Scope::Input, Verb::CycleWorkspace, &queueable),
            Some(("shift+tab", "now / queued"))
        );
        assert_eq!(resolve(Scope::Input, Key::Ctrl('t'), &queueable), None);
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

    /// A prompt needs a box to land in, and a parked claude has none —
    /// `Sleeping` is live and paneless. Until 2026-09-04 that made the key
    /// inert there; now the press is the same verb and the hint names the
    /// wake the daemon does on the way. The distinction still lives in the
    /// hint, which is what tells the user a pane is about to be spent.
    #[test]
    fn prompting_a_parked_claude_says_it_wakes() {
        let rich = |promptable| Ctx {
            has_ticket: true,
            ticket_has_agent: true,
            ticket_promptable: promptable,
            rich_keys: true,
            ..Default::default()
        };
        assert_eq!(resolve(Scope::Board, Key::ShiftEnter, &rich(true)), Some(Verb::Prompt));
        assert_eq!(resolve(Scope::Board, Key::ShiftEnter, &rich(false)), Some(Verb::Prompt));
        assert_eq!(
            hint_for(Scope::Board, Verb::Prompt, &rich(true)),
            Some(("shift+enter", "ask claude"))
        );
        assert_eq!(
            hint_for(Scope::Board, Verb::Prompt, &rich(false)),
            Some(("shift+enter", "wake + ask claude"))
        );
        // And plain Enter is untouched either way: the two live side by side
        // in the footer, and only one of them spends the terminal.
        assert_eq!(resolve(Scope::Board, Key::Enter, &rich(true)), Some(Verb::Act));
    }

    /// An EMPTY seat is the composer's moment come round again: the ticket
    /// exists but no claude does, so Shift+Enter starts one on the title —
    /// same verb, and the hint says which sentence it is about to say. A
    /// parked claude is not an empty seat — the key wakes it and asks rather
    /// than starting a second — an empty column has no title to ask, and the
    /// legacy floor still gets nothing.
    #[test]
    fn shift_enter_on_an_empty_seat_starts_claude_on_the_title() {
        let empty = Ctx { has_ticket: true, rich_keys: true, ..Default::default() };
        assert_eq!(resolve(Scope::Board, Key::ShiftEnter, &empty), Some(Verb::Prompt));
        assert_eq!(
            hint_for(Scope::Board, Verb::Prompt, &empty),
            Some(("shift+enter", "ask claude the title"))
        );
        let parked = Ctx { ticket_has_agent: true, ..empty.clone() };
        assert_eq!(
            hint_for(Scope::Board, Verb::Prompt, &parked),
            Some(("shift+enter", "wake + ask claude"))
        );
        // T-294: with another claude working in the same checkout the same
        // press opens the field instead, so the start can wait its turn.
        let busy = Ctx { checkout_busy: true, ..empty.clone() };
        assert_eq!(resolve(Scope::Board, Key::ShiftEnter, &busy), Some(Verb::Prompt));
        assert_eq!(
            hint_for(Scope::Board, Verb::Prompt, &busy),
            Some(("shift+enter", "start claude"))
        );
        // A seat that is taken says what it always said: the busy checkout
        // changes which DEFAULT the field opens at, never the word.
        let busy_parked = Ctx { checkout_busy: true, ..parked.clone() };
        assert_eq!(
            hint_for(Scope::Board, Verb::Prompt, &busy_parked),
            Some(("shift+enter", "wake + ask claude"))
        );
        let no_card = Ctx { has_ticket: false, ..empty.clone() };
        assert_eq!(resolve(Scope::Board, Key::ShiftEnter, &no_card), None);
        let legacy = Ctx { rich_keys: false, ..empty };
        assert_eq!(resolve(Scope::Board, Key::ShiftEnter, &legacy), None);
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
    const OFF_FLOOR: &[Key] = &[
        Key::ShiftEnter,
        // ctrl+shift+s: Shift+Enter's clause, `ctrl_shift_s_is_inert_without_rich_keys`.
        Key::Ctrl('S'),
        // ctrl+shift+k: the same clause, `ctrl_shift_k_is_inert_without_rich_keys`.
        Key::Ctrl('K'),
        Key::AltLeft,
        Key::AltRight,
        Key::AltUp,
        Key::AltDown,
    ];

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
    /// FOOTER teaches the nudge and `> <` fell to the overlay. For three days
    /// the footer spelled it `option+hjkl`, so a terminal that ate the
    /// modifier read a move key that did nothing and found the working one
    /// only in `?`. Since 2026-09-04 `HJKL` sits in the nudge's own key list
    /// — the picker's arrangement, brought to the board — and the footer
    /// names THAT, so the hinted half of the clause is bought back: the key
    /// the footer teaches is one every terminal can send.
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

        // The board's. The capability it accelerates is on the floor twice
        // over — `> <` aiming, `HJKL` in the accelerator's own key list — and
        // the footer teaches the shifted spelling; `> <` keeps the overlay.
        let ctx =
            Ctx { has_ticket: true, multi_column: true, can_nudge: true, ..Default::default() };
        assert_eq!(resolve(Scope::Board, Key::Char('>'), &ctx), Some(Verb::Grab));
        assert_eq!(hint_for(Scope::Board, Verb::Grab, &ctx), Some(("> <", "move card, twice")));
        assert_eq!(resolve(Scope::Board, Key::AltLeft, &ctx), Some(Verb::Nudge));
        for k in [Key::Char('H'), Key::Char('J'), Key::Char('K'), Key::Char('L')] {
            assert_eq!(resolve(Scope::Board, k, &ctx), Some(Verb::Nudge), "{k:?}");
        }
        assert_eq!(hint_for(Scope::Board, Verb::Nudge, &ctx), Some(("HJKL", "move card")));
        let shown: Vec<&str> = footer_items(Scope::Board, &ctx).iter().map(|b| b.show).collect();
        assert!(shown.contains(&"HJKL"), "the footer must name the move: {shown:?}");
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
        assert_eq!(resolve(Scope::Board, Key::Char('H'), &alone), None);
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
        let editing =
            Ctx { editing: true, editor_composing: true, tags_exist: true, ..Default::default() };
        assert_eq!(resolve(Scope::Editor, Key::Char('1'), &editing), None);
        // The shifted spellings reach none of it, and `!` still opens a shell.
        for sym in "!@#$%^&*()".chars() {
            assert_ne!(resolve(Scope::Board, Key::Char(sym), &ctx), Some(Verb::TagCycle), "{sym}");
            assert_ne!(resolve(Scope::Ticket, Key::Char(sym), &ctx), Some(Verb::TagCycle), "{sym}");
        }
        // `!` is the project's terminal on every screen (T-273; before it, the
        // diff's shell-in-worktree): the atom Shift+1 really produces was
        // spoken for before this feature existed.
        assert_eq!(resolve(Scope::Board, Key::Char('!'), &ctx), Some(Verb::Terminal));
        assert_eq!(resolve(Scope::Ticket, Key::Char('!'), &ctx), Some(Verb::Terminal));
        assert_eq!(resolve(Scope::Diff, Key::Char('!'), &ctx), Some(Verb::Terminal));
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
            (Key::BackTab, Verb::TagColorBack),
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
        for k in [Key::Tab, Key::BackTab, Key::Char('r'), Key::Char('d')] {
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
            Key::BackTab,
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
        // The editor is the composer's second room: same key while composing,
        // and none while a note is being written on a ticket that exists.
        let editing = Ctx { editing: true, editor_composing: true, ..Default::default() };
        assert_eq!(resolve(Scope::Editor, Key::Ctrl('t'), &editing), Some(Verb::TagPrefix));
        let noting = Ctx { editing: true, ..Default::default() };
        assert_eq!(resolve(Scope::Editor, Key::Ctrl('t'), &noting), None);
        assert_eq!(resolve(Scope::Editor, Key::BackTab, &noting), None);
        // …until the ticket's workspace is open to change: then Shift+Tab
        // is the composer's pick, a press late — and it closes again the
        // moment work starts on the ticket.
        let open = Ctx { editing: true, workspace_open: true, ..Default::default() };
        assert_eq!(resolve(Scope::Editor, Key::BackTab, &open), Some(Verb::CycleWorkspace));
        // Bound but unhinted: the key is spelled on the context row beside
        // the pick, never in the dialog's edge.
        assert!(hint_for(Scope::Editor, Verb::CycleWorkspace, &open).is_none());
        assert_eq!(resolve(Scope::Editor, Key::Ctrl('t'), &open), None, "tags stay the picker's");
    }

    /// T-309: the workspace pick is the composer's, but the choice it makes
    /// stays open for as long as the daemon would take it — no session, no
    /// worktree binding — so the same key answers on the board and on the
    /// ticket page, where a ticket is set up before anyone starts on it.
    /// One verb, one atom, three screens.
    #[test]
    fn the_workspace_choice_is_open_until_work_starts() {
        let open = Ctx { has_ticket: true, workspace_open: true, ..Default::default() };
        for scope in [Scope::Board, Scope::Ticket] {
            assert_eq!(
                resolve(scope, Key::BackTab, &open),
                Some(Verb::CycleWorkspace),
                "{scope:?}"
            );
            // The word is the DESTINATION, `t`'s idiom: the card and the
            // state row say where the ticket stands, the hint says where the
            // press would leave it.
            assert_eq!(
                hint_for(scope, Verb::CycleWorkspace, &open),
                Some(("shift+tab", "own worktree")),
                "{scope:?}"
            );
            let already = Ctx { workspace_worktree: true, ..open.clone() };
            assert_eq!(
                hint_for(scope, Verb::CycleWorkspace, &already),
                Some(("shift+tab", "shared checkout")),
                "{scope:?}"
            );
            // Locked — a session or a worktree exists, which is exactly what
            // `set_workspace` refuses by. UNHINTED, but still live: `m`'s
            // shape, so the press can name the ticket and say why instead of
            // reading as a broken key (user, 2026-09-07).
            let shut = Ctx { workspace_open: false, ..open.clone() };
            assert_eq!(resolve(scope, Key::BackTab, &shut), Some(Verb::CycleWorkspace));
            assert_eq!(hint_for(scope, Verb::CycleWorkspace, &shut), None, "{scope:?}");
            // An archived ticket offers nothing at all (T-300's shape).
            let gone = Ctx { ticket_archived: true, ..open.clone() };
            assert_eq!(resolve(scope, Key::BackTab, &gone), None, "{scope:?}");
            // And no card at all is no subject.
            assert_eq!(resolve(scope, Key::BackTab, &Ctx::default()), None, "{scope:?}");
        }
        // The ticket page hints it in the footer; the board keeps it to the
        // overlay, where `n` and `s` already are — the board's row is at its
        // width at 120 columns and the card's own mark says which way the
        // ticket is set.
        assert!(
            footer_items(Scope::Ticket, &open).iter().any(|b| b.show == "shift+tab"),
            "{:?}",
            footer_items(Scope::Ticket, &open).iter().map(|b| b.show).collect::<Vec<_>>()
        );
        assert!(!footer_items(Scope::Board, &open).iter().any(|b| b.show == "shift+tab"));
        assert!(overlay(Scope::Board, &open)
            .into_iter()
            .flat_map(|(_, rows)| rows)
            .any(|(k, h)| k == "shift+tab" && h == "own worktree"));
    }

    /// `Tab` grows the one-line composer into the editor, and only there: a
    /// rename has no description and a prompt is not a ticket. On the board
    /// the same key on a card opens the same dialog on the ticket's own
    /// description (T-163), and nowhere else is `tab` a verb: the ticket
    /// page and the diff keep it inert, and `needs you` no longer has it.
    #[test]
    fn tab_opens_the_editor_from_the_composer_and_the_card() {
        let composing = Ctx { composing: true, ..Default::default() };
        assert_eq!(resolve(Scope::Input, Key::Tab, &composing), Some(Verb::Describe));
        assert_eq!(hint_for(Scope::Input, Verb::Describe, &composing), Some(("tab", "describe")));
        let prompting = Ctx { prompting: true, ..Default::default() };
        assert_eq!(resolve(Scope::Input, Key::Tab, &prompting), None);
        assert_eq!(resolve(Scope::Input, Key::Tab, &Ctx::default()), None);
        let card = Ctx { has_ticket: true, ..Default::default() };
        assert_eq!(resolve(Scope::Board, Key::Tab, &card), Some(Verb::Describe));
        assert_eq!(hint_for(Scope::Board, Verb::Describe, &card), Some(("tab", "describe")));
        assert_eq!(resolve(Scope::Board, Key::Tab, &Ctx::default()), None, "no card, no tab");
        for scope in [Scope::Ticket, Scope::Diff, Scope::Global] {
            assert_eq!(resolve(scope, Key::Tab, &card), None, "{scope:?}");
        }
        // Its shift is another verb entirely — the workspace pick, on the two
        // screens that have a card (`the_workspace_choice_is_open_until_work_starts`).
        for scope in [Scope::Diff, Scope::Global] {
            assert_eq!(resolve(scope, Key::BackTab, &card), None, "{scope:?}");
        }
        // Inside the editor Tab is nothing: not a character (a note holds no
        // tabs) and not a verb.
        let editing = Ctx { editing: true, editor_composing: true, ..Default::default() };
        assert_eq!(resolve(Scope::Editor, Key::Tab, &editing), None);
    }

    /// The editor owns its keys the way the composer does: the screen keys
    /// under it are letters to type, and its own verbs answer.
    #[test]
    fn the_editor_is_a_barrier() {
        let ctx = Ctx { editing: true, editor_dirty: true, ..Default::default() };
        for k in [Key::Char('q'), Key::Char('?'), Key::Char('u'), Key::Char('j'), Key::Char('n')] {
            assert_eq!(resolve(Scope::Editor, k, &ctx), None, "{k:?} must type, not act");
        }
        assert_eq!(resolve(Scope::Editor, Key::Ctrl('s'), &ctx), Some(Verb::EditorSave));
        // Composing, `^s` keeps the draft and folds back into the small
        // composer (2026-09-04), and it is live before the body is touched —
        // the carried title is not "dirty" against the editor's baseline, and
        // a `Tab` then `^s` with nothing typed used to press nothing.
        let composing = Ctx { editing: true, editor_composing: true, ..Default::default() };
        assert!(!composing.editor_dirty);
        assert_eq!(resolve(Scope::Editor, Key::Ctrl('s'), &composing), Some(Verb::EditorSave));
        assert_eq!(hint_for(Scope::Editor, Verb::EditorSave, &composing), Some(("^s", "save")));
        // A note says the same word, dirty or clean: it saves and leaves.
        let clean_note = Ctx { editing: true, ..Default::default() };
        assert_eq!(hint_for(Scope::Editor, Verb::EditorSave, &clean_note), Some(("^s", "save")));
        let dirty_note = Ctx { editing: true, editor_dirty: true, ..Default::default() };
        assert_eq!(hint_for(Scope::Editor, Verb::EditorSave, &dirty_note), Some(("^s", "save")));
        assert_eq!(resolve(Scope::Editor, Key::Esc, &ctx), Some(Verb::Cancel));
        // `^]` closes too, in both of its spellings.
        assert_eq!(resolve(Scope::Editor, Key::Ctrl(']'), &ctx), Some(Verb::Cancel));
        assert_eq!(resolve(Scope::Editor, Key::Ctrl('5'), &ctx), Some(Verb::Cancel));
        assert_eq!(resolve(Scope::Editor, Key::Enter, &ctx), Some(Verb::EditorNewline));
        assert_eq!(resolve(Scope::Editor, Key::Up, &ctx), Some(Verb::EditorUp));
        assert_eq!(resolve(Scope::Editor, Key::Down, &ctx), Some(Verb::EditorDown));
        assert_eq!(resolve(Scope::Editor, Key::PageUp, &ctx), Some(Verb::PageUp));
        assert_eq!(resolve(Scope::Editor, Key::PageDown, &ctx), Some(Verb::PageDown));
        assert_eq!(resolve(Scope::Editor, Key::Ctrl('u'), &ctx), Some(Verb::EditKillToStart));
        // And with the editor down, its scope answers nothing at all.
        assert_eq!(resolve(Scope::Editor, Key::Ctrl('s'), &Ctx::default()), None);
    }

    #[test]
    fn braces_use_half_pages_and_physical_page_keys_use_full_pages() {
        let ctx = Ctx { preview_scrolls: true, ..Default::default() };
        for scope in [Scope::Ticket, Scope::Diff, Scope::Releases] {
            for (key, verb) in [
                (Key::Char('}'), Verb::HalfPageDown),
                (Key::Char('{'), Verb::HalfPageUp),
                (Key::PageDown, Verb::PageDown),
                (Key::PageUp, Verb::PageUp),
            ] {
                assert_eq!(resolve(scope, key, &ctx), Some(verb), "{scope:?} {key:?}");
                if scope == Scope::Ticket {
                    assert_eq!(resolve(scope, key, &Ctx::default()), None);
                }
            }
        }
        let editing = Ctx { editing: true, ..Default::default() };
        for key in [Key::Char('{'), Key::Char('}')] {
            assert_eq!(resolve(Scope::Editor, key, &editing), None, "braces remain text");
        }
    }

    /// `^g` opens the user's own editor on the body, and the footer names
    /// which one. With no editor word (every test app) the key is inert and
    /// unhinted, so a developer's `$EDITOR` never reaches a golden; and it
    /// is the editor's key, not the one-line composer's.
    #[test]
    fn ctrl_g_hands_the_body_to_the_users_editor() {
        let wired = Ctx { editing: true, editor_word: "nvim", ..Default::default() };
        assert_eq!(resolve(Scope::Editor, Key::Ctrl('g'), &wired), Some(Verb::EditorExternal));
        assert_eq!(hint_for(Scope::Editor, Verb::EditorExternal, &wired), Some(("^g", "nvim")));
        let composing = Ctx { editor_composing: true, ..wired };
        assert_eq!(
            resolve(Scope::Editor, Key::Ctrl('g'), &composing),
            Some(Verb::EditorExternal),
            "the composer's description takes the same road"
        );
        let unwired = Ctx { editing: true, ..Default::default() };
        assert_eq!(resolve(Scope::Editor, Key::Ctrl('g'), &unwired), None);
        assert_eq!(hint_for(Scope::Editor, Verb::EditorExternal, &unwired), None);
        let one_line = Ctx { composing: true, editor_word: "nvim", ..Default::default() };
        assert_eq!(resolve(Scope::Input, Key::Ctrl('g'), &one_line), None);
        assert_eq!(resolve(Scope::Editor, Key::Ctrl('g'), &Ctx::default()), None);
    }

    /// `^s` says `save` and is always live in the editor: it saves what
    /// there is and leaves (2026-09-04 — it used to be inert on a clean note
    /// and say `tell claude` on a saved one; telling is `^S` now).
    #[test]
    fn ctrl_s_says_save_and_always_leaves() {
        let dirty = Ctx { editing: true, editor_dirty: true, ..Default::default() };
        assert_eq!(hint_for(Scope::Editor, Verb::EditorSave, &dirty), Some(("^s", "save")));
        let clean = Ctx { editing: true, ..Default::default() };
        assert_eq!(resolve(Scope::Editor, Key::Ctrl('s'), &clean), Some(Verb::EditorSave));
        assert_eq!(hint_for(Scope::Editor, Verb::EditorSave, &clean), Some(("^s", "save")));
        assert_eq!(resolve(Scope::Board, Key::Ctrl('s'), &Ctx::default()), None);
        // Enter's word is the title's: it is the way down. In the body it is
        // silent — a newline needs no teaching.
        assert_eq!(
            hint_for(Scope::Editor, Verb::EditorNewline, &clean),
            Some(("enter", "to body"))
        );
        let body = Ctx { editing: true, editor_body: true, ..Default::default() };
        assert_eq!(hint_for(Scope::Editor, Verb::EditorNewline, &body), None);
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
        for absent in ["> <", "r", "d", "a", "c", "s", "x", "n", "N", "enter"] {
            assert!(!shown.contains(&absent), "{absent} hinted with no ticket selected: {shown:?}");
        }
        assert!(shown.contains(&"o"), "open-ticket must always be offered: {shown:?}");
        // The quit left the FOOTER, not the board: it still resolves and the
        // overlay still names it. The menu's `esc` is back, in the footer's
        // RIGHT cluster beside `? keys` (T-158) — the app keys, apart from
        // the card's, so the board's one door is named without competing
        // with the selection.
        let named = |k: &str| {
            overlay(Scope::Board, &ctx).iter().any(|(_, ks)| ks.iter().any(|(s, _)| *s == k))
        };
        assert!(!shown.contains(&"q"), "q is overlay-only: {shown:?}");
        assert!(named("q"), "the overlay is the complete answer and must name q");
        let (own, app) = footer_split(Scope::Board, &ctx);
        let right: Vec<&str> = app.iter().map(|b| b.show).collect();
        assert_eq!(right, vec!["esc", "?"], "the right cluster is the app's keys, in order");
        assert!(own.iter().all(|b| b.group != Group::App), "no app key in the left cluster");
        assert_eq!(resolve(Scope::Board, Key::Esc, &ctx), Some(Verb::Menu));
        assert_eq!(resolve(Scope::Board, Key::Char('q'), &ctx), Some(Verb::Quit));
        assert_eq!(resolve(Scope::Board, Key::Ctrl('c'), &ctx), Some(Verb::Quit));
    }

    /// And the other half: an unavailable key is inert, not merely unhinted.
    #[test]
    fn unavailable_keys_do_nothing() {
        let empty = Ctx { multi_column: true, ..Default::default() };
        for k in [
            Key::Char('>'),
            Key::Char('r'),
            Key::Char('d'),
            Key::Char('c'),
            Key::Char('n'),
            Key::Char('z'),
        ] {
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
            ticket_rail_rows: 2,
            sel_session: true,
            multi_column: true,
            has_worktree: true,
            merge_actionable: true,
            merge_word: "merge",
            git_repo: true,
            // Both `s` keys are gated now (T-300); the shape they share
            // across the two screens is what this test is about.
            ticket_shells: true,
            ..Default::default()
        };
        for (key, verb) in [
            (Key::Char('c'), Verb::Agent),
            (Key::Char('s'), Verb::Shell),
            (Key::Char('r'), Verb::Rename),
            (Key::Char('d'), Verb::DeletePrefix),
            (Key::Char('a'), Verb::ArchivePrefix),
            (Key::Char('z'), Verb::SnoozePrefix),
            (Key::Char('x'), Verb::Sleep),
            (Key::Char('n'), Verb::NoteEdit),
            // One verb, two subjects: the board's `v` diffs the checkout, the
            // ticket page's diffs that ticket's branch (T-221). What differs
            // is what the screen is about, never what the key means.
            (Key::Char('v'), Verb::OpenDiff),
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

    /// The board's `v` needs a repository under it, and says so by being
    /// inert — the "a key that is hinted works" clause. The ticket page's `v`
    /// keeps its own gate, which is a worktree, not a checkout.
    #[test]
    fn v_needs_a_repository_and_is_the_same_verb_on_both_screens() {
        let bare = Ctx { has_ticket: true, ..Default::default() };
        assert_eq!(resolve(Scope::Board, Key::Char('v'), &bare), None, "no repo, no diff");
        assert_eq!(hint_for(Scope::Board, Verb::OpenDiff, &bare), None);

        let repo = Ctx { git_repo: true, ..Default::default() };
        assert_eq!(resolve(Scope::Board, Key::Char('v'), &repo), Some(Verb::OpenDiff));
        assert_eq!(hint_for(Scope::Board, Verb::OpenDiff, &repo), Some(("v", "diff")));
        // A card under the cursor changes nothing: the board's `v` is the
        // repository's, whatever the cursor is over.
        let carded = Ctx { git_repo: true, has_ticket: true, has_worktree: true, ..repo.clone() };
        assert_eq!(resolve(Scope::Board, Key::Char('v'), &carded), Some(Verb::OpenDiff));

        // The ticket page reads the worktree, not the checkout.
        let wt = Ctx { has_worktree: true, ..Default::default() };
        assert_eq!(resolve(Scope::Ticket, Key::Char('v'), &wt), Some(Verb::OpenDiff));
        assert_eq!(resolve(Scope::Ticket, Key::Char('v'), &repo), None, "a repo is not a branch");

        // It is a view, not a branch action: the board has no other Worktree
        // binding and must not grow a one-row BRANCH section in `?`.
        let b = bindings(Scope::Board).iter().find(|b| b.verb == Verb::OpenDiff).unwrap();
        assert_eq!(b.group, Group::View);
        assert!(!b.mutates, "a diff reads and nothing else");
    }

    /// `x` is one verb wearing two words (and silence on a sleeper), and the
    /// word has to be the one the row under the cursor can actually do. The dismiss case is the one
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
        // A sleeper's `x` wakes but is not hinted: `enter` on the row says
        // "wake" already.
        assert_eq!(hint_for(Scope::Ticket, Verb::Sleep, &asleep), None);
        assert_eq!(hint_for(Scope::Ticket, Verb::Act, &asleep), Some(("enter", "wake")));
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
        // On a column header the same chord deletes the COLUMN (T-117), and
        // says so; the discard form needs a worktree, which a header lacks.
        let header = Ctx { col_header: true, ..Default::default() };
        assert_eq!(resolve(Scope::Board, Key::Char('d'), &header), Some(Verb::DeletePrefix));
        assert_eq!(
            hint_for(Scope::Board, Verb::DeletePrefix, &header),
            Some(("d", "delete column"))
        );
        assert_eq!(
            hint_for(Scope::DeleteChord, Verb::Delete, &header),
            Some(("d", "delete column"))
        );
        assert_eq!(resolve(Scope::DeleteChord, Key::Char('D'), &header), None);
    }

    /// The column dialog's `Sort now` row owns `h` and `l` (T-117): they step
    /// the order and Enter runs it, and on every other row the two keys are
    /// inert and unhinted, so the list's vertical shape stays a list.
    #[test]
    fn the_sort_row_owns_h_and_l() {
        let sort = Ctx { col_on_sort: true, ..Default::default() };
        let other = Ctx::default();
        assert_eq!(resolve(Scope::ColumnSettings, Key::Char('l'), &sort), Some(Verb::CursorRight));
        assert_eq!(resolve(Scope::ColumnSettings, Key::Char('h'), &sort), Some(Verb::CursorLeft));
        assert_eq!(resolve(Scope::ColumnSettings, Key::Right, &sort), Some(Verb::CursorRight));
        assert_eq!(resolve(Scope::ColumnSettings, Key::Char('l'), &other), None);
        assert_eq!(hint_for(Scope::ColumnSettings, Verb::CursorLeft, &other), None);
        assert_eq!(hint_for(Scope::ColumnSettings, Verb::Act, &sort), Some(("enter", "sort now")));
        assert_eq!(hint_for(Scope::ColumnSettings, Verb::Act, &other), Some(("enter", "choose")));
        let armed = Ctx { col_delete_armed: true, ..Default::default() };
        assert_eq!(
            hint_for(Scope::ColumnSettings, Verb::Act, &armed),
            Some(("enter", "delete it"))
        );
        // A new column has its Name row and nothing else until it exists.
        let fresh = Ctx { col_new: true, ..Default::default() };
        let rows: Vec<Verb> = column_items(&fresh).iter().map(|m| m.verb).collect();
        assert_eq!(rows, vec![Verb::ColumnName]);
        assert_eq!(column_items(&other).len(), 7);
        assert!(!column_items(&other)
            .iter()
            .any(|m| matches!(m.verb, Verb::ColumnName | Verb::DeleteColumn)));
        let agents = Ctx { column_agents: true, ..Default::default() };
        assert_eq!(
            column_items(&agents).iter().map(|m| m.verb).collect::<Vec<_>>(),
            [
                Verb::ColumnClaudeMode,
                Verb::ColumnTools,
                Verb::ColumnAutoRun,
                Verb::ColumnOnWorking,
                Verb::ColumnOnDone
            ]
        );
        assert_eq!(
            column_items(&Ctx { multi_repo: true, ..Default::default() }).len(),
            6,
            "a workspace board offers no worktree choice"
        );
    }

    /// A column header is a cursor position (T-117): over an empty column it
    /// adds exactly the four column verbs — settings, rename, move, delete —
    /// and nothing a ticket needs, so `x`, `a`, `z`, tags, `c`, `s` and the
    /// rest stay as inert as they are on an empty column.
    #[test]
    fn a_header_offers_exactly_the_column_verbs() {
        let header =
            Ctx { col_header: true, multi_column: true, can_nudge: true, ..Default::default() };
        let empty = Ctx { col_header: false, can_nudge: false, ..header.clone() };
        let mut added: Vec<Verb> = Vec::new();
        for b in bindings(Scope::Board) {
            if (b.avail)(&header) && !(b.avail)(&empty) {
                added.push(b.verb);
            }
        }
        added.sort_by_key(|v| format!("{v:?}"));
        assert_eq!(added, vec![Verb::Act, Verb::DeletePrefix, Verb::Nudge, Verb::Rename]);
        assert_eq!(hint_for(Scope::Board, Verb::Act, &header), Some(("enter", "column settings")));
        assert_eq!(hint_for(Scope::Board, Verb::Rename, &header), Some(("r", "rename column")));
        assert_eq!(hint_for(Scope::Board, Verb::Nudge, &header), Some(("HJKL", "move column")));
        let shown: Vec<&str> = footer_items(Scope::Board, &header).iter().map(|b| b.show).collect();
        for k in ["enter", "r", "HJKL", "o"] {
            assert!(shown.contains(&k), "{k} belongs on a header's footer: {shown:?}");
        }
        for k in ["x", "a", "z", "tab", "^t", "c", "s", "space", "> <", "shift+enter", "1-0"] {
            assert!(!shown.contains(&k), "{k} needs a ticket: {shown:?}");
        }
        // And on a header the ticket verbs are inert, not merely unhinted.
        for k in [Key::Char('x'), Key::Char('a'), Key::Char('z'), Key::Char('c'), Key::Char('s')] {
            assert_eq!(resolve(Scope::Board, k, &header), None, "{k:?}");
        }
    }

    /// The board's own top row (T-305) is a cursor position one step above a
    /// column header, and it owns three keys and no more: `j` back into the
    /// column, Enter on the one section that is focusable, Esc to pop. It
    /// binds nothing sideways and nothing upward while the git clause is the
    /// only section, and it is a board scope, so `?` and the app keys reach
    /// it while the board's own selection keys do not.
    #[test]
    fn the_top_row_owns_three_keys() {
        let repo = Ctx { git_repo: true, ..Default::default() };
        assert_eq!(resolve(Scope::Header, Key::Char('j'), &repo), Some(Verb::CursorDown));
        assert_eq!(resolve(Scope::Header, Key::Down, &repo), Some(Verb::CursorDown));
        assert_eq!(resolve(Scope::Header, Key::Enter, &repo), Some(Verb::Act));
        assert_eq!(hint_for(Scope::Header, Verb::Act, &repo), Some(("enter", "diff")));
        assert_eq!(resolve(Scope::Header, Key::Esc, &repo), Some(Verb::Back));
        assert_eq!(resolve(Scope::Header, Key::Char('?'), &repo), Some(Verb::Help));
        // One section: nothing walks sideways, and nothing is above the top.
        for k in [Key::Char('h'), Key::Char('l'), Key::Char('k'), Key::Up] {
            assert_eq!(resolve(Scope::Header, k, &repo), None, "{k:?}");
        }
        // The board's own keys are the board's; none of them reaches up here.
        let full = Ctx { has_ticket: true, multi_column: true, ..repo.clone() };
        for k in [Key::Char('o'), Key::Char('v'), Key::Char('r'), Key::Char('d'), Key::Char('x')] {
            assert_eq!(resolve(Scope::Header, k, &full), None, "{k:?}");
        }
        // Enter is gated the way the board's `v` is: no repository, no clause
        // to stand on, so nothing to read.
        let bare = Ctx::default();
        assert_eq!(resolve(Scope::Header, Key::Enter, &bare), None);
        assert_eq!(hint_for(Scope::Header, Verb::Act, &bare), None);
        let shown: Vec<&str> = footer_items(Scope::Header, &repo).iter().map(|b| b.show).collect();
        assert_eq!(shown, vec!["j", "enter", "esc", "?"]);
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

    /// A snooze is an archive with a deadline, and it is a chord the same
    /// way: `z` arms, `z` again walks the ring, Enter takes the pick, Esc or
    /// any stray key leaves. The prefix is inert on an archived ticket (`a`
    /// restores there) and on an empty column; the tail names the pick.
    #[test]
    fn snooze_is_a_chord() {
        let live = Ctx { has_ticket: true, ..Default::default() };
        for s in [Scope::Board, Scope::Ticket] {
            assert_eq!(resolve(s, Key::Char('z'), &live), Some(Verb::SnoozePrefix), "{s:?}");
        }
        let armed = Ctx { has_ticket: true, snooze_word: "tomorrow 9:00", ..Default::default() };
        assert_eq!(resolve(Scope::SnoozeChord, Key::Char('z'), &armed), Some(Verb::SnoozeNext));
        assert_eq!(resolve(Scope::SnoozeChord, Key::Enter, &armed), Some(Verb::SnoozeConfirm));
        assert_eq!(resolve(Scope::SnoozeChord, Key::Esc, &armed), Some(Verb::SnoozeCancel));
        assert_eq!(resolve(Scope::SnoozeChord, Key::Char('x'), &armed), None);
        assert_eq!(resolve(Scope::SnoozeChord, Key::Char('?'), &armed), None, "a barrier");
        assert_eq!(
            hint_for(Scope::SnoozeChord, Verb::SnoozeConfirm, &armed),
            Some(("enter", "snooze until tomorrow 9:00"))
        );
        let gone = Ctx { has_ticket: true, ticket_archived: true, ..Default::default() };
        assert_eq!(resolve(Scope::Board, Key::Char('z'), &gone), None);
        assert_eq!(resolve(Scope::Ticket, Key::Char('z'), &gone), None);
        assert_eq!(resolve(Scope::Board, Key::Char('z'), &Ctx::default()), None);
        // The chord's word is its own, so the footer says SNOOZE while armed.
        assert_eq!(Scope::SnoozeChord.word(), "SNOOZE");
        assert_eq!(Scope::SnoozeChord.parent(), None);
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
        for s in [
            Scope::Board,
            Scope::Ticket,
            Scope::Diff,
            Scope::Drawer,
            Scope::Menu,
            Scope::Theme,
            Scope::Releases,
        ] {
            assert_eq!(resolve(s, Key::Char('u'), &ctx), Some(Verb::Undo), "{s:?}");
        }
        let nothing = Ctx::default();
        assert_eq!(resolve(Scope::Board, Key::Char('u'), &nothing), None);
        // And it says which thing it would undo.
        let arch = Ctx { can_undo: true, undo_word: "undo archive", ..Default::default() };
        assert_eq!(hint_for(Scope::Board, Verb::Undo, &arch), Some(("u", "undo archive")));
    }

    /// A ticket's own shell is behind a gate (T-300, user: "keep the feature
    /// but gate it for now"). Both keys are inert AND unhinted while it is
    /// shut — the keymap's one invariant, applied to a feature that is still
    /// whole underneath — and the board's `s` goes with them: where a ticket
    /// may not grow a shell, no screen may start one. `!` never went behind
    /// it, because the project's terminal is a place to stand and not a
    /// session of the ticket.
    #[test]
    fn a_ticket_shell_is_behind_the_gate() {
        let off = Ctx { has_ticket: true, sel_session: true, ..Default::default() };
        assert_eq!(resolve(Scope::Ticket, Key::Char('s'), &off), None);
        assert_eq!(resolve(Scope::Ticket, Key::Char('S'), &off), None);
        assert_eq!(resolve(Scope::Board, Key::Char('s'), &off), None);
        assert_eq!(hint_for(Scope::Ticket, Verb::Shell, &off), None);
        assert_eq!(hint_for(Scope::Ticket, Verb::ShellNew, &off), None);
        assert_eq!(resolve(Scope::Ticket, Key::Char('!'), &off), Some(Verb::Terminal));
        let on = Ctx { ticket_shells: true, ..off };
        assert_eq!(resolve(Scope::Ticket, Key::Char('s'), &on), Some(Verb::Shell));
        assert_eq!(resolve(Scope::Ticket, Key::Char('S'), &on), Some(Verb::ShellNew));
        assert_eq!(resolve(Scope::Board, Key::Char('s'), &on), Some(Verb::Shell));
        assert_eq!(hint_for(Scope::Ticket, Verb::Shell, &on), Some(("s", "shell")));
    }

    /// What replaced the pair of spawn hints under an empty rail (T-300): a
    /// row, and Enter. One act, one spelling — so `c` on the ticket page has
    /// exactly one word left, and it is for the seat that is already taken.
    #[test]
    fn the_offer_is_a_row_and_the_key_that_said_it_stands_down() {
        let offered = Ctx { sel_new_agent: true, ticket_rail_rows: 1, ..Default::default() };
        assert_eq!(resolve(Scope::Ticket, Key::Enter, &offered), Some(Verb::Act));
        assert_eq!(hint_for(Scope::Ticket, Verb::Act, &offered), Some(("enter", "start claude")));
        assert_eq!(hint_for(Scope::Ticket, Verb::Agent, &offered), None, "no second spelling");
        // The one word `c` keeps: a parked claude holds the seat, so there is
        // no row to offer and the key is what wakes it.
        let parked = Ctx { ticket_has_agent: true, ..Default::default() };
        assert_eq!(hint_for(Scope::Ticket, Verb::Agent, &parked), Some(("c", "wake claude")));
        // A claude that is up says nothing here either: `enter` on its row does.
        let up = Ctx { ticket_promptable: true, ..parked };
        assert_eq!(hint_for(Scope::Ticket, Verb::Agent, &up), None);
        // And the rail walks on rows, not on sessions: one row is not a list,
        // two are — whether or not either is a session.
        assert_eq!(resolve(Scope::Ticket, Key::Char('j'), &offered), None);
        let with_note = Ctx { ticket_rail_rows: 2, ..offered };
        assert_eq!(resolve(Scope::Ticket, Key::Char('j'), &with_note), Some(Verb::CursorDown));
        assert_eq!(
            hint_for(Scope::Ticket, Verb::CursorDown, &with_note),
            Some(("jk", "select row"))
        );
        let seated = Ctx { ticket_has_sessions: true, ..with_note };
        assert_eq!(
            hint_for(Scope::Ticket, Verb::CursorDown, &seated),
            Some(("jk", "select session"))
        );
    }

    /// Shift hardens, forces or widens the same verb. It never switches
    /// verbs — which is why the bulk sleep is `X` (the selection's `x`,
    /// widened to the done column) and not `Z`, which sat beside `z` once
    /// that snoozed the ticket: two verbs on one letter (2026-09-04, user).
    #[test]
    fn shift_stays_on_one_axis() {
        let t = Ctx { sel_session: true, ticket_shells: true, ..Default::default() };
        assert_eq!(resolve(Scope::Ticket, Key::Char('c'), &t), Some(Verb::Agent));
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
        // And the board's, the same bargain on the card: `hjkl` steps the
        // cursor, `HJKL` steps it carrying the card.
        let board = Ctx { has_ticket: true, can_nudge: true, ..Default::default() };
        assert_eq!(resolve(Scope::Board, Key::Char('l'), &board), Some(Verb::CursorRight));
        assert_eq!(resolve(Scope::Board, Key::Char('L'), &board), Some(Verb::Nudge));
        // The old unrelated pairs are gone: no board key at all for the bulk
        // verbs, and none for the two lists.
        let full = Ctx {
            has_ticket: true,
            bulk_sleep: 3,
            bulk_archive: 3,
            has_archived: true,
            // `v` is live on the board now, so `V` has a lowercase twin to be
            // retired against rather than being absent from the scope.
            git_repo: true,
            ..Default::default()
        };
        assert_eq!(resolve(Scope::Board, Key::Char('v'), &full), Some(Verb::OpenDiff));
        for retired in [Key::Char('Z'), Key::Char('A'), Key::Char('V'), Key::Char('e')] {
            assert_eq!(resolve(Scope::Board, retired, &full), None, "{retired:?} is retired");
        }
        // The editor's save key, composing: `^s` keeps the draft, `^S` mints
        // it and starts claude — Enter / Shift+Enter's bargain on the save
        // key's own shift (2026-09-04).
        let composing_full =
            Ctx { editing: true, editor_composing: true, rich_keys: true, ..Default::default() };
        assert_eq!(resolve(Scope::Editor, Key::Ctrl('s'), &composing_full), Some(Verb::EditorSave));
        assert_eq!(
            resolve(Scope::Editor, Key::Ctrl('S'), &composing_full),
            Some(Verb::EditorSaveStart)
        );
        // `o` mints a card in the column, `O` mints a column after it (T-117)
        // — the same act one level wider, the way `x`/`X` widen the sleep.
        assert_eq!(resolve(Scope::Board, Key::Char('o'), &full), Some(Verb::OpenTicket));
        assert_eq!(resolve(Scope::Board, Key::Char('O'), &full), Some(Verb::AddColumn));
        // `p` opens the cursor card's reply, `P` every card's (T-237): the
        // same row, widened. The board's and nobody else's — the ticket page
        // draws no cards, and `P` is unbound there since the pin went.
        assert_eq!(resolve(Scope::Board, Key::Char('p'), &full), Some(Verb::Peek));
        assert_eq!(resolve(Scope::Board, Key::Char('P'), &full), Some(Verb::PeekAll));
        assert_eq!(resolve(Scope::Ticket, Key::Char('P'), &full), None);
        // `n` edits the note the cursor means, `N` forces a fresh one: same
        // target, harder — on both screens.
        for scope in [Scope::Board, Scope::Ticket] {
            assert_eq!(resolve(scope, Key::Char('n'), &full), Some(Verb::NoteEdit), "{scope:?}");
            assert_eq!(resolve(scope, Key::Char('N'), &full), Some(Verb::NoteNew), "{scope:?}");
        }
        // The bulk sleep is the exception, and it is one the header teaches:
        // `x`'s own shift, gated on the same predicate as the row and the
        // chip, and kept off the footer so the board still reads as the
        // selection's.
        let seated = Ctx { ticket_has_sessions: true, ..full.clone() };
        assert_eq!(resolve(Scope::Board, Key::Char('x'), &seated), Some(Verb::Sleep));
        assert_eq!(resolve(Scope::Board, Key::Char('X'), &full), Some(Verb::SleepAllDone));
        assert_eq!(resolve(Scope::Board, Key::Char('X'), &Ctx::default()), None);
        assert_eq!(resolve(Scope::Ticket, Key::Char('X'), &full), None, "board-only");
        assert!(
            !footer_items(Scope::Board, &full).iter().any(|b| b.show == "X"),
            "the header chip carries this offer; the footer stays out of it"
        );
        assert_eq!(
            hint_for(Scope::Board, Verb::SleepAllDone, &full),
            Some(("X", "sleep the finished agents"))
        );
        // And `z` is the card's own zzz (T-74), with nothing on its shift:
        // snoozing a ticket and sleeping a column's agents are two verbs,
        // and shift may not carry a second one.
        assert_eq!(resolve(Scope::Board, Key::Char('z'), &full), Some(Verb::SnoozePrefix));
        assert!(!footer_items(Scope::Board, &full).iter().any(|b| b.show == "z"), "overlay-only");
    }

    /// Board-wide actions live in the menu, and the menu is one Esc away.
    #[test]
    fn menu_holds_the_board_wide_actions() {
        let ctx = Ctx {
            bulk_sleep: 2,
            bulk_archive: 2,
            has_archived: true,
            update_ready: true,
            default_column: "TODO".into(),
            ..Default::default()
        };
        assert_eq!(resolve(Scope::Board, Key::Esc, &ctx), Some(Verb::Menu));
        let verbs: Vec<Verb> = menu_items(&ctx).iter().map(|m| m.verb).collect();
        for v in [
            Verb::ExternalDrawer,
            Verb::ArchivedList,
            Verb::SleepAllDone,
            Verb::ArchiveAllDone,
            Verb::Settings,
            Verb::ReleaseNotes,
            Verb::Quit,
        ] {
            assert!(verbs.contains(&v), "{v:?} missing from the menu: {verbs:?}");
        }
        // The preferences are one level down, behind the Settings row, and
        // not in the menu proper: a menu row is an action or a door.
        let prefs: Vec<Verb> = settings_items(&ctx).iter().map(|m| m.verb).collect();
        assert_eq!(
            prefs,
            [Verb::SettingsAppearance, Verb::SettingsBehaviour, Verb::SettingsAgents]
        );
        for (section, expected) in [
            (
                SettingsSection::Appearance,
                vec![Verb::ThemePick, Verb::Notifications, Verb::StatusLine],
            ),
            (
                SettingsSection::Behaviour,
                vec![Verb::MergeTrain, Verb::SnoozeQuiet, Verb::WeekStart, Verb::DefaultColumn],
            ),
            (
                SettingsSection::Agents,
                vec![Verb::AgentProvider, Verb::SystemPrompt, Verb::McpTools],
            ),
        ] {
            let c = Ctx { settings_section: section, ..ctx.clone() };
            assert_eq!(settings_items(&c).iter().map(|m| m.verb).collect::<Vec<_>>(), expected);
            assert!(!settings_items(&c).iter().any(|m| m.verb == Verb::Peek));
        }
        let on: Vec<Verb> = settings_items(&Ctx {
            settings_section: SettingsSection::Behaviour,
            merge_train: true,
            ..ctx.clone()
        })
        .iter()
        .map(|m| m.verb)
        .collect();
        assert_eq!(on[0..2], [Verb::MergeTrain, Verb::MergeTrainNotice]);
        for v in prefs {
            assert!(!verbs.contains(&v), "{v:?} is a preference and belongs in Settings");
        }
        assert_eq!(resolve(Scope::Settings, Key::Esc, &ctx), Some(Verb::Back));
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
        for m in menu_items(&ctx).into_iter().chain(settings_items(&ctx)) {
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
            assert!(
                !SETTINGS_ITEMS.iter().any(|m| m.verb == s.verb),
                "suggestion {:?} points one level down, where the chip cannot take it",
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
            default_column: "TODO".into(),
            ..Default::default()
        };
        for m in MENU_ITEMS.iter().chain(SETTINGS_ITEMS).chain(COLUMN_ITEMS) {
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

    #[test]
    fn menu_omits_fetch_and_actions_with_contextual_keys() {
        let tracking = Ctx {
            git_upstream: true,
            git_remote: "origin".into(),
            git_fetch_note: "2 to push ∙ never fetched".into(),
            ..Default::default()
        };
        for ctx in [Ctx::default(), tracking] {
            for verb in [Verb::GitFetch, Verb::Help, Verb::AddColumn, Verb::ColumnSettings] {
                assert!(!menu_items(&ctx).iter().any(|m| m.verb == verb));
                assert!(!settings_items(&ctx).iter().any(|m| m.verb == verb));
            }
            assert_eq!(resolve(Scope::Board, Key::Char('O'), &ctx), Some(Verb::AddColumn));
            assert_eq!(resolve(Scope::Board, Key::Char('?'), &ctx), Some(Verb::Help));
        }
    }

    #[test]
    fn new_column_is_hinted_only_on_column_headers() {
        for col_header in [false, true] {
            let ctx = Ctx { col_header, ..Default::default() };
            assert_eq!(
                footer_items(Scope::Board, &ctx).iter().any(|b| b.verb == Verb::AddColumn),
                col_header
            );
            assert_eq!(resolve(Scope::Board, Key::Char('O'), &ctx), Some(Verb::AddColumn));
        }
        let ctx = Ctx::default();
        for verb in [Verb::NextFile, Verb::PageDown, Verb::ScrollDown] {
            assert!(!footer_items(Scope::Diff, &ctx).iter().any(|b| b.verb == verb));
            assert!(binding_for(Scope::Diff, verb, &ctx).is_some());
        }
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
            if matches!(scope, Scope::Input | Scope::Editor) {
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
        for s in [
            Scope::Ticket,
            Scope::Diff,
            Scope::Drawer,
            Scope::Archived,
            Scope::Menu,
            Scope::Theme,
            Scope::Settings,
            Scope::Releases,
            Scope::Links,
            Scope::ColumnSettings,
            Scope::Header,
        ] {
            assert_eq!(resolve(s, Key::Char('q'), &ctx), Some(Verb::Back), "{s:?}");
            assert_eq!(resolve(s, Key::Esc, &ctx), Some(Verb::Back), "{s:?}");
        }
        assert_eq!(resolve(Scope::Move, Key::Char('q'), &ctx), Some(Verb::Cancel));
        for s in Scope::ALL {
            if matches!(
                s,
                Scope::Input
                    | Scope::Move
                    | Scope::Editor
                    | Scope::DiffView
                    | Scope::DeleteChord
                    | Scope::ArchiveChord
                    | Scope::SnoozeChord
                    | Scope::TagChord
            ) {
                continue; // chord tails, pending move and text fields own every key
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
            composing: true,
            editing: true,
            rich_keys: true,
            ..Default::default()
        };
        for width in [40usize, 60, 80, 100, 140, 200] {
            let line = footer(Scope::Board, &ctx, width);
            assert!(line.ends_with("? keys"), "width {width}: {line}");
            assert!(line.chars().count() <= width, "width {width} overflowed: {line}");
        }
        // And ONLY where `?` resolves (T-158): every scope that inherits
        // Global ends on the tail; a barrier — a text field, a chord tail —
        // never promises a key that would type a `?` or cancel the chord.
        for scope in Scope::ALL {
            let line = footer(scope, &ctx, 200);
            let works = resolve(scope, Key::Char('?'), &ctx) == Some(Verb::Help);
            assert_eq!(
                line.ends_with("? keys"),
                works,
                "{scope:?}: `? keys` promised iff `?` works — {line}"
            );
            assert!(line.chars().count() <= 200);
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
            ticket_hot: true,
            ..Default::default()
        };
        let rows: Vec<&str> =
            overlay(Scope::Board, &ctx).into_iter().flat_map(|(_, r)| r).map(|(k, _)| k).collect();
        for present in ["space", "g", "G", "?", "^L", "esc"] {
            assert!(rows.contains(&present), "{present} missing from the overlay: {rows:?}");
        }
        // `space` speaks only while `enter` goes to the agent; on a cold
        // ticket it is silent (and still works — `resolve` says so).
        let cold = Ctx { ticket_hot: false, ..ctx.clone() };
        assert_eq!(
            hint_for(Scope::Board, Verb::TicketScreen, &ctx),
            Some(("space", "ticket page"))
        );
        assert_eq!(hint_for(Scope::Board, Verb::TicketScreen, &cold), None);
        assert_eq!(resolve(Scope::Board, Key::Space, &cold), Some(Verb::TicketScreen));
        let hot_footer: Vec<&str> =
            footer_items(Scope::Board, &ctx).iter().map(|b| b.show).collect();
        assert_eq!(&hot_footer[..2], &["enter", "space"], "{hot_footer:?}");
    }

    /// T-225: on a workspace board the composer offers no workspace choice
    /// — a worktree of the root holds none of the code — while the same key
    /// still flips an ask between `now` and `queued`.
    #[test]
    fn a_workspace_board_offers_no_worktree_choice() {
        let composing = Ctx { editing: true, composing: true, ..Default::default() };
        assert_eq!(resolve(Scope::Input, Key::BackTab, &composing), Some(Verb::CycleWorkspace));
        let on_workspace = Ctx { multi_repo: true, ..composing.clone() };
        assert_eq!(resolve(Scope::Input, Key::BackTab, &on_workspace), None);
        assert_eq!(hint_for(Scope::Input, Verb::CycleWorkspace, &on_workspace), None);
        let asking = Ctx {
            editing: true,
            prompting: true,
            ask_queueable: true,
            multi_repo: true,
            ..Default::default()
        };
        assert_eq!(resolve(Scope::Input, Key::BackTab, &asking), Some(Verb::CycleWorkspace));
        let editor =
            Ctx { editing: true, workspace_open: true, multi_repo: true, ..Default::default() };
        assert_eq!(resolve(Scope::Editor, Key::BackTab, &editor), None);
        // And the two screens the key reached in T-309 are held by the same
        // clause: a worktree of the root holds none of the code, wherever the
        // press comes from. Offered is HINTED there — the key stays live so
        // the press can say why (`m`'s shape), and `App::set_ticket_workspace`
        // refuses it before the wire.
        let screen =
            Ctx { has_ticket: true, workspace_open: true, multi_repo: true, ..Default::default() };
        for scope in [Scope::Board, Scope::Ticket] {
            assert_eq!(hint_for(scope, Verb::CycleWorkspace, &screen), None, "{scope:?}");
            assert!(!overlay(scope, &screen)
                .into_iter()
                .flat_map(|(_, rows)| rows)
                .any(|(k, _)| k == "shift+tab"));
        }
    }
}
