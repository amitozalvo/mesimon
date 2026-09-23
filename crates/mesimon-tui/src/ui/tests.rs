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
use unicode_width::UnicodeWidthStr;

use crate::app::{App, InputPurpose, Mode, Screen, SharingRow};
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
        created_by: String::new(),
        created_from: None,
        entered_at: None,
        previous_column: None,
        woke_at: None,
        manual_merge: false,
        execution_policy: Default::default(),
        import_origin: None,
        raised: None,
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
        b.columns.push(Column::new(*name, format!("{i}")));
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
    lines_of(&cells(app, w, h))
}

/// `{ }` on the ticket page, LANDED: a page turn is a glide, so the frame
/// after the press still shows the old rows. This dates the glide a full
/// `GLIDE` into the past, the way the composer's grow test pins its frames,
/// so a test reads the page the press asked for.
fn page(app: &mut App, c: char) {
    press(app, c);
    settle_preview(app);
}

fn settle_preview(app: &mut App) {
    if let Some(g) = app.preview.glide.get() {
        app.preview.glide.set(Some(crate::app::Glide {
            at: std::time::Instant::now() - crate::app::GLIDE,
            ..g
        }));
    }
}

fn lines_of(buffer: &ratatui::buffer::Buffer) -> Vec<String> {
    let area = buffer.area();
    let mut lines = Vec::new();
    for y in 0..area.height {
        let mut line = String::new();
        for x in 0..area.width {
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

/// A dialog row without its frame's left edge and the pad after it — what
/// the row itself begins with.
fn unframed(line: &str) -> &str {
    line.trim_start_matches([' ', '│', '|'])
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
        t.archived = Some(mesimon_core::board::Archived {
            at: "@100".into(),
            by: "local".into(),
            until: None,
            needs_you: false,
        });
    }
    b
}

/// The archived fixture with T-7's archive carrying a deadline (T-74): a
/// snooze, waking in the year 2100 so the row reads a stable `wakes in >1y`.
fn fixture_snoozed() -> Board {
    let mut b = fixture_archived();
    if let Some(t) = b.tickets.iter_mut().find(|t| t.id == ulid_n(7)) {
        t.archived = Some(mesimon_core::board::Archived {
            at: "@100".into(),
            by: "local".into(),
            until: Some("@4102444800".into()),
            needs_you: true,
        });
    }
    b
}

/// The calm fixture with T-4 back from a snooze that asked to be lit — no
/// session on it at all, so whatever attn appears is the ticket's own.
fn fixture_woke() -> Board {
    let mut b = fixture(false);
    if let Some(t) = b.tickets.iter_mut().find(|t| t.id == ulid_n(4)) {
        t.woke_at = Some("@100".into());
    }
    b
}

/// The calm fixture with T-5's agent asking for a person (T-107): its turn
/// is over (`Idle{EndTurn}`, so the card would wear the done mark) and the
/// hand is up, which is exactly the pair the tool exists to tell apart.
fn fixture_raised() -> Board {
    let mut b = fixture(false);
    if let Some(t) = b.tickets.iter_mut().find(|t| t.id == ulid_n(5)) {
        t.raised = Some(mesimon_core::board::Raised {
            at: "@100".into(),
            by: "agent:00000000-0000-0000-0000-000000000000".into(),
            reason: "Auth0 or cookie?".into(),
        });
    }
    b
}

/// Install a deterministic diff view on ticket 3 and enter `Screen::Diff`.
/// Seeded directly: FakeTransport's snapshot carries no worktrees, so the
/// TICKET target's `v` entry path dead-ends in tests. (The checkout target's
/// does not — see `board_v_opens_the_checkout_diff`.)
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
        merged_in: String::new(),
        merged_oid: String::new(),
        conflict: false,
        ahead: 2,
        needs_rebase: false,
        detail: None,
        path: Some("/wt/T-3-fix-osc-11-detection".into()),
        repos: vec![],
    }];
    app.diff = Some(crate::app::DiffState {
        commits: false,
        target: mesimon_core::command::DiffTarget::Ticket { id: ulid_n(3) },
        rail_idx: 0,
        branch: "msmn/T-3-fix-osc-11-detection".into(),
        base_oid: "a1b2c3d4".repeat(5),
        branch_oid: "b".repeat(40),
        files,
        file_idx: 0,
        pager: crate::app::Pager::default(),
        marquee: std::cell::Cell::new(None),
        density: 3,
        cache,
        z_armed: false,
        swap: false,
        worktree_present: true,
    });
    app.screen = Screen::Diff;
}

/// The same screen on the board's own checkout (T-221): no ticket, no
/// worktree, and an untracked row that opens like any other add.
fn install_checkout_diff(app: &mut App) {
    use mesimon_core::diff::{FileDiff, FileEntry, Hunk, HunkLine, Render, Sign};
    let entry = |path: &str, status: &str, adds: u32, dels: u32, untracked: bool| FileEntry {
        path: path.into(),
        old_path: None,
        status: status.into(),
        old_mode: if untracked { "000000".into() } else { "100644".into() },
        new_mode: "100644".into(),
        old_blob: if untracked { String::new() } else { "a".repeat(40) },
        new_blob: "0".repeat(40),
        adds: Some(adds),
        dels: Some(dels),
        // Everything in a checkout diff is uncommitted; only `untracked` says
        // anything the row does not already say.
        dirty: true,
        untracked,
    };
    let files = vec![
        entry("crates/mesimon-tui/src/ui/diff.rs", "M", 12, 3, false),
        entry("AGENTS.md", "A", 2, 0, true),
    ];
    let line =
        |sign, old_ln, new_ln, text: &str| HunkLine { sign, old_ln, new_ln, text: text.into() };
    let mut cache = std::collections::HashMap::new();
    cache.insert(
        "crates/mesimon-tui/src/ui/diff.rs".to_string(),
        FileDiff {
            path: "crates/mesimon-tui/src/ui/diff.rs".into(),
            old_path: None,
            render: Render::Text,
            hunks: vec![Hunk {
                old_start: 70,
                old_len: 3,
                new_start: 70,
                new_len: 4,
                header: " fn draw".into(),
                lines: vec![
                    line(Sign::Ctx, Some(70), Some(70), "    let n = d.files.len();"),
                    line(Sign::Del, Some(71), None, "    let base8 = d.base_oid;"),
                    line(Sign::Add, None, Some(71), "    let against = \"uncommitted\";"),
                ],
            }],
        },
    );
    cache.insert(
        "AGENTS.md".to_string(),
        FileDiff {
            path: "AGENTS.md".into(),
            old_path: None,
            render: Render::Text,
            hunks: vec![Hunk {
                old_start: 0,
                old_len: 0,
                new_start: 1,
                new_len: 2,
                header: String::new(),
                lines: vec![
                    line(Sign::Add, None, Some(1), "# Agents"),
                    line(Sign::Add, None, Some(2), "This repo is driven by mesimon."),
                ],
            }],
        },
    );
    app.diff = Some(crate::app::DiffState {
        commits: false,
        target: mesimon_core::command::DiffTarget::Checkout,
        rail_idx: 0,
        branch: "main".into(),
        base_oid: "c1d2e3f4".repeat(5),
        branch_oid: String::new(),
        files,
        file_idx: 0,
        pager: crate::app::Pager::default(),
        marquee: std::cell::Cell::new(None),
        density: 3,
        cache,
        z_armed: false,
        swap: false,
        worktree_present: true,
    });
    app.screen = Screen::Diff;
}

/// Three deterministic releases on the RELEASES screen, `v0.9.0-alpha.3`
/// being "this build". Seeded directly rather than parsed from the real
/// `CHANGELOG.md`, which changes every release and would drift every golden
/// with it; `test_real_changelog_reads_lawfully` walks the real one.
fn install_releases(app: &mut App) {
    use mesimon_core::relnotes::Release;
    let rel = |tag: &str, date: &str, body: &str| Release {
        tag: tag.into(),
        date: date.into(),
        body: body.into(),
    };
    let releases = vec![
        rel(
            "v0.9.0-alpha.3",
            "2026-09-04",
            "- **The board reads its own release notes.** The Esc menu's `Release notes` row \
             opens the changelog the binary was built with, newest first, one band per \
             release, with `this build` on the entry you are running.\n\n- **A card's age \
             is time in column.** It was the newest session state change, which reset on \
             every hook — so a ticket that had sat in review for a week read `2m` after a \
             single prompt.\n\n- **Smaller.** `n N` step between releases; `{ }` page; a \
             long entry keeps its band pinned while its notes scroll under it.",
        ),
        rel(
            "v0.9.0-alpha.2",
            "2026-09-02",
            "- **Shift+Enter asks the agent from the board.** On a ticket with a live claude \
             pane it opens a one-line field under the card; Enter sends and stays.\n\n\
             - **Paths in a shell pane keep their shape:**\n\n  ```\n  mesimon exec --env \
             <file> -- claude --settings <hooks>\n  ```\n\n  and the pid stays the agent's.",
        ),
        rel(
            "v0.9.0-alpha.1",
            "2026-09-01",
            "- **First alpha.** A board, a daemon, a private tmux. Alphas can and will change \
             state-file formats; when they do, the old file is preserved, never overwritten.",
        ),
    ];
    app.releases = Some(crate::app::ReleasesState::new(releases, "v0.9.0-alpha.3"));
    app.screen = Screen::Releases;
}

// ---- goldens ---------------------------------------------------------------

#[test]
fn golden_calm_board_120() {
    let mut app = app_graphite(fixture(false));
    app.cursor_col = 1;
    app.cursor_row = Some(0);
    golden("board_calm_120x30", &render(&app, 120, 30));
}

#[test]
fn golden_waiting_board_120() {
    let mut app = app_graphite(fixture(true));
    app.cursor_col = 1;
    app.cursor_row = Some(0);
    golden("board_waiting_120x30", &render(&app, 120, 30));
}

/// A raised hand (T-107): the mark replaces the done mark on the card, and
/// the cursor card carries the agent's own sentence in the context row the
/// snooze preset and the owed row share.
#[test]
fn golden_raised_board_120() {
    let mut app = app_graphite(fixture_raised());
    app.cursor_col = 2;
    app.cursor_row = Some(0);
    let lines = render(&app, 120, 30);
    assert!(lines[0].contains("!1"), "the header counts it: {}", lines[0]);
    assert!(
        lines.iter().any(|l| l.contains("Auth0 or cookie?")),
        "the cursor card says why: {lines:?}"
    );
    golden("board_raised_120x30", &lines);
}

/// The same hand from the ticket page: the state row says who asked, when,
/// and what they asked. The page is where it is answered, and the mark is
/// lowered on the way OUT, so the row is still here while it is being read.
#[test]
fn golden_ticket_raised_120() {
    let mut app = app_graphite(fixture_raised());
    app.screen = Screen::Ticket { ticket: ulid_n(5), rail_idx: 0 };
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("agent asked") && l.contains("Auth0")), "{lines:?}");
    golden("ticket_raised_120x30", &lines);
}

/// An Esc-interrupted agent on an otherwise sessionless card: the card
/// carries `⊘` where it used to carry nothing (2026-09-04).
#[test]
fn golden_interrupted_board_120() {
    let mut b = fixture(false);
    b.sessions.push(session(
        21,
        ulid_n(2),
        SessionKind::Claude,
        SessionState::Idle { stop_reason: StopReason::Interrupted },
    ));
    let mut app = app_graphite(b);
    app.cursor_col = 1;
    app.cursor_row = Some(0);
    golden("board_interrupted_120x30", &render(&app, 120, 30));
}

#[test]
fn golden_move_ghost_120() {
    let mut app = app_graphite(fixture(false));
    app.mode = Mode::Move { ticket: ulid_n(3), col: 2, idx: 0, grab: '>', home: (1, 0) };
    golden("board_move_120x30", &render(&app, 120, 30));
}

#[test]
fn golden_empty_board_120() {
    let mut b = Board::default();
    for (i, name) in ["todo", "in progress", "review", "done"].iter().enumerate() {
        b.columns.push(Column::new(*name, format!("{i}")));
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

/// The armed snooze chord (T-74): the footer is the chord's scope naming
/// the ring and the pick, and the card opens with the preset on a row of
/// its own. A second `z` walks the ring — the row and the footer move
/// together.
#[test]
fn golden_snooze_armed_120() {
    let mut app = app_graphite(fixture(false));
    app.cursor_col = 0;
    app.handle_key(
        ratatui::crossterm::event::KeyCode::Char('z'),
        ratatui::crossterm::event::KeyModifiers::NONE,
    )
    .unwrap();
    let lines = render(&app, 120, 30);
    assert!(
        lines.last().is_some_and(|l| l.contains("enter snooze 1h")),
        "the armed state must name the pick: {:?}",
        lines.last()
    );
    assert!(
        lines.iter().any(|l| l.contains("snooze 1h")),
        "the card must carry the preset while armed"
    );
    golden("board_snooze_armed_120x30", &lines);
    app.handle_key(
        ratatui::crossterm::event::KeyCode::Char('z'),
        ratatui::crossterm::event::KeyModifiers::NONE,
    )
    .unwrap();
    let lines = render(&app, 120, 30);
    assert!(lines.last().is_some_and(|l| l.contains("enter snooze 4h")), "{:?}", lines.last());
    assert!(lines.iter().any(|l| l.contains("snooze 4h")));
}

/// The archived dialog with a snoozed row: it says when the ticket comes
/// back where a plain archive says how long it has been gone.
#[test]
fn golden_archived_snoozed_120() {
    let mut app = app_graphite(fixture_snoozed());
    app.mode = Mode::Archived { idx: 0 };
    let lines = render(&app, 120, 30);
    assert!(
        lines.iter().any(|l| l.contains("wakes in >1y")),
        "the snoozed row must say when it wakes"
    );
    golden("archived_snoozed_120x30", &lines);
    // And the ticket page says the same beside its archived badge.
    app.mode = Mode::Normal;
    app.screen = Screen::Ticket { ticket: ulid_n(7), rail_idx: 0 };
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("archived ∙ wakes in >1y")), "{lines:#?}");
}

/// The Esc menu: the board-wide actions, which deliberately have no keys.
#[test]
fn golden_menu_120() {
    let mut app = app_graphite(fixture_archived());
    app.mode = Mode::Menu { idx: 0 };
    golden("menu_120x30", &render(&app, 120, 30));
}

/// The settings list over the board: the three preferences, the theme row
/// naming the one worn, no suggestion mark anywhere, `esc back` in its edge.
#[test]
fn golden_settings_120() {
    let mut app = app_graphite(fixture_archived());
    app.mode = Mode::Settings { idx: 0 };
    golden("settings_120x30", &render(&app, 120, 30));
}

#[test]
fn golden_codex_provider_and_existing_claude() {
    let mut board = fixture(false);
    board.agent_provider = mesimon_core::board::AgentProvider::Codex;
    let mut app = app_graphite(board);
    app.settings_section = mesimon_core::keymap::SettingsSection::Agents;
    app.mode = Mode::Settings { idx: 0 };
    golden("settings_codex_60x20", &render(&app, 60, 20));
    app.mode = Mode::Normal;
    app.screen = Screen::Ticket { ticket: ulid_n(1), rail_idx: 0 };
    let rows = render(&app, 120, 30);
    assert!(rows.iter().any(|r| r.contains("+ agent session")));
    assert!(rows.iter().any(|r| r.contains("start agent")));
    golden("ticket_new_codex_120x30", &rows);
    app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    let rows = render(&app, 120, 30);
    assert!(rows.iter().any(|r| r.contains("claude")));
    assert!(!rows.iter().any(|r| r.contains("+ agent session")));
}

#[test]
fn golden_settings_groups_fit_short_and_wide_terminals() {
    use mesimon_core::keymap::SettingsSection;
    for (section, name) in [
        (SettingsSection::Appearance, "appearance"),
        (SettingsSection::Behaviour, "behaviour"),
        (SettingsSection::Agents, "agents"),
    ] {
        let mut app = app_graphite(fixture_archived());
        app.settings_section = section;
        app.mode = Mode::Settings { idx: 0 };
        for (w, h) in [(60, 20), (120, 30)] {
            let rows = render(&app, w, h);
            assert!(!rows.iter().any(|r| r.contains("agent replies")));
            for item in mesimon_core::keymap::settings_items(&app.ctx()) {
                assert!(rows.iter().any(|r| r.contains(&(item.label)(&app.ctx()))), "{rows:?}");
            }
            golden(&format!("settings_{name}_{w}x{h}"), &rows);
        }
    }
}

/// The notifications list, one level under Settings (T-282), turned ON so
/// every row draws: the master switch, the two moments, what a banner is
/// allowed to SAY (T-292), the two sounds and the two exceptions.
/// `NOTIFICATIONS` in the frame's top edge and the list's own keys in its
/// bottom one.
#[test]
fn golden_notifications_120() {
    let mut app = app_graphite(fixture_archived());
    app.seed_pref(|p| p.notify = true);
    app.mode = Mode::Notifications { idx: 0 };
    golden("notifications_120x30", &render(&app, 120, 30));
}

/// The Settings dialog in board scope (T-361): `THIS BOARD` in the title,
/// the auto-merge row set for this board with the machine's value quoted
/// in its detail, the keep-awake row inherited, and `b machine` in the
/// tail.
#[test]
fn golden_settings_behaviour_board_120() {
    let mut app = app_graphite(fixture_archived());
    app.settings_section = mesimon_core::keymap::SettingsSection::Behaviour;
    app.settings_board_scope = true;
    app.board_prefs.set_bool(mesimon_core::prefs::PrefKey::MergeTrain, true);
    app.resolve_prefs();
    app.mode = Mode::Settings { idx: app.settings_row(mesimon_core::keymap::Verb::MergeTrain) };
    golden("settings_behaviour_board_120x30", &render(&app, 120, 30));
}

/// Board scope under Appearance, on the status line row: a machine-only
/// key reads `(machine)` first.
#[test]
fn golden_settings_appearance_board_60() {
    let mut app = app_graphite(fixture_archived());
    app.settings_section = mesimon_core::keymap::SettingsSection::Appearance;
    app.settings_board_scope = true;
    app.mode = Mode::Settings { idx: app.settings_row(mesimon_core::keymap::Verb::StatusLine) };
    golden("settings_appearance_board_60x20", &render(&app, 60, 20));
}

/// The notifications list in board scope: the master switch set for this
/// board over a machine that has them off.
#[test]
fn golden_notifications_board_120() {
    let mut app = app_graphite(fixture_archived());
    app.settings_board_scope = true;
    app.board_prefs.set_bool(mesimon_core::prefs::PrefKey::Notify, true);
    app.resolve_prefs();
    app.mode = Mode::Notifications { idx: 0 };
    golden("notifications_board_120x30", &render(&app, 120, 30));
}

/// The agent-prompt list (T-353), one level under Settings > Agents: the
/// four sentences mesimon types into an agent's box, each row saying whose
/// words stand there and the selected one's detail saying when it is sent
/// and what it says. `AGENT PROMPTS` in the frame's top edge.
#[test]
fn golden_agent_prompts_120() {
    let mut app = app_graphite(fixture_archived());
    app.mode = Mode::Prompts { idx: 0, editing: None };
    golden("agent_prompts_120x30", &render(&app, 120, 30));
}

/// The same list with the rebase template open as a field: the row's own
/// name leads it, the cursor sits in the text, and the detail teaches the
/// placeholders and the way back to mesimon's words.
#[test]
fn golden_agent_prompt_editing_120() {
    let mut app = app_graphite(fixture_archived());
    app.mode = Mode::Prompts { idx: 0, editing: None };
    app.handle_key(
        ratatui::crossterm::event::KeyCode::Enter,
        ratatui::crossterm::event::KeyModifiers::NONE,
    )
    .expect("enter");
    assert!(matches!(app.mode, Mode::Prompts { editing: Some(_), .. }), "{:?}", app.mode);
    golden("agent_prompt_editing_120x30", &render(&app, 120, 30));
}

/// The sharing dialog signed out (T-334, one dialog since T-335): the YOU
/// section alone — the two fields unset and `Sign in` saying what it
/// needs — with `SHARING` in the frame's top edge.
#[test]
fn golden_sharing_signed_out_120() {
    let mut app = app_graphite(fixture_archived());
    app.mode = Mode::Sharing { idx: 1, editing: None, armed: false };
    golden("sharing_signed_out_120x30", &render(&app, 120, 30));
}

/// The same with the relay row open as a field, an address half typed:
/// the row's name leads the field, the cursor sits at its end, and the
/// detail teaches the address form.
#[test]
fn golden_sharing_editing_120() {
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    let mut app = app_graphite(fixture_archived());
    app.team_name_draft = "Dana".into();
    app.mode = Mode::Sharing { idx: 1, editing: None, armed: false };
    app.handle_key(KeyCode::Enter, KeyModifiers::NONE).expect("enter");
    for c in "relay.example".chars() {
        app.handle_key(KeyCode::Char(c), KeyModifiers::NONE).expect("type");
    }
    assert!(matches!(app.mode, Mode::Sharing { editing: Some(_), .. }), "{:?}", app.mode);
    golden("sharing_editing_120x30", &render(&app, 120, 30));
}

/// Signed in, the board not yet published: the identity, then THIS BOARD
/// with the publish row counting what goes out and the notes switch, then
/// BOARDS with the way onto one. The cursor is on the publish row, where
/// the dialog opens.
#[test]
fn golden_sharing_publish_120() {
    let mut app = app_graphite(fixture_archived());
    app.team.device = crate::app::shared_team_fixture().device;
    app.seed_team_drafts_for_test();
    let publish = app.sharing_rows().iter().position(|r| *r == SharingRow::Publish).expect("row");
    app.mode = Mode::Sharing { idx: publish, editing: None, armed: false };
    golden("sharing_publish_120x30", &render(&app, 120, 30));
}

/// Once shared, as the owner: the two invite rows, the code that is out,
/// four members in four states — the owner, a contributor, one waiting
/// for a key, one removed — and the way to stop; the sync word and the
/// drafts waiting in the frame's title. The cursor is on the contributor,
/// whose detail says what Enter does to them.
#[test]
fn golden_sharing_members_120() {
    let mut app = app_graphite(fixture_archived());
    app.team = crate::app::shared_team_fixture();
    app.seed_team_drafts_for_test();
    let dana = app
        .sharing_rows()
        .iter()
        .position(|r| matches!(r, SharingRow::Member(d) if d.starts_with("dd")))
        .expect("dana");
    app.mode = Mode::Sharing { idx: dana, editing: None, armed: false };
    let rows = render(&app, 120, 30);
    assert!(rows.iter().any(|r| r.contains("SHARING ∙ OFFLINE ∙ 2 DRAFTS")), "{rows:?}");
    golden("sharing_members_120x30", &rows);
}

/// A joined board as a contributor (T-335): the members and `Leave this
/// board`, then the boards — this one (open now), one Dana can open, one
/// she owns from a checkout elsewhere. The cursor is on the board Enter
/// would open.
#[test]
fn golden_sharing_joined_120() {
    let mut app = app_graphite(fixture_archived());
    app.team = crate::app::joined_team_fixture();
    app.seed_team_drafts_for_test();
    let rows = app.sharing_rows();
    let join = rows.iter().position(|r| *r == SharingRow::Join).expect("join");
    app.mode = Mode::Sharing { idx: join + 2, editing: None, armed: false };
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|r| r.contains("Sam's board ∙ viewer")), "{lines:?}");
    golden("sharing_joined_120x30", &lines);
}

/// The join row open as a field, a code half pasted: `Code: ` leads the
/// field and the edge reads the text field's keys.
#[test]
fn golden_sharing_joining_120() {
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    let mut app = app_graphite(fixture_archived());
    app.team = crate::app::joined_team_fixture();
    app.seed_team_drafts_for_test();
    let join = app.sharing_rows().iter().position(|r| *r == SharingRow::Join).expect("join");
    app.mode = Mode::Sharing { idx: join, editing: None, armed: false };
    app.handle_key(KeyCode::Enter, KeyModifiers::NONE).expect("enter");
    for c in "7A3K-M9Q2-XB4D".chars() {
        app.handle_key(KeyCode::Char(c), KeyModifiers::NONE).expect("type");
    }
    assert!(matches!(app.mode, Mode::Sharing { editing: Some(_), .. }), "{:?}", app.mode);
    golden("sharing_joining_120x30", &render(&app, 120, 30));
}

/// Off, the list is a SINGLE row: four settings for a thing that is not
/// happening are four rows saying nothing.
#[test]
fn the_notifications_list_is_one_row_while_it_is_off() {
    let mut app = app_graphite(fixture_archived());
    app.mode = Mode::Notifications { idx: 0 };
    let lines = render(&app, 120, 30);
    assert!(
        lines.iter().any(|l| l.contains("Notifications: off")),
        "the master switch is always there: {lines:#?}"
    );
    for absent in ["Sound when an agent needs you", "Also when a turn finishes", "focused"] {
        assert!(
            !lines.iter().any(|l| l.contains(absent)),
            "{absent} is offered for a thing that is off: {lines:#?}"
        );
    }
}

/// A subtitle wider than the dialog reveals itself on the selected row, the
/// way an overlong card title and an overlong rail name do. A preference's
/// detail is where it says what it will do, so the half past the `~` is the
/// half worth reading.
#[test]
fn the_settings_subtitle_marquees() {
    let mut app = app_graphite(fixture_archived());
    // The merge train's row: the longest detail in the list, and off by
    // default, which is the sentence that explains the standing consent.
    // The first row of BEHAVIOUR, which is where the train sits now; keep
    // awake is under it, and neither index moves the other.
    app.settings_section = mesimon_core::keymap::SettingsSection::Behaviour;
    app.mode = Mode::Settings { idx: 0 };
    let row = |lines: &[String]| -> String {
        lines
            .iter()
            .find(|l| l.contains("merges and asks to rebase"))
            .unwrap_or_else(|| panic!("the merge train's subtitle: {lines:#?}"))
            .clone()
    };
    let resting = row(&render(&app, 120, 30));
    assert!(resting.contains("mesimon merges and asks"), "the pass starts at the start: {resting}");
    assert!(resting.contains('~'), "and it is cut, which is what the walk repairs: {resting}");
    // Past the opening hold: the draw armed the clock, so date it into the
    // past rather than sleeping through six steps of it.
    let (key, _) = app.menu_marquee.get().expect("an overflowing subtitle arms the clock");
    app.menu_marquee
        .set(Some((key, std::time::Instant::now() - std::time::Duration::from_millis(2000))));
    let walked = row(&render(&app, 120, 30));
    assert!(!walked.contains("mesimon merges"), "the words have moved: {walked}");
    assert!(!walked.contains('~'), "a walking marquee hard-clips: {walked}");
    assert!(walked.contains("board is q"), "and it reveals what the cut hid: {walked}");
    // An unselected row is still cut: one sentence moves, the list is quiet.
    app.mode = Mode::Settings { idx: 1 };
    let quiet = row(&render(&app, 120, 30));
    assert!(quiet.contains("mesimon merges and asks"), "{quiet}");
    assert!(quiet.contains('~'), "{quiet}");
}

/// The theme picker over the board: six rows, the flavor's ground at the
/// right edge, and the saved slots named in words on their rows.
#[test]
fn golden_theme_picker_120() {
    let mut app = app_graphite(fixture_archived());
    app.mode = Mode::Theme { idx: 0 };
    golden("theme_picker_120x30", &render(&app, 120, 30));
}

/// The picker in board scope (T-361): the inherit row first, naming the
/// machine's pick, then the six flavors.
#[test]
fn golden_theme_picker_board_120() {
    let mut app = app_graphite(fixture_archived());
    app.settings_board_scope = true;
    app.mode = Mode::Theme { idx: 0 };
    golden("theme_picker_board_120x30", &render(&app, 120, 30));
}

/// The agent-brief dialog: the reach named, the text verbatim on the elevated
/// surface, and the four answers in the bottom edge. This golden is the
/// promise the feature makes — what is on the screen is what every claude
/// mesimon starts will be told.
#[test]
fn golden_brief_120() {
    let mut app = app_graphite(fixture_archived());
    offer_brief(&mut app);
    app.mode = Mode::Brief { from_settings: false };
    golden("brief_120x30", &render(&app, 120, 30));
}

/// A board whose CLAUDE.md does not carry the line, so the offer stands.
fn offer_brief(app: &mut App) {
    app.claude_md = mesimon_core::command::ClaudeMdStatus {
        path: "/repo/kanban-tui/CLAUDE.md".into(),
        sampled: true,
        present: false,
        offer: true,
    };
}

/// The text reaches the screen unwrapped and unabridged, under a line that
/// says who gets it and that nothing is written. A dialog that reflowed the
/// text would be showing something other than what Enter sends, which is the
/// one thing this surface may not do.
#[test]
fn the_dialog_shows_the_brief_verbatim() {
    let mut app = app_graphite(fixture_archived());
    offer_brief(&mut app);
    app.mode = Mode::Brief { from_settings: false };
    let lines = render(&app, 120, 30);
    for want in mesimon_core::brief::TEXT.lines().filter(|l| !l.trim().is_empty()) {
        assert!(
            lines.iter().any(|l| l.contains(want)),
            "the brief line `{want}` is not on screen whole:\n{}",
            lines.join("\n")
        );
    }
    // The consent sentence: where it goes, and that it goes nowhere else.
    assert!(lines.iter().any(|l| l.contains("system prompt") && l.contains("mesimon starts")));
    assert!(lines.iter().any(|l| l.contains("nothing written to disk ∙ Settings turns it off")));
}

