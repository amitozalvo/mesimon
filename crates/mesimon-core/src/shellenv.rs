//! What a pane's environment should be — the pure half.
//!
//! D29 built the child env from a 9-name allowlist (`HOME USER LOGNAME SHELL
//! LANG LC_ALL LC_CTYPE TMPDIR PATH`) taken from the daemon's own environment.
//! Two things were wrong with that, and both were measured on a live board
//! (2026-09-01):
//!
//! 1. **It was frozen.** The tmux server captures its global environment when
//!    it starts and `update-environment ""` stops it ever refreshing; nothing
//!    in mesimon kills the server, so a two-day-old PATH was still being handed
//!    to fresh Claude panes.
//! 2. **It was too narrow.** A Claude pane is exec'd directly by tmux — no
//!    shell runs, so `~/.zshrc` is never read — which means an `export` the
//!    user added to their rc file could not reach the agent, its MCP servers,
//!    or its hooks by any route at all. A shell pane sourced the rc and an
//!    agent pane did not, so the two disagreed about the same machine.
//!
//! What replaces it: the daemon asks the user's own login shell what its
//! environment is (`mesimon_daemon::shellenv::capture`) and this module decides
//! what of that a pane may keep. The allowlist becomes a *denylist*, because
//! the set mesimon must withhold is small, closed and knowable — tmux's own
//! plumbing, a description of somebody else's terminal, and a shell's
//! process-local bookkeeping — while the set a user might legitimately export
//! is not enumerable in advance. That is the whole reason the old shape failed.
//!
//! This module forks nothing and reads no file: it parses a captured dump and
//! filters it. The capture lives in the daemon because it spawns a shell.

/// Names never carried from a captured shell environment into a pane.
///
/// Three kinds, and each one is a bug if it gets through:
///
/// * **tmux's plumbing.** `TMUX`/`TMUX_PANE` name the socket and pane of
///   whatever tmux the capture ran under; handing them to a new pane makes it
///   believe it is nested inside another session.
/// * **Somebody else's terminal.** `TERM` and friends describe the terminal
///   the capture happened in, not the pane. The backend sets `TERM` itself,
///   and `LINES`/`COLUMNS` would pin a pane to a dead window's geometry.
/// * **Process-local bookkeeping.** `PWD`, `OLDPWD`, `SHLVL` and `_` belong to
///   the shell that produced the dump. `PWD` is the sharp one: the backend
///   spawns with `-c <cwd>`, and a stale `PWD` disagreeing with the real
///   working directory is exactly the kind of lie a tool believes.
///
/// `PATH` is on the list for a different and non-obvious reason — see
/// [`PATH_IS_THE_CLIENTS`].
const DENY: &[&str] = &[
    "TMUX",
    "TMUX_PANE",
    "TERM",
    "TERM_PROGRAM",
    "TERM_PROGRAM_VERSION",
    "PWD",
    "OLDPWD",
    "SHLVL",
    "_",
    "LINES",
    "COLUMNS",
    "PATH",
];

/// Why `PATH` is denied here rather than passed like everything else.
///
/// tmux does not take a pane's `PATH` from the session environment. It takes it
/// from the environment of the *client process that issued the spawn*, so that
/// a command given by bare name can be found — and it does that in preference
/// to `new-session -e PATH=…`, which lands in the session's environment table
/// and is then ignored by the child. Measured 2026-09-01 against tmux 3.6a:
/// a pane spawned with `-e PATH=/EPATH/bin:…` from a client holding
/// `PATH=/CLIENTPATH/bin:…` came up with the client's, and `show-environment`
/// on that same session reported the `-e` one. A bare command name that exists
/// only on the `-e` PATH dies with status 127.
///
/// So `PATH` travels a different road: `TmuxBackend::set_env` puts it in the
/// client environment of every tmux invocation. Passing it through `-e` as well
/// would be inert at best and, since the two roads can disagree, a false
/// explanation for the next person debugging this.
pub const PATH_IS_THE_CLIENTS: &str =
    "tmux takes a pane's PATH from the spawning client, not from `-e`";

/// mesimon's own per-session variables (`MESIMON_TICKET`,
/// `MESIMON_WORKTREE_BRANCH`) are minted per spawn. A capture that ran inside
/// a mesimon pane would otherwise hand the next session the previous one's
/// ticket — the capture's shell inherits them like any other child.
const MESIMON_PREFIX: &str = "MESIMON_";

/// A single value's ceiling. Nothing legitimate is this big; a runaway one
/// would ride the tmux argv on every spawn forever.
pub const MAX_VALUE: usize = 32 * 1024;
/// And a ceiling on the whole set, for the same reason.
pub const MAX_TOTAL: usize = 128 * 1024;

