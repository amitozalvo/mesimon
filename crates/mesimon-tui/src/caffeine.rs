//! Power assertion and subprocess backends for the board's keep-awake observer.
//!
//! `caffeine_watch` tracks activity even during terminal handovers. macOS
//! uses a process-owned IOKit assertion; the display and closed-lid behavior
//! are unaffected. Linux uses logind idle AND sleep inhibition so desktop
//! power managers cannot suspend an active turn. The sleep lock can also
//! block explicit suspend requests, subject to the desktop's override policy.
//!
//! `MESIMON_CAFFEINATE=off` disables every backend. `windows` opts into the
//! unverified WSL bridge. `caffeinate` (including an absolute path) wraps
//! `cat` so closing our stdin pipe ends the assertion even after SIGKILL or
//! exec. Other values name custom programs: they MUST hold only while stdin
//! is open and release on EOF, including any children they create. Mesimon
//! kills the direct child on normal release, but cannot enforce that custom
//! contract after its own process is killed.
//!
//! The native assertion belongs to this process. Built-in subprocess
//! backends terminate when our pipe closes. The WSL bridge also has a
//! four-hour cap and a taskkill fallback; it remains opt-in and unverified.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

/// What the hold is called where the OS lists it (`pmset -g assertions`,
/// `systemd-inhibit --list`). It names the product and the reason, because
/// the person reading that list is asking "what is holding this machine
/// awake, and can I close it".
const WHY: &str = "mesimon: an agent is working";

/// The longest the WSL bridge holds before releasing on its own, in
/// milliseconds — the cap that bounds the one rung whose death we cannot
/// guarantee from here. Four hours: longer than any turn, shorter than a
/// night.
const WSL_MAX_HOLD_MS: u64 = 4 * 60 * 60 * 1000;

/// Where WSL's PATH usually has it, for the case where interop is on but the
/// System32 directory is not on PATH.
const WSL_POWERSHELL: &str = "/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe";

/// What the ladder answered: how this machine can be held awake, if it can.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hold {
    /// A power assertion taken by this process (macOS).
    Native,
    /// A program we spawn and kill.
    Spawn(Rung),
    /// Nothing here can hold it.
    None,
}

/// A spawned holder: which one, and the program that runs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rung {
    kind: Kind,
    prog: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Logind idle and sleep locks, wrapped around a reader of our pipe.
    Systemd,
    /// Apple's wrapper watches `cat`, which exits when our pipe closes.
    Caffeinate,
    /// `powershell.exe` through WSL interop, holding the Windows host.
    Windows,
    /// Whatever `MESIMON_CAFFEINATE` named. We hold its stdin and kill it;
    /// what it does in between is its own.
    Custom,
}

impl Rung {
    /// The command line. Built here rather than at `find` time so the ladder
    /// stays a pure answer about the machine and this stays testable with no
    /// process in it.
    pub fn argv(&self) -> Vec<String> {
        let mut v = vec![self.prog.clone()];
        match self.kind {
            Kind::Systemd => {
                v.push("--what=idle:sleep".into());
                v.push("--who=mesimon".into());
                v.push(format!("--why={WHY}"));
                v.push("--mode=block".into());
                // `cat` on the stdin pipe we hold, and this is the whole
                // crash story: the kernel closing our end is what ends the
                // hold, so a SIGKILL, a panic and the reload's `exec` are
                // all covered without a `Drop` of ours running.
                v.push("cat".into());
            }
            Kind::Windows => {
                v.push("-NoProfile".into());
                v.push("-NonInteractive".into());
                v.push("-Command".into());
                v.push(windows_script());
            }
            Kind::Caffeinate => {
                v.push("-i".into());
                v.push("cat".into());
            }
            Kind::Custom => {}
        }
        v
    }

    /// Is this the one rung whose process is on the far side of the WSL
    /// boundary, where a kill may not cross — so the pid it prints is worth
    /// reading?
    fn windows(&self) -> bool {
        self.kind == Kind::Windows
    }
}

