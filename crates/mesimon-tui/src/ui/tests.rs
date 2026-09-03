//! Rendering tests (06 §12 subset): golden layout snapshots over TestBackend,
//! the attn-provenance law, the SGR bans, and no-drawn-structure. Goldens live
//! in `testdata/golden/`; regenerate with `MESIMON_UPDATE_GOLDEN=1`.

use mesimon_core::board::{
    Board, Column, ExitReason, FailReason, Reason, SessionKind, SessionRecord, SessionState,
    StopReason, Ticket,
};
use ratatui::backend::TestBackend;
use ratatui::style::{Color, Modifier};
use ratatui::Terminal;

use crate::app::{App, Mode, Screen};
use crate::theme::{Flavor, Profile, Theme};

fn ulid_n(n: u128) -> ulid::Ulid {
    ulid::Ulid(n)
}

fn uuid_n(n: u128) -> uuid::Uuid {
    uuid::Uuid::from_u128(n)
}

fn session(n: u128, ticket: ulid::Ulid, kind: SessionKind, state: SessionState) -> SessionRecord {
    let mut s = session_record(n, ticket, kind, state);
    // Deterministic age slot: epoch-adjacent timestamps always render `>1y`.
    s.state_changed_at = Some(1);
    s
}

fn session_record(
    n: u128,
    ticket: ulid::Ulid,
    kind: SessionKind,
    state: SessionState,
) -> SessionRecord {
    SessionRecord::new(uuid_n(n), kind, ticket, vec!["claude".into()], "/repo".into(), state)
}

fn ticket(n: u128, key: &str, title: &str, column: &str, order: &str) -> Ticket {
    Ticket {
        id: ulid_n(n),
        short_key: key.into(),
        title: title.into(),
        column: column.into(),
        order: order.into(),
        // Epoch-adjacent so the identity line's age renders a stable `>1y`.
        created_at: "1970-01-01T00:00:00Z".into(),
        entered_at: None,
        workspace: None,
        tags: Vec::new(),
        notes: Vec::new(),
        archived: None,
    }
}

/// The default 4-column board (D33i) with one card per interesting state.
fn fixture(waiting: bool) -> Board {
    let mut b = Board::default();
    for (i, name) in ["todo", "in progress", "review", "done"].iter().enumerate() {
        b.columns.push(Column { name: (*name).into(), order: format!("{i}") });
    }
    b.tickets.push(ticket(1, "T-1", "Decay treatments", "todo", "a"));
    b.tickets.push(ticket(2, "T-2", "Keymap validator", "todo", "b"));
    b.tickets.push(ticket(3, "T-3", "Fix OSC-11 detection", "in progress", "a"));
    b.tickets.push(ticket(4, "T-4", "Adopt drawer import", "in progress", "b"));
    b.tickets.push(ticket(5, "T-5", "Grapheme truncation", "review", "a"));
    b.tickets.push(ticket(6, "T-6", "Flaky e2e on runner", "review", "b"));
    b.tickets.push(ticket(7, "T-7", "Painted accent bar", "done", "a"));

    let t3 = ulid_n(3);
    b.sessions.push(session(31, t3, SessionKind::Claude, SessionState::Running));
    // A shell as the daemon actually records one: `Running` for the whole
    // life of its pane, because pane death is the only shell event there is.
    b.sessions.push(session(32, t3, SessionKind::Bash, SessionState::Running));
    if waiting {
        let mut s = session(
            41,
            ulid_n(4),
            SessionKind::Claude,
            SessionState::RequiresAction { reason: Reason::Permission },
        );
        s.waiting_since = Some(1);
        s.detail = Some("Bash(rm -rf node_modules)".into());
        b.sessions.push(s);
    } else {
        b.sessions.push(session(41, ulid_n(4), SessionKind::Claude, SessionState::Running));
    }
    b.sessions.push(session(
        51,
        ulid_n(5),
        SessionKind::Claude,
        SessionState::Idle { stop_reason: StopReason::EndTurn },
    ));
    b.sessions.push(session(
        61,
        ulid_n(6),
        SessionKind::Claude,
        SessionState::Failed { reason: FailReason::Server },
    ));
    b.sessions.push(session(71, ulid_n(7), SessionKind::Bash, SessionState::Sleeping));
    b
}

fn press(app: &mut App, c: char) {
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    app.handle_key(KeyCode::Char(c), KeyModifiers::NONE).expect("key");
}

fn render(app: &App, w: u16, h: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("test terminal");
    terminal.draw(|f| super::draw(f, app)).expect("draw");
    let buffer = terminal.backend().buffer().clone();
    let mut lines = Vec::new();
    for y in 0..h {
        let mut line = String::new();
        for x in 0..w {
            line.push_str(buffer[(x, y)].symbol());
        }
        lines.push(line.trim_end().to_string());
    }
    lines
}

fn cells(app: &App, w: u16, h: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("test terminal");
    terminal.draw(|f| super::draw(f, app)).expect("draw");
    terminal.backend().buffer().clone()
}

fn golden(name: &str, lines: &[String]) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/golden");
    let path = dir.join(format!("{name}.txt"));
    let rendered = lines.join("\n") + "\n";
    if std::env::var_os("MESIMON_UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(&dir).expect("golden dir");
        std::fs::write(&path, &rendered).expect("write golden");
        return;
    }
    let want = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing golden {name}; run with MESIMON_UPDATE_GOLDEN=1"));
    assert_eq!(rendered, want, "golden {name} drifted; review + MESIMON_UPDATE_GOLDEN=1");
}

fn app_graphite(board: Board) -> App {
    App::for_test(board, Theme::new(Flavor::Graphite, Profile::TrueColor))
}

/// The fixture with T-7 (done, sleeping bash) archived — off the board, in
/// the dialog, badge on its ticket page. Stable stamp so ages render `>1y`.
fn fixture_archived() -> Board {
    let mut b = fixture(false);
    if let Some(t) = b.tickets.iter_mut().find(|t| t.id == ulid_n(7)) {
        t.archived = Some(mesimon_core::board::Archived { at: "@100".into(), by: "local".into() });
    }
    b
}

/// Install a deterministic diff view on ticket 3 and enter `Screen::Diff`.
/// Seeded directly: FakeTransport's snapshot carries no worktrees and its
/// catch-all answers `Ok`, so the `v` entry path dead-ends in tests.
fn install_diff(app: &mut App) {
    use mesimon_core::diff::{FileDiff, FileEntry, Hunk, HunkLine, Render, Sign};
    let entry = |path: &str, status: &str, adds: Option<u32>, dels: Option<u32>| FileEntry {
        path: path.into(),
        old_path: None,
        status: status.into(),
        old_mode: "100644".into(),
        new_mode: "100644".into(),
        old_blob: "a".repeat(40),
        new_blob: "b".repeat(40),
        adds,
        dels,
        dirty: false,
        untracked: false,
    };
    let mut files = vec![
        entry("src/auth/callback.ts", "M", Some(3), Some(1)),
        entry("img/logo.bin", "M", None, None),
        entry("tool.sh", "M", Some(0), Some(0)),
        entry(".env.local", "", None, None),
    ];
    files[0].dirty = true;
    files[3].untracked = true;
    let text_hunks = vec![
        Hunk {
            old_start: 18,
            old_len: 3,
            new_start: 18,
            new_len: 4,
            header: "export async function handleCallback(".into(),
            lines: vec![
                HunkLine { sign: Sign::Ctx, old_ln: Some(18), new_ln: Some(18), text: "  const code = url.searchParams.get('code')".into() },
                HunkLine { sign: Sign::Del, old_ln: Some(19), new_ln: None, text: "  const t = await exchange(code)".into() },
                HunkLine { sign: Sign::Add, old_ln: None, new_ln: Some(19), text: "  const verifier = sessionStore.take(state) // a deliberately long line that soft-wraps at every pane width so the continuation gutter earns its keep".into() },
                HunkLine { sign: Sign::Add, old_ln: None, new_ln: Some(20), text: "  const t = await exchange(code, verifier)\r".into() },
                HunkLine { sign: Sign::Ctx, old_ln: Some(20), new_ln: Some(21), text: "  return persist(t)".into() },
            ],
        },
        Hunk {
            old_start: 41,
            old_len: 2,
            new_start: 43,
            new_len: 3,
            header: "function persist(token: Token) {".into(),
            lines: vec![
                HunkLine { sign: Sign::Ctx, old_ln: Some(41), new_ln: Some(43), text: "  const enc = seal(token)".into() },
                HunkLine { sign: Sign::Add, old_ln: None, new_ln: Some(44), text: "  metrics.increment('auth.callback.ok')".into() },
            ],
        },
    ];
    let fd = |path: &str, render: Render, hunks: Vec<Hunk>| FileDiff {
        path: path.into(),
        old_path: None,
        render,
        hunks,
    };
    let mut cache = std::collections::HashMap::new();
    cache.insert(
        "src/auth/callback.ts".to_string(),
        fd("src/auth/callback.ts", Render::Text, text_hunks),
    );
    cache.insert("img/logo.bin".to_string(), fd("img/logo.bin", Render::Binary, vec![]));
    cache.insert(
        "tool.sh".to_string(),
        fd(
            "tool.sh",
            Render::ModeOnly { old_mode: "100644".into(), new_mode: "100755".into() },
            vec![],
        ),
    );
    app.worktrees = vec![mesimon_core::command::WorktreeItem {
        ticket: ulid_n(3),
        branch: "msmn/T-3-fix-osc-11-detection".into(),
        status: "attached".into(),
        merged: false,
        conflict: false,
        ahead: 2,
        needs_rebase: false,
        detail: None,
        path: Some("/wt/T-3-fix-osc-11-detection".into()),
    }];
    app.diff = Some(crate::app::DiffState {
        ticket: ulid_n(3),
        rail_idx: 0,
        branch: "msmn/T-3-fix-osc-11-detection".into(),
        base_oid: "a1b2c3d4".repeat(5),
        branch_oid: "b".repeat(40),
        files,
        file_idx: 0,
        scroll: std::cell::Cell::new(0),
        marquee: std::cell::Cell::new(None),
        density: 3,
        cache,
        z_armed: false,
        swap: false,
        worktree_present: true,
    });
    app.screen = Screen::Diff { ticket: ulid_n(3) };
}

// ---- goldens ---------------------------------------------------------------

#[test]
fn golden_calm_board_120() {
    let mut app = app_graphite(fixture(false));
    app.cursor_col = 1;
    app.cursor_row = 0;
    golden("board_calm_120x30", &render(&app, 120, 30));
}

#[test]
fn golden_waiting_board_120() {
    let mut app = app_graphite(fixture(true));
    app.cursor_col = 1;
    app.cursor_row = 0;
    golden("board_waiting_120x30", &render(&app, 120, 30));
}

#[test]
fn golden_move_ghost_120() {
    let mut app = app_graphite(fixture(false));
    app.mode = Mode::Move { ticket: ulid_n(3), col: 2, idx: 1, grab: '>', home: (1, 0) };
    golden("board_move_120x30", &render(&app, 120, 30));
}

#[test]
fn golden_empty_board_120() {
    let mut b = Board::default();
    for (i, name) in ["todo", "in progress", "review", "done"].iter().enumerate() {
        b.columns.push(Column { name: (*name).into(), order: format!("{i}") });
    }
    let app = app_graphite(b);
    golden("board_empty_120x30", &render(&app, 120, 30));
}

#[test]
fn golden_spine_100() {
    let mut app = app_graphite(fixture(true));
    app.cursor_col = 0;
    golden("board_spine_100x24", &render(&app, 100, 24));
}

#[test]
fn golden_archived_board_120() {
    // T-7 gone from done, header counts 6, the archive offer in the header.
    let mut app = app_graphite(fixture_archived());
    app.resources.archive_tickets = 1;
    golden("board_archived_120x30", &render(&app, 120, 30));
}

/// The `?` overlay, the surface the audit found missing. It is rendered from
/// the keymap, so this golden is also a picture of what the board can do.
#[test]
fn golden_help_overlay_120() {
    let mut app = app_graphite(fixture(true));
    app.cursor_col = 1;
    app.help = true;
    golden("help_board_120x30", &render(&app, 120, 30));
}

/// The same overlay on the ticket screen lists a different set — the proof
/// that it answers "here", not "in general".
#[test]
fn golden_help_ticket_120() {
    let mut app = app_graphite(fixture(true));
    app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    app.help = true;
    golden("help_ticket_120x30", &render(&app, 120, 30));
}

/// The armed archive chord: the footer becomes the chord's own scope and
/// names the key still to press. The resting hint said only `a`.
#[test]
fn golden_archive_armed_120() {
    let mut app = app_graphite(fixture(false));
    app.cursor_col = 0;
    app.handle_key(
        ratatui::crossterm::event::KeyCode::Char('a'),
        ratatui::crossterm::event::KeyModifiers::NONE,
    )
    .unwrap();
    let lines = render(&app, 120, 30);
    // Armed, and saying so — not silently waiting.
    assert!(
        lines.last().is_some_and(|l| l.contains("archives")),
        "the armed state must say what the next press does: {:?}",
        lines.last()
    );
    golden("board_archive_armed_120x30", &lines);
}

/// The Esc menu: the board-wide actions, which deliberately have no keys.
#[test]
fn golden_menu_120() {
    let mut app = app_graphite(fixture_archived());
    app.mode = Mode::Menu { idx: 0 };
    golden("menu_120x30", &render(&app, 120, 30));
}

/// The theme picker over the board: five rows, the flavor's ground at the
/// right edge, and the saved slots named in words on their rows.
#[test]
fn golden_theme_picker_120() {
    let mut app = app_graphite(fixture_archived());
    app.mode = Mode::Theme { idx: 0 };
    golden("theme_picker_120x30", &render(&app, 120, 30));
}

/// The three standing offers, in priority order, right-aligned in the header —
/// and the same three at the top of the menu wearing the same `›`. This golden
/// is the whole suggestion language in one picture.
#[test]
fn golden_suggestions_120() {
    let mut app = suggesting_app();
    golden("board_suggestions_120x30", &render(&app, 120, 30));
    app.mode = Mode::Menu { idx: 0 };
    golden("menu_suggestions_120x30", &render(&app, 120, 30));
}

/// A board with all three offers standing.
fn suggesting_app() -> App {
    let mut app = app_graphite(fixture_archived());
    app.force_update_ready();
    app.resources.reclaim_sessions = 3;
    app.resources.reclaim_bytes = 3 << 30;
    app.resources.archive_tickets = 2;
    app
}

