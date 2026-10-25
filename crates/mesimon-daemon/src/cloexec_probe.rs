//! The close-on-exec gap on macOS (T-981), measured. `cargo test -p mesimon-daemon
//! cloexec_probe -- --ignored --nocapture` prints the counts STALE-MAP cites.
//!
//! macOS has no `accept4` and no `SOCK_CLOEXEC`/`pipe2`, so std's `UnixListener::accept`
//! (and `UnixStream::pair`, `connect`, `io::pipe`) makes the fd and only then sets
//! `FD_CLOEXEC`. A spawn on another thread inside that gap hands the child a copy. The
//! copy is of the daemon's own end: the daemon still reads EOF when its client closes,
//! and it is the client that waits past the daemon's close until the child exits.
use std::io::{ErrorKind, Read};
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

const ACCEPTS: u64 = 100_000;

/// Readable within `ms`: an EOF, since nothing is ever written.
fn eof_within(fd: &impl AsRawFd, ms: i32) -> bool {
    let mut p = libc::pollfd { fd: fd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
    unsafe { libc::poll(&mut p, 1, ms) == 1 }
}

/// In the forked child, before exec: every fd above 2 close-on-exec. The spawn-side
/// fix this file measures; `pre_exec` makes std fork instead of `posix_spawn`.
fn cloexec_all() -> std::io::Result<()> {
    let mut fds = [libc::proc_fdinfo { proc_fd: 0, proc_fdtype: 0 }; 1024];
    let size = std::mem::size_of_val(&fds) as libc::c_int;
    let bytes = unsafe {
        libc::proc_pidinfo(libc::getpid(), libc::PROC_PIDLISTFDS, 0, fds.as_mut_ptr().cast(), size)
    };
    let n = bytes.max(0) as usize / std::mem::size_of::<libc::proc_fdinfo>();
    for f in fds.iter().take(n).filter(|f| f.proc_fd > 2) {
        unsafe { libc::fcntl(f.proc_fd, libc::F_SETFD, libc::FD_CLOEXEC) };
    }
    Ok(())
}

fn sleeper(sweep: bool) -> Command {
    let mut cmd = Command::new("/bin/sleep");
    cmd.arg("0.05").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    if sweep {
        unsafe { cmd.pre_exec(cloexec_all) };
    }
    cmd
}

/// A thread spawning `/bin/sleep 0.05` as fast as it can (or every `pace`), reaping as it goes.
struct Spawner {
    stop: Arc<AtomicBool>,
    spawned: Arc<AtomicU64>,
    thread: std::thread::JoinHandle<()>,
}

impl Spawner {
    fn start(sweep: bool, pace: Option<Duration>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let spawned = Arc::new(AtomicU64::new(0));
        let (s, n) = (stop.clone(), spawned.clone());
        let thread = std::thread::spawn(move || {
            let mut kids: Vec<Child> = Vec::new();
            while !s.load(Ordering::Relaxed) {
                if let Ok(child) = sleeper(sweep).spawn() {
                    kids.push(child);
                    n.fetch_add(1, Ordering::Relaxed);
                }
                kids.retain_mut(|c| !matches!(c.try_wait(), Ok(Some(_))));
                if let Some(pace) = pace {
                    std::thread::sleep(pace);
                }
            }
            for mut c in kids {
                let _ = c.wait();
            }
        });
        Spawner { stop, spawned, thread }
    }

    fn finish(self) -> u64 {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.join().expect("spawner");
        self.spawned.load(Ordering::Relaxed)
    }
}

struct Run {
    accepts: u64,
    spawns: u64,
    secs: f64,
    /// The daemon's end, dropped at accept, still open 10 ms later: a child holds it.
    held: u64,
    /// Of `held`, the ones that read EOF once every child was reaped.
    released: u64,
}

/// The daemon's shape: one thread accepts and drops each connection at once; the
/// client asks whether its peer is gone.
fn accept_run(accepts: u64, spawner: Option<Spawner>) -> Run {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("probe.sock");
    let listener = UnixListener::bind(&path).expect("bind");
    let (tx, rx) = mpsc::channel::<()>();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            drop(stream);
            if tx.send(()).is_err() {
                break;
            }
        }
    });
    let started = Instant::now();
    let mut held = Vec::new();
    for _ in 0..accepts {
        let client = UnixStream::connect(&path).expect("connect");
        rx.recv().expect("accepted");
        client.set_nonblocking(true).expect("nonblocking");
        match (&client).read(&mut [0u8; 1]) {
            Ok(0) => {}
            // A fork child holds it until its exec (well under 10 ms); an exec'd
            // `/bin/sleep 0.05` holds it for 50 ms.
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                if !eof_within(&client, 10) {
                    held.push(client);
                }
            }
            other => panic!("the client read {other:?}"),
        }
    }
    let secs = started.elapsed().as_secs_f64();
    let spawns = spawner.map_or(0, Spawner::finish);
    let released = held.iter().filter(|c| eof_within(*c, 2000)).count() as u64;
    Run { accepts, spawns, secs, held: held.len() as u64, released }
}

