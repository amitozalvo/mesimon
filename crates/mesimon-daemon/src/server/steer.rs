//! A running Codex turn takes a paste only while its original pane and
//! receipt still belong to the send. The caller waits for the paste result.
use super::*;
use crate::agents::codex::{self, SteerSight};
use std::sync::mpsc::Receiver;

pub(super) struct Attempt {
    session: uuid::Uuid,
    receipt: ulid::Ulid,
    result: Sender<Result<(), String>>,
}

/// Use tmux's immutable pane id for the keys, even when a wake reuses the
/// session name. Older session records learn the current key on the worker.
pub(super) fn target(
    backend: &TmuxBackend,
    sid: &str,
    expected: Option<&str>,
) -> Result<String, String> {
    let key = backend.pane_key(sid).map_err(|e| format!("could not deliver: {e}"))?;
    if expected.is_some_and(|prev| prev != key) {
        return Err("the pane changed before delivery".into());
    }
    key.split_once(':')
        .map(|(_, pane)| pane.to_owned())
        .filter(|s| s.starts_with('%'))
        .ok_or_else(|| "could not identify the pane for delivery".into())
}

pub(super) type AfterPaste = Box<dyn FnOnce(&mut Daemon, bool) + Send>;

/// Ask again after a slow read. A revoked phone grant or a new receipt
/// ends this dispatch even if its worker still has the old pane's screen.
pub(super) fn check(tx: &Sender<Msg>, id: uuid::Uuid, receipt: ulid::Ulid) -> Result<(), String> {
    let (ask, permit) = channel();
    tx.send(Msg::SteerCheck(id, receipt, ask)).map_err(|_| "daemon gone".to_string())?;
    permit.recv().map_err(|_| "daemon gone".to_string())?
}

impl Daemon {
    pub(super) fn checking_steer(&self, ticket: ulid::Ulid) -> Option<ulid::Ulid> {
        let owed = self.owed.get(&self.prompt_target(ticket)?)?;
        (owed.steer.is_some() && owed.sent_at.is_none()).then_some(owed.receipt_id)
    }

    /// Success bookkeeping for a send accepted by the dispatch. A pending
    /// steer earns it only when its paste lands, as a synchronous send did.
    pub(super) fn after_paste(
        &mut self,
        ticket: ulid::Ulid,
        apply: impl FnOnce(&mut Daemon, bool) + Send + 'static,
    ) {
        if let Some(receipt) = self.checking_steer(ticket) {
            self.steer_effects.entry(receipt).or_default().push(Box::new(apply));
        } else {
            apply(self, true);
        }
    }

    pub(super) fn codex_steer(
        &mut self,
        id: uuid::Uuid,
        ticket: ulid::Ulid,
        text: &str,
        ack: Ack,
        from: u64,
    ) -> Result<(), String> {
        let rec = self
            .board
            .session(id)
            .ok_or("no live agent session on this ticket — start or wake one first")?;
        let sid = rec.sid16();
        let key = rec.pane_key.clone();
        let now = now_ms();
        let owed = Owed {
            presses: SUBMIT_ATTEMPTS,
            expires: None,
            sent_at: None,
            steer: Some(from),
            sent: Some(Parked { text: text.to_owned(), brief: false, title: false }),
            ..Owed::pasted(ticket, ack, now)
        };
        let receipt = owed.receipt_id;
        self.owed.insert(id, owed);
        let (result, rx) = channel();
        if let Some(calls) = &mut self.steer_calls {
            calls.push(rx);
        }
        let job = Attempt { session: id, receipt, result };
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let words = text.to_owned();
        std::thread::spawn(move || {
            let outcome = (|| {
                let pane = target(&backend, &sid, key.as_deref())?;
                let sight = codex::steer_sight(|i| {
                    if i > 0 {
                        std::thread::sleep(Duration::from_millis(codex::STEER_READ_GAP_MS));
                    }
                    check(&tx, id, receipt)?;
                    backend
                        .capture_input_screen(&pane)
                        .map_err(|e| format!("could not deliver: {e}"))
                })?;
                match sight {
                    SteerSight::Composer => {
                        check(&tx, id, receipt)?;
                        if target(&backend, &sid, key.as_deref())? != pane {
                            return Err("the pane changed before delivery".into());
                        }
                        backend
                            .paste_input(&pane, &words)
                            .map_err(|e| format!("could not deliver: {e}"))
                    }
                    SteerSight::Dialog => Err(mesimon_core::command::ANSWER_IN_PANE_FIRST.into()),
                    SteerSight::Unseen => Err(mesimon_core::command::COMPOSER_NOT_FOUND.into()),
                }
            })();
            let _ = tx.send(Msg::SteerPasted(job, outcome));
        });
        Ok(())
    }

    pub(super) fn steer_allowed(
        &mut self,
        id: uuid::Uuid,
        receipt: ulid::Ulid,
    ) -> Result<(), String> {
        if !self.owed.get(&id).is_some_and(|o| o.receipt_id == receipt)
            || !self.control_delivery_allowed(id)
        {
            return Err("the prompt is no longer pending for this pane".into());
        }
        let rec = self
            .board
            .session(id)
            .filter(|r| r.state.has_pane() && !r.codex_stopping)
            .ok_or("the pane changed before delivery")?;
        if matches!(rec.state, SessionState::RequiresAction { .. }) {
            return Err(mesimon_core::command::ANSWER_IN_PANE_FIRST.into());
        }
        Ok(())
    }

    pub(super) fn steer_pasted(&mut self, job: Attempt, result: Result<(), String>) {
        let id = job.session;
        for apply in self.steer_effects.remove(&job.receipt).unwrap_or_default() {
            apply(self, result.is_ok());
        }
        let Some(owed) = self.owed.get_mut(&id).filter(|o| o.receipt_id == job.receipt) else {
            let _ = job.result.send(result);
            return;
        };
        match &result {
            Ok(()) => {
                let now = now_ms();
                owed.sent_at = Some(now);
                owed.screen_after = now;
                owed.next_press = Some(now + SUBMIT_RETRY_MS);
                self.control_pasted(id);
            }
            Err(message) => {
                let ticket = owed.ticket;
                if let Some(ask) = owed.ask {
                    self.crown_ask_unknown(ticket, Some(ask), message);
                }
                self.control_unsent(id);
                self.drop_owed(id);
                self.feed.board_outcome("daemon", "prompt_not_sent", Some(ticket), message);
            }
        }
        self.broadcast();
        let _ = job.result.send(result);
    }
}

pub(super) fn reply_after_pastes(
    calls: Vec<Receiver<Result<(), String>>>,
    saved: Result<(), String>,
    mut response: Response,
    told: Option<(uuid::Uuid, u64)>,
    reply: Sender<ClientReply>,
) {
    std::thread::spawn(move || {
        for call in calls {
            if let Err(message) = call.recv().unwrap_or_else(|_| Err("daemon gone".into())) {
                let body = match &mut response {
                    Response::CrownRules { inner, .. } => inner.as_mut(),
                    body => body,
                };
                match body {
                    Response::AgentMerged { notice, .. } => *notice = "failed".into(),
                    Response::Merge { notified, .. } => *notified = false,
                    Response::Asked { sent, failed, .. } => {
                        *sent = sent.saturating_sub(1);
                        *failed += 1;
                    }
                    _ => *body = Response::Err { message },
                }
            }
        }
        let (response, told) = match saved {
            Ok(()) => (response, told),
            Err(message) => (Response::Err { message }, None),
        };
        let _ = reply.send(ClientReply { response, told, delivered: None });
    });
}
