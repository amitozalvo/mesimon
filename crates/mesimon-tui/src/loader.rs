//! The reload screen (T-1102), "moving day": while the board waits for its
//! daemon, the shin carries a small board along a floor of stations, one
//! for each step of the wait, and sets it down when the board is ready.
//!
//! Every station is a step the client really takes. A `U` stops the old
//! daemon (`client::await_daemon_gone`), execs the new binary, dials or
//! spawns the daemon (`client::connect_or_spawn`) and reads the first
//! snapshot (`App::new`). Opening a board has only the last two. The
//! client reports each step through [`step`], a process-wide atomic, so
//! nothing is threaded through the connect's signatures.
//!
//! The screen is the board's own: its ground, its ramp, the shin's inks.
//! A `U` never leaves the alternate screen: the old process draws the
//! first station and execs, and the new one takes the same screen over
//! ([`RELOAD_FROM`] says it is already up), so the terminal's own ground
//! never shows between the two boards. Opening a board waits [`GRACE_MS`]
//! before drawing anything, so a quick connect shows only the ground.
//!
//! The state ([`Show`]) is a pure function of its clock and the steps
//! reported, which is what the tests drive. [`Loader`] runs it on a thread
//! that owns the terminal for the length of the wait, because the connect
//! blocks the main thread. Ctrl+C on that thread gives the terminal back
//! and exits, as it did when the wait was on the primary screen.

use std::io;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ratatui::backend::Backend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::Terminal;

use crate::creature::{self, Anim, Frame, Role, Size};
use crate::theme::{CreatureInk, Theme};

// -- what the client reports ------------------------------------------------------

/// What the wait is doing, as the person would say it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Step {
    /// A daemon is shutting down and holds the lock.
    Stop = 0,
    /// The new binary is starting.
    Switch = 1,
    /// Dialling a daemon that may already be listening.
    Dial = 2,
    /// No daemon answered, so one is being started.
    Spawn = 3,
    /// Connected: the first snapshot is on its way.
    Read = 4,
}

static STEP: AtomicU8 = AtomicU8::new(Step::Dial as u8);

/// Say what the wait is doing now. Cheap enough for a connect loop: a store
/// no one reads once the board is up.
pub(crate) fn step(s: Step) {
    STEP.store(s as u8, Ordering::Relaxed);
}

fn reported() -> Step {
    match STEP.load(Ordering::Relaxed) {
        0 => Step::Stop,
        1 => Step::Switch,
        3 => Step::Spawn,
        4 => Step::Read,
        _ => Step::Dial,
    }
}

/// The version a `U` came from, handed across the `exec`. Its presence also
/// says the alternate screen is already up, with the first station on it.
pub(crate) const RELOAD_FROM: &str = "MESIMON_RELOAD_FROM";

/// This binary's version: the station sign at the right end of a `U`.
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

/// [`RELOAD_FROM`], taken once and unset, so nothing this board starts
/// inherits it.
pub(crate) fn take_reload_from() -> Option<String> {
    let from = std::env::var_os(RELOAD_FROM)?;
    std::env::remove_var(RELOAD_FROM);
    Some(from.to_string_lossy().into_owned())
}

// -- the scene ----------------------------------------------------------------------

/// Which wait this is: a `U` (from the version it left) or a board opening.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Mode {
    Reload { from: String },
    Open,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Station {
    Old,
    Update,
    Daemon,
    Board,
}

impl Station {
    fn word(self) -> &'static str {
        match self {
            Station::Old => "old daemon",
            Station::Update => "update",
            Station::Daemon => "daemon",
            Station::Board => "board",
        }
    }
}

const RELOAD: [Station; 4] = [Station::Old, Station::Update, Station::Daemon, Station::Board];
const OPEN: [Station; 2] = [Station::Daemon, Station::Board];

/// How the wait ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum End {
    Ready,
    /// No daemon: the board opens anyway and keeps dialling.
    Failed,
}