fn report(label: &str, r: &Run) {
    println!(
        "{label}: {} accepts, {} spawns in {:.1} s ({:.0}/s): held {} ({:.1e} per accept), released after the children {}",
        r.accepts,
        r.spawns,
        r.secs,
        r.spawns as f64 / r.secs,
        r.held,
        r.held as f64 / r.accepts as f64,
        r.released,
    );
}

#[test]
#[ignore = "a measurement (T-981), ~75 s in a debug build; run with --ignored --nocapture"]
fn cloexec_probe() {
    let control = accept_run(ACCEPTS, None);
    report("no spawner", &control);
    assert_eq!(control.held, 0, "nothing spawned, so nothing can hold a dropped end");

    let flat_out = accept_run(ACCEPTS, Some(Spawner::start(false, None)));
    report("std spawns, flat out", &flat_out);
    assert_eq!(flat_out.released, flat_out.held, "every held end was a child's copy");

    // The board daemon's rate, measured from its children (≈ 6 a second).
    let paced =
        accept_run(30 * ACCEPTS, Some(Spawner::start(false, Some(Duration::from_millis(166)))));
    report("std spawns, 6 a second", &paced);
    assert_eq!(paced.released, paced.held);

    let swept = accept_run(3 * ACCEPTS, Some(Spawner::start(true, None)));
    report("fork + pre_exec sweep, flat out", &swept);
    assert_eq!(swept.held, 0, "the sweep leaves no exec'd child an inherited end");

    // std's pipe has the same gap: a dropped writer that a child holds keeps the read open.
    let spawner = Spawner::start(false, None);
    let pipes_held = (0..ACCEPTS)
        .filter(|_| {
            let (reader, writer) = std::io::pipe().expect("pipe");
            drop(writer);
            !eof_within(&reader, 10)
        })
        .count();
    let spawns = spawner.finish();
    println!("pipes: {ACCEPTS} pipes beside {spawns} spawns: writer held {pipes_held}");

    // The cost of the sweep: std's posix_spawn against fork + sweep, for a child with a pipe.
    let ballast = vec![1u8; 48 << 20]; // the board daemon's resident size, touched
    let per_spawn = |sweep: bool| {
        let started = Instant::now();
        for _ in 0..500 {
            let mut cmd = Command::new("/usr/bin/true");
            cmd.stdout(Stdio::piped());
            if sweep {
                unsafe { cmd.pre_exec(cloexec_all) };
            }
            cmd.output().expect("true");
        }
        started.elapsed().as_secs_f64() * 1000.0 / 500.0
    };
    println!(
        "cost: posix_spawn {:.2} ms a spawn, fork + sweep {:.2} ms (ballast {} MiB)",
        per_spawn(false),
        per_spawn(true),
        ballast.len() >> 20,
    );
}

/// Which end a child's copy keeps open, made certain by handing one over on purpose.
#[test]
#[ignore = "spawns a child that holds an fd for 0.3 s (T-981); run with --ignored"]
fn a_child_holding_the_daemons_end_delays_the_client_not_the_daemon() {
    let dir = tempfile::tempdir().expect("tempdir");
    let listener = UnixListener::bind(dir.path().join("s.sock")).expect("bind");
    let leak = |stream: &UnixStream| {
        assert_eq!(unsafe { libc::fcntl(stream.as_raw_fd(), libc::F_SETFD, 0) }, 0);
        let child =
            Command::new("/bin/sleep").arg("0.3").stdin(Stdio::null()).spawn().expect("sleep");
        assert_eq!(unsafe { libc::fcntl(stream.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) }, 0);
        child
    };

    // The client closes: the daemon reads EOF at once, whoever holds the daemon's end.
    let client = UnixStream::connect(dir.path().join("s.sock")).expect("connect");
    let (daemon_end, _) = listener.accept().expect("accept");
    let mut first = leak(&daemon_end);
    drop(client);
    assert!(eof_within(&daemon_end, 100), "the daemon's read sees its client's close");

    // The daemon closes: the client waits for the child.
    let client = UnixStream::connect(dir.path().join("s.sock")).expect("connect");
    let (daemon_end, _) = listener.accept().expect("accept");
    let mut second = leak(&daemon_end);
    drop(daemon_end);
    assert!(!eof_within(&client, 200), "the child's copy keeps the client's read open");
    let _ = second.wait();
    assert!(eof_within(&client, 100), "and the child's exit closes it");

    // A shutdown acts on the socket, not the fd: the client's EOF no longer waits for copies.
    let client = UnixStream::connect(dir.path().join("s.sock")).expect("connect");
    let (daemon_end, _) = listener.accept().expect("accept");
    let mut third = leak(&daemon_end);
    daemon_end.shutdown(std::net::Shutdown::Write).expect("shutdown");
    drop(daemon_end);
    assert!(eof_within(&client, 100), "a shut-down end reads EOF beside a child's copy");
    let _ = third.wait();
    let _ = first.wait();
}
