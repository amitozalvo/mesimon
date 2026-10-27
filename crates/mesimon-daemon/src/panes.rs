//! The pane sampler (T-857): every `tmux` and `ps` read the tick makes, on
//! a worker thread, one in flight at most.
//!
//! T-856 found the author's work machine stalling the writer thread for
//! seconds at a time: every tick forked `tmux` (`pane_facts`, the startup and
//! activity probes, the composer captures) and `ps` (the RSS figure, the
//! bash-mode foreground) synchronously on the single writer, and under a
//! Zoom call's load a fork that takes 50 ms idle took seconds, with every
//! hook frame and client request waiting behind it. The git sample, the
//! worktree flags and the forge already ran on workers; this is the same
//! shape for the panes.
//!
//! The writer builds a [`Query`] from its state over the ticks (`want_*` in
//! `server.rs`), hands it to one worker when none is out, and the worker
//! sends back a [`Sample`] as `Msg::PaneSampled`. The sample carries the
//! time its reads began, and the attention machine is fed THAT time, never
//! the clock at absorb: `core::attention` is time-injected on purpose, and a
//! silence measured at the read is a silence as of the read. Only the writer
//! mutates state (D22); this module computes and sends.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};

use mesimon_backend_tmux::{Access, InputScreen, PaneFacts, TmuxBackend};
use mesimon_core::board::ProcRow;

pub struct HookProcess {
    pub pid: i32,
    pub ppid: i32,
    pub pgid: i32,
    pub born_ms: u64,
    pub argv: String,
}

/// What the tick wants read, gathered until a worker is free to take it. A
/// bucket that comes round while a sample is still out adds to the query
/// rather than skipping: the read is late, never lost.
#[derive(Default)]
pub struct Query {
    /// `pane_facts`: titles, foregrounds and the `!` terminals
    /// (`refresh_panes`), and the pane pids the RSS figure measures.
    pub facts: bool,
    /// `activity`: the activity probe has a `Running` candidate.
    pub activity: bool,
    /// The startup probe's candidates (`needs_poll` said so, which advances
    /// the adapter's stage, so each is asked exactly once): the record and
    /// its pane's name, for one `capture_tail` and one `pane_title` each.
    pub startup: Vec<(uuid::Uuid, String)>,
    /// Panes whose input screen is wanted: the Claude and Codex startup
    /// probes' candidates and the panes with launch words parked.
    pub screens: Vec<String>,
    /// The RSS figure is due: `ps` over the pane process groups of these
    /// session names (ours), read off the facts.
    pub rss: Option<HashSet<String>>,
    /// `ps axo` for the bash-mode foreground (T-707): an idle Claude pane
    /// exists.
    pub rows: bool,
    /// Full argv for a Claude turn waiting on unaccounted child processes.
    pub hook_rows: bool,
    /// `server_alive`, for the wholesale-death guard.
    pub alive: bool,
    /// `folder_access`, while the cut-off notice stands (T-690).
    pub access: bool,
}

impl Query {
    pub fn is_empty(&self) -> bool {
        !self.facts
            && !self.activity
            && self.startup.is_empty()
            && self.screens.is_empty()
            && self.rss.is_none()
            && !self.rows
            && !self.hook_rows
            && !self.alive
            && !self.access
    }

    /// Ask for a pane's screen, once.
    pub fn screen(&mut self, sid16: &str) {
        if !self.screens.iter().any(|s| s == sid16) {
            self.screens.push(sid16.to_string());
        }
    }

    /// Ask the startup probe of a record, keeping one ask per record.
    pub fn startup(&mut self, id: uuid::Uuid, sid16: String) {
        self.startup.retain(|(other, _)| *other != id);
        self.startup.push((id, sid16));
    }

    /// How many forks the worker will make: the journal's word for a slow
    /// sample.
    pub fn forks(&self) -> usize {
        usize::from(self.facts || self.rss.is_some())
            + usize::from(self.activity)
            + self.startup.len() * 2
            + self.screens.len()
            + usize::from(self.rss.is_some())
            + usize::from(self.rows)
            + usize::from(self.hook_rows)
            + usize::from(self.alive)
            + usize::from(self.access)
    }
}