/// A station's walk, in milliseconds.
const PACE_MS: f32 = 600.0;
/// A station's walk once the board is ready and the shin is not there yet:
/// on an idle machine a whole `U` is over in a tenth of a second, and the
/// walk is the scene (T-1102). Four stations cost at most 0.6 s.
const HURRY_MS: f32 = 200.0;
/// Opening a board draws nothing for this long: a quick connect is the
/// ground and then the board.
pub(crate) const GRACE_MS: u64 = 300;
/// The old daemon's shutdown counts its seconds after this.
const SLOW_MS: u64 = 2400;
/// The hop and the set-down, once the board is ready.
const READY_MS: u64 = 400;
/// Long enough to read why, before the board says it again.
const FAILED_MS: u64 = 1500;

/// The screen's state at its clock's `now`.
#[derive(Debug, Clone)]
pub(crate) struct Show {
    mode: Mode,
    step: Step,
    step_at: u64,
    /// The station it is walking to, never one behind it: a step reported
    /// twice (a build-skew restart after the dial) changes the words only.
    goal: usize,
    /// Where it is, in stations; below 0 is off the left edge.
    pos: f32,
    arrived_at: u64,
    end: Option<End>,
    /// When the last beat began: at the ending for a failure, on reaching
    /// the board for a ready one.
    finale: Option<u64>,
    shown_from: u64,
    now: u64,
}

impl Show {
    pub(crate) fn new(mode: Mode, step: Step) -> Show {
        let open = mode == Mode::Open;
        let mut s = Show {
            mode,
            step,
            step_at: 0,
            goal: 0,
            pos: 0.0,
            arrived_at: 0,
            end: None,
            finale: None,
            shown_from: if open { GRACE_MS } else { 0 },
            now: 0,
        };
        s.goal = s.station_of(step);
        // A board opening walks in from the left; a reload is already on
        // its way from the station before.
        s.pos = if open { -1.0 } else { s.goal.saturating_sub(1) as f32 };
        s
    }

