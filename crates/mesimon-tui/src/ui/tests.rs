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

fn session(
    n: u128,
    ticket: ulid::Ulid,
    kind: SessionKind,
    state: SessionState,
) -> SessionRecord {
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
        workspace: None,
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
    b.sessions.push(session(
        32,
        t3,
        SessionKind::Bash,
        SessionState::Idle { stop_reason: StopReason::Interrupted },
    ));
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
    app.mode = Mode::Move { ticket: ulid_n(3), col: 2, idx: 1 };
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

#[test]
fn golden_ticket_peek_120() {
    // The left zone previews the selected rail session's latest assistant
    // reply under a TRANSCRIPT heading — always on, no toggle (the zone is
    // otherwise empty until documents land in M4).
    let dir = std::env::temp_dir().join(format!("msmn-tpeek-golden-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("peek dir");
    let path = dir.join("t.jsonl");
    std::fs::write(
        &path,
        "{\"uuid\":\"u1\",\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\
         \"text\":\"Fixed the OSC-11 race: the query now runs once before raw mode; goldens updated and clippy is clean.\"}]}}\n",
    )
    .expect("peek transcript");
    let mut b = fixture(false);
    b.sessions
        .iter_mut()
        .find(|s| s.id == uuid_n(31))
        .expect("session 31")
        .transcript_path = Some(path.to_string_lossy().into_owned());
    let mut app = app_graphite(b);
    app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    golden("ticket_peek_120x30", &render(&app, 120, 30));
    let _ = std::fs::remove_dir_all(&dir);
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
fn golden_peek_board_120() {
    // `p`: the cursor card grows wrapped transcript-peek rows under its
    // session rows, read straight from the transcript file at draw time.
    let dir = std::env::temp_dir().join(format!("msmn-peek-golden-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("peek dir");
    let path = dir.join("t.jsonl");
    std::fs::write(
        &path,
        "{\"uuid\":\"u1\",\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\
         \"text\":\"Fixed the OSC-11 race: the query now runs once before raw mode; goldens updated and clippy is clean.\"}]}}\n",
    )
    .expect("peek transcript");
    let mut b = fixture(false);
    b.sessions
        .iter_mut()
        .find(|s| s.id == uuid_n(31))
        .expect("session 31")
        .transcript_path = Some(path.to_string_lossy().into_owned());
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
    let lines = render(&app, 120, 30);
    assert!(
        !lines.iter().any(|l| l.contains("claude")),
        "single-session accordion must not repeat the session as a row"
    );
    // Two sessions still list both rows (cursor_row 0 is T-3: claude + bash).
    app.cursor_row = 0;
    let lines = render(&app, 120, 30);
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
        }
    }
}

const ATTN_GRAPHITE: Color = Color::Rgb(0xF0, 0xA9, 0x3A);

/// L3: on a calm board not one cell renders the saturated colour.
#[test]
fn test_attn_provenance_calm() {
    let mut app = app_graphite(fixture(false));
    app.cursor_col = 1;
    let buf = cells(&app, 120, 30);
    for y in 0..30 {
        for x in 0..120 {
            let c = &buf[(x, y)];
            assert_ne!(c.fg, ATTN_GRAPHITE, "attn fg at {x},{y} on a calm board");
            assert_ne!(c.bg, ATTN_GRAPHITE, "attn bg at {x},{y} on a calm board");
        }
    }
}

/// L3: on a waiting board every attn cell sits on the header row or on the
/// waiting card's own rows.
#[test]
fn test_attn_provenance_waiting() {
    let mut app = app_graphite(fixture(true));
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
    let legal: Vec<usize> =
        card_rows.iter().flat_map(|y| [*y, *y + 1]).chain([0usize]).collect();
    let mut seen_attn = false;
    for y in 0..30usize {
        for x in 0..120u16 {
            let c = &buf[(x, y as u16)];
            if c.fg == ATTN_GRAPHITE || c.bg == ATTN_GRAPHITE {
                seen_attn = true;
                assert!(legal.contains(&y), "attn cell at {x},{y} outside the earned rows");
            }
        }
    }
    assert!(seen_attn, "the waiting board must show attn somewhere");
}

/// 06 §5.1: banned SGR never reaches a cell; REVERSED only in Mono/Ansi8.
#[test]
fn test_no_banned_sgr() {
    for (flavor, profile) in [
        (Flavor::Graphite, Profile::TrueColor),
        (Flavor::Chalk, Profile::TrueColor),
        (Flavor::Graphite, Profile::Ansi256),
    ] {
        let mut app = App::for_test(fixture(true), Theme::new(flavor, profile));
        app.cursor_col = 1;
        for buf in [cells(&app, 120, 30), {
            app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
            cells(&app, 120, 30)
        }] {
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
}

/// L1: zero drawn structure — no box-drawing or block-element codepoints
/// anywhere (the accent bar is a painted space).
#[test]
fn test_no_drawn_structure() {
    let mut app = app_graphite(fixture(true));
    app.cursor_col = 1;
    let screens: Vec<Vec<String>> = vec![render(&app, 120, 30), {
        app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
        render(&app, 120, 30)
    }];
    for lines in screens {
        for l in &lines {
            for ch in l.chars() {
                let cp = ch as u32;
                assert!(
                    !(0x2500..=0x259F).contains(&cp),
                    "drawn-structure codepoint {ch:?} in {l:?}"
                );
            }
        }
    }
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
    let y = lines
        .iter()
        .position(|l| l.contains("Flaky e2e on runner"))
        .expect("failed card rendered");
    // Its bar cell is the first cell of the review column's slot.
    let mut found = false;
    for x in 0..120 {
        if buf[(x, y as u16)].bg == err {
            found = true;
        }
    }
    assert!(found, "failed card lost its err bar");
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
    // that was not `x`-dismissed. Bash corpses and older ones stay off.
    let mut b = fixture(false);
    let t3 = ulid_n(3);
    let mut old =
        session(33, t3, SessionKind::Claude, SessionState::Exited { reason: ExitReason::UserQuit });
    old.state_changed_at = Some(10);
    let mut newer =
        session(34, t3, SessionKind::Claude, SessionState::Exited { reason: ExitReason::Crashed });
    newer.state_changed_at = Some(20);
    let mut killed =
        session(35, t3, SessionKind::Claude, SessionState::Exited { reason: ExitReason::Killed });
    killed.state_changed_at = Some(30);
    let mut bash =
        session(36, t3, SessionKind::Bash, SessionState::Exited { reason: ExitReason::UserQuit });
    bash.state_changed_at = Some(40);
    b.sessions.extend([old, newer, killed, bash]);
    let app = app_graphite(b);
    let ids: Vec<uuid::Uuid> = app.rail_sessions(t3).iter().map(|s| s.id).collect();
    assert!(ids.contains(&uuid_n(31)), "live claude stays");
    assert!(ids.contains(&uuid_n(32)), "live bash stays");
    assert!(ids.contains(&uuid_n(34)), "latest resumable corpse rides the rail");
    assert!(!ids.contains(&uuid_n(33)), "only the latest corpse shows");
    assert!(!ids.contains(&uuid_n(35)), "x-killed stays dismissed");
    assert!(!ids.contains(&uuid_n(36)), "bash corpses are not resumable");
    // Fixed creation order (06 §7 R4): the corpse sits where it was spawned.
    assert_eq!(ids, vec![uuid_n(31), uuid_n(32), uuid_n(34)]);
}
