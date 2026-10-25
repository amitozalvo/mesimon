//! The crown's sample of an agent it started (T-973): `sample_agent`.
//!
//! What the board reads of a worker at the call, facts first, then text,
//! each bounded: how long its turn has run (`Daemon::turn_since`), the tool
//! in flight and its last tools and text (the transcript's tail,
//! `tail::sample`), its background tasks (the linger wake's numbers) and its
//! pane's last rows. The pane and the transcript are read off the writer
//! thread, on a worker of their own (T-857), and the call's reply waits for
//! them parked, as `accept_plan`'s does for its press. Nothing reaches the
//! agent, and the daemon judges nothing: the reading is the crown's to read.
//!
//! Crown only, behind the board's `crown_samples`, and only an agent the
//! crown started (`Action::SampleAgent`): a person's agent, watched or not,
//! and the crown's own are refused.

use super::*;
use crate::agents::claude::tail::{self, TailSample};
use mesimon_core::command::{AgentSampleView, AgentToolView};

/// How many of the pane's last rows a sample carries.
const PANE_ROWS: usize = 40;
/// And at most how many bytes of them, the newest kept.
const PANE_BYTES: usize = 4 * 1024;
/// At most how many bytes of the agent's last text, its end kept.
const TEXT_BYTES: usize = 1024;

/// A sample on its way: who asked, which seat it reads, and the reply the
/// writer loop parked for it (`sample_park_reply`).
pub(super) struct Sampling {
    session: uuid::Uuid,
    target: ulid::Ulid,
    key: String,
    seat: uuid::Uuid,
    pane_key: Option<String>,
    reply: Option<Sender<ClientReply>>,
}

/// What the worker thread read: the pane's last rows and the transcript's
/// tail, each `None` where there was nothing to read.
pub(super) struct SampleRead {
    at_ms: u64,
    pane: Option<Vec<String>>,
    tail: Option<TailSample>,
}

/// The pane's rows as they leave for another process: scrubbed, the last
/// `PANE_ROWS`, the newest `PANE_BYTES`.
fn pane_tail(rows: &[String]) -> Option<String> {
    let rows: Vec<String> = rows.iter().map(|r| mesimon_core::text::scrub_text(r)).collect();
    let from = rows.len().saturating_sub(PANE_ROWS);
    let text = rows[from..].join("\n");
    let text = mesimon_core::text::tail_bytes(text.trim_end(), PANE_BYTES);
    (!text.trim().is_empty()).then(|| text.to_string())
}

/// The agent's last text as it leaves: scrubbed, its newest `TEXT_BYTES`.
fn last_text(text: &str) -> Option<String> {
    let clean = mesimon_core::text::scrub_lines(text);
    let text = mesimon_core::text::tail_bytes(clean.trim(), TEXT_BYTES).trim();
    (!text.is_empty()).then(|| text.to_string())
}

impl Daemon {
    /// `sample_agent` (T-973): the refusals in order, then the read handed
    /// to a worker thread, the reply parked until it lands.
    pub(super) fn agent_sample(
        &mut self,
        session: uuid::Uuid,
        own: ulid::Ulid,
        key: &str,
    ) -> Response {
        let refused = |d: &mut Daemon, target: Option<ulid::Ulid>, message: String| {
            d.feed.board_outcome("agent", "sample_agent", target, "refused");
            Response::Err { message }
        };
        let target = match self.crown_target(own, key, false) {
            Ok(t) => t,
            Err(message) => return refused(self, None, message),
        };
        if target == own {
            let message = format!(
                "{key} is this session's own ticket; sample_agent is for an agent the crown \
                 started"
            );
            return refused(self, Some(target), message);
        }
        if !self.board.crown_samples {
            let message = format!(
                "Settings → Crown → Samples agents is off; a person turns it on, or {key}'s \
                 card and get_ticket say what the board shows"
            );
            return refused(self, Some(target), message);
        }
        let Some(rec) = self.board.live_agent(target) else {
            let message = format!("{key} has no agent; start_agent starts one");
            return refused(self, Some(target), message);
        };
        let started = rec.started_by.is_some();
        let (seat, kind, pane_key, sid16, transcript) = (
            rec.id,
            rec.kind,
            rec.pane_key.clone(),
            rec.state.has_pane().then(|| rec.sid16()),
            crate::cost::transcript_of(rec).map(|(path, _)| path),
        );
        if !started {
            let message = format!(
                "a person started {key}'s agent, and it is the person's to read, in its pane; \
                 sample_agent is for an agent the crown started"
            );
            return refused(self, Some(target), message);
        }
        let by = Principal::Agent { session };
        let action = Action::SampleAgent { crown: self.board.is_crowned(own) && started };
        if let Decision::Deny { reason } = authorize(&by, &action, &Resource::Session { id: seat })
        {
            return refused(self, Some(target), format!("denied: {reason}"));
        }
        self.sample_seq = self.sample_seq.wrapping_add(1);
        let id = self.sample_seq;
        let key =
            self.board.ticket(target).map_or_else(|| key.to_string(), |t| t.short_key.clone());
        self.samples.insert(
            id,
            Sampling { session, target, key: key.clone(), seat, pane_key, reply: None },
        );
        self.sample_wait = Some(id);
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let claude = kind == SessionKind::Claude;
        std::thread::spawn(move || {
            let pane = sid16.and_then(|sid| backend.capture_tail(&sid, PANE_ROWS).ok());
            let tail = transcript.filter(|_| claude).and_then(|path| tail::sample(&path));
            let read = SampleRead { at_ms: now_ms(), pane, tail };
            let _ = tx.send(Msg::AgentSampled(id, Box::new(read)));
        });
        // Never sent: the writer loop parks the reply on `sample_wait`.
        Response::Ok
    }

