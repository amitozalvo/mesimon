//! Bounded background reads and connection-owned attachment preparation.
use super::*;

#[derive(Clone, PartialEq, Eq)]
pub(super) enum ReadKey {
    Note(ulid::Ulid, ulid::Ulid, u64),
    Archive(u64),
    Held(ulid::Ulid, u64),
    Tail(TailKey),
}
impl ReadKey {
    fn slot(&self) -> usize {
        match self {
            Self::Note(..) => 0,
            Self::Archive(..) => 1,
            Self::Held(..) => 2,
            Self::Tail(..) => 3,
        }
    }
}

/// The status when `App::submission_pending` keeps a draft.
pub(super) const SUBMISSION_PENDING: &str = "previous submission is still pending ∙ draft kept";

#[derive(Default)]
pub(super) struct Reads([Option<(ReadKey, Receiver<Result<Response>>)>; 4]);

/// One worker, with duplicate acknowledgements coalesced until the board
/// reflects them. Navigation never waits for these incidental writes.
#[derive(Default)]
pub(super) struct Acknowledgements {
    queued: std::collections::VecDeque<(ulid::Ulid, bool)>,
    sent: std::collections::HashMap<(ulid::Ulid, bool), String>,
    active: Option<PendingAcknowledgement>,
}

struct PendingAcknowledgement {
    key: (ulid::Ulid, bool),
    rx: Receiver<Result<Response>>,
}

type FinishSubmission = Box<dyn FnOnce(&mut App, Response) -> Result<()>>;

pub(super) struct Submission {
    rx: Receiver<Result<Response>>,
    finish: FinishSubmission,
}

pub(super) struct CreatedFollow {
    pub id: ulid::Ulid,
    pub start: Option<bool>,
    pub generation: u64,
    pub after: u64,
}

pub(super) struct FocusAsk {
    pub(super) started: Instant,
    target: FocusTarget,
    screen: Screen,
    preview: Option<std::rc::Rc<crate::live::Shot>>,
    subject: Option<ulid::Ulid>,
    scope: Scope,
    phase: Phase,
    rx: Option<Receiver<Result<Response>>>,
}
enum Phase {
    Gate(Command),
    Attach,
    Release,
}

impl App {
    /// Keep the picture that was on the page when Enter stopped its reader.
    /// A different rail row or ticket must never inherit that picture.
    pub(super) fn focus_preview(
        &self,
        session: uuid::Uuid,
    ) -> Option<std::rc::Rc<crate::live::Shot>> {
        let ask = self.focus_ask.as_ref()?;
        if ask.screen == self.screen
            && ask.target == FocusTarget::Session(session, FocusOrigin::Ticket)
        {
            ask.preview.clone()
        } else {
            None
        }
    }

    pub(super) fn submit(
        &mut self,
        command: Command,
        finish: impl FnOnce(&mut App, Response) -> Result<()> + 'static,
    ) -> Result<()> {
        debug_assert!(self.submission.is_none());
        let rx = self.client.request_detached(command);
        self.submit_reply(rx, finish)
    }

    pub(super) fn submit_reply(
        &mut self,
        rx: Receiver<Result<Response>>,
        finish: impl FnOnce(&mut App, Response) -> Result<()> + 'static,
    ) -> Result<()> {
        self.submission = Some(Submission { rx, finish: Box::new(finish) });
        self.poll_submission()?;
        Ok(())
    }

