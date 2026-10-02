//! The road a Claude session's events and mesimon's commands travel (T-574).
//!
//! Claude Code reports to mesimon through the generated hook set (`hooks`,
//! a settings file of commands that exec `mesimon hook`) and, from 2.1.287,
//! can also through a mod laid by the daemon and loaded with `--plugin-dir`
//! (`mod`, T-573's measurements). Phase 1 runs the mod in SHADOW: it relays
//! every hook-set event as a frame marked `road: mod`, the daemon pairs it
//! with the hook set's frame and says when they disagree, and only the hook
//! set's frame is ingested. `hooks` is the default until the soak passes and
//! the kill switch after. Codex has no mods and ignores all of it.
//!
//! Down the other way, the daemon addresses [`ModFrame`]s to one session's
//! mod; `mesimon mod-bridge`, spawned by the mod, long-polls them off
//! `orch.sock` and prints each as one NDJSON line the mod reads.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What a launch got: the road its session reports on. Recorded on the
/// session (`SessionRecord::road`) so the shadow pairs only frames from a
/// session that was handed the mod.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Road {
    #[default]
    Hooks,
    Mod,
}

impl Road {
    pub fn is_hooks(&self) -> bool {
        *self == Road::Hooks
    }

    pub fn word(self) -> &'static str {
        match self {
            Road::Hooks => "hooks",
            Road::Mod => "mod",
        }
    }

    /// A frame header's `road`: absent or `hooks` is the hook set, `mod` the
    /// mod. Any other word is a frame from a build that knows a road this one
    /// does not; `None`, and the frame is dropped rather than ingested twice.
    pub fn from_header(word: Option<&str>) -> Option<Road> {
        match word {
            None | Some("hooks") => Some(Road::Hooks),
            Some("mod") => Some(Road::Mod),
            Some(_) => None,
        }
    }
}

/// Which road a Claude launch asks for. Every launch asks `Auto` (T-588):
/// the mod when the Claude Code on PATH is new enough and `claude plugin
/// validate` passes on the laid mod, the hook set otherwise. The seam
/// `MESIMON_CLAUDE_ROAD` names one of the three words, for the tests alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RoadPref {
    Hooks,
    #[default]
    Auto,
    Mod,
}

impl RoadPref {
    pub const ALL: [RoadPref; 3] = [RoadPref::Hooks, RoadPref::Auto, RoadPref::Mod];

    pub fn word(self) -> &'static str {
        match self {
            RoadPref::Hooks => "hooks",
            RoadPref::Auto => "auto",
            RoadPref::Mod => "mod",
        }
    }

    pub fn from_word(word: &str) -> Option<RoadPref> {
        RoadPref::ALL.into_iter().find(|p| p.word() == word)
    }
}

/// The oldest Claude Code that loads mods (T-573: "mods require Claude Code
/// v2.1.287 or later").
pub const MOD_FLOOR: (u32, u32, u32) = (2, 1, 287);

/// `claude --version`'s answer as numbers: its first `x.y.z` token, as in
/// `2.1.287 (Claude Code)`.
pub fn parse_claude_version(out: &str) -> Option<(u32, u32, u32)> {
    let token = out.split_whitespace().next()?;
    let mut parts = token.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((major, minor, patch))
}

/// Whether that version loads mods.
pub fn has_mods(version: (u32, u32, u32)) -> bool {
    version >= MOD_FLOOR
}

/// The hook-set events the mod relays, by the names `mesimon hook` sends for
/// a Claude session — the twins the shadow pairs. Never `PaneDied` (tmux's),
/// `GateDenied` (`mesimon gate`'s), `RemotePermission` (`mesimon approve`'s)
/// or [`MOD_PONG`]. The daemon's unit test holds this list to the hook set's
/// and to the mod's `register.ts`.
pub const PAIRED_EVENTS: [&str; 17] = [
    "SessionStart",
    "SessionEnd",
    "StopFailure",
    "PreToolUse",
    "PostToolUse",
    "UserPromptSubmit",
    "Stop",
    "SubagentStart",
    "SubagentStop",
    "TeammateIdle",
    "PermissionRequest",
    "PermissionDenied",
    "Notification",
    "Elicitation",
    "ElicitationResult",
    "PreCompact",
    "PostCompact",
];

/// The event the mod answers a [`ModCommand::Ping`] with, relayed through
/// `mesimon hook --road mod` with the ping's id as its reason: the round trip
/// daemon → bridge → mod → hook.sock, whole.
pub const MOD_PONG: &str = "ModPong";

