//! The reload screen (T-1102, redrawn as a dialog by T-1120): while the
//! board waits for its daemon, a compact dialog lists the steps of the
//! wait, ticks each one off, and sweeps a bar so it plainly is working.
//!
//! Every row is a step the client really takes. A `U` stops the old daemon
//! (`client::await_daemon_gone`), execs the new binary, dials or spawns the
//! daemon (`client::connect_or_spawn`) and reads the first snapshot
//! (`App::new`). Opening a board has only the last two. The client reports
//! each step through [`step`], a process-wide atomic, so nothing is
//! threaded through the connect's signatures.
//!
//! Behind the dialog ([`Behind`]): on a `U`, the board the person was
//! looking at, muted to one faint ink so it does not compete; on an open,
//! where there is no board yet, the theme's ground. A `U` never leaves the
//! alternate screen. The old process mutes its last frame and draws the
//! dialog over it, then execs; the new one ([`RELOAD_FROM`] says the screen
//! is up) draws the dialog's cells only, so the muted board it cannot know
//! stays on the terminal around it. Opening a board waits [`GRACE_MS`]
//! before drawing anything, so a quick connect shows only the ground. Once
//! the board is up, [`Veil`] lifts the mute (or the ground) in three frames.
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
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::Terminal;

use crate::glyphs::{self, Tier};
use crate::theme::{Profile, Theme};

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

/// One row of the dialog: a step of the wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Old,
    Update,
    Daemon,
    Board,
}

const RELOAD: [Row; 4] = [Row::Old, Row::Update, Row::Daemon, Row::Board];
const OPEN: [Row; 2] = [Row::Daemon, Row::Board];

/// How the wait ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum End {
    Ready,
    /// No daemon: the board opens anyway and keeps dialling.
    Failed,
}

/// Opening a board draws nothing for this long: a quick connect is the
/// ground and then the board.
pub(crate) const GRACE_MS: u64 = 300;
/// The old daemon's shutdown counts its seconds after this.
const SLOW_MS: u64 = 2400;
/// Every row ticked, before the board: long enough to see the last tick land.
const READY_MS: u64 = 400;
/// Long enough to read why, before the board says it again.
const FAILED_MS: u64 = 1500;
/// The bar's bright run crosses the track once in this long, and back.
const SWEEP_MS: u64 = 1700;

/// The screen's state at its clock's `now`.
#[derive(Debug, Clone)]
pub(crate) struct Show {
    mode: Mode,
    step: Step,
    step_at: u64,
    /// The row in progress, never one behind it: a step reported twice (a
    /// build-skew restart after the dial) changes the words only.
    goal: usize,
    /// The daemon was started, not found listening.
    spawned: bool,
    end: Option<(End, u64)>,
    shown_from: u64,
    now: u64,
}

impl Show {
    pub(crate) fn new(mode: Mode, step: Step) -> Show {
        let shown_from = if mode == Mode::Open { GRACE_MS } else { 0 };
        let mut s = Show {
            mode,
            step,
            step_at: 0,
            goal: 0,
            spawned: step == Step::Spawn,
            end: None,
            shown_from,
            now: 0,
        };
        s.goal = s.row_of(step);
        s
    }

