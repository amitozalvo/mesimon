//! A page of an agent's transcript for Remote Control (T-626). The file is
//! append-only, so a record's byte offset is its place for good: a page
//! wholly before the file's end never changes, and a phone keeps it. A page
//! is read backwards from its end in 64 KiB windows, as the peek reads the
//! tail (`claude/history.rs`), and costs what the page holds, not the file.
//! What a record says is its adapter's business (`AgentAdapter::rows`).

use mesimon_core::mesophon::{TranscriptRow, TRANSCRIPT_PAGE_BYTES, TRANSCRIPT_ROWS};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Page {
    pub rows: Vec<TranscriptRow>,
    /// The page covers the records in `[from, end)`.
    pub from: u64,
    pub end: u64,
    /// Where the page before this one ends; `None` at the file's start.
    pub next_before: Option<u64>,
}

/// Which page: the one ending at `before` (the file's last whole record
/// without), going back no further than `after`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ask {
    pub before: Option<u64>,
    pub after: Option<u64>,
    pub limit: Option<usize>,
}

const WINDOW: u64 = 64 * 1024;
/// A record longer than this is not read whole: a pasted image or a huge
/// tool result, never words a phone shows.
const LINE_MAX: usize = 2 * 1024 * 1024;
/// The most a page walks before it stops short and says where it stopped,
/// so a run of tool output with no rows in it costs a bounded read.
const SCAN_MAX: u64 = 8 * 1024 * 1024;

/// Read one page. `rows` turns a record, at its offset, into rows.
pub fn page(
    path: &Path,
    ask: Ask,
    rows: &dyn Fn(u64, &serde_json::Value) -> Vec<TranscriptRow>,
) -> std::io::Result<Page> {
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    let top = ask.before.map_or(len, |b| b.min(len));
    // `after` came from an earlier page of this file; past `top` it is not.
    let floor = ask.after.filter(|a| *a <= top).unwrap_or(0);
    let limit = ask.limit.unwrap_or(TRANSCRIPT_ROWS).clamp(1, TRANSCRIPT_ROWS);

    // Newest record first; each record's rows stay together and in order.
    let mut kept: Vec<Vec<TranscriptRow>> = Vec::new();
    let (mut count, mut bytes) = (0usize, 0usize);
    // The end of the last whole record: the bytes past it are a record
    // still being written (or nothing), and wait for the next read.
    let mut end: Option<u64> = None;
    // The start of the oldest line handled: the page's `from` so far.
    let mut from = top;
    let mut carry: Vec<u8> = Vec::new();
    let mut oversized = false;
    let mut pos = top;
    let mut stopped = false;

    // One whole line at `start`: true when the page is full and the walk
    // stops before it.
    let mut take = |start: u64, line: &[u8], oversized: bool, from: &mut u64| -> bool {
        let line_rows =
            if oversized || line.len() > LINE_MAX { Vec::new() } else { parse(start, line, rows) };
        if !line_rows.is_empty() {
            let size: usize =
                line_rows.iter().map(|r| serde_json::to_vec(r).map_or(0, |v| v.len() + 1)).sum();
            if count > 0
                && (count + line_rows.len() > limit || bytes + size > TRANSCRIPT_PAGE_BYTES)
            {
                return true;
            }
            count += line_rows.len();
            bytes += size;
            kept.push(line_rows);
        }
        *from = start;
        false
    };

    'walk: while pos > floor {
        if top - pos >= SCAN_MAX && end.is_some() {
            break;
        }
        let lo = pos.saturating_sub(WINDOW).max(floor);
        let mut buf = vec![0; (pos - lo) as usize];
        file.seek(SeekFrom::Start(lo))?;
        file.read_exact(&mut buf)?;
        // `buf` then `carry` is every byte from `lo` to the current line's
        // end; lines are cut off it from the right.
        // While a record too long to keep is being passed over, its bytes
        // are dropped as they come.
        let joined: Vec<u8> =
            if oversized { buf } else { [buf, std::mem::take(&mut carry)].concat() };
        let mut line_end = joined.len();
        for i in (0..joined.len()).rev() {
            if joined[i] != b'\n' {
                continue;
            }
            let start = lo + i as u64 + 1;
            match end {
                // The bytes after the last newline are not a whole record.
                None => end = Some(start),
                Some(_) => {
                    if take(start, &joined[i + 1..line_end], oversized, &mut from) {
                        stopped = true;
                        break 'walk;
                    }
                }
            }
            oversized = false;
            line_end = i;
        }
        carry = joined[..line_end].to_vec();
        if carry.len() > LINE_MAX {
            carry.clear();
            oversized = true;
        }
        pos = lo;
    }
    // The walk reached `floor`, a record's start: what is left is a whole
    // line, if a newline was ever seen above it. A full page leaves it for
    // the page before.
    if !stopped && pos == floor && end.is_some() {
        take(floor, &carry, oversized, &mut from);
    }
    let end = end.unwrap_or(floor);
    // No whole record between `floor` and `top`: an empty page at `floor`.
    let from = if end == floor { floor } else { from };
    let rows: Vec<TranscriptRow> = kept.into_iter().rev().flatten().collect();
    Ok(Page { rows, from, end, next_before: (from > 0).then_some(from) })
}