/// The release offer: a newer tag than this build is published, and the
/// version is in both places the offer lives — the header chip and the menu
/// row it points at. `update_ready` is deliberately NOT forced here; this is
/// the half of the pair where nothing is on disk yet.
#[test]
fn golden_release_offer_120() {
    let mut app = app_graphite(fixture_archived());
    app.force_release_available("v0.1.0-alpha.5");
    let head = &render(&app, 120, 30)[0];
    assert!(
        head.ends_with("◦ v0.1.0-alpha.5 available (esc)"),
        "the offer names the version and routes through the menu: {head:?}"
    );
    app.mode = Mode::Menu { idx: 0 };
    golden("menu_release_offer_120x30", &render(&app, 120, 30));
}

/// A binary already waiting on disk outranks a download. Both halves can be
/// true at once — `install.sh` run in another terminal is exactly that — and
/// when they are, the board offers the restart and stops offering the fetch:
/// one update, one row, no second copy of the same bytes.
#[test]
fn test_a_landed_binary_outranks_a_download() {
    let mut app = app_graphite(fixture(false));
    app.force_release_available("v0.1.0-alpha.5");
    app.force_update_ready();
    let head = &render(&app, 120, 30)[0];
    assert!(head.ends_with("◦ update ready (U ∙ esc)"), "{head:?}");
    assert!(!head.contains("available"), "one offer, not both: {head:?}");
    app.mode = Mode::Menu { idx: 0 };
    let menu = render(&app, 120, 30).join("\n");
    assert!(menu.contains("Restart on the new build"), "{menu}");
    assert!(!menu.contains("Install v0.1.0-alpha.5"), "the download row stands down too: {menu}");
}

/// One offer at a time, right-aligned: the highest priority, in words, with the
/// key that takes it. No count of what is queued behind it — the menu is where
/// the rest are read.
#[test]
fn test_header_offers_one_suggestion_at_a_time() {
    let app = suggesting_app();
    let head = &render(&app, 120, 30)[0];
    assert!(
        head.ends_with("◦ update ready (U ∙ esc)"),
        "the top offer sits at the right edge, alone: {head:?}"
    );
    assert!(!head.contains("sleep"), "only one offer is spelled out: {head:?}");
    assert!(!head.contains('+'), "no queue depth in the header: {head:?}");
    assert!(
        head.find('◦').expect("mark") > head.find("tickets").expect("count"),
        "the offer sits right of the state"
    );
    // A lone offer names itself the same way — the chip has no other shape.
    let mut app = app_graphite(fixture(false));
    app.force_update_ready();
    let head = &render(&app, 120, 30)[0];
    assert!(head.ends_with("◦ update ready (U ∙ esc)"), "{head:?}");
    // Nothing standing, nothing said.
    let quiet = &render(&app_graphite(fixture(false)), 120, 30)[0];
    assert!(!quiet.contains('◦'), "a quiet board offers nothing: {quiet:?}");
}

/// A shell startup file that moved is an offer like any other: one chip, the
/// esc route, and a menu row that says what taking it will and will not touch.
#[test]
fn test_shell_env_change_is_offered_and_says_what_it_reaches() {
    let mut app = app_graphite(fixture(false));
    app.shell_env = mesimon_core::command::ShellEnvStatus { stale: true, ..Default::default() };
    let head = &render(&app, 120, 30)[0];
    assert!(head.ends_with("◦ shell env changed (esc)"), "{head:?}");

    // The row a person lands on must name the boundary: a running process's
    // environment cannot be changed, so a live pane keeps the one it has.
    app.mode = Mode::Menu { idx: 0 };
    let menu = render(&app, 120, 30).join("\n");
    assert!(menu.contains("Reload the shell environment"), "{menu}");
    assert!(
        menu.contains("live panes keep theirs"),
        "the row must name what it cannot reach:\n{menu}"
    );

    // A reload already running is not still an offer — the press must visibly
    // land even while the user's rc files are being read.
    app.shell_env.reloading = true;
    let head = &render(&app, 120, 30)[0];
    assert!(!head.contains("shell env"), "a reload in flight is not an offer: {head:?}");

    // A capture that FAILED is the same act but not the same news: panes are
    // running on a fallback, and the chip is the only place that gets said.
    app.shell_env.reloading = false;
    app.shell_env = mesimon_core::command::ShellEnvStatus { failed: true, ..Default::default() };
    let head = &render(&app, 120, 30)[0];
    assert!(head.ends_with("◦ shell env unreadable (esc)"), "{head:?}");
    app.mode = Mode::Menu { idx: 0 };
    let menu = render(&app, 120, 30).join("\n");
    assert!(menu.contains("Try the shell environment again"), "{menu}");
    assert!(menu.contains("panes are on a fallback"), "{menu}");

    // And an update outranks it: one chip, highest priority, as ever.
    app.mode = Mode::Normal;
    app.shell_env.stale = true;
    app.force_update_ready();
    let head = &render(&app, 120, 30)[0];
    assert!(head.ends_with("◦ update ready (U ∙ esc)"), "{head:?}");
}

/// Priority decides which one is spoken, not which one is loudest: with no
/// update on disk the sleep offer takes the chip, and archive waits in the menu.
#[test]
fn test_suggestion_priority_picks_the_chip() {
    let mut app = app_graphite(fixture_archived());
    app.resources.reclaim_sessions = 3;
    app.resources.reclaim_bytes = 3 << 30;
    app.resources.archive_tickets = 2;
    let head = &render(&app, 120, 30)[0];
    assert!(head.ends_with("◦ sleep 3 agents (Z ∙ esc)"), "{head:?}");
    // A chip that cannot fit says nothing rather than shearing the line — the
    // menu is still one Esc away. Scarcity outranks an offer for the room.
    app.resources.pty_total = 511;
    app.resources.pty_used = 409;
    let narrow = &render(&app, 80, 30)[0];
    assert!(narrow.contains("close to the limit"), "the warning keeps its words: {narrow:?}");
    assert!(!narrow.contains('◦'), "no room, no chip: {narrow:?}");
}

/// The header and the menu can never disagree about what is on offer.
///
/// They did, and it shipped (author 2026-08-31: "the suggestion hint showed
/// archive even though sleep was in the menu as well"): the header gated its
/// sleep clause on a 0.1 GiB payoff while the menu row gated on session count,
/// so a small sleep offer was a menu row with no chip and archive took the
/// header alone. The payoff floor now lives on the words in the row's detail,
/// never on whether the offer exists.
#[test]
fn test_a_sub_floor_sleep_offer_still_outranks_archive() {
    let mut app = app_graphite(fixture_archived());
    app.resources.reclaim_sessions = 2;
    app.resources.reclaim_bytes = 4 << 20; // well under the old 0.1 GiB floor
    app.resources.archive_tickets = 3;
    let head = &render(&app, 120, 30)[0];
    assert!(
        head.ends_with("◦ sleep 2 agents (Z ∙ esc)"),
        "sleep outranks archive at any size: {head:?}"
    );
    // And the row it points at says so in words rather than claiming ~0.0GiB.
    app.mode = Mode::Menu { idx: 0 };
    let lines = render(&app, 120, 30);
    let top = lines.iter().find(|l| l.contains("Sleep 2 agents")).expect("the sleep row leads");
    assert!(top.trim_start().starts_with('◦'), "the chip's row is marked: {top:?}");
    let detail = lines.iter().find(|l| l.contains("wake where they left off")).expect("detail");
    assert!(!detail.contains("GiB"), "a payoff that rounds to nothing is spelled: {detail:?}");
}

/// The chip names one offer; the menu holds them all, marked, in the same
/// order — so Esc then Enter takes the one the header named and the rest are
/// right there under it. One glyph, two places.
#[test]
fn test_suggested_rows_lead_the_menu_and_wear_the_mark() {
    let mut app = suggesting_app();
    app.mode = Mode::Menu { idx: 0 };
    let lines = render(&app, 120, 30);
    // Past the header (whose chip wears the same mark): a 12-row menu starts
    // on row 1 of a 30-row frame, beside the board's own text, so the mark
    // is looked for anywhere on the line rather than at its start.
    let marked: Vec<&String> = lines.iter().skip(1).filter(|l| l.contains('◦')).collect();
    assert_eq!(marked.len(), 3, "one marked row per chip: {marked:?}");
    assert!(marked[0].contains("Restart on the new build"), "{marked:?}");
    assert!(marked[1].contains("Sleep 3 agents in done"), "{marked:?}");
    assert!(marked[2].contains("Archive 2 tickets in done"), "{marked:?}");
    // The chip named the first of them and nothing else.
    let head = &render(&app, 120, 30)[0];
    assert!(head.contains("◦ update ready"), "{head:?}");
    assert!(!head.contains("Sleep"), "the menu spells out what the chip does not");
    // Marked rows come before the unmarked ones.
    let first_plain =
        lines.iter().position(|l| l.contains("External sessions")).expect("the plain rows follow");
    let last_marked =
        lines.iter().rposition(|l| l.trim_start().starts_with('◦')).expect("marked rows");
    assert!(last_marked < first_plain, "offers must lead the menu");
    // And the mark is the header's mark, nowhere else on the screen.
    let board = render(&app_graphite(fixture(false)), 120, 30);
    assert!(!board.iter().any(|l| l.contains('◦')), "the mark is not board furniture");
}

/// Nothing selected: the footer offers only what an empty column can do, and
/// the overlay agrees with it. This is the user's rule made visible.
#[test]
fn golden_help_empty_column_120() {
    let mut board = fixture(false);
    board.tickets.retain(|t| t.column != "done"); // leave one column empty
    let mut app = app_graphite(board);
    app.cursor_col = 3;
    app.help = true;
    let lines = render(&app, 120, 30);
    // The user's rule, asserted and not merely pictured: with no card under
    // the cursor, nothing that needs one is offered — in the footer or the
    // overlay, because both read the same predicate.
    for absent in ["move card", "rename", "archive", "delete", "start claude"] {
        assert!(
            !lines.iter().any(|l| l.contains(absent)),
            "{absent:?} offered with an empty column selected"
        );
    }
    assert!(lines.iter().any(|l| l.contains("open ticket")), "creating must always be offered");
    golden("help_empty_column_120x30", &lines);
}

#[test]
fn golden_archived_dialog_120() {
    let mut app = app_graphite(fixture_archived());
    app.mode = Mode::Archived { idx: 0 };
    golden("board_archived_dialog_120x30", &render(&app, 120, 30));
}

#[test]
fn golden_ticket_archived_120() {
    // The identity line carries the archived badge; A restores from here.
    let mut app = app_graphite(fixture_archived());
    app.screen = Screen::Ticket { ticket: ulid_n(7), rail_idx: 0 };
    golden("ticket_archived_120x30", &render(&app, 120, 30));
}

#[test]
fn golden_ticket_screen_120() {
    let mut app = app_graphite(fixture(true));
    app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    golden("ticket_120x30", &render(&app, 120, 30));
}

#[test]
fn golden_ticket_screen_100() {
    let mut app = app_graphite(fixture(true));
    app.screen = Screen::Ticket { ticket: ulid_n(4), rail_idx: 0 };
    golden("ticket_100x24", &render(&app, 100, 24));
}

#[test]
fn golden_ticket_corpse_120() {
    // The rail's one resumable corpse: dim row, "enter resumes" hint.
    let mut b = fixture(false);
    b.sessions.push(session(
        39,
        ulid_n(3),
        SessionKind::Claude,
        SessionState::Exited { reason: ExitReason::UserQuit },
    ));
    let mut app = app_graphite(b);
    app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    golden("ticket_corpse_120x30", &render(&app, 120, 30));
}

/// ...and with the cursor ON it, which is the only place the corpse's own
/// affordances are legible. `x` there is `dismiss`, not `sleep` — a corpse
/// cannot be slept, and the press used to come back "only idle sessions
/// sleep" — and `enter` is `resume`.
#[test]
fn golden_ticket_corpse_selected_120() {
    let mut b = fixture(false);
    b.sessions.push(session(
        39,
        ulid_n(3),
        SessionKind::Claude,
        SessionState::Exited { reason: ExitReason::UserQuit },
    ));
    let mut app = app_graphite(b);
    let idx = app
        .rail_sessions(ulid_n(3))
        .iter()
        .position(|s| s.id == uuid_n(39))
        .expect("the corpse is on the rail");
    app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: idx };
    golden("ticket_corpse_selected_120x30", &render(&app, 120, 30));
}