    fn rows(&self) -> &'static [Row] {
        match self.mode {
            Mode::Reload { .. } => &RELOAD,
            Mode::Open => &OPEN,
        }
    }

    fn row_of(&self, step: Step) -> usize {
        let at = |want: Row| self.rows().iter().position(|&r| r == want).unwrap_or(0);
        match step {
            Step::Stop => at(Row::Old),
            Step::Switch => at(Row::Update),
            Step::Dial | Step::Spawn => at(Row::Daemon),
            Step::Read => at(Row::Board),
        }
    }

    /// Move the clock to `now`, with the step the client reports.
    pub(crate) fn advance(&mut self, now: u64, step: Step) {
        self.now = now.max(self.now);
        if step != self.step {
            self.step = step;
            self.step_at = self.now;
            self.goal = self.goal.max(self.row_of(step));
        }
        self.spawned |= step == Step::Spawn;
    }

    /// The wait is over. Seen, its last beat plays; unseen, it simply ends.
    pub(crate) fn finish(&mut self, end: End) {
        if self.end.is_none() {
            self.end = Some((end, self.now));
        }
    }

    pub(crate) fn visible(&self) -> bool {
        self.now >= self.shown_from
    }

    /// Something was drawn: the board fades in, rather than appearing.
    pub(crate) fn seen(&self) -> bool {
        match self.end {
            Some((_, at)) => at >= self.shown_from,
            None => self.visible(),
        }
    }

    /// Done: ended, and its last beat played if anyone saw the scene.
    pub(crate) fn over(&self) -> bool {
        let Some((end, at)) = self.end else { return false };
        if at < self.shown_from {
            return true;
        }
        self.now - at >= if end == End::Ready { READY_MS } else { FAILED_MS }
    }

    fn ready(&self) -> bool {
        matches!(self.end, Some((End::Ready, _)))
    }

    fn failed(&self) -> bool {
        matches!(self.end, Some((End::Failed, _)))
    }

    /// A `U` that changes the version names both ends of it.
    fn changed(&self) -> bool {
        matches!(&self.mode, Mode::Reload { from } if from != VERSION)
    }

    /// The word set into the top edge, and what follows it.
    fn title(&self) -> (&'static str, String) {
        match &self.mode {
            Mode::Reload { from } if from != VERSION => ("UPDATE", format!("{from} → {VERSION}")),
            Mode::Reload { .. } => ("UPDATE", VERSION.to_string()),
            Mode::Open => ("OPENING THE BOARD", String::new()),
        }
    }

    fn mark(&self, j: usize) -> Mark {
        match self.end {
            Some((End::Ready, _)) => Mark::Done,
            Some((End::Failed, _)) if j == self.goal => Mark::Failed,
            _ if j < self.goal => Mark::Done,
            _ if j == self.goal => Mark::Now,
            _ => Mark::Todo,
        }
    }

    /// Row `j`'s words, and a count of seconds when its step is slow.
    fn words(&self, j: usize) -> (String, Option<String>) {
        let row = self.rows()[j];
        match self.mark(j) {
            Mark::Failed => ("the daemon did not start".into(), None),
            Mark::Done => {
                let w = match row {
                    Row::Old => "old daemon stopped".to_string(),
                    Row::Update if self.changed() => format!("{VERSION} started"),
                    Row::Update => "new build started".into(),
                    Row::Daemon if self.spawned => "daemon started".into(),
                    Row::Daemon => "daemon connected".into(),
                    Row::Board => "board read".into(),
                };
                (w, None)
            }
            Mark::Todo => {
                let w = match row {
                    Row::Old => "stopping the old daemon".to_string(),
                    Row::Update if self.changed() => format!("switching to {VERSION}"),
                    Row::Update => "starting the new build".into(),
                    Row::Daemon => "connecting to the daemon".into(),
                    Row::Board => "reading the board".into(),
                };
                (w, None)
            }
            Mark::Now => self.sentence(),
        }
    }

    /// The step in progress, as the person would say it.
    fn sentence(&self) -> (String, Option<String>) {
        let waited = self.now - self.step_at;
        let slow = self.step == Step::Stop && waited >= SLOW_MS;
        let reload = matches!(self.mode, Mode::Reload { .. });
        let words = match self.step {
            Step::Stop if slow => "the old daemon is still finishing up".to_string(),
            Step::Stop if reload => "stopping the old daemon".into(),
            Step::Stop => "waiting for the last daemon to finish".into(),
            Step::Switch if self.changed() => format!("switching to {VERSION}"),
            Step::Switch => "starting the new build".into(),
            Step::Dial => "connecting to the daemon".into(),
            Step::Spawn => "starting the daemon".into(),
            Step::Read => "reading the board".into(),
        };
        let dots = [".  ", ".. ", "..."][((self.now / 400) % 3) as usize];
        (format!("{words}{dots}"), slow.then(|| format!("  {} s", waited / 1000)))
    }

    fn sub(&self) -> &'static [&'static str] {
        match (self.failed(), &self.mode) {
            (true, _) => &["opening the board anyway", "it keeps trying every 2 s"],
            // Two rows on a `U`, failed or not, so the dialog the new image
            // draws covers every cell the old image's did.
            (false, Mode::Reload { .. }) => &["your agents keep running", ""],
            (false, Mode::Open) => &[],
        }
    }

    /// The dialog's rows, top to bottom inside its frame.
    fn lines(&self) -> Vec<Line> {
        let mut v = vec![Line::Blank];
        v.extend((0..self.rows().len()).map(Line::Step));
        v.extend([Line::Blank, Line::Bar, Line::Blank]);
        let sub = self.sub();
        v.extend(sub.iter().map(|s| Line::Sub(s)));
        if sub.last().is_some_and(|s| !s.is_empty()) {
            v.push(Line::Blank);
        }
        v
    }

    /// Where the bar's bright run is centred, 0 to 1 along the track: there
    /// and back again, easing at either end.
    fn sweep(&self) -> f32 {
        let p = (self.now % (2 * SWEEP_MS)) as f32 / SWEEP_MS as f32;
        let u = if p < 1.0 { p } else { 2.0 - p };
        u * u * (3.0 - 2.0 * u)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mark {
    Done,
    Now,
    Todo,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Line {
    Blank,
    Step(usize),
    Bar,
    Sub(&'static str),
}

// -- drawing ---------------------------------------------------------------------

/// The theme's colours the screen draws in, copied out so the thread that
/// draws holds no `Theme`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Inks {
    bg: Option<Color>,
    base: Color,
    dim1: Color,
    dim2: Color,
    dim3: Color,
    calm: Color,
    err: Color,
    tier: Tier,
    /// No colour to mute with: the board does not stay behind the dialog.
    mono: bool,
}

impl Inks {
    pub(crate) fn of(theme: &Theme) -> Inks {
        Inks {
            bg: theme.bg,
            base: theme.rest.base,
            dim1: theme.rest.dim1,
            dim2: theme.rest.dim2,
            dim3: theme.rest.dim3,
            calm: theme.calm,
            err: theme.err,
            tier: theme.glyph_tier(),
            mono: theme.profile == Profile::Mono,
        }
    }

    fn mark(&self, m: Mark) -> (char, Style, Style) {
        let ascii = self.tier == Tier::Ascii;
        let (ch, mark, words) = match m {
            Mark::Done => (if ascii { '+' } else { '✓' }, self.calm, self.dim2),
            Mark::Failed => (if ascii { 'x' } else { '✗' }, self.err, self.err),
            Mark::Todo => (if ascii { '-' } else { '·' }, self.dim3, self.dim3),
            // The spinner is drawn in its place.
            Mark::Now => (' ', self.base, self.base),
        };
        (ch, Style::default().fg(mark), Style::default().fg(words))
    }

    /// The one ink a muted board's text wears: its faintest step, a little
    /// further toward the ground where the colours can be mixed.
    fn muted(&self) -> Color {
        match self.bg {
            Some(bg) => mix(self.dim3, bg, 0.3),
            None => self.dim3,
        }
    }

    /// A painted surface (a card, the cursor's row) on a muted board.
    fn muted_bg(&self, bg: Color) -> Color {
        match (bg, self.bg) {
            (Color::Reset, _) => Color::Reset,
            (bg, Some(ground)) => mix(bg, ground, 0.75),
            (_, None) => Color::Reset,
        }
    }
}

/// `a`, `t` of the way to `b`: mixed where both are RGB, else whichever is
/// nearer.
fn mix(a: Color, b: Color, t: f32) -> Color {
    match (a, b) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
            let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
            Color::Rgb(m(r1, r2), m(g1, g2), m(b1, b2))
        }
        _ if t >= 0.5 => b,
        _ => a,
    }
}