impl Hold {
    /// A rung that is just a program, the way `MESIMON_CAFFEINATE=<program>`
    /// names one. The one road into `Hold::Spawn` from outside this module,
    /// so `Kind` stays private and a test elsewhere can still build a rung —
    /// which is all this is for: `find_from` is how a real board gets one.
    #[cfg(test)]
    pub fn program(prog: &str) -> Hold {
        Hold::Spawn(Rung { kind: Kind::Custom, prog: prog.to_string() })
    }
}

/// The script the WSL bridge runs. A CONSTANT: no ticket text, no id, no
/// word of the user's reaches it, which is what `workspace.rs`'s "argv
/// arrays always" rule asks for on the one road where argv is not on offer.
///
/// It prints its own pid (so a kill that does not cross the boundary has
/// `taskkill.exe` behind it), takes `ES_CONTINUOUS | ES_SYSTEM_REQUIRED`,
/// then blocks on ONE asynchronous read of the stdin we hold: EOF ends it,
/// and the wait's own timeout is the cap. One read, never a loop of them —
/// two pending reads on one stream is a bug waiting for a stray byte.
fn windows_script() -> String {
    format!(
        "$ErrorActionPreference='Stop'; \
         Add-Type -Namespace Mesimon -Name Power -MemberDefinition \
         '[DllImport(\"kernel32.dll\")] public static extern uint SetThreadExecutionState(uint f);'; \
         [Console]::Out.WriteLine($PID); [Console]::Out.Flush(); \
         [Mesimon.Power]::SetThreadExecutionState(0x80000001) | Out-Null; \
         $b = New-Object byte[] 1; \
         [Console]::OpenStandardInput().ReadAsync($b,0,1).Wait({WSL_MAX_HOLD_MS}) | Out-Null; \
         [Mesimon.Power]::SetThreadExecutionState(0x80000000) | Out-Null"
    )
}

/// The ladder, read off the environment and the machine.
pub fn find() -> Hold {
    find_from(
        std::env::var("MESIMON_CAFFEINATE").ok().as_deref(),
        cfg!(target_os = "macos"),
        crate::opener::is_wsl(),
        crate::opener::which_on_path,
    )
}

/// Every input is a parameter, `opener::find_from`'s rule — and `macos` is a
/// parameter rather than a `cfg!` inside, so every rung is exercised on
/// every platform instead of half the ladder going untested on each.
fn find_from(
    env: Option<&str>,
    macos: bool,
    wsl: bool,
    which: impl Fn(&str) -> Option<PathBuf>,
) -> Hold {
    match env.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) if v.eq_ignore_ascii_case("off") => return Hold::None,
        Some(v) if v.eq_ignore_ascii_case("windows") => {
            let prog = which("powershell.exe")
                .map(|_| "powershell.exe".to_string())
                .unwrap_or_else(|| WSL_POWERSHELL.to_string());
            return Hold::Spawn(Rung { kind: Kind::Windows, prog });
        }
        Some(v) => {
            let kind = if std::path::Path::new(v).file_name().is_some_and(|n| n == "caffeinate") {
                Kind::Caffeinate
            } else {
                Kind::Custom
            };
            return Hold::Spawn(Rung { kind, prog: v.to_string() });
        }
        None => {}
    }
    if macos {
        return Hold::Native;
    }
    // Under WSL a Linux inhibitor governs the WSL VM and not the host that
    // decides when to sleep, so answering here would be a rung that looks
    // like it works. `MESIMON_CAFFEINATE=windows` is the way in until
    // somebody has watched that bridge work.
    if wsl {
        return Hold::None;
    }
    match which("systemd-inhibit") {
        Some(_) => Hold::Spawn(Rung { kind: Kind::Systemd, prog: "systemd-inhibit".into() }),
        None => Hold::None,
    }
}

