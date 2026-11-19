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

#[derive(Default)]
pub(super) struct Reads([Option<(ReadKey, Receiver<Result<Response>>)>; 4]);

pub(super) struct FocusAsk {
    target: FocusTarget,
    screen: Screen,
    subject: Option<ulid::Ulid>,
    scope: Scope,
    phase: Phase,
    rx: Receiver<Result<Response>>,
}
enum Phase {
    Gate(Command),
    Attach,
    Release,
}

impl App {
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
                return Some(match result {
                    Ok(response) => response,
                    Err(e) => {
                        self.note_daemon_down(crate::client::outage(&e));
                        Response::Err { message: self.status.clone() }
                    }
                });
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
        self.background_reads.0[slot] =
            Some((key.clone(), self.client.read_detached(command.clone())));
        // Fakes and already available answers preserve immediate cached frames.
        let (_, rx) = self.background_reads.0[slot].as_ref().expect("just asked");
        if let Ok(result) = rx.try_recv() {
            self.background_reads.0[slot] = None;
            return Some(match result {
                Ok(response) => response,
                Err(e) => {
                    self.note_daemon_down(crate::client::outage(&e));
                    Response::Err { message: self.status.clone() }
                }
            });
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

    pub(super) fn begin_focus(&mut self, target: FocusTarget, grant: Command) {
        if self.focus_ask.is_some() {
            return;
        }
        self.focus_ask = Some(FocusAsk {
            target,
            screen: self.screen.clone(),
            subject: self.subject(),
            scope: self.scope(),
            phase: Phase::Gate(grant),
            rx: self.client.request_async(Command::GateStatus),
        });
        self.poll_focus();
    }

    pub(super) fn poll_focus(&mut self) -> bool {
        let mut changed = false;
        // At most gate, attachment and cancellation. Immediate fakes retain
        // the old behavior; a real delayed reply returns to the event loop.
        for _ in 0..3 {
            let Some(ask) = &self.focus_ask else { break };
            let result = match ask.rx.try_recv() {
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
                    ask.rx = self.client.request_async(grant);
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
                    ask.rx = self.client.request_async(release);
                    self.focus_ask = Some(ask);
                }
                (_, Response::Err { message }) => self.status = message,
                _ => {}
            }
        }
        changed
    }
}