/// What a veil draws toward.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Toward {
    /// The board, in one faint ink.
    Muted,
    /// Nothing: the theme's ground.
    Ground,
}

/// Draw `buf` `k` of the way toward `toward`: at 1 all of it, at 0 none.
pub(crate) fn veil(buf: &mut Buffer, inks: &Inks, toward: Toward, k: f32) {
    if k <= 0.0 {
        return;
    }
    let ground = inks.bg.unwrap_or(Color::Reset);
    for cell in &mut buf.content {
        let (fg, bg) = match toward {
            Toward::Muted => (inks.muted(), inks.muted_bg(cell.bg)),
            Toward::Ground => (ground, ground),
        };
        cell.fg = mix(cell.fg, fg, k);
        cell.bg = mix(cell.bg, bg, k);
        if k >= 0.5 {
            // Bold, reverse and underlines would each stand out of the mute.
            cell.modifier = Modifier::empty();
            cell.underline_color = Color::Reset;
        }
    }
}

/// The board's first frames after the wait: the veil the dialog stood over,
/// lifted in two steps, a tick apart.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Veil {
    from: Instant,
    toward: Toward,
    inks: Inks,
}

/// Each step of the lift holds this long: the board loop's tick.
const LIFT_STEP_MS: u64 = 100;

impl Veil {
    fn after(mode: &Mode, inks: Inks) -> Option<Veil> {
        let toward = match mode {
            Mode::Reload { .. } => Toward::Muted,
            Mode::Open => Toward::Ground,
        };
        // Mono mutes nothing, and a ground the terminal paints cannot be
        // faded from.
        if inks.mono || (toward == Toward::Ground && inks.bg.is_none()) {
            return None;
        }
        Some(Veil { from: Instant::now(), toward, inks })
    }

