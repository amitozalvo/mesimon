//! The tick's tmux and `ps` reads ride a worker (T-857): a tmux that takes
//! two seconds to answer slows the pane sampler, never the board.
//!
//! T-856 measured the author's work machine: every tick forked `tmux` and
//! `ps` on the single writer thread, a loaded box made each fork take
//! seconds, and the board sat behind them. Here `MESIMON_TMUX_BIN` names a
//! wrapper that sleeps 2 s before the real tmux while a marker file stands;
//! with one spawned stub session a `Snapshot` round-trip stays under
//! 500 ms, and the journal names none of the moved stages as a slow turn.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use mesimon_core::board::{SessionKind, SessionState};
use mesimon_core::command::{Command, Response};
use mesimon_core::Principal;

/// The stages whose forks left the writer thread: none may be the slowest
/// stage of a slow turn.
const MOVED: [&str; 5] =
    ["refresh_panes", "refresh_rss", "probe_spawning", "probe_activity", "reclaim_archived"];

/// A directory of this test's own for the wrapper and its marker, since the
/// fixture's directory exists only once the daemon's environment is fixed.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_slow_tmux_never_holds_the_board() {
    let scratch = Scratch(std::env::temp_dir().join(format!(
        "msmn-slow-tmux-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis()
    )));
    std::fs::create_dir_all(&scratch.0).unwrap();
    let marker = scratch.0.join("slow");
    let wrapper = scratch.0.join("slow-tmux.sh");
    // The real tmux, 2 s late while the marker stands: the boot and the
    // spawn run at full speed, the measurement under the slow one.
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\n[ -e '{}' ] && sleep 2\nexec '{}' \"$@\"\n",
            marker.display(),
            mesimon_backend_tmux::tmux_bin().display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::os::unix::fs::PermissionsExt::from_mode(0o755))
        .unwrap();

    const STUB: &str = "#!/bin/sh\nexec sleep 300\n";
    let Some(h) = Harness::boot_with_env(
        "slow_tmux",
        Some(STUB),
        &[("MESIMON_TMUX_BIN", wrapper.to_str().unwrap())],
    ) else {
        return;
    };
    let tmux_sock = h.paths.tmux_sock();
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("slow_tmux");

    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "slow tmux".into(),
        workspace: None,
        tier: None,
    });
    let ticket = c.board().tickets.first().expect("ticket").id;
    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    wait_until(Duration::from_secs(15), "the stub's pane", || {
        tmux(&tmux_sock)
            .args(["list-panes", "-a", "-F", "#{pane_pid}"])
            .output()
            .map(|o| !o.stdout.is_empty())
            .unwrap_or(false)
    });

    // The shell environment's landing publishes `PATH` to the server, two
    // forks on the writer once at startup: let it land before the clock
    // starts, so what is measured is the tick's own forks.
    wait_until(Duration::from_secs(10), "the shell environment", || {
        h.paths.shell_env_file().exists()
    });
    // From here every tmux the daemon forks answers 2 s late.
    std::fs::write(&marker, b"").unwrap();
    let mut worst = Duration::ZERO;
    let mut round_trips = 0u32;
    // A round-trip the client gave up on (its 10 s read timeout) is the
    // measurement too, and the connection is remade for the next one: a
    // late answer would otherwise land as the reply to a later request.
    let mut measure = |c: &mut TestClient, for_secs: u64| {
        let until = Instant::now() + Duration::from_secs(for_secs);
        while Instant::now() < until {
            let t = Instant::now();
            let answered = c.try_send(Principal::Local, Command::Snapshot).is_some();
            let took = t.elapsed();
            if !answered {
                if let Some(mut fresh) =
                    TestClient::try_connect(&h.paths.orch_sock(), Duration::from_secs(5))
                {
                    let _ = fresh.try_send(
                        Principal::Local,
                        Command::Hello {
                            version: mesimon_core::command::PROTOCOL_VERSION,
                            client: "slow_tmux".into(),
                        },
                    );
                    *c = fresh;
                }
            }
            worst = worst.max(took);
            round_trips += 1;
            std::thread::sleep(Duration::from_millis(250));
        }
    };
    // Spawning: the startup probe at 10 s, the Claude startup screen each
    // second, the pane facts every 2 s, the RSS bucket at 10 s.
    measure(&mut c, 11);
    // Running: the activity probe each second, the server guard at 15 s.
    hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"session_id":"x"}"#);
    // The hook's edge is on the writer too: it lands when the writer does.
    measure(&mut c, 6);
    let running = c.try_send(Principal::Local, Command::Snapshot).is_some_and(|resp| {
        board_of(resp).sessions.iter().any(|s| s.id == sid && s.state == SessionState::Running)
    });
    let _ = std::fs::remove_file(&marker);

    let journal = std::fs::read_to_string(h.paths.daemon_log()).unwrap_or_default();
    let slow: Vec<&str> = journal.lines().filter(|l| l.contains("slow turn")).collect();
    eprintln!("{round_trips} round-trips, the slowest {} ms", worst.as_millis());
    // The journal's word for what tmux cost, off the writer (STALE-MAP's
    // after-number), beside any slow turn.
    for line in
        journal.lines().filter(|l| l.contains("slow pane sample") || l.contains("slow turn"))
    {
        eprintln!("{line}");
    }
    assert!(running, "the prompt's edge never reached the record:\n{}", slow.join("\n"));
    assert!(
        worst < Duration::from_millis(500),
        "a snapshot round-trip took {} ms under a 2 s tmux:\n{}",
        worst.as_millis(),
        slow.join("\n")
    );
    let named: Vec<&&str> =
        slow.iter().filter(|l| MOVED.iter().any(|stage| l.contains(stage))).collect();
    assert!(named.is_empty(), "a moved stage held the writer:\n{named:?}");
}