/// What the worker read. `None` where the query did not ask or tmux did not
/// answer: the writer's absorb of each part runs only on `Some`.
pub struct Sample {
    pub gen: u64,
    /// Epoch ms when the reads began: the time every observation is dated.
    pub at: u64,
    pub facts: Option<Vec<PaneFacts>>,
    pub activity: Option<Vec<(String, u64)>>,
    /// Per startup candidate: whether the pane has printed anything, and its
    /// title.
    pub startup: Vec<(uuid::Uuid, bool, Option<String>)>,
    pub screens: HashMap<String, InputScreen>,
    /// RSS by pane process group, over the query's names.
    pub rss: Option<HashMap<i32, u64>>,
    pub rows: Option<Vec<ProcRow>>,
    /// None means not requested; Err means the requested process read failed.
    pub hook_rows: Option<Result<Vec<HookProcess>, ()>>,
    pub alive: Option<bool>,
    pub access: Option<Access>,
    /// How long the reads took, for the journal.
    pub took: Duration,
    pub forks: usize,
}

/// The worker's whole job: every read the query asks, in one pass.
pub fn take(backend: &TmuxBackend, repo: &Path, q: Query, gen: u64, at: u64) -> Sample {
    let started = Instant::now();
    let forks = q.forks();
    let facts = (q.facts || q.rss.is_some()).then(|| backend.pane_facts().ok()).flatten();
    let rows = q.rows.then(process_rows);
    let hook_rows = q.hook_rows.then(|| facts.as_deref().and_then(hook_process_rows).ok_or(()));
    let rss = match (&q.rss, &facts) {
        (Some(ours), Some(facts)) => {
            let pgids: HashSet<i32> = facts
                .iter()
                .filter(|p| !p.pane_dead && ours.contains(&p.session_name))
                .map(|p| p.pane_pid)
                .collect();
            Some(crate::resources::measure_rss_by(&pgids))
        }
        _ => None,
    };
    let activity = q.activity.then(|| backend.activity().ok()).flatten();
    let startup = q
        .startup
        .iter()
        .map(|(id, sid)| {
            let has_output =
                backend.capture_tail(sid, 3).map(|lines| !lines.is_empty()).unwrap_or(false);
            (*id, has_output, backend.pane_title(sid).ok())
        })
        .collect();
    let screens = q
        .screens
        .iter()
        .filter_map(|sid| backend.capture_input_screen(sid).ok().map(|s| (sid.clone(), s)))
        .collect();
    let alive = q.alive.then(|| backend.server_alive());
    let access = q.access.then(|| backend.folder_access(repo));
    Sample {
        gen,
        at,
        facts,
        activity,
        startup,
        screens,
        rss,
        rows,
        hook_rows,
        alive,
        access,
        took: started.elapsed(),
        forks,
    }
}

/// The process table, as `bash_mode_foreground` reads it (T-707).
pub fn process_rows() -> Vec<ProcRow> {
    let Ok(out) =
        std::process::Command::new("ps").args(["axo", "pid=,ppid=,etime=,comm="]).output()
    else {
        return Vec::new();
    };
    mesimon_core::board::parse_ps_rows(&String::from_utf8_lossy(&out.stdout))
}