/// Every answer is taught in the frame's own edge, so the dialog says what it
/// can do without the footer repeating it.
#[test]
fn the_dialog_teaches_its_four_answers() {
    let mut app = app_graphite(fixture_archived());
    offer_brief(&mut app);
    app.mode = Mode::Brief { from_settings: false };
    let lines = render(&app, 120, 30);
    let edge = lines
        .iter()
        .find(|l| l.contains("turn on"))
        .unwrap_or_else(|| panic!("the bottom edge: {lines:#?}"));
    for key in ["enter turn on", "c copy", "i never ask again", "esc not now"] {
        assert!(edge.contains(key), "`{key}` missing from the edge: {edge}");
    }
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

/// The chip reads the daemon's answer and nothing else (T-247): the clauses
/// live in `ClaudeMdStatus::offered`, tested beside it in core. Here, an
/// unsampled snapshot — an older daemon, a first sample still in flight —
/// offers nothing, and the answered one is taken as it comes.
#[test]
fn the_offer_is_the_daemons_answer() {
    let mut app = app_graphite(fixture_archived());
    use mesimon_core::keymap::{is_suggested, Verb};
    let offered = |a: &App| is_suggested(Verb::BriefOffer, &a.ctx());

    assert!(!offered(&app), "an unsampled board offers nothing");
    offer_brief(&mut app);
    assert!(offered(&app), "the daemon said so");
    // The TUI does not re-derive the answer from the switches it can see:
    // the snapshot after Enter or `i` is what withdraws the chip.
    app.claude_md.offer = false;
    assert!(!offered(&app), "and the daemon's no is a no");
}

/// The offer is a chip AND the menu row it points at, and they are the same
/// availability — the whole suggestion language in one assertion.
#[test]
fn the_offer_reaches_the_menu_and_the_header() {
    let mut app = app_graphite(fixture_archived());
    offer_brief(&mut app);
    let header = render(&app, 120, 30);
    assert!(
        header.iter().any(|l| l.contains("tell agents to read the ticket (esc)")),
        "{}",
        header.join("\n")
    );
    app.mode = Mode::Menu { idx: 0 };
    let menu = render(&app, 120, 30);
    assert!(menu.iter().any(|l| l.contains("Tell agents to read the ticket")), "{menu:#?}");
    // Wearing the same mark the chip does — read from the glyph table, so
    // the two can never be checked against a stale transcription.
    let mark = crate::glyphs::suggest_mark(app.theme.glyph_tier());
    let row = menu.iter().find(|l| l.contains("Tell agents to read")).expect("the row");
    assert!(row.contains(mark), "a suggested row wears `{mark}`: {row}");
    assert!(header.iter().any(|l| l.contains(mark)), "and so does the chip");
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
    assert!(head.ends_with("◦ sleep 3 agents (X ∙ esc)"), "{head:?}");
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
        head.ends_with("◦ sleep 2 agents (X ∙ esc)"),
        "sleep outranks archive at any size: {head:?}"
    );
    // And the row it points at says so in words rather than claiming ~0.0GiB.
    app.mode = Mode::Menu { idx: 0 };
    let lines = render(&app, 120, 30);
    let top = lines.iter().find(|l| l.contains("Sleep 2 agents")).expect("the sleep row leads");
    // From the dialog's left edge: the board is still drawn beside the frame.
    let row = top.split_once('│').map(|(_, r)| r).unwrap_or(top);
    assert!(unframed(row).starts_with('◦'), "the chip's row is marked: {top:?}");
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
    assert!(marked[1].contains("Sleep 3 agents on finished tickets"), "{marked:?}");
    assert!(marked[2].contains("Archive 2 finished tickets"), "{marked:?}");
    // The chip named the first of them and nothing else.
    let head = &render(&app, 120, 30)[0];
    assert!(head.contains("◦ update ready"), "{head:?}");
    assert!(!head.contains("Sleep"), "the menu spells out what the chip does not");
    // Marked rows come before the unmarked ones.
    let first_plain =
        lines.iter().position(|l| l.contains("External sessions")).expect("the plain rows follow");
    let last_marked =
        lines.iter().rposition(|l| unframed(l).starts_with('◦')).expect("marked rows");
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
    // overlay, because both read the same predicate. An empty column IS its
    // header (T-117), so the column's own verbs are what is offered instead.
    for absent in ["move card", "archive", "start agent", "delete ticket", "ticket page"] {
        assert!(
            !lines.iter().any(|l| l.contains(absent)),
            "{absent:?} offered with an empty column selected"
        );
    }
    assert!(lines.iter().any(|l| l.contains("new ticket")), "creating must always be offered");
    for present in ["rename column", "move column", "column settings", "delete column"] {
        assert!(lines.iter().any(|l| l.contains(present)), "{present:?} is the header's");
    }
    golden("help_empty_column_120x30", &lines);
}

/// The cursor on a column header (T-117): the header wears the cursor bar,
/// no card is selected, and the footer offers the column's verbs.
#[test]
fn golden_board_header_120() {
    let mut app = app_graphite(fixture(false));
    app.cursor_col = 1;
    app.cursor_row = None;
    let lines = render(&app, 120, 30);
    let foot = lines.last().unwrap();
    for present in ["column settings", "rename column", "move column"] {
        assert!(foot.contains(present), "{present:?} missing from {foot:?}");
    }
    assert!(!foot.contains("describe"), "{foot:?}");
    golden("board_header_120x30", &lines);
}

/// `r` on a header: the name edited in place in the header row, the badges
/// standing down, the mode chip saying RENAME.
#[test]
fn golden_board_header_rename_120() {
    let mut app = app_graphite(fixture(false));
    app.cursor_col = 1;
    app.cursor_row = None;
    press(&mut app, 'r');
    assert!(matches!(
        &app.mode,
        Mode::Input { purpose: InputPurpose::RenameColumn { name }, .. } if name == "in progress"
    ));
    for c in " now".chars() {
        press(&mut app, c);
    }
    let lines = render(&app, 120, 30);
    assert!(lines[2].contains("in progress now"), "{:?}", lines[2]);
    golden("board_header_rename_120x30", &lines);
}

/// A column that does something wears the one mark after its count.
#[test]
fn golden_board_header_automated_120() {
    let mut board = fixture(false);
    board.columns[0].settings.auto_run = true;
    board.columns[1].settings.on_done = Some("review".into());
    let app = app_graphite(board);
    let lines = render(&app, 120, 30);
    assert!(lines[2].contains('→'), "{:?}", lines[2]);
    golden("board_header_automated_120x30", &lines);
}

/// The column settings dialog over the board (T-117): thirteen rows, the
/// selected row's detail under them, `esc back` in its edge.
#[test]
fn golden_column_settings_120() {
    let mut app = app_graphite(fixture(false));
    app.cursor_col = 1;
    app.cursor_row = None;
    app.handle_key(
        ratatui::crossterm::event::KeyCode::Enter,
        ratatui::crossterm::event::KeyModifiers::NONE,
    )
    .unwrap();
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("COLUMN ∙ IN PROGRESS")), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("Agent behaviour")), "{lines:?}");
    golden("column_settings_120x30", &lines);
}

/// Agent options live one level below the column's own settings.
#[test]
fn golden_column_agent_behaviour_120() {
    let mut app = app_graphite(fixture(false));
    app.cursor_col = 1;
    app.cursor_row = None;
    app.handle_key(
        ratatui::crossterm::event::KeyCode::Enter,
        ratatui::crossterm::event::KeyModifiers::NONE,
    )
    .unwrap();
    let row = mesimon_core::keymap::column_items(&app.ctx())
        .iter()
        .position(|m| m.verb == mesimon_core::keymap::Verb::ColumnAgentBehaviour)
        .unwrap();
    if let Mode::ColumnSettings { idx, .. } = &mut app.mode {
        *idx = row;
    }
    app.handle_key(
        ratatui::crossterm::event::KeyCode::Enter,
        ratatui::crossterm::event::KeyModifiers::NONE,
    )
    .unwrap();
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("Mode: inherit (auto)")), "{lines:?}");
    golden("column_agent_behaviour_120x30", &lines);
}

#[test]
fn golden_codex_column_agent_behaviour() {
    let mut board = fixture(false);
    board.agent_provider = mesimon_core::board::AgentProvider::Codex;
    board.columns[1].settings.codex_sandbox = mesimon_core::board::CodexSandbox::WorkspaceWrite;
    board.columns[1].settings.codex_approval = mesimon_core::board::CodexApproval::OnRequest;
    let mut app = app_graphite(board);
    app.cursor_col = 1;
    app.cursor_row = None;
    app.handle_key(
        ratatui::crossterm::event::KeyCode::Enter,
        ratatui::crossterm::event::KeyModifiers::NONE,
    )
    .unwrap();
    let row = mesimon_core::keymap::column_items(&app.ctx())
        .iter()
        .position(|m| m.verb == mesimon_core::keymap::Verb::ColumnAgentBehaviour)
        .unwrap();
    if let Mode::ColumnSettings { idx, .. } = &mut app.mode {
        *idx = row;
    }
    app.handle_key(
        ratatui::crossterm::event::KeyCode::Enter,
        ratatui::crossterm::event::KeyModifiers::NONE,
    )
    .unwrap();
    for (width, height, name) in
        [(120, 30, "column_codex_agent_120x30"), (60, 20, "column_codex_agent_60x20")]
    {
        let lines = render(&app, width, height);
        assert!(lines.iter().any(|l| l.contains("Codex sandbox: workspace write")), "{lines:?}");
        assert!(lines.iter().any(|l| l.contains("Codex approvals: on request")), "{lines:?}");
        assert!(!lines.iter().any(|l| l.contains("Mode: inherit")), "{lines:?}");
        golden(name, &lines);
    }
}

/// `O`: the dialog on a column that does not exist yet — the Name row alone.
#[test]
fn golden_column_add_120() {
    let mut app = app_graphite(fixture(false));
    press(&mut app, 'O');
    for c in "qa".chars() {
        press(&mut app, c);
    }
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("NEW COLUMN")), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("Name: qa")), "{lines:?}");
    golden("column_add_120x30", &lines);
}

/// The `?` overlay on a column header: the column's verbs and none of a
/// ticket's.
#[test]
fn golden_help_header_120() {
    let mut app = app_graphite(fixture(false));
    app.cursor_col = 1;
    app.cursor_row = None;
    app.help = true;
    let lines = render(&app, 120, 30);
    for present in
        ["column settings", "rename column", "move column", "delete column", "new column"]
    {
        assert!(lines.iter().any(|l| l.contains(present)), "{present:?}");
    }
    for absent in ["describe", "start agent", "archive", "snooze"] {
        assert!(!lines.iter().any(|l| l.contains(absent)), "{absent:?}");
    }
    golden("help_header_120x30", &lines);
}

/// The `?` overlay on the board's own top row (T-305): the three keys the
/// row has and nothing the column or a card owns.
#[test]
fn golden_help_header_bar_120() {
    let mut app = app_graphite(fixture(false));
    app.cursor_col = 1;
    app.git = git_state("main", 2, 1, 3);
    press(&mut app, 'k');
    press(&mut app, 'k');
    app.help = true;
    let lines = render(&app, 120, 30);
    for present in ["back to the board", "diff", "back"] {
        assert!(lines.iter().any(|l| l.contains(present)), "{present:?}");
    }
    for absent in ["column settings", "rename column", "new ticket", "menu"] {
        assert!(!lines.iter().any(|l| l.contains(absent)), "{absent:?}");
    }
    golden("help_header_bar_120x30", &lines);
}

/// The composer on a column with a workspace default says so on its row.
#[test]
fn golden_composer_column_default_120() {
    let mut board = fixture(false);
    board.columns[0].settings.workspace = Some(mesimon_core::board::WorkspaceStrategy::Worktree);
    let mut app = app_graphite(board);
    press(&mut app, 'o');
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("worktree (default)")), "{lines:?}");
    golden("composer_column_default_120x30", &lines);
}

/// A column pinned collapsed is a spine at 120 columns, where every column
/// would otherwise expand; the cursor entering it expands it.
#[test]
fn golden_board_pinned_120() {
    let mut board = fixture(false);
    board.columns[3].settings.collapsed = true;
    let mut app = app_graphite(board);
    let lines = render(&app, 120, 30);
    assert!(!lines[2].contains("DONE"), "pinned: {:?}", lines[2]);
    golden("board_pinned_120x30", &lines);
    app.cursor_col = 3;
    let lines = render(&app, 120, 30);
    assert!(lines[2].contains("DONE"), "the cursor expands it: {:?}", lines[2]);
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

/// The ticket's `!` terminal on the rail (T-366): a ghost row under the
/// sessions, its pane in the preview zone, and the adoption's two presses.
#[test]
fn golden_ticket_terminal_120() {
    let mut app = app_graphite(fixture(true));
    let t3 = ulid_n(3);
    app.terminals.push(mesimon_core::command::TerminalItem { ticket: Some(t3), foreground: None });
    // Sessions first: the ghost is the third row, after the claude and the shell.
    app.screen = Screen::Ticket { ticket: t3, rail_idx: 2 };
    assert!(matches!(app.rail_row(), Some(crate::app::RailRow::Terminal(_))));
    let tail: Vec<String> = vec!["$ git status".into(), "On branch main".into(), "$ ".into()];
    app.shell_tail = Some(crate::app::ShellTail::new(crate::app::TailKey::Terminal(t3), tail));
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("$ terminal")), "{lines:#?}");
    assert!(lines.iter().any(|l| l.contains("enter adopts")), "{lines:#?}");
    assert!(lines.iter().any(|l| l.contains("On branch main")), "the pane is previewed");
    golden("ticket_terminal_120x30", &lines);

    // Armed: the row says what the next Enter does, and a busy terminal
    // names its command in place of the word.
    app.handle_key(
        ratatui::crossterm::event::KeyCode::Enter,
        ratatui::crossterm::event::KeyModifiers::NONE,
    )
    .unwrap();
    app.terminals[0].foreground = Some("cargo".into());
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("$ cargo")), "{lines:#?}");
    assert!(lines.iter().any(|l| l.contains("enter again adopts")), "{lines:#?}");
    golden("ticket_terminal_armed_120x30", &lines);

    // Before the first capture lands the zone says so rather than standing empty.
    app.shell_tail = None;
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("reading its pane")), "{lines:#?}");
}

/// A shell running a command (T-366): the rail row wears the spinner and the
/// command's name, and the card on the board spins — a shell at its prompt
/// still does neither (`a_shell_never_spins`).
#[test]
fn golden_ticket_shell_busy_120() {
    let mut app = app_graphite(fixture(true));
    let t3 = ulid_n(3);
    app.board.sessions.iter_mut().find(|s| s.id == uuid_n(32)).unwrap().foreground =
        Some("cargo".into());
    app.screen = Screen::Ticket { ticket: t3, rail_idx: 1 };
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("$ cargo")), "{lines:#?}");
    golden("ticket_shell_busy_120x30", &lines);
}

#[test]
fn golden_ticket_previous_column() {
    let mut app = app_graphite(fixture(true));
    app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    let t = app.board.ticket_mut(ulid_n(3)).unwrap();
    t.column = "REVIEW".into();
    t.previous_column =
        Some(mesimon_core::board::ColumnStay { column: "IN PROGRESS".into(), seconds: 3661 });
    let lines = render(&app, 120, 30);
    assert!(lines[3].contains("REVIEW for >1y ∙ previously IN PROGRESS for 1h 1m ∙ created"));
    golden("ticket_previous_column_120x30", &lines);
    golden("ticket_previous_column_100x24", &render(&app, 100, 24));
    for seconds in [0, 60, 61, 99, 3600, 86400, 90000] {
        app.board.ticket_mut(ulid_n(3)).unwrap().previous_column.as_mut().unwrap().seconds =
            seconds;
        let line = render(&app, 120, 30)[3].clone();
        if seconds <= 60 {
            assert!(!line.contains("previously"), "{line}");
        } else {
            let duration = match seconds {
                61 | 99 => "1m",
                3600 => "1h",
                86400 => "1d",
                _ => "1d 1h",
            };
            assert!(line.contains(&format!("previously IN PROGRESS for {duration} ∙")), "{line}");
        }
    }
}

/// T-253: a ticket an agent filed says so on the state row, beside its
/// created age; a person's ticket (and a pre-field one) says nothing by it.
#[test]
fn ticket_page_names_an_agent_creator() {
    let mut app = app_graphite(fixture(true));
    app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    let before = render(&app, 120, 30).join("\n");
    assert!(before.contains("created >1y ago"), "{before}");
    assert!(!before.contains(" by claude"), "{before}");

    let t = app.board.tickets.iter_mut().find(|t| t.id == ulid_n(3)).unwrap();
    t.created_by = "agent:00000000-0000-0000-0000-000000000003".into();
    let after = render(&app, 120, 30).join("\n");
    assert!(after.contains("created >1y ago by agent"), "{after}");
    assert!(!after.contains(" on T-"), "{after}");

    // The parent ticket names itself by key while it is on the board…
    let parent_key = app.board.ticket(ulid_n(4)).unwrap().short_key.clone();
    let t = app.board.tickets.iter_mut().find(|t| t.id == ulid_n(3)).unwrap();
    t.created_from = Some(ulid_n(4));
    let with_parent = render(&app, 120, 30).join("\n");
    assert!(with_parent.contains(&format!("by agent on {parent_key}")), "{with_parent}");

    // …and a deleted parent takes its key with it, leaving the author.
    app.board.tickets.retain(|t| t.id != ulid_n(4));
    let orphaned = render(&app, 120, 30).join("\n");
    assert!(orphaned.contains("created >1y ago by agent"), "{orphaned}");
    assert!(!orphaned.contains(" on T-"), "{orphaned}");

    let session = app.board.sessions.iter_mut().find(|s| s.id == uuid_n(31)).unwrap();
    session.kind = SessionKind::Codex;
    let t = app.board.tickets.iter_mut().find(|t| t.id == ulid_n(3)).unwrap();
    t.created_by = format!("agent:{}", uuid_n(31));
    let codex = render(&app, 120, 30).join("\n");
    assert!(codex.contains("created >1y ago by agent"), "{codex}");
    assert!(codex.contains("> codex"), "{codex}");
    assert!(!codex.contains("+ agent session"), "Codex holds the agent seat: {codex}");
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

/// `{ }` on the ticket page turns the preview half a page at a time, the way the
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
    // The page keys sit on the PREVIEW heading, beside the zone they page
    // (T-158), and the footer does not repeat them.
    let heading = |app: &App| {
        render(app, 120, 30).iter().find(|l| l.contains("PREVIEW")).cloned().unwrap_or_default()
    };
    let footer = |app: &App| render(app, 120, 30).last().cloned().unwrap_or_default();

    assert!(shows(&app, "row 01"), "a fresh page starts at the top");
    assert!(!shows(&app, "row 60"));
    assert!(heading(&app).contains("{ } page"), "an overflowing preview offers the page keys");
    assert!(!footer(&app).contains("{ }"), "and the footer does not repeat them");
    let v = app.preview.view.get();
    assert!(v.max > 0 && v.page > 1 && !v.follows_tail, "{v:?}");

    // A press is a GLIDE: the record moves to the next page at once, the
    // rows follow it over `GLIDE`. Frame zero (the glide dated into the
    // future, so `elapsed` saturates at zero on a loaded box) still shows
    // the first row; halfway through, the window is between the two pages;
    // landed, the first row is gone.
    press(&mut app, '}');
    let first = app.preview.view.get().offset;
    assert_eq!(first, v.page / 2, "the record is already half a page down");
    let g = app.preview.glide.get().expect("the press arms a glide");
    assert_eq!((g.key, g.from), (v.key.expect("a document"), 0));
    assert!(app.animating(), "the frame after the press is in motion");
    let future = std::time::Instant::now() + std::time::Duration::from_secs(1);
    app.preview.glide.set(Some(crate::app::Glide { at: future, ..g }));
    assert!(shows(&app, "row 01"), "frame zero: the old page is still on screen");
    assert_eq!(app.preview.view.get().offset, first, "the record does not move with the frame");
    let half = std::time::Instant::now() - crate::app::GLIDE / 2;
    app.preview.glide.set(Some(crate::app::Glide { at: half, ..g }));
    let mid = render(&app, 120, 30);
    let top_row = mid
        .iter()
        .find_map(|l| l.find("row ").map(|i| l[i + 4..i + 6].parse::<usize>().unwrap_or(0)))
        .expect("a reply row on screen");
    assert!(top_row > 1 && top_row <= first, "midway, between the pages: {top_row}");
    settle_preview(&mut app);
    assert!(!app.animating(), "landed");
    assert!(!shows(&app, "row 01"), "one page down and the first row is gone");
    assert!(app.preview.glide.get().is_none(), "the draw retires a landed glide");
    // Past the end: the last window is a FULL one, marked nowhere.
    for _ in 0..20 {
        page(&mut app, '}');
    }
    assert!(shows(&app, "row 60"));
    assert_eq!(app.preview.view.get().offset, v.max);
    assert!(
        !render(&app, 120, 30).iter().any(|l| l.contains("row 60 of the reply~")),
        "the last row is not a cut"
    );
    page(&mut app, '{');
    assert!(!shows(&app, "row 60"));
    for _ in 0..20 {
        page(&mut app, '{');
    }
    assert!(shows(&app, "row 01"));
    assert_eq!(app.preview.view.get().offset, 0);

    // A second press mid-glide starts from where the eye IS, not from where
    // the first press started: one continuous scroll, no restart.
    press(&mut app, '}');
    let g = app.preview.glide.get().expect("glide");
    let g = crate::app::Glide { at: std::time::Instant::now() - crate::app::GLIDE / 2, ..g };
    app.preview.glide.set(Some(g));
    let eye = g.offset(first);
    assert!(eye > 0 && eye < first, "{eye}");
    press(&mut app, '}');
    let g2 = app.preview.glide.get().expect("glide");
    assert_eq!(g2.from, eye, "the second turn begins where the first had got to");
    assert_eq!(app.preview.view.get().offset, 2 * (v.page / 2));
    for _ in 0..20 {
        page(&mut app, '{');
    }

    // Scrolled halfway, then the reply changes: the new one opens at its top.
    page(&mut app, '}');
    assert!(!shows(&app, "row 01"));
    std::fs::write(&path, reply_record(&long.replace("of the reply", "of the next reply")))
        .expect("rewrite");
    let meta = std::fs::metadata(&path).expect("meta");
    // The peek cache keys on (len, mtime) once its stat window has passed
    // (T-255); the length differs, which is enough.
    assert_ne!(meta.len(), 0);
    app.peek_cache.expire();
    assert!(shows(&app, "row 01 of the next reply"), "a new reply starts at its top");
    assert!(app.preview.glide.get().is_none(), "and a glide on the old one is dropped");

    // A reply that fits offers nothing to turn: keys inert, hint gone.
    std::fs::write(&path, reply_record("short")).expect("rewrite");
    app.peek_cache.expire();
    assert!(shows(&app, "short"));
    assert!(!footer(&app).contains("{ }"));
    press(&mut app, '}');
    assert_eq!(app.preview.view.get().offset, 0);
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
    app.shell_tail =
        Some(crate::app::ShellTail::new(crate::app::TailKey::Session(uuid_n(32)), tail.clone()));
    let shows = |app: &App, row: &str| render(app, 120, 30).iter().any(|l| l.contains(row));

    assert!(shows(&app, "line 60") && !shows(&app, "line 01"), "a tail opens at its bottom");
    let v = app.preview.view.get();
    assert!(v.follows_tail && v.offset == v.max && v.max > 0, "{v:?}");
    assert!(app.preview.request.get().is_none(), "following is the absence of a request");

    page(&mut app, '{');
    assert!(!shows(&app, "line 60"));
    assert!(shows(&app, "line 60~") || render(&app, 120, 30).iter().any(|l| l.ends_with('~')));
    // New output while scrolled up: the reader's window holds still.
    let before = app.preview.view.get().offset;
    let mut more = tail.clone();
    more.push("line 61".into());
    app.shell_tail =
        Some(crate::app::ShellTail::new(crate::app::TailKey::Session(uuid_n(32)), more));
    let _ = render(&app, 120, 30);
    assert_eq!(app.preview.view.get().offset, before);
    assert!(!shows(&app, "line 61"));

    // One page back down lands where the bottom WAS: the pane grew a row
    // meanwhile, so the window stops one short, says so, and stays pinned
    // (a page is a page, as in the diff). The next press reaches the end
    // and releases it — the new line arrives with it.
    page(&mut app, '}');
    assert!(shows(&app, "line 60~") && !shows(&app, "line 61"));
    assert!(app.preview.request.get().is_some());
    page(&mut app, '}');
    assert!(shows(&app, "line 61"), "back at the bottom, and the new line is there");
    assert!(app.preview.request.get().is_none(), "at the bottom the tail is released");
}

#[test]
fn golden_diff_screen_120() {
    let mut app = app_graphite(fixture(false));
    install_diff(&mut app);
    golden("diff_120x30", &render(&app, 120, 30));
}

fn install_long_diff(app: &mut App) {
    install_diff(app);
    let d = app.diff.as_mut().unwrap();
    let file = d.cache.get_mut(&d.files[0].path).unwrap();
    file.hunks.truncate(1);
    file.hunks[0].lines = (1..=100)
        .map(|n| mesimon_core::diff::HunkLine {
            sign: mesimon_core::diff::Sign::Ctx,
            old_ln: Some(n),
            new_ln: Some(n),
            text: format!("diff row {n:03}"),
        })
        .collect();
}

#[test]
fn diff_hints_live_beside_their_panes_and_pages_use_the_viewport() {
    for (width, height) in [(60, 20), (90, 24), (100, 24), (120, 30), (160, 40)] {
        let mut app = app_graphite(fixture(false));
        install_long_diff(&mut app);
        if width < 100 {
            let files = render(&app, width, height);
            assert!(files[5].contains("n N file"), "{}", files[5]);
            press(&mut app, '}');
            assert_eq!(
                app.diff.as_ref().unwrap().pager.view.get().offset,
                0,
                "hidden diff does not page"
            );
            app.diff.as_mut().unwrap().swap = true;
        }
        let rows = render(&app, width, height);
        assert!(rows[5].contains("n N file"), "{}", rows[5]);
        assert!(rows[5].contains("{ } page"), "{}", rows[5]);
        if width == 60 || width == 120 {
            golden(&format!("diff_paging_{width}x{height}"), &rows);
        }
        for hint in ["n N", "{ }", "jk scroll"] {
            assert!(!rows.last().unwrap().contains(hint), "{rows:?}");
        }
        let v = app.diff.as_ref().unwrap().pager.view.get();
        assert_eq!(v.page, height as usize - 9);
        press(&mut app, '}');
        assert_eq!(app.diff.as_ref().unwrap().pager.view.get().offset, v.page / 2);
        for _ in 0..20 {
            press(&mut app, '}');
        }
        assert_eq!(app.diff.as_ref().unwrap().pager.view.get().offset, v.max);
        for _ in 0..20 {
            press(&mut app, '{');
        }
        assert_eq!(app.diff.as_ref().unwrap().pager.view.get().offset, 0);
    }
}

#[test]
fn half_page_jumps_round_small_views_and_preserve_full_page_keys() {
    use crate::app::View;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};

    for screen in
        [Screen::Diff, Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 }, Screen::Releases]
    {
        for (page, half) in [(1, 1), (2, 1), (11, 5), (20, 10)] {
            let mut app = app_graphite(fixture(false));
            install_long_diff(&mut app);
            install_releases(&mut app);
            app.screen = screen.clone();
            let view = View { key: Some(1), page, max: 97, ..Default::default() };
            app.pager().expect("a reading screen").view.set(view);
            let offset = |app: &App| app.pager().expect("a reading screen").view.get().offset;
            for (key, expected) in [
                (KeyCode::Char('}'), half),
                (KeyCode::Char('}'), 2 * half),
                (KeyCode::Char('{'), half),
                (KeyCode::PageDown, half + page),
                (KeyCode::PageUp, half),
                (KeyCode::Char('{'), 0),
            ] {
                app.handle_key(key, KeyModifiers::NONE).unwrap();
                assert_eq!(offset(&app), expected, "{screen:?}, page {page}, {key:?}");
            }
            for (key, expected) in [('}', 97), ('{', 0)] {
                for _ in 0..100 {
                    press(&mut app, key);
                }
                assert_eq!(offset(&app), expected, "{screen:?}, page {page}, {key}");
            }
        }
    }
}

#[test]
fn diff_pages_glide_and_file_changes_cancel_the_motion() {
    use crate::app::{Glide, GLIDE};
    use std::time::{Duration, Instant};
    let mut app = app_graphite(fixture(false));
    install_long_diff(&mut app);
    let before = render(&app, 120, 30);
    let page = app.diff.as_ref().unwrap().pager.view.get().page / 2;
    press(&mut app, '}');
    let g = app.diff.as_ref().unwrap().pager.glide.get().unwrap();
    assert_eq!(g.from, 0);
    assert!(app.animating());
    app.diff
        .as_ref()
        .unwrap()
        .pager
        .glide
        .set(Some(Glide { at: Instant::now() + Duration::from_secs(1), ..g }));
    assert_eq!(render(&app, 120, 30), before, "frame zero retains the old page");
    let top_row = |rows: &[String]| {
        rows.iter()
            .find_map(|row| {
                row.find("diff row ").and_then(|i| row[i + 9..i + 12].parse::<usize>().ok())
            })
            .unwrap()
    };
    app.diff.as_ref().unwrap().pager.glide.set(Some(Glide { at: Instant::now() - GLIDE / 2, ..g }));
    let mid = top_row(&render(&app, 120, 30));
    assert!(mid > 1 && mid < page, "{mid} between 1 and {page}");
    press(&mut app, '}');
    let next = app.diff.as_ref().unwrap().pager.glide.get().unwrap();
    assert!(next.from >= mid && next.from < page, "continues from the visible row");
    assert_eq!(app.diff.as_ref().unwrap().pager.view.get().offset, 2 * page);
    app.diff.as_ref().unwrap().pager.glide.set(Some(Glide { at: Instant::now() - GLIDE, ..next }));
    let landed = render(&app, 120, 30);
    assert_eq!(top_row(&landed), 2 * page);
    assert!(!app.animating());
    press(&mut app, '{');
    assert!(app.animating(), "paging up also glides");
    press(&mut app, 'n');
    assert!(!app.animating());
    assert_eq!(app.diff.as_ref().unwrap().pager.view.get().offset, 0);
    assert!(!render(&app, 120, 30)[5].contains("{ } page"), "binary fits");

    // File navigation stays visible even on a display-only untracked entry.
    app.diff.as_mut().unwrap().file_idx = 3;
    app.diff.as_mut().unwrap().swap = true;
    assert!(render(&app, 90, 24)[5].contains("n N file"));
}

#[test]
fn column_header_footer_teaches_new_column() {
    let mut app = app_graphite(fixture(false));
    assert!(!render(&app, 120, 30).last().unwrap().contains("O new column"));
    app.cursor_row = None;
    assert!(render(&app, 120, 30).last().unwrap().contains("O new column"));
}

#[test]
fn golden_checkout_diff_120() {
    let mut app = app_graphite(fixture(false));
    install_checkout_diff(&mut app);
    // Sampled and out of sync, which is the shape the title has to carry
    // (T-347): the arrows on the branch and `tab` beside them.
    app.git = git_state("main", 2, 1, 2);
    golden("diff_checkout_120x30", &render(&app, 120, 30));
}