/// A transcript file holding one assistant reply (plus whatever else the
/// caller appends), written where a peek can read it.
fn write_transcript(name: &str, jsonl: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("msmn-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("peek dir");
    let path = dir.join("t.jsonl");
    std::fs::write(&path, jsonl).expect("peek transcript");
    path
}

/// The agent's last words as the record the peek actually parses (serialized,
/// so a reply full of newlines and backticks escapes itself).
fn reply_record(text: &str) -> String {
    let v = serde_json::json!({
        "uuid": "u1",
        "type": "assistant",
        "message": { "content": [{ "type": "text", "text": text }] },
    });
    format!("{v}\n")
}

fn attach_transcript(b: &mut Board, path: &std::path::Path) {
    b.sessions.iter_mut().find(|s| s.id == uuid_n(31)).expect("session 31").transcript_path =
        Some(path.to_string_lossy().into_owned());
}

/// The markdown an agent reply is actually made of — one of each thing the
/// zone knows how to draw.
const RICH_REPLY: &str = "## What changed\n\nThe OSC-11 query now runs **once**, before raw \
     mode, and the answer is cached for the session.\n\n- `detect.rs` asks the terminal, \
     then hands the flavor to `Theme::new`\n- the goldens moved with it\n\n```sh\ncargo test \
     -p mesimon-tui\n```\n\nStill open: the `--color=never` path is ~~untested~~ covered now.";

#[test]
fn golden_ticket_peek_120() {
    // The left zone previews the selected rail session's latest assistant
    // reply under the PREVIEW heading — always on, no toggle, and the whole
    // zone (a DOCUMENTS placeholder stood above it until 2026-09-01). A
    // running session's indicator names the step underway, not just that
    // one is.
    let path = write_transcript(
        "tpeek-golden",
        &format!(
            "{}{}",
            reply_record(
                "Fixed the OSC-11 race: the query now runs once before raw mode; goldens \
                 updated and clippy is clean."
            ),
            "{\"uuid\":\"u2\",\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"name\":\"Bash\",\
             \"input\":{\"command\":\"cargo test -p mesimon-tui\",\"description\":\"Run the golden tests\"}}]}}\n"
        ),
    );
    let mut b = fixture(false);
    attach_transcript(&mut b, &path);
    let mut app = app_graphite(b);
    app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    golden("ticket_peek_120x30", &render(&app, 120, 30));
    let _ = std::fs::remove_dir_all(path.parent().expect("dir"));
}

#[test]
fn golden_ticket_richtext_120() {
    // An agent reply IS markdown, so the zone draws it as such (rich.rs):
    // the heading takes weight and a breathing row, the list gets bullets and
    // a hanging indent, the fence becomes a painted slab with no border, and
    // the asterisks and backticks stop reaching the screen. Nothing here is
    // colour: value, weight, paint and space carry all of it.
    let path = write_transcript("trich-golden", &reply_record(RICH_REPLY));
    let mut b = fixture(false);
    attach_transcript(&mut b, &path);
    let mut app = app_graphite(b);
    app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    golden("ticket_richtext_120x30", &render(&app, 120, 30));
    let _ = std::fs::remove_dir_all(path.parent().expect("dir"));
}

/// `{ }` on the ticket page turns the preview a page at a time, the way the
/// diff's hunk pane does. The keys are hinted only while there is a further
/// page, the window clamps at the last full one, and a reply that changes
/// under the reader starts over at its top.
#[test]
fn test_preview_pages_a_long_reply() {
    let long: String = (1..=60).map(|i| format!("row {i:02} of the reply\n\n")).collect();
    let path = write_transcript("preview-pages", &reply_record(&long));
    let mut b = fixture(false);
    attach_transcript(&mut b, &path);
    let mut app = app_graphite(b);
    app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    let shows = |app: &App, row: &str| render(app, 120, 30).iter().any(|l| l.contains(row));
    let footer = |app: &App| render(app, 120, 30).last().cloned().unwrap_or_default();

    assert!(shows(&app, "row 01"), "a fresh page starts at the top");
    assert!(!shows(&app, "row 60"));
    assert!(footer(&app).contains("{ }"), "an overflowing preview offers the page keys");
    let v = app.preview_view.get();
    assert!(v.max > 0 && v.page > 1 && !v.follows_tail, "{v:?}");

    press(&mut app, '}');
    assert!(!shows(&app, "row 01"), "one page down and the first row is gone");
    let first = app.preview_view.get().offset;
    assert_eq!(first, v.page);
    // Past the end: the last window is a FULL one, marked nowhere.
    for _ in 0..20 {
        press(&mut app, '}');
    }
    assert!(shows(&app, "row 60"));
    assert_eq!(app.preview_view.get().offset, v.max);
    assert!(
        !render(&app, 120, 30).iter().any(|l| l.contains("row 60 of the reply~")),
        "the last row is not a cut"
    );
    press(&mut app, '{');
    assert!(!shows(&app, "row 60"));
    for _ in 0..20 {
        press(&mut app, '{');
    }
    assert!(shows(&app, "row 01"));
    assert_eq!(app.preview_view.get().offset, 0);

    // Scrolled halfway, then the reply changes: the new one opens at its top.
    press(&mut app, '}');
    assert!(!shows(&app, "row 01"));
    std::fs::write(&path, reply_record(&long.replace("of the reply", "of the next reply")))
        .expect("rewrite");
    let meta = std::fs::metadata(&path).expect("meta");
    // The peek cache keys on (len, mtime); the length differs, which is enough.
    assert_ne!(meta.len(), 0);
    assert!(shows(&app, "row 01 of the next reply"), "a new reply starts at its top");

    // A reply that fits offers nothing to turn: keys inert, hint gone.
    std::fs::write(&path, reply_record("short")).expect("rewrite");
    assert!(shows(&app, "short"));
    assert!(!footer(&app).contains("{ }"));
    press(&mut app, '}');
    assert_eq!(app.preview_view.get().offset, 0);
    let _ = std::fs::remove_dir_all(path.parent().expect("dir"));
}

/// The same keys on a shell's pane: it opens at its bottom (the newest line
/// is what a tail is for), `{` walks up, and `}` back down releases it to
/// follow the pane again rather than pinning it to today's last row.
#[test]
fn test_preview_pages_a_shell_tail() {
    let mut app = app_graphite(fixture(false));
    app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 1 };
    let tail: Vec<String> = (1..=60).map(|i| format!("line {i:02}")).collect();
    app.shell_tail = Some(crate::app::ShellTail::new(uuid_n(32), tail.clone()));
    let shows = |app: &App, row: &str| render(app, 120, 30).iter().any(|l| l.contains(row));

    assert!(shows(&app, "line 60") && !shows(&app, "line 01"), "a tail opens at its bottom");
    let v = app.preview_view.get();
    assert!(v.follows_tail && v.offset == v.max && v.max > 0, "{v:?}");
    assert!(app.preview_scroll.get().is_none(), "following is the absence of a request");

    press(&mut app, '{');
    assert!(!shows(&app, "line 60"));
    assert!(shows(&app, "line 60~") || render(&app, 120, 30).iter().any(|l| l.ends_with('~')));
    // New output while scrolled up: the reader's window holds still.
    let before = app.preview_view.get().offset;
    let mut more = tail.clone();
    more.push("line 61".into());
    app.shell_tail = Some(crate::app::ShellTail::new(uuid_n(32), more));
    let _ = render(&app, 120, 30);
    assert_eq!(app.preview_view.get().offset, before);
    assert!(!shows(&app, "line 61"));

    // One page back down lands where the bottom WAS: the pane grew a row
    // meanwhile, so the window stops one short, says so, and stays pinned
    // (a page is a page, as in the diff). The next press reaches the end
    // and releases it — the new line arrives with it.
    press(&mut app, '}');
    assert!(shows(&app, "line 60~") && !shows(&app, "line 61"));
    assert!(app.preview_scroll.get().is_some());
    press(&mut app, '}');
    assert!(shows(&app, "line 61"), "back at the bottom, and the new line is there");
    assert!(app.preview_scroll.get().is_none(), "at the bottom the tail is released");
}

#[test]
fn golden_diff_screen_120() {
    let mut app = app_graphite(fixture(false));
    install_diff(&mut app);
    golden("diff_120x30", &render(&app, 120, 30));
}

#[test]
fn golden_diff_screen_140() {
    // ≥140: the file list widens to the 08 §10.2 outline width (36).
    let mut app = app_graphite(fixture(false));
    install_diff(&mut app);
    golden("diff_140x40", &render(&app, 140, 40));
}

#[test]
fn golden_diff_screen_100() {
    // 100 is the two-pane floor (08 §10.2 minus the rail): soft-wrap is
    // doing real work in the 68-column hunk pane here.
    let mut app = app_graphite(fixture(false));
    install_diff(&mut app);
    golden("diff_100x24", &render(&app, 100, 24));
}

#[test]
fn golden_diff_screen_narrow_90() {
    // Below the breakpoint: single pane (file list first, z p swaps).
    let mut app = app_graphite(fixture(false));
    install_diff(&mut app);
    golden("diff_90x24", &render(&app, 90, 24));
    if let Some(d) = app.diff.as_mut() {
        d.swap = true;
    }
    golden("diff_90x24_swap", &render(&app, 90, 24));
}

#[test]
fn golden_card_branch_line_120() {
    // The board card's worktree mark + merge-available state (M4a surface,
    // golden owed since; spec §G item 7).
    let mut app = app_graphite(fixture(false));
    app.worktrees = vec![mesimon_core::command::WorktreeItem {
        ticket: ulid_n(3),
        branch: "msmn/T-3-fix-osc-11-detection".into(),
        status: "attached".into(),
        merged: false,
        conflict: false,
        ahead: 2,
        needs_rebase: false,
        detail: None,
        path: Some("/wt/T-3-fix-osc-11-detection".into()),
    }];
    app.cursor_col = 1;
    app.cursor_row = 0;
    golden("board_worktree_120x30", &render(&app, 120, 30));
}

/// Shift+Enter on a worktree ticket parks the spawn while the worktree is
/// cut, so there is no session record to hang the launch mark on — and that
/// is exactly the longest wait on the board. The binding's own status stands
/// in for the record: provisioning is lazy, so a queued binding IS a parked
/// spawn (author 2026-09-01).
#[test]
fn a_provisioning_ticket_launches_too() {
    let mut app = app_graphite(fixture(false));
    let wt = |status: &str| mesimon_core::command::WorktreeItem {
        ticket: ulid_n(1),
        branch: "msmn/T-1-decay-treatments".into(),
        status: status.into(),
        merged: false,
        conflict: false,
        ahead: 0,
        needs_rebase: false,
        detail: None,
        path: None,
    };
    // T-1 has no sessions at all, so whatever mark sits in front of its
    // title is the launch one. The board row spans every column — hence the
    // title in the needle, or T-3's real spinner answers for it.
    let mark = crate::glyphs::launching(crate::glyphs::Tier::Unicode, 0);
    let want = format!("{mark} Decay treatments");
    let launching = |app: &App| render(app, 120, 30).iter().any(|l| l.contains(&want));
    assert!(!launching(&app), "an unprovisioned card already had the mark");
    for status in ["queued", "provisioning"] {
        app.worktrees = vec![wt(status)];
        assert!(launching(&app), "{status} card says nothing");
    }
    // Attached is the end of the wait — the mark goes with it.
    app.worktrees = vec![wt("attached")];
    assert!(!launching(&app), "attached card still launching");
}

/// The tags fixture keeps its own tickets so the thirteen board goldens above
/// stay byte-identical: an untagged card must render exactly as it did before
/// tags existed, which is what the zero-width tag zone buys.
fn fixture_tagged() -> Board {
    let mut b = fixture(false);
    let tag = |b: &mut Board, id: u128, pairs: &[(u8, &str)]| {
        for (g, name) in pairs {
            let _ = b.register_tag(*g, name);
        }
        if let Some(t) = b.tickets.iter_mut().find(|t| t.id == ulid_n(id)) {
            for (g, name) in pairs {
                t.set_tag(*g, Some((*name).to_string()));
            }
        }
    };
    tag(&mut b, 3, &[(1, "BUG"), (2, "STAGING")]);
    // T-7's only session is asleep: the fixture's card for the third level.
    tag(&mut b, 7, &[(1, "FTR")]);
    tag(&mut b, 1, &[(1, "FTR")]);
    // Four tags on one card: the run caps at three and collapses to `+1`.
    tag(&mut b, 5, &[(1, "REGR"), (2, "PRODUCTION"), (3, "auth"), (4, "p1")]);
    b
}

/// Goldens capture `.symbol()` only, and the mark is paint with no symbol at
/// all — so nothing above would notice if it vanished. This is the test that
/// actually looks at the colour.
#[test]
fn test_the_bar_carries_the_tags_in_their_own_tints() {
    let mut app = app_graphite(fixture_tagged());
    app.cursor_col = 0;
    let buf = cells(&app, 120, 30);
    let lines = render(&app, 120, 30);
    // Column 0's cards start at the board's one-cell left margin, and the
    // bar IS the mark — the card spends no cell of its own on tags.
    let bar_x = 1u16;

    // T-1 "Decay treatments" wears exactly one tag (FTR): the bar is painted
    // in its tint and asks for no stroke.
    let y = lines.iter().position(|l| l.contains("Decay treatments")).expect("card") as u16;
    // "Decay treatments" is the first card of the cursor column, so it is the
    // cursor card and wears its tag at full strength.
    let tint = app.theme.pip(app.board.tag_def(1, "FTR").expect("registered").tint() as usize);
    let cell = &buf[(bar_x, y)];
    assert_eq!(cell.symbol(), " ", "the bar drew a glyph");
    assert_eq!(cell.bg, tint, "the bar is not painted in the tag's tint");
    assert!(!cell.modifier.contains(Modifier::UNDERLINED), "one tag, but a stroke appeared");

    // An untagged card keeps the neutral bar the state ladder gave it.
    let uy = lines.iter().position(|l| l.contains("Keymap validator")).expect("card") as u16;
    let plain = &buf[(bar_x, uy)];
    assert_ne!(plain.bg, tint, "an untagged card wore a tag colour");
    assert!(!plain.modifier.contains(Modifier::UNDERLINED));
}

/// Two tags, one cell: `▀` stacked across it, the first over the second. It
/// costs the card no width, which is the whole reason the bar carries them.
/// Two rivals were built and cut — a `▌` split down the cell, and the card's
/// right-edge pad — so there is one home and this test is its whole story.
#[test]
fn test_two_tags_ride_one_cell() {
    let mut app = app_graphite(fixture_tagged());
    app.cursor_col = 0;
    // The card is not the cursor and not asleep, so its tags are at rest.
    let at = |group: u8, name: &str| {
        app.theme.pip_at(
            app.board.tag_def(group, name).expect("registered").tint() as usize,
            crate::theme::TagLevel::Rest,
        )
    };
    let (first, second) = (at(1, "BUG"), at(2, "STAGING"));
    assert_ne!(first, second, "the two channels must read apart");
    let y =
        render(&app, 120, 30).iter().position(|l| l.contains("Fix OSC-11")).expect("card") as u16;

    let buf = cells(&app, 120, 30);
    let x = (0..120u16).find(|x| buf[(*x, y)].symbol() == "▀").expect("no stacked mark");
    assert_eq!(buf[(x, y)].fg, first, "the first tag must be the top half");
    assert_eq!(buf[(x, y)].bg, second);
    // One cell, and nothing else on the row wears either tint — no second
    // block beside the bar, nothing on the trailing pad.
    let painted: Vec<u16> =
        (0..120u16).filter(|c| buf[(*c, y)].bg == first || buf[(*c, y)].bg == second).collect();
    assert_eq!(painted, vec![x], "the second tag took a cell of its own: {painted:?}");
}