    /// Veil a board frame as it stands now; false once the veil is gone.
    pub(crate) fn apply(&self, buf: &mut Buffer) -> bool {
        let k = lift_at(self.from.elapsed().as_millis() as u64);
        veil(buf, &self.inks, self.toward, k);
        k > 0.0
    }
}

fn lift_at(ms: u64) -> f32 {
    match ms / LIFT_STEP_MS {
        0 => 2.0 / 3.0,
        1 => 1.0 / 3.0,
        _ => 0.0,
    }
}

/// What stands behind the dialog.
#[derive(Debug, Clone)]
pub(crate) enum Behind {
    /// The theme's ground: an open, or a screen whose size changed.
    Ground,
    /// The board the `U` was pressed on, muted ([`Behind::board`]).
    Board(Buffer),
    /// Whatever is on the terminal: a `U`'s new image, over the muted board
    /// the old one left. Only the dialog's cells are drawn.
    Kept,
}

impl Behind {
    /// The board's last frame, muted, behind a `U`'s dialog.
    pub(crate) fn board(mut frame: Buffer, inks: &Inks) -> Behind {
        if inks.mono {
            return Behind::Ground;
        }
        veil(&mut frame, inks, Toward::Muted, 1.0);
        Behind::Board(frame)
    }

    /// A `U`'s new image: the old one's muted board stays where it is.
    pub(crate) fn kept(inks: &Inks) -> Behind {
        if inks.mono {
            Behind::Ground
        } else {
            Behind::Kept
        }
    }
}

/// The dialog's width with its frame, terminal permitting.
const DIALOG_W: u16 = 56;

/// Where the dialog stands in `area`.
fn dialog_rect(area: Rect, show: &Show) -> Rect {
    let h = (show.lines().len() as u16 + 2).min(area.height);
    let w = DIALOG_W.min(area.width.saturating_sub(4)).max(area.width.min(8));
    Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    }
}

/// Draw the scene into `area`: what stands behind always, the dialog once
/// visible.
pub(crate) fn render(buf: &mut Buffer, area: Rect, show: &Show, inks: &Inks, behind: &Behind) {
    match behind {
        Behind::Board(frame) if frame.area == area => buf.merge(frame),
        Behind::Kept => {}
        _ => {
            if let Some(bg) = inks.bg {
                buf.set_style(area, Style::default().bg(bg));
            }
        }
    }
    if !show.visible() || area.width < 12 || area.height < 4 {
        return;
    }
    dialog(buf, dialog_rect(area, show), show, inks);
}

fn dialog(buf: &mut Buffer, r: Rect, show: &Show, inks: &Inks) {
    let ground = inks.bg.unwrap_or(Color::Reset);
    // Every cell inked, so each one differs from an empty cell and is
    // written even where the screen was kept: what the old image drew under
    // the dialog never shows through.
    let blank = Style::default().fg(inks.dim3).bg(ground);
    for y in r.top()..r.bottom() {
        for x in r.left()..r.right() {
            buf[(x, y)].reset();
            buf[(x, y)].set_char(' ').set_style(blank);
        }
    }
    let put = |buf: &mut Buffer, x: u16, y: u16, s: &str, style: Style, end: u16| {
        let mut c = x;
        for ch in s.chars() {
            if c >= end {
                break;
            }
            buf[(c, y)].set_char(ch).set_style(style);
            c += 1;
        }
        c
    };

    // The frame, the title set into its top edge and the key into its
    // bottom, as every dialog on the board wears them.
    let g = glyphs::frame_set(inks.tier);
    let edge = Style::default().fg(if show.ready() { inks.calm } else { inks.dim3 });
    let (top, bottom, left, right) = (r.top(), r.bottom() - 1, r.left(), r.right() - 1);
    for x in left..=right {
        buf[(x, top)].set_char(g.h).set_style(edge);
        buf[(x, bottom)].set_char(g.h).set_style(edge);
    }
    for y in top..=bottom {
        buf[(left, y)].set_char(g.v).set_style(edge);
        buf[(right, y)].set_char(g.v).set_style(edge);
    }
    buf[(left, top)].set_char(g.tl);
    buf[(right, top)].set_char(g.tr);
    buf[(left, bottom)].set_char(g.bl);
    buf[(right, bottom)].set_char(g.br);
    let edge_end = right.saturating_sub(1);
    let (word, detail) = show.title();
    let x = put(buf, left + 2, top, &format!(" {word} "), Style::default().fg(inks.base), edge_end);
    if !detail.is_empty() {
        put(buf, x, top, &format!("∙ {detail} "), Style::default().fg(inks.dim2), edge_end);
    }
    let keys = " ctrl+c quit ";
    if let Some(kx) = right.checked_sub(1 + keys.len() as u16).filter(|&kx| kx > left + 1) {
        put(buf, kx, bottom, keys, Style::default().fg(inks.dim2), edge_end);
    }

    // The rows.
    let (x0, end) = (left + 3, right.saturating_sub(2));
    let spin = glyphs::spinner(inks.tier, (show.now / glyphs::SPIN_STEP_MS) as usize);
    for (i, line) in show.lines().into_iter().enumerate() {
        let y = top + 1 + i as u16;
        if y >= bottom {
            break;
        }
        match line {
            Line::Blank => {}
            Line::Step(j) => {
                let m = show.mark(j);
                let (ch, mark, words) = inks.mark(m);
                let ch = if m == Mark::Now { spin } else { ch };
                put(buf, x0, y, &ch.to_string(), mark, end);
                let (w, count) = show.words(j);
                let x = put(buf, x0 + 2, y, &w, words, end);
                if let Some(count) = count {
                    put(buf, x, y, &count, Style::default().fg(inks.dim2), end);
                }
            }
            Line::Bar => bar(buf, x0, y, end.saturating_sub(x0), show, inks),
            Line::Sub(s) => {
                let ink = if show.failed() { inks.dim2 } else { inks.dim3 };
                put(buf, x0, y, s, Style::default().fg(ink), end);
            }
        }
    }
}