/// T-347: the push/pull state is on the identity row, and the key that goes
/// to it sits beside it rather than in the footer. Before this the row said
/// nothing at all and the footer's `tab` read the same whether or not
/// anything was pending.
#[test]
fn checkout_diff_title_carries_push_pull_and_its_key() {
    let mut app = app_graphite(fixture(false));
    install_checkout_diff(&mut app);
    let row = |app: &App| render(app, 120, 30)[2].clone();
    let footer = |app: &App| render(app, 120, 30)[29].clone();

    // Unsampled says nothing — an unknown must not read as "in sync" — but
    // the key is still offered, because the view exists either way.
    let r = row(&app);
    assert!(r.contains("⎇ main ∙ uncommitted"), "{r}");
    assert!(r.contains("tab push / pull"), "{r}");

    app.git = git_state("main", 2, 1, 2);
    let r = row(&app);
    assert!(r.contains("⎇ main ↑2 ↓1 ∙ uncommitted"), "{r}");
    assert!(r.contains("tab push / pull"), "{r}");
    // One home for the hint: off the footer since it is drawn on the row.
    let f = footer(&app);
    assert!(!f.contains("tab"), "{f}");
    assert!(f.contains("q back"), "{f}");

    // One direction at a time, and level says nothing.
    app.git = git_state("main", 3, 0, 0);
    assert!(row(&app).contains("⎇ main ↑3 ∙"), "{}", row(&app));
    app.git = git_state("main", 0, 4, 0);
    assert!(row(&app).contains("⎇ main ↓4 ∙"), "{}", row(&app));
    app.git = git_state("main", 0, 0, 0);
    let r = row(&app);
    assert!(r.contains("⎇ main ∙ uncommitted"), "{r}");
    assert!(!r.contains('↑') && !r.contains('↓'), "{r}");

    // The other side of the toggle carries the same arrows and the way back.
    app.git = git_state("main", 2, 1, 2);
    app.diff.as_mut().unwrap().commits = true;
    let r = row(&app);
    assert!(r.contains("⎇ main ↑2 ↓1 ∙ push / pull ∙ origin/main"), "{r}");
    assert!(r.contains("tab uncommitted"), "{r}");

    // A ticket's branch diff is measured against its base, not against the
    // checkout's upstream — no arrows there, and no key either.
    let mut app = app_graphite(fixture(false));
    install_diff(&mut app);
    app.git = git_state("main", 2, 1, 2);
    let r = row(&app);
    assert!(!r.contains('↑') && !r.contains('↓'), "{r}");
    assert!(!r.contains("tab"), "{r}");
}

#[test]
fn checkout_commits_show_both_directions_and_empty_states() {
    use mesimon_core::command::GitCommit;
    let mut app = app_graphite(fixture(false));
    install_checkout_diff(&mut app);
    app.diff.as_mut().unwrap().commits = true;
    app.git = git_state("main", 2, 1, 0);
    let commit =
        |oid: &str, subject: &str| GitCommit { oid: oid.repeat(40), subject: subject.into() };
    app.git.to_push = Some(vec![commit("a", "Add commit lists"), commit("b", "Prepare Git view")]);
    app.git.to_pull = Some(vec![commit("c", "Fix upstream regression")]);
    for width in [60, 120] {
        let rows = render(&app, width, 30);
        let text = rows.join("\n");
        assert!(text.contains("TO PUSH (2)"));
        assert!(text.contains("aaaaaaa  Add commit lists"));
        assert!(text.contains("TO PULL (1)"));
        assert!(text.contains("ccccccc  Fix upstream regression"));
        assert!(text.contains("tab uncommitted"), "{text}");
        golden(&format!("git_commits_{width}x30"), &rows);
    }
    app.git.to_push = None;
    assert!(render(&app, 120, 30).join("\n").contains("Commit list unavailable"));
    app.git.ahead = 0;
    app.git.behind = 0;
    let text = render(&app, 120, 30).join("\n");
    assert_eq!(text.matches("Nothing pending").count(), 2);
    assert!(text.contains("in sync"));
    app.git.upstream = None;
    assert!(render(&app, 120, 30).join("\n").contains("No upstream configured"));
    app.git.detached = true;
    assert!(render(&app, 120, 30).join("\n").contains("Detached HEAD"));
    app.git.sampled = false;
    assert!(render(&app, 120, 30).join("\n").contains("Git status unavailable"));
}

#[test]
fn checkout_commits_scroll_to_incoming_and_clamp_after_snapshot_shrinks() {
    let mut app = app_graphite(fixture(false));
    install_checkout_diff(&mut app);
    app.diff.as_mut().unwrap().commits = true;
    app.git = git_state("main", 105, 1, 0);
    app.git.to_push = Some(
        (0..100)
            .map(|i| mesimon_core::command::GitCommit {
                oid: "a".repeat(40),
                subject: format!("outgoing {i} {}", "統一碼".repeat(60)),
            })
            .collect(),
    );
    app.git.to_pull = Some(vec![mesimon_core::command::GitCommit {
        oid: "b".repeat(40),
        subject: "incoming commit".into(),
    }]);
    let text = render(&app, 60, 20).join("\n");
    assert!(text.contains("outgoing 0"));
    assert!(!text.contains("incoming commit"));
    app.diff.as_ref().unwrap().pager.request.set(Some((crate::ui::diff::COMMITS_KEY, usize::MAX)));
    let text = render(&app, 60, 20).join("\n");
    assert!(text.contains("5 more commits"));
    assert!(text.contains("incoming commit"));
    assert!(app.diff.as_ref().unwrap().pager.view.get().max > 0);
    app.git.ahead = 0;
    app.git.behind = 0;
    let _ = render(&app, 60, 20);
    assert_eq!(app.diff.as_ref().unwrap().pager.view.get().offset, 0);
}

#[test]
fn empty_diffs_say_no_changes_and_titles_omit_density() {
    for checkout in [false, true] {
        let mut app = app_graphite(fixture(false));
        if checkout {
            install_checkout_diff(&mut app);
        } else {
            install_diff(&mut app);
        }
        for density in [1, 3, 8] {
            app.diff.as_mut().unwrap().density = density;
            let rows = render(&app, 120, 30);
            assert!(!rows[2].contains(super::diff::density_word(density)), "{}", rows[2]);
        }
        app.diff.as_mut().unwrap().files.clear();
        let rows = render(&app, 120, 30);
        assert!(rows[2].contains("∙ no changes"), "{}", rows[2]);
        assert!(!rows[2].contains("0 files"), "{}", rows[2]);
        assert!(!rows[2].contains("uncommitted"), "{}", rows[2]);
        assert!(!rows[2].contains("+0 -0"), "{}", rows[2]);
    }
}

#[test]
fn focusing_git_preserves_header_text_and_geometry() {
    for width in [60, 80, 100, 120, 160] {
        for branch in ["main", "msmn/T-124-git-status-pull-push-indication"] {
            let mut app = app_graphite(fixture(false));
            app.git = git_state(branch, 2, 1, 3);
            app.force_update_ready();
            let before = render(&app, width, 30)[0].clone();
            app.header_focus = true;
            assert_eq!(render(&app, width, 30)[0], before, "width {width}, branch {branch}");
        }
    }
}

/// The checkout diff is the same screen answering a different question, and
/// the three places it has to say so (T-221).
#[test]
fn checkout_diff_says_uncommitted_and_offers_the_checkout_terminal() {
    let mut app = app_graphite(fixture(false));
    install_checkout_diff(&mut app);
    let rows = lines_of(&cells(&app, 120, 30));

    // The header's leaf is the ticket a diff belongs to; this one belongs to
    // the repository, which the breadcrumb already names.
    assert!(rows[0].starts_with(" DIFF   mesimon › kanban-tui"), "{}", rows[0]);
    assert!(!rows[0].contains('>') || rows[0].matches('>').count() == 1, "no leaf: {}", rows[0]);
    // Measured against itself, in a word rather than an oid.
    assert!(rows[2].contains("⎇ main ∙ uncommitted ∙ 2 files"), "{}", rows[2]);
    assert!(!rows[2].contains(" vs "), "{}", rows[2]);
    assert!(!rows[2].contains("worktree evicted"), "{}", rows[2]);

    // `!` is the project's terminal on the diff's own target (T-273): here
    // the checkout, so the overlay's word says nothing of a worktree. The
    // footer no longer hints it at all (T-277: `?` is where a standing key
    // is listed). (Before T-273 it was a foreground shell in the worktree
    // and inert on this diff.)
    let footer = rows.last().unwrap();
    assert!(!footer.contains("terminal"), "{footer}");
    assert!(footer.contains("q back"), "{footer}");
    let help = mesimon_core::keymap::overlay(mesimon_core::keymap::Scope::Diff, &app.ctx());
    let (_, hint) = help
        .iter()
        .flat_map(|(_, rows)| rows.iter())
        .find(|(key, _)| *key == "!")
        .expect("the overlay lists the terminal");
    assert_eq!(*hint, "terminal");
    assert_eq!(
        mesimon_core::keymap::resolve(
            mesimon_core::keymap::Scope::Diff,
            mesimon_core::keymap::Key::Char('!'),
            &app.ctx()
        ),
        Some(mesimon_core::keymap::Verb::Terminal)
    );

    // Every row is uncommitted here, so `D` would be a letter on every line
    // saying nothing; `U` still earns its cell.
    let listed: String = rows.iter().filter(|r| r.contains("AGENTS.md")).cloned().collect();
    assert!(listed.contains("AU AGENTS.md"), "the untracked row is an add, and says it: {listed}");
    assert!(!rows.iter().any(|r| r.contains("MD ")), "no dirty letter on a checkout diff");

    // And `q` lands on the board, not on some ticket page.
    press(&mut app, 'q');
    assert!(matches!(app.screen, Screen::Board));
    assert!(app.diff.is_none());
}

/// `v` on the board opens it for real, through dispatch and the wire.
#[test]
fn board_v_opens_the_checkout_diff() {
    let mut app = app_graphite(fixture(false));
    app.git = git_state("main", 2, 1, 3);
    assert!(app.ctx().git_repo, "a sampled checkout is what the key needs");
    press(&mut app, 'v');
    assert!(matches!(app.screen, Screen::Diff));
    let d = app.diff.as_ref().unwrap();
    assert!(!d.is_branch(), "the board's v is the checkout's, whatever the cursor is over");
    assert_eq!(d.branch, "main");
    assert_eq!(app.diff_ticket(), None, "no ticket is the subject here");

    // Without a repository under it the key does nothing at all.
    let mut bare = app_graphite(fixture(false));
    press(&mut bare, 'v');
    assert!(matches!(bare.screen, Screen::Board));
    assert!(bare.diff.is_none());
}

/// The board's own top row as a cursor position (T-305): `k` off a column
/// header paints the git clause on the elevated surface — surrounding gaps
/// stay on the page, the greys on the `sel` ramp — takes the cursor bar off the column
/// header while leaving it its band, and hands the footer to `Scope::Header`,
/// where Enter is the checkout diff.
#[test]
fn golden_board_header_bar_120() {
    let mut app = app_graphite(fixture(false));
    app.cursor_col = 1;
    app.git = git_state("main", 2, 1, 3);
    press(&mut app, 'k');
    assert!(app.on_column_header(), "one press onto the column header");
    press(&mut app, 'k');
    assert!(app.header_focus, "the second lands on the row above it");

    let buf = cells(&app, 120, 30);
    let band = Color::Rgb(0x27, 0x2B, 0x31);
    // ` ⎇ main ↑2 ↓1 ∙ 3 changed ` starts after `mesimon › kanban-tui`.
    // A CELL index, never a byte offset into the row: the breadcrumb ahead of
    // the clause is not ASCII (`›` is three bytes), so `str::find` would land
    // two cells late.
    let at = (0..120u16).find(|&x| buf[(x, 0)].symbol() == "⎇").expect("the clause is drawn");
    for x in [at, at + 12, at + 23] {
        assert_eq!(buf[(x, 0)].bg, band, "the focused clause is painted at {x}");
    }
    assert_ne!(buf[(at - 1, 0)].bg, band, "the repository gap stays on the page");
    assert_ne!(buf[(at + 24, 0)].bg, band, "the following gap stays on the page");
    // The column keeps its band — that is what says where `j` goes back to —
    // and gives the cursor bar up.
    assert_eq!(buf[(40, 2)].bg, band, "the cursor column is still the cursor column");
    assert_eq!(buf[(31, 2)].symbol(), " ", "no cursor bar on the column header");

    let lines = render(&app, 120, 30);
    let foot = lines.last().unwrap();
    assert!(foot.contains("HEADER"), "{foot:?}");
    assert!(foot.contains("enter diff"), "{foot:?}");
    assert!(foot.contains("j back to the board"), "{foot:?}");
    assert!(!foot.contains("esc menu"), "esc pops off the row up here: {foot:?}");
    golden("board_header_bar_120x30", &lines);
}

/// Enter on the focused clause is the board's `v`, through dispatch and the
/// wire; `q` comes back to the board with the row still holding the cursor.
#[test]
fn enter_on_the_header_bar_opens_the_checkout_diff() {
    let mut app = app_graphite(fixture(false));
    app.git = git_state("main", 2, 1, 3);
    press(&mut app, 'k');
    press(&mut app, 'k');
    assert_eq!(app.scope(), mesimon_core::keymap::Scope::Header);
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    app.handle_key(KeyCode::Enter, KeyModifiers::NONE).expect("enter");
    assert!(matches!(app.screen, Screen::Diff));
    let d = app.diff.as_ref().unwrap();
    assert!(!d.is_branch(), "the top row is the repository's, like the board under it");
    assert_eq!(app.diff_ticket(), None);
    press(&mut app, 'q');
    assert!(matches!(app.screen, Screen::Board));
    assert!(app.header_focus, "and the cursor is where it was left");
}

/// The board's `v` no longer teaches itself in the header (T-305): the git
/// clause is a place the cursor can stand, so the key that reads it is the
/// footer's to name once the cursor is there. It stays off the board's own
/// footer — that one is the selection's — and `?` still lists it.
#[test]
fn the_header_no_longer_spells_the_diff_key() {
    let mut app = app_graphite(fixture(false));
    app.git = git_state("main", 2, 1, 3);
    let head = &render(&app, 120, 30)[0];
    assert!(head.contains("∙ 3 changed   7 tickets"), "{head:?}");
    assert!(!head.contains("v diff"), "the count no longer carries the key: {head:?}");

    let ctx = app.ctx();
    let (left, _) = mesimon_core::keymap::footer_split(mesimon_core::keymap::Scope::Board, &ctx);
    assert!(!left.iter().any(|b| b.show == "v"), "and stays off the footer");
    // Overlay-only means the overlay still has it — that is the whole bargain.
    let overlay = mesimon_core::keymap::overlay(mesimon_core::keymap::Scope::Board, &ctx);
    assert!(
        overlay.iter().any(|(_, items)| items.iter().any(|(show, _)| *show == "v")),
        "`?` is where a prio-0 key is always listed"
    );
    assert_eq!(
        mesimon_core::keymap::resolve(
            mesimon_core::keymap::Scope::Board,
            mesimon_core::keymap::Key::Char('v'),
            &ctx
        ),
        Some(mesimon_core::keymap::Verb::OpenDiff)
    );
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
fn golden_releases_120() {
    let mut app = app_graphite(fixture(false));
    install_releases(&mut app);
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("this build")), "the build's entry is marked");
    golden("releases_120x30", &lines);
}

#[test]
fn golden_releases_scrolled_120() {
    // Fourteen rows in: the top entry's band is off the window, so it is
    // pinned to the first row while its notes scroll under it.
    let mut app = app_graphite(fixture(false));
    install_releases(&mut app);
    app.releases
        .as_ref()
        .expect("state")
        .pager
        .request
        .set(Some((crate::ui::releases::DOC_KEY, 14)));
    let lines = render(&app, 120, 30);
    assert!(lines[4].contains("v0.9.0-alpha.3"), "band pinned: {:?}", lines[4]);
    golden("releases_scrolled_120x30", &lines);
}

#[test]
fn golden_releases_160() {
    // Wider than the measure: the column stays a hundred cells and centres.
    let mut app = app_graphite(fixture(false));
    install_releases(&mut app);
    golden("releases_160x30", &render(&app, 160, 30));
}

/// The menu row opens the real changelog on this build's entry; the diff's
/// reading keys page it, `n`/`N` step by release, `q` returns to the board.
#[test]
fn test_release_notes_from_the_menu() {
    use mesimon_core::keymap::{self, Verb};
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    let mut app = app_graphite(fixture(false));
    let ctx = app.ctx();
    let idx = keymap::menu_items(&ctx)
        .iter()
        .position(|m| m.verb == Verb::ReleaseNotes)
        .expect("the release notes row");
    app.mode = Mode::Menu { idx };
    app.handle_key(KeyCode::Enter, KeyModifiers::NONE).expect("enter");
    assert!(matches!(app.screen, Screen::Releases));
    let build = concat!("v", env!("CARGO_PKG_VERSION"));
    let lines = render(&app, 120, 30);
    assert!(lines[0].contains("RELEASES"), "chip: {:?}", lines[0]);
    assert!(lines[2].contains(&format!("this is {build}")), "ident: {:?}", lines[2]);
    assert!(lines[4].contains(build) && lines[4].contains("this build"), "band: {:?}", lines[4]);
    assert!(lines.last().expect("footer").contains("next / previous release"));

    // `}` pages — a glide, as on the preview and the diff (T-246: one
    // pager, one motion); `{` back to the top.
    press(&mut app, '}');
    assert!(app.animating(), "a page turn on the notes glides");
    let page = app.releases.as_ref().expect("state").pager.view.get().page;
    assert!(page > 1);
    assert_eq!(app.releases.as_ref().expect("state").pager.view.get().offset, page / 2);
    press(&mut app, '{');
    assert_eq!(app.releases.as_ref().expect("state").pager.view.get().offset, 0);
    // `n` lands the second release's band on the first row; `N` comes back.
    press(&mut app, 'n');
    let _ = render(&app, 120, 30);
    let lines = render(&app, 120, 30);
    let second = &app.releases.as_ref().expect("state").releases[1].tag;
    assert!(lines[4].contains(second.as_str()), "n: {:?}", lines[4]);
    press(&mut app, 'N');
    assert_eq!(app.releases.as_ref().expect("state").pager.view.get().offset, 0);
    // `j` scrolls one row and never past the end.
    press(&mut app, 'j');
    assert_eq!(app.releases.as_ref().expect("state").pager.view.get().offset, 1);
    press(&mut app, 'G');
    press(&mut app, 'k');
    assert_eq!(app.releases.as_ref().expect("state").pager.view.get().offset, 0);
    press(&mut app, 'q');
    assert!(matches!(app.screen, Screen::Board));
    assert!(app.releases.is_none());
}

/// The real `CHANGELOG.md`, every page of it, at three widths: no rule or
/// box glyph reaches a cell (L1 — markdown is full of them), and every
/// release's band is reachable by `n`.
#[test]
fn test_real_changelog_reads_lawfully() {
    use mesimon_core::relnotes;
    let releases = relnotes::parse(relnotes::SOURCE);
    for (w, h) in [(120u16, 30u16), (100, 24), (200, 50)] {
        let mut app = app_graphite(fixture(false));
        app.releases = Some(crate::app::ReleasesState::new(
            releases.clone(),
            concat!("v", env!("CARGO_PKG_VERSION")),
        ));
        app.screen = Screen::Releases;
        let _ = render(&app, w, h);
        let st = app.releases.as_ref().expect("state");
        let (max, page) = (st.pager.view.get().max, st.pager.view.get().page);
        assert_eq!(st.doc.borrow().as_ref().expect("drawn").starts.len(), releases.len());
        let mut top = 0;
        loop {
            st.pager.jump(top);
            for line in render(&app, w, h) {
                for c in line.chars() {
                    let u = c as u32;
                    assert!(
                        !(0x2500..=0x257F).contains(&u),
                        "{w}x{h} top {top}: drawn structure {c:?} in {line:?}"
                    );
                }
            }
            if top >= max {
                break;
            }
            top += page;
        }
    }
}

/// T-444: the notes are laid out once per width and flavor, not once per
/// frame — a glide draws sixty frames a second and a held `j` one per key,
/// and each used to parse and wrap the whole changelog to show a window.
#[test]
fn test_release_notes_render_once() {
    let mut app = app_graphite(fixture(false));
    install_releases(&mut app);
    let doc =
        |app: &App| app.releases.as_ref().and_then(|r| r.doc.borrow().clone()).expect("drawn");
    let _ = render(&app, 120, 30);
    let first = doc(&app);
    press(&mut app, 'j');
    let _ = render(&app, 120, 30);
    press(&mut app, '}');
    let _ = render(&app, 120, 30);
    assert!(std::rc::Rc::ptr_eq(&first, &doc(&app)), "a scroll reuses the document");
    let _ = render(&app, 160, 30);
    assert!(!std::rc::Rc::ptr_eq(&first, &doc(&app)), "a resize lays it out again");
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
        merged_in: String::new(),
        merged_oid: String::new(),
        conflict: false,
        ahead: 2,
        needs_rebase: false,
        detail: None,
        path: Some("/wt/T-3-fix-osc-11-detection".into()),
        repos: vec![],
    }];
    app.cursor_col = 1;
    app.cursor_row = Some(0);
    golden("board_worktree_120x30", &render(&app, 120, 30));
}

/// T-309: a ticket set to a worktree of its own before anyone has started on
/// it. Provisioning is lazy — the tree is cut at the first spawn — so between
/// the pick and that spawn the card drew NOTHING at all, and shift+tab on the
/// board (which is what put the pick on this screen) had no answer to show
/// for itself. One dot, in the dormant register: less than `queued`'s three.
#[test]
fn a_worktree_asked_for_but_not_cut_wears_a_dormant_mark() {
    let mut app = app_graphite(fixture(false));
    let mark = crate::glyphs::branch_mark(crate::glyphs::Tier::Unicode);
    let planned = format!("{mark}\u{b7}");
    let marked = |app: &App| {
        render(app, 120, 30).iter().filter(|l| l.contains(mark)).cloned().collect::<Vec<_>>()
    };
    assert!(marked(&app).is_empty(), "the shared checkout says nothing");
    for t in app.board.tickets.iter_mut().filter(|t| t.id == ulid_n(1)) {
        t.workspace = Some(mesimon_core::board::WorkspaceStrategy::Worktree);
    }
    app.cursor_row = Some(0);
    let rows = marked(&app);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(rows[0].contains(&planned), "{rows:?}");
    // The mark is right-aligned against the age slot, so its width IS the
    // glyph's column — see `the_worktree_glyph_holds_one_column`.
    assert_eq!(unicode_width::UnicodeWidthStr::width(planned.as_str()), 2);
    golden("board_worktree_planned_120x30", &render(&app, 120, 30));
    // A binding, once there is one, takes the mark back: the dot is the gap
    // between the choice and the tree, not a second way to say worktree.
    app.worktrees = vec![mesimon_core::command::WorktreeItem {
        ticket: ulid_n(1),
        branch: "msmn/T-1-decay-treatments".into(),
        status: "attached".into(),
        merged: false,
        merged_in: String::new(),
        merged_oid: String::new(),
        conflict: false,
        ahead: 0,
        needs_rebase: false,
        detail: None,
        path: Some("/wt/T-1".into()),
        repos: vec![],
    }];
    let rows = marked(&app);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(!rows[0].contains(&planned), "{rows:?}");
}