/// A failed read is not an empty tree. Full argv stays in memory and is
/// never put in the feed (it can contain the person's prompt).
fn hook_process_rows(facts: &[PaneFacts]) -> Option<Vec<HookProcess>> {
    let out = std::process::Command::new("ps")
        .args(["axww", "-o", "pid=,ppid=,etime=,args="])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let roots: HashSet<_> = facts.iter().filter(|f| !f.pane_dead).map(|f| f.pane_pid).collect();
    let birth = birth_reader();
    let mut rows = Vec::new();
    for row in mesimon_core::board::parse_ps_rows(&String::from_utf8_lossy(&out.stdout)) {
        if !roots.contains(&row.pid) && !roots.contains(&row.ppid) {
            continue;
        }
        // Whole-second etime cannot distinguish infrastructure born just
        // before a prompt from a hook born during the turn. An unreadable
        // birth leaves this sample unknown, never an empty tree.
        let Some((born_ms, pgid)) = birth(row.pid) else {
            // Exiting between ps and the birth read is positive absence,
            // not a broken reader: it must retain the rewake grace.
            // SAFETY: signal zero only checks whether this PID exists.
            let gone = unsafe { libc::kill(row.pid, 0) } == -1
                && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
            if gone {
                continue;
            }
            return None;
        };
        rows.push(HookProcess { pid: row.pid, ppid: row.ppid, pgid, born_ms, argv: row.comm });
    }
    Some(rows)
}

/// A process's birth (epoch ms) and process group, by pid.
#[cfg(target_os = "macos")]
fn birth_reader() -> impl Fn(i32) -> Option<(u64, i32)> {
    process_birth_ms
}

/// A process's birth (epoch ms) and process group, by pid: the boot clock
/// read once a sample, not once a process.
#[cfg(target_os = "linux")]
fn birth_reader() -> impl Fn(i32) -> Option<(u64, i32)> {
    let clock = boot_clock();
    move |pid| process_birth_ms(pid, clock.as_ref()?)
}

#[cfg(target_os = "macos")]
fn process_birth_ms(pid: i32) -> Option<(u64, i32)> {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::uninit();
    let size = std::mem::size_of::<libc::proc_bsdinfo>();
    // SAFETY: proc_pidinfo receives a correctly sized writable buffer; it
    // is read only after the kernel reports that it filled the whole one.
    let read = unsafe {
        libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, info.as_mut_ptr().cast(), size as i32)
    };
    if read != size as i32 {
        return None;
    }
    // SAFETY: the successful read above initialized every byte.
    let info = unsafe { info.assume_init() };
    Some((
        info.pbi_start_tvsec.saturating_mul(1000) + info.pbi_start_tvusec / 1000,
        info.pbi_pgid as i32,
    ))
}

/// When the machine booted (epoch ms) and its clock ticks a second: what
/// `/proc/<pid>/stat`'s start time is counted from and in.
#[cfg(target_os = "linux")]
struct BootClock {
    boot_ms: u64,
    hz: u64,
}

#[cfg(target_os = "linux")]
fn boot_clock() -> Option<BootClock> {
    let uptime = std::fs::read_to_string("/proc/uptime").ok()?;
    let uptime = uptime.split_whitespace().next()?.parse::<f64>().ok()?;
    // SAFETY: sysconf reads a constant system clock rate and takes no pointers.
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if hz <= 0 {
        return None;
    }
    let now = mesimon_core::clock::now_ms();
    Some(BootClock { boot_ms: now.saturating_sub((uptime * 1000.0) as u64), hz: hz as u64 })
}

#[cfg(target_os = "linux")]
fn process_birth_ms(pid: i32, clock: &BootClock) -> Option<(u64, i32)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, fields) = stat.rsplit_once(')')?;
    let pgid = fields.split_whitespace().nth(2)?.parse::<i32>().ok()?;
    let ticks = fields.split_whitespace().nth(19)?.parse::<u64>().ok()?;
    Some((clock.boot_ms + ticks * 1000 / clock.hz, pgid))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_counts_its_forks_and_asks_each_pane_once() {
        let mut q = Query::default();
        assert!(q.is_empty());
        q.screen("a");
        q.screen("a");
        q.screen("b");
        let id = uuid::Uuid::new_v4();
        q.startup(id, "a".into());
        q.startup(id, "a".into());
        q.facts = true;
        q.rss = Some(HashSet::new());
        q.rows = true;
        assert!(!q.is_empty());
        assert_eq!(q.screens, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(q.startup.len(), 1);
        // facts once for the facts and the rss, two per startup, one per
        // screen, the rss's ps, the rows' ps.
        assert_eq!(q.forks(), 1 + 2 + 2 + 1 + 1);
    }
}