    fn stations(&self) -> &'static [Station] {
        match self.mode {
            Mode::Reload { .. } => &RELOAD,
            Mode::Open => &OPEN,
        }
    }

    fn station_of(&self, step: Step) -> usize {
        let at = |want: Station| self.stations().iter().position(|&s| s == want).unwrap_or(0);
        match step {
            Step::Stop => at(Station::Old),
            Step::Switch => at(Station::Update),
            Step::Dial | Step::Spawn => at(Station::Daemon),
            Step::Read => at(Station::Board),
        }
    }

    /// Move the clock to `now`, with the step the client reports.
    pub(crate) fn advance(&mut self, now: u64, step: Step) {
        let dt = now.saturating_sub(self.now) as f32;
        self.now = now.max(self.now);
        if step != self.step {
            self.step = step;
            self.step_at = self.now;
            self.goal = self.goal.max(self.station_of(step));
        }
        if self.walking() {
            let pace = if self.end == Some(End::Ready) { HURRY_MS } else { PACE_MS };
            self.pos = (self.pos + dt / pace).min(self.goal as f32);
            if !self.walking() {
                self.arrived_at = self.now;
            }
        }
        if self.end == Some(End::Ready) && self.finale.is_none() && !self.walking() {
            self.finale = Some(self.now);
        }
    }

    /// The wait is over. Ready, the shin hurries on to the board and sets
    /// it down there; failed, it stops where it is. Unseen, it simply ends.
    pub(crate) fn finish(&mut self, end: End) {
        if self.end.is_some() {
            return;
        }
        self.end = Some(end);
        if end == End::Ready {
            self.goal = self.stations().len() - 1;
        }
        if end == End::Failed || !self.visible() || !self.walking() {
            self.finale = Some(self.now);
        }
    }

    fn walking(&self) -> bool {
        self.finale.is_none() && self.end != Some(End::Failed) && self.pos < self.goal as f32
    }

    pub(crate) fn visible(&self) -> bool {
        self.now >= self.shown_from
    }

    /// Done: ended, and its last beat played if anyone saw the scene.
    pub(crate) fn over(&self) -> bool {
        let (Some(end), Some(at)) = (self.end, self.finale) else { return false };
        if at < self.shown_from {
            return true;
        }
        self.now - at >= if end == End::Ready { READY_MS } else { FAILED_MS }
    }

    /// The shin's pose, and its bob in pixels.
    fn pose(&self) -> (Frame, i16) {
        if let (Some(end), Some(at)) = (self.end, self.finale) {
            let anim = if end == End::Ready { Anim::Done } else { Anim::Failed };
            return (*creature::frame_at(anim, self.now - at), 0);
        }
        if self.walking() {
            let step = if self.end.is_some() { 80 } else { 150 };
            return (creature::WALK, ((self.now / step) % 2) as i16);
        }
        let waited = self.now - self.step_at;
        let since = self.now - self.arrived_at.max(self.step_at);
        let anim = match (self.step, &self.mode) {
            (Step::Stop, _) if waited >= SLOW_MS => {
                return (*creature::frame_at(Anim::Throttled, waited - SLOW_MS), 0);
            }
            (Step::Stop, Mode::Reload { .. }) => Anim::Sleeping,
            (Step::Stop, Mode::Open) | (Step::Read, _) => Anim::Monitoring,
            (Step::Switch, _) => Anim::Spawning,
            (Step::Dial | Step::Spawn, _) => Anim::Thinking,
        };
        (*creature::frame_at(anim, since), 0)
    }

    /// The sentence: the words, and a count of seconds when it is slow.
    fn sentence(&self) -> (String, Option<String>, Mark) {
        match self.end {
            Some(End::Ready) => return ("ready".into(), None, Mark::Done),
            Some(End::Failed) => {
                return ("the daemon did not start".into(), None, Mark::Failed);
            }
            None => {}
        }
        let waited = self.now - self.step_at;
        let reload = matches!(self.mode, Mode::Reload { .. });
        let words = match self.step {
            Step::Stop if waited >= SLOW_MS => "the old daemon is still finishing up".to_string(),
            Step::Stop if reload => "stopping the old daemon".into(),
            Step::Stop => "waiting for the last daemon to finish".into(),
            Step::Switch => match &self.mode {
                Mode::Reload { from } if from != VERSION => format!("switching to {VERSION}"),
                _ => "starting the new build".into(),
            },
            Step::Dial => "connecting to the daemon".into(),
            Step::Spawn => "starting the daemon".into(),
            Step::Read => "reading the board".into(),
        };
        let dots = [".  ", ".. ", "..."][((self.now / 400) % 3) as usize];
        let count = (self.step == Step::Stop && waited >= SLOW_MS)
            .then(|| format!("  {} s", waited / 1000));
        (format!("{words}{dots}"), count, Mark::Now)
    }

    fn subline(&self) -> Option<&'static str> {
        match (self.end, &self.mode) {
            (Some(End::Failed), _) => Some("opening the board anyway ∙ it keeps trying every 2 s"),
            (_, Mode::Reload { .. }) => Some("your agents keep running"),
            _ => None,
        }
    }

    fn mark(&self, j: usize) -> Mark {
        match (self.end, self.finale) {
            (Some(End::Ready), Some(_)) => Mark::Done,
            (Some(End::Failed), _) if j == self.goal => Mark::Failed,
            _ if j < self.goal => Mark::Done,
            _ if j == self.goal => Mark::Now,
            _ => Mark::Todo,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mark {
    Done,
    Now,
    Todo,
    Failed,
}

// -- drawing ---------------------------------------------------------------------

/// The theme's colours the screen draws in, copied out so the thread that
/// draws holds no `Theme`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Inks {
    bg: Option<Color>,
    base: Color,
    dim2: Color,
    dim3: Color,
    calm: Color,
    err: Color,
    block_gaps: bool,
    /// None on a mono profile: the words, and no picture.
    shin: Option<CreatureInk>,
}

impl Inks {
    pub(crate) fn of(theme: &Theme) -> Inks {
        Inks {
            bg: theme.bg,
            base: theme.rest.base,
            dim2: theme.rest.dim2,
            dim3: theme.rest.dim3,
            calm: theme.calm,
            err: theme.err,
            block_gaps: theme.block_gaps,
            shin: theme.creature_ink(false),
        }
    }

    fn mark(&self, m: Mark) -> Style {
        Style::default().fg(match m {
            Mark::Done => self.calm,
            Mark::Now => self.base,
            Mark::Todo => self.dim3,
            Mark::Failed => self.err,
        })
    }
}

/// Rows the scene takes: the sentence and its subline, a blank, the
/// board's four rows over the shin's eight, the floor, the station signs,
/// a blank and the version signs.
const BLOCK: u16 = 19;
const SHIN_ROW: i16 = 7;
const FLOOR_ROW: u16 = 15;
const SIGN_ROW: u16 = 16;
const VERSION_ROW: u16 = 18;
/// How far in from either edge the first and last stations stand.
const MARGIN: i16 = 12;
/// The board it carries, in pixels: a frame, a dark screen, three columns
/// of cards.
const BOARD: [&str; 7] = [
    "###########",
    "#.........#",
    "#.11.22.33#",
    "#.11.22...#",
    "#.11......#",
    "#.........#",
    "###########",
];

fn board_role(ch: u8) -> Option<Role> {
    match ch {
        b'#' => Some(Role::Bezel),
        b'.' => Some(Role::Screen),
        b'1' => Some(Role::Add),
        b'2' => Some(Role::Drop),
        b'3' => Some(Role::Blush),
        _ => None,
    }
}

/// Pixels, one column and half a row each.
struct Canvas {
    w: i16,
    h: i16,
    px: Vec<Option<Color>>,
}

impl Canvas {
    fn put(&mut self, x: i16, y: i16, c: Color) {
        if x >= 0 && y >= 0 && x < self.w && y < self.h {
            self.px[(y * self.w + x) as usize] = Some(c);
        }
    }

    fn get(&self, x: i16, y: i16) -> Option<Color> {
        (x >= 0 && y >= 0 && x < self.w && y < self.h)
            .then(|| self.px[(y * self.w + x) as usize])
            .flatten()
    }
}

/// Draw the scene into `area`: the ground always, the rest once visible.
pub(crate) fn render(buf: &mut Buffer, area: Rect, show: &Show, inks: &Inks) {
    if let Some(bg) = inks.bg {
        buf.set_style(area, Style::default().bg(bg));
    }
    if !show.visible() || area.width == 0 || area.height == 0 {
        return;
    }
    let top = area.y + area.height.saturating_sub(BLOCK) / 2;
    let w = area.width as i16;
    let stations = show.stations();
    let n = stations.len();
    let span = (w - 2 * MARGIN).max(0) as f32;
    let col = |j: f32| -> i16 {
        if n < 2 {
            return w / 2;
        }
        MARGIN + (j * span / (n - 1) as f32).round() as i16
    };
    let text = |buf: &mut Buffer, x: i16, row: u16, s: &str, style: Style| {
        let y = top + row;
        if y >= area.bottom() {
            return;
        }
        // Clipped at both edges, a character at a time.
        for (i, ch) in s.chars().enumerate() {
            let c = x + i as i16;
            if c >= 0 && c < w {
                buf[Position::new(area.x + c as u16, y)].set_char(ch).set_style(style);
            }
        }
    };
    let centre = |s: &str| (w - s.chars().count() as i16) / 2;

    // The picture: the shin, and the board it carries.
    if let Some(ink) = inks.shin {
        let (fr, bob) = show.pose();
        let pic = creature::picture(Size::Medium, &fr);
        let x0 = col(show.pos) - 10;
        let y0 = SHIN_ROW * 2 - bob;
        let mut cv =
            Canvas { w, h: BLOCK as i16 * 2, px: vec![None; w as usize * BLOCK as usize * 2] };
        for y in 0..pic.h {
            for x in 0..pic.w {
                if let Some(role) = pic.at(x, y) {
                    cv.put(x0 + x, y0 + y, role.ink(&ink));
                }
            }
        }
        let carried = (f32::from(x0 + 4), f32::from(y0 - 7));
        let (bx, by) = match (show.end, show.finale) {
            (Some(end), Some(at)) => {
                // Up and over the arms first, then down to the floor: set
                // down to the left once ready (where the walk came from, and
                // the last station stands too near the right edge), dropped
                // to the right on a failure (the daemon is never the last).
                let floor = f32::from(FLOOR_ROW as i16 * 2 - 7);
                let (lands, ms) = if end == End::Ready { (-13.0, 300.0) } else { (22.0, 240.0) };
                let q = ((show.now - at) as f32 / ms).min(1.0);
                let across = (q * 2.0).min(1.0);
                let down = (q * 2.0 - 1.0).max(0.0);
                let lift = (across * std::f32::consts::PI).sin() * 3.0 * (1.0 - down);
                (
                    carried.0 + (f32::from(x0) + lands - carried.0) * across,
                    carried.1 + (floor - carried.1) * down - lift,
                )
            }
            _ => carried,
        };
        let (bx, by) = (bx.round() as i16, by.round() as i16);
        for (dy, row) in BOARD.iter().enumerate() {
            for (dx, ch) in row.bytes().enumerate() {
                if let Some(role) = board_role(ch) {
                    cv.put(bx + dx as i16, by + dy as i16, role.ink(&ink));
                }
            }
        }
        for r in 0..BLOCK as i16 {
            for c in 0..w {
                let (t, b) = (cv.get(c, 2 * r), cv.get(c, 2 * r + 1));
                if let Some((ch, style)) = creature::pixel_cell(t, b, &ink, inks.block_gaps) {
                    let at = Position::new(area.x + c as u16, top + r as u16);
                    if buf.area.contains(at) {
                        buf[at].set_char(ch).set_style(style);
                    }
                }
            }
        }
        // Marks stand in cells no pixel touches.
        let body_row = y0.div_euclid(2);
        for (mc, mr, ch, role, bold) in pic.marks() {
            let (c, r) = (x0 + mc, body_row + mr);
            if c < 0 || r < 0 || c >= w || r >= BLOCK as i16 {
                continue;
            }
            if cv.get(c, 2 * r).is_some() || cv.get(c, 2 * r + 1).is_some() {
                continue;
            }
            let style = Style::default().fg(role.ink(&ink));
            let style = if bold { style.add_modifier(Modifier::BOLD) } else { style };
            text(buf, c, r as u16, &ch.to_string(), style);
        }
    }

    // The words.
    let (words, count, mark) = show.sentence();
    let len = words.chars().count() + count.as_deref().map_or(0, |c| c.chars().count());
    let x = (w - len as i16) / 2;
    text(buf, x, 0, &words, inks.mark(mark));
    if let Some(count) = count {
        text(buf, x + words.chars().count() as i16, 0, &count, Style::default().fg(inks.dim2));
    }
    if let Some(sub) = show.subline() {
        let ink = if show.end == Some(End::Failed) { inks.dim2 } else { inks.dim3 };
        text(buf, centre(sub), 1, sub, Style::default().fg(ink));
    }

    // The floor and its stations.
    text(buf, 0, FLOOR_ROW, &"─".repeat(area.width as usize), Style::default().fg(inks.dim3));
    for (j, station) in stations.iter().enumerate() {
        let m = show.mark(j);
        let c = col(j as f32);
        let tick = if m == Mark::Todo { inks.dim3 } else { inks.dim2 };
        text(buf, c, FLOOR_ROW, "┴", Style::default().fg(tick));
        let sign = match m {
            Mark::Done => format!("✓ {}", station.word()),
            Mark::Failed => format!("✗ {}", station.word()),
            _ => station.word().to_string(),
        };
        text(buf, c - sign.chars().count() as i16 / 2, SIGN_ROW, &sign, inks.mark(m));
    }
    if let Mode::Reload { from } = &show.mode {
        if from != VERSION {
            text(buf, 1, VERSION_ROW, from, Style::default().fg(inks.dim3));
            let x = w - 1 - VERSION.chars().count() as i16;
            text(buf, x, VERSION_ROW, VERSION, Style::default().fg(inks.dim2));
        }
    }
}

// -- the thread ------------------------------------------------------------------

#[derive(Default)]
struct Ctl {
    end: Option<End>,
    abandon: bool,
}

/// The screen, playing on a thread of its own that holds the terminal until
/// the wait ends.
pub(crate) struct Loader<B: Backend + Send + 'static> {
    ctl: Arc<Mutex<Ctl>>,
    handle: Option<JoinHandle<io::Result<Terminal<B>>>>,
    restore: fn(),
}