/// What `mesimon doctor` says: whether the preference is on, which rung
/// would answer — named even while it is off, because "would it work if I
/// turned it on" is the question somebody reads this line to ask — and,
/// the point of the line, when nothing here can hold it at all.
pub fn doctor_line() -> String {
    let on = crate::prefs::load_home().prefs.keep_awake;
    let word = if on { "on" } else { "off (Settings turns it on)" };
    let env = std::env::var("MESIMON_CAFFEINATE").ok().filter(|v| !v.trim().is_empty());
    let rung = match (env.as_deref().map(str::trim), find()) {
        (Some(v), _) if v.eq_ignore_ascii_case("off") => {
            "nothing holds it ∙ MESIMON_CAFFEINATE=off".to_string()
        }
        (_, Hold::Native) => "an IOKit PreventUserIdleSystemSleep assertion, held by this board \
                              while an agent is mid-turn ∙ the display still sleeps, and a closed \
                              lid still sleeps ∙ pmset -g assertions names it"
            .to_string(),
        (Some(v), Hold::Spawn(r)) if v.eq_ignore_ascii_case("windows") => format!(
            "{} bridges to the Windows host (unverified — MESIMON_CAFFEINATE=windows asked for it)",
            r.prog
        ),
        (Some(_), Hold::Spawn(r)) if r.kind == Kind::Caffeinate => {
            format!("{} -i cat ($MESIMON_CAFFEINATE) ∙ releases on stdin EOF", r.prog)
        }
        (Some(_), Hold::Spawn(r)) => format!(
            "{} ($MESIMON_CAFFEINATE) ∙ custom holder MUST release on stdin EOF, including children",
            r.prog
        ),
        (_, Hold::Spawn(r)) => format!(
            "{} --what=idle:sleep while an agent is mid-turn ∙ may also block explicit suspend \
             ∙ systemd-inhibit --list names it",
            r.prog
        ),
        (_, Hold::None) if crate::opener::is_wsl() => {
            "nothing here can hold it ∙ the Windows host owns sleep under WSL ∙ \
             MESIMON_CAFFEINATE=windows bridges to it (unverified)"
                .to_string()
        }
        (_, Hold::None) => "nothing here can hold it ∙ no systemd-inhibit on PATH ∙ \
                            MESIMON_CAFFEINATE names a program that can"
            .to_string(),
    };
    format!("{word} ∙ {rung}")
}

/// The hold itself: at most one, taken on the edge into work and dropped on
/// the edge out of it.
pub struct Caffeine {
    hold: Hold,
    held: Option<Held>,
    /// An acquire that failed is not retried until the want goes away and
    /// comes back — otherwise a machine that refuses would be asked ten
    /// times a second for as long as the agent worked.
    failed: bool,
    trouble: Option<String>,
}

enum Held {
    /// A power assertion of ours, by id.
    Assertion(u32),
    /// A holder process. `win_pid` is what the Windows bridge printed, read
    /// on a thread so nothing here ever blocks on a pipe.
    Child { child: Child, win_pid: Option<Arc<Mutex<Option<u32>>>> },
}

impl Caffeine {
    pub fn new(hold: Hold) -> Self {
        Caffeine { hold, held: None, failed: false, trouble: None }
    }

    /// Is there any way to hold this machine awake? The Settings row asks,
    /// so that a preference which can do nothing here says so.
    pub fn possible(&self) -> bool {
        !matches!(self.hold, Hold::None)
    }

    /// Is one held RIGHT NOW? This is what the header's mark draws, so it
    /// must be the fact and never the intention.
    pub fn holding(&self) -> bool {
        self.held.is_some()
    }