/// An OPEN card has a stripe five or six cells tall, and there the two tags
/// are full painted blocks — ~70% the first from the top, ~30% the second
/// under it — rather than two halves of one cell. The half-block is not
/// reached for at all, which is the L1 exception going unspent.
#[test]
fn test_an_open_card_runs_the_tags_down_its_stripe() {
    let path = write_transcript("tags-split", &reply_record("Rebased and green."));
    let mut b = fixture_tagged();
    attach_transcript(&mut b, &path);
    let mut app = app_graphite(b);
    app.cursor_col = 1; // T-3 "Fix OSC-11 detection": BUG + STAGING
    app.cursor_row = 0;
    app.peek = true;
    let at = |group: u8, name: &str| {
        app.theme.pip_at(
            app.board.tag_def(group, name).expect("registered").tint() as usize,
            crate::theme::TagLevel::Selected,
        )
    };
    let (first, second) = (at(1, "BUG"), at(2, "STAGING"));
    let lines = render(&app, 120, 30);
    let top = lines.iter().position(|l| l.contains("Fix OSC-11")).expect("card") as u16;
    let buf = cells(&app, 120, 30);
    let bar_x = (0..120u16).find(|x| buf[(*x, top)].bg == first).expect("no stripe");

    // Walk the stripe down while it stays one of the two tints.
    let mut run: Vec<ratatui::style::Color> = Vec::new();
    for y in top..30 {
        let c = &buf[(bar_x, y)];
        if c.bg != first && c.bg != second {
            break;
        }
        assert_eq!(c.symbol(), " ", "the open stripe drew a glyph at row {y}");
        run.push(c.bg);
    }
    assert!(run.len() >= 3, "the open card was too short to split: {} rows", run.len());
    let low = run.iter().filter(|c| **c == second).count();
    assert_eq!(low, crate::tags::second_rows(run.len()).expect("tall enough"));
    assert!(low >= 1 && low < run.len() - low, "the second tag is not the smaller run");
    assert_eq!(run[0], first, "the first tag is the top of the stripe");
    assert_eq!(*run.last().expect("rows"), second, "the second tag is the bottom");
    // The runs are contiguous: one changeover, not stripes.
    let flips = run.windows(2).filter(|w| w[0] != w[1]).count();
    assert_eq!(flips, 1, "the stripe changed tint {flips} times");
    let _ = std::fs::remove_dir_all(path.parent().expect("dir"));
}

/// Two loudnesses on one board: the cursor card at full strength, every
/// other card a step down — a parked ticket included, since the glyph says
/// asleep and the block says which tag (the quieter third level was cut
/// 2026-09-02 as too muted). There is no alpha in a terminal, so the step
/// is a blend toward the ground.
#[test]
fn test_the_card_state_sets_the_tag_loudness() {
    let mut app = app_graphite(fixture_tagged());
    app.cursor_col = 0;
    app.cursor_row = 0; // "Decay treatments" (FTR) is the cursor card
    let buf = cells(&app, 120, 30);
    let lines = render(&app, 120, 30);
    let tint = app.board.tag_def(1, "FTR").expect("registered").tint() as usize;
    let at = |level| app.theme.pip_at(tint, level);
    // Cards from every column share a screen row, and two of them wear the
    // same tag here — so the search is scoped to the card's own column.
    let bar_of = |lines: &[String],
                  needle: &str,
                  buf: &ratatui::buffer::Buffer,
                  cols: std::ops::Range<u16>| {
        let y = lines.iter().position(|l| l.contains(needle)).expect("card") as u16;
        cols.clone()
            .find(|x| {
                buf[(*x, y)].bg == at(crate::theme::TagLevel::Selected)
                    || buf[(*x, y)].bg == at(crate::theme::TagLevel::Rest)
            })
            .map(|x| buf[(x, y)].bg)
    };
    assert_eq!(
        bar_of(&lines, "Decay treatments", &buf, 0..30),
        Some(at(crate::theme::TagLevel::Selected)),
        "the cursor card must wear its tag at full strength"
    );
    // T-7's session is asleep, and its tag sits at rest like any other
    // card off the cursor: the block does not say "asleep", the glyph does.
    assert_eq!(
        bar_of(&lines, "Painted accent bar", &buf, 88..120),
        Some(at(crate::theme::TagLevel::Rest)),
        "a sleeping ticket wears its tag at the ordinary resting level"
    );
    // Move the cursor off, and the same card steps down to rest.
    app.cursor_row = 1;
    let buf = cells(&app, 120, 30);
    let lines = render(&app, 120, 30);
    assert_eq!(
        bar_of(&lines, "Decay treatments", &buf, 0..30),
        Some(at(crate::theme::TagLevel::Rest)),
        "a card off the cursor sits at rest"
    );
}

/// The two loudnesses are the CARD's, not the palette's: an untagged board
/// steps too. This is the one that shipped broken twice — the levels only
/// reached tag tints, so an untagged card off the cursor looked exactly like
/// the cursor card. And a parked ticket is NOT a third step: it sits at rest
/// with every other card off the cursor (the quieter level was cut
/// 2026-09-02 as too muted; the glyph is what says asleep).
#[test]
fn test_an_untagged_block_ladders_too() {
    let mut app = app_graphite(fixture(false));
    app.cursor_col = 0;
    app.cursor_row = 0; // "Decay treatments": no sessions, and the cursor card
    let buf = cells(&app, 120, 30);
    let lines = render(&app, 120, 30);
    let bar = |needle: &str, lines: &[String], buf: &ratatui::buffer::Buffer, x: u16| {
        let y = lines.iter().position(|l| l.contains(needle)).expect("card") as u16;
        buf[(x, y)].bg
    };
    let selected = bar("Decay treatments", &lines, &buf, 1);
    let resting = bar("Keymap validator", &lines, &buf, 1);
    // T-7's only session is asleep; its column starts near the right edge.
    let sleeping = bar("Painted accent bar", &lines, &buf, 91);
    assert_ne!(selected, resting, "selection did not brighten the block");
    assert_eq!(resting, sleeping, "a sleeping ticket must wear the ordinary resting block");

    // And they are ordered: further from the page ground means louder.
    let lum = |c: ratatui::style::Color| match c {
        ratatui::style::Color::Rgb(r, g, b) => r as u32 + g as u32 + b as u32,
        other => panic!("the block is not painted: {other:?}"),
    };
    assert!(lum(selected) > lum(resting), "the cursor card must be the loudest");
}

/// A tagged card that is waiting wears BOTH: the tag on the bar, the alarm
/// on the inverted title row and the glyph. The bar carries tags only, so
/// needs-you had to survive without it — and it does, on the loudest surface
/// the board has.
#[test]
fn test_a_waiting_card_still_shouts() {
    let mut b = fixture_tagged();
    let mut s = session(
        42,
        ulid_n(3),
        SessionKind::Claude,
        SessionState::RequiresAction { reason: Reason::Permission },
    );
    s.waiting_since = Some(1);
    b.sessions.push(s);
    let mut app = app_graphite(b);
    app.cursor_col = 0;
    let buf = cells(&app, 120, 30);
    let lines = render(&app, 120, 30);
    let y = lines.iter().position(|l| l.contains("Fix OSC-11")).expect("card") as u16;
    let tint = app.theme.pip_at(
        app.board.tag_def(1, "BUG").expect("registered").tint() as usize,
        crate::theme::TagLevel::Rest,
    );
    assert!(
        (0..120u16).any(|x| buf[(x, y)].bg == tint || buf[(x, y)].fg == tint),
        "the waiting card lost its tag"
    );
    assert!(
        (0..120u16).any(|x| buf[(x, y)].bg == ATTN_GRAPHITE),
        "the waiting card lost its alarm row"
    );
}

/// Tags now spend ink on the underline channel, so the one-saturated-colour
/// law is checked there too: no tag may wear the attention accent.
#[test]
fn test_tag_underlines_never_spend_the_accent() {
    let mut app = app_graphite(fixture_tagged());
    app.cursor_col = 0;
    for peek in [false, true] {
        app.peek = peek;
        let buf = cells(&app, 120, 30);
        for y in 0..30u16 {
            for x in 0..120u16 {
                let c = &buf[(x, y)];
                assert_ne!(c.bg, ATTN_GRAPHITE, "attn bg at {x},{y} peek={peek}");
                if c.modifier.contains(Modifier::UNDERLINED) {
                    assert_ne!(c.underline_color, ATTN_GRAPHITE, "attn underline at {x},{y}");
                }
            }
        }
    }
}

/// The second tag costs the card nothing: the same board, the same text, the
/// same widths whether a ticket wears one tag or two. It rides a cell that
/// was already there — at rest the lower half of the bar, open the lower
/// third of the stripe — and that is the whole reason it is paint.
#[test]
fn test_the_second_tag_costs_no_width() {
    let plain = |lines: Vec<String>| -> Vec<String> {
        lines.iter().map(|l| l.replace('▀', " ")).collect()
    };
    for peek in [false, true] {
        let mut two = app_graphite(fixture_tagged());
        two.cursor_col = 1;
        two.cursor_row = 0;
        two.peek = peek;
        let mut one = app_graphite(fixture_tagged());
        one.cursor_col = 1;
        one.cursor_row = 0;
        one.peek = peek;
        one.board.ticket_mut(ulid_n(3)).expect("ticket").set_tag(2, None);
        // The peek row names the tags, so it legitimately differs; every
        // other row must be identical.
        let (a, b) = (plain(render(&two, 120, 30)), plain(render(&one, 120, 30)));
        assert_eq!(a.len(), b.len(), "peek {peek}: the board changed height");
        for (x, y) in a.iter().zip(&b) {
            if x.contains("STAGING") {
                continue;
            }
            assert_eq!(x, y, "peek {peek}: the second tag moved something");
        }
    }
}

/// The peek names the tags: colour says how many, words say which. The row
/// sits under the title and above the reply.
#[test]
fn test_the_peek_names_the_tags() {
    let path = write_transcript("tags-peek", &reply_record("Rebased and green."));
    let mut b = fixture_tagged();
    attach_transcript(&mut b, &path);
    let mut app = app_graphite(b);
    app.cursor_col = 1; // T-3 "Fix OSC-11 detection": BUG + STAGING
    app.cursor_row = 0;
    app.peek = true;
    let lines = render(&app, 120, 30);
    let title = lines.iter().position(|l| l.contains("Fix OSC-11")).expect("card");
    let names = &lines[title + 1];
    assert!(names.contains("BUG") && names.contains("STAGING"), "tag row missing: {names:?}");
    let reply = lines.iter().position(|l| l.contains("Rebased and green")).expect("reply");
    assert!(reply > title + 1, "the tags must sit above the reply");
    // Painted in their own tints, the same ones the mark under the card uses.
    let buf = cells(&app, 120, 30);
    let tint = app.theme.pip(app.board.tag_def(1, "BUG").expect("registered").tint() as usize);
    let y = (title + 1) as u16;
    assert!((30..58u16).any(|x| buf[(x, y)].bg == tint), "the chips are not wearing the tag tint");
    let _ = std::fs::remove_dir_all(path.parent().expect("dir"));
}

/// The card a quick-tag digit actually lands on usually has no session at
/// all — a backlog ticket — and the row that names its tags used to be gated
/// on the AGENT having a transcript to peek, so the commonest tagged card on
/// the board could never show it. The digit opens the card, the card names
/// the tag, and the reveal expires on its own.
#[test]
fn test_a_quick_tag_names_the_tag_on_a_session_less_card() {
    let mut app = app_graphite(fixture_tagged());
    app.cursor_col = 0;
    app.cursor_row = 0; // T-1 "Decay treatments": wears FTR, has no session.
    let card = |a: &App| {
        let lines = render(a, 120, 30);
        let title = lines.iter().position(|l| l.contains("Decay treatments")).expect("card");
        lines[title + 1].clone()
    };
    // At rest the card is one line: the stripe says "tagged", never which.
    assert!(!card(&app).contains("FTR"), "no tag row before the press");

    app.tag_flash = Some((ulid_n(1), std::time::Instant::now()));
    assert!(card(&app).contains("FTR"), "the flash names the tag: {:?}", card(&app));
    // In the tag's own tint — the same one the stripe under it is wearing.
    let lines = render(&app, 120, 30);
    let y = lines.iter().position(|l| l.contains("Decay treatments")).expect("card") as u16 + 1;
    let buf = cells(&app, 120, 30);
    let tint = app.theme.pip(app.board.tag_def(1, "FTR").expect("registered").tint() as usize);
    assert!((1..40u16).any(|x| buf[(x, y)].bg == tint), "the chip is not wearing the tint");

    // A moment, not a mode — and it belongs to the card it tagged, so the
    // neighbour the cursor is not on stays shut.
    app.tag_flash = Some((ulid_n(2), std::time::Instant::now()));
    assert!(!card(&app).contains("FTR"), "another ticket's flash must not open this card");
}

/// A peeked card with a long vocabulary still names every tag: the names
/// share the row instead of the first one eating it.
#[test]
fn test_a_crowded_peek_row_names_them_all() {
    let path = write_transcript("tags-peek-crowd", &reply_record("Done."));
    let mut b = fixture_tagged();
    attach_transcript(&mut b, &path);
    // Session 31 is on T-3; move the crowded ticket's tags onto it.
    if let Some(t) = b.tickets.iter_mut().find(|t| t.id == ulid_n(3)) {
        t.set_tag(3, Some("auth".into()));
        t.set_tag(4, Some("p1".into()));
    }
    let mut app = app_graphite(b);
    app.cursor_col = 1;
    app.cursor_row = 0;
    app.peek = true;
    let lines = render(&app, 120, 30);
    let title = lines.iter().position(|l| l.contains("Fix OSC-11")).expect("card");
    let names = &lines[title + 1];
    // Four tags on a 28-cell card: the long one gives up cells, the short
    // ones keep theirs.
    for tag in ["BUG", "ST", "auth", "p1"] {
        assert!(names.contains(tag), "{tag} went unnamed in {names:?}");
    }
    let _ = std::fs::remove_dir_all(path.parent().expect("dir"));
}

#[test]
fn golden_board_tags_120() {
    // At rest a tag is one lowercase letter in its own tint — the minimal
    // indication (D18/D31b). T-5 wears four, so its run caps at `+1`.
    let mut app = app_graphite(fixture_tagged());
    app.cursor_col = 0;
    golden("board_tags_120x30", &render(&app, 120, 30));
}

/// The peeked card with its tags named: the row under the title, then the
/// reply. This is the picture the colour alone could not give.
#[test]
fn golden_board_tags_peek_120() {
    let path = write_transcript(
        "tags-peek-golden",
        &reply_record("Rebased onto main, tests green, ready to merge."),
    );
    let mut b = fixture_tagged();
    attach_transcript(&mut b, &path);
    let mut app = app_graphite(b);
    app.cursor_col = 1;
    app.cursor_row = 0;
    app.peek = true;
    golden("board_tags_peek_120x30", &render(&app, 120, 30));
    let _ = std::fs::remove_dir_all(path.parent().expect("dir"));
}