impl<B> Loader<B>
where
    B: Backend<Error = io::Error> + Send + 'static,
{
    /// Start playing on `term`. Set the first [`step`] before this.
    /// `restore` gives the terminal back: on Ctrl+C, and on any way out but
    /// [`Loader::finish`] and [`Loader::stop`].
    pub(crate) fn start(mut term: Terminal<B>, inks: Inks, mode: Mode, restore: fn()) -> Self {
        let ctl = Arc::new(Mutex::new(Ctl::default()));
        let theirs = Arc::clone(&ctl);
        let handle = std::thread::spawn(move || {
            let start = Instant::now();
            let mut show = Show::new(mode, reported());
            loop {
                let now = start.elapsed().as_millis() as u64;
                let (end, abandon) = {
                    let c = theirs.lock().unwrap_or_else(|e| e.into_inner());
                    (c.end, c.abandon)
                };
                if abandon {
                    return Ok(term);
                }
                show.advance(now, reported());
                if let Some(end) = end {
                    show.finish(end);
                }
                if show.over() {
                    return Ok(term);
                }
                term.draw(|f| {
                    let area = f.area();
                    render(f.buffer_mut(), area, &show, &inks);
                })?;
                // Raw mode makes Ctrl+C a key: the one key read here, the
                // way out it was before the wait moved onto this screen.
                if event::poll(Duration::from_millis(40))? {
                    if let Event::Key(k) = event::read()? {
                        let ctrl_c = k.code == KeyCode::Char('c')
                            && k.modifiers.contains(KeyModifiers::CONTROL);
                        if k.kind == KeyEventKind::Press && ctrl_c {
                            restore();
                            std::process::exit(130);
                        }
                    }
                }
            }
        });
        Loader { ctl, handle: Some(handle), restore }
    }

    /// The wait is over: play its last beat, then hand the terminal back.
    pub(crate) fn finish(mut self, end: End) -> io::Result<Terminal<B>> {
        self.ctl.lock().unwrap_or_else(|e| e.into_inner()).end = Some(end);
        self.join()
    }

    /// Stop at once (the `exec`, whose new image carries on).
    pub(crate) fn stop(mut self) -> io::Result<Terminal<B>> {
        self.ctl.lock().unwrap_or_else(|e| e.into_inner()).abandon = true;
        self.join()
    }

    fn join(&mut self) -> io::Result<Terminal<B>> {
        let handle = self.handle.take().expect("joined once");
        let out = handle
            .join()
            .unwrap_or_else(|_| Err(io::Error::other("the reload screen's thread panicked")));
        if out.is_err() {
            (self.restore)();
        }
        out
    }
}