/// The working bar: a bright run sweeping a faint track, all `calm` once
/// the board is ready, still once it failed.
fn bar(buf: &mut Buffer, x0: u16, y: u16, width: u16, show: &Show, inks: &Inks) {
    let ascii = inks.tier == Tier::Ascii;
    let centre = (show.sweep() * f32::from(width.saturating_sub(1))).round() as i32;
    for i in 0..width {
        let d = (i32::from(i) - centre).abs();
        let (ch, ink) = if show.ready() {
            (if ascii { '=' } else { '━' }, inks.calm)
        } else if show.failed() {
            (if ascii { '-' } else { '━' }, inks.dim3)
        } else if ascii {
            (if d <= 3 { '=' } else { '-' }, inks.base)
        } else {
            let ink = match d {
                0..=1 => inks.base,
                2..=3 => inks.dim1,
                4..=5 => inks.dim2,
                _ => inks.dim3,
            };
            ('━', ink)
        };
        buf[(x0 + i, y)].set_char(ch).set_style(Style::default().fg(ink));
    }
}

// -- the thread ------------------------------------------------------------------

#[derive(Default)]
struct Ctl {
    end: Option<End>,
    abandon: bool,
}

type Out<B> = io::Result<(Terminal<B>, Option<Veil>)>;

/// The screen, playing on a thread of its own that holds the terminal until
/// the wait ends.
pub(crate) struct Loader<B: Backend + Send + 'static> {
    ctl: Arc<Mutex<Ctl>>,
    handle: Option<JoinHandle<Out<B>>>,
    restore: fn(),
}

