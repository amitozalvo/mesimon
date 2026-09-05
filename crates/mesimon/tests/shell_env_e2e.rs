//! An export in the user's shell startup file, end to end into an agent's pane.
//!
//! This is the one thing no unit test can show. A Claude pane is exec'd
//! directly by tmux — no shell runs on that path, so no rc file is read — and
//! before this existed the pane's environment was a nine-name slice of whatever
//! the daemon happened to inherit, frozen at the tmux server's first launch.
//! An `export` added to `~/.zshrc` could not reach an agent, its MCP servers or
//! its hooks by any route at all.
//!
//! The test stands in a fake shell for the user's real one (`$SHELL` is what
//! `shellenv::capture` runs), so the assertion is about mesimon's wiring rather
//! than about whose dotfiles the runner happens to have. Two roads are checked
//! separately because tmux treats them differently: an ordinary variable rides
//! `new-session -e`, while `PATH` has to ride the tmux CLIENT environment —
//! tmux takes a pane's PATH from the spawning client and ignores the `-e` one.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::time::{Duration, Instant};

use mesimon_core::board::SessionKind;
use mesimon_core::command::{Command, Response};

fn shell_env_of(resp: Response) -> mesimon_core::command::ShellEnvStatus {
    match resp {
        Response::Board { shell_env, .. } => shell_env,
        other => panic!("expected board, got {other:?}"),
    }
}

/// A stand-in for the user's login shell, standing in for their rc files.
///
/// `capture` runs `$SHELL -l -i -c 'env -0 > <dump>'`, so this exports what a
/// real `~/.zshrc` would and then runs the command it was given — which is the
/// faithful simulation, not a shortcut: the dump is produced by the real
/// `env -0`, with these exports in force, exactly as it would be on a real
/// machine. `MESIMON_TICKET` is exported too, as a hostile rc would, so the
/// filter that refuses to let it shadow the real ticket is under test.
fn write_fake_shell(path: &std::path::Path, marker: &str, extra_bin: &std::path::Path) {
    std::fs::write(
        path,
        format!(
            "#!/bin/sh\n\
             export E2E_FROM_RC='{marker}'\n\
             export MESIMON_TICKET='bogus-from-rc'\n\
             export PATH='{bin}':/usr/bin:/bin\n\
             shift 3   # -l -i -c\n\
             eval \"$@\"\n",
            bin = extra_bin.display(),
            marker = marker,
        ),
    )
    .unwrap();
    let mut perm = std::fs::metadata(path).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perm, 0o755);
    std::fs::set_permissions(path, perm).unwrap();
}

