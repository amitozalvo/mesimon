//! Deterministic observation replay. No processes, network, model calls, or I/O.
//! Raw hooks pass through the production ingest adapter; transcript records pass
//! through the production classifier. Timing is explicit in the fixture.

use mesimon_core::adopt::{classify_tail_record, TailEvent, TailTool, ToolLedger};
use mesimon_core::attention::{Machine, Signal, TailHint};
use mesimon_core::board::{template_settings, Confidence, SessionState};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ingest::{signal_of, HookFrame};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub schema: u32,
    pub id: String,
    pub title: String,
    pub provenance: Value,
    pub initial: SessionState,
    #[serde(default)]
    pub confidence: Confidence,
    pub column: String,
    pub steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub at_ms: u64,
    pub input: Input,
    pub expect: Expected,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum Input {
    Hook {
        event: String,
        #[serde(default)]
        reason: Option<String>,
        #[serde(default)]
        payload: Value,
    },
    Transcript {
        record: Value,
    },
    /// A probe whose freshness/identity checks have already passed. Does not
    /// claim to test file discovery: that is interrupt_status_e2e's job.
    StatusIdle {
        turn_done: bool,
    },
    PermissionResumed,
    PaneQuiet,
    TranscriptQuiet,
    Tick,
    Restart,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expected {
    pub state: SessionState,
    pub column: String,
    #[serde(default)]
    pub confidence: Option<Confidence>,
    #[serde(default)]
    pub pending: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub id: String,
    pub passed: bool,
    pub failures: Vec<String>,
    pub timeline: Vec<Value>,
}

pub fn tail_signal(record: &Value) -> Option<Signal> {
    let kind = match classify_tail_record(record) {
        TailEvent::AssistantText { .. } => TailHint::AssistantText,
        TailEvent::ToolInFlight => TailHint::ToolInFlight,
        TailEvent::NeedsHuman { tool: TailTool::AskUserQuestion } => TailHint::AskUserQuestion,
        TailEvent::NeedsHuman { tool: TailTool::ExitPlanMode } => TailHint::ExitPlanMode,
        TailEvent::TurnComplete => TailHint::TurnComplete,
        TailEvent::Aborted => TailHint::AbortedMidStream,
        TailEvent::Latch | TailEvent::Other => return None,
    };
    Some(Signal::TranscriptHint { kind })
}

pub fn replay(scenario: &Scenario) -> anyhow::Result<Report> {
    anyhow::ensure!(scenario.schema == 1, "unsupported scenario schema {}", scenario.schema);
    anyhow::ensure!(!scenario.steps.is_empty(), "scenario has no assertions");
    anyhow::ensure!(scenario.provenance.is_object(), "scenario requires provenance");
    let mut machine = Machine::restore(scenario.initial.clone(), scenario.confidence, 0);
    let mut column = scenario.column.clone();
    let mut tools = ToolLedger::default();
    let mut report =
        Report { id: scenario.id.clone(), passed: true, failures: vec![], timeline: vec![] };
    let mut last_at = 0;
    for (index, step) in scenario.steps.iter().enumerate() {
        anyhow::ensure!(step.at_ms >= last_at, "step {index}: arrival time runs backwards");
        last_at = step.at_ms;
        let signal = match &step.input {
            Input::Hook { event, reason, payload } => signal_of(&HookFrame {
                session: "lab-session".into(),
                event: event.clone(),
                reason: reason.clone(),
                payload: payload.clone(),
            }),
            Input::Transcript { record } => {
                tools.observe(record);
                tail_signal(record)
            }
            Input::StatusIdle { turn_done } => {
                Some(Signal::StatusFileIdle { turn_done: *turn_done })
            }
            Input::PermissionResumed => Some(Signal::StatusFilePermissionResumed),
            Input::PaneQuiet => Some(Signal::PaneQuiet),
            Input::TranscriptQuiet if !tools.is_busy() => {
                Some(Signal::TranscriptHint { kind: TailHint::StaleQuiet })
            }
            _ => None,
        };
        let (change, decision) = if let Some(signal) = signal {
            let (change, decision) = machine.apply_explained(&signal, step.at_ms);
            (change, serde_json::to_value(decision)?)
        } else if matches!(step.input, Input::Tick) {
            (machine.tick(step.at_ms), json!({"outcome": "clock_tick"}))
        } else if matches!(step.input, Input::Restart) {
            machine = Machine::restore(SessionState::unknown(), Confidence::Low, step.at_ms);
            (None, json!({"outcome": "restart"}))
        } else {
            (
                None,
                json!({"outcome": if tools.is_busy() { "outstanding_tool" } else { "no_state_evidence" }}),
            )
        };
        let mut movement = json!({"outcome": "no_committed_transition"});
        if let Some(change) = change {
            if let Some(settings) = template_settings(&column) {
                let explanation =
                    mesimon_core::automove::explain(&settings, &change.to, change.confidence);
                movement = serde_json::to_value(&explanation)?;
                if let Some(destination) = explanation.destination {
                    column = destination.to_string();
                }
            } else {
                movement = json!({"outcome": "no_column_rule"});
            }
        }
        let view = machine.view();
        if view.state != step.expect.state
            || column != step.expect.column
            || step.expect.confidence.is_some_and(|c| view.confidence != c)
            || step.expect.pending.is_some_and(|p| view.pending.is_some() != p)
        {
            report.failures.push(format!("step {index} at {}ms: expected {:?} in {} ({:?}); got {:?} in {} ({:?}), pending={}",
                step.at_ms, step.expect.state, step.expect.column, step.expect.confidence,
                view.state, column, view.confidence, view.pending.is_some()));
        }
        report.timeline.push(json!({"step": index, "at_ms": step.at_ms, "machine": view,
            "column": column, "decision": decision, "movement": movement, "outstanding_tool": tools.is_busy()}));
    }
    report.passed = report.failures.is_empty();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_in_scenarios_match_production_decisions() {
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/state-scenarios");
        let mut count = 0;
        for entry in std::fs::read_dir(root).expect("scenario directory") {
            let path = entry.expect("entry").path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let bytes = std::fs::read(&path).expect("fixture");
            let scenario: Scenario = serde_json::from_slice(&bytes).expect("valid scenario");
            let result = replay(&scenario).expect("replay");
            assert!(result.passed, "{}: {:?}", path.display(), result.failures);
            count += 1;
        }
        assert!(count >= 20, "the state map must retain its coverage");
    }
}