impl<B> Loader<B>
where
    B: Backend<Error = io::Error> + Send + 'static,
{
    /// Start playing on `term`, over `behind`. Set the first [`step`]
    /// before this. `restore` gives the terminal back: on Ctrl+C, and on
    /// any way out but [`Loader::finish`] and [`Loader::stop`].
    pub(crate) fn start(
        mut term: Terminal<B>,
        inks: Inks,
        mode: Mode,
        behind: Behind,
        restore: fn(),
    ) -> Self {
        let ctl = Arc::new(Mutex::new(Ctl::default()));
        let theirs = Arc::clone(&ctl);
        let handle = std::thread::spawn(move || {
            let start = Instant::now();
            let mut behind = behind;
            let first = term.size()?;
            let mut show = Show::new(mode.clone(), reported());
            loop {
                let now = start.elapsed().as_millis() as u64;
                let (end, abandon) = {
                    let c = theirs.lock().unwrap_or_else(|e| e.into_inner());
                    (c.end, c.abandon)
                };
                if abandon {
                    return Ok((term, None));
                }
                show.advance(now, reported());
                if let Some(end) = end {
                    show.finish(end);
                }
                if show.over() {
                    // A board that paints no ground repaints nothing blank,
                    // so the muted board kept around the dialog would stay.
                    if matches!(behind, Behind::Kept) && inks.bg.is_none() {
                        crate::quiet::clear(&mut term)?;
                    }
                    let veil = if show.seen() { Veil::after(&mode, inks) } else { None };
                    return Ok((term, veil));
                }
                term.draw(|f| {
                    let area = f.area();
                    // A resize clears the screen: nothing is kept, and a
                    // board of the old size no longer fits.
                    if area.width != first.width || area.height != first.height {
                        behind = Behind::Ground;
                    }
                    render(f.buffer_mut(), area, &show, &inks, &behind);
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

    /// The wait is over: play its last beat, then hand the terminal back,
    /// with the veil the board's first frames lift when the scene was seen.
    pub(crate) fn finish(mut self, end: End) -> Out<B> {
        self.ctl.lock().unwrap_or_else(|e| e.into_inner()).end = Some(end);
        self.join()
    }

    /// Stop at once (the `exec`, whose new image carries on).
    pub(crate) fn stop(mut self) -> io::Result<Terminal<B>> {
        self.ctl.lock().unwrap_or_else(|e| e.into_inner()).abandon = true;
        self.join().map(|(term, _)| term)
    }

    fn join(&mut self) -> Out<B> {
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
    use crate::theme::Flavor;
    use ratatui::layout::Position;

    fn inks(profile: Profile) -> Inks {
        Inks::of(&Theme::new(Flavor::Graphite, profile))
    }

    fn draw_over(show: &Show, w: u16, h: u16, inks: &Inks, behind: &Behind) -> Buffer {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        render(&mut buf, area, show, inks, behind);
        buf
    }

    fn draw(show: &Show, w: u16, h: u16, inks: &Inks) -> Buffer {
        draw_over(show, w, h, inks, &Behind::Ground)
    }

    fn rows(buf: &Buffer) -> Vec<String> {
        let a = buf.area;
        (0..a.height)
            .map(|y| (0..a.width).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect()
    }

    fn text(buf: &Buffer) -> String {
        rows(buf).join("\n")
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

    /// A board with colour in it: text in attn and calm, a painted card.
    fn a_board(w: u16, h: u16) -> Buffer {
        let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
        let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
        buf.set_style(buf.area, Style::default().bg(theme.bg.unwrap()));
        buf.set_string(3, 4, "Release notes pass", Style::default().fg(theme.attn));
        buf.set_string(
            3,
            6,
            "Fix OSC-11 detection",
            Style::default().fg(theme.rest.base).bg(theme.selected_bg.unwrap()),
        );
        buf.set_string(
            3,
            8,
            "✓ Grapheme",
            Style::default().fg(theme.calm).add_modifier(Modifier::BOLD),
        );
        buf
    }

    #[test]
    fn the_rows_tick_off_in_order_and_never_back() {
        let ink = inks(Profile::TrueColor);
        let mut show = Show::new(reload("0.0.1"), Step::Stop);
        let mut t = 0;
        for (step, done) in [(Step::Stop, 0), (Step::Switch, 1), (Step::Spawn, 2), (Step::Read, 3)]
        {
            t += 600;
            run(&mut show, t, step);
            let marks: Vec<Mark> = (0..4).map(|j| show.mark(j)).collect();
            assert!(marks[..done].iter().all(|&m| m == Mark::Done), "{step:?}: {marks:?}");
            assert_eq!(marks[done], Mark::Now, "{step:?}");
        }
        let at_board = text(&draw(&show, 80, 24, &ink));
        assert!(at_board.contains("✓ daemon started"), "a spawn says started:\n{at_board}");
        // A build-skew restart reports the stop again: the words change, the
        // ticks do not.
        run(&mut show, t + 600, Step::Stop);
        assert_eq!(show.goal, 3);
        assert!(show.words(3).0.starts_with("stopping the old daemon"));
        assert_eq!(show.mark(0), Mark::Done);
    }

    #[test]
    fn a_new_image_picks_up_with_the_old_daemon_ticked() {
        // The old process drew the first row; the new one starts at the next.
        let ink = inks(Profile::TrueColor);
        let show = Show::new(reload("0.0.1"), Step::Switch);
        assert!(show.visible(), "a reload shows at once: the screen was already up");
        let t = text(&draw(&show, 80, 24, &ink));
        assert!(t.contains("✓ old daemon stopped"), "{t}");
        assert!(t.contains(&format!("switching to {VERSION}")), "{t}");
        assert!(t.contains("· connecting to the daemon"), "{t}");
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
            (reload("0.0.1"), Step::Spawn, 2000, Some(End::Failed)),
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
                let buf = draw_over(&show, w, h, &ink, &Behind::board(a_board(w, h), &ink));
                let t = text(&buf);
                for j in 0..show.rows().len() {
                    let words = show.words(j).0;
                    let words = words.trim_end_matches(['.', ' ']);
                    assert!(
                        t.contains(words),
                        "{mode:?} {step:?} at {w}x{h}: no {words:?} in\n{t}"
                    );
                }
                assert!(t.contains('╭') && t.contains('╯'), "framed");
                for cell in buf.content() {
                    assert_ne!(cell.fg, theme.attn, "{mode:?} {step:?}: attn is needs-you's alone");
                    assert_ne!(cell.bg, theme.attn, "{mode:?} {step:?}: attn is needs-you's alone");
                }
            }
        }
    }

    #[test]
    fn an_open_wears_the_ground_from_its_first_frame() {
        let ink = inks(Profile::TrueColor);
        let show = Show::new(Mode::Open, Step::Dial);
        let buf = draw(&show, 60, 20, &ink);
        assert!(
            buf.content().iter().all(|c| Some(c.bg) == ink.bg),
            "every cell on the theme's ground"
        );
    }

    #[test]
    fn a_reload_mutes_the_board_behind_the_dialog() {
        let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
        let ink = inks(Profile::TrueColor);
        let show = Show::new(reload("0.0.1"), Step::Stop);
        // Tall enough that the dialog stands below the cards.
        let buf = draw_over(&show, 80, 40, &ink, &Behind::board(a_board(80, 40), &ink));
        let t = rows(&buf);
        assert!(t[4].contains("Release notes pass"), "the board stays in sight:\n{}", t.join("\n"));
        let row = |y: u16| (3..21).map(move |x| (x, y));
        for (x, y) in row(4).chain(row(6)).chain(row(8)) {
            let c = &buf[(x, y)];
            assert_eq!(c.fg, ink.muted(), "({x},{y}) {:?}: one faint ink", c.symbol());
            assert!(c.modifier.is_empty(), "nothing bold stands out of the mute");
            assert_ne!(c.bg, theme.selected_bg.unwrap(), "the card's surface is muted too");
        }
    }

    #[test]
    fn a_kept_screen_draws_the_dialog_and_nothing_else() {
        // The new image cannot know the muted board the old one left; the
        // cells it leaves empty are the ones the terminal keeps.
        let ink = inks(Profile::TrueColor);
        let show = Show::new(reload("0.0.1"), Step::Switch);
        let area = Rect::new(0, 0, 80, 24);
        let buf = draw_over(&show, 80, 24, &ink, &Behind::Kept);
        let r = dialog_rect(area, &show);
        for y in 0..24 {
            for x in 0..80 {
                let inside = r.contains(Position::new(x, y));
                let empty = buf[(x, y)] == ratatui::buffer::Cell::default();
                assert_eq!(empty, !inside, "({x},{y}): inside the dialog {inside}");
            }
        }
    }

    #[test]
    fn a_reloads_dialog_never_shrinks_from_one_image_to_the_next() {
        // What the old image drew outside the new image's dialog would stay.
        let area = Rect::new(0, 0, 80, 24);
        let old = dialog_rect(area, &Show::new(reload(VERSION), Step::Stop));
        for step in [Step::Switch, Step::Dial, Step::Spawn, Step::Read, Step::Stop] {
            let mut show = Show::new(reload("0.0.1"), Step::Switch);
            run(&mut show, 3000, step);
            assert_eq!(dialog_rect(area, &show), old, "{step:?}");
            show.finish(End::Ready);
            assert_eq!(dialog_rect(area, &show), old, "{step:?}, ready");
        }
        let mut failed = Show::new(reload("0.0.1"), Step::Spawn);
        failed.finish(End::Failed);
        let grown = dialog_rect(area, &failed);
        assert!(grown.height >= old.height && grown.width == old.width);
    }

    #[test]
    fn the_bar_sweeps_while_it_waits() {
        let ink = inks(Profile::TrueColor);
        let mut show = Show::new(Mode::Open, Step::Dial);
        let brightest = |show: &Show| {
            let buf = draw(show, 80, 24, &ink);
            let r = dialog_rect(buf.area, show);
            let y = r.y + 1 + show.lines().iter().position(|l| *l == Line::Bar).unwrap() as u16;
            (r.x..r.right()).find(|&x| buf[(x, y)].symbol() == "━" && buf[(x, y)].fg == ink.base)
        };
        run(&mut show, GRACE_MS, Step::Dial);
        let a = brightest(&show).expect("a bright run on the track");
        run(&mut show, GRACE_MS + SWEEP_MS / 2, Step::Dial);
        let b = brightest(&show).expect("still running");
        assert!(b > a + 10, "it moved along: {a} then {b}");
        show.finish(End::Ready);
        assert_eq!(brightest(&show), None, "ready: the whole track in calm");
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
        assert!(!show.seen(), "and the board just appears");
    }

    #[test]
    fn a_seen_scene_shows_every_row_ticked_before_it_ends() {
        let ink = inks(Profile::TrueColor);
        let mut show = Show::new(Mode::Open, Step::Dial);
        run(&mut show, 1500, Step::Read);
        show.finish(End::Ready);
        let t = text(&draw(&show, 80, 24, &ink));
        assert!(t.contains("✓ daemon connected") && t.contains("✓ board read"), "{t}");
        run(&mut show, 1500 + READY_MS - 50, Step::Read);
        assert!(!show.over());
        run(&mut show, 1500 + READY_MS, Step::Read);
        assert!(show.over() && show.seen());
    }

    #[test]
    fn a_failed_start_says_so_on_the_daemons_row() {
        let ink = inks(Profile::TrueColor);
        let mut show = Show::new(Mode::Open, Step::Dial);
        run(&mut show, 1000, Step::Spawn);
        show.finish(End::Failed);
        let t = text(&draw(&show, 80, 24, &ink));
        assert!(t.contains("✗ the daemon did not start"), "{t}");
        assert!(t.contains("opening the board anyway"), "{t}");
        assert!(t.contains("it keeps trying every 2 s"), "{t}");
        assert!(t.contains("· reading the board"), "{t}");
    }

    #[test]
    fn a_slow_handover_counts_its_wait() {
        let ink = inks(Profile::TrueColor);
        let mut show = Show::new(reload("0.0.1"), Step::Stop);
        run(&mut show, 5300, Step::Stop);
        let t = text(&draw(&show, 80, 24, &ink));
        assert!(t.contains("the old daemon is still finishing up"), "{t}");
        assert!(t.contains("  5 s"), "{t}");
        assert!(t.contains("your agents keep running"), "{t}");
    }

    #[test]
    fn the_title_names_both_versions_only_across_a_change() {
        let ink = inks(Profile::TrueColor);
        let changed =
            rows(&draw(&Show::new(reload("0.0.1"), Step::Switch), 80, 24, &ink)).join("\n");
        assert!(changed.contains(&format!("UPDATE ∙ 0.0.1 → {VERSION}")), "{changed}");
        let same = rows(&draw(&Show::new(reload(VERSION), Step::Switch), 80, 24, &ink)).join("\n");
        assert!(!same.contains('→'), "a rebuild of one version names one:\n{same}");
        assert!(same.contains("starting the new build"), "{same}");
    }

    #[test]
    fn mono_says_the_words_in_ascii_over_no_board() {
        let ink = inks(Profile::Mono);
        let mut show = Show::new(reload("0.0.1"), Step::Stop);
        run(&mut show, 800, Step::Spawn);
        let t = text(&draw(&show, 80, 24, &ink));
        assert!(t.contains("starting the daemon"), "{t}");
        assert!(t.contains("+ old daemon stopped"), "{t}");
        assert!(!t.contains(['╭', '━', '✓']), "mono draws ascii:\n{t}");
        assert!(matches!(Behind::kept(&ink), Behind::Ground), "mono cannot mute a board");
        assert!(Veil::after(&reload("0.0.1"), ink).is_none());
    }

    #[test]
    fn the_veil_lifts_in_two_steps() {
        let ink = inks(Profile::TrueColor);
        assert_eq!(lift_at(0), 2.0 / 3.0);
        assert_eq!(lift_at(LIFT_STEP_MS), 1.0 / 3.0);
        assert_eq!(lift_at(2 * LIFT_STEP_MS), 0.0);
        let board = a_board(40, 10);
        let mut half = board.clone();
        veil(&mut half, &ink, Toward::Muted, 1.0 / 3.0);
        let (was, now) = (board[(3, 4)].fg, half[(3, 4)].fg);
        assert!(now != was && now != ink.muted(), "a third of the way: {was:?} {now:?}");
        let mut none = board.clone();
        veil(&mut none, &ink, Toward::Muted, 0.0);
        assert_eq!(none, board, "lifted, the board is itself");
    }
}