    /// Driven once an observer beat with the activity level. Two jobs: notice a
    /// holder that DIED on its own, then act on the edge.
    ///
    /// The poll is not housekeeping. `systemd-inhibit` exists on PATH in
    /// plenty of places with no logind to talk to (a container, an ssh
    /// session, WSL without systemd) and exits at once — and a board that
    /// kept drawing the mark over a dead child would be the failure this
    /// feature cannot have: it would say the machine is held when it is not.
    pub fn drive(&mut self, want: bool) {
        let died = match self.held.as_mut() {
            Some(Held::Child { child, .. }) => matches!(child.try_wait(), Ok(Some(_))),
            _ => false,
        };
        if died {
            let word = self.word().to_string();
            self.held = None;
            self.failed = true;
            self.trouble =
                Some(format!("keep awake: {word} stopped ∙ the machine is not being held"));
        }
        if !want {
            self.failed = false;
            if let Some(h) = self.held.take() {
                release(h);
            }
            return;
        }
        // A board with no rung holds nothing and says nothing: the Settings
        // row already carries that sentence, and a status line every time an
        // agent starts a turn would be the same news ten times a day.
        if self.held.is_some() || self.failed || !self.possible() {
            return;
        }
        match self.acquire() {
            Ok(h) => self.held = Some(h),
            Err(why) => {
                self.failed = true;
                self.trouble = Some(why);
            }
        }
    }

    /// What went wrong, once — the board's own loop puts it in the status
    /// line, the way it does for the notification thread.
    pub fn trouble(&mut self) -> Option<String> {
        self.trouble.take()
    }

    pub(crate) fn report_trouble(&mut self, trouble: String) {
        self.trouble = Some(trouble);
    }

    fn word(&self) -> &str {
        match &self.hold {
            Hold::Native => "the power assertion",
            Hold::Spawn(r) => &r.prog,
            Hold::None => "nothing",
        }
    }

    fn acquire(&self) -> Result<Held, String> {
        match &self.hold {
            Hold::Native => acquire_native().map(Held::Assertion),
            Hold::Spawn(rung) => spawn(rung),
            Hold::None => Err("nothing on this machine can hold it awake".into()),
        }
    }
}

impl Drop for Caffeine {
    fn drop(&mut self) {
        self.drive(false);
    }
}

/// Spawn a holder. Its stdin is a pipe WE hold: closing it is the release
/// built-in backends rely on, including after SIGKILL or exec. Custom
/// programs are required to implement that same EOF contract.
fn spawn(rung: &Rung) -> Result<Held, String> {
    let argv = rung.argv();
    let Some((prog, rest)) = argv.split_first() else {
        return Err("keep awake: nothing to run".into());
    };
    let mut cmd = Command::new(prog);
    // stderr null like every other launch here: `systemd-inhibit` writing
    // "failed to connect to bus" would land on the alt screen mid-frame.
    cmd.args(rest)
        .stdin(Stdio::piped())
        .stdout(if rung.windows() { Stdio::piped() } else { Stdio::null() })
        .stderr(Stdio::null());
    let mut child = cmd.spawn().map_err(|e| format!("keep awake: {prog} did not start ∙ {e}"))?;
    let win_pid = rung.windows().then(|| read_pid(&mut child));
    Ok(Held::Child { child, win_pid })
}

/// Read the Windows holder's own pid off its first line, on a thread — the
/// board's loop may never block on a pipe, and the pid is not wanted until
/// the release.
fn read_pid(child: &mut Child) -> Arc<Mutex<Option<u32>>> {
    let slot: Arc<Mutex<Option<u32>>> = Arc::new(Mutex::new(None));
    let Some(out) = child.stdout.take() else { return slot };
    let into = Arc::clone(&slot);
    let _ = std::thread::Builder::new().name("mesimon-awake-pid".into()).spawn(move || {
        use std::io::BufRead;
        let mut line = String::new();
        if std::io::BufReader::new(out).read_line(&mut line).is_ok() {
            if let Ok(pid) = line.trim().parse::<u32>() {
                *into.lock().unwrap_or_else(|e| e.into_inner()) = Some(pid);
            }
        }
    });
    slot
}