    /// The writer's reply for the `sample_agent` call that started sample
    /// `id`: kept with it, answered when the read lands. Handed back when
    /// there is no such sample, to be sent now.
    pub(super) fn sample_park_reply(
        &mut self,
        id: u64,
        reply: Sender<ClientReply>,
    ) -> Option<Sender<ClientReply>> {
        match self.samples.get_mut(&id) {
            Some(s) if s.reply.is_none() => {
                s.reply = Some(reply);
                None
            }
            _ => Some(reply),
        }
    }

    /// The worker's read landed: the facts that live on this thread are read
    /// now, as the read's moment, and the reply goes. A seat that changed
    /// meanwhile (gone, relaunched onto another pane) is refused rather than
    /// answered with another agent's pane.
    pub(super) fn sample_landed(&mut self, id: u64, read: SampleRead) {
        let Some(s) = self.samples.remove(&id) else { return };
        let rec = self
            .board
            .live_agent(s.target)
            .filter(|r| r.id == s.seat && (read.pane.is_none() || r.pane_key == s.pane_key));
        let response = match rec {
            None => {
                self.feed.board_outcome("agent", "sample_agent", Some(s.target), "changed");
                Response::Err {
                    message: format!(
                        "{}'s agent changed during the sample; sample_agent reads it again",
                        s.key
                    ),
                }
            }
            Some(rec) => {
                let state = mesimon_core::board::agent_state_word(&rec.state).to_string();
                let tail = read.tail.unwrap_or_default();
                let sample = AgentSampleView {
                    key: s.key.clone(),
                    state,
                    turn_secs: self
                        .turn_since
                        .get(&s.seat)
                        .map(|at| read.at_ms.saturating_sub(*at) / 1000),
                    current_tool: tail.current_tool.map(|(name, at)| AgentToolView {
                        name: mesimon_core::text::scrub_text(&name),
                        secs: at.map(|at| read.at_ms.saturating_sub(at) / 1000),
                    }),
                    recent_tools: tail
                        .recent_tools
                        .iter()
                        .map(|n| mesimon_core::text::scrub_text(n))
                        .collect(),
                    last_text: tail.last_text.as_deref().and_then(last_text),
                    background: self.agent_background(s.target),
                    pane_tail: read.pane.as_deref().and_then(pane_tail),
                    captured_at_ms: read.at_ms,
                };
                self.feed.board_outcome("agent", "sample_agent", Some(s.target), "ok");
                Response::AgentSample { sample }
            }
        };
        let Some(reply) = s.reply else { return };
        let _ = self.feed.flush();
        let (response, told) = self.with_crown_rules(s.session, response);
        let _ = reply.send(ClientReply { response, delivered: None, told });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pane_tail_keeps_the_last_rows_scrubbed_and_bounded() {
        let rows: Vec<String> = (0..60).map(|i| format!("row {i}\u{1b}[31m")).collect();
        let tail = pane_tail(&rows).unwrap();
        assert!(tail.starts_with("row 20[31m"), "the last forty, escapes gone: {tail}");
        assert!(tail.ends_with("row 59[31m"));
        assert!(!tail.contains('\u{1b}'));
        let wide: Vec<String> = (0..40).map(|_| "é".repeat(200)).collect();
        let tail = pane_tail(&wide).unwrap();
        assert!(tail.len() <= PANE_BYTES && tail.ends_with('é'), "{}", tail.len());
        assert_eq!(pane_tail(&["  ".into(), String::new()]), None, "a blank pane says nothing");
    }

    #[test]
    fn a_last_text_keeps_its_end() {
        let long = format!("{}the end", "x".repeat(2000));
        let text = last_text(&long).unwrap();
        assert!(text.len() <= TEXT_BYTES && text.ends_with("the end"));
        assert_eq!(last_text("two\r\nlines\u{7}").as_deref(), Some("two\nlines"));
        assert_eq!(last_text("  \n "), None);
    }
}