    pub(super) fn poll_submission(&mut self) -> Result<bool> {
        let Some(submission) = self.submission.take() else { return Ok(false) };
        let result = match submission.rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => {
                self.submission = Some(submission);
                return Ok(false);
            }
            Err(TryRecvError::Disconnected) => Err(anyhow::anyhow!("submission reply lost")),
        };
        let response = self.answer(result);
        (submission.finish)(self, response)?;
        Ok(true)
    }

    /// A reply as the board reads it: a transport failure becomes an
    /// ordinary Err response carrying the outage status, and flags the
    /// reconnect cadence.
    pub(super) fn answer(&mut self, result: Result<Response>) -> Response {
        match result {
            Ok(response) => response,
            Err(e) => {
                self.note_daemon_down(crate::client::outage(&e));
                Response::Err { message: self.status.clone() }
            }
        }
    }

    /// A save, send or file that would race the one still on the wire, or
    /// the draft a reply is holding back, keeps its draft instead
    /// (`SUBMISSION_PENDING`).
    pub(super) fn submission_pending(&self) -> bool {
        self.submission.is_some() || self.created_follow.is_some() || self.write_recovery.is_some()
    }

    pub(super) fn recover_submission(&mut self, mode: Mode) {
        if matches!(self.mode, Mode::Normal) {
            self.mode = mode;
        } else {
            // A reply must never overwrite words typed after submission.
            self.write_recovery = Some(mode);
        }
    }

    pub(super) fn poll_created(&mut self) -> Result<bool> {
        let Some(follow) = self.created_follow.as_ref() else { return Ok(false) };
        if self.board.ticket(follow.id).is_none() {
            if self.landing >= follow.after {
                self.created_follow = None;
                self.status = "saved ticket is no longer on the board".into();
                return Ok(true);
            }
            return Ok(false);
        }
        let follow = self.created_follow.take().expect("follow was checked above");
        if self.input_generation == follow.generation {
            self.select_ticket(follow.id);
            if follow.start.is_none() {
                self.just_created = Some(follow.id);
                self.status =
                    format!("enter starts {} ∙ space opens the ticket", keymap::AGENT_WORD);
            }
        }
        if let Some(plan) = follow.start {
            self.start_composed(follow.id, plan);
        }
        Ok(true)
    }

    pub(super) fn acknowledge(&mut self, id: ulid::Ulid, hand: bool) -> Result<()> {
        let stamp = self.board.ticket(id).and_then(|t| {
            if hand {
                t.raised.as_ref().map(|r| r.at.clone())
            } else {
                t.woke_at.clone()
            }
        });
        let Some(stamp) = stamp else { return Ok(()) };
        if self.acknowledgements.sent.get(&(id, hand)) != Some(&stamp) {
            self.acknowledgements.sent.insert((id, hand), stamp);
            if !self.acknowledgements.queued.contains(&(id, hand)) {
                self.acknowledgements.queued.push_back((id, hand));
            }
        }
        self.poll_acknowledgements()?;
        Ok(())
    }

    pub(super) fn poll_acknowledgements(&mut self) -> Result<bool> {
        let mut dirty = false;
        loop {
            if self.acknowledgements.active.is_none() {
                let Some((id, hand)) = self.acknowledgements.queued.pop_front() else { break };
                let command =
                    if hand { Command::LowerHand { id } } else { Command::SeenTicket { id } };
                self.acknowledgements.active = Some(PendingAcknowledgement {
                    key: (id, hand),
                    rx: self.client.request_detached(command),
                });
            }
            let PendingAcknowledgement { key, rx } =
                self.acknowledgements.active.as_ref().expect("an acknowledgement was started");
            let key = *key;
            let result = match rx.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => Err(anyhow::anyhow!("acknowledgement lost")),
            };
            self.acknowledgements.active = None;
            match result {
                Ok(Response::Err { message }) => {
                    self.acknowledgements.sent.remove(&key);
                    self.status = message;
                }
                Err(e) => {
                    self.acknowledgements.sent.remove(&key);
                    self.note_daemon_down(crate::client::outage(&e));
                }
                _ => {}
            }
            // No automatic replay. A later key may acknowledge the mark again.
            self.refresh_soon()?;
            dirty = true;
        }
        self.acknowledgements.sent.retain(|(id, hand), stamp| {
            self.board.ticket(*id).is_some_and(|t| {
                let current =
                    if *hand { t.raised.as_ref().map(|r| &r.at) } else { t.woke_at.as_ref() };
                current == Some(stamp)
            })
        });
        Ok(dirty)
    }

    /// One request per category, including obsolete requests still on the wire.
    /// A changed selection never creates an unbounded queue of worker threads.
    pub(super) fn background_read(&mut self, key: ReadKey, command: Command) -> Option<Response> {
        let slot = key.slot();
        if let Some((asked, rx)) = &self.background_reads.0[slot] {
            let result = match rx.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => Err(anyhow::anyhow!("background read lost")),
            };
            let current = asked == &key;
            let prior = asked.clone();
            self.background_reads.0[slot] = None;
            if current {
                return Some(self.answer(result));
            }
            // A note for a previous selection still belongs in the cache if
            // its revision is current. Otherwise summaries and the visible
            // note could keep discarding each other's completed reads.
            if let (ReadKey::Note(ticket, id, rev), Ok(Response::Note { text, meta })) =
                (prior, result)
            {
                if meta.id == id
                    && meta.rev == rev
                    && self
                        .board
                        .ticket(ticket)
                        .is_some_and(|t| t.notes.iter().any(|n| n.id == id && n.rev == rev))
                {
                    self.remember_note(id, rev, Some(crate::peek::sanitize(&text)));
                }
            }
        }
        self.background_reads.0[slot] = Some((key, self.client.read_detached(command)));
        // Fakes and already available answers preserve immediate cached frames.
        let (_, rx) = self.background_reads.0[slot].as_ref().expect("just asked");
        if let Ok(result) = rx.try_recv() {
            self.background_reads.0[slot] = None;
            return Some(self.answer(result));
        }
        None
    }

    pub(super) fn poll_note(&mut self, ticket: ulid::Ulid, id: ulid::Ulid, rev: u64) -> bool {
        let Some(response) = self.background_read(
            ReadKey::Note(ticket, id, rev),
            Command::ReadNote { ticket, note: id },
        ) else {
            return false;
        };
        let text = match response {
            Response::Note { text, meta } if meta.id == id && meta.rev == rev => {
                Some(crate::peek::sanitize(&text))
            }
            // A newer revision will arrive on the snapshot; never stamp newer
            // text as the older version that was requested.
            Response::Note { .. } => return false,
            _ => None,
        };
        self.remember_note(id, rev, text);
        true
    }

    /// The focus road every attach takes: GATE (D20) — prove the unfocus key
    /// once before the first real focus — then the command that grants the
    /// attach argv, parked for the main loop's handover with the target it
    /// holds the token on, so `after_handover` knows what to release.
    pub(super) fn begin_focus(&mut self, target: FocusTarget, grant: Command) {
        if self.focus_ask.is_some() {
            return;
        }
        let preview = match target {
            FocusTarget::Session(id, FocusOrigin::Ticket) => self.live_shot(id),
            _ => None,
        };
        self.park_live();
        self.focus_ask = Some(FocusAsk {
            started: Instant::now(),
            target,
            screen: self.screen.clone(),
            preview,
            subject: self.subject(),
            scope: self.scope(),
            phase: Phase::Gate(grant),
            rx: None,
        });
        self.poll_focus();
    }

    pub(super) fn poll_focus(&mut self) -> bool {
        let mut changed = false;
        // At most gate, attachment and cancellation. Immediate fakes retain
        // the old behavior; a real delayed reply returns to the event loop.
        for _ in 0..3 {
            if self.focus_ask.as_ref().is_some_and(|ask| ask.rx.is_none()) {
                if !self.live_stopped() {
                    break;
                }
                let rx = self.client.request_async(Command::GateStatus);
                self.focus_ask.as_mut().expect("pending above").rx = Some(rx);
            }
            let Some(ask) = &self.focus_ask else { break };
            let result = match ask.rx.as_ref().expect("started above").try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => Err(anyhow::anyhow!("attachment request lost")),
            };
            let mut ask = self.focus_ask.take().expect("pending above");
            changed = true;
            let current = self.screen == ask.screen
                && self.subject() == ask.subject
                && self.scope() == ask.scope
                && !self.quit
                && !self.pending_suspend;
            let response = match result {
                Ok(response) => response,
                Err(e) => {
                    self.note_daemon_down(crate::client::outage(&e));
                    break;
                }
            };
            match (ask.phase, response) {
                (Phase::Gate(grant), Response::Gate { passed: true, .. }) if current => {
                    ask.phase = Phase::Attach;
                    ask.rx = Some(self.client.request_async(grant));
                    self.focus_ask = Some(ask);
                }
                (Phase::Gate(_), Response::Gate { passed: false, attach_argv: Some(argv) })
                    if current =>
                {
                    self.pending_attach = Some(self.client_argv(argv));
                    self.pending_gate_then = Some(ask.target);
                }
                (Phase::Attach, Response::Attach { argv }) if current => {
                    self.pending_attach = Some(self.client_argv(argv));
                    self.focused_session_hint = Some(ask.target);
                }
                (Phase::Attach, Response::Attach { .. }) => {
                    let release = match ask.target {
                        FocusTarget::Session(session, _) => Command::FocusEnd { session },
                        FocusTarget::Terminal => Command::TerminalEnd,
                    };
                    ask.phase = Phase::Release;
                    ask.rx = Some(self.client.request_async(release));
                    self.focus_ask = Some(ask);
                }
                (_, Response::Err { message }) => {
                    self.status = message;
                    if current {
                        if let FocusTarget::Session(sid, FocusOrigin::Board) = ask.target {
                            if let Some(rec) = self.board.session(sid) {
                                let ticket = rec.ticket;
                                let rail_idx = self
                                    .rail_sessions(ticket)
                                    .iter()
                                    .position(|s| s.id == sid)
                                    .unwrap_or(0);
                                self.screen = Screen::Ticket { ticket, rail_idx };
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        changed
    }
}
