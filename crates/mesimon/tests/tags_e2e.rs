//! Tags (T-83) end to end against a real daemon: `SetTag` replaces within a
//! group and clears with `None`, the name is sanitized at the boundary, a bad
//! group is refused, the tags survive the disk round-trip, and `[[tags]]`
//! lands before `[archived]` in the file that gets written.
//!
//! No tmux and no agent — tags are pure board state, so this one runs
//! everywhere.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use mesimon_core::board::Board;
use mesimon_core::command::{Command, Envelope, Response};
use mesimon_core::Principal;

struct TestClient {
    write: UnixStream,
    read: BufReader<UnixStream>,
}

impl TestClient {
    /// Retries, like `restart_e2e`'s: this test restarts the daemon mid-run,
    /// and the socket file exists from `bind` a moment before `listen` is
    /// accepting on it. Waiting for the path alone would connect into that
    /// window on a slower runner.
    fn connect(sock: &std::path::Path) -> Self {
        let deadline = Instant::now() + Duration::from_secs(5);
        let stream = loop {
            match UnixStream::connect(sock) {
                Ok(s) => break s,
                Err(e) => {
                    assert!(Instant::now() < deadline, "daemon never accepted: {e}");
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        };
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let read = BufReader::new(stream.try_clone().unwrap());
        Self { write: stream, read }
    }

    fn request(&mut self, command: Command) -> Response {
        let env = Envelope { principal: Principal::Local, command };
        let line = serde_json::to_string(&env).unwrap();
        writeln!(self.write, "{line}").unwrap();
        loop {
            let mut buf = String::new();
            self.read.read_line(&mut buf).expect("read");
            if let Ok(resp) = serde_json::from_str::<Response>(&buf) {
                return resp;
            }
        }
    }
}

fn board_of(resp: Response) -> Board {
    match resp {
        Response::Board { board, .. } => board,
        other => panic!("expected board, got {other:?}"),
    }
}

fn err_containing(resp: Response, needle: &str) {
    match resp {
        Response::Err { message } => {
            assert!(message.contains(needle), "expected {needle:?} in {message:?}")
        }
        other => panic!("expected refusal containing {needle:?}, got {other:?}"),
    }
}

#[test]
fn tags_round_trip_through_the_daemon_and_the_disk() {
    let dir = std::path::PathBuf::from(format!("/tmp/msmn-e2e-tags-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    let paths = mesimon_daemon::Paths::for_repo(&repo).unwrap();
    let sock = paths.orch_sock();
    let state_dir = paths.state_dir.clone();
    let rt_dir = paths.rt_dir.clone();

    let daemon_repo = repo.clone();
    let daemon = std::thread::spawn(move || {
        let _ = mesimon_daemon::run_foreground(&daemon_repo);
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() {
        assert!(Instant::now() < deadline, "daemon socket never appeared");
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut c = TestClient::connect(&sock);
    assert!(matches!(
        c.request(Command::Hello { version: 1, client: "tags".into() }),
        Response::Hello { .. }
    ));

    let _ = c.request(Command::CreateTicket { column: "TODO".into(), title: "tag me".into() });
    let board = board_of(c.request(Command::Snapshot));
    let id = board.tickets[0].id;
    let key = board.tickets[0].short_key.clone();
    let ticket_toml = repo.join(".mesimon/board/tickets").join(&key).join("ticket.toml");

    // ---- one per group ----------------------------------------------------
    let set = |c: &mut TestClient, group: u8, name: Option<&str>| {
        c.request(Command::SetTag { id, group, name: name.map(|s| s.to_string()) })
    };
    assert!(matches!(set(&mut c, 1, Some("BUG")), Response::Ok));
    assert!(matches!(set(&mut c, 2, Some("STAGING")), Response::Ok));
    let board = board_of(c.request(Command::Snapshot));
    let t = board.ticket(id).unwrap();
    assert_eq!(t.tags.len(), 2);
    assert_eq!(t.tag_in(1).unwrap().name, "BUG");
    assert_eq!(t.tag_in(2).unwrap().name, "STAGING");

    // Setting the same group again REPLACES — a group is an axis.
    assert!(matches!(set(&mut c, 1, Some("REGR")), Response::Ok));
    let board = board_of(c.request(Command::Snapshot));
    let t = board.ticket(id).unwrap();
    assert_eq!(t.tags.len(), 2, "replaced, not appended");
    assert_eq!(t.tag_in(1).unwrap().name, "REGR");

    // The registry is DERIVED, and this is the consequence to be explicit
    // about: nothing wears BUG any more, so BUG is gone from the vocabulary.
    // A tag exists exactly as long as some ticket carries it — which is what
    // "create on the fly, nothing seeded" costs, and why there is no rename
    // or delete to build.
    assert_eq!(board.group_tags(1), vec!["REGR"]);

    // Give BUG a home on a second ticket and it is back in the cycle, for
    // both tickets, sorted — the order never depends on who wears what.
    let _ = c.request(Command::CreateTicket { column: "TODO".into(), title: "other".into() });
    let board = board_of(c.request(Command::Snapshot));
    let other = board.tickets.iter().find(|t| t.title == "other").unwrap().id;
    assert!(matches!(
        c.request(Command::SetTag { id: other, group: 1, name: Some("BUG".into()) }),
        Response::Ok
    ));
    let board = board_of(c.request(Command::Snapshot));
    assert_eq!(board.group_tags(1), vec!["BUG", "REGR"]);

    // ---- the boundary sanitizes and refuses --------------------------------
    err_containing(set(&mut c, 0, Some("x")), "1-9");
    err_containing(set(&mut c, 10, Some("x")), "1-9");
    err_containing(set(&mut c, 3, Some("   ")), "empty");
    // Control chars and the drawn-structure range never reach a card row.
    assert!(matches!(set(&mut c, 3, Some("a\nb\u{2500}c")), Response::Ok));
    let board = board_of(c.request(Command::Snapshot));
    assert_eq!(board.ticket(id).unwrap().tag_in(3).unwrap().name, "abc");
    // And the length cap holds.
    assert!(matches!(set(&mut c, 4, Some(&"x".repeat(80))), Response::Ok));
    let board = board_of(c.request(Command::Snapshot));
    assert!(board.ticket(id).unwrap().tag_in(4).unwrap().name.len() <= 24);

    // ---- the file on disk --------------------------------------------------
    let raw = std::fs::read_to_string(&ticket_toml).expect("ticket.toml");
    assert!(raw.contains("[[tags]]"), "tags reached the disk:\n{raw}");
    assert!(raw.contains("REGR"));
    // A scalar after a table is a TOML serialize error, so this ordering is
    // not cosmetic — it is the thing that keeps `save_ticket` from failing.
    let tags_at = raw.find("[[tags]]").unwrap();
    for scalar in ["id =", "short_key =", "title =", "column =", "created_at ="] {
        let at = raw.find(scalar).unwrap_or_else(|| panic!("{scalar} missing from\n{raw}"));
        assert!(at < tags_at, "{scalar} must serialize before [[tags]]");
    }

    // ---- clearing ----------------------------------------------------------
    assert!(matches!(set(&mut c, 1, None), Response::Ok));
    let board = board_of(c.request(Command::Snapshot));
    let t = board.ticket(id).unwrap();
    assert!(t.tag_in(1).is_none(), "cleared");
    assert_eq!(t.tag_in(2).unwrap().name, "STAGING", "other axes untouched");

    // ---- survives a daemon restart (the real migration test) ---------------
    let _ = c.request(Command::Shutdown);
    let _ = daemon.join();
    let daemon_repo = repo.clone();
    let daemon = std::thread::spawn(move || {
        let _ = mesimon_daemon::run_foreground(&daemon_repo);
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() {
        assert!(Instant::now() < deadline, "daemon socket never reappeared");
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut c = TestClient::connect(&sock);
    assert!(matches!(
        c.request(Command::Hello { version: 1, client: "tags".into() }),
        Response::Hello { .. }
    ));
    let board = board_of(c.request(Command::Snapshot));
    let t = board.ticket(id).unwrap();
    assert_eq!(t.tag_in(2).unwrap().name, "STAGING", "tags reloaded from disk");
    assert_eq!(t.tag_in(3).unwrap().name, "abc");
    assert!(t.tag_in(1).is_none());

    let _ = c.request(Command::Shutdown);
    let _ = daemon.join();
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&state_dir);
    let _ = std::fs::remove_dir_all(&rt_dir);
}