impl<B: Backend + Send + 'static> Drop for Loader<B> {
    /// An error between start and finish (the connect's `?`): the thread
    /// stops and the terminal goes back, so the error lands on the shell.
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            self.ctl.lock().unwrap_or_else(|e| e.into_inner()).abandon = true;
            let _ = handle.join();
            (self.restore)();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Flavor, Profile};

    fn inks(profile: Profile) -> Inks {
        Inks::of(&Theme::new(Flavor::Graphite, profile))
    }

    fn draw(show: &Show, w: u16, h: u16, inks: &Inks) -> Buffer {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        render(&mut buf, area, show, inks);
        buf
    }

    fn rows(buf: &Buffer) -> Vec<String> {
        let a = buf.area;
        (0..a.height)
            .map(|y| (0..a.width).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect()
    }

    fn reload(from: &str) -> Mode {
        Mode::Reload { from: from.into() }
    }

    /// Advance in 50 ms ticks, the thread's pace or near it.
    fn run(show: &mut Show, until: u64, step: Step) {
        let mut t = show.now;
        while t < until {
            t = (t + 50).min(until);
            show.advance(t, step);
        }
    }

    #[test]
    fn the_shin_walks_station_to_station_and_never_back() {
        let mut show = Show::new(reload("0.0.1"), Step::Stop);
        assert_eq!(show.pos, 0.0, "a reload starts on the old daemon's station");
        let mut last = show.pos;
        let mut t = 0;
        for (step, ms) in
            [(Step::Stop, 800), (Step::Switch, 900), (Step::Spawn, 900), (Step::Read, 900)]
        {
            t += ms;
            run(&mut show, t, step);
            assert!(show.pos >= last, "{step:?}: walked back from {last} to {}", show.pos);
            last = show.pos;
        }
        assert_eq!(show.pos, 3.0, "every step reported, it stands at the board");
        // A build-skew restart reports the stop again: words change, feet do not.
        run(&mut show, t + 600, Step::Stop);
        assert_eq!(show.pos, 3.0);
        assert!(show.sentence().0.starts_with("stopping the old daemon"));
    }

    #[test]
    fn a_new_image_picks_the_walk_up_where_the_old_one_left_it() {
        // The old process drew the first station; the new one starts at it,
        // already on its way to the update.
        let show = Show::new(reload("0.0.1"), Step::Switch);
        assert_eq!((show.pos, show.goal), (0.0, 1));
        assert!(show.visible(), "a reload shows at once: the screen was already up");
    }

    #[test]
    fn every_moment_draws_inside_the_smallest_board_and_never_in_attn() {
        let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
        let ink = inks(Profile::TrueColor);
        let moments: Vec<(Mode, Step, u64, Option<End>)> = vec![
            (reload("0.0.1"), Step::Stop, 500, None),
            (reload("0.0.1"), Step::Stop, 4000, None),
            (reload("0.0.1"), Step::Switch, 900, None),
            (reload(VERSION), Step::Spawn, 1500, None),
            (reload("0.0.1"), Step::Read, 2000, Some(End::Ready)),
            (Mode::Open, Step::Dial, 1000, None),
            (Mode::Open, Step::Spawn, 2500, Some(End::Failed)),
        ];
        for (mode, step, ms, end) in moments {
            for (w, h) in [(60, 20), (120, 40)] {
                let mut show = Show::new(mode.clone(), step);
                run(&mut show, ms, step);
                if let Some(end) = end {
                    show.finish(end);
                    run(&mut show, ms + 200, step);
                }
                let buf = draw(&show, w, h, &ink);
                let text = rows(&buf).join("\n");
                let words = show.sentence().0;
                let words = words.trim_end_matches(['.', ' ']);
                assert!(
                    text.contains(words),
                    "{mode:?} {step:?} at {w}x{h}: no {words:?} in\n{text}"
                );
                assert!(text.contains('┴'), "the floor is drawn");
                for cell in buf.content() {
                    assert_ne!(cell.fg, theme.attn, "{mode:?} {step:?}: attn is needs-you's alone");
                    assert_ne!(cell.bg, theme.attn, "{mode:?} {step:?}: attn is needs-you's alone");
                }
            }
        }
    }

    #[test]
    fn the_screen_wears_the_boards_ground_from_its_first_frame() {
        let ink = inks(Profile::TrueColor);
        let show = Show::new(Mode::Open, Step::Dial);
        let buf = draw(&show, 60, 20, &ink);
        assert!(
            buf.content().iter().all(|c| Some(c.bg) == ink.bg),
            "every cell on the theme's ground"
        );
    }

    #[test]
    fn a_quick_open_shows_only_the_ground_and_ends_at_once() {
        let ink = inks(Profile::TrueColor);
        let mut show = Show::new(Mode::Open, Step::Dial);
        run(&mut show, GRACE_MS - 100, Step::Read);
        let buf = draw(&show, 80, 24, &ink);
        assert!(buf.content().iter().all(|c| c.symbol() == " "), "nothing drawn inside the grace");
        show.finish(End::Ready);
        assert!(show.over(), "an unseen scene plays no finale");
    }

    #[test]
    fn a_seen_scene_sets_the_board_down_before_it_ends() {
        // Two stations' walk, in from the edge and on to the board, and then some.
        let mut show = Show::new(Mode::Open, Step::Dial);
        run(&mut show, 1500, Step::Read);
        show.finish(End::Ready);
        assert!(!show.over());
        run(&mut show, 1500 + READY_MS - 50, Step::Read);
        assert!(!show.over());
        run(&mut show, 1500 + READY_MS, Step::Read);
        assert!(show.over());
    }

    #[test]
    fn a_quick_reload_hurries_to_the_board_before_setting_it_down() {
        // An idle machine's `U`: ready before the first station's walk ends.
        let mut show = Show::new(reload("0.0.1"), Step::Switch);
        run(&mut show, 100, Step::Read);
        show.finish(End::Ready);
        assert!(!show.over(), "the walk is the scene: it is not cut");
        run(&mut show, 100 + 3 * HURRY_MS as u64, Step::Read);
        assert_eq!(show.pos, 3.0, "at the board within a hurried walk");
        let arrived = show.finale.expect("the set-down began on arrival");
        run(&mut show, arrived + READY_MS - 50, Step::Read);
        assert!(!show.over());
        run(&mut show, arrived + READY_MS, Step::Read);
        assert!(show.over());
    }

    #[test]
    fn a_failed_start_says_so_and_marks_the_daemon() {
        let ink = inks(Profile::TrueColor);
        let mut show = Show::new(Mode::Open, Step::Dial);
        run(&mut show, 1000, Step::Spawn);
        show.finish(End::Failed);
        let text = rows(&draw(&show, 80, 24, &ink)).join("\n");
        assert!(text.contains("the daemon did not start"), "{text}");
        assert!(text.contains("✗ daemon"), "{text}");
        assert!(text.contains("it keeps trying every 2 s"), "{text}");
    }

    #[test]
    fn a_slow_handover_counts_its_wait() {
        let ink = inks(Profile::TrueColor);
        let mut show = Show::new(reload("0.0.1"), Step::Stop);
        run(&mut show, 5300, Step::Stop);
        let text = rows(&draw(&show, 80, 24, &ink)).join("\n");
        assert!(text.contains("the old daemon is still finishing up"), "{text}");
        assert!(text.contains("  5 s"), "{text}");
        assert!(text.contains("your agents keep running"), "{text}");
    }

    #[test]
    fn the_version_signs_stand_only_across_a_change() {
        let ink = inks(Profile::TrueColor);
        let changed =
            rows(&draw(&Show::new(reload("0.0.1"), Step::Switch), 80, 24, &ink)).join("\n");
        assert!(changed.contains("0.0.1") && changed.contains(VERSION), "{changed}");
        assert!(changed.contains(&format!("switching to {VERSION}")), "{changed}");
        let same = rows(&draw(&Show::new(reload(VERSION), Step::Switch), 80, 24, &ink)).join("\n");
        assert!(!same.contains(VERSION), "a rebuild of one version names none:\n{same}");
        assert!(same.contains("starting the new build"), "{same}");
    }

    #[test]
    fn mono_says_the_words_without_a_picture() {
        let ink = inks(Profile::Mono);
        let mut show = Show::new(reload("0.0.1"), Step::Stop);
        run(&mut show, 800, Step::Spawn);
        let text = rows(&draw(&show, 80, 24, &ink)).join("\n");
        assert!(text.contains("starting the daemon"), "{text}");
        assert!(!text.contains(['▀', '▄', '█']), "mono draws no pixels:\n{text}");
    }

    #[test]
    fn the_carried_board_comes_down_beside_the_shin() {
        let ink = inks(Profile::TrueColor);
        let mut show = Show::new(Mode::Open, Step::Read);
        run(&mut show, 2000, Step::Read);
        let carried = draw(&show, 80, 24, &ink);
        show.finish(End::Ready);
        run(&mut show, 2300, Step::Read);
        let landed = draw(&show, 80, 24, &ink);
        // The board's screen ink: above the shin while carried, then on the
        // floor beside it.
        let screen = ink.shin.unwrap().screen;
        let lowest = |buf: &Buffer| {
            buf.content()
                .iter()
                .enumerate()
                .filter(|(_, c)| c.fg == screen || c.bg == screen)
                .map(|(i, _)| i as u16 / 80)
                .max()
                .expect("the board is drawn")
        };
        assert!(lowest(&landed) > lowest(&carried), "the board came down");
    }
}
