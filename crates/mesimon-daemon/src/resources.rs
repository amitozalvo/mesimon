//! Resource measurement (D33e, 14 §5.1, spike ASK-23).
//!
//! RSS is a real `ps` aggregate over the process groups mesimon tracks —
//! never a per-session constant times a count. The 200 MiB constant appears
//! in exactly one place: the pre-spawn headroom estimate (spawn passes
//! through the peak). The PTY "used" figure on Darwin is the allocation
//! high-water mark, not live usage (B-D16) — budget conservatively.

use std::collections::HashSet;

/// Spawn-peak RSS (ASK-23): pre-spawn headroom only. Not a display value.
pub const SPAWN_PEAK_BYTES: u64 = 200 * 1024 * 1024;
/// 14 §5.1: never plan to consume more than this fraction of free PTYs.
const PTY_BUDGET_FRACTION: f64 = 0.40;

#[derive(Debug, Clone, Copy, Default)]
pub struct PtyFigures {
    pub total: u32,
    pub used: u32,
    pub budget: u32,
}

/// One `ps axo pgid=,rss=` fork, split per tracked pane process group —
/// the per-session figure the sleep suggestion needs. rss column is KiB on
/// both platforms.
pub fn parse_ps_rss_by(pgids: &HashSet<i32>, ps_out: &str) -> std::collections::HashMap<i32, u64> {
    let mut by: std::collections::HashMap<i32, u64> = std::collections::HashMap::new();
    for line in ps_out.lines() {
        let mut it = line.split_whitespace();
        let (Some(pgid), Some(rss)) = (it.next(), it.next()) else { continue };
        let (Ok(pgid), Ok(rss)) = (pgid.parse::<i32>(), rss.parse::<u64>()) else { continue };
        if pgids.contains(&pgid) {
            *by.entry(pgid).or_default() += rss * 1024;
        }
    }
    by
}

/// The aggregate view: (bytes, groups actually seen).
pub fn parse_ps_rss(pgids: &HashSet<i32>, ps_out: &str) -> (u64, usize) {
    let by = parse_ps_rss_by(pgids, ps_out);
    (by.values().sum(), by.len())
}

pub fn measure_rss_by(pgids: &HashSet<i32>) -> std::collections::HashMap<i32, u64> {
    if pgids.is_empty() {
        return std::collections::HashMap::new();
    }
    let Ok(out) = std::process::Command::new("ps").args(["axo", "pgid=,rss="]).output() else {
        return std::collections::HashMap::new();
    };
    parse_ps_rss_by(pgids, &String::from_utf8_lossy(&out.stdout))
}

pub fn budget_of(total: u32, used: u32) -> u32 {
    ((total.saturating_sub(used)) as f64 * PTY_BUDGET_FRACTION).floor() as u32
}

/// PTY headroom (14 §5.1). Darwin: sysctl total, `/dev/ttys*` count as the
/// (high-water) used figure. Linux: live figures from /proc.
pub fn pty_figures() -> PtyFigures {
    #[cfg(target_os = "linux")]
    {
        let read =
            |p: &str| -> Option<u32> { std::fs::read_to_string(p).ok()?.trim().parse().ok() };
        let total = read("/proc/sys/kernel/pty/max").unwrap_or(0);
        let used = read("/proc/sys/kernel/pty/nr").unwrap_or(0);
        PtyFigures { total, used, budget: budget_of(total, used) }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let total = std::process::Command::new("sysctl")
            .args(["-n", "kern.tty.ptmx_max"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
            .unwrap_or(511);
        let used = std::fs::read_dir("/dev")
            .map(|d| {
                d.flatten().filter(|e| e.file_name().to_string_lossy().starts_with("ttys")).count()
                    as u32
            })
            .unwrap_or(0);
        PtyFigures { total, used, budget: budget_of(total, used) }
    }
}

/// Free memory, best-effort. Darwin: `vm_stat` free+inactive pages (free
/// alone under-reports — the page cache gives memory back under pressure).
/// Linux: MemAvailable. `None` = could not measure — never block a spawn on
/// a failed measurement.
pub fn free_ram_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let text = std::fs::read_to_string("/proc/meminfo").ok()?;
        parse_meminfo(&text)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let out = std::process::Command::new("vm_stat").output().ok()?;
        parse_vm_stat(&String::from_utf8_lossy(&out.stdout))
    }
}