/// The worktree mark is right-aligned against the fixed age slot, so its
/// WIDTH is its glyph's column: a state char beside the branch glyph pulls
/// the glyph one cell left, and a clean attached worktree — the one arm with
/// nothing to report — used to render one cell narrower and sit a column
/// right of every other card's, with the freed cell handed back to the
/// title. The board reads down a column, so the anchor is the glyph.
#[test]
fn the_worktree_glyph_holds_one_column() {
    use unicode_width::UnicodeWidthStr;
    let wt = |id: u128, ahead: u32, merged: bool| mesimon_core::command::WorktreeItem {
        merged_in: String::new(),
        merged_oid: String::new(),
        ticket: ulid_n(id),
        branch: format!("msmn/T-{id}"),
        status: "attached".into(),
        merged,
        conflict: false,
        ahead,
        needs_rebase: false,
        detail: None,
        path: Some("/wt/x".into()),
        repos: vec![],
    };
    let mut app = app_graphite(fixture(false));
    // T-3 and T-4 share IN PROGRESS, so the two cards sit one above the other.
    app.worktrees = vec![wt(3, 2, false), wt(4, 0, false)];
    let mark = crate::glyphs::branch_mark(crate::glyphs::Tier::Unicode);
    let col = |l: &str| l.find(mark).map(|b| l[..b].width());
    let cols: Vec<usize> = render(&app, 120, 30).iter().filter_map(|l| col(l)).collect();
    assert_eq!(cols.len(), 2, "expected both worktree cards to draw a mark");
    assert_eq!(cols[0], cols[1], "the ahead and the clean mark sit in different columns");
    // And the state char never widens the slot past its neighbour's.
    app.worktrees = vec![wt(3, 0, true), wt(4, 0, false)];
    let cols: Vec<usize> = render(&app, 120, 30).iter().filter_map(|l| col(l)).collect();
    assert_eq!(cols.len(), 2);
    assert_eq!(cols[0], cols[1], "the merged and the clean mark sit in different columns");
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
        merged_in: String::new(),
        merged_oid: String::new(),
        conflict: false,
        ahead: 0,
        needs_rebase: false,
        detail: None,
        path: None,
        repos: vec![],
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

/// Two full-height marks on the row's actual surface, including when open.
#[test]
fn test_two_tags_keep_their_columns_when_the_card_opens() {
    let path = write_transcript("tags-split", &reply_record("Rebased and green."));
    for flavor in crate::theme::Flavor::ALL {
        for selected in [false, true] {
            for peek in [false, true] {
                let mut b = fixture_tagged();
                attach_transcript(&mut b, &path);
                let mut app = app_graphite(b);
                app.theme = crate::theme::Theme::new(flavor, crate::theme::Profile::TrueColor);
                app.cursor_col = if selected { 1 } else { 0 };
                app.cursor_row = Some(0);
                app.peek = peek;
                let at = |group, name| {
                    app.theme
                        .pip(app.board.tag_def(group, name).expect("registered").tint() as usize)
                };
                let (first, second) = (at(1, "BUG"), at(2, "STAGING"));
                let lines = render(&app, 120, 30);
                let top = lines.iter().position(|l| l.contains("Fix OSC-11")).expect("card") as u16;
                let buf = cells(&app, 120, 30);
                let x = (30..60).find(|x| buf[(*x, top)].symbol() == "▉").expect("tag bar");
                let surface = if selected { app.theme.selected_bg } else { app.theme.bg }.unwrap();
                let mut rows = 0;
                for y in top..30 {
                    if buf[(x, y)].symbol() != "▉" {
                        break;
                    }
                    assert_eq!(buf[(x + 1, y)].symbol(), "▉");
                    assert_eq!(buf[(x, y)].fg, first);
                    assert_eq!(buf[(x + 1, y)].fg, second);
                    assert_eq!(buf[(x, y)].bg, surface, "gap at {x},{y}");
                    assert_eq!(buf[(x + 1, y)].bg, surface);
                    rows += 1;
                }
                if selected && peek {
                    assert!(rows >= 3);
                } else {
                    assert!(rows >= 1);
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(path.parent().expect("dir"));
}

/// Selection and sleep do not dim tag identity.
#[test]
fn test_tags_keep_full_strength_off_the_cursor() {
    let mut app = app_graphite(fixture_tagged());
    app.cursor_col = 0;
    let tint = app.theme.pip(app.board.tag_def(1, "FTR").expect("tag").tint() as usize);
    for row in [0, 1] {
        app.cursor_row = Some(row);
        let buf = cells(&app, 120, 30);
        let lines = lines_of(&buf);
        for (name, x) in [("Decay treatments", 1), ("Painted accent", 91)] {
            let y = lines.iter().position(|l| l.contains(name)).expect("card") as u16;
            for dx in 0..2 {
                assert_eq!(buf[(x + dx, y)].bg, tint);
                assert_eq!(buf[(x + dx, y)].symbol(), " ");
            }
        }
    }
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
    app.cursor_row = Some(0); // "Decay treatments": no sessions, and the cursor card
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
    for flavor in crate::theme::Flavor::ALL {
        for selected in [false, true] {
            let mut b = fixture_tagged();
            let mut session = session(
                42,
                ulid_n(3),
                SessionKind::Claude,
                SessionState::RequiresAction { reason: Reason::Permission },
            );
            session.waiting_since = Some(1);
            b.sessions.push(session);
            let mut app = app_graphite(b);
            app.theme = crate::theme::Theme::new(flavor, crate::theme::Profile::TrueColor);
            app.cursor_col = if selected { 1 } else { 0 };
            let buf = cells(&app, 120, 30);
            let lines = lines_of(&buf);
            let y = lines.iter().position(|l| l.contains("Fix OSC-11")).expect("card") as u16;
            let x = (30..60).find(|x| buf[(*x, y)].symbol() == "▉").expect("tags");
            let surface = if selected { app.theme.selected_bg } else { app.theme.bg }.unwrap();
            for (dx, group, name) in [(0, 1, "BUG"), (1, 2, "STAGING")] {
                let tint = app.theme.pip(app.board.tag_def(group, name).unwrap().tint() as usize);
                assert_eq!(buf[(x + dx, y)].fg, tint);
                assert_eq!(buf[(x + dx, y)].bg, surface, "keep tags off the attention ground");
            }
            assert_eq!(buf[(x + 2, y)].bg, app.theme.attn, "the title keeps its alarm band");
        }
    }
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

/// All cards reserve the same two cells: adding a second tag only changes
/// the bar and the named chips, never title position or available width.
#[test]
fn test_the_second_tag_costs_no_width() {
    let plain = |lines: Vec<String>| -> Vec<String> {
        lines.iter().map(|l| l.replace('▉', " ")).collect()
    };
    for peek in [false, true] {
        let mut two = app_graphite(fixture_tagged());
        two.cursor_col = 1;
        two.cursor_row = Some(0);
        two.peek = peek;
        let mut one = app_graphite(fixture_tagged());
        one.cursor_col = 1;
        one.cursor_row = Some(0);
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

/// The editor's hardware cursor uses the same fixed inset as its title,
/// including in narrow columns and the ASCII fallback profiles.
#[test]
fn test_tag_count_keeps_the_title_editor_aligned() {
    use super::card;
    use crate::theme::{Flavor, Profile};
    let buffer = crate::text::EditBuffer::from_text("hello".into(), 100);
    for profile in
        [Profile::TrueColor, Profile::Ansi256, Profile::Ansi16, Profile::Ansi8, Profile::Mono]
    {
        let theme = crate::theme::Theme::new(Flavor::Graphite, profile);
        for width in [12, 29, 48] {
            let ctx = card::CardCtx { theme: &theme, width, now_ms: 0, spin: 0, names_key: true };
            let mut previous = None;
            for n in 0..=2 {
                let worn: Vec<_> =
                    (0..n).map(|tint| crate::tags::Painted { name: "tag".into(), tint }).collect();
                let (line, cursor) = card::render_edit(&ctx, &buffer, &worn);
                let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                let title_x = text.replace('▉', " ").find("hello").expect("title");
                assert_eq!(title_x, crate::tags::BAR_WIDTH + 1);
                assert!(line.width() <= width as usize);
                assert!(cursor < width);
                if let Some(prev) = previous {
                    assert_eq!(cursor, prev);
                }
                previous = Some(cursor);
            }
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
    app.cursor_row = Some(0);
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
    app.cursor_row = Some(0); // T-1 "Decay treatments": wears FTR, has no session.
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
    app.cursor_row = Some(0);
    app.peek = true;
    let lines = render(&app, 120, 30);
    let title = lines.iter().position(|l| l.contains("Fix OSC-11")).expect("card");
    let names = &lines[title + 1];
    // Four tags on a 28-cell card, with the key holding the row's last
    // cells (T-410): the long one gives up cells, the short ones keep
    // theirs, and the one that will not go drops from the tail — the mark
    // under the card still carries the count. The key is never cut.
    for tag in ["BUG", "ST", "auth"] {
        assert!(names.contains(tag), "{tag} went unnamed in {names:?}");
    }
    assert!(names.trim_end().ends_with("T-3"), "the key closes the row: {names:?}");
    let _ = std::fs::remove_dir_all(path.parent().expect("dir"));
}

/// The peek names the ticket too (T-410). A session says "T-3" and a resting
/// card never did, so the id was one focus away on every card. With `p` on,
/// the cursor card's meta row closes with its short key, right-aligned under
/// the age slot — tags or no tags — and under `P` every card's key lands in
/// that one column, the way the ages do, so a reader holding an id scans a
/// column rather than every row.
#[test]
fn test_the_peek_names_the_ticket() {
    let mut app = app_graphite(fixture_tagged());
    app.cursor_col = 1;
    app.cursor_row = Some(0);
    // Off: the key is nowhere on the board.
    let lines = render(&app, 120, 30);
    assert!(!lines.iter().any(|l| l.contains("T-3")), "no key before `p`");
    app.peek = true;
    let lines = render(&app, 120, 30);
    let title = lines.iter().position(|l| l.contains("Fix OSC-11")).expect("card");
    let row = &lines[title + 1];
    assert!(row.contains("BUG") && row.trim_end().ends_with("T-3"), "key after the chips: {row:?}");
    // Cells, not bytes: the glyph before the title is one cell of three bytes.
    let cells_to = |l: &str, end: usize| UnicodeWidthStr::width(&l[..end]);
    let at = lines[title].find("Fix OSC-11").expect("title");
    let age_end = cells_to(&lines[title], at + lines[title][at..].find(">1y").expect("age") + 3);
    let key_end = cells_to(row, row.rfind("T-3").expect("key") + 3);
    assert_eq!(key_end, age_end, "the key sits under the age slot");
    // `P`: a card with no tags still names itself, and the keys line up.
    app.peek_all = true;
    let lines = render(&app, 120, 30);
    let t2 = lines.iter().position(|l| l.contains("Keymap validator")).expect("untagged card");
    let row2 = &lines[t2 + 1];
    assert!(row2.contains("T-2"), "an untagged card still names itself: {row2:?}");
    let t1 = lines.iter().position(|l| l.contains("Decay treatments")).expect("card 1");
    let k1 = lines[t1 + 1].find("T-1").expect("key 1");
    let k2 = row2.find("T-2").expect("key 2");
    assert_eq!(k1, k2, "keys in one column under P");
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
    app.cursor_row = Some(0);
    app.peek = true;
    golden("board_tags_peek_120x30", &render(&app, 120, 30));
    let _ = std::fs::remove_dir_all(path.parent().expect("dir"));
}

/// `P` opens every card (T-237): the cursor is on the first column, and the
/// reply still shows under the card in the second — on the resting ramp, no
/// surface, the chips row above it — while the cursor card keeps its own
/// accordion. The session list stays the cursor card's.
#[test]
fn golden_board_peek_all_120() {
    let path = write_transcript(
        "peek-all-golden",
        &reply_record("Rebased onto main, tests green, ready to merge."),
    );
    let mut b = fixture_tagged();
    attach_transcript(&mut b, &path);
    let mut app = app_graphite(b);
    app.cursor_col = 0;
    app.cursor_row = Some(0);
    app.peek = true;
    app.peek_all = true;
    let lines = render(&app, 120, 30);
    let y = lines.iter().position(|l| l.contains("Rebased onto main")).expect("a reply row");
    assert!(
        !lines[y - 1].contains("Decay treatments"),
        "the reply hangs under its own card, not the cursor card"
    );
    let buf = cells(&app, 120, 30);
    let x = lines[y].find("Rebased").expect("reply") as u16;
    assert_ne!(
        Some(buf[(x, y as u16)].bg),
        app.theme.selected_bg,
        "a resting card's reply wears no cursor surface"
    );
    golden("board_peek_all_120x30", &lines);
    let _ = std::fs::remove_dir_all(path.parent().expect("dir"));
}

/// A session-less card's chips sit FLUSH under its title: there is no glyph
/// column on that card, so the title starts at the bar and the row follows it
/// (author 2026-09-01: "non session tickets tags line shouldn't be indent").
#[test]
fn golden_board_tags_peek_sessionless_120() {
    let mut app = app_graphite(fixture_tagged());
    app.cursor_col = 0;
    app.cursor_row = Some(0);
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
    app.cursor_row = Some(0);
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
    app.cursor_row = Some(0);
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
    app.cursor_row = Some(0);
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
            description: None,
            images: Vec::new(),
            plan: false,
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
    app.cursor_row = Some(0);
    let mut buffer = crate::text::EditBuffer::new(mesimon_core::board::TITLE_MAX_BYTES);
    for c in "rebase onto main".chars() {
        buffer.insert(c);
    }
    app.mode = Mode::Input {
        purpose: crate::app::InputPurpose::Prompt {
            target: crate::app::AskTarget::Ticket(ulid_n(3)),
            walk: None,
            queued: false,
            accept_plan: false,
            plan: false,
        },
        buffer,
    };
    let lines = render(&app, 120, 30);
    assert!(
        lines.iter().any(|l| l.contains("Fix OSC-11")),
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

/// The same field with its delivery at `queued` (2026-09-04): the row under
/// it says so, and Enter's word is `queue`.
#[test]
fn golden_prompt_field_queued_120() {
    let mut app = app_graphite(fixture(false));
    app.rich_keys = true;
    app.cursor_col = 1;
    app.cursor_row = Some(0);
    let mut buffer = crate::text::EditBuffer::new(mesimon_core::board::TITLE_MAX_BYTES);
    for c in "commit what you have".chars() {
        buffer.insert(c);
    }
    app.mode = Mode::Input {
        purpose: crate::app::InputPurpose::Prompt {
            target: crate::app::AskTarget::Ticket(ulid_n(3)),
            walk: None,
            queued: true,
            accept_plan: false,
            plan: false,
        },
        buffer,
    };
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("queued  shift+tab")), "{}", lines.join("\n"));
    assert!(lines.last().is_some_and(|l| l.contains("enter queue")), "{:?}", lines.last());
    golden("board_prompt_queued_120x30", &render(&app, 120, 30));
}

/// T-378: the same field on a COLUMN HEADER. The header stays whole and the
/// field hangs under it — the header is what names where the words go —
/// with the delivery row at `queued` under that, and every card below,
/// untouched: they are who is being asked.
#[test]
fn golden_column_prompt_field_120() {
    let mut app = app_graphite(fixture(false));
    app.rich_keys = true;
    // "in progress": T-3 and T-4 both carry a running claude.
    app.cursor_col = 1;
    app.cursor_row = None;
    assert_eq!(app.column_reach("in progress"), 2);
    let mut buffer = crate::text::EditBuffer::new(mesimon_core::command::PROMPT_MAX_BYTES);
    for c in "commit what you have".chars() {
        buffer.insert(c);
    }
    app.mode = Mode::Input {
        purpose: crate::app::InputPurpose::Prompt {
            target: crate::app::AskTarget::Column("in progress".into()),
            walk: None,
            queued: true,
            accept_plan: false,
            plan: false,
        },
        buffer,
    };
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("commit what you have")), "{}", lines.join("\n"));
    assert!(lines.iter().any(|l| l.contains("queued  shift+tab")), "{}", lines.join("\n"));
    for card in ["Fix OSC-11", "Adopt drawer"] {
        assert!(
            lines.iter().any(|l| l.contains(card)),
            "every card survives the field — they are who is asked:\n{}",
            lines.join("\n")
        );
    }
    let field = lines.iter().position(|l| l.contains("commit what you have")).unwrap();
    let first_card = lines.iter().position(|l| l.contains("Fix OSC-11")).unwrap();
    assert!(field < first_card, "the field hangs under the header, above the cards");
    assert!(lines.last().is_some_and(|l| l.contains("ASK")), "{:?}", lines.last());
    assert!(lines.last().is_some_and(|l| l.contains("enter queue")), "{:?}", lines.last());
    golden("board_column_prompt_120x30", &render(&app, 120, 30));
}

/// The same field empty and at `now`: the placeholder says what the key is
/// for in the key's own words, and the hardware cursor sits on it.
#[test]
fn golden_column_prompt_field_now_120() {
    let mut app = app_graphite(fixture(false));
    app.rich_keys = true;
    app.cursor_col = 1;
    app.cursor_row = None;
    app.mode = Mode::Input {
        purpose: crate::app::InputPurpose::Prompt {
            target: crate::app::AskTarget::Column("in progress".into()),
            walk: None,
            queued: false,
            accept_plan: false,
            plan: false,
        },
        buffer: crate::text::EditBuffer::new(mesimon_core::command::PROMPT_MAX_BYTES),
    };
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("ask every agent")), "{}", lines.join("\n"));
    assert!(lines.iter().any(|l| l.contains("now  shift+tab")), "{}", lines.join("\n"));
    assert!(lines.last().is_some_and(|l| l.contains("enter send")), "{:?}", lines.last());
    golden("board_column_prompt_now_120x30", &render(&app, 120, 30));
}

/// T-405: the same field on a column whose seats are EMPTY. The key is bound
/// there now — the press starts them — and the placeholder says what a blank
/// Enter does, the plural of the card's `start on the title`.
#[test]
fn golden_column_prompt_field_start_120() {
    let mut app = app_graphite(fixture(false));
    app.rich_keys = true;
    // "todo": T-1 and T-2, neither with a session.
    app.cursor_col = 0;
    app.cursor_row = None;
    assert!(app.column_starts("todo"));
    assert_eq!(
        mesimon_core::keymap::hint_for(
            mesimon_core::keymap::Scope::Board,
            mesimon_core::keymap::Verb::Prompt,
            &app.ctx()
        ),
        Some(("shift+enter", "ask every agent")),
        "a column of empty seats offers the key"
    );
    app.mode = Mode::Input {
        purpose: crate::app::InputPurpose::Prompt {
            target: crate::app::AskTarget::Column("todo".into()),
            walk: None,
            queued: true,
            accept_plan: false,
            plan: false,
        },
        buffer: crate::text::EditBuffer::new(mesimon_core::command::PROMPT_MAX_BYTES),
    };
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("start on the titles")), "{}", lines.join("\n"));
    assert!(lines.iter().any(|l| l.contains("queued  shift+tab")), "{}", lines.join("\n"));
    golden("board_column_prompt_start_120x30", &render(&app, 120, 30));
}

/// T-294: the same field on a ticket with NO claude, which is where the press
/// would otherwise have spawned one without asking. It opens at `queued`, and
/// the empty field says what a blank Enter does — the title is the prompt.
#[test]
fn golden_prompt_field_start_120() {
    let mut app = app_graphite(fixture(false));
    app.rich_keys = true;
    // T-1, no session on it at all.
    app.cursor_col = 0;
    app.cursor_row = Some(0);
    app.mode = Mode::Input {
        purpose: crate::app::InputPurpose::Prompt {
            target: crate::app::AskTarget::Ticket(ulid_n(1)),
            walk: None,
            queued: true,
            accept_plan: false,
            plan: false,
        },
        buffer: crate::text::EditBuffer::new(mesimon_core::command::PROMPT_MAX_BYTES),
    };
    let lines = render(&app, 120, 30);
    assert!(
        lines.iter().any(|l| l.contains("start on the title")),
        "the blank Enter names itself:\n{}",
        lines.join("\n")
    );
    assert!(lines.iter().any(|l| l.contains("queued  shift+tab")), "{}", lines.join("\n"));
    assert!(lines.last().is_some_and(|l| l.contains("enter queue")), "{:?}", lines.last());
    golden("board_prompt_start_120x30", &render(&app, 120, 30));
}

/// And the card says a SESSION is coming, not that words are waiting: a start
/// is louder than a paste, and it is what the queue is holding back.
#[test]
fn golden_queued_start_open_120() {
    let mut app = app_graphite(fixture(false));
    app.pending = vec![mesimon_core::command::Pending {
        ticket: ulid_n(1),
        action: mesimon_core::command::PendingAction::Start,
        waits_on: vec!["T-3".into()],
        text: None,
        in_flight: false,
        by: None,
        accept_plan: false,
        plan: false,
        held: None,
    }];
    app.cursor_col = 0;
    app.cursor_row = Some(0);
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("starts ∙ after T-3")), "{}", lines.join("\n"));
    golden("board_queued_start_120x30", &render(&app, 120, 30));
}

fn pending_ask(ticket: u128, waits_on: &[&str]) -> mesimon_core::command::Pending {
    mesimon_core::command::Pending {
        ticket: ulid_n(ticket),
        action: mesimon_core::command::PendingAction::Ask,
        waits_on: waits_on.iter().map(|s| s.to_string()).collect(),
        text: Some("commit what you have".into()),
        in_flight: false,
        by: None,
        accept_plan: false,
        plan: false,
        held: None,
    }
}

/// A ticket with an ask waiting wears the owed mark on its resting card —
/// over the still done-check, in the grey register, at the slow cadence.
#[test]
fn golden_queued_mark_120() {
    let mut app = app_graphite(fixture(false));
    app.pending = vec![pending_ask(5, &["T-3"])];
    let lines = render(&app, 120, 30);
    let mark = crate::glyphs::queued(crate::glyphs::Tier::Unicode, 0);
    assert!(
        lines.iter().any(|l| l.contains(mark) && l.contains("Grapheme truncation")),
        "T-5 wears the owed mark:\n{}",
        lines.join("\n")
    );
    golden("board_queued_120x30", &render(&app, 120, 30));
}

/// The cursor card opens on the owed row: what mesimon will do next to it,
/// and what it waits on.
#[test]
fn golden_queued_open_120() {
    let mut app = app_graphite(fixture(false));
    app.pending = vec![pending_ask(5, &["T-3"])];
    app.cursor_col = 2;
    app.cursor_row = Some(0);
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("queued ∙ after T-3")), "{}", lines.join("\n"));
    golden("board_queued_open_120x30", &render(&app, 120, 30));
}

/// The ticket page carries the same words, on the row under the state one
/// (T-346) — where there is no branch to join, the owed row opens it.
#[test]
fn golden_ticket_queued_120() {
    let mut app = app_graphite(fixture(false));
    app.pending = vec![pending_ask(5, &["T-3"])];
    app.screen = crate::app::Screen::Ticket { ticket: ulid_n(5), rail_idx: 0 };
    let lines = render(&app, 120, 30);
    let state = lines.iter().position(|l| l.contains("REVIEW")).expect("state row");
    assert_eq!(
        lines[state + 1].trim_end(),
        " queued ∙ after T-3 ∙ ^y send now ∙ ^u take back",
        "{}",
        lines.join("\n")
    );
    golden("ticket_queued_120x30", &render(&app, 120, 30));
}

/// The merge train armed (2026-09-04): a REVIEW card it will merge wears
/// the owed mark and, open, says so and names what it waits on; a card it
/// will ask to rebase says that. The HEADER says nothing — the train only
/// reaches attached worktree tickets, so it has no board-wide word.
#[test]
fn golden_train_120() {
    let mut app = app_graphite(fixture(false));
    app.automation.merge_train = true;
    app.pending = vec![
        mesimon_core::command::Pending {
            ticket: ulid_n(5),
            action: mesimon_core::command::PendingAction::Merge,
            waits_on: vec!["T-3".into(), "T-4".into()],
            text: None,
            in_flight: false,
            by: None,
            accept_plan: false,
            plan: false,
            held: None,
        },
        mesimon_core::command::Pending {
            ticket: ulid_n(6),
            action: mesimon_core::command::PendingAction::Rebase,
            waits_on: vec!["T-3".into(), "T-4".into()],
            text: None,
            in_flight: false,
            by: None,
            accept_plan: false,
            plan: false,
            held: None,
        },
    ];
    app.cursor_col = 2;
    app.cursor_row = Some(0);
    let lines = render(&app, 120, 30);
    assert!(
        !lines[0].contains("train") && !lines[0].contains("merge"),
        "an armed train adds no word to the header:\n{}",
        lines[0]
    );
    // The card is 22 cells: the count is what the row gives up first
    // (`auto-merge ∙ after T-3 ~`); the ticket page below carries it whole.
    assert!(lines.iter().any(|l| l.contains("auto-merge ∙ after T-3")), "{}", lines.join("\n"));
    let mark = crate::glyphs::queued(crate::glyphs::Tier::Unicode, 0);
    // T-6's claude FAILED: the error mark outranks the owed one, so the card
    // keeps its `x` and only the open row would say what the train owes.
    assert!(
        lines.iter().any(|l| l.contains("x Flaky e2e on runner") && !l.contains(mark)),
        "the error mark wins over the owed mark:\n{}",
        lines.join("\n")
    );
    golden("train_120x30", &render(&app, 120, 30));
}

/// The checkout refused the merge (T-289): the card that was promising one
/// says the door is shut instead, and the reason — a sentence, which no card
/// row can hold — is the advisory row's, in the same voice the flap fuse
/// uses. Before this the row read `auto-merge ∙ next` for as long as the
/// tree stayed dirty and nothing anywhere said why.
#[test]
fn golden_train_blocked_120() {
    let mut app = app_graphite(fixture(false));
    app.automation.merge_train = true;
    app.notices = vec![mesimon_core::command::Notice::new(
        "merge_train_blocked",
        "merge train held for T-5 — uncommitted changes in the main \
         checkout — commit or stash them first",
    )];
    app.pending = vec![mesimon_core::command::Pending {
        ticket: ulid_n(5),
        action: mesimon_core::command::PendingAction::Merge,
        waits_on: vec!["T-3".into()],
        text: Some("uncommitted changes in the main checkout — commit or stash them first".into()),
        in_flight: false,
        by: None,
        accept_plan: false,
        plan: false,
        held: None,
    }];
    app.cursor_col = 2;
    app.cursor_row = Some(0);
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("auto-merge ∙ blocked")), "{}", lines.join("\n"));
    assert!(
        !lines.iter().any(|l| l.contains("after T-3")),
        "a blocked merge is not waiting on the board:\n{}",
        lines.join("\n")
    );
    assert!(
        lines.iter().any(|l| l.contains("commit or stash them first")),
        "the advisory row says why:\n{}",
        lines.join("\n")
    );
    golden("train_blocked_120x30", &render(&app, 120, 30));
}

/// A pull request merged somewhere else (T-267): the branch is not an
/// ancestor of anything, and the ticket page still says the work landed —
/// naming the ref, because the user did not merge it here, and the commit
/// that carries it, because a squash is one commit and it can be looked at.
/// The card says only `⎇✓`: the mark already means this, and line 1 has no
/// room for a second word.
#[test]
fn golden_merged_upstream_120() {
    let mut app = app_graphite(fixture(false));
    app.worktrees = vec![mesimon_core::command::WorktreeItem {
        ticket: ulid_n(5),
        branch: "msmn/T-5-grapheme-truncation".into(),
        status: "attached".into(),
        merged: true,
        merged_in: "origin/main".into(),
        merged_oid: "1a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d".into(),
        conflict: false,
        ahead: 2,
        needs_rebase: false,
        detail: None,
        path: Some("/wt/T-5-grapheme-truncation".into()),
        repos: vec![],
    }];
    app.cursor_col = 2;
    app.cursor_row = Some(0);
    let board = render(&app, 120, 30);
    let check = crate::glyphs::branch_mark(crate::glyphs::Tier::Unicode);
    assert!(
        board.iter().any(|l| l.contains(&format!("{check}✓"))),
        "the card wears the merged mark:\n{}",
        board.join("\n")
    );
    app.screen = crate::app::Screen::Ticket { ticket: ulid_n(5), rail_idx: 0 };
    let lines = render(&app, 120, 30);
    assert!(
        lines.iter().any(|l| l.contains("merged into origin/main as 1a2b3c4")),
        "the page names where it landed:\n{}",
        lines.join("\n")
    );
    golden("ticket_merged_upstream_120x30", &lines);

    // A merge made here is still the bare word: `main` is the ref the whole
    // page is already about, and there is no one commit to name.
    app.worktrees[0].merged_in.clear();
    app.worktrees[0].merged_oid.clear();
    let lines = render(&app, 120, 30);
    assert!(
        lines.iter().any(|l| l.contains("∙ merged") && !l.contains("merged into")),
        "{}",
        lines.join("\n")
    );
}

/// The merge dialog (T-431) asks in a frame, and the confirmed merge says it
/// is merging there while it runs (T-352): the request rides a detached
/// connection, so the loop keeps drawing the spinner, and nothing on the
/// page still offers `m` for the merge — the row under the dialog drops its
/// offer while the dialog holds it.
#[test]
fn a_confirmed_merge_says_it_is_merging() {
    let mut app = app_graphite(fixture(false));
    app.worktrees = vec![mesimon_core::command::WorktreeItem {
        ticket: ulid_n(5),
        branch: "msmn/T-5-grapheme-truncation".into(),
        status: "attached".into(),
        merged: false,
        merged_in: String::new(),
        merged_oid: String::new(),
        conflict: false,
        ahead: 2,
        needs_rebase: false,
        detail: None,
        path: Some("/wt/T-5-grapheme-truncation".into()),
        repos: vec![],
    }];
    app.screen = crate::app::Screen::Ticket { ticket: ulid_n(5), rail_idx: 0 };
    press(&mut app, 'm');
    let lines = render(&app, 120, 30);
    let text = lines.join("\n");
    assert!(text.contains("MERGE ∙ T-5"), "the first press opens the dialog:\n{text}");
    assert!(text.contains("merge 2 commits of msmn/T-5-grapheme-truncation?"), "{text}");
    assert!(text.contains("m merge"), "the frame's edge names the key:\n{text}");
    assert!(text.contains("esc cancel"), "{text}");
    assert!(!text.contains("m confirms"), "{text}");
    press(&mut app, 'm');
    assert!(app.merge_in_flight(), "the key sends the merge detached and holds");
    let lines = render(&app, 120, 30);
    let text = lines.join("\n");
    assert!(text.contains("merging 2 commits of msmn/T-5-grapheme-truncation…"), "{text}");
    assert!(text.contains("stay on this page: when it lands, m tells the agent"), "{text}");
    assert!(
        !text.contains("m merge") && !text.contains("esc cancel"),
        "no key is offered while git works:\n{text}"
    );
}

/// A worktree ticket the train can reach offers `t merge by hand` in the
/// footer (T-227); taken off the train it wears no owed mark, its row reads
/// `auto-merge ∙ off`, the hint flips to `t auto-merge`, and the ticket page
/// says the same beside the branch — with the row's full words, which the
/// card cannot fit.
#[test]
fn golden_train_manual_120() {
    let mut app = app_graphite(fixture(false));
    app.automation.merge_train = true;
    app.worktrees = vec![mesimon_core::command::WorktreeItem {
        ticket: ulid_n(5),
        branch: "msmn/T-5-grapheme-truncation".into(),
        status: "attached".into(),
        merged: false,
        merged_in: String::new(),
        merged_oid: String::new(),
        conflict: false,
        ahead: 2,
        needs_rebase: false,
        detail: None,
        path: Some("/wt/T-5-grapheme-truncation".into()),
        repos: vec![],
    }];
    let candidate = mesimon_core::command::Pending {
        ticket: ulid_n(5),
        action: mesimon_core::command::PendingAction::Merge,
        waits_on: vec!["T-3".into(), "T-4".into()],
        text: None,
        in_flight: false,
        by: None,
        accept_plan: false,
        plan: false,
        held: None,
    };
    app.pending = vec![candidate.clone()];
    app.cursor_col = 2;
    app.cursor_row = Some(0);
    let lines = render(&app, 120, 30);
    assert!(lines.last().is_some_and(|l| l.contains("t merge by hand")), "{:?}", lines.last());
    app.screen = crate::app::Screen::Ticket { ticket: ulid_n(5), rail_idx: 0 };
    let lines = render(&app, 120, 30);
    assert!(
        lines.iter().any(|l| l.contains("∙ auto-merge ∙ after T-3 +1")),
        "{}",
        lines.join("\n")
    );
    assert!(lines.last().is_some_and(|l| l.contains("t merge by hand")), "{:?}", lines.last());

    // Off the train: the daemon lists nothing for it and the ticket says so.
    app.screen = crate::app::Screen::Board;
    app.pending.clear();
    app.board.ticket_mut(ulid_n(5)).unwrap().manual_merge = true;
    let lines = render(&app, 120, 30);
    let mark = crate::glyphs::queued(crate::glyphs::Tier::Unicode, 0);
    assert!(
        lines.iter().any(|l| l.contains("Grapheme trunca") && !l.contains(mark)),
        "nothing owed, no owed mark:\n{}",
        lines.join("\n")
    );
    assert!(lines.iter().any(|l| l.contains("auto-merge ∙ off")), "{}", lines.join("\n"));
    assert!(lines.last().is_some_and(|l| l.contains("t auto-merge")), "{:?}", lines.last());
    golden("train_manual_120x30", &lines);
    app.screen = crate::app::Screen::Ticket { ticket: ulid_n(5), rail_idx: 0 };
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("∙ auto-merge ∙ off")), "{}", lines.join("\n"));
    golden("ticket_train_manual_120x30", &lines);
    // The train switched off altogether: the mark stays (it is the
    // ticket's), and so does the key that clears it.
    app.automation.merge_train = false;
    app.seed_pref(|p| p.merge_train = false);
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("∙ auto-merge ∙ off")), "{}", lines.join("\n"));
    assert!(lines.last().is_some_and(|l| l.contains("t auto-merge")), "{:?}", lines.last());
}

/// An empty field says what it is for, in the same words the key was hinted
/// with — otherwise the state is a blank row under a card.
#[test]
fn test_an_empty_prompt_field_names_itself() {
    use mesimon_core::board::AgentProvider;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};

    for (kind, placeholder) in
        [(SessionKind::Claude, "› ask agent"), (SessionKind::Codex, "› ask agent")]
    {
        for provider in [AgentProvider::ClaudeCode, AgentProvider::Codex] {
            for state in [SessionState::Running, SessionState::Sleeping] {
                let mut board = fixture(false);
                board.agent_provider = provider;
                let agent = board.sessions.iter_mut().find(|s| s.id == uuid_n(31)).unwrap();
                agent.kind = kind;
                agent.state = state.clone();
                let mut app = app_graphite(board);
                app.rich_keys = true;
                app.cursor_col = 1;
                app.cursor_row = Some(0);
                app.handle_key(KeyCode::Enter, KeyModifiers::SHIFT).expect("open ask field");
                for width in [120, crate::layout::MIN_W] {
                    let lines = render(&app, width, 30);
                    assert!(
                        lines.iter().any(|l| l.contains(placeholder)),
                        "the prompt must name {kind:?} ({state:?}), with board default \
                         {provider:?} at {width}:\n{}",
                        lines.join("\n")
                    );
                    assert!(
                        lines.last().is_some_and(|l| {
                            l.contains("enter send") || l.contains("enter queue")
                        }),
                        "the field must not offer `save`: {:?}",
                        lines.last()
                    );
                    if kind == SessionKind::Codex
                        && provider == AgentProvider::ClaudeCode
                        && state == SessionState::Running
                        && width == 120
                    {
                        golden("prompt_field_codex_empty_120x30", &lines);
                    }
                }
            }
        }
    }
}

/// A field reopened on a WAITING ask and emptied says what a blank Enter
/// does (T-241): the delivery row used to carry ` ∙ blank enter drops` and a
/// narrow column cut it to `dr`. The word lives in the placeholder now, and
/// the delivery row is two words at any width.
#[test]
fn test_an_emptied_queued_ask_says_enter_drops() {
    let mut app = app_graphite(fixture(false));
    app.rich_keys = true;
    app.cursor_col = 1;
    app.cursor_row = Some(0);
    app.pending = vec![mesimon_core::command::Pending {
        ticket: ulid_n(3),
        action: mesimon_core::command::PendingAction::Ask,
        waits_on: vec!["T-1".into()],
        text: Some("commit it".into()),
        in_flight: false,
        by: None,
        accept_plan: false,
        plan: false,
        held: None,
    }];
    app.mode = Mode::Input {
        purpose: crate::app::InputPurpose::Prompt {
            target: crate::app::AskTarget::Ticket(ulid_n(3)),
            walk: None,
            queued: true,
            accept_plan: false,
            plan: false,
        },
        buffer: crate::text::EditBuffer::new(mesimon_core::board::TITLE_MAX_BYTES),
    };
    // The narrowest board there is: every column at MIN_COL.
    for w in [120u16, crate::layout::MIN_W] {
        let lines = render(&app, w, 30);
        assert!(
            lines.iter().any(|l| l.contains("› enter drops")),
            "an emptied queued ask must say how it is dropped at {w}:\n{}",
            lines.join("\n")
        );
        assert!(
            !lines.iter().any(|l| l.contains("blank enter")),
            "the delivery row no longer carries the clause at {w}:\n{}",
            lines.join("\n")
        );
    }
}