fn release(held: Held) {
    match held {
        Held::Assertion(id) => release_native(id),
        Held::Child { mut child, win_pid } => {
            // The pipe FIRST. It is the release the child was written
            // around, it reaches a `cat` under `systemd-inhibit` that a kill
            // aimed at the parent would orphan, and for the Windows bridge
            // it is the only thing that certainly crosses the boundary.
            drop(child.stdin.take());
            let _ = child.kill();
            let _ = child.wait();
            // And behind both, for the one rung whose process is not really
            // ours: a kill that did not cross leaves the host held, so spend
            // one detached fork on making sure.
            if let Some(slot) = win_pid {
                let pid = *slot.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(pid) = pid {
                    let argv = vec![
                        "taskkill.exe".to_string(),
                        "/F".to_string(),
                        "/PID".to_string(),
                        pid.to_string(),
                    ];
                    let _ = crate::opener::launch(&argv, None);
                }
            }
        }
    }
}

/// Take a power assertion in THIS process.
///
/// Verified against the SDK's own `IOPMLib.h`: `IOPMAssertionID` is a
/// `uint32_t`, `IOPMAssertionLevel` a `uint32_t`, `IOReturn` a
/// `kern_return_t`, `kIOPMAssertionLevelOn` is 255, success is 0, and "no
/// special privileges are necessary to make this call". The assertion is
/// listed against its owning process (`IOPMCopyAssertionsByProcess`), which
/// is what makes a crash safe with no child to reap.
///
/// On any other platform this is unreachable — `find_from` never answers
/// `Native` there — and says so rather than compiling to a different shape.
fn acquire_native() -> Result<u32, String> {
    #[cfg(target_os = "macos")]
    {
        mac::acquire(mac::PREVENT_IDLE_SLEEP, WHY)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("no power assertion API on this platform".into())
    }
}