/// A session-less card's chips sit FLUSH under its title: there is no glyph
/// column on that card, so the title starts at the bar and the row follows it
/// (author 2026-09-01: "non session tickets tags line shouldn't be indent").
#[test]
fn golden_board_tags_peek_sessionless_120() {
    let mut app = app_graphite(fixture_tagged());
    app.cursor_col = 0;
    app.cursor_row = 0;
    app.peek = true;
    let lines = render(&app, 120, 30);
    let y = lines.iter().position(|l| l.contains("Decay treatments")).expect("card");
    let title_x = lines[y].find("Decay").expect("title");
    // A chip is " name " — its paint edge is one cell left of the name.
    let chip_x = lines[y + 1].find("FTR").expect("chip row") - 1;
    assert_eq!(chip_x, title_x, "chips start under the title's first character");
    golden("board_tags_peek_sessionless_120x30", &lines);
}

#[test]
fn golden_tag_chord_120() {
    // The `^t` tail: the advisory row names every axis that holds something,
    // because mesimon seeds no vocabulary and a bare digit would mean nothing.
    let mut app = app_graphite(fixture_tagged());
    app.cursor_col = 1;
    app.cursor_row = 0;
    app.tag_armed = Some(crate::app::TagArm {
        ticket: Some(ulid_n(3)),
        row: 0,
        col: 1,
        naming: None,
        forget_armed: false,
    });
    golden("board_tag_chord_120x30", &render(&app, 120, 30));
}

#[test]
fn golden_tag_forget_armed_120() {
    // Retiring reaches every ticket wearing the tag, so it takes two presses
    // and the first one names the blast radius before asking for the second.
    let mut app = app_graphite(fixture_tagged());
    app.cursor_col = 1;
    app.cursor_row = 0;
    app.tag_armed = Some(crate::app::TagArm {
        ticket: Some(ulid_n(3)),
        row: 0,
        col: 0,
        naming: None,
        forget_armed: true,
    });
    golden("board_tag_forget_armed_120x30", &render(&app, 120, 30));
}

#[test]
fn golden_tag_naming_120() {
    // Naming a new tag: the tail falls silent and the row becomes a field.
    let mut app = app_graphite(fixture_tagged());
    app.cursor_col = 1;
    app.cursor_row = 0;
    let mut buffer = crate::text::EditBuffer::new(mesimon_core::board::TITLE_MAX_BYTES);
    for c in "HOTFIX".chars() {
        buffer.insert(c);
    }
    app.tag_armed = Some(crate::app::TagArm {
        ticket: Some(ulid_n(3)),
        row: 0,
        col: 0,
        naming: Some((crate::app::Naming::New, buffer)),
        forget_armed: false,
    });
    golden("board_tag_naming_120x30", &render(&app, 120, 30));
}

#[test]
fn golden_compose_tags_120() {
    // `^t` reaches the composer, and the picks ride on the half-typed ticket
    // until it has an id. This is the case a bare `t` could never serve.
    let mut app = app_graphite(fixture_tagged());
    app.rich_keys = true;
    let mut buffer = crate::text::EditBuffer::new(mesimon_core::board::TITLE_MAX_BYTES);
    for c in "Fix the merge race".chars() {
        buffer.insert(c);
    }
    app.mode = Mode::Input {
        purpose: crate::app::InputPurpose::Create {
            workspace: None,
            tags: vec![mesimon_core::board::TagRef { name: "BUG".into(), group: 1 }],
        },
        buffer,
    };
    app.tag_armed = Some(crate::app::TagArm {
        ticket: None,
        row: 0,
        col: 0,
        naming: None,
        forget_armed: false,
    });
    // The picked tag bands the phantom card, and the picker marks it worn —
    // the two surfaces have to agree before the ticket even exists.
    app.peek = true;
    let lines = render(&app, 120, 30);
    assert!(
        lines.iter().any(|l| l.contains("BUG")),
        "the composer must show what it is about to tag:\n{}",
        lines.join("\n")
    );
    app.peek = false;
    golden("board_compose_tags_120x30", &render(&app, 120, 30));
}

/// The board's prompt field: the card stays WHOLE — glyph, title, sessions —
/// and the field hangs under it. That is the whole design argument in one
/// picture: a prompt has a destination, and the card is the only thing on
/// this screen that names it.
#[test]
fn golden_prompt_field_120() {
    let mut app = app_graphite(fixture(false));
    app.rich_keys = true;
    // T-3, in progress, with a live claude on it.
    app.cursor_col = 1;
    app.cursor_row = 0;
    let mut buffer = crate::text::EditBuffer::new(mesimon_core::board::TITLE_MAX_BYTES);
    for c in "rebase onto main".chars() {
        buffer.insert(c);
    }
    app.mode = Mode::Input {
        purpose: crate::app::InputPurpose::Prompt { ticket: ulid_n(3), walk: None },
        buffer,
    };
    let lines = render(&app, 120, 30);
    assert!(
        lines.iter().any(|l| l.contains("Fix OSC-11 detection")),
        "the card must survive the field — you are typing AT it:\n{}",
        lines.join("\n")
    );
    assert!(
        lines.iter().any(|l| l.contains("rebase onto main")),
        "the prompt is on screen:\n{}",
        lines.join("\n")
    );
    // The mode word names what the text will do, and every other field here
    // saves to the board.
    assert!(lines.last().is_some_and(|l| l.contains("ASK")), "{:?}", lines.last());
    golden("board_prompt_120x30", &render(&app, 120, 30));
}

/// An empty field says what it is for, in the same words the key was hinted
/// with — otherwise the state is a blank row under a card.
#[test]
fn test_an_empty_prompt_field_names_itself() {
    let mut app = app_graphite(fixture(false));
    app.rich_keys = true;
    app.cursor_col = 1;
    app.cursor_row = 0;
    app.mode = Mode::Input {
        purpose: crate::app::InputPurpose::Prompt { ticket: ulid_n(3), walk: None },
        buffer: crate::text::EditBuffer::new(mesimon_core::board::TITLE_MAX_BYTES),
    };
    let lines = render(&app, 120, 30);
    assert!(
        lines.iter().any(|l| l.contains("› ask claude")),
        "an empty prompt field must show its caret and its purpose:\n{}",
        lines.join("\n")
    );
    // …and the footer says how to send it, in the word that is true here.
    assert!(
        lines.last().is_some_and(|l| l.contains("enter send")),
        "the field must not offer `save`: {:?}",
        lines.last()
    );
}

/// The prompt row costs the card no width. It hangs under the frame rather
/// than inside it, so nothing above it moves by a cell.
#[test]
fn test_the_prompt_field_moves_no_text() {
    let mut app = app_graphite(fixture(false));
    app.rich_keys = true;
    app.cursor_col = 1;
    app.cursor_row = 0;
    let before = render(&app, 120, 30);
    app.mode = Mode::Input {
        purpose: crate::app::InputPurpose::Prompt { ticket: ulid_n(3), walk: None },
        buffer: crate::text::EditBuffer::new(mesimon_core::board::TITLE_MAX_BYTES),
    };
    let after = render(&app, 120, 30);
    let card = before
        .iter()
        .position(|l| l.contains("Fix OSC-11 detection"))
        .expect("the card renders without the field");
    assert_eq!(
        before[card], after[card],
        "the card's own rows must be identical with the field open"
    );
}

#[test]
fn golden_ticket_tags_120() {
    // The ticket page spells tags out on the identity line — you came here to
    // read, so there is nothing to decode.
    let mut app = app_graphite(fixture_tagged());
    app.screen = crate::app::Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    golden("ticket_tags_120x30", &render(&app, 120, 30));
}

/// `^t` is bound on the ticket screen, and the screen's early return in
/// `ui::draw` used to skip the panel entirely: the footer flipped to the
/// chord's hints over a grid nobody drew. The keys were live and invisible.
#[test]
fn test_the_picker_reaches_the_ticket_screen() {
    let mut app = app_graphite(fixture_tagged());
    app.screen = crate::app::Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    app.tag_armed = Some(crate::app::TagArm {
        ticket: Some(ulid_n(3)),
        row: 0,
        col: 1,
        naming: None,
        forget_armed: false,
    });
    let lines = render(&app, 120, 30);
    let joined = lines.join("\n");
    assert!(joined.contains(" TAGS "), "the picker panel never drew:\n{joined}");
    assert!(joined.contains("STAGING"), "the grid drew no vocabulary:\n{joined}");
    // The panel is anchored to the bottom and covers the footer row, so the
    // footer is redrawn over it — the chord's keys have to stay named.
    assert!(
        lines.last().is_some_and(|l| l.contains("TAG ") && l.contains("hjkl move")),
        "the footer lost the chord's hints:\n{joined}"
    );
}

/// A FULL axis is wider than an 80-column row — ten tags at eleven cells
/// each — so the picker windows it around the cursor rather than letting the
/// panel clip the tail. Before the cap went to ten the row always fit, and
/// clipping was invisible; now the last tags of a full group have to be
/// reachable, which means the cell under the cursor is always drawn and a `~`
/// says which side is holding the rest.
#[test]
fn test_the_picker_windows_a_full_axis() {
    let mut board = fixture(false);
    for i in 0..mesimon_core::board::MAX_TAGS_PER_GROUP {
        board.register_tag(1, &format!("AXIS{i}")).expect("registered");
    }
    let arm = |col: usize| crate::app::TagArm {
        ticket: Some(ulid_n(3)),
        row: 0,
        col,
        naming: None,
        forget_armed: false,
    };
    let mut app = app_graphite(board);

    // Cursor at the head: the row is anchored at its start and the tail is
    // the side that gets the marker.
    app.tag_armed = Some(arm(0));
    let lines = render(&app, 80, 30);
    let row = lines.iter().find(|l| l.contains("AXIS0")).expect("group 1 row").clone();
    assert!(!row.contains("AXIS9"), "eighty columns cannot hold ten tags: {row:?}");
    assert!(row.contains('~'), "a windowed row must say there is more: {row:?}");

    // Cursor at the tail: the window follows it there, and the head is what
    // scrolls off instead.
    app.tag_armed = Some(arm(mesimon_core::board::MAX_TAGS_PER_GROUP - 1));
    let lines = render(&app, 80, 30);
    let row = lines
        .iter()
        .find(|l| l.contains("AXIS9"))
        .expect("the cursor cell must be drawn, wherever it sits on the axis")
        .clone();
    assert!(row.contains("[   AXIS9]"), "the cursor cell keeps its brackets: {row:?}");
    assert!(!row.contains("AXIS0"), "the head is what scrolls off: {row:?}");
    assert!(row.contains('~'), "a windowed row must say there is more: {row:?}");

    // Given the room, the whole axis is drawn and nothing is marked.
    let lines = render(&app, 120, 30);
    let row = lines.iter().find(|l| l.contains("AXIS0")).expect("group 1 row").clone();
    assert!(row.contains("AXIS9"), "120 columns hold the whole axis: {row:?}");
    assert!(!row.contains('~'), "a row that fits spends no marker: {row:?}");
}

#[test]
fn golden_ticket_tag_chord_120() {
    let mut app = app_graphite(fixture_tagged());
    app.screen = crate::app::Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    app.tag_armed = Some(crate::app::TagArm {
        ticket: Some(ulid_n(3)),
        row: 0,
        col: 1,
        naming: None,
        forget_armed: false,
    });
    golden("ticket_tag_chord_120x30", &render(&app, 120, 30));
}

#[test]
fn golden_composer_selector_120() {
    // The quick-add composer's Shift+Tab workspace selector (M4a surface) and
    // its Shift+Enter save+start. `rich_keys` is what puts the second one in
    // the footer at all — on the legacy floor the row is one hint shorter,
    // which `shift_enter_is_inert_without_rich_keys` pins from the keymap side.
    let mut app = app_graphite(fixture(false));
    app.rich_keys = true;
    let mut buffer = crate::text::EditBuffer::new(mesimon_core::board::TITLE_MAX_BYTES);
    for c in "Ship the diff viewer".chars() {
        buffer.insert(c);
    }
    app.mode = Mode::Input {
        purpose: crate::app::InputPurpose::Create {
            workspace: Some(mesimon_core::board::WorkspaceStrategy::Worktree),
            tags: Vec::new(),
        },
        buffer,
    };
    golden("board_compose_worktree_120x30", &render(&app, 120, 30));
}

/// Adds ride the calm register, deletes the err register (author 2026-08-30
/// colour amendment) — and context stays grey, so the registers stay earned.
#[test]
fn test_diff_add_del_registers() {
    for flavor in Flavor::ALL {
        diff_registers_hold(flavor);
    }
}