/// The prompt row costs the card no width. It hangs under the frame rather
/// than inside it, so nothing above it moves by a cell.
#[test]
fn test_the_prompt_field_moves_no_text() {
    let mut app = app_graphite(fixture(false));
    app.rich_keys = true;
    app.cursor_col = 1;
    app.cursor_row = Some(0);
    let before = render(&app, 120, 30);
    app.mode = Mode::Input {
        purpose: crate::app::InputPurpose::Prompt {
            target: crate::app::AskTarget::Ticket(ulid_n(3)),
            walk: None,
            queued: false,
            accept_plan: false,
            plan: false,
        },
        buffer: crate::text::EditBuffer::new(mesimon_core::board::TITLE_MAX_BYTES),
    };
    let after = render(&app, 120, 30);
    let card = before
        .iter()
        .position(|l| l.contains("Fix OSC-11"))
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

/// The chips LEAD the state row (T-346) and the ` ∙` separator belongs to
/// them: it went in before any chip was tried, so T-163's page read
/// `created 19m ago ∙ ∙ ⎇ msmn/…` (dogfood 2026-09-03). A bullet appears
/// only with a chip behind it, the rest of the row is what the chips give
/// way to, and the branch is no longer on this row at all.
#[test]
fn the_state_row_never_shows_an_empty_tag_bullet() {
    let mut app = app_graphite(fixture_tagged());
    app.screen = crate::app::Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    app.worktrees = vec![tagged_worktree()];
    for width in [120u16, 100, 80, 60] {
        let lines = render(&app, width, 30);
        let row = lines.iter().find(|l| l.contains("IN PROGRESS")).expect("state row");
        assert!(!row.contains("∙ ∙"), "{width}: empty bullet in {row:?}");
        assert!(!row.contains("∙  ∙"), "{width}: empty bullet in {row:?}");
        assert!(!row.contains('⎇'), "{width}: the branch is a row of its own: {row:?}");
        assert!(row.trim_end().width() < width as usize, "{width}: off the edge: {row:?}");
        // A chip only ever sits in front of the column it is about.
        if let Some(bug) = row.find(" BUG ") {
            let column = row.find("IN PROGRESS").unwrap_or(0);
            assert!(bug < column, "{width}: column before tags in {row:?}");
        }
    }
    // At 120 every chip fits, ahead of everything else the row says.
    let row = render(&app, 120, 30).into_iter().find(|l| l.contains("IN PROGRESS")).unwrap();
    assert!(row.starts_with("  BUG   STAGING  ∙ IN PROGRESS"), "{row:?}");
}

/// The workspace row is the branch's own (T-346), so the name gives way to
/// nothing but the merge state beside it — cut with the `~` marker, never
/// below its floor and never off the right edge.
#[test]
fn the_workspace_row_keeps_the_branch_name() {
    let mut app = app_graphite(fixture_tagged());
    app.screen = crate::app::Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    app.worktrees = vec![tagged_worktree()];
    for width in [120u16, 100, 80, 60] {
        let lines = render(&app, width, 30);
        let state = lines.iter().position(|l| l.contains("IN PROGRESS")).expect("state row");
        let row = &lines[state + 1];
        assert!(row.starts_with(" ⎇ msmn/T-3"), "{width}: the row opens with the branch: {row:?}");
        assert!(row.contains("∙ 2 to merge"), "{width}: the merge state is never cut: {row:?}");
        assert!(row.trim_end().width() < width as usize, "{width}: off the edge: {row:?}");
        // Wide enough for the floor and the merge state: the name is cut with
        // the marker rather than clipped.
        if width < 100 {
            assert!(row.contains("~ ∙ 2 to merge"), "{width}: not cut with the marker: {row:?}");
        }
    }
    // At 120 the whole slug fits, and the description-unread nudge and the
    // ages are still on the row above it.
    let lines = render(&app, 120, 30);
    let state = lines.iter().position(|l| l.contains("IN PROGRESS")).unwrap();
    assert!(lines[state + 1].contains("composer-drop-ta ∙ 2 to merge"), "{:?}", lines[state + 1]);
}

/// A long branch on a tagged ticket: the two facts that used to share the
/// state row.
fn tagged_worktree() -> mesimon_core::command::WorktreeItem {
    mesimon_core::command::WorktreeItem {
        ticket: ulid_n(3),
        branch: "msmn/T-3-tab-to-open-description-editing-like-ticket-composer-drop-ta".into(),
        status: "attached".into(),
        merged: false,
        merged_in: String::new(),
        merged_oid: String::new(),
        conflict: false,
        ahead: 2,
        needs_rebase: false,
        detail: None,
        path: Some("/wt/T-3".into()),
        repos: vec![],
    }
}

/// A workspace binding's legs on the ticket page (T-368): the aggregate
/// state reads after the name, and the legs with something to say sit
/// between them — `root +1 ∙ api +3` here, `web` untouched and silent.
#[test]
fn golden_ticket_workspace_wt_120() {
    let mut app = app_graphite(fixture(false));
    app.screen = Screen::Ticket { ticket: ulid_n(5), rail_idx: 0 };
    app.worktrees = vec![workspace_worktree()];
    let lines = render(&app, 120, 30);
    let state = lines.iter().position(|l| l.contains("REVIEW")).expect("state row");
    let row = &lines[state + 1];
    assert!(
        row.contains("⎇ msmn/T-5-grapheme-truncation ∙ root +1 ∙ api +3 ∙ 4 to merge"),
        "{row:?}"
    );
    assert!(!row.contains("web"), "an untouched leg is silent: {row:?}");
    golden("ticket_workspace_wt_120x30", &lines);
}

/// Only the legs with something to say are named, with the card's marks —
/// and the ASCII tier's spellings of them.
#[test]
fn the_workspace_row_names_only_the_legs_with_something_to_say() {
    use mesimon_core::command::WorktreeRepoItem;
    let mut app = app_graphite(fixture(false));
    app.screen = Screen::Ticket { ticket: ulid_n(5), rail_idx: 0 };
    let mut w = workspace_worktree();
    w.repos = vec![
        WorktreeRepoItem {
            name: "api".into(),
            base: "main".into(),
            conflict: true,
            ..Default::default()
        },
        WorktreeRepoItem {
            name: "web".into(),
            base: "main".into(),
            merged: true,
            ..Default::default()
        },
        WorktreeRepoItem {
            name: "infra".into(),
            base: "main".into(),
            needs_rebase: true,
            ..Default::default()
        },
        WorktreeRepoItem {
            name: "ops".into(),
            base: "main".into(),
            needs_rebase: true,
            ahead: 2,
            ..Default::default()
        },
        WorktreeRepoItem { name: "docs".into(), base: "main".into(), ..Default::default() },
    ];
    w.conflict = true;
    app.worktrees = vec![w];
    let row_of = |app: &App| {
        let lines = render(app, 120, 30);
        let state = lines.iter().position(|l| l.contains("REVIEW")).expect("state row");
        lines[state + 1].clone()
    };
    let row = row_of(&app);
    assert!(row.contains("∙ api ! ∙ web ✓ ∙ infra ↓ ∙ ops +2↓ ∙ branch shared!"), "{row:?}");
    assert!(!row.contains("docs"), "{row:?}");
    app.theme = Theme::new(Flavor::Graphite, Profile::Mono);
    let row = row_of(&app);
    assert!(
        row.contains("api ! ")
            && row.contains("web + ")
            && row.contains("infra v ")
            && row.contains("ops +2v"),
        "{row:?}"
    );
}

/// Nineteen legs fit no terminal: the legs give before the branch name,
/// kept from the left with the rest counted, and the name still keeps its
/// floor and the merge state is never cut.
#[test]
fn the_workspace_row_drops_legs_before_the_branch_name() {
    use mesimon_core::command::WorktreeRepoItem;
    let mut app = app_graphite(fixture_tagged());
    app.screen = crate::app::Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    let mut w = tagged_worktree();
    w.repos = (0..6)
        .map(|i| WorktreeRepoItem {
            name: format!("service-{i}"),
            base: "main".into(),
            ahead: 1,
            ..Default::default()
        })
        .collect();
    w.ahead = 6;
    app.worktrees = vec![w];
    for width in [120u16, 100, 80, 60] {
        let lines = render(&app, width, 30);
        let state = lines.iter().position(|l| l.contains("IN PROGRESS")).expect("state row");
        let row = &lines[state + 1];
        assert!(row.starts_with(" ⎇ msmn/T-3"), "{width}: {row:?}");
        assert!(row.contains("∙ 6 to merge"), "{width}: the merge state is never cut: {row:?}");
        assert!(row.trim_end().width() < width as usize, "{width}: off the edge: {row:?}");
        assert!(row.contains("∙ service-0 +1"), "{width}: the first leg is kept: {row:?}");
        if width <= 80 {
            assert!(row.contains("∙ +"), "{width}: dropped legs are counted: {row:?}");
        }
    }
    let row = render(&app, 120, 30).into_iter().find(|l| l.contains("∙ 6 to merge")).unwrap();
    assert!(row.contains("service-5 +1 ∙ 6 to merge"), "{row:?}");
}

/// T-5's workspace binding: the meta and two children, work in two of them.
fn workspace_worktree() -> mesimon_core::command::WorktreeItem {
    use mesimon_core::command::WorktreeRepoItem;
    mesimon_core::command::WorktreeItem {
        ticket: ulid_n(5),
        branch: "msmn/T-5-grapheme-truncation".into(),
        status: "attached".into(),
        merged: false,
        merged_in: String::new(),
        merged_oid: String::new(),
        conflict: false,
        ahead: 4,
        needs_rebase: false,
        detail: None,
        path: Some("/wt/T-5-grapheme-truncation".into()),
        repos: vec![
            WorktreeRepoItem {
                name: String::new(),
                base: "master".into(),
                ahead: 1,
                ..Default::default()
            },
            WorktreeRepoItem {
                name: "api".into(),
                base: "main".into(),
                ahead: 3,
                ..Default::default()
            },
            WorktreeRepoItem { name: "web".into(), base: "main".into(), ..Default::default() },
        ],
    }
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
    // footer is redrawn over it — the chord's mode word has to stay, and the
    // chord's keys are in the panel's own bottom edge (T-158).
    assert!(
        lines.last().is_some_and(|l| l.contains("TAG")),
        "the footer lost the chord's word:\n{joined}"
    );
    assert!(joined.contains("hjkl move"), "the panel lost the chord's hints:\n{joined}");
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
            description: None,
            images: Vec::new(),
            plan: false,
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
        // A palette declaring no `diff` keeps every row on the ground. No
        // shipped flavor does since the phosphors took a red `err`
        // (2026-09-03), but the seam is real and the arm says what it means.
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
        crate::app::TailKey::Session(uuid_n(32)),
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
    app.cursor_row = Some(0);
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
    app.cursor_row = Some(1); // T-4: exactly one (claude) session
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
    app.cursor_row = Some(0);
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
            // The diff screen across the same matrix, both single-pane swaps
            // and both targets — the checkout's has no header leaf, which is
            // the one width the branch's never exercises.
            for swap in [false, true] {
                for checkout in [false, true] {
                    let mut app = app_graphite(fixture(true));
                    if checkout {
                        install_checkout_diff(&mut app);
                    } else {
                        install_diff(&mut app);
                    }
                    if let Some(d) = app.diff.as_mut() {
                        d.swap = swap;
                    }
                    let _ = render(&app, w, h);
                }
            }
        }
    }
}

/// Graphite's accent, for the tests that render graphite alone. The two
/// provenance laws sweep every flavor's own `attn` instead.
const ATTN_GRAPHITE: Color = Color::Rgb(0xF0, 0xA9, 0x3A);

/// L3: on a calm board not one cell renders the saturated colour — on any
/// flavor. On a phosphor the ground, the bar and `calm` share the accent's
/// hue, so this and `attn_is_its_own_colour` are what keep "the one bright
/// thing" true there.
#[test]
fn test_attn_provenance_calm() {
    for flavor in Flavor::ALL {
        let theme = Theme::new(flavor, Profile::TrueColor);
        let attn = theme.attn;
        let mut app = App::for_test(fixture(false), theme);
        app.cursor_col = 1;
        // The checkout's arrows are calm, never attn (T-124).
        app.git = git_state("main", 2, 1, 3);
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
        attn_stays_on_the_waiting_card(flavor, fixture(true));
    }
}

/// L3 for the one ticket-level producer (T-74): a card back from a snooze
/// that asked to be lit wears attn on its own rows and lights the header
/// count, with no session behind it — and nowhere else. Same law, same
/// sweep, a different reason for the colour.
#[test]
fn test_attn_provenance_woke() {
    for flavor in Flavor::ALL {
        attn_stays_on_the_waiting_card(flavor, fixture_woke());
    }
    // The header counts it: one ticket, `!1`.
    let app = app_graphite(fixture_woke());
    let lines = render(&app, 120, 30);
    assert!(lines[0].contains("!1"), "the header must count the woke ticket: {}", lines[0]);
    // And its spine, collapsed, carries the mark too — the woke card is in
    // the column a narrow board folds.
    let mut narrow = app_graphite(fixture_woke());
    narrow.cursor_col = 0;
    let lines = render(&narrow, 60, 30);
    assert!(lines.iter().any(|l| l.contains('!')), "the folded column must show the mark");
}

/// L3 for the other ticket-level producer (T-107): a raised hand lights the
/// card and the header count with no attention SESSION behind it — T-5's
/// claude is idle after an end of turn — and the saturated colour appears
/// nowhere else. The same sweep the snooze's wake gets.
#[test]
fn test_attn_provenance_raised() {
    for flavor in Flavor::ALL {
        attn_stays_on_the_card(flavor, fixture_raised(), "Grapheme truncation");
    }
    let app = app_graphite(fixture_raised());
    let lines = render(&app, 120, 30);
    assert!(lines[0].contains("!1"), "the header must count the raised hand: {}", lines[0]);
}

/// T-271: the spine's `!` is the needs-you row's own inverted treatment —
/// the cell is PAINTED `attn` with `attn_ink` on it, not a coloured stroke
/// on the ground. One cell is the smallest mark the board makes, and a
/// folded column is standing in for every card inside it.
#[test]
fn the_folded_column_paints_its_needs_you_mark() {
    for flavor in Flavor::ALL {
        let theme = Theme::new(flavor, Profile::TrueColor);
        let (attn, ink) = (theme.attn, theme.attn_ink);
        let mut app = App::for_test(fixture_woke(), theme);
        app.cursor_col = 0; // fold the column the woke ticket is in
        let buf = cells(&app, 60, 30);
        let mut found = false;
        for x in 0..60u16 {
            let c = &buf[(x, 0u16)];
            if c.symbol() != "!" {
                continue;
            }
            found = true;
            assert_eq!(c.bg, attn, "{flavor:?}: the spine's ! sits on the beam");
            assert_eq!(c.fg, ink, "{flavor:?}: the spine's ! is written in the ink");
            assert!(
                c.modifier.contains(Modifier::BOLD),
                "{flavor:?}: the spine's ! is bold, like the header's !N chip"
            );
        }
        assert!(found, "{flavor:?}: the folded column must show the mark");
    }
}

/// T-302: a folded column's count reads at the TOP, on the very row every
/// expanded column writes its own count on — at the foot it was twenty rows
/// away from every other number on the board (user: "bottom too far"). The
/// `!` keeps that cell when the column is waiting, and the count takes the
/// row under it: the mark is what the folded column is standing in for.
#[test]
fn the_folded_column_reads_its_count_at_the_top() {
    // Row 2 is the column-header row: the expanded columns' counts sit on
    // it, and now so does the spine's.
    let mut calm = app_graphite(fixture(false));
    calm.cursor_col = 0;
    let lines = render(&calm, 100, 24);
    let row: Vec<char> = lines[2].chars().collect();
    let x = row.iter().rposition(|c| *c != ' ').expect("a count");
    assert_eq!(row[x], '1', "the folded DONE column's count: {}", lines[2]);
    assert!(
        lines[3..].iter().all(|l| !l.chars().nth(x).is_some_and(|c| c.is_ascii_digit())),
        "nothing is left at the foot: {lines:?}"
    );
    // And the name still runs down from under it, a letter a row.
    let letters: String = lines[4..8].iter().filter_map(|l| l.chars().nth(x)).collect();
    assert_eq!(letters, "DONE", "the name under the count: {letters}");

    // Waiting: the `!` holds row 0 and the count is the row beneath it.
    let mut waiting = app_graphite(fixture_woke());
    waiting.cursor_col = 0; // fold the column the woke ticket is in
    let lines = render(&waiting, 60, 30);
    let x = lines[2].find('!').expect("the folded column's mark");
    assert_eq!(lines[3].chars().nth(x), Some('2'), "the count under the mark: {}", lines[3]);
    let letters: String = lines[5..9].iter().filter_map(|l| l.chars().nth(x)).collect();
    assert_eq!(letters, "INPR", "the name under both: {letters}");
}

/// T-359: a folded column of ten or more reads `9` over `⁺`, one cell each,
/// where twelve used to stack `1` over `2` and push the name a row down.
#[test]
fn the_folded_column_caps_its_count_at_nine_plus() {
    let mut board = fixture(false);
    for n in 20..32u128 {
        board.tickets.push(ticket(n, &format!("T-{n}"), "Filler", "done", &format!("z{n}")));
    }
    let mut app = app_graphite(board);
    app.cursor_col = 0;
    let lines = render(&app, 100, 24);
    let row: Vec<char> = lines[2].chars().collect();
    let x = row.iter().rposition(|c| *c != ' ').expect("a count");
    assert_eq!(row[x], '9', "thirteen reads as 9: {}", lines[2]);
    assert_eq!(lines[3].chars().nth(x), Some('⁺'), "with the plus beneath: {}", lines[3]);
    // The name starts on the same row it does for a one-digit count.
    let letters: String = lines[4..8].iter().filter_map(|l| l.chars().nth(x)).collect();
    assert_eq!(letters, "DONE", "the name under the count: {letters}");
}

fn attn_stays_on_the_waiting_card(flavor: Flavor, board: Board) {
    attn_stays_on_the_card(flavor, board, "Adopt drawer import");
}

/// The L3 law, over whichever card earned the colour: the saturated register
/// appears on the header's count and on that card's own rows, and nowhere
/// else on the board. The producer is the caller's — an attention session, a
/// snooze's wake (T-74), a raised hand (T-107) — and the law is one law.
fn attn_stays_on_the_card(flavor: Flavor, board: Board, title: &str) {
    let theme = Theme::new(flavor, Profile::TrueColor);
    let attn = theme.attn;
    let mut app = App::for_test(board, theme);
    app.cursor_col = 0; // cursor away from the lit card
    let buf = cells(&app, 120, 30);
    // Rows that legally carry attn: the header (0) and the lit card's own.
    let lines = render(&app, 120, 30);
    let card_rows: Vec<usize> =
        lines.iter().enumerate().filter(|(_, l)| l.contains(title)).map(|(y, _)| y).collect();
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

/// The links dialog over T-3 (T-256): three kinds of row, the cursor on the
/// second. The list rides the mode, so no fetch and no disk.
fn app_links() -> App {
    use crate::app::{LinkTarget, TicketLink};
    let mut app = app_noted();
    app.mode = Mode::Links {
        ticket: ulid_n(3),
        links: vec![
            TicketLink {
                label: Some("the Jira ticket".into()),
                text: "https://jira.example.com/browse/ABC-123".into(),
                target: LinkTarget::Url("https://jira.example.com/browse/ABC-123".into()),
            },
            TicketLink { label: None, text: "T-1".into(), target: LinkTarget::Ticket(ulid_n(1)) },
            TicketLink {
                label: None,
                text: "crates/mesimon-core/src/board.rs:42".into(),
                target: LinkTarget::File {
                    path: "/repo/crates/mesimon-core/src/board.rs".into(),
                    line: Some(42),
                },
            },
        ],
        idx: 1,
    };
    app
}

#[test]
fn golden_links_120() {
    golden("links_120x30", &render(&app_links(), 120, 30));
}

/// The ticket page does NOT name `^k` on its state row (T-312) — not with a
/// link fetched, not without one. `?` is the key's one home, the way `!`'s
/// is (T-277); the state row says what the ticket is.
#[test]
fn the_ticket_page_never_names_the_links_key_on_its_state_row() {
    let mut app = app_noted();
    app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    let state_row = |app: &App| render(app, 120, 30)[3].clone();
    assert!(!state_row(&app).contains("^k"), "{}", state_row(&app));
    app.remember_note(ulid_n(91), 1, Some("see https://a.test/x".into()));
    assert!(!app.ticket_links(ulid_n(3)).is_empty(), "the fixture holds a link");
    assert!(!state_row(&app).contains("^k"), "{}", state_row(&app));
    // The key is still bound and still listed by `?`.
    assert!(mesimon_core::keymap::overlay(mesimon_core::keymap::Scope::Ticket, &app.ctx())
        .iter()
        .any(|(_, rows)| rows.contains(&("^k", "links"))));
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
            plan: false,
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
fn the_composer_dialog_grows_out_of_its_card() {
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
        purpose: crate::app::InputPurpose::Create {
            workspace: None,
            tags: Vec::new(),
            description: None,
            images: Vec::new(),
            plan: false,
        },
        buffer,
    };
    let before = render(&app, 120, 30);
    let card_row = before
        .iter()
        .position(|l| l.contains("Ship the diff viewer"))
        .expect("the phantom card is on the board");
    let card = app.cursor_card.get().expect("the draw records the phantom card");
    assert_eq!(card.y as usize, card_row);
    assert_eq!(card.x, crate::layout::LPAD, "the first column's bar cell");
    assert_eq!(card.height, 2, "title row + workspace selector");
    assert!(card.width < 60, "one column, not the board: {card:?}");

    // Tab grows it: the editor carries the card's rectangle as its origin.
    app.handle_key(KeyCode::Tab, KeyModifiers::NONE).expect("tab");
    let Mode::Editor(ed) = &app.mode else { panic!("tab opens the editor") };
    assert_eq!(ed.grow.map(|(r, _)| r), Some(card));
    assert!(app.animating(), "the frame after Tab is in motion");
    // Pin frame zero: date the grow a second into the future, so `elapsed`
    // saturates at zero however long a loaded machine takes to get to the
    // render below. Without this the test raced the 180 ms animation and
    // lost under a parallel `cargo test` (another session's build stalling
    // this process for tens of ms was enough for the dialog to have grown
    // past the card and put its frame on).
    if let Mode::Editor(ed) = &mut app.mode {
        ed.grow = Some((card, std::time::Instant::now() + std::time::Duration::from_secs(1)));
    }

    // Frame zero: the dialog IS the card's rectangle — the title sits in the
    // card's cells, the meta row under it where the card's was, and the rest
    // of the board is still on screen around it.
    let first = render(&app, 120, 30);
    assert!(first[card_row].starts_with("    Ship the diff viewer"), "{:?}", first[card_row]);
    assert!(first[card_row + 1].starts_with("    NEW TICKET"), "{:?}", first[card_row + 1]);
    assert!(
        first.iter().any(|l| l.contains("Fix OSC-11")),
        "the other columns show through while the dialog is small"
    );
    assert!(!first.iter().any(|l| l.contains("describe it")), "no body at the card's size");

    // Settled: the dialog covers the two middle columns whole, two rows
    // under the column headers — the title on its first row, the column
    // named on its second, the body hint under them — and the outer columns
    // show their cards complete on both sides of it.
    if let Mode::Editor(ed) = &mut app.mode {
        ed.grow = Some((card, std::time::Instant::now() - crate::app::GROW));
    }
    assert!(!app.animating());
    let after = render(&app, 120, 30);
    assert!(after[2].contains("TODO") && after[2].contains("IN PROGRESS"), "{:?}", after[2]);
    let title_at =
        after[4].find("Ship the diff viewer").expect("the title on the dialog's first row");
    let dialog_x = after[4][..title_at].chars().count() - 2;
    assert_eq!(dialog_x, 32, "one cell before the title, after the two-cell bar: {:?}", after[4]);
    // The frame's top edge names the dialog on the breathing row over the
    // cards; the context row under the title starts at the column.
    assert!(after[3].contains("NEW TICKET"), "{:?}", after[3]);
    assert!(after[5].contains("TODO ∙ ⎇"), "{:?}", after[5]);
    assert!(after[7].contains("describe it"), "{:?}", after[7]);
    for covered in ["Fix OSC-11 detection", "Grapheme truncation"] {
        assert!(!after.iter().any(|l| l.contains(covered)), "{covered} is under the dialog");
    }
    assert!(
        after[4].starts_with("    Decay treatments      >1y")
            && after[4].contains("z Painted accent bar >1y"),
        "whole cards on both sides, never a sliver: {:?}",
        after[4]
    );
    // The dialog's keys are in its own bottom edge; the footer under it
    // says only which mode is on (`?` is text in the editor, so no tail).
    assert!(after.iter().any(|l| l.contains("esc close")), "the frame names the editor's keys");
    assert_eq!(after[29].trim(), "NEW", "the footer under a dialog is its chip: {:?}", after[29]);
}

/// T-420: the ask room grown from a field at `accept plan` says so in its
/// empty body, not `ask agent` — the send there accepts the plan first.
#[test]
fn the_ask_room_at_accept_plan_says_what_the_send_does() {
    let mut app = app_graphite(fixture_tagged());
    app.cursor_col = 0;
    app.cursor_row = Some(0);
    let room = |accept_plan: bool| {
        editor_on(
            crate::app::EditorPurpose::Ask {
                target: crate::app::AskTarget::Ticket(ulid_n(3)),
                queued: true,
                accept_plan,
                plan: false,
            },
            "Fix OSC-11 detection",
            "",
        )
    };
    app.mode = Mode::Editor(room(true));
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("accept the plan")), "{lines:?}");
    assert!(!lines.iter().any(|l| l.contains("ask agent")), "{lines:?}");
    app.mode = Mode::Editor(room(false));
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("ask agent")), "{lines:?}");
}

#[test]
fn golden_editor_compose_tags_120() {
    let mut app = app_graphite(fixture_tagged());
    let ed = editor_on(
        crate::app::EditorPurpose::Compose { workspace: None, tags: Vec::new(), plan: false },
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

/// The note editor from the TICKET PAGE takes the screen; over the board
/// it is the dialog (`golden_editor_describe_120`).
#[test]
fn golden_editor_note_120() {
    let mut app = app_noted();
    app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
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
    app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    let long = "a line long enough to wrap within the editor while keeping the cursor visible, which is what the narrow golden is for, and then some more";
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
fn golden_editor_wrapped_description_and_visual_navigation() {
    use crate::app::{EditorPurpose, Field};
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    let mut app = app_graphite(fixture_tagged());
    app.cursor_col = 1;
    let body = "A description can be a long paragraph with several sentences. The editor wraps these words within the dialog and keeps the cursor on the same text when the terminal changes size.\n\n    Indentation and explicit line breaks stay in the saved note.\n你好 cafe\u{301} — Unicode text stays whole.";
    app.mode = Mode::Editor(editor_on(
        EditorPurpose::Compose { workspace: None, tags: Vec::new(), plan: false },
        "Wrap description text",
        body,
    ));
    golden("editor_wrapped_compose_120x30", &render(&app, 120, 30));
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal.draw(|f| super::draw(f, &app)).unwrap();
    let first = terminal.get_cursor_position().unwrap();
    app.handle_key(KeyCode::Down, KeyModifiers::NONE).unwrap();
    terminal.draw(|f| super::draw(f, &app)).unwrap();
    let second = terminal.get_cursor_position().unwrap();
    assert_eq!(second.x, first.x);
    assert_eq!(second.y, first.y + 1);
    // Up from a continuation stays in the body; only Up from its first
    // visual row returns a composer to its title.
    app.handle_key(KeyCode::Up, KeyModifiers::NONE).unwrap();
    let Mode::Editor(ed) = &app.mode else { panic!("editor") };
    assert_eq!(ed.focus, Field::Body);
    assert_eq!(ed.body.cursor(), 0);
    assert_eq!(ed.body.as_str(), body);
    app.handle_key(KeyCode::Up, KeyModifiers::NONE).unwrap();
    let Mode::Editor(ed) = &app.mode else { panic!("editor") };
    assert_eq!(ed.focus, Field::Title);

    app.mode = Mode::Editor(editor_on(
        EditorPurpose::Note { ticket: ulid_n(3), note: None },
        "Wrap description text",
        body,
    ));
    golden("editor_wrapped_description_120x30", &render(&app, 120, 30));

    app = app_noted();
    app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    app.mode = Mode::Editor(editor_on(
        EditorPurpose::Note { ticket: ulid_n(3), note: None },
        "Wrap description text",
        body,
    ));
    golden("editor_wrapped_note_100x24", &render(&app, 100, 24));
    app.handle_key(KeyCode::Down, KeyModifiers::NONE).unwrap();
    let Mode::Editor(ed) = &app.mode else { panic!("editor") };
    assert!(ed.body.cursor() > 0);
    assert_eq!(ed.body.cursor_line(), 0);
    assert_eq!(ed.body.as_str(), body);
}

/// `Tab` on a card: the description in the composer's dialog over the
/// board, the stripe wearing the ticket's own tags, the context row naming
/// the note and the ticket's workspace.
#[test]
fn golden_editor_describe_120() {
    let mut b = fixture_tagged();
    if let Some(t) = b.tickets.iter_mut().find(|t| t.id == ulid_n(3)) {
        t.notes.push(note_meta(90, "What changed", "local"));
        t.workspace = Some(mesimon_core::board::WorkspaceStrategy::Worktree);
    }
    let mut app = app_graphite(b);
    app.cursor_col = 1;
    let mut ed = editor_on(
        crate::app::EditorPurpose::Note { ticket: ulid_n(3), note: Some(ulid_n(90)) },
        "Fix OSC-11 detection",
        COMPOSE_BODY,
    );
    ed.body.end();
    app.mode = Mode::Editor(ed);
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("DESCRIPTION")), "the frame names it: {lines:?}");
    assert!(lines.iter().any(|l| l.contains("⎇ worktree ∙ edited by")), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("Decay treatments")), "the board shows through");
    golden("editor_describe_120x30", &lines);
}

/// T-440: on a ticket with three notes the dialog counts where it stands
/// — `NOTE 2/3` in the frame's top edge — and its bottom edge offers
/// `tab next note`. Dirty and refused, the context row's `unsaved` goes
/// from the dim register to full ink.
#[test]
fn golden_editor_note_ring_120() {
    let mut b = fixture_tagged();
    if let Some(t) = b.tickets.iter_mut().find(|t| t.id == ulid_n(3)) {
        t.notes.push(note_meta(90, "What changed", "local"));
        t.notes.push(note_meta(91, "Repro steps", "local"));
        t.notes.push(note_meta(92, "Follow-up", "local"));
    }
    let mut app = app_graphite(b);
    app.cursor_col = 1;
    let ed = editor_on(
        crate::app::EditorPurpose::Note { ticket: ulid_n(3), note: Some(ulid_n(91)) },
        "Fix OSC-11 detection",
        SECOND_NOTE,
    );
    app.mode = Mode::Editor(ed.clone());
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("NOTE 2/3")), "the frame counts: {lines:?}");
    assert!(lines.iter().any(|l| l.contains("tab next note")), "the edge offers it: {lines:?}");
    golden("editor_note_ring_120x30", &lines);

    let ink = match app.theme.selected_bg {
        Some(_) => app.theme.sel,
        None => app.theme.rest,
    };
    let unsaved_fg = |app: &App| {
        let buf = cells(app, 120, 30);
        let lines = render(app, 120, 30);
        lines.iter().enumerate().find_map(|(y, l)| {
            let ix = l.find("unsaved")?;
            Some(buf[(l[..ix].chars().count() as u16, y as u16)].fg)
        })
    };
    let mut dirty = ed;
    dirty.body.insert('x');
    app.mode = Mode::Editor(dirty.clone());
    assert_eq!(unsaved_fg(&app), Some(ink.dim2), "at rest the word is quiet");
    dirty.tab_refused = true;
    app.mode = Mode::Editor(dirty);
    assert_eq!(unsaved_fg(&app), Some(ink.base), "a refused tab says it in full");
}