fn parse(
    start: u64,
    line: &[u8],
    rows: &dyn Fn(u64, &serde_json::Value) -> Vec<TranscriptRow>,
) -> Vec<TranscriptRow> {
    if line.iter().all(u8::is_ascii_whitespace) {
        return Vec::new();
    }
    serde_json::from_slice::<serde_json::Value>(line)
        .map_or_else(|_| Vec::new(), |v| rows(start, &v))
}

/// The name a phone keys a transcript's pages by: the path's hash, so the
/// path itself never leaves the machine (the control surface carries none).
pub fn conversation(path: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(path.as_bytes());
    digest[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// Slash commands as Claude Code and Codex record them, `<command-name>/x
/// </command-name>…<command-args>y</command-args>`, as the person typed
/// them: `/x y`. `None` for words that are not one.
pub fn command_words(text: &str) -> Option<String> {
    let inner = |tag: &str| {
        let open = format!("<{tag}>");
        let close = format!("</{tag}>");
        let at = text.find(&open)? + open.len();
        let to = text[at..].find(&close)? + at;
        Some(text[at..to].trim())
    };
    let name = inner("command-name")?;
    let args = inner("command-args").unwrap_or("");
    Some(if args.is_empty() { name.to_string() } else { format!("{name} {args}") })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesimon_core::mesophon::RowKind;

    fn tmp(name: &str, body: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir()
            .join(format!("msmn-transcript-page-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("t.jsonl");
        std::fs::write(&p, body).unwrap();
        p
    }

    /// Every record with a `say` is one reply row.
    fn says(at: u64, v: &serde_json::Value) -> Vec<TranscriptRow> {
        v["say"]
            .as_str()
            .and_then(|t| TranscriptRow::new(at, RowKind::Reply, t, None))
            .into_iter()
            .collect()
    }

    fn body(n: usize, pad: usize) -> String {
        (0..n)
            .map(|i| {
                if i % 3 == 2 {
                    format!("{{\"noise\":\"{}\"}}\n", "x".repeat(pad))
                } else {
                    format!("{{\"say\":\"row {i} {}\"}}\n", "y".repeat(pad))
                }
            })
            .collect()
    }

    fn words(rows: &[TranscriptRow]) -> Vec<String> {
        rows.iter().map(|r| r.text.split(' ').take(2).collect::<Vec<_>>().join(" ")).collect()
    }

    #[test]
    fn the_tail_page_ends_at_the_last_whole_record() {
        let text = format!("{}{{\"say\":\"half writ", body(6, 0));
        let p = tmp("tail", &text);
        let page = page(&p, Ask::default(), &says).unwrap();
        assert_eq!(words(&page.rows), ["row 0", "row 1", "row 3", "row 4"]);
        assert_eq!(page.end as usize, text.rfind('\n').unwrap() + 1, "the half record waits");
        assert_eq!((page.from, page.next_before), (0, None));
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    /// Pages walked back from the tail, each from the last one's
    /// `next_before`, join into the whole file: no row lost, none twice,
    /// whatever the window cuts through.
    #[test]
    fn pages_join_without_a_gap_or_a_double() {
        for pad in [0, 300, 70 * 1024] {
            let text = body(60, pad);
            let p = tmp(&format!("join{pad}"), &text);
            let whole = page(&p, Ask::default(), &says);
            let mut all = Vec::new();
            let mut before = None;
            let mut pages = 0;
            loop {
                let pg = page(&p, Ask { before, after: None, limit: Some(7) }, &says).unwrap();
                assert!(pg.rows.len() <= 7);
                assert!(pg.rows.iter().all(|r| r.at >= pg.from && r.at < pg.end), "{pad}");
                if let Some(before) = before {
                    assert_eq!(pg.end, before, "a page ends where the next one began");
                }
                let mut rows = pg.rows;
                rows.append(&mut all);
                all = rows;
                pages += 1;
                match pg.next_before {
                    Some(b) => before = Some(b),
                    None => break,
                }
                assert!(pages < 100);
            }
            let expect: Vec<String> =
                (0..60).filter(|i| i % 3 != 2).map(|i| format!("row {i}")).collect();
            assert_eq!(words(&all), expect, "pad {pad}");
            if pad < 1000 {
                assert_eq!(words(&whole.unwrap().rows), expect);
            }
            std::fs::remove_dir_all(p.parent().unwrap()).ok();
        }
    }

    /// A page that would not fit the reply stops at a record's start, and
    /// the next page holds the record it left out.
    #[test]
    fn a_full_page_stops_at_a_record_boundary() {
        let text = body(30, 4 * 1024);
        let p = tmp("full", &text);
        let pg = page(&p, Ask::default(), &says).unwrap();
        let size: usize = pg.rows.iter().map(|r| serde_json::to_vec(r).unwrap().len()).sum();
        assert!(size <= TRANSCRIPT_PAGE_BYTES && pg.rows.len() >= 5, "{size}");
        let b = pg.next_before.expect("more before");
        assert!(text.as_bytes()[b as usize - 1] == b'\n', "a boundary");
        assert_eq!(pg.from, b);
        let prev = page(&p, Ask { before: Some(b), ..Ask::default() }, &says).unwrap();
        let last = prev.rows.last().unwrap();
        let first = &pg.rows[0];
        let n = |r: &TranscriptRow| r.text.split(' ').nth(1).unwrap().parse::<usize>().unwrap();
        let next = (n(last) + 1..).find(|i| i % 3 != 2).unwrap();
        assert_eq!(n(first), next, "the record left out opens the page before");
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    /// With `after`, only what was written since: a page that reaches
    /// `after` starts there, and a phone appends it to what it holds.
    #[test]
    fn after_reads_only_what_was_written_since() {
        let first = body(9, 10);
        let p = tmp("after", &first);
        let pg = page(&p, Ask::default(), &says).unwrap();
        let held = pg.end;
        assert_eq!(held as usize, first.len());
        let none = page(&p, Ask { after: Some(held), ..Ask::default() }, &says).unwrap();
        assert_eq!((none.rows.len(), none.from, none.end), (0, held, held));
        let more = "{\"say\":\"row 90\"}\n{\"noise\":1}\n{\"say\":\"row 91\"}\n";
        std::fs::write(&p, format!("{first}{more}")).unwrap();
        let new = page(&p, Ask { after: Some(held), ..Ask::default() }, &says).unwrap();
        assert_eq!(words(&new.rows), ["row 90", "row 91"]);
        assert_eq!(new.from, held, "contiguous with what the phone holds");
        assert_eq!(new.end as usize, first.len() + more.len());
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    /// A record too long to read whole is passed over, and the walk goes on.
    #[test]
    fn an_oversized_record_is_skipped() {
        let big = format!("{{\"say\":\"{}\"}}\n", "z".repeat(LINE_MAX + 10));
        let text = format!("{{\"say\":\"row 0\"}}\n{big}{{\"say\":\"row 2\"}}\n");
        let p = tmp("big", &text);
        let pg = page(&p, Ask::default(), &says).unwrap();
        assert_eq!(words(&pg.rows), ["row 0", "row 2"]);
        assert_eq!(pg.from, 0);
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    #[test]
    fn a_command_reads_as_typed() {
        assert_eq!(
            command_words(
                "<command-name>/effort</command-name>\n  <command-message>effort</command-message>\n  <command-args>xhigh</command-args>"
            )
            .as_deref(),
            Some("/effort xhigh")
        );
        assert_eq!(
            command_words("<command-name>/clear</command-name><command-args></command-args>")
                .as_deref(),
            Some("/clear")
        );
        assert_eq!(command_words("plain words"), None);
    }
}