#[test]
fn an_export_in_the_users_rc_reaches_an_agents_pane() {
    if !common::require_tmux() {
        return;
    }
    let fixture = common::TestFixture::new("shellenv");
    let dir = fixture.dir.clone();
    let repo = dir.join("repo");
    let bin = dir.join("bin");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&bin).unwrap();

    let fake_shell = dir.join("fakeshell");
    write_fake_shell(&fake_shell, "carried-all-the-way", &bin);

    // The agent stand-in: dumps its own environment and stays alive, so the
    // assertion is about the process tmux actually exec'd.
    let out = dir.join("agent-env.txt");
    let claude = dir.join("fakeclaude");
    std::fs::write(&claude, format!("#!/bin/sh\nenv > '{}'\nexec sleep 120\n", out.display()))
        .unwrap();
    let mut perm = std::fs::metadata(&claude).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perm, 0o755);
    std::fs::set_permissions(&claude, perm).unwrap();

    // A real rc file for the staleness clock to watch. Its CONTENTS are
    // irrelevant — the fake shell is standing in for what an rc does — but its
    // mtime is what `shell_env_stale` compares against, so it has to exist.
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join(".zshrc"), "# start\n").unwrap();

    fixture.set_env("HOME", &home);
    fixture.set_env("SHELL", &fake_shell);
    fixture.set_env("MESIMON_CLAUDE_BIN", &claude);
    fixture.set_env("MESIMON_HOOK_BIN", env!("CARGO_BIN_EXE_mesimon"));
    let paths = fixture.paths(&repo);
    let sock = paths.orch_sock();
    let tmux_sock = paths.tmux_sock();

    let daemon_repo = repo.clone();
    let daemon = fixture.daemon(&daemon_repo);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() {
        assert!(Instant::now() < deadline, "daemon socket never appeared");
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut c = TestClient::connect(&sock);
    assert!(matches!(
        c.request(Command::Hello { version: 1, client: "shellenv".into() }),
        Response::Hello { .. }
    ));

    // The startup capture is off the writer thread; wait for it to land before
    // spawning, or the spawn races it and proves nothing either way.
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let st = shell_env_of(c.request(Command::Snapshot));
        if st.vars > 0 && !st.reloading {
            break;
        }
        assert!(Instant::now() < deadline, "no shell environment was ever captured: {st:?}");
        std::thread::sleep(Duration::from_millis(200));
    }

    let _ = c.request(Command::CreateTicket { column: "TODO".into(), title: "env".into() });
    let ticket = board_of(c.request(Command::Snapshot)).tickets[0].id;
    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };

    let deadline = Instant::now() + Duration::from_secs(20);
    let env = loop {
        if let Ok(text) = std::fs::read_to_string(&out) {
            if text.contains("E2E_FROM_RC") {
                break text;
            }
        }
        assert!(Instant::now() < deadline, "the agent never dumped its environment");
        std::thread::sleep(Duration::from_millis(200));
    };

    // The whole point: a name mesimon has never heard of, exported by the
    // user's shell, inside the agent's process.
    assert!(
        env.contains("E2E_FROM_RC=carried-all-the-way"),
        "the user's own variable never reached the pane:\n{env}"
    );
    // And PATH, which cannot ride the same road — tmux takes it from the
    // spawning client. A regression here looks like nothing at all until a
    // command the user just installed is not found.
    assert!(
        env.lines().any(|l| l.starts_with("PATH=") && l.contains(&bin.display().to_string())),
        "the captured PATH never reached the pane:\n{env}"
    );
    // mesimon's own per-session variables are minted per spawn, never
    // inherited: the fake rc exports `MESIMON_TICKET=bogus-from-rc`, and the
    // pane must still carry the real ticket. A capture taken INSIDE a mesimon
    // pane would otherwise hand the next session the previous one's ticket.
    assert!(env.contains("MESIMON_TICKET="), "the ticket variable was lost:\n{env}");
    // The values travel INSIDE the pane. A `new-session -e K=V` command line
    // is readable by every user on the machine (`ps`), and that is where the
    // user's exported keys used to be.
    let start = tmux(&tmux_sock).args(["list-panes", "-a", "-F", "#{pane_start_command}"]).output();
    let start = String::from_utf8_lossy(&start.unwrap().stdout).to_string();
    assert!(start.contains(" exec "), "the pane should start through the launcher:\n{start}");
    assert!(
        !start.contains("carried-all-the-way"),
        "an exported value is on the pane's command line:\n{start}"
    );
    assert!(
        !env.contains("bogus-from-rc"),
        "a MESIMON_* from the shell must never shadow the real one:\n{env}"
    );

    // An rc file that moves is offered, and taking the offer clears it.
    assert!(!shell_env_of(c.request(Command::Snapshot)).stale, "nothing has changed yet");
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(home.join(".zshrc"), "# start\nexport ADDED=1\n").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if shell_env_of(c.request(Command::Snapshot)).stale {
            break;
        }
        assert!(Instant::now() < deadline, "an edited rc file was never noticed");
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(matches!(c.request(Command::ReloadShellEnv), Response::Ok));
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let st = shell_env_of(c.request(Command::Snapshot));
        if !st.stale && !st.reloading {
            break;
        }
        assert!(Instant::now() < deadline, "the offer never cleared: {st:?}");
        std::thread::sleep(Duration::from_millis(200));
    }

    // The dump is a copy of the user's whole environment. It must not outlive
    // the capture that made it.
    assert!(!paths.shell_env_dump().exists(), "the environment dump was left on disk");

    let _ = c.request(Command::KillSession { id: sid });
    let _ = c.request(Command::Shutdown);
    let _ = daemon.join();
}