/// The ask room (T-380): the one-line field's prompt in the composer's
/// dialog. The frame names who the words reach, the title row is the
/// ticket (read-only), the context row is the field's delivery row, and the
/// footer is the field's — `ASK`, `^s send`.
#[test]
fn golden_editor_ask_120() {
    let mut app = app_graphite(fixture(false));
    app.rich_keys = true;
    app.cursor_col = 1;
    app.cursor_row = Some(0);
    let mut ed = editor_on(
        crate::app::EditorPurpose::Ask {
            target: crate::app::AskTarget::Ticket(ulid_n(3)),
            queued: false,
            accept_plan: false,
            plan: false,
        },
        "Fix OSC-11 detection",
        "rebase onto main\n\nthen run the suite and report the first failure, nothing else",
    );
    ed.body.page(4);
    ed.body.end();
    app.mode = Mode::Editor(ed);
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("ASK AGENT")), "the frame names it: {lines:?}");
    assert!(lines.iter().any(|l| l.contains("Fix OSC-11 detection")), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("T-3 ∙ now")), "the delivery row: {lines:?}");
    assert!(lines.iter().any(|l| l.contains("rebase onto main")), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("report the first failure")), "{lines:?}");
    assert!(lines.last().is_some_and(|l| l.contains("ASK")), "{:?}", lines.last());
    assert!(lines.last().is_some_and(|l| l.contains("^s send")), "{:?}", lines.last());
    assert!(!lines.last().is_some_and(|l| l.contains("^S")), "no second send: {:?}", lines.last());
    golden("editor_ask_120x30", &lines);
}

/// A column's ask in the room: the header's name is the title, the context
/// row counts the seats the words reach, and `queued` is the toggle's
/// setting — `^s queue`.
#[test]
fn golden_editor_ask_column_120() {
    let mut app = app_graphite(fixture(false));
    app.rich_keys = true;
    app.cursor_col = 1;
    app.cursor_row = None;
    let mut ed = editor_on(
        crate::app::EditorPurpose::Ask {
            target: crate::app::AskTarget::Column("in progress".into()),
            queued: true,
            accept_plan: false,
            plan: false,
        },
        "IN PROGRESS",
        "commit what you have",
    );
    ed.body.end();
    app.mode = Mode::Editor(ed);
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("ASK EVERY AGENT")), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("2 agents ∙ queued  shift+tab")), "{lines:?}");
    assert!(lines.last().is_some_and(|l| l.contains("^s queue")), "{:?}", lines.last());
    golden("editor_ask_column_120x30", &lines);
}

/// `Tab` on a card grows the ticket's description out of that card, the
/// composer's own motion on a ticket that exists; a card with no
/// description gets the fresh note that becomes it.
#[test]
fn the_description_dialog_grows_out_of_the_card() {
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    let mut app = app_graphite(fixture(false));
    app.cursor_col = 0;
    app.cursor_row = Some(0);
    let before = render(&app, 120, 30);
    let card_row =
        before.iter().position(|l| l.contains("Decay treatments")).expect("T-1 is on the board");
    let card = app.cursor_card.get().expect("the draw records the cursor card");
    assert_eq!(card.y as usize, card_row);
    assert_eq!(card.x, crate::layout::LPAD, "the first column's bar cell");
    assert!(card.width < 60, "one column, not the board: {card:?}");

    app.handle_key(KeyCode::Tab, KeyModifiers::NONE).expect("tab");
    let Mode::Editor(ed) = &app.mode else { panic!("tab opens the editor: {:?}", app.mode) };
    assert!(matches!(ed.purpose, crate::app::EditorPurpose::Note { note: None, .. }));
    assert_eq!(ed.grow.map(|(r, _)| r), Some(card));
    assert!(app.animating());
    // Pin frame zero (see `the_composer_dialog_grows_out_of_its_card`).
    if let Mode::Editor(ed) = &mut app.mode {
        ed.grow = Some((card, std::time::Instant::now() + std::time::Duration::from_secs(1)));
    }

    // Frame zero is the card: the title in the card's cells, and with no
    // frame edge to carry it the context row says what the text is.
    let first = render(&app, 120, 30);
    assert!(first[card_row].starts_with("    Decay treatments"), "{:?}", first[card_row]);
    assert!(first.iter().any(|l| l.contains("Fix OSC-11")), "the board shows through");

    // Settled: the dialog over the middle columns, its frame's top edge
    // naming the note, the ticket's workspace on the context row, and the
    // body asking to be written.
    if let Mode::Editor(ed) = &mut app.mode {
        ed.grow = Some((card, std::time::Instant::now() - crate::app::GROW));
    }
    let after = render(&app, 120, 30);
    assert!(after[2].contains("TODO") && after[2].contains("IN PROGRESS"), "{:?}", after[2]);
    assert!(after[3].contains("NEW DESCRIPTION"), "{:?}", after[3]);
    assert!(after[4].contains("Decay treatments"), "{:?}", after[4]);
    assert!(after[5].contains("⎇ shared"), "{:?}", after[5]);
    assert!(after[7].contains("describe it"), "{:?}", after[7]);
    assert!(after.iter().any(|l| l.contains("Keymap validator")), "the first column is whole");
    assert!(!after.iter().any(|l| l.contains("tab needs you")), "the attention walk is gone");
}

/// The rail of a ticket with no session of its own (T-300): one row, the
/// offer, with the cursor on it — ahead of the note, which is the point. The
/// two spawn hints that used to sit under an empty rail (`c start claude ∙
/// s shell`) are gone: the row is the offer and `enter` is the press.
#[test]
fn golden_ticket_new_claude_120() {
    let mut b = fixture(false);
    if let Some(t) = b.tickets.iter_mut().find(|t| t.id == ulid_n(1)) {
        t.notes.push(note_meta(90, "What changed", "local"));
    }
    let mut app = app_graphite(b);
    app.remember_note(ulid_n(90), 1, Some(crate::peek::sanitize(RICH_REPLY)));
    app.screen = Screen::Ticket { ticket: ulid_n(1), rail_idx: 0 };
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("+ agent session")), "the offer: {lines:?}");
    assert!(lines.iter().any(|l| l.contains("enter start agent")), "and the press: {lines:?}");
    assert!(!lines.iter().any(|l| l.contains("s shell")), "the gated key is silent: {lines:?}");
    golden("ticket_new_claude_120x30", &lines);
    // The zone the row sits beside is no longer blank (T-308): the mark, the
    // press in the keymap's own words, and what the session would be.
    assert!(lines.iter().any(|l| l.contains("▀███████████████▀")), "the mark: {lines:?}");
    assert!(lines.iter().any(|l| l.contains("starts in the checkout")), "where: {lines:?}");
    assert!(
        lines.iter().any(|l| l.contains("types the ticket title into its box, and sends nothing")),
        "the contract this road keeps: {lines:?}"
    );
}

/// The same zone on a ticket whose column narrows what its claude gets
/// (T-117 x T-308): the clauses are read off the column live, so what the
/// board grants is what the preview says. A column that changes nothing says
/// nothing — the plain case is `golden_ticket_new_claude_120`.
#[test]
fn golden_ticket_new_claude_worktree_120() {
    let mut b = fixture(false);
    if let Some(c) = b.columns.iter_mut().find(|c| c.name == "todo") {
        c.settings.claude_mode = mesimon_core::board::ClaudeMode::Plan;
        c.settings.agent_tools = mesimon_core::board::AgentTools::Read;
    }
    if let Some(t) = b.tickets.iter_mut().find(|t| t.id == ulid_n(2)) {
        t.workspace = Some(mesimon_core::board::WorkspaceStrategy::Worktree);
    }
    let mut app = app_graphite(b);
    app.screen = Screen::Ticket { ticket: ulid_n(2), rail_idx: 0 };
    let lines = render(&app, 120, 30);
    assert!(
        lines
            .iter()
            .any(|l| l.contains("starts in a worktree of its own ∙ plan mode ∙ read tools")),
        "the column's own grant: {lines:?}"
    );
    assert!(
        !lines.iter().any(|l| l.contains("already writing")),
        "a worktree ticket shares no checkout: {lines:?}"
    );
    golden("ticket_new_claude_worktree_120x30", &lines);
}

/// The other blank the zone had (T-308): a session selected with nothing to
/// read. A claude that has not been prompted yet wears the seat's own mark —
/// the conversation has not started, which is the same thing the offer row
/// said one press ago — and the words say what is in its box and whose Enter
/// it waits on.
#[test]
fn golden_ticket_starting_120() {
    let mut b = fixture(false);
    b.sessions.retain(|s| s.ticket != ulid_n(1));
    let mut s = session(
        11,
        ulid_n(1),
        SessionKind::Claude,
        SessionState::Idle { stop_reason: mesimon_core::board::StopReason::Unknown },
    );
    // The launching window: the daemon has typed the title and still owes the
    // Enter (`pending_submit`, T-224's retry clock).
    s.pending_submit = true;
    b.sessions.push(s);
    let mut app = app_graphite(b);
    app.screen = Screen::Ticket { ticket: ulid_n(1), rail_idx: 0 };
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("▀███████████████▀")), "the mark: {lines:?}");
    assert!(lines.iter().any(|l| l.contains("starting up")), "the state: {lines:?}");
    assert!(
        lines.iter().any(|l| l.contains("mesimon presses enter when it is ready")),
        "whose Enter it waits on: {lines:?}"
    );
    golden("ticket_starting_120x30", &lines);
}

/// Why there is nothing to read, per state — read as words, without a frame.
/// The rail's own vocabulary is reused wherever nothing better is true: this
/// zone may not invent a second name for a state the card already names.
#[test]
fn the_quiet_zone_says_why_there_are_no_words() {
    use mesimon_core::board::{ExitReason, Reason, StopReason};
    let claude = |state| session_record(11, ulid_n(1), SessionKind::Claude, state);
    let words = |s: &SessionRecord| super::ticket::quiet_words(s);

    // Before the first turn: the box, and whose Enter it waits on.
    let mut fresh = claude(SessionState::Idle { stop_reason: StopReason::Unknown });
    assert_eq!(words(&fresh).0, "waiting for you");
    assert_eq!(words(&fresh).1, vec!["the ticket title is in its box, unsent".to_string()]);
    fresh.pending_submit = true;
    assert_eq!(words(&fresh).0, "starting up");
    assert!(words(&fresh).1[0].contains("mesimon presses enter"));

    // A finished turn with no readable words is NOT a fresh box: it keeps the
    // rail's word and says what is missing instead.
    let done = claude(SessionState::Idle { stop_reason: StopReason::EndTurn });
    assert_eq!(words(&done).0, "done");
    assert_eq!(words(&done).1, vec!["it left no transcript".to_string()]);

    // A turn in flight gives the row to the pulse.
    assert_eq!(words(&claude(SessionState::Running)).0, "");
    assert!(words(&claude(SessionState::Running)).1[0].starts_with("nothing said yet"));

    // Needs-you: the question itself, which the 26-cell rail row cannot hold.
    let mut asked = claude(SessionState::RequiresAction { reason: Reason::Permission });
    asked.detail = Some("Bash(rm -rf node_modules)".into());
    assert_eq!(words(&asked).0, "Bash(rm -rf node_modules)");

    // A sleeper with no conversation: waking it starts a fresh one, which is
    // worth knowing BEFORE the press.
    let asleep = claude(SessionState::Sleeping);
    assert!(words(&asleep).1[0].contains("waking it starts a fresh one"));
    let mut slept = claude(SessionState::Sleeping);
    slept.transcript_path = Some("/t.jsonl".into());
    assert_eq!(words(&slept).1, vec!["nothing to read in its transcript".to_string()]);

    // A shell keeps no transcript, so "nothing to read" is never news about
    // one: what it means is the pane has not been captured yet.
    let shell = |state| session_record(12, ulid_n(1), SessionKind::Bash, state);
    assert_eq!(words(&shell(SessionState::Running)).0, "reading its pane");
    assert!(words(&shell(SessionState::Sleeping)).1[0].contains("the pane was the record"));

    // And the corpse the rail already offers to resume.
    let dead = claude(SessionState::Exited { reason: ExitReason::UserQuit });
    assert_eq!(words(&dead).0, "exited");
}

/// The picture is what the zone gives up first: when its full drawing and the text cannot fit the
/// words stay whole and the mark goes, because the sentence is what the
/// press needs and the art is what it earns.
#[test]
fn the_empty_seat_drops_its_mark_before_its_words() {
    let mut b = fixture(false);
    if let Some(t) = b.tickets.iter_mut().find(|t| t.id == ulid_n(1)) {
        t.notes.push(note_meta(90, "What changed", "local"));
    }
    let mut app = app_graphite(b);
    app.remember_note(ulid_n(90), 1, Some(crate::peek::sanitize(RICH_REPLY)));
    app.screen = Screen::Ticket { ticket: ulid_n(1), rail_idx: 0 };
    let tall = render(&app, 120, 30);
    assert!(tall.iter().any(|l| l.contains("▀███████████████▀")), "the mark fits at 30: {tall:?}");
    // A description takes the rows off the top of the zone, which is the
    // ordinary way it runs out.
    let short = render(&app, 120, 20);
    assert!(!short.iter().any(|l| l.contains("▀███████████████▀")), "the mark goes: {short:?}");
    assert!(short.iter().any(|l| l.contains("enter start agent")), "the press stays: {short:?}");
    assert!(
        short.iter().any(|l| l.contains("starts in the checkout")),
        "and so do the words: {short:?}"
    );
}

#[test]
fn the_shin_is_static_grey_and_only_precedes_a_conversation() {
    let mut app = app_graphite(fixture(false));
    app.screen = Screen::Ticket { ticket: ulid_n(1), rail_idx: 0 };
    let buf = cells(&app, 120, 30);
    let rect = app.mascot.borrow().expect("the empty seat has a mascot");
    for (dy, row) in crate::mascot::COMPACT.lines().enumerate() {
        for (dx, ch) in row.chars().enumerate() {
            if ch != ' ' {
                let cell = &buf[(rect.x + dx as u16, rect.y + dy as u16)];
                assert_eq!(cell.symbol(), ch.to_string());
                assert_eq!(Some(cell.fg), app.theme.dim1().fg);
            }
        }
    }
    app.spin_epoch.set(Some(std::time::Instant::now() - std::time::Duration::from_secs(10)));
    assert_eq!(buf, cells(&app, 120, 30), "the mascot never animates");

    app.theme = Theme::new(Flavor::Graphite, Profile::Mono);
    let lines = render(&app, 120, 30);
    assert!(app.mascot.borrow().is_none());
    assert!(!lines.iter().any(|row| row.contains('█') || row.contains('▄')));
    assert!(
        lines.iter().any(|row| row.trim_start().starts_with("mesimon")),
        "mono gets the wordmark"
    );

    app.theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
    app.board.sessions.retain(|s| s.ticket != ulid_n(1));
    for (state, expected) in [
        (SessionState::Spawning, true),
        (SessionState::Idle { stop_reason: mesimon_core::board::StopReason::Unknown }, true),
        (SessionState::Running, false),
        (SessionState::Sleeping, false),
        (SessionState::Idle { stop_reason: mesimon_core::board::StopReason::EndTurn }, false),
    ] {
        app.board.sessions.retain(|s| s.ticket != ulid_n(1));
        app.board.sessions.push(session(11, ulid_n(1), SessionKind::Claude, state.clone()));
        let _ = cells(&app, 120, 30);
        assert_eq!(app.mascot.borrow().is_some(), expected, "{state:?}");
    }
    app.screen = Screen::Board;
    let _ = cells(&app, 120, 30);
    assert!(app.mascot.borrow().is_none(), "a previous preview never exempts the board");
}

/// EXACTLY one rail row wears the cursor surface, wherever `rail_idx` sits.
/// A phantom row makes that easy to break and the goldens cannot see it —
/// they are colourless — which is how T-300 shipped painting the offer and
/// the first note together, and then nothing at all one press down: the note
/// rows were offsetting by the SESSION count while `rail_rows` (and so
/// `rail_idx`) counted the offer between them. Walked over both shapes: a
/// ticket with no session, and one with two.
#[test]
fn exactly_one_rail_row_wears_the_cursor() {
    let sel_bg = {
        let t = Theme::new(Flavor::Graphite, Profile::TrueColor);
        t.selected_bg.expect("truecolor paints the cursor row")
    };
    // The rail's own cells, sampled at its right edge, which every selected
    // row pads out to. The left zone can elevate a code block on the same
    // ground, so the sample never leaves the rail; the ticket's own band
    // above the zones and the footer band below them are on that surface
    // too, so the window is the rail's list and nothing else.
    let lit = |app: &App| -> Vec<u16> {
        let buf = cells(app, 120, 30);
        let head = (0..30u16)
            .find(|y| {
                (0..120u16).map(|x| buf[(x, *y)].symbol()).collect::<String>().contains("SESSIONS")
            })
            .expect("the rail's heading");
        (head + 1..29u16).filter(|y| buf[(117u16, *y)].bg == sel_bg).collect()
    };
    let one_row = |app: &App, idx: usize, what: &str| {
        let rows = lit(app);
        assert_eq!(rows.len(), 1, "{what} at rail_idx {idx}: rows {rows:?}");
    };
    // No session: the offer is row 0 and the note is row 1.
    let mut b = fixture(false);
    if let Some(t) = b.tickets.iter_mut().find(|t| t.id == ulid_n(1)) {
        t.notes.push(note_meta(90, "What changed", "local"));
        t.notes.push(note_meta(91, "Repro steps", "local"));
    }
    let mut app = app_graphite(b);
    for idx in 0..3 {
        app.screen = Screen::Ticket { ticket: ulid_n(1), rail_idx: idx };
        one_row(&app, idx, "offer, then two notes");
    }
    // And the offer sits between the sessions and the notes, so a ticket
    // with sessions offsets by both.
    let mut b = fixture(false);
    b.sessions.retain(|s| s.ticket != ulid_n(3) || s.kind == SessionKind::Bash);
    if let Some(t) = b.tickets.iter_mut().find(|t| t.id == ulid_n(3)) {
        t.notes.push(note_meta(90, "What changed", "local"));
    }
    let mut app = app_graphite(b);
    assert!(app.new_agent_row(ulid_n(3)), "a shell does not fill the agent seat");
    for idx in 0..3 {
        app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: idx };
        one_row(&app, idx, "one shell, the offer, one note");
    }
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

/// The state row says `description unread` exactly while there is something
/// to have read, a claude that has had a turn, and no record of it reading
/// (T-224). Reading it — `get_ticket`, or the composed spawn's paste — takes
/// the clause off; a claude still launching has not skipped anything yet; a
/// ticket with no description has nothing to skip.
#[test]
fn test_description_unread_follows_the_record() {
    let row = |app: &App| {
        render(app, 120, 30)
            .into_iter()
            .find(|l| l.contains("IN PROGRESS for"))
            .expect("the state row")
    };
    let mut app = app_noted();
    app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    assert!(row(&app).contains("description unread"), "{}", row(&app));

    // The record says it read the ticket.
    let mut read = app_noted();
    read.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    for s in read.board.sessions.iter_mut().filter(|s| s.ticket == ulid_n(3)) {
        s.ticket_read = true;
    }
    assert!(!row(&read).contains("unread"), "{}", row(&read));

    // Not yet prompted: nothing has been skipped.
    let mut launching = app_noted();
    launching.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    for s in launching.board.sessions.iter_mut().filter(|s| s.ticket == ulid_n(3)) {
        s.state = SessionState::Spawning;
    }
    assert!(!row(&launching).contains("unread"), "{}", row(&launching));

    // No description: nothing to read.
    let mut bare = app_graphite(fixture(false));
    bare.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    assert!(!row(&bare).contains("unread"), "{}", row(&bare));
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

/// The cursor on the DESCRIPTION row: the band's excerpt has gone, the zone
/// reads the whole thing under its own heading, and the rail has not moved
/// (T-344).
#[test]
fn golden_ticket_description_selected_120() {
    let mut app = app_noted();
    app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 2 };
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("DESCRIPTION")), "the zone's heading: {lines:?}");
    golden("ticket_description_selected_120x30", &lines);
}

/// The description is on the page ONCE. The band's excerpt is CONTEXT for
/// whatever the zone is reading; the moment the zone is reading the notes —
/// and the first of them IS the description — the band would be holding a
/// truncated copy of the words the zone has whole, which is what T-344
/// filed. It gives its rows to the zone there, and only its rows: the rail
/// keeps its `y`, so the row under the cursor does not move as the cursor
/// walks into the notes.
#[test]
fn the_description_leaves_the_band_while_a_note_is_read() {
    let sentence = "The OSC-11 query now runs once";
    let bar = " \u{258e} ";
    let mut app = app_noted();
    let shot = |app: &mut App, idx: usize, w: u16, h: u16| -> Vec<String> {
        app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: idx };
        render(app, w, h)
    };
    let says = |lines: &[String], needle: &str| lines.iter().filter(|l| l.contains(needle)).count();
    let at = |lines: &[String], needle: &str| {
        lines
            .iter()
            .position(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("{needle} is nowhere: {lines:?}"))
    };
    // The zone's heading opens its own row (the zone starts one cell in),
    // which is what tells it from the rail's `NOTES` further along the line.
    let head_at = |lines: &[String], word: &str| {
        let head = format!("  {word}");
        lines
            .iter()
            .position(|l| l.starts_with(&head))
            .unwrap_or_else(|| panic!("no {word} heading: {lines:?}"))
    };

    // A session row: the excerpt is the band's, under the state line, and
    // the zone beside it is the session's.
    let session = shot(&mut app, 0, 120, 30);
    assert_eq!(says(&session, sentence), 1, "the excerpt, once: {session:?}");
    assert!(session.iter().any(|l| l.starts_with(bar)), "the block's bar: {session:?}");
    assert_eq!(head_at(&session, "PREVIEW"), 13, "{session:?}");

    // The description row: the same words, once, in the zone — headed by
    // the role the band used to carry, with no bar row left above.
    let desc = shot(&mut app, 2, 120, 30);
    assert_eq!(says(&desc, sentence), 1, "the description, once: {desc:?}");
    assert!(!desc.iter().any(|l| l.starts_with(bar)), "the block is gone: {desc:?}");
    assert_eq!(head_at(&desc, "DESCRIPTION"), 6, "the zone took the band's rows: {desc:?}");

    // Any other note reads the same way — one shape for the whole list —
    // and the description is nowhere on the page while it does.
    let note = shot(&mut app, 3, 120, 30);
    assert_eq!(says(&note, sentence), 0, "no excerpt over another note: {note:?}");
    assert_eq!(head_at(&note, "NOTE"), 6, "{note:?}");

    // The rail stays where the eye left it, and the rows the band gave up
    // come back to the zone on the left.
    for (idx, lines) in [(2usize, &desc), (3, &note)] {
        assert_eq!(
            at(lines, "SESSIONS"),
            at(&session, "SESSIONS"),
            "the rail moved at rail_idx {idx}: {lines:?}"
        );
    }

    // The rail names the first note by its ROLE — its own name is its
    // body's first line, which is what the band and the zone both show —
    // and every other note by its name.
    assert!(desc.iter().any(|l| l.contains("\u{2261} description")), "{desc:?}");
    assert!(desc.iter().any(|l| l.contains("\u{2261} Repro steps")), "{desc:?}");

    // Narrow: there is no zone to read a note in, so the band keeps the
    // excerpt whatever the rail's cursor is on.
    let narrow = shot(&mut app, 2, 100, 24);
    assert_eq!(says(&narrow, sentence), 1, "the excerpt stays in one zone: {narrow:?}");
    assert!(narrow.iter().any(|l| l.starts_with(bar)), "{narrow:?}");

    // And a ticket whose description has not been fetched keeps the
    // geometry it always had: an empty block gives nothing up.
    app.notes.clear();
    let bare = shot(&mut app, 2, 120, 30);
    assert_eq!(
        at(&bare, "SESSIONS"),
        head_at(&bare, "DESCRIPTION"),
        "nothing to give up: {bare:?}"
    );
}