/// Is this a name mesimon will carry into a pane?
///
/// The syntax check is not decoration: bash exports functions as entries named
/// `BASH_FUNC_foo%%`, and tmux's `-e` parses `NAME=value` positionally, so a
/// name carrying `=` or a `%` would either be refused or silently reinterpreted.
pub fn admissible(name: &str) -> bool {
    if name.is_empty() || DENY.contains(&name) || name.starts_with(MESIMON_PREFIX) {
        return false;
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap_or('\0');
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Parse a NUL-separated `env -0` dump.
///
/// NUL separation is what makes this safe to do at all: a newline-separated
/// dump cannot be parsed correctly, because a value may contain newlines and
/// nothing distinguishes that from the next entry. Anything not valid UTF-8, or
/// carrying no `=`, is dropped rather than guessed at.
pub fn parse_env0(bytes: &[u8]) -> Vec<(String, String)> {
    bytes
        .split(|b| *b == 0)
        .filter(|chunk| !chunk.is_empty())
        .filter_map(|chunk| std::str::from_utf8(chunk).ok())
        .filter_map(|entry| entry.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// The `-e` set a pane should carry: admissible names, sorted, first
/// occurrence wins, truncated at [`MAX_TOTAL`].
///
/// Sorting is not cosmetic — it makes the argv a spawn produces a function of
/// the environment alone, so two spawns from one capture are byte-identical and
/// a diff between captures is readable.
pub fn select(captured: &[(String, String)]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut total = 0usize;
    let mut sorted: Vec<&(String, String)> = captured.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    for (k, v) in sorted {
        if !admissible(k) || v.len() > MAX_VALUE || out.iter().any(|(n, _)| n == k) {
            continue;
        }
        let cost = k.len() + v.len() + 2;
        if total + cost > MAX_TOTAL {
            continue;
        }
        total += cost;
        out.push((k.clone(), v.clone()));
    }
    out
}

/// The captured `PATH`, if the dump had a usable one. Empty is not usable: a
/// pane with an empty PATH cannot find `claude`, and falling back to the
/// daemon's own stale-but-real PATH beats shipping a broken one.
pub fn path_of(captured: &[(String, String)]) -> Option<String> {
    captured
        .iter()
        .find(|(k, _)| k == "PATH")
        .map(|(_, v)| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn nul_separation_survives_a_value_with_newlines() {
        // The whole reason the dump is `env -0`: a newline split would read
        // this as three entries, two of them garbage.
        let dump = b"A=one\ntwo\nthree\0B=plain\0";
        assert_eq!(parse_env0(dump), cap(&[("A", "one\ntwo\nthree"), ("B", "plain")]));
    }

    #[test]
    fn a_malformed_entry_is_dropped_not_guessed_at() {
        assert_eq!(parse_env0(b"NOEQUALS\0A=1\0"), cap(&[("A", "1")]));
        assert_eq!(parse_env0(b"\0\0"), Vec::new());
    }

    #[test]
    fn tmux_plumbing_never_reaches_a_pane() {
        // A capture that ran inside a mesimon pane carries all of these.
        for name in ["TMUX", "TMUX_PANE", "TERM", "TERM_PROGRAM", "LINES", "COLUMNS"] {
            assert!(!admissible(name), "{name} must not travel");
        }
    }

    #[test]
    fn a_shells_own_bookkeeping_never_reaches_a_pane() {
        // PWD is the sharp one: the backend spawns with `-c <cwd>` and a stale
        // PWD would contradict the real working directory.
        for name in ["PWD", "OLDPWD", "SHLVL", "_"] {
            assert!(!admissible(name), "{name} must not travel");
        }
    }

    #[test]
    fn path_is_denied_because_it_travels_as_the_client_env() {
        assert!(!admissible("PATH"));
        assert_eq!(path_of(&cap(&[("PATH", "/a:/b")])), Some("/a:/b".into()));
        // An empty PATH is worse than a stale one — nothing would resolve.
        assert_eq!(path_of(&cap(&[("PATH", "  ")])), None);
        assert_eq!(path_of(&cap(&[("HOME", "/h")])), None);
    }

    #[test]
    fn mesimons_own_variables_are_minted_per_spawn_never_inherited() {
        // A capture run inside a pane inherits the pane's ticket; carrying it
        // would tell the NEXT session it belongs to the previous ticket.
        assert!(!admissible("MESIMON_TICKET"));
        assert!(!admissible("MESIMON_WORKTREE_BRANCH"));
        assert!(!admissible("MESIMON_ANYTHING_AT_ALL"));
    }

    #[test]
    fn exported_bash_functions_cannot_be_named_in_a_tmux_e_flag() {
        assert!(!admissible("BASH_FUNC_foo%%"));
        assert!(!admissible("has space"));
        assert!(!admissible("has=equals"));
        assert!(!admissible("1LEADINGDIGIT"));
        assert!(!admissible(""));
        assert!(admissible("_UNDERSCORE_LEAD"));
        assert!(admissible("ANTHROPIC_API_KEY"));
    }

    #[test]
    fn the_user_env_travels_and_it_is_not_an_allowlist() {
        // The point of the change: a name mesimon has never heard of gets
        // through, because the set a user may export is not enumerable.
        let sel = select(&cap(&[
            ("HOME", "/Users/x"),
            ("SOME_PRIVATE_TOKEN", "abc"),
            ("TMUX", "/tmp/s,1,0"),
            ("PATH", "/a:/b"),
        ]));
        assert_eq!(sel, cap(&[("HOME", "/Users/x"), ("SOME_PRIVATE_TOKEN", "abc")]));
    }

    #[test]
    fn select_is_sorted_and_first_wins() {
        let sel = select(&cap(&[("B", "2"), ("A", "1"), ("A", "shadow")]));
        assert_eq!(sel, cap(&[("A", "1"), ("B", "2")]));
    }

    #[test]
    fn an_absurd_value_rides_no_spawn() {
        let big = "x".repeat(MAX_VALUE + 1);
        let sel = select(&cap(&[("BIG", &big), ("SMALL", "ok")]));
        assert_eq!(sel, cap(&[("SMALL", "ok")]));
    }

    #[test]
    fn the_whole_set_is_capped() {
        let chunk = "y".repeat(MAX_VALUE);
        let pairs: Vec<(String, String)> =
            (0..16).map(|i| (format!("V{i:02}"), chunk.clone())).collect();
        let sel = select(&pairs);
        let total: usize = sel.iter().map(|(k, v)| k.len() + v.len() + 2).sum();
        assert!(total <= MAX_TOTAL, "{total} over cap");
        assert!(!sel.is_empty(), "the cap must not empty the set");
    }
}