fn diff_registers_hold(flavor: Flavor) {
    let theme = Theme::new(flavor, Profile::TrueColor);
    let mut app = App::for_test(fixture(false), Theme::new(flavor, Profile::TrueColor));
    install_diff(&mut app);
    let buf = cells(&app, 120, 30);
    let lines = render(&app, 120, 30);
    let fg_at = |needle: &str| {
        for (y, l) in lines.iter().enumerate() {
            if let Some(ix) = l.find(needle) {
                let x = l[..ix].chars().count() as u16;
                return Some(buf[(x, y as u16)].fg);
            }
        }
        None
    };
    assert_eq!(fg_at("metrics.increment").expect("add line"), theme.calm, "{flavor:?} adds = calm");
    assert_eq!(
        fg_at("const t = await exchange(code)").expect("del line"),
        theme.err,
        "{flavor:?} dels = err"
    );
    assert_eq!(
        fg_at("return persist").expect("ctx line"),
        theme.rest.dim2,
        "{flavor:?} ctx = grey"
    );

    // Full-line grounds (M4b dogfood): the tint spans the WHOLE row — the
    // text cells and the trailing empty cells alike — and context rows stay
    // on the page ground.
    let bg_row = |needle: &str| {
        for (y, l) in lines.iter().enumerate() {
            if let Some(ix) = l.find(needle) {
                let x = l[..ix].chars().count() as u16;
                return Some((buf[(x, y as u16)].bg, buf[(118, y as u16)].bg));
            }
        }
        None
    };
    let page = theme.bg.expect("truecolor paints the page");
    match (theme.diff_add_bg(), theme.diff_del_bg()) {
        (Some(add_bg), Some(del_bg)) => {
            let (text, tail) = bg_row("metrics.increment").expect("add line");
            assert_eq!((text, tail), (add_bg, add_bg), "{flavor:?} add tint spans the row");
            let (text, tail) = bg_row("const t = await exchange(code)").expect("del line");
            assert_eq!((text, tail), (del_bg, del_bg), "{flavor:?} del tint spans the row");
            let (text, _) = bg_row("return persist").expect("ctx line");
            assert_ne!(text, add_bg, "{flavor:?} ctx stays on the page ground");
            assert_ne!(text, del_bg, "{flavor:?} ctx stays on the page ground");
        }
        // A phosphor has no second hue to tint a row with: the register on
        // the text and the glyph carry it, and every row keeps the ground.
        (None, None) => {
            for needle in ["metrics.increment", "const t = await exchange(code)"] {
                let (text, tail) = bg_row(needle).expect(needle);
                assert_eq!((text, tail), (page, page), "{flavor:?} untinted row keeps the ground");
            }
        }
        other => panic!("{flavor:?}: half a diff tint {other:?}"),
    }

    // The top band (row 3) actually paints — the empty-Line idiom regressed
    // silently once already (invisible since M3.5).
    let band_bg = theme.selected_bg.expect("truecolor paints selected");
    assert_eq!(buf[(5u16, 3u16)].bg, band_bg, "{flavor:?} top band paints");
    assert_eq!(buf[(118u16, 3u16)].bg, band_bg, "{flavor:?} top band spans the width");
}

/// The diff footer mirrors the ticket rule: a status outranks the hints.
#[test]
fn test_diff_footer_shows_status() {
    let mut app = app_graphite(fixture(false));
    install_diff(&mut app);
    app.status = "context -U8".into();
    let lines = render(&app, 120, 30);
    let footer = lines.last().expect("footer row");
    assert!(footer.contains("context -U8"), "status missing from diff footer: {footer:?}");
    assert!(!footer.contains("jk scroll"), "hints should yield to status");
}

/// A shell keeps no transcript, so the ticket page previews its pane instead:
/// the latest command and what it printed, under the same PREVIEW heading an
/// agent's reply gets. One name, because both are the last of a record and
/// neither is the record — the rail row says which session it belongs to.
#[test]
fn test_shell_zone_previews_the_pane() {
    let mut app = app_graphite(fixture(false));
    // Rail row 1 of T-3 is the shell; row 0 is the agent.
    app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 1 };
    app.shell_tail = Some(crate::app::ShellTail::new(
        uuid_n(32),
        vec!["$ cargo test -p mesimon-tui".into(), "test result: ok. 212 passed; 0 failed".into()],
    ));
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("PREVIEW")), "the zone keeps its heading");
    assert!(
        lines.iter().any(|l| l.contains("$ cargo test -p mesimon-tui")),
        "the command must be on screen"
    );
    assert!(
        lines.iter().any(|l| l.contains("test result: ok. 212 passed")),
        "and so must what it printed"
    );

    // A capture belongs to the session it was taken from: select the agent
    // and no shell output is left standing under the heading.
    app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    let lines = render(&app, 120, 30);
    assert!(
        !lines.iter().any(|l| l.contains("cargo test -p mesimon-tui")),
        "another session's pane must not follow the cursor"
    );
}

/// A daemon refusal set into `app.status` must reach the ticket footer —
/// it outranks the key hints there just as it does on the board.
#[test]
fn test_ticket_footer_shows_status() {
    let mut app = app_graphite(fixture(true));
    app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    app.status = "pinned awake".into();
    let lines = render(&app, 120, 30);
    let footer = lines.last().expect("footer row");
    assert!(footer.contains("pinned awake"), "status missing from ticket footer: {footer:?}");
    assert!(!footer.contains("jk select"), "hints should yield to status");
}

#[test]
fn thinking_replaces_the_state_word_when_the_prompt_is_newer() {
    // A running agent whose transcript holds nothing since the user's
    // message: the reply below it answers an older question, so the zone
    // shows the question instead and says what the agent is doing about it.
    let dir = std::env::temp_dir().join(format!("msmn-think-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("peek dir");
    let path = dir.join("t.jsonl");
    std::fs::write(
        &path,
        "{\"uuid\":\"u1\",\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\
         \"text\":\"Fixed the OSC-11 race.\"}]}}\n\
         {\"uuid\":\"u2\",\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"now do the other thing\"}}\n",
    )
    .expect("peek transcript");
    let mut b = fixture(false);
    b.sessions.iter_mut().find(|s| s.id == uuid_n(31)).expect("session 31").transcript_path =
        Some(path.to_string_lossy().into_owned());
    let mut app = app_graphite(b);
    app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    let lines = render(&app, 120, 30);
    assert!(
        // The peek marks the user's own words with `>`; the zone reads that
        // as the quote it is and draws 06 §5.1's mark instead (rich.rs).
        lines.iter().any(|l| l.contains("\u{203A} now do the other thing")),
        "the zone shows the question the agent is on, not the stale answer"
    );
    assert!(
        !lines.iter().any(|l| l.contains("Fixed the OSC-11 race")),
        "the stale reply must not sit under a live spinner"
    );
    assert!(
        lines.iter().any(|l| l.contains("● thinking")),
        "and `thinking` replaces `working`: it has the prompt and nothing to show yet"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn golden_peek_board_120() {
    // `p`: the cursor card grows wrapped transcript-peek rows under its
    // session rows, read straight from the transcript file at draw time,
    // closing with the step the agent is on right now.
    let dir = std::env::temp_dir().join(format!("msmn-peek-golden-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("peek dir");
    let path = dir.join("t.jsonl");
    std::fs::write(
        &path,
        "{\"uuid\":\"u1\",\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\
         \"text\":\"Fixed the OSC-11 race: the query now runs once before raw mode; goldens updated and clippy is clean.\"}]}}\n\
         {\"uuid\":\"u2\",\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"name\":\"Bash\",\
         \"input\":{\"command\":\"cargo test -p mesimon-tui\",\"description\":\"Run the golden tests\"}}]}}\n",
    )
    .expect("peek transcript");
    let mut b = fixture(false);
    b.sessions.iter_mut().find(|s| s.id == uuid_n(31)).expect("session 31").transcript_path =
        Some(path.to_string_lossy().into_owned());
    let mut app = app_graphite(b);
    app.cursor_col = 1;
    app.cursor_row = 0;
    app.peek = true;
    golden("board_peek_120x30", &render(&app, 120, 30));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn golden_mono_board_120() {
    let mut app = App::for_test(fixture(true), Theme::new(Flavor::Graphite, Profile::Mono));
    app.cursor_col = 1;
    golden("board_mono_120x30", &render(&app, 120, 30));
}

/// A single-session cursor card lists no session row — line 1's aggregate
/// glyph + age ARE that session, so the accordion row would be a duplicate.
#[test]
fn test_single_session_card_hides_session_row() {
    let mut app = app_graphite(fixture(false));
    app.cursor_col = 1; // "in progress"
    app.cursor_row = 1; // T-4: exactly one (claude) session
                        // The columns only — the footer legitimately names `c claude` now that
                        // hints come from the keymap, and that is not an accordion row.
    let body = |app: &App| -> Vec<String> {
        let lines = render(app, 120, 30);
        lines[..lines.len() - 1].to_vec()
    };
    assert!(
        !body(&app).iter().any(|l| l.contains("claude")),
        "single-session accordion must not repeat the session as a row"
    );
    // Two sessions still list both rows (cursor_row 0 is T-3: claude + bash).
    app.cursor_row = 0;
    let lines = body(&app);
    assert!(lines.iter().any(|l| l.contains("claude")) && lines.iter().any(|l| l.contains("bash")));
}

// ---- laws ------------------------------------------------------------------

/// Drawing never panics across the size matrix (layout arithmetic holds).
#[test]
fn test_layout_arithmetic() {
    for w in [60u16, 80, 100, 110, 120, 140, 200] {
        for h in [20u16, 24, 40] {
            for cursor in 0..4 {
                let mut app = app_graphite(fixture(true));
                app.cursor_col = cursor;
                let _ = render(&app, w, h);
            }
            // The diff screen across the same matrix, both single-pane swaps.
            for swap in [false, true] {
                let mut app = app_graphite(fixture(true));
                install_diff(&mut app);
                if let Some(d) = app.diff.as_mut() {
                    d.swap = swap;
                }
                let _ = render(&app, w, h);
            }
        }
    }
}

/// Graphite's accent, for the tests that render graphite alone. The two
/// provenance laws sweep every flavor's own `attn` instead.
const ATTN_GRAPHITE: Color = Color::Rgb(0xF0, 0xA9, 0x3A);

/// L3: on a calm board not one cell renders the saturated colour — on any
/// flavor. On a phosphor every token shares a hue, so this and
/// `attn_is_its_own_colour` are what keep "the one bright thing" true there.
#[test]
fn test_attn_provenance_calm() {
    for flavor in Flavor::ALL {
        let theme = Theme::new(flavor, Profile::TrueColor);
        let attn = theme.attn;
        let mut app = App::for_test(fixture(false), theme);
        app.cursor_col = 1;
        let mut arch = App::for_test(fixture_archived(), Theme::new(flavor, Profile::TrueColor));
        arch.mode = Mode::Archived { idx: 0 };
        for buf in [cells(&app, 120, 30), cells(&arch, 120, 30)] {
            for y in 0..30 {
                for x in 0..120 {
                    let c = &buf[(x, y)];
                    assert_ne!(c.fg, attn, "{flavor:?} attn fg at {x},{y} on a calm board");
                    assert_ne!(c.bg, attn, "{flavor:?} attn bg at {x},{y} on a calm board");
                }
            }
        }
    }
}

/// L3: on a waiting board every attn cell sits on the header row or on the
/// waiting card's own rows.
#[test]
fn test_attn_provenance_waiting() {
    for flavor in Flavor::ALL {
        attn_stays_on_the_waiting_card(flavor);
    }
}

fn attn_stays_on_the_waiting_card(flavor: Flavor) {
    let theme = Theme::new(flavor, Profile::TrueColor);
    let attn = theme.attn;
    let mut app = App::for_test(fixture(true), theme);
    app.cursor_col = 0; // cursor away from the waiting card
    let buf = cells(&app, 120, 30);
    // Rows that legally carry attn: the header (0) and the rows of the card
    // whose title is "Adopt drawer import".
    let lines = render(&app, 120, 30);
    let card_rows: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.contains("Adopt drawer import"))
        .map(|(y, _)| y)
        .collect();
    assert!(!card_rows.is_empty());
    let legal: Vec<usize> = card_rows.iter().flat_map(|y| [*y, *y + 1]).chain([0usize]).collect();
    let mut seen_attn = false;
    for y in 0..30usize {
        for x in 0..120u16 {
            let c = &buf[(x, y as u16)];
            if c.fg == attn || c.bg == attn {
                seen_attn = true;
                assert!(
                    legal.contains(&y),
                    "{flavor:?} attn cell at {x},{y} outside the earned rows"
                );
            }
        }
    }
    assert!(seen_attn, "{flavor:?}: the waiting board must show attn somewhere");
}

/// What a shell pane actually holds: a command that drew a tree, a progress
/// bar, an SGR escape that survived `capture-pane`, and an invisible width
/// hazard. The preview zone renders pane bytes, so the two laws below have
/// to see them or they do not cover the zone at all.
/// T-3 with two notes: the description (a rich markdown reply's worth) and
/// a short second one — the bodies seeded into the app's cache the way a
/// fetch would land them.
fn note_meta(n: u128, name: &str, by: &str) -> mesimon_core::board::NoteMeta {
    mesimon_core::board::NoteMeta {
        id: ulid_n(n),
        name: name.into(),
        rev: 1,
        created_at: "@100".into(),
        created_by: by.into(),
        edited_at: "@100".into(),
        edited_by: by.into(),
    }
}

const SECOND_NOTE: &str = "Repro steps\n\n1. open the board\n2. press `p`\n3. watch the peek";

fn app_noted() -> App {
    let mut b = fixture(false);
    if let Some(t) = b.tickets.iter_mut().find(|t| t.id == ulid_n(3)) {
        t.notes.push(note_meta(90, "What changed", "local"));
        t.notes.push(note_meta(91, "Repro steps", "agent:00000000-0000-0000-0000-000000000000"));
    }
    let mut app = app_graphite(b);
    app.remember_note(ulid_n(90), 1, Some(crate::peek::sanitize(RICH_REPLY)));
    app.remember_note(ulid_n(91), 1, Some(crate::peek::sanitize(SECOND_NOTE)));
    app
}

fn editor_on(purpose: crate::app::EditorPurpose, title: &str, body: &str) -> crate::app::Editor {
    crate::app::Editor::new(
        purpose,
        crate::text::EditBuffer::from_text(title.to_string(), mesimon_core::board::TITLE_MAX_BYTES),
        crate::text::TextArea::from_text(body, mesimon_core::board::NOTE_MAX_BYTES),
        crate::app::Field::Body,
    )
}

const COMPOSE_BODY: &str = "## Why\n\nThe diff viewer is read-only and the `v` key is free.\n\n- one pane under 107 cols\n- `z` cycles the density";

#[test]
fn golden_editor_compose_120() {
    let mut app = app_graphite(fixture_tagged());
    app.rich_keys = true;
    let mut ed = editor_on(
        crate::app::EditorPurpose::Compose {
            workspace: Some(mesimon_core::board::WorkspaceStrategy::Worktree),
            tags: vec![mesimon_core::board::TagRef { name: "BUG".into(), group: 1 }],
        },
        "Ship the diff viewer",
        COMPOSE_BODY,
    );
    ed.body.page(2);
    ed.body.end();
    app.mode = Mode::Editor(ed);
    golden("editor_compose_120x30", &render(&app, 120, 30));
}

#[test]
fn the_composer_panel_grows_out_of_its_card() {
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    // The one-line composer, in the first column: the board records where
    // its phantom card landed.
    let mut app = app_graphite(fixture(false));
    app.cursor_col = 0;
    let mut buffer = crate::text::EditBuffer::new(mesimon_core::board::TITLE_MAX_BYTES);
    for c in "Ship the diff viewer".chars() {
        buffer.insert(c);
    }
    app.mode = Mode::Input {
        purpose: crate::app::InputPurpose::Create { workspace: None, tags: Vec::new() },
        buffer,
    };
    let before = render(&app, 120, 30);
    let card_row = before
        .iter()
        .position(|l| l.contains("Ship the diff viewer"))
        .expect("the phantom card is on the board");
    let card = app.compose_card.get().expect("the draw records the phantom card");
    assert_eq!(card.y as usize, card_row);
    assert_eq!(card.x, 0, "the first column, with its left pad");
    assert_eq!(card.height, 2, "title row + workspace selector");
    assert!(card.width < 60, "one column, not the board: {card:?}");

    // Tab grows it: the editor carries the card's rectangle as its origin.
    app.handle_key(KeyCode::Tab, KeyModifiers::NONE).expect("tab");
    let Mode::Editor(ed) = &app.mode else { panic!("tab opens the editor") };
    assert_eq!(ed.grow.map(|(r, _)| r), Some(card));
    assert!(app.animating(), "the frame after Tab is in motion");

    // Frame zero: the panel IS the card's rectangle — the title sits on the
    // card's row and the rest of the board is still on screen around it.
    let first = render(&app, 120, 30);
    assert!(first[card_row].starts_with("   Ship the diff viewer"), "{:?}", first[card_row]);
    assert!(
        first.iter().any(|l| l.contains("Fix OSC-11 detection")),
        "the other columns show through while the panel is small"
    );
    assert!(!first.iter().any(|l| l.contains("describe it")), "no body at the card's size");

    // Settled: the panel covers the card rows edge to edge, the column
    // headers stay above it, the body hint and the column name are in it.
    if let Mode::Editor(ed) = &mut app.mode {
        ed.grow = Some((card, std::time::Instant::now() - crate::app::GROW));
    }
    assert!(!app.animating());
    let after = render(&app, 120, 30);
    assert!(after[2].contains("TODO") && after[2].contains("IN PROGRESS"), "{:?}", after[2]);
    assert!(after[4].starts_with("   Ship the diff viewer"), "{:?}", after[4]);
    assert!(after[6].contains("NEW TICKET ∙ TODO column"), "{:?}", after[6]);
    assert!(after[8].contains("describe it"), "{:?}", after[8]);
    assert!(
        !after.iter().any(|l| l.contains("Fix OSC-11 detection")),
        "the cards are under the panel"
    );
    assert!(
        after[29].contains("esc close"),
        "the board's footer speaks for the editor: {:?}",
        after[29]
    );
}

#[test]
fn golden_editor_compose_tags_120() {
    let mut app = app_graphite(fixture_tagged());
    let ed = editor_on(
        crate::app::EditorPurpose::Compose { workspace: None, tags: Vec::new() },
        "Ship the diff viewer",
        "why",
    );
    app.mode = Mode::Editor(ed);
    app.tag_armed = Some(crate::app::TagArm {
        ticket: None,
        row: 0,
        col: 0,
        naming: None,
        forget_armed: false,
    });
    golden("editor_compose_tags_120x30", &render(&app, 120, 30));
}

#[test]
fn golden_editor_note_120() {
    let mut app = app_noted();
    let body: String =
        (1..=30).map(|i| format!("line {i} of the note")).collect::<Vec<_>>().join("\n");
    let mut ed = editor_on(
        crate::app::EditorPurpose::Note { ticket: ulid_n(3), note: Some(ulid_n(90)) },
        "Fix OSC-11 detection",
        &body,
    );
    ed.body.page(12);
    app.mode = Mode::Editor(ed);
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("line 13 of the note")), "{lines:?}");
    golden("editor_note_120x30", &lines);
}

#[test]
fn golden_editor_note_100() {
    let mut app = app_noted();
    let long = "a line long enough to need the window to scroll under the cursor, which is what the narrow golden is for, and then some more";
    let body = format!("{long}\nshort\n{long}");
    let mut ed = editor_on(
        crate::app::EditorPurpose::Note { ticket: ulid_n(3), note: None },
        "Fix OSC-11 detection",
        &body,
    );
    ed.body.end();
    app.mode = Mode::Editor(ed);
    golden("editor_note_100x24", &render(&app, 100, 24));
}

#[test]
fn golden_ticket_description_120() {
    let mut app = app_noted();
    app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("What changed")), "the description block: {lines:?}");
    assert!(lines.iter().any(|l| l.contains("NOTES")), "the rail's notes: {lines:?}");
    golden("ticket_description_120x30", &lines);
}

#[test]
fn golden_ticket_note_selected_120() {
    let mut app = app_noted();
    // Sessions 31 and 32 first, then the two notes: the second note.
    app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 3 };
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("open the board")), "the note in the zone: {lines:?}");
    golden("ticket_note_selected_120x30", &lines);
}