/// The ticket page's header section — title, state line, description — is
/// ONE band on the elevated surface (author 2026-09-03): the chip row above
/// it and the zones below it stay on the page ground, and the description
/// inside it is the card's body, with the neutral bar down its left edge.
#[test]
fn test_ticket_header_section_is_a_band() {
    let mut app = app_noted();
    app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    let buf = cells(&app, 120, 30);
    let lines = lines_of(&buf);
    let elevated = app.theme.selected_bg.expect("graphite paints an elevation");
    let title_y = lines.iter().position(|l| l.contains("Fix OSC-11 detection")).expect("title");
    assert_eq!(title_y, 2, "the title is the band's first written row");
    let sessions_y = lines.iter().position(|l| l.contains("SESSIONS")).expect("zones");
    // One band, from the pad under the header to the pad under the
    // description, painted edge to edge.
    for y in 1..sessions_y - 1 {
        for x in [0u16, 3, 119] {
            assert_eq!(buf[(x, y as u16)].bg, elevated, "band row {y} cell {x} unpainted");
        }
    }
    // The breadcrumb row, the breathing row and the zones are on the ground.
    for y in [0, sessions_y - 1, sessions_y] {
        assert_ne!(buf[(40, y as u16)].bg, elevated, "row {y} is on the ground");
    }
    // The description is the card's body inside the band: one blank row
    // under the state line, then rows carrying the NEUTRAL cursor-weight bar
    // in column 1 — a quarter-cell glyph in the bar's colour on the band, not
    // a painted cell (author 2026-09-03: "reduce thickness") — and their text
    // from column 3.
    let first = lines.iter().position(|l| l.contains("What changed")).expect("description");
    let last = lines.iter().position(|l| l.contains("the goldens moved")).expect("last row");
    assert_eq!(first, 5, "one blank row between the state line and the body");
    let (bar_ch, bar_style) = app.theme.desc_bar();
    assert_eq!(bar_ch, '▎', "the thin bar is the quarter block");
    for y in first..=last {
        assert_eq!(buf[(1, y as u16)].symbol(), bar_ch.to_string(), "row {y} has the bar");
        assert_eq!(Some(buf[(1, y as u16)].fg), bar_style.fg, "row {y}'s bar is the neutral bar");
        assert_eq!(buf[(1, y as u16)].bg, elevated, "row {y}'s bar sits on the band");
    }
    assert_eq!(buf[(1, 4)].symbol(), " ", "the blank row over the body carries no bar");
    assert!(lines[first].starts_with(" ▎ What changed"), "{:?}", lines[first]);
    // A code span sinks to the page ground rather than vanishing into the band.
    let code_y = lines.iter().position(|l| l.contains("detect.rs")).expect("a code row");
    let code_x = lines[code_y].find("detect").map(|b| lines[code_y][..b].chars().count()).unwrap();
    assert_eq!(Some(buf[(code_x as u16, code_y as u16)].bg), app.theme.bg);
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
    assert!(with[29].contains("? keys"), "the footer stays put: {}", with[29]);
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
        // A card with an unread reply: the heavy done mark, calm.
        seed_spoke(&mut app, ulid_n(5));
        let mut arch = App::for_test(fixture_archived(), Theme::new(flavor, profile));
        arch.mode = Mode::Archived { idx: 0 };
        let mut picker = App::for_test(fixture(false), Theme::new(flavor, profile));
        picker.mode = Mode::Theme { idx: 2 };
        // The snooze chord armed on a woke board: the open card's preset row
        // and the ticket-level attn mark, both new paint (T-74).
        let mut armed = App::for_test(fixture_woke(), Theme::new(flavor, profile));
        armed.cursor_col = 0;
        armed
            .handle_key(
                ratatui::crossterm::event::KeyCode::Char('z'),
                ratatui::crossterm::event::KeyModifiers::NONE,
            )
            .unwrap();
        // The ticket page renders markdown on two surfaces, and since T-344
        // they are never on screen together: the band's description block
        // while the zone reads something that is not a note, and the note
        // itself in the zone, which is where the band's rows went.
        let mut n = App::for_test(fixture(false), Theme::new(flavor, profile));
        if let Some(t) = n.board.tickets.iter_mut().find(|t| t.id == ulid_n(3)) {
            t.notes.push(note_meta(90, "What changed", "local"));
            t.notes.push(note_meta(91, "tree", "local"));
        }
        n.remember_note(ulid_n(90), 1, Some(crate::peek::sanitize(RICH_REPLY)));
        n.remember_note(ulid_n(91), 1, Some(crate::peek::sanitize(&dirty_tail().join("\n"))));
        n.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
        assert!(
            render(&n, 120, 30).iter().any(|l| l.contains("What changed")),
            "description on screen"
        );
        let desc_band = cells(&n, 120, 30);
        n.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 3 };
        assert!(render(&n, 120, 30).iter().any(|l| l.contains("in 2.4s")), "note on screen");
        let note_zone = cells(&n, 120, 30);
        for buf in [
            {
                assert!(
                    render(&app, 120, 30).iter().any(|l| l.contains(DONE_UNREAD)),
                    "the unread done mark must be ON SCREEN, or this law does not bite"
                );
                cells(&app, 120, 30)
            },
            cells(&arch, 120, 30),
            cells(&picker, 120, 30),
            {
                assert!(
                    render(&armed, 120, 30).iter().any(|l| l.contains("snooze 1h")),
                    "the armed snooze must be ON SCREEN, or this law does not bite"
                );
                cells(&armed, 120, 30)
            },
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
                app.shell_tail = Some(crate::app::ShellTail::new(
                    crate::app::TailKey::Session(uuid_n(32)),
                    dirty_tail(),
                ));
                assert!(
                    render(&app, 120, 30).iter().any(|l| l.contains("in 2.4s")),
                    "the whole shell tail must be ON SCREEN, or this law does not bite"
                );
                cells(&app, 120, 30)
            },
            {
                // The empty seat's own preview (T-308): the mark and the
                // sentences that stand where the zone used to be blank.
                let mut seat = App::for_test(fixture(false), Theme::new(flavor, profile));
                seat.screen = Screen::Ticket { ticket: ulid_n(1), rail_idx: 0 };
                assert!(
                    render(&seat, 120, 30).iter().any(|l| l.contains(
                        if profile == Profile::Mono {
                            "mesimon"
                        } else {
                            "▀███████████████▀"
                        }
                    )),
                    "the empty seat's mark must be ON SCREEN, or this law does not bite"
                );
                cells(&seat, 120, 30)
            },
            {
                // The quiet zone's other half (T-308): a needs-you session
                // whose whole headline is the agent's own question, in the
                // one register this zone is allowed to reach for.
                let mut asked = App::for_test(fixture(true), Theme::new(flavor, profile));
                asked.screen = Screen::Ticket { ticket: ulid_n(4), rail_idx: 0 };
                assert!(
                    render(&asked, 120, 30).iter().any(|l| l.contains("rm -rf node_modules")),
                    "the question must be ON SCREEN, or this law does not bite"
                );
                cells(&asked, 120, 30)
            },
            {
                install_diff(&mut app);
                assert!(
                    render(&app, 120, 30).iter().any(|l| l.contains("vs a1b2c3d4")),
                    "the branch diff must be ON SCREEN, or this law does not bite"
                );
                cells(&app, 120, 30)
            },
            {
                install_checkout_diff(&mut app);
                assert!(
                    render(&app, 120, 30).iter().any(|l| l.contains("uncommitted")),
                    "the checkout diff must be ON SCREEN, or this law does not bite"
                );
                cells(&app, 120, 30)
            },
            {
                // The release notes: rendered markdown on painted bands, and
                // the one bold allowed there is the tag.
                install_releases(&mut app);
                assert!(
                    render(&app, 120, 30).iter().any(|l| l.contains("this build")),
                    "the notes must be ON SCREEN, or this law does not bite"
                );
                cells(&app, 120, 30)
            },
            {
                // The board's prompt field, on its card. New vocabulary is
                // exactly what these sweeps exist to catch.
                let mut p = App::for_test(fixture(false), Theme::new(flavor, profile));
                p.rich_keys = true;
                p.cursor_col = 1;
                p.mode = Mode::Input {
                    purpose: crate::app::InputPurpose::Prompt {
                        target: crate::app::AskTarget::Ticket(ulid_n(3)),
                        walk: None,
                        queued: false,
                        accept_plan: false,
                        plan: false,
                    },
                    buffer: crate::text::EditBuffer::new(mesimon_core::board::TITLE_MAX_BYTES),
                };
                cells(&p, 120, 30)
            },
            {
                // The search picker (T-349): new vocabulary on a new surface
                // — a prompt, painted highlight runs, and a whole card
                // rendered inside a second frame.
                let sp =
                    searching(App::for_test(fixture_archived(), Theme::new(flavor, profile)), "ac");
                assert!(
                    render(&sp, 120, 30).iter().any(|l| l.contains("SEARCH ∙ 2/7")),
                    "the picker must be ON SCREEN, or this law does not bite"
                );
                cells(&sp, 120, 30)
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
            desc_band,
            note_zone,
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

/// L1: no drawn structure outside the explicit tag/description, dialog
/// perimeter and exact mascot-cell exceptions (the accent bar is painted).
#[test]
fn test_no_drawn_structure() {
    // Every screen swept is kept as its cell grid AND the dialog frames its
    // draw recorded: a box glyph is legal on a frame's perimeter and nowhere
    // else (T-158, the one allowlisted role), and the perimeter is a fact of
    // the frame the draw itself reported — the test transcribes nothing.
    type DrawnFrame =
        (ratatui::buffer::Buffer, Vec<ratatui::layout::Rect>, Option<ratatui::layout::Rect>);
    let swept: std::cell::RefCell<Vec<DrawnFrame>> = std::cell::RefCell::new(Vec::new());
    let sweep = |app: &App| -> Vec<String> {
        let buf = cells(app, 120, 30);
        swept.borrow_mut().push((buf.clone(), app.frames.borrow().clone(), *app.mascot.borrow()));
        lines_of(&buf)
    };
    let path = write_transcript("drawn-law", &reply_record(RICH_REPLY));
    let mut app = app_graphite(fixture(true));
    // Markdown is full of rules and boxes; none of them may reach a cell.
    attach_transcript(&mut app.board, &path);
    app.cursor_col = 1;
    // A card with an unread reply: the heavy done mark, calm.
    seed_spoke(&mut app, ulid_n(5));
    let mut arch = app_graphite(fixture_archived());
    arch.mode = Mode::Archived { idx: 0 };
    let mut picker = app_graphite(fixture(false));
    picker.mode = Mode::Theme { idx: 2 };
    // The cursor on a column header (T-117): its bar is a painted cell, and
    // the automation mark is outside the banned range.
    let mut header = app_graphite(fixture(false));
    header.board.columns[1].settings.on_done = Some("review".into());
    header.cursor_col = 1;
    header.cursor_row = None;
    let mut coldlg = app_graphite(fixture(false));
    coldlg.cursor_col = 1;
    coldlg.cursor_row = None;
    coldlg
        .handle_key(
            ratatui::crossterm::event::KeyCode::Enter,
            ratatui::crossterm::event::KeyModifiers::NONE,
        )
        .unwrap();
    let mut armed = app_graphite(fixture_woke());
    armed.cursor_col = 0;
    armed
        .handle_key(
            ratatui::crossterm::event::KeyCode::Char('z'),
            ratatui::crossterm::event::KeyModifiers::NONE,
        )
        .unwrap();
    let screens: Vec<Vec<String>> = vec![
        {
            let lines = sweep(&app);
            assert!(
                lines.iter().any(|l| l.contains(DONE_UNREAD)),
                "the unread done mark must be ON SCREEN, or this law does not bite"
            );
            lines
        },
        sweep(&arch),
        sweep(&picker),
        {
            let lines = sweep(&header);
            assert!(
                lines.iter().any(|l| l.contains("column settings")),
                "the header cursor must be ON SCREEN, or this law does not bite"
            );
            lines
        },
        {
            let lines = sweep(&coldlg);
            assert!(
                lines.iter().any(|l| l.contains("COLUMN ∙ IN PROGRESS")),
                "the column dialog must be ON SCREEN, or this law does not bite"
            );
            lines
        },
        {
            let lines = sweep(&app_links());
            assert!(
                lines.iter().any(|l| l.contains("LINKS ∙ T-3")),
                "the links dialog must be ON SCREEN, or this law does not bite"
            );
            lines
        },
        {
            let lines = sweep(&armed);
            assert!(
                lines.iter().any(|l| l.contains("snooze 1h")),
                "the armed snooze must be ON SCREEN, or this law does not bite"
            );
            lines
        },
        {
            app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
            let lines = sweep(&app);
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
            app.shell_tail = Some(crate::app::ShellTail::new(
                crate::app::TailKey::Session(uuid_n(32)),
                dirty_tail(),
            ));
            let lines = sweep(&app);
            assert!(
                lines.iter().any(|l| l.contains("in 2.4s")),
                "the whole shell tail must be ON SCREEN, or this law does not bite"
            );
            lines
        },
        {
            // The shin's block glyphs are legal only at its recorded cells.
            // All other preview content still crosses the scrub boundary.
            let mut seat = app_graphite(fixture(false));
            seat.screen = Screen::Ticket { ticket: ulid_n(1), rail_idx: 0 };
            let lines = sweep(&seat);
            assert!(
                lines.iter().any(|l| l.contains("▀███████████████▀")),
                "the empty seat's mark must be ON SCREEN, or this law does not bite"
            );
            lines
        },
        {
            // And the quiet zone drawing an agent's own question, which is
            // text from a hook payload like any other.
            let mut asked = app_graphite(fixture(true));
            asked.screen = Screen::Ticket { ticket: ulid_n(4), rail_idx: 0 };
            let lines = sweep(&asked);
            assert!(
                lines.iter().any(|l| l.contains("rm -rf node_modules")),
                "the question must be ON SCREEN, or this law does not bite"
            );
            lines
        },
        {
            install_diff(&mut app);
            let lines = sweep(&app);
            assert!(
                lines.iter().any(|l| l.contains("vs a1b2c3d4")),
                "the branch diff must be ON SCREEN, or this law does not bite"
            );
            lines
        },
        {
            install_checkout_diff(&mut app);
            let lines = sweep(&app);
            assert!(
                lines.iter().any(|l| l.contains("uncommitted")),
                "the checkout diff must be ON SCREEN, or this law does not bite"
            );
            lines
        },
        {
            install_releases(&mut app);
            let lines = sweep(&app);
            assert!(
                lines.iter().any(|l| l.contains("this build")),
                "the notes must be ON SCREEN, or this law does not bite"
            );
            lines
        },
        {
            // The picker draws TWO frames — the list and the preview beside
            // it — and every box glyph on either must be a recorded
            // perimeter, which is the whole reason it uses `dialog::frame`
            // rather than a rule of its own.
            let sp = searching(app_graphite(fixture_archived()), "ac");
            let lines = sweep(&sp);
            assert!(
                lines.iter().any(|l| l.contains("SEARCH ∙ 2/7")),
                "the picker must be ON SCREEN, or this law does not bite"
            );
            assert_eq!(sp.frames.borrow().len(), 2, "both panes are recorded frames");
            lines
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
            sweep(&t)
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
            sweep(&t)
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
                purpose: crate::app::InputPurpose::Prompt {
                    target: crate::app::AskTarget::Ticket(ulid_n(3)),
                    walk: None,
                    queued: false,
                    accept_plan: false,
                    plan: false,
                },
                buffer: buf,
            };
            let lines = sweep(&p);
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
                crate::app::EditorPurpose::Compose {
                    workspace: None,
                    tags: Vec::new(),
                    plan: false,
                },
                "Ship it",
                "",
            );
            ed.body.paste(&dirty_tail().join("\n"));
            e.mode = Mode::Editor(ed);
            let lines = sweep(&e);
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
            lines.into_iter().chain(sweep(&e)).collect()
        },
        {
            // The page's two markdown surfaces, which since T-344 are never
            // on screen together: the band's description block while the
            // zone reads a session, and a note in the zone, which is where
            // the band's rows went.
            let mut n = app_noted();
            if let Some(t) = n.board.tickets.iter_mut().find(|t| t.id == ulid_n(3)) {
                t.notes.push(note_meta(92, "tree", "local"));
            }
            n.remember_note(ulid_n(92), 1, Some(crate::peek::sanitize(&dirty_tail().join("\n"))));
            n.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
            let lines = sweep(&n);
            assert!(lines.iter().any(|l| l.contains("What changed")), "description on screen");
            n.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 4 };
            let note = sweep(&n);
            assert!(note.iter().any(|l| l.contains("in 2.4s")), "note on screen");
            lines.into_iter().chain(note).collect()
        },
    ];
    // Two entries sweep a screen of their own besides the one they return:
    // the note editor over the board, and the ticket page's second markdown
    // surface.
    assert_eq!(swept.borrow().len(), screens.len() + 2, "one grid per swept screen");
    let on_perimeter = |frames: &[ratatui::layout::Rect], x: u16, y: u16| {
        frames.iter().any(|r| {
            let inside = x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height;
            let edge = x == r.x || x == r.x + r.width - 1 || y == r.y || y == r.y + r.height - 1;
            inside && edge
        })
    };
    let mut framed = 0usize;
    for (buf, frames, mascot) in swept.borrow().iter() {
        framed += frames.len();
        let area = buf.area();
        for y in 0..area.height {
            for x in 0..area.width {
                for ch in buf[(x, y)].symbol().chars() {
                    let cp = ch as u32;
                    if !(0x2500..=0x259F).contains(&cp) {
                        continue;
                    }
                    // The tag bar admits `▉` for its narrow gaps. `▎` belongs
                    // to description bars; frames and the mascot keep their
                    // own position-scoped exceptions.
                    assert!(
                        ch == '▉'
                            || ch == '▎'
                            || on_perimeter(frames, x, y)
                            || mascot.is_some_and(|r| {
                                x >= r.x
                                    && y >= r.y
                                    && x < r.right()
                                    && y < r.bottom()
                                    && crate::mascot::COMPACT
                                        .lines()
                                        .nth((y - r.y) as usize)
                                        .and_then(|row| row.chars().nth((x - r.x) as usize))
                                        == Some(ch)
                            }),
                        "drawn-structure codepoint {ch:?} at {x},{y} off any dialog frame"
                    );
                }
            }
        }
    }
    assert!(framed >= 5, "the sweep must cover framed dialogs, or this law does not bite");
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

/// The done mark as a card row spells it, read and unread.
const DONE: &str = "✓";
const DONE_UNREAD: &str = "✔";

/// Mark `ticket` as having spoken since the cursor was on it — the entry the
/// scan would hold after a reply landed on a card the cursor was not on.
fn seed_spoke(app: &mut App, ticket: ulid::Ulid) {
    app.spoke.insert(
        ticket,
        crate::app::Spoke { session: uuid_n(0), path: String::new(), key: 1, seen: 0 },
    );
}

/// The done mark on T-5's row (the finished agent in `review`): its glyph
/// and its colour.
fn done_mark(app: &App) -> (String, ratatui::style::Color) {
    let buf = cells(app, 120, 30);
    let lines = render(app, 120, 30);
    let y = lines.iter().position(|l| l.contains("Grapheme")).expect("T-5") as u16;
    let x0 = lines[y as usize].find("Grapheme").expect("title") as u16;
    // The glyph is two cells left of the title, in T-5's own column.
    let x = (0..x0)
        .rev()
        .find(|&x| matches!(buf[(x, y)].symbol(), DONE | DONE_UNREAD))
        .expect("T-5 wears a done mark");
    (buf[(x, y)].symbol().to_string(), buf[(x, y)].fg)
}

/// T-173: the done mark decays once seen. While the reply it stands for is
/// one the cursor has not been on the card for it is the heavy `✔` in the
/// calm register; once it has, the thin `✓` on the grey ramp — shape and
/// loudness both step, no cell spent. The cursor card is always seen.
#[test]
fn test_done_mark_decays_once_seen() {
    let mut app = app_graphite(fixture(false));
    app.cursor_col = 1;
    let calm = app.theme.calm;
    let grey = app.theme.rest.dim2;
    assert_ne!(calm, grey, "or the decay is no step at all");
    assert_eq!(done_mark(&app), (DONE.into(), grey), "nothing unread: thin, on the grey ramp");
    seed_spoke(&mut app, ulid_n(5));
    assert_eq!(done_mark(&app), (DONE_UNREAD.into(), calm), "unread: heavy, calm");
    app.cursor_col = 2;
    app.cursor_row = Some(0);
    assert_eq!(done_mark(&app).0, DONE, "the cursor card is seen as it is drawn");
    // Every flavor keeps the colour step: the two are different tokens by law.
    for flavor in Flavor::ALL {
        let mut app = App::for_test(fixture(false), Theme::new(flavor, Profile::TrueColor));
        app.cursor_col = 1;
        let before = done_mark(&app).1;
        seed_spoke(&mut app, ulid_n(5));
        assert_ne!(done_mark(&app).1, before, "{flavor:?}: unread must look different");
    }
    // Mono has no colour and no heavier `+`: the mark reads `+` either way.
    let mut mono = App::for_test(fixture(false), Theme::new(Flavor::Graphite, Profile::Mono));
    mono.cursor_col = 1;
    seed_spoke(&mut mono, ulid_n(5));
    let row = render(&mono, 120, 30).into_iter().find(|l| l.contains("Grapheme")).expect("T-5");
    assert!(row.contains("+ Grapheme"), "{row:?}");
}

/// A sampled checkout for the header (T-124), tracking `origin/main`.
fn git_state(
    branch: &str,
    ahead: u32,
    behind: u32,
    changed: u32,
) -> mesimon_core::command::RepoGit {
    mesimon_core::command::RepoGit {
        sampled: true,
        branch: branch.into(),
        upstream: Some("origin/main".into()),
        ahead,
        behind,
        changed,
        ..Default::default()
    }
}

/// The board's own checkout on the header (T-124): the branch, the arrows and
/// the change count after the breadcrumb, with the offer still at the right
/// edge. Fetch is no longer a menu row.
#[test]
fn golden_git_120() {
    let mut app = app_graphite(fixture(false));
    app.git = git_state("main", 2, 1, 3);
    app.force_update_ready();
    golden("board_git_120x30", &render(&app, 120, 30));
    app.mode = Mode::Menu { idx: 0 };
    let menu = render(&app, 120, 30);
    for removed in ["Fetch origin", "All keys on this screen", "Add a column"] {
        assert!(!menu.iter().any(|l| l.contains(removed)), "{removed}: {menu:?}");
    }
}

/// Nothing until a sample lands; then only what is out of sync is said —
/// a clean branch in sync is the branch name and nothing more.
#[test]
fn test_git_clause_is_silent_until_sampled_and_quiet_in_sync() {
    let app = app_graphite(fixture(false));
    let head = &render(&app, 120, 30)[0];
    assert!(!head.contains('⎇'), "unsampled draws nothing: {head:?}");
    let mut app = app_graphite(fixture(false));
    app.git = git_state("main", 0, 0, 0);
    let head = &render(&app, 120, 30)[0];
    // The clause is facts and nothing else: no key rides it since T-305, and
    // the terminal's `!` never did — it was there for a day (T-273) and `?`
    // is its one home (T-277).
    assert!(head.contains("kanban-tui ⎇ main   7 tickets"), "{head:?}");
    assert!(!head.contains("terminal"), "{head:?}");
    app.git = git_state("main", 0, 3, 0);
    let head = &render(&app, 120, 30)[0];
    assert!(head.contains("⎇ main ↓3   7 tickets"), "{head:?}");
    app.git = git_state("main", 1, 0, 2);
    let head = &render(&app, 120, 30)[0];
    assert!(head.contains("⎇ main ↑1 ∙ 2 changed   7 tickets"), "{head:?}");
    // Detached: the short oid stands in for the name, no arrows without an upstream.
    app.git = mesimon_core::command::RepoGit {
        detached: true,
        upstream: None,
        ..git_state("a1b2c3d", 0, 0, 1)
    };
    let head = &render(&app, 120, 30)[0];
    assert!(head.contains("⎇ a1b2c3d ∙ 1 changed   7 tickets"), "{head:?}");
}

/// The offer has first claim on the row. The clause gives its parts up in
/// order — the name down to its floor, then the count — and the arrows are
/// never cut; when even the floor will not fit it stands aside whole.
#[test]
fn test_git_clause_gives_way_to_the_offer() {
    let long = "msmn/T-124-git-status-pull-push-indication";
    assert_eq!(long.width(), 42);
    let mut app = app_graphite(fixture(false));
    app.git = git_state(long, 1, 0, 3);
    app.force_release_available("v0.1.0-alpha.5");
    let head = &render(&app, 160, 30)[0];
    assert!(head.contains(&format!("⎇ {long} ↑1 ∙ 3 changed   7 tickets")), "{head:?}");
    assert!(head.ends_with("◦ v0.1.0-alpha.5 available (esc)"), "{head:?}");
    // The name is what gives first, and the count rides its truncation down.
    let head = &render(&app, 110, 30)[0];
    assert!(head.contains("~ ↑1 ∙ 3 changed   7 tickets"), "the count outlives it: {head:?}");
    let head = &render(&app, 100, 30)[0];
    assert!(head.ends_with("◦ v0.1.0-alpha.5 available (esc)"), "the offer stays: {head:?}");
    assert!(head.contains("⎇ msmn/T-124"), "the name is kept to its floor: {head:?}");
    assert!(head.contains("~ ↑1   7 tickets"), "the arrow rides the cut name: {head:?}");
    assert!(!head.contains("changed"), "the count goes next: {head:?}");
    let head = &render(&app, 90, 30)[0];
    assert!(head.ends_with("◦ v0.1.0-alpha.5 available (esc)"), "the offer stays: {head:?}");
    assert!(!head.contains('⎇'), "below the floor the clause stands aside whole: {head:?}");
    // With no offer the clause has the row: the name gives a little and the
    // count stays, because the name is still above its floor.
    let mut app = app_graphite(fixture(false));
    app.git = git_state(long, 1, 0, 3);
    let head = &render(&app, 100, 30)[0];
    assert!(
        head.contains("⎇ msmn/T-124-git-status-pull-push-indicat~ ↑1 ∙ 3 changed   7 tickets"),
        "{head:?}"
    );
}

/// The ASCII tier spells the clause with the card's own fallbacks.
#[test]
fn test_git_clause_has_an_ascii_spelling() {
    let mut app = App::for_test(fixture(false), Theme::new(Flavor::Graphite, Profile::Mono));
    app.git = git_state("main", 2, 1, 3);
    let head = &render(&app, 120, 30)[0];
    assert!(head.contains("& main ^2 v1 ∙ 3 changed"), "{head:?}");
}

#[test]
fn golden_awake_120() {
    let mut app = app_graphite(fixture(false));
    app.git = git_state("main", 2, 1, 3);
    app.seed_pref(|p| p.keep_awake = true);
    app.caffeinated = true;
    golden("board_awake_120x30", &render(&app, 120, 30));
    app.caffeinated = false;
    golden("board_awake_idle_120x30", &render(&app, 120, 30));
    app.cursor_row = None;
    app.header_focus = true;
    app.header_awake = true;
    golden("board_awake_focused_120x30", &render(&app, 120, 30));
}

#[test]
fn wake_indicator_visibility_follows_preference_on_every_screen() {
    let mut app = app_graphite(fixture(false));
    app.git = git_state("main", 2, 1, 3);
    for screen in [Screen::Board, Screen::Ticket { ticket: ulid_n(5), rail_idx: 0 }] {
        app.screen = screen;
        for held in [true, false] {
            app.caffeinated = held;
            app.seed_pref(|p| p.keep_awake = false);
            let off = render(&app, 120, 30)[0].clone();
            assert!(!off.contains('☕') && !off.contains('☾'), "disabled: {off}");
            app.seed_pref(|p| p.keep_awake = true);
            let mark = if held { "☕️" } else { "☾" };
            let on = render(&app, 120, 30)[0].clone();
            let label =
                if matches!(app.screen, Screen::Board) { "7 tickets" } else { "kanban-tui" };
            assert!(on.contains(&format!("{label} {mark}")), "enabled: {on}");
        }
    }
}

#[test]
fn wake_activity_and_focus_never_move_the_header() {
    for profile in [Profile::TrueColor, Profile::Mono] {
        let mut app = App::for_test(fixture(false), Theme::new(Flavor::Graphite, profile));
        app.seed_pref(|p| p.keep_awake = true);
        app.git = git_state("main", 2, 1, 3);
        app.force_update_ready();
        app.resources.rss_measured = 1;
        app.resources.rss_bytes = 400 * 1024 * 1024;
        for w in [60, 80, 90, 100, 110, 120, 160] {
            app.caffeinated = false;
            app.header_focus = false;
            let idle = render(&app, w, 30)[0].clone();
            let idle_label = crate::glyphs::awake_label(app.theme.glyph_tier(), false);
            let active_label = crate::glyphs::awake_label(app.theme.glyph_tier(), true);
            let needle = format!("7 tickets {idle_label}");
            assert!(idle.contains(&needle), "missing idle glyph at {w}: {idle}");
            // A wide emoji contributes a blank continuation cell to the buffer dump.
            let continuation = if profile == Profile::Mono { "" } else { " " };
            let expected = idle
                .replacen(&needle, &format!("7 tickets {active_label}{continuation}"), 1)
                .trim_end()
                .to_string();
            app.caffeinated = true;
            assert_eq!(render(&app, w, 30)[0], expected, "activity shifted header at {w}");
            app.header_focus = true;
            app.header_awake = true;
            assert_eq!(render(&app, w, 30)[0], expected, "focus shifted header at {w}");
        }
    }
}

#[test]
fn wake_indicator_has_ascii_states() {
    let mut app = App::for_test(fixture(false), Theme::new(Flavor::Graphite, Profile::Mono));
    app.seed_pref(|p| p.keep_awake = true);
    for (held, label) in [(true, "@"), (false, "z")] {
        app.caffeinated = held;
        let head = render(&app, 120, 30)[0].clone();
        assert!(head.contains(&format!("7 tickets {label}")), "{head:?}");
    }
}