pub fn parse_meminfo(text: &str) -> Option<u64> {
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("MemAvailable:") {
            let kb: u64 = rest.trim().trim_end_matches(" kB").trim().parse().ok()?;
            return Some(kb * 1024);
        }
    }
    None
}

pub fn parse_vm_stat(text: &str) -> Option<u64> {
    let page_size: u64 = text
        .lines()
        .next()?
        .split("page size of ")
        .nth(1)?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    let pages = |label: &str| -> u64 {
        text.lines()
            .find(|l| l.starts_with(label))
            .and_then(|l| l.split(':').nth(1))
            .and_then(|v| v.trim().trim_end_matches('.').parse::<u64>().ok())
            .unwrap_or(0)
    };
    Some((pages("Pages free") + pages("Pages inactive")) * page_size)
}

/// Bytes allocated under `root` (what `du` counts; symlinks are not
/// followed), added to `total`. False when `deadline` came first, and the
/// total is then a floor: a tree with a `target/` holds hundreds of
/// thousands of files. Doctor walks under a budget; the archive offer's
/// pricing (T-679) walks off the writer thread with none.
pub fn tree_bytes(
    root: &std::path::Path,
    deadline: Option<std::time::Instant>,
    total: &mut u64,
) -> bool {
    use std::os::unix::fs::MetadataExt;
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        if deadline.is_some_and(|d| std::time::Instant::now() >= d) {
            return false;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            // `DirEntry::metadata` does not traverse a symlink.
            let Ok(m) = e.metadata() else { continue };
            *total += m.blocks() * 512;
            if m.is_dir() {
                dirs.push(e.path());
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ps_aggregate_filters_by_pgid() {
        let out = "  100  1024\n  200  2048\n  100  512\ngarbage line\n  300  9999\n";
        let pgids: HashSet<i32> = [100, 200].into_iter().collect();
        let (bytes, seen) = parse_ps_rss(&pgids, out);
        assert_eq!(bytes, (1024 + 2048 + 512) * 1024);
        assert_eq!(seen, 2);
        let (bytes, seen) = parse_ps_rss(&HashSet::new(), out);
        assert_eq!((bytes, seen), (0, 0));
    }

    #[test]
    fn ps_per_group_split_sums_each_group() {
        let out = "  100  1024\n  200  2048\n  100  512\ngarbage line\n  300  9999\n";
        let pgids: HashSet<i32> = [100, 200].into_iter().collect();
        let by = parse_ps_rss_by(&pgids, out);
        assert_eq!(by.get(&100), Some(&((1024 + 512) * 1024)));
        assert_eq!(by.get(&200), Some(&(2048 * 1024)));
        assert_eq!(by.get(&300), None);
    }

    #[test]
    fn budget_is_forty_percent_of_free() {
        assert_eq!(budget_of(511, 181), 132);
        assert_eq!(budget_of(511, 511), 0);
        assert_eq!(budget_of(0, 100), 0);
    }

    #[test]
    fn meminfo_and_vm_stat_parse() {
        let mi = "MemTotal:       16384000 kB\nMemFree:         1000000 kB\nMemAvailable:    8000000 kB\n";
        assert_eq!(parse_meminfo(mi), Some(8_000_000 * 1024));

        let vs = "Mach Virtual Memory Statistics: (page size of 16384 bytes)\n\
                  Pages free:                               10000.\n\
                  Pages active:                            200000.\n\
                  Pages inactive:                           50000.\n";
        assert_eq!(parse_vm_stat(vs), Some((10_000 + 50_000) * 16384));
        assert!(parse_vm_stat("garbage").is_none());
    }
}