/// A description block eats rows from the zones below, never from the
/// footer, and a ticket with no description keeps its old geometry.
#[test]
fn test_description_block_yields_to_the_zones() {
    let mut noted = app_noted();
    let mut plain = app_graphite(fixture(false));
    noted.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    plain.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    let with = render(&noted, 120, 30);
    let without = render(&plain, 120, 30);
    let sessions_at = |lines: &[String]| lines.iter().position(|l| l.contains("SESSIONS")).unwrap();
    assert!(sessions_at(&with) > sessions_at(&without), "the block pushes the zones down");
    assert!(with[29].trim_start().starts_with("TICKET"), "the footer stays put: {}", with[29]);
    assert!(with[29].contains("n describe"), "{}", with[29]);
}

fn dirty_tail() -> Vec<String> {
    vec![
        "$ tree -L 1 crates".to_string(),
        "\u{251c}\u{2500}\u{2500} mesimon-core  \u{2588}\u{2588}\u{2594} 60%".to_string(),
        "\u{1b}[1m\u{1b}[3mdone\u{1b}[0m in 2.4s\u{200b}\u{fe0f}".to_string(),
    ]
}

/// 06 §5.1: banned SGR never reaches a cell; REVERSED only in Mono/Ansi8.
#[test]
fn test_no_banned_sgr() {
    let path = write_transcript("sgr-law", &reply_record(RICH_REPLY));
    let pairs = Flavor::ALL
        .map(|f| (f, Profile::TrueColor))
        .into_iter()
        .chain([(Flavor::Graphite, Profile::Ansi256), (Flavor::Blue, Profile::Ansi256)]);
    for (flavor, profile) in pairs {
        let mut app = App::for_test(fixture(true), Theme::new(flavor, profile));
        // Rich transcript text is the one surface that renders arbitrary
        // markdown, so it is where a banned attribute would sneak in.
        attach_transcript(&mut app.board, &path);
        app.cursor_col = 1;
        let mut arch = App::for_test(fixture_archived(), Theme::new(flavor, profile));
        arch.mode = Mode::Archived { idx: 0 };
        let mut picker = App::for_test(fixture(false), Theme::new(flavor, profile));
        picker.mode = Mode::Theme { idx: 2 };
        for buf in [
            cells(&app, 120, 30),
            cells(&arch, 120, 30),
            cells(&picker, 120, 30),
            {
                app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
                assert!(
                    render(&app, 120, 30).iter().any(|l| l.contains("What changed")),
                    "the rich transcript must be ON SCREEN, or this law does not bite"
                );
                cells(&app, 120, 30)
            },
            {
                // The shell's preview zone: raw pane bytes on the same page.
                app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 1 };
                app.shell_tail = Some(crate::app::ShellTail::new(uuid_n(32), dirty_tail()));
                assert!(
                    render(&app, 120, 30).iter().any(|l| l.contains("in 2.4s")),
                    "the whole shell tail must be ON SCREEN, or this law does not bite"
                );
                cells(&app, 120, 30)
            },
            {
                install_diff(&mut app);
                cells(&app, 120, 30)
            },
            {
                // The board's prompt field, on its card. New vocabulary is
                // exactly what these sweeps exist to catch.
                let mut p = App::for_test(fixture(false), Theme::new(flavor, profile));
                p.rich_keys = true;
                p.cursor_col = 1;
                p.mode = Mode::Input {
                    purpose: crate::app::InputPurpose::Prompt { ticket: ulid_n(3), walk: None },
                    buffer: crate::text::EditBuffer::new(mesimon_core::board::TITLE_MAX_BYTES),
                };
                cells(&p, 120, 30)
            },
            {
                // The note editor, holding pane-grade dirt pasted in.
                let mut e = App::for_test(fixture(false), Theme::new(flavor, profile));
                let mut ed = editor_on(
                    crate::app::EditorPurpose::Note { ticket: ulid_n(3), note: None },
                    "Fix OSC-11 detection",
                    "",
                );
                ed.body.paste(&dirty_tail().join("\n"));
                e.mode = Mode::Editor(ed);
                assert!(
                    render(&e, 120, 30).iter().any(|l| l.contains("in 2.4s")),
                    "the note body must be ON SCREEN, or this law does not bite"
                );
                cells(&e, 120, 30)
            },
            {
                // The ticket page with a description block and a note in the
                // zone: two more surfaces that render markdown.
                let mut n = App::for_test(fixture(false), Theme::new(flavor, profile));
                if let Some(t) = n.board.tickets.iter_mut().find(|t| t.id == ulid_n(3)) {
                    t.notes.push(note_meta(90, "What changed", "local"));
                    t.notes.push(note_meta(91, "tree", "local"));
                }
                n.remember_note(ulid_n(90), 1, Some(crate::peek::sanitize(RICH_REPLY)));
                n.remember_note(
                    ulid_n(91),
                    1,
                    Some(crate::peek::sanitize(&dirty_tail().join("\n"))),
                );
                n.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 3 };
                let lines = render(&n, 120, 30);
                assert!(lines.iter().any(|l| l.contains("What changed")), "description on screen");
                assert!(lines.iter().any(|l| l.contains("in 2.4s")), "note on screen");
                cells(&n, 120, 30)
            },
        ] {
            for y in 0..30 {
                for x in 0..120 {
                    let m = buf[(x, y)].modifier;
                    assert!(!m.contains(Modifier::DIM), "DIM at {x},{y}");
                    assert!(!m.contains(Modifier::ITALIC), "ITALIC at {x},{y}");
                    assert!(!m.contains(Modifier::SLOW_BLINK), "BLINK at {x},{y}");
                    assert!(!m.contains(Modifier::CROSSED_OUT), "STRIKE at {x},{y}");
                    assert!(!m.contains(Modifier::REVERSED), "REVERSED at {x},{y} in {profile:?}");
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(path.parent().expect("dir"));
}

/// L1: zero drawn structure — no box-drawing or block-element codepoints
/// anywhere (the accent bar is a painted space).
#[test]
fn test_no_drawn_structure() {
    let path = write_transcript("drawn-law", &reply_record(RICH_REPLY));
    let mut app = app_graphite(fixture(true));
    // Markdown is full of rules and boxes; none of them may reach a cell.
    attach_transcript(&mut app.board, &path);
    app.cursor_col = 1;
    let mut arch = app_graphite(fixture_archived());
    arch.mode = Mode::Archived { idx: 0 };
    let mut picker = app_graphite(fixture(false));
    picker.mode = Mode::Theme { idx: 2 };
    let screens: Vec<Vec<String>> = vec![
        render(&app, 120, 30),
        render(&arch, 120, 30),
        render(&picker, 120, 30),
        {
            app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
            let lines = render(&app, 120, 30);
            assert!(
                lines.iter().any(|l| l.contains("What changed")),
                "the rich transcript must be ON SCREEN, or this law does not bite"
            );
            lines
        },
        {
            // A `tree` in a shell pane is a boxful of the banned range, and
            // the preview zone draws pane bytes: it has to be swept too.
            app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 1 };
            app.shell_tail = Some(crate::app::ShellTail::new(uuid_n(32), dirty_tail()));
            let lines = render(&app, 120, 30);
            assert!(
                lines.iter().any(|l| l.contains("in 2.4s")),
                "the whole shell tail must be ON SCREEN, or this law does not bite"
            );
            lines
        },
        {
            install_diff(&mut app);
            render(&app, 120, 30)
        },
        // The tag picker, open and mid-rename. It was NOT covered here, and
        // that is exactly how a U+2588 cursor got shipped into it.
        {
            let mut t = app_graphite(fixture_tagged());
            t.tag_armed = Some(crate::app::TagArm {
                ticket: Some(ulid_n(3)),
                row: 0,
                col: 0,
                naming: None,
                forget_armed: false,
            });
            render(&t, 120, 30)
        },
        {
            let mut t = app_graphite(fixture_tagged());
            let mut buf = crate::text::EditBuffer::new(mesimon_core::board::TITLE_MAX_BYTES);
            for c in "HOTFIX".chars() {
                buf.insert(c);
            }
            t.tag_armed = Some(crate::app::TagArm {
                ticket: Some(ulid_n(3)),
                row: 0,
                col: 0,
                naming: Some((crate::app::Naming::Rename, buf)),
                forget_armed: false,
            });
            render(&t, 120, 30)
        },
        {
            // The prompt field: a caret glyph the board did not have before,
            // and a row that has to survive the same law as every other.
            let mut p = app_graphite(fixture(false));
            p.rich_keys = true;
            p.cursor_col = 1;
            let mut buf = crate::text::EditBuffer::new(mesimon_core::board::TITLE_MAX_BYTES);
            for c in "rebase onto main".chars() {
                buf.insert(c);
            }
            p.mode = Mode::Input {
                purpose: crate::app::InputPurpose::Prompt { ticket: ulid_n(3), walk: None },
                buffer: buf,
            };
            let lines = render(&p, 120, 30);
            assert!(
                lines.iter().any(|l| l.contains("rebase onto main")),
                "the field must be ON SCREEN, or this law does not bite"
            );
            lines
        },
        {
            // The note editor: a body pasted from a pane, through the scrub
            // path, and the composer's picker open over it.
            let mut e = app_graphite(fixture_tagged());
            let mut ed = editor_on(
                crate::app::EditorPurpose::Compose { workspace: None, tags: Vec::new() },
                "Ship it",
                "",
            );
            ed.body.paste(&dirty_tail().join("\n"));
            e.mode = Mode::Editor(ed);
            let lines = render(&e, 120, 30);
            assert!(
                lines.iter().any(|l| l.contains("in 2.4s")),
                "the note body must be ON SCREEN, or this law does not bite"
            );
            e.tag_armed = Some(crate::app::TagArm {
                ticket: None,
                row: 0,
                col: 0,
                naming: None,
                forget_armed: false,
            });
            lines.into_iter().chain(render(&e, 120, 30)).collect()
        },
        {
            let mut n = app_noted();
            if let Some(t) = n.board.tickets.iter_mut().find(|t| t.id == ulid_n(3)) {
                t.notes.push(note_meta(92, "tree", "local"));
            }
            n.remember_note(ulid_n(92), 1, Some(crate::peek::sanitize(&dirty_tail().join("\n"))));
            n.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 4 };
            let lines = render(&n, 120, 30);
            assert!(lines.iter().any(|l| l.contains("What changed")), "description on screen");
            assert!(lines.iter().any(|l| l.contains("in 2.4s")), "note on screen");
            lines
        },
    ];
    for lines in screens {
        for l in &lines {
            for ch in l.chars() {
                let cp = ch as u32;
                // `▀` U+2580 is the ONE admitted codepoint in the range, on
                // an explicit exception from the author (2026-09-01): it
                // carries the second tag inside a resting card's single bar
                // cell, which no attribute can do — an underline is a pixel
                // at the bottom of a painted cell and cannot be seen. `▌`
                // U+258C went back to being banned with the home that spent
                // it; `▔` and `█` were never admitted, nor was the rest.
                assert!(
                    !(0x2500..=0x259F).contains(&cp) || ch == '▀',
                    "drawn-structure codepoint {ch:?} in {l:?}"
                );
            }
        }
    }
    let _ = std::fs::remove_dir_all(path.parent().expect("dir"));
}