#[test]
fn wake_indicator_is_quiet_and_focus_is_visible() {
    for flavor in Flavor::ALL {
        for held in [false, true] {
            let theme = Theme::new(flavor, Profile::TrueColor);
            let mut app = App::for_test(fixture(false), theme);
            app.seed_pref(|p| p.keep_awake = true);
            app.caffeinated = held;
            app.git = git_state("main", 2, 1, 3);
            let plain = cells(&app, 120, 30);
            let mark = if held { "☕️" } else { "☾" };
            let x = (0..120u16).find(|x| plain[(*x, 0)].symbol() == mark).unwrap();
            assert_eq!(
                plain[(x, 0)].fg,
                if held { app.theme.rest.base } else { app.theme.rest.dim3 }
            );
            app.header_focus = true;
            app.header_awake = true;
            let focused = cells(&app, 120, 30);
            assert_ne!(
                plain[(x, 0)].style(),
                focused[(x, 0)].style(),
                "{flavor:?} cursor is invisible"
            );
            let selected_ink =
                if app.theme.selected_bg.is_some() { &app.theme.sel } else { &app.theme.rest };
            let expected = if held { selected_ink.base } else { selected_ink.dim3 };
            assert_eq!(focused[(x, 0)].fg, expected, "{flavor:?}, held={held}");
            if !held {
                assert_eq!(plain[(x + 1, 0)].symbol(), " ");
                assert_eq!(plain[(x + 1, 0)].fg, app.theme.rest.dim3);
                assert_eq!(focused[(x + 1, 0)].fg, expected);
            }
            for buf in [&plain, &focused] {
                assert_ne!(buf[(x, 0)].fg, app.theme.attn);
                assert_ne!(buf[(x, 0)].bg, app.theme.attn);
            }
            assert!(render(&app, 120, 30).last().unwrap().contains("enter keep awake settings"));
        }
    }
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

/// The first `d` arms a delete, and the card says so until the second `d`
/// or the cancel (author 2026-09-03): it flashes as a deletion — the diff's
/// del tint under an `err` title — on the MOVE ghost's cadence, and a cancel
/// puts the ordinary cursor surface back. Swept over every flavor, because
/// the first amber had no red to flash ("red flash before delete not
/// visible there"): the del tint must be a red the flavor's `err` is not.
#[test]
fn test_delete_armed_flashes_the_card() {
    for flavor in Flavor::ALL {
        delete_flash_holds(flavor);
    }
}

fn delete_flash_holds(flavor: Flavor) {
    let theme = Theme::new(flavor, Profile::TrueColor);
    let cell_at = |buf: &ratatui::buffer::Buffer, needle: &str| {
        for y in 0..30u16 {
            let row: String = (0..120u16).map(|x| buf[(x, y)].symbol()).collect::<String>();
            if let Some(ix) = row.find(needle) {
                let x = row[..ix].chars().count() as u16;
                let c = &buf[(x, y)];
                return Some((c.fg, c.bg));
            }
        }
        None
    };
    let del_bg = theme.diff_del_bg().expect("every flavor tints");
    let sel_bg = theme.selected_bg.expect("truecolor paints selected");
    assert_ne!(del_bg, sel_bg, "{flavor:?}: the lit ground is not the cursor surface");
    assert_ne!(theme.err, theme.sel.base, "{flavor:?}: the lit title is not the cursor title");
    let mut app = App::for_test(fixture(false), Theme::new(flavor, Profile::TrueColor));
    app.cursor_col = 0;
    let (_, resting_bg) = cell_at(&cells(&app, 120, 30), "Decay").expect("cursor title");
    assert_eq!(resting_bg, sel_bg, "{flavor:?}: the cursor card rests on the cursor surface");
    press(&mut app, 'd');
    app.spin_epoch.set(Some(std::time::Instant::now()));
    let (fg, bg) = cell_at(&cells(&app, 120, 30), "Decay").expect("armed title, frame 0");
    assert_eq!((fg, bg), (theme.err, del_bg), "{flavor:?}: lit phase draws the card as a deletion");
    let bystander0 = cell_at(&cells(&app, 120, 30), "Grapheme").expect("bystander title");
    // 410 ms back → frame 4 (or 5 under scheduler slop) — both the dark phase.
    app.spin_epoch.set(Some(std::time::Instant::now() - std::time::Duration::from_millis(410)));
    let (fg, bg) = cell_at(&cells(&app, 120, 30), "Decay").expect("armed title, frame 4");
    assert_eq!((fg, bg), (theme.sel.base, sel_bg), "dark phase is the cursor surface");
    let bystander4 = cell_at(&cells(&app, 120, 30), "Grapheme").expect("bystander title");
    assert_eq!(bystander0, bystander4, "bystander cards hold still");
    // A stray key cancels, and the flash goes with the arming.
    app.spin_epoch.set(Some(std::time::Instant::now()));
    press(&mut app, 'x');
    assert_eq!(app.status, "delete cancelled");
    let (fg, bg) = cell_at(&cells(&app, 120, 30), "Decay").expect("title after cancel");
    assert_eq!((fg, bg), (theme.sel.base, sel_bg), "cancel puts the cursor surface back");
}

/// The armed snooze blinks the card (user 2026-09-04: "flash while in snooze
/// not confirmed yet") — the move ghost's blink, not the delete's red: from
/// the `z` to the Enter or the cancel the title square-waves between the
/// cursor title and dim3, bystanders hold still, and the cancel puts the
/// cursor title back.
#[test]
fn test_snooze_armed_blinks_the_card() {
    for flavor in Flavor::ALL {
        let theme = Theme::new(flavor, Profile::TrueColor);
        let title_fg = |buf: &ratatui::buffer::Buffer, needle: &str| {
            for y in 0..30u16 {
                let row: String = (0..120u16).map(|x| buf[(x, y)].symbol()).collect::<String>();
                if let Some(ix) = row.find(needle) {
                    return Some(buf[(row[..ix].chars().count() as u16, y)].fg);
                }
            }
            None
        };
        let mut app = App::for_test(fixture(false), Theme::new(flavor, Profile::TrueColor));
        app.cursor_col = 0;
        press(&mut app, 'z');
        app.spin_epoch.set(Some(std::time::Instant::now()));
        let lit = cells(&app, 120, 30);
        assert_eq!(title_fg(&lit, "Decay"), Some(theme.sel.base), "{flavor:?}: bright phase");
        let bystander0 = title_fg(&lit, "Grapheme");
        app.spin_epoch.set(Some(std::time::Instant::now() - std::time::Duration::from_millis(410)));
        let dark = cells(&app, 120, 30);
        assert_eq!(title_fg(&dark, "Decay"), Some(theme.sel.dim3), "{flavor:?}: dark phase");
        assert_eq!(title_fg(&dark, "Grapheme"), bystander0, "bystander cards hold still");
        press(&mut app, 'x');
        assert_eq!(app.status, "snooze cancelled");
        let after = cells(&app, 120, 30);
        assert_eq!(title_fg(&after, "Decay"), Some(theme.sel.base), "cancel puts the title back");
    }
}

/// And the ticket page's title row, that page's card, blinks the same way.
#[test]
fn test_snooze_armed_blinks_the_ticket_title() {
    let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
    let mut app = app_graphite(fixture(false));
    app.cursor_col = 0;
    app.screen = Screen::Ticket { ticket: ulid_n(1), rail_idx: 0 };
    press(&mut app, 'z');
    let fg_at = |app: &App| {
        let buf = cells(app, 120, 30);
        let row: String = (0..120u16).map(|x| buf[(x, 2u16)].symbol()).collect();
        let x = row.find("Decay").expect("title row") as u16;
        buf[(x, 2u16)].fg
    };
    app.spin_epoch.set(Some(std::time::Instant::now()));
    assert_eq!(fg_at(&app), theme.sel.base);
    app.spin_epoch.set(Some(std::time::Instant::now() - std::time::Duration::from_millis(410)));
    assert_eq!(fg_at(&app), theme.sel.dim3);
}

/// A refused archive shakes the card (T-423, user: "show indication when
/// archive impossible on the ticket itself"): `a` on a ticket whose sessions
/// are awake is refused in the status line as before, and the card itself
/// says no — drawn one cell right, then left, three times, bar and all, on
/// the `SHAKE_STEP` clock, and back in its place after the last step.
/// Bystanders hold still. Colourless, so it is neither the delete's red nor
/// the move blink, and the golden pins the first step.
#[test]
fn test_archive_refused_shakes_the_card() {
    let mut app = app_graphite(fixture(false));
    app.cursor_col = 1; // in progress: T-3 holds a running claude and a shell
    app.cursor_row = Some(0);
    let at = |app: &App, needle: &str| -> (u16, u16) {
        let buf = cells(app, 120, 30);
        for y in 0..30u16 {
            let row: String = (0..120u16).map(|x| buf[(x, y)].symbol()).collect();
            if let Some(ix) = row.find(needle) {
                return (row[..ix].chars().count() as u16, y);
            }
        }
        panic!("{needle} not on screen");
    };
    let (rest, y) = at(&app, "Fix OSC-11");
    let bystander = at(&app, "Adopt drawer");
    // bar(2) + pad + glyph + pad precede the title of a card with sessions.
    // The bar is a painted cell: its ground is what moves.
    let bar_x = rest - 5;
    let bar = cells(&app, 120, 30)[(bar_x, y)].bg;
    assert_ne!(bar, ratatui::style::Color::Reset, "the cursor card wears its bar");
    press(&mut app, 'a');
    assert_eq!(app.status, "its sessions are awake — sleep them first (x)");
    let id = ulid_n(3);
    let seed = |app: &mut App, ms: u64| {
        app.refused = Some((id, std::time::Instant::now() - std::time::Duration::from_millis(ms)));
    };
    seed(&mut app, 30); // step 0: one cell right
    assert_eq!(at(&app, "Fix OSC-11"), (rest + 1, y), "first step is one cell right");
    let buf = cells(&app, 120, 30);
    assert_ne!(buf[(bar_x, y)].bg, bar, "the bar moved with the card");
    assert_eq!(buf[(bar_x + 1, y)].bg, bar, "and stands one cell right");
    assert_eq!(at(&app, "Adopt drawer"), bystander, "bystanders hold still");
    golden("board_archive_refused_120x30", &render(&app, 120, 30));
    seed(&mut app, 90); // step 1: one cell left, into the gutter
    assert_eq!(at(&app, "Fix OSC-11"), (rest - 1, y), "second step is one cell left");
    assert_eq!(at(&app, "Adopt drawer"), bystander, "bystanders hold still");
    seed(&mut app, 400); // past the last step: settled
    assert_eq!(at(&app, "Fix OSC-11"), (rest, y), "settled back in place");
    assert!(!app.animating(), "a settled shake no longer asks for fast frames");
}

/// And the ticket page's title row, that page's card, shakes within its band.
#[test]
fn test_archive_refused_shakes_the_ticket_title() {
    let mut app = app_graphite(fixture(false));
    app.cursor_col = 1;
    app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    let x_at = |app: &App| -> u16 {
        let buf = cells(app, 120, 30);
        let row: String = (0..120u16).map(|x| buf[(x, 2u16)].symbol()).collect();
        row.find("Fix OSC-11").expect("title row") as u16
    };
    let rest = x_at(&app);
    press(&mut app, 'a');
    assert_eq!(app.status, "its sessions are awake — sleep them first (x)");
    let id = ulid_n(3);
    let seed = |app: &mut App, ms: u64| {
        app.refused = Some((id, std::time::Instant::now() - std::time::Duration::from_millis(ms)));
    };
    seed(&mut app, 30);
    assert_eq!(x_at(&app), rest + 1);
    seed(&mut app, 90);
    assert_eq!(x_at(&app), rest - 1);
    seed(&mut app, 400);
    assert_eq!(x_at(&app), rest);
}

/// The ticket page's title row is that page's card: `d` there flashes it the
/// same way.
#[test]
fn test_delete_armed_flashes_the_ticket_title() {
    let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
    let del_bg = theme.diff_del_bg().expect("graphite tints");
    let mut app = app_graphite(fixture(false));
    app.cursor_col = 0;
    app.screen = Screen::Ticket { ticket: ulid_n(1), rail_idx: 0 };
    press(&mut app, 'd');
    app.spin_epoch.set(Some(std::time::Instant::now()));
    let buf = cells(&app, 120, 30);
    let row: String = (0..120u16).map(|x| buf[(x, 2u16)].symbol()).collect();
    let x = row.find("Decay").expect("title row") as u16;
    assert_eq!((buf[(x, 2u16)].fg, buf[(x, 2u16)].bg), (theme.err, del_bg));
    assert_eq!(buf[(118u16, 2u16)].bg, del_bg, "the tint spans the row");
    assert_ne!(buf[(118u16, 3u16)].bg, del_bg, "the state row keeps the band");
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

fn fixture_overflow() -> Board {
    let mut b = Board::default();
    b.columns.push(Column::new("todo", "0"));
    b.columns.push(Column::new("done", "1"));
    b.register_tag(1, "BUG").unwrap();
    b.register_tag(2, "UI").unwrap();
    for i in 0..12u128 {
        let mut t =
            ticket(i + 1, &format!("T-{i}"), &format!("Load {i:02}"), "todo", &format!("{i:02}"));
        t.set_tag(1, Some("BUG".into()));
        if i % 2 == 1 {
            t.set_tag(2, Some("UI".into()));
        }
        b.tickets.push(t);
    }
    b.tickets.push(ticket(99, "T-99", "Elsewhere", "done", "00"));
    b
}

/// Overflow is a counted cue, never a restyled copy of a card. All visible
/// cards retain their full-width tag bars, and counts include the boundary
/// cards displaced by a cue. Walk both directions to exercise scroll state.
#[test]
fn test_clipped_columns_keep_whole_cards_and_count_hidden_tickets() {
    for flavor in Flavor::ALL {
        let mut app = app_graphite(fixture_overflow());
        app.theme = Theme::new(flavor, Profile::TrueColor);
        app.peek = false;
        app.cursor_col = 0;
        for expanded in [false, true] {
            app.peek_all = expanded;
            app.scroll_row.set(0);
            for cursor in (0..12).chain((0..12).rev()) {
                app.cursor_row = Some(cursor);
                let buf = cells(&app, 120, 20);
                let lines = lines_of(&buf);
                let mut visible = Vec::new();
                for y in 0..20u16 {
                    let row: String = (0..40).map(|x| buf[(x, y)].symbol()).collect();
                    if let Some(byte_x) = row.find("Load ") {
                        let x = row[..byte_x].chars().count() as u16;
                        let index: usize = row[byte_x + 5..byte_x + 7].parse().unwrap();
                        visible.push(index);
                        if expanded {
                            assert!(
                                lines[y as usize + 1].contains("BUG"),
                                "the edge must keep the whole card"
                            );
                        }
                        assert_eq!(x, 4, "title inset at the edge");
                        assert_eq!(
                            buf[(x, y)].fg,
                            if index == cursor { app.theme.sel.base } else { app.theme.rest.base }
                        );
                        let tint = |group, name| {
                            app.theme.pip(app.board.tag_def(group, name).unwrap().tint() as usize)
                        };
                        if index % 2 == 1 {
                            for (dx, color) in [(0, tint(1, "BUG")), (1, tint(2, "UI"))] {
                                assert_eq!(buf[(1 + dx, y)].symbol(), "▉");
                                assert_eq!(buf[(1 + dx, y)].fg, color);
                            }
                        } else {
                            for x in [1, 2] {
                                assert_eq!(buf[(x, y)].symbol(), " ");
                                assert_eq!(buf[(x, y)].bg, tint(1, "BUG"));
                            }
                        }
                    }
                }
                assert!(visible.contains(&cursor), "cursor {cursor} lost: {visible:?}");
                assert!(visible.windows(2).all(|w| w[1] == w[0] + 1));
                let above = visible[0];
                let below = 11 - visible.last().unwrap();
                if above > 0 {
                    assert!(lines.iter().any(|l| l.contains(&format!("↑ {above} above"))));
                } else {
                    assert!(!lines.iter().any(|l| l.contains("above")));
                }
                if below > 0 {
                    assert!(lines.iter().any(|l| l.contains(&format!("↓ {below} below"))));
                } else {
                    assert!(!lines.iter().any(|l| l.contains("below")));
                }
                for line in &lines[16..19] {
                    assert!(line.trim().is_empty(), "footer clearance: {line:?}");
                }
                let card = app.cursor_card.get().expect("whole cursor card");
                assert!(card.bottom() <= 16);
            }
        }
        app.peek_all = false;
        app.cursor_col = 1;
        let lines = render(&app, 120, 40);
        assert_eq!(lines.iter().filter(|l| l.contains("Load ")).count(), 12);
        assert!(!lines.iter().any(|l| l.contains("above") || l.contains("below")));
    }
}

#[test]
fn golden_column_overflow_cues() {
    let mut app = app_graphite(fixture_overflow());
    app.peek = false;
    app.cursor_col = 1;
    golden("board_overflow_below_120x20", &render(&app, 120, 20));
    app.cursor_col = 0;
    app.cursor_row = Some(6);
    golden("board_overflow_both_120x20", &render(&app, 120, 20));
    app.cursor_row = Some(11);
    golden("board_overflow_above_120x20", &render(&app, 120, 20));
    app.theme = Theme::new(Flavor::Graphite, Profile::Mono);
    let lines = render(&app, 120, 20);
    assert!(lines.iter().any(|l| l.contains("^ ") && l.contains("above")));
    assert!(!lines.iter().any(|l| l.contains('↑') || l.contains('↓')));
}

#[test]
fn test_overflow_keeps_hidden_attention_in_the_header() {
    let mut app = app_graphite(fixture_overflow());
    app.peek = false;
    app.cursor_col = 1;
    for id in [8, 12] {
        app.board.sessions.push(session(
            id,
            ulid_n(id),
            SessionKind::Claude,
            SessionState::RequiresAction { reason: Reason::Permission },
        ));
    }
    let lines = render(&app, 120, 20);
    assert!(lines[2].contains("!2"), "hidden attention: {:?}", lines[2]);
    app.cursor_col = 0;
    app.cursor_row = Some(11);
    let lines = render(&app, 120, 20);
    assert!(!lines[2].contains("!2"), "visible attention must not be counted twice");
}

#[test]
fn test_overflow_keeps_a_tall_cards_prompt_visible() {
    let path = write_transcript(
        "overflow-tall",
        &reply_record(&"Detailed context about the task and its next steps. ".repeat(12)),
    );
    let mut board = fixture_overflow();
    for id in [31, 32, 33] {
        let mut s = session(id, ulid_n(7), SessionKind::Claude, SessionState::Running);
        s.transcript_path = Some(path.to_string_lossy().into_owned());
        board.sessions.push(s);
    }
    let mut app = app_graphite(board);
    app.cursor_col = 0;
    app.cursor_row = Some(6);
    app.peek = true;
    app.mode = Mode::Input {
        purpose: InputPurpose::Prompt {
            target: crate::app::AskTarget::Ticket(ulid_n(7)),
            walk: None,
            queued: false,
            accept_plan: false,
            plan: false,
        },
        buffer: crate::text::EditBuffer::from_text("continue here".into(), 100),
    };
    for width in [60, 120] {
        let mut terminal = Terminal::new(TestBackend::new(width, 20)).unwrap();
        terminal.draw(|f| super::draw(f, &app)).unwrap();
        let lines = lines_of(terminal.backend().buffer());
        assert!(lines.iter().any(|l| l.contains("Load 06")), "title: {lines:?}");
        let prompt =
            lines.iter().position(|l| l.contains("continue here")).expect("visible prompt");
        assert!(prompt < 16, "field leaves room below: {lines:?}");
        assert_eq!(terminal.get_cursor_position().unwrap().y as usize, prompt);
        assert!(app.cursor_card.get().is_some(), "the whole group fits before footer clearance");
        assert!(lines.iter().any(|l| l.contains('↑')), "above cue survives a tall card");
        assert!(lines.iter().any(|l| l.contains('↓')), "below cue survives a tall card");
    }
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// A workspace (T-225): the root's own branch and arrows lead, the count of
/// repos nested under it is a clause, and the change count is summed across
/// every repo. A folder of several has no branch and is named by the count.
#[test]
fn test_git_clause_names_a_workspace_by_its_count() {
    let mut app = app_graphite(fixture(false));
    app.git = mesimon_core::command::RepoGit {
        repos: vec!["api".into(), "infra".into(), "web".into()],
        ..git_state("master", 2, 1, 7)
    };
    let head = &render(&app, 120, 30)[0];
    assert!(head.contains("⎇ master ↑2 ↓1 ∙ 3 repos ∙ 7 changed"), "{head:?}");
    // A folder of repos has no branch of its own and still speaks.
    app.git.branch.clear();
    app.git.upstream = None;
    app.git.ahead = 0;
    app.git.behind = 0;
    let head = &render(&app, 120, 30)[0];
    assert!(head.contains("⎇ 3 repos ∙ 7 changed"), "{head:?}");
    // Clean: the count goes quiet, the workspace stays named.
    app.git.changed = 0;
    let head = &render(&app, 120, 30)[0];
    assert!(head.contains("⎇ 3 repos   "), "{head:?}");
    golden("board_workspace_120x30", &render(&app, 120, 30));
}

/// One nested repo never reads `1 repo`: a root that is a repository keeps
/// its own branch (the mesimon checkout's `mt/` scratch repo on `orphan`
/// must not become the board's branch), and a folder of one carries that
/// one's branch in the sample already (author 2026-09-05).
#[test]
fn test_git_clause_never_says_one_repo() {
    let mut app = app_graphite(fixture(false));
    app.git =
        mesimon_core::command::RepoGit { repos: vec!["mt".into()], ..git_state("main", 2, 0, 3) };
    let head = &render(&app, 120, 30)[0];
    assert!(head.contains("⎇ main ↑2 ∙ 3 changed"), "{head:?}");
    assert!(!head.contains("repo"), "{head:?}");
}

// ---- the search picker (T-349) ------------------------------------------

use ratatui::crossterm::event as key;

/// Open the picker and type `q` into it.
fn searching(mut app: App, query: &str) -> App {
    app.handle_key(key::KeyCode::Char('/'), key::KeyModifiers::NONE).expect("open");
    for c in query.chars() {
        app.handle_key(key::KeyCode::Char(c), key::KeyModifiers::NONE).expect("type");
    }
    app
}

/// The picker over a board: the query, the ranked rows with the matched
/// characters lifted onto the value ramp, and the cursor row's own card in
/// the frame beside it. T-7 is archived, so the list also shows the tier —
/// a live card first, the archived one under it with the word that says so.
#[test]
fn golden_search_120() {
    let app = searching(app_graphite(fixture_archived()), "ac");
    golden("search_120x30", &render(&app, 120, 30));
}

/// The same picker with nothing typed: the whole board, live cards in board
/// order and the archived one last, which is what `/` opens on.
#[test]
fn golden_search_open_120() {
    let app = searching(app_graphite(fixture_archived()), "");
    golden("search_open_120x30", &render(&app, 120, 30));
}

/// Narrow: the preview goes rather than being squeezed, and the frame's
/// bottom edge drops keys from the end the way every other footer does.
#[test]
fn golden_search_narrow_80() {
    let app = searching(app_graphite(fixture_archived()), "ac");
    golden("search_80x24", &render(&app, 80, 24));
}

/// A query nothing matches says so in a sentence, and names the half of the
/// board it did not look in when that is the reason.
#[test]
fn golden_search_empty_120() {
    let mut app = searching(app_graphite(fixture_archived()), "zzzz");
    assert!(render(&app, 120, 30).iter().any(|l| l.contains("no matches on this board")));
    app.handle_key(key::KeyCode::Tab, key::KeyModifiers::NONE).expect("tab");
    golden("search_no_matches_120x30", &render(&app, 120, 30));
}

/// Open ticket `n`'s page and come back to the board. The visit is recorded
/// after a keypress on the page — any key; `Null` is one the page ignores.
fn visit(app: &mut App, n: u128) {
    app.screen = Screen::Ticket { ticket: ulid_n(n), rail_idx: 0 };
    app.handle_key(key::KeyCode::Null, key::KeyModifiers::NONE).expect("a no-op press");
    app.screen = Screen::Board;
}

/// With pages opened this run, `/` opens on THEM, newest first, under a
/// subtitle that says so (T-355) — not on the whole board.
#[test]
fn golden_search_recent_120() {
    let mut app = app_graphite(fixture_archived());
    visit(&mut app, 3);
    visit(&mut app, 7);
    visit(&mut app, 1);
    visit(&mut app, 3);
    let app = searching(app, "");
    let rows: Vec<String> = app.search_rows();
    assert_eq!(rows, ["T-3", "T-1", "T-7"], "newest first, one copy of each");
    golden("search_recent_120x30", &render(&app, 120, 30));
}

/// The first keystroke is the board again; deleting it back to nothing is
/// the recent list again. And `tab` hides an archived page like any other
/// archived row.
#[test]
fn search_recent_yields_to_a_query_and_to_the_archive_toggle() {
    let mut app = app_graphite(fixture_archived());
    visit(&mut app, 7);
    visit(&mut app, 2);
    let mut app = searching(app, "a");
    let lines = render(&app, 120, 30);
    assert!(!lines.iter().any(|l| l.contains("viewed recently")), "a query is the board");
    assert!(app.search_rows().len() > 2, "the whole board, ranked");
    app.handle_key(key::KeyCode::Backspace, key::KeyModifiers::NONE).expect("clear");
    assert_eq!(app.search_rows(), ["T-2", "T-7"]);
    assert!(render(&app, 120, 30).iter().any(|l| l.contains("viewed recently")));
    app.handle_key(key::KeyCode::Tab, key::KeyModifiers::NONE).expect("live only");
    assert_eq!(app.search_rows(), ["T-2"], "the archived page is hidden with the archive");
    // Nothing visited yet: the picker is the board, as before.
    let fresh = searching(app_graphite(fixture_archived()), "");
    assert_eq!(fresh.search_rows().len(), 7);
    assert!(!render(&fresh, 120, 30).iter().any(|l| l.contains("viewed recently")));
}

/// The highlight is the value ramp and nothing else: the matched characters
/// come up to `base` and go bold, their neighbours sit at `dim1`, and the
/// saturated colour never appears — it is needs-you's and nothing else's.
#[test]
fn search_highlights_on_the_value_ramp_and_never_on_the_attn_colour() {
    let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
    let app = searching(App::for_test(fixture_archived(), theme), "decay");
    let buf = cells(&app, 120, 30);
    let theme = &app.theme;
    // The LIST row, not the preview's copy of the same title: the frame the
    // draw recorded says where the list pane is, so the sweep cannot wander
    // into the pane beside it.
    let pane = app.frames.borrow()[0];
    let row = lines_of(&buf)
        .iter()
        .position(|l| l.contains("T-1") && l.contains("Decay treatments"))
        .expect("the hit is on screen") as u16;
    let mut lit = 0usize;
    let mut rest = 0usize;
    for x in pane.x + 1..pane.x + pane.width - 1 {
        let cell = &buf[(x, row)];
        assert_ne!(cell.fg, theme.attn, "the saturated colour is needs-you's alone");
        if cell.symbol().trim().is_empty() {
            continue;
        }
        if cell.modifier.contains(Modifier::BOLD) {
            assert_eq!(cell.fg, theme.sel.base, "a matched cell is the ramp's top");
            lit += 1;
        } else if cell.fg == theme.sel.dim1 {
            rest += 1;
        }
    }
    // `decay` lands on five characters of the title and nothing else; the
    // rest of the title is the step below.
    assert_eq!(lit, 5, "exactly the matched characters are lifted");
    assert!(rest > 0, "and the rest of the row is a step down from them");
}

/// A list longer than the pane: the window follows the cursor rather than the
/// other way round, so `^n` at the bottom edge scrolls by exactly one and the
/// selected row is never off screen. The off-by-one here is the whole reason
/// this is a render test and not a state one — `top` is written by the draw,
/// because only the draw knows how tall the pane is.
#[test]
fn the_search_list_scrolls_by_one_to_keep_the_cursor_on_screen() {
    const N: usize = 40;
    let mut board = fixture(false);
    for n in 0..N {
        let key = 20 + n as u128;
        board.tickets.push(ticket(key, &format!("T-{key}"), &format!("Zebra {key}"), "todo", "z"));
    }
    // A query no fixture ticket answers, so the list is exactly these rows.
    let mut app = searching(app_graphite(board), "zebra");
    // The LIST's rows: the preview draws the cursor row's title too, and only
    // a list row carries the ticket's key beside it.
    fn rows(app: &App) -> Vec<String> {
        render(app, 120, 30)
            .into_iter()
            .filter(|l| l.contains("Zebra ") && l.contains("T-"))
            .collect()
    }
    let down = |app: &mut App| {
        app.handle_key(key::KeyCode::Char('n'), key::KeyModifiers::CONTROL).expect("down");
    };
    let visible = rows(&app).len();
    assert!(visible < N, "the fixture must be taller than the pane, or this proves nothing");
    assert!(rows(&app)[0].contains("Zebra 20"), "{:?}", rows(&app));

    // Walk to the last visible row: nothing has scrolled yet.
    for _ in 0..visible - 1 {
        down(&mut app);
    }
    assert!(rows(&app)[0].contains("Zebra 20"), "{:?}", rows(&app));
    // One more, and the window steps by exactly one.
    down(&mut app);
    let shown = rows(&app);
    assert!(shown[0].contains("Zebra 21"), "{shown:?}");
    assert_eq!(shown.len(), visible, "the pane did not change size");

    // On to the last hit, then one more, which wraps the ring and the window
    // with it.
    for _ in 0..N - 1 - visible {
        down(&mut app);
    }
    assert!(rows(&app).last().expect("a row").contains(&format!("Zebra {}", 19 + N)));
    down(&mut app);
    assert!(rows(&app)[0].contains("Zebra 20"), "{:?}", rows(&app));
}

#[test]
fn golden_mesophon_pair_and_revoke() {
    let mut app = app_graphite(fixture_archived());
    app.mesophon_available = true;
    app.mode = Mode::Sharing { idx: 0, editing: None, armed: false };
    golden("sharing_remote_control_120x30", &render(&app, 120, 30));
    app.mesophon_dialog = true;
    app.team.device = crate::app::shared_team_fixture().device;
    app.seed_team_drafts_for_test();
    app.control = mesimon_core::mesophon::Info {
        enabled: true,
        connected: true,
        origin: "https://relay.example:8444".into(),
        code: Some("msmn1-example-pairing-code".into()),
        error: None,
        devices: vec![mesimon_core::mesophon::Device {
            grant: "phone".into(),
            name: "My phone".into(),
        }],
    };
    let rows = app.sharing_rows();
    assert!(!rows.contains(&SharingRow::Publish));
    assert!(!rows.contains(&SharingRow::Join));
    let idx = rows.iter().position(|r| matches!(r, SharingRow::Code(_))).unwrap();
    app.mode = Mode::Sharing { idx, editing: None, armed: false };
    golden("mesophon_pair_120x30", &render(&app, 120, 30));
    let idx = rows.iter().position(|r| matches!(r, SharingRow::ControlDevice(_))).unwrap();
    app.mode = Mode::Sharing { idx, editing: None, armed: true };
    golden("mesophon_revoke_80x24", &render(&app, 80, 24));
}

#[test]
fn golden_editor_picture_120() {
    let mut app = app_noted();
    app.screen = Screen::Ticket { ticket: ulid_n(3), rail_idx: 0 };
    let body = format!(
        "The failure appears here:\n[Image #1]({})\nExpected: the whole screenshot is visible.",
        mesimon_core::attachment::target(ulid_n(91))
    );
    app.mode = Mode::Editor(editor_on(
        crate::app::EditorPurpose::Note { ticket: ulid_n(3), note: Some(ulid_n(90)) },
        "Fix OSC-11 detection",
        &body,
    ));
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("[Image #1]")));
    assert!(!lines.iter().any(|l| l.contains("mesimon-attachment")));
    golden("editor_picture_120x30", &lines);
}

#[test]
fn golden_unavailable_picture_link_120() {
    let mut app = app_noted();
    app.mode = Mode::Links {
        ticket: ulid_n(3),
        idx: 0,
        links: vec![crate::app::TicketLink {
            label: Some("Image #1".into()),
            text: mesimon_core::attachment::target(ulid_n(91)),
            target: crate::app::LinkTarget::Attachment {
                ticket: ulid_n(3),
                attachment: ulid_n(91),
            },
        }],
    };
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("image unavailable on this machine")));
    assert!(!lines.iter().any(|l| l.contains("mesimon-attachment")));
    golden("picture_link_unavailable_120x30", &lines);
}

/// One synthetic drawer row, named so a test can find it on screen.
fn external_item(n: usize) -> mesimon_core::command::ExternalItem {
    mesimon_core::command::ExternalItem {
        id: uuid::Uuid::from_u128(0x5000 + n as u128),
        provider: mesimon_core::board::AgentProvider::ClaudeCode,
        conversation_id: format!("conv-{n}"),
        cwd: "/repo".into(),
        transcript_path: format!("/t/{n}.jsonl"),
        mtime_ms: 1_700_000_000_000,
        preview: Some(format!("preview of session {n}")),
        name: Some(format!("session-{n:03}")),
        running_elsewhere: false,
    }
}

/// The External drawer over a census still walking (T-437): it opens on
/// nothing and says so, spins over a stale answer, and only a *finished*
/// empty census closes it with the word.
#[test]
fn external_drawer_waits_for_the_census_and_closes_only_on_a_finished_empty_one() {
    let mut app = app_graphite(fixture(false));
    app.external_scanning = true;
    app.mode = Mode::External { idx: 0 };
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("EXTERNAL")), "{lines:#?}");
    assert!(lines.iter().any(|l| l.contains("scanning for sessions started outside mesimon")));
    // Still walking, still empty: the drawer stays.
    app.settle_drawer();
    assert!(matches!(app.mode, Mode::External { .. }));
    // A stale answer under a new walk: the rows show and the title spins.
    app.external = vec![external_item(1)];
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("session-001")), "{lines:#?}");
    assert!(!lines.iter().any(|l| l.contains("EXTERNAL ∙ 1")), "spinning, not counted");
    // The walk lands with nothing: closed, with the word.
    app.external.clear();
    app.external_scanning = false;
    app.settle_drawer();
    assert!(matches!(app.mode, Mode::Normal));
    assert_eq!(app.status, "no external sessions found for this repo");
    // Landed with rows: the plain counted title.
    app.external = vec![external_item(1), external_item(2)];
    app.mode = Mode::External { idx: 0 };
    let lines = render(&app, 120, 30);
    assert!(lines.iter().any(|l| l.contains("EXTERNAL ∙ 2")), "{lines:#?}");
}

/// A list dialog taller than the screen scrolls to its cursor (T-437): the
/// selected row is always drawn, the title carries the position, and every
/// index from the first to the last is reachable.
#[test]
fn list_dialog_scrolls_to_keep_the_cursor_on_screen() {
    let mut app = app_graphite(fixture(false));
    app.external = (0..60).map(external_item).collect();
    for idx in [0, 7, 8, 30, 59] {
        app.mode = Mode::External { idx };
        let lines = render(&app, 120, 20);
        let want = format!("session-{idx:03}");
        assert!(lines.iter().any(|l| l.contains(&want)), "row {idx} off screen:\n{lines:#?}");
        let title = format!("EXTERNAL ∙ {}/60", idx + 1);
        assert!(lines.iter().any(|l| l.contains(&title)), "{title} missing:\n{lines:#?}");
    }
    // The window moves with the cursor: the last page shows the tail.
    app.mode = Mode::External { idx: 59 };
    let lines = render(&app, 120, 20);
    assert!(lines.iter().any(|l| l.contains("session-058")));
    assert!(!lines.iter().any(|l| l.contains("session-000")));
    // And a list that fits is counted, not positioned.
    app.external.truncate(3);
    app.mode = Mode::External { idx: 2 };
    let lines = render(&app, 120, 20);
    assert!(lines.iter().any(|l| l.contains("EXTERNAL ∙ 3")), "{lines:#?}");
    assert!(!lines.iter().any(|l| l.contains("3/3")));
}

/// The crowning sweeps the holder's title (T-442): on the cursor card, where
/// `^o` leaves it and where T-411's flash was hidden under the cursor's own
/// title ink, and on the ticket page's title row. Mid-sweep one cell of the
/// title wears the crown's tint as its ground and the letters behind it wear
/// it as ink; once the crowning is over, neither — and never `attn`.
#[test]
fn the_crowning_sweeps_the_title_on_the_card_and_the_page() {
    let t1 = ulid_n(1);
    let title = "Decay treatments";
    let crowned = |ago: u64, screen: Screen| {
        let mut b = fixture(false);
        b.crown = Some(t1);
        let mut app = app_graphite(b);
        (app.cursor_col, app.cursor_row) = (0, Some(0));
        app.crowned_at = Some((t1, mesimon_core::clock::now_ms() - ago));
        app.screen = screen;
        app
    };
    // The title's own cells, on every row that shows it.
    let title_cells = |app: &App| {
        let buf = cells(app, 120, 30);
        let mut out = Vec::new();
        for (y, line) in lines_of(&buf).iter().enumerate() {
            let Some(at) = line.find(title) else { continue };
            let x0 = line[..at].width() as u16;
            for x in x0..x0 + title.width() as u16 {
                out.push(buf[(x, y as u16)].clone());
            }
        }
        assert!(!out.is_empty(), "the title is on screen");
        out
    };
    let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
    let tint = theme.pip(5);
    for screen in [Screen::Board, Screen::Ticket { ticket: t1, rail_idx: 0 }] {
        let app = crowned(600, screen.clone());
        assert!(app.animating(), "{screen:?}: the loop runs fast while it sweeps");
        let mid = title_cells(&app);
        assert!(mid.iter().any(|c| c.bg == tint), "{screen:?}: no lit head mid-sweep");
        assert!(mid.iter().any(|c| c.fg == tint && c.bg != tint), "{screen:?}: nothing filled");
        assert!(mid.iter().all(|c| c.fg != theme.attn && c.bg != theme.attn));
        let app = crowned(3_000, screen.clone());
        assert!(!app.animating());
        let after = title_cells(&app);
        assert!(after.iter().all(|c| c.bg != tint), "{screen:?}: still lit after the crowning");
    }
}