/// What a `PreToolUse` frame is compared by. The mod's `classic.PreToolUse`
/// is the tool envelope (`{ tool, tool_use_id, ...arguments }`), not the
/// command hook's stdin (T-573 row 1), so the mod rebuilds these three and
/// the shadow reads both roads through the same three.
pub fn pre_tool_use_projection(payload: &Value) -> Value {
    serde_json::json!({
        "tool_name": payload.get("tool_name").cloned().unwrap_or(Value::Null),
        "tool_use_id": payload.get("tool_use_id").cloned().unwrap_or(Value::Null),
        "tool_input": payload.get("tool_input").cloned().unwrap_or(Value::Null),
    })
}

/// A command the daemon addresses to one session's mod. Phase 1 has the
/// ping alone; prompt delivery (`submit`, T-575) and the question's answer
/// (`answer`, T-576) are the next phases'. Every `submit` will be
/// `asUser: true` and nothing here may carry words for the model to read
/// that the person did not write (README promise 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModCommand {
    Ping,
}

/// One line the bridge prints: the command and its id. The id is what the
/// bridge acks and what the mod drops a re-sent duplicate by; delivery is at
/// least once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModFrame {
    pub id: String,
    #[serde(flatten)]
    pub command: ModCommand,
}

/// The bridge exits with this when the daemon refused it for good (the
/// session is unknown or gone, the pane is not its own, another bridge took
/// the seat); the mod does not respawn a bridge that exited so.
pub const BRIDGE_REFUSED_EXIT: i32 = 3;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_is_its_first_three_numbers() {
        assert_eq!(parse_claude_version("2.1.287 (Claude Code)"), Some((2, 1, 287)));
        assert_eq!(parse_claude_version("2.1.287\n"), Some((2, 1, 287)));
        assert_eq!(parse_claude_version("10.0.1"), Some((10, 0, 1)));
        assert_eq!(parse_claude_version("Claude Code 2.1.287"), None);
        assert_eq!(parse_claude_version("2.1"), None);
        assert_eq!(parse_claude_version("2.1.287.4"), None);
        assert_eq!(parse_claude_version(""), None);
    }

    #[test]
    fn the_floor_is_2_1_287() {
        assert!(has_mods((2, 1, 287)));
        assert!(has_mods((2, 1, 300)));
        assert!(has_mods((2, 2, 0)));
        assert!(has_mods((3, 0, 0)));
        assert!(!has_mods((2, 1, 286)));
        assert!(!has_mods((2, 0, 999)));
        assert!(!has_mods((1, 9, 999)));
    }

    #[test]
    fn the_seam_words_round_trip_and_the_road_is_auto() {
        for p in RoadPref::ALL {
            assert_eq!(RoadPref::from_word(p.word()), Some(p));
        }
        assert_eq!(RoadPref::from_word("Mod"), None);
        assert_eq!(RoadPref::default(), RoadPref::Auto);
    }

    #[test]
    fn a_header_road_reads_hooks_when_absent_and_drops_a_foreign_word() {
        assert_eq!(Road::from_header(None), Some(Road::Hooks));
        assert_eq!(Road::from_header(Some("hooks")), Some(Road::Hooks));
        assert_eq!(Road::from_header(Some("mod")), Some(Road::Mod));
        assert_eq!(Road::from_header(Some("mcp")), None);
    }

    #[test]
    fn a_frame_is_one_flat_object_with_its_kind() {
        let f = ModFrame { id: "01J".into(), command: ModCommand::Ping };
        assert_eq!(serde_json::to_string(&f).unwrap(), r#"{"id":"01J","kind":"ping"}"#);
        let back: ModFrame = serde_json::from_str(r#"{"kind":"ping","id":"x"}"#).unwrap();
        assert_eq!(back.command, ModCommand::Ping);
    }

    #[test]
    fn the_projection_keeps_three_fields_and_nothing_else() {
        let hook = serde_json::json!({
            "session_id": "s", "transcript_path": "/t", "cwd": "/c",
            "hook_event_name": "PreToolUse", "tool_name": "AskUserQuestion",
            "tool_use_id": "toolu_1", "tool_input": { "questions": [] },
        });
        let modded = serde_json::json!({
            "hook_event_name": "PreToolUse", "tool_name": "AskUserQuestion",
            "tool_use_id": "toolu_1", "tool_input": { "questions": [] },
        });
        assert_eq!(pre_tool_use_projection(&hook), pre_tool_use_projection(&modded));
        let other = serde_json::json!({ "tool_name": "AskUserQuestion", "tool_use_id": "toolu_2" });
        assert_ne!(pre_tool_use_projection(&hook), pre_tool_use_projection(&other));
    }
}