/// An alarm card never demotes: with the cursor elsewhere, the failed card
/// keeps its err bar (bg paint), never dormant grey.
#[test]
fn test_alarm_never_dimmed() {
    let mut app = app_graphite(fixture(false));
    app.cursor_col = 0;
    let buf = cells(&app, 120, 30);
    let lines = render(&app, 120, 30);
    let err = Color::Rgb(0xD5, 0x80, 0x9A);
    let y =
        lines.iter().position(|l| l.contains("Flaky e2e on runner")).expect("failed card rendered");
    // The bar became the tag channel (2026-09-01), so the err register lives
    // on the card's glyph — at full value, never dimmed, cursor elsewhere.
    let found = (0..120u16).any(|x| buf[(x, y as u16)].fg == err);
    assert!(found, "failed card lost its err register");
    assert!(
        (0..120u16).all(|x| buf[(x, y as u16)].bg != err),
        "the bar is the tag channel now; err must not paint it"
    );
}

/// A parked card's mark recedes with its bar. The sleeping `z` rides
/// `Register::Dormant` = `dim3`, one step under the grey the idle ring and
/// the spinner ride: on `dim2` it sat as loud as a live mark beside a bar
/// that had already faded to its Sleeping level (author 2026-09-02, "z is
/// low effort" — the colour was, not the letter). The goldens are text-only,
/// so this reads the cell.
#[test]
fn test_sleeping_mark_is_dormant() {
    let mut app = app_graphite(fixture(true));
    app.cursor_col = 0;
    let (dim2, dim3) = (app.theme.rest.dim2, app.theme.rest.dim3);
    let buf = cells(&app, 120, 30);
    let lines = render(&app, 120, 30);
    let y =
        lines.iter().position(|l| l.contains("Painted accent bar")).expect("T-7 rendered") as u16;
    let x = (0..120u16).find(|&x| buf[(x, y)].symbol() == "z").expect("sleeping card wears z");
    assert_eq!(buf[(x, y)].fg, dim3, "the sleeping mark rides the de-emphasis floor");
    assert_ne!(dim3, dim2, "or the floor is no step at all");
    // The rail says the same thing of the session itself.
    app.screen = Screen::Ticket { ticket: ulid_n(7), rail_idx: 0 };
    let buf = cells(&app, 120, 30);
    let lines = render(&app, 120, 30);
    let y = lines.iter().position(|l| l.contains("$ bash")).expect("rail row") as u16;
    let x = (0..120u16).find(|&x| buf[(x, y)].symbol() == "z").expect("rail wears z");
    assert_eq!(buf[(x, y)].fg, dim3, "the rail's sleeping mark is dormant too");
}

/// PTY headroom stays hidden until 80% of the OS cap, then warns.
#[test]
fn test_pty_warning_threshold() {
    let mut app = app_graphite(fixture(false));
    app.resources.pty_total = 511;
    app.resources.pty_used = 83;
    let lines = render(&app, 120, 30);
    assert!(!lines[0].contains("ptys"), "quiet headroom must stay hidden");
    app.resources.pty_used = 409; // 80% of 511, rounded up
    let lines = render(&app, 120, 30);
    assert!(lines[0].contains("ptys 409/511 ∙ close to the limit"));
}

/// The cursor-column treatment: the cursor column's header row is a
/// full-width `selected`-surface band (its name at sel.base bold); a
/// non-cursor header is unpainted dim1.
#[test]
fn test_cursor_column_header() {
    let mut app = app_graphite(fixture(false));
    app.cursor_col = 1;
    let buf = cells(&app, 120, 30);
    let band = Color::Rgb(0x27, 0x2B, 0x31);
    // Column headers sit at y=2 (header, breathing row, then columns);
    // column 1 starts at x = 1 + 29 + 1 = 31 (layout: [29,29,29,28], GUT 1).
    for x in [31u16, 40, 59] {
        assert_eq!(buf[(x, 2)].bg, band, "cursor column header band missing at {x}");
    }
    assert_ne!(buf[(1, 2)].bg, band, "non-cursor column must not carry the band");
}

#[test]
fn test_rail_corpse_rules() {
    // The rail keeps exactly one resumable corpse: the latest exited claude
    // that was not dismissed. A deliberate kill stays resumable (the
    // conversation survives the process); only `Dismissed` — `x` on a corpse
    // — hides. Bash corpses and older ones stay off.
    let mut b = fixture(false);
    let t3 = ulid_n(3);
    let mut old =
        session(33, t3, SessionKind::Claude, SessionState::Exited { reason: ExitReason::UserQuit });
    old.state_changed_at = Some(10);
    let mut killed =
        session(34, t3, SessionKind::Claude, SessionState::Exited { reason: ExitReason::Killed });
    killed.state_changed_at = Some(20);
    let mut dismissed = session(
        35,
        t3,
        SessionKind::Claude,
        SessionState::Exited { reason: ExitReason::Dismissed },
    );
    dismissed.state_changed_at = Some(30);
    let mut bash =
        session(36, t3, SessionKind::Bash, SessionState::Exited { reason: ExitReason::UserQuit });
    bash.state_changed_at = Some(40);
    b.sessions.extend([old, killed, dismissed, bash]);
    let app = app_graphite(b);
    let ids: Vec<uuid::Uuid> = app.rail_sessions(t3).iter().map(|s| s.id).collect();
    assert!(ids.contains(&uuid_n(31)), "live claude stays");
    assert!(ids.contains(&uuid_n(32)), "live bash stays");
    assert!(
        ids.contains(&uuid_n(34)),
        "latest non-dismissed corpse rides the rail — killed included"
    );
    assert!(!ids.contains(&uuid_n(33)), "only the latest corpse shows");
    assert!(!ids.contains(&uuid_n(35)), "dismissed corpse stays hidden");
    assert!(!ids.contains(&uuid_n(36)), "bash corpses are not resumable");
    // Fixed creation order (06 §7 R4): the corpse sits where it was spawned.
    assert_eq!(ids, vec![uuid_n(31), uuid_n(32), uuid_n(34)]);
}

/// The MOVE ghost blinks in place (STALE-MAP 2026-08-30): the held card's
/// title cell paints `sel.base` in the bright phase and `sel.dim3` in the
/// dark one, while an ordinary card's title holds still across frames.
#[test]
fn test_move_ghost_blinks() {
    let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
    let title_fg = |buf: &ratatui::buffer::Buffer, needle: &str| {
        for y in 0..30u16 {
            let row: String = (0..120u16).map(|x| buf[(x, y)].symbol()).collect::<String>();
            if let Some(ix) = row.find(needle) {
                let x = row[..ix].chars().count() as u16;
                return Some(buf[(x, y)].fg);
            }
        }
        None
    };
    let mut app = app_graphite(fixture(false));
    // Grab ticket 1 ("Decay treatments") in place.
    app.mode = Mode::Move { ticket: ulid_n(1), col: 0, idx: 0, grab: '<', home: (1, 0) };
    app.spin_epoch.set(Some(std::time::Instant::now()));
    let bright = title_fg(&cells(&app, 120, 30), "Decay").expect("held title, frame 0");
    assert_eq!(bright, theme.sel.base, "bright phase rides sel.base");
    let calm0 = title_fg(&cells(&app, 120, 30), "Grapheme").expect("bystander title");
    // 410 ms back → frame 4 (or 5 under scheduler slop) — both the dark phase.
    app.spin_epoch.set(Some(std::time::Instant::now() - std::time::Duration::from_millis(410)));
    let dark = title_fg(&cells(&app, 120, 30), "Decay").expect("held title, frame 4");
    assert_eq!(dark, theme.sel.dim3, "dark phase rides sel.dim3");
    // The blink belongs to the held card alone.
    let calm4 = title_fg(&cells(&app, 120, 30), "Grapheme").expect("bystander title");
    assert_eq!(calm0, calm4, "bystander cards hold still");
}

/// Requirement 2 of the pending-move gesture: while the ghost blinks in its
/// target column, the ORIGINAL card stays visible semi-transparent (dim3
/// title, ghost bar) — and the same title therefore appears twice on the row.
#[test]
fn test_move_trail_is_semi_transparent() {
    let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
    let mut app = app_graphite(fixture(false));
    // Ticket 1 lives in todo (col 0); its ghost is pending in col 1.
    app.mode = Mode::Move { ticket: ulid_n(1), col: 1, idx: 0, grab: '>', home: (0, 0) };
    app.spin_epoch.set(Some(std::time::Instant::now()));
    let buf = cells(&app, 120, 30);
    let mut fgs = Vec::new();
    for y in 0..30u16 {
        let row: String = (0..120u16).map(|x| buf[(x, y)].symbol()).collect();
        let mut from = 0;
        while let Some(ix) = row[from..].find("Decay") {
            let x = row[..from + ix].chars().count() as u16;
            fgs.push(buf[(x, y)].fg);
            from += ix + 5;
        }
    }
    assert_eq!(fgs.len(), 2, "trail + ghost both render");
    // Column order: the trail (todo) sits left of the ghost (in progress).
    assert_eq!(fgs[0], theme.rest.dim3, "original is semi-transparent");
    assert_eq!(fgs[1], theme.sel.base, "ghost blinks at full value (bright phase)");
}

/// A column clipped below keeps every fitting card at full value and shows
/// the next not-fully-visible card as a one-line dim3 ghost at the edge,
/// separated by the card-rhythm blank — same demotion both directions
/// (author 2026-08-30). A board that fits whole fades nothing.
#[test]
fn test_clipped_column_edge_peeks() {
    let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
    let mut b = Board::default();
    b.columns.push(Column { name: "todo".into(), order: "0".into() });
    b.columns.push(Column { name: "done".into(), order: "1".into() });
    for i in 0..12u128 {
        b.tickets.push(ticket(
            i + 1,
            &format!("T-{i}"),
            &format!("Load {i}"),
            "todo",
            &format!("{i:02}"),
        ));
    }
    b.tickets.push(ticket(99, "T-99", "Elsewhere", "done", "00"));
    let mut app = app_graphite(b);
    app.cursor_col = 1; // todo is a bystander column: scroll pinned to 0

    let rows = |app: &App, h: u16| -> Vec<(u16, ratatui::style::Color)> {
        let buf = cells(app, 120, h);
        let mut out = Vec::new();
        for y in 0..h {
            let row: String = (0..120u16).map(|x| buf[(x, y)].symbol()).collect();
            if let Some(ix) = row.find("Load ") {
                let x = row[..ix].chars().count() as u16;
                out.push((y, buf[(x, y)].fg));
            }
        }
        out
    };

    // Clipped below: fewer than 12 cards visible; the bottom-most is the
    // dim3 ghost, a blank row away from the last full-value card.
    let clipped = rows(&app, 20);
    assert!(
        clipped.len() > 2 && clipped.len() < 12,
        "20 rows must clip; visible={}",
        clipped.len()
    );
    let (gy, gfg) = *clipped.last().unwrap();
    assert_eq!(gfg, theme.rest.dim3, "bottom edge is a ghost peek");
    assert!(gy - clipped[clipped.len() - 2].0 >= 2, "blank row before the bottom ghost");
    for (_, fg) in &clipped[..clipped.len() - 1] {
        assert_eq!(*fg, theme.rest.base, "cards above the edge hold full value");
    }

    // Clipped above: cursor at the tail scrolls the column; the top-most
    // visible card is the ghost, blank-separated, and the cursor card holds
    // full value.
    app.cursor_col = 0;
    app.cursor_row = 11;
    let scrolled = rows(&app, 20);
    assert!(scrolled.len() < 12, "still clipped after scrolling to the tail");
    let (ty, tfg) = *scrolled.first().unwrap();
    assert_eq!(tfg, theme.rest.dim3, "top edge is a ghost peek");
    assert!(scrolled[1].0 - ty >= 2, "blank row after the top ghost");
    assert_eq!(scrolled.last().unwrap().1, theme.sel.base, "cursor card at full value");
    for (_, fg) in &scrolled[1..scrolled.len() - 1] {
        assert_eq!(*fg, theme.rest.base, "interior cards hold full value");
    }

    // Mid-column cursor: both edges peek at once (scroll reset first — the
    // Cell carries the tail scroll from the case above).
    app.scroll_row.set(0);
    app.cursor_row = 6;
    let mid = rows(&app, 20);
    assert_eq!(mid.first().unwrap().1, theme.rest.dim3, "top ghost with a mid cursor");
    assert_eq!(mid.last().unwrap().1, theme.rest.dim3, "bottom ghost with a mid cursor");

    // Whole: nothing fades.
    app.cursor_col = 1;
    app.cursor_row = 0;
    let whole = rows(&app, 40);
    assert_eq!(whole.len(), 12, "40 rows fit the whole column");
    for (_, fg) in &whole {
        assert_eq!(*fg, theme.rest.base, "a fully visible column never fades");
    }
}