fn release_native(id: u32) {
    #[cfg(target_os = "macos")]
    {
        mac::release(id);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = id;
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::os::raw::{c_char, c_void};

    type CFStringRef = *const c_void;

    /// `kIOPMAssertionTypePreventUserIdleSystemSleep`. The SYSTEM's idle
    /// sleep and not the display's: the machine keeps working, the screen
    /// still sleeps and still locks. It is also the one that holds on
    /// BATTERY — `PreventSystemSleep` (`caffeinate -s`) is documented as
    /// valid only on AC power.
    pub const PREVENT_IDLE_SLEEP: &str = "PreventUserIdleSystemSleep";
    /// `kIOPMAssertionLevelOn`.
    const LEVEL_ON: u32 = 255;
    /// `kCFStringEncodingUTF8`.
    const UTF8: u32 = 0x0800_0100;
    /// `kIOReturnSuccess`.
    const SUCCESS: i32 = 0;

    #[allow(non_snake_case)]
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringCreateWithCString(
            alloc: *const c_void,
            cstr: *const c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFRelease(cf: *const c_void);
    }

    #[allow(non_snake_case)]
    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOPMAssertionCreateWithName(
            assertion_type: CFStringRef,
            level: u32,
            name: CFStringRef,
            id: *mut u32,
        ) -> i32;
        fn IOPMAssertionRelease(id: u32) -> i32;
    }

    pub fn acquire(kind: &str, name: &str) -> Result<u32, String> {
        let kind = std::ffi::CString::new(kind).map_err(|_| "assertion type has a NUL")?;
        // The header caps a name at 128 characters; ours is a constant well
        // under it, and truncating would only make the OS's own list lie.
        let name = std::ffi::CString::new(name).map_err(|_| "assertion name has a NUL")?;
        // SAFETY: both strings are NUL-terminated and live across the calls;
        // every pointer handed back is checked before use, and each CFString
        // is released exactly once on every road out. `IOPMAssertionCreateWithName`
        // copies what it keeps, which is why they can go as soon as it returns.
        unsafe {
            let t = CFStringCreateWithCString(std::ptr::null(), kind.as_ptr(), UTF8);
            if t.is_null() {
                return Err("could not make the assertion type string".into());
            }
            let n = CFStringCreateWithCString(std::ptr::null(), name.as_ptr(), UTF8);
            if n.is_null() {
                CFRelease(t);
                return Err("could not make the assertion name string".into());
            }
            let mut id: u32 = 0;
            let rc = IOPMAssertionCreateWithName(t, LEVEL_ON, n, &mut id);
            CFRelease(n);
            CFRelease(t);
            if rc == SUCCESS {
                Ok(id)
            } else {
                Err(format!("the system refused the wake assertion ({rc})"))
            }
        }
    }

    pub fn release(id: u32) {
        // SAFETY: `id` is one this process took and has not released.
        unsafe {
            IOPMAssertionRelease(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &str) -> Option<PathBuf> {
        None
    }
    fn all(n: &str) -> Option<PathBuf> {
        Some(PathBuf::from(format!("/usr/bin/{n}")))
    }

    #[test]
    fn the_env_wins_every_rung() {
        assert_eq!(find_from(Some("off"), true, false, all), Hold::None, "even on macOS");
        assert_eq!(find_from(Some(" OFF "), false, true, all), Hold::None, "trimmed, case-blind");
        let Hold::Spawn(r) = find_from(Some("caffeinate"), true, false, none) else {
            panic!("a program outranks the native rung — it was asked for by name");
        };
        assert_eq!(
            r.argv(),
            vec!["caffeinate", "-i", "cat"],
            "the wrapper watches our pipe reader"
        );
        let Hold::Spawn(r) = find_from(Some("/usr/bin/caffeinate"), true, false, none) else {
            panic!("absolute caffeinate path");
        };
        assert_eq!(r.argv(), vec!["/usr/bin/caffeinate", "-i", "cat"]);
        // An empty value is an absent one, as everywhere else.
        assert_eq!(find_from(Some("   "), false, false, none), find_from(None, false, false, none));
    }

    #[test]
    fn the_platform_answers_when_the_env_does_not() {
        assert_eq!(find_from(None, true, false, none), Hold::Native, "no child, no PATH");
        assert_eq!(find_from(None, false, false, none), Hold::None, "no systemd-inhibit, no rung");
        let Hold::Spawn(r) = find_from(None, false, false, all) else {
            panic!("systemd-inhibit answers")
        };
        let argv = r.argv();
        assert_eq!(argv[0], "systemd-inhibit");
        assert!(
            argv.contains(&"--what=idle:sleep".to_string()),
            "cover logind's idle action and desktop-initiated Suspend calls"
        );
        assert!(argv.contains(&"--mode=block".to_string()));
        assert_eq!(
            argv.last().map(String::as_str),
            Some("cat"),
            "the pipe is the guard: a pid guard would not fire on the reload's exec, \
             which reuses the pid — {argv:?}"
        );
    }

    #[test]
    fn wsl_answers_nothing_until_it_is_asked_for() {
        // A Linux inhibitor there governs the VM, not the host that sleeps.
        assert_eq!(find_from(None, false, true, all), Hold::None);
        let Hold::Spawn(r) = find_from(Some("windows"), false, true, all) else {
            panic!("the bridge is opt-in, and this opted in");
        };
        assert!(r.windows());
        let argv = r.argv();
        assert_eq!(argv[0], "powershell.exe");
        assert!(argv.contains(&"-NoProfile".to_string()));
        // And where interop is on but System32 is not on PATH.
        let Hold::Spawn(r) = find_from(Some("windows"), false, true, none) else {
            panic!("still a rung")
        };
        assert_eq!(r.argv()[0], WSL_POWERSHELL);
    }

    /// The one script mesimon hands to a shell-ish program carries no word
    /// from anywhere else — not a ticket, not a path, not an id.
    #[test]
    fn the_windows_script_is_a_constant_with_its_own_cap_in_it() {
        let s = windows_script();
        assert!(s.contains("SetThreadExecutionState(0x80000001)"), "continuous + system required");
        assert!(s.contains("SetThreadExecutionState(0x80000000)"), "and it puts it back");
        assert!(s.contains(&WSL_MAX_HOLD_MS.to_string()), "the cap is the wait's own timeout");
        assert!(s.contains("$PID"), "it says who it is, for the taskkill behind the kill");
        assert_eq!(s.matches("ReadAsync").count(), 1, "one pending read, never two");
        assert_eq!(WSL_MAX_HOLD_MS, 14_400_000, "four hours");
    }

    #[test]
    fn a_board_with_no_rung_holds_nothing_and_says_nothing() {
        let mut c = Caffeine::new(Hold::None);
        assert!(!c.possible());
        c.drive(true);
        assert!(!c.holding());
        assert_eq!(c.trouble(), None, "the Settings row already says it; a status line would nag");
    }

    #[test]
    fn a_refused_hold_is_said_once_and_not_retried() {
        let mut c = Caffeine::new(Hold::Spawn(Rung {
            kind: Kind::Custom,
            prog: "/nonexistent/mesimon-keep-awake".into(),
        }));
        c.drive(true);
        assert!(!c.holding());
        let said = c.trouble().expect("a rung that will not start is worth a sentence");
        assert!(said.contains("did not start"), "{said}");
        c.drive(true);
        assert_eq!(c.trouble(), None, "and it is not asked again while the want stands");
        // The want going away re-arms it: the next turn tries once more.
        c.drive(false);
        c.drive(true);
        assert!(c.trouble().is_some());
    }

    #[test]
    fn holding_is_an_edge_and_the_release_kills_the_child() {
        // `cat` is the real Linux rung's inner command, and it blocks on the
        // pipe we hold — which is the property under test.
        let mut c = Caffeine::new(Hold::Spawn(Rung { kind: Kind::Custom, prog: "cat".into() }));
        c.drive(true);
        assert!(c.holding(), "{:?}", c.trouble);
        let pid = match &c.held {
            Some(Held::Child { child, .. }) => child.id(),
            _ => panic!("a spawned rung holds a child"),
        };
        c.drive(true);
        match &c.held {
            Some(Held::Child { child, .. }) => assert_eq!(child.id(), pid, "no second child"),
            _ => panic!("still holding"),
        }
        c.drive(false);
        assert!(!c.holding());
        // Reaped, so the pid is gone rather than a zombie answering to 0.
        // SAFETY: signal 0 asks whether the pid exists; it changes nothing.
        assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1, "the holder is gone");
        c.drive(false);
    }

    /// The failure this feature may not have: a mark on screen over a holder
    /// that is not holding. `systemd-inhibit` with no logind to talk to
    /// exits at once, and this is that shape.
    #[test]
    fn a_holder_that_dies_on_its_own_lets_go_of_the_mark() {
        let mut c = Caffeine::new(Hold::Spawn(Rung { kind: Kind::Custom, prog: "true".into() }));
        c.drive(true);
        let mut said = None;
        for _ in 0..200 {
            c.drive(true);
            if !c.holding() {
                said = c.trouble();
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!c.holding(), "a holder that exited is not a hold");
        let said = said.expect("and it is worth one sentence");
        assert!(said.contains("stopped"), "{said}");
    }

    /// Close the guard pipe without invoking release/Drop, as the kernel
    /// does after a board crash or exec. Bare caffeinate fails this test.
    #[cfg(target_os = "macos")]
    #[test]
    fn caffeinate_override_ends_on_pipe_eof_without_a_parent_kill() {
        let mut c = Caffeine::new(find_from(Some("/usr/bin/caffeinate"), true, false, none));
        c.drive(true);
        assert!(c.holding(), "{:?}", c.trouble);
        let Some(Held::Child { child, .. }) = c.held.as_mut() else {
            panic!("the override is a subprocess");
        };
        drop(child.stdin.take());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while child.try_wait().unwrap().is_none() {
            assert!(std::time::Instant::now() < deadline, "holder survived EOF");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        c.drive(true);
        assert!(!c.holding());
    }

    /// The FFI is the risky part, so take a real assertion and put it back.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_native_assertion_is_taken_and_released() {
        let mut c = Caffeine::new(Hold::Native);
        c.drive(true);
        assert!(c.holding(), "IOKit refused: {:?}", c.trouble);
        c.drive(false);
        assert!(!c.holding());
    }
}
