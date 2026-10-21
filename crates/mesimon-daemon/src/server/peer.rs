//! Boards that reach each other on one machine (T-733): the machine's
//! `links.toml`, the call to another board's daemon, and the two roads words
//! take between two crowns.
//!
//! The author ran several boards at once and carried asks from one crown to
//! the other by hand. Now a person links two boards (Settings → Crown →
//! Reaches boards), and the crown of either calls `ask_board` naming the
//! other. This daemon connects to that board's `orch.sock` — the same socket
//! its own TUI speaks to, keyed by the repository's canonical path
//! (`Paths::for_repo`) — and sends `PeerAsk` as `Principal::Peer { board }`.
//! The receiving daemon admits that one command, from a board the file pairs
//! with its own, and parks the words on its crown's card as the crown's own
//! asks are parked for a worker (`park_ask`): the queue sends them at the
//! crown's idle under autonomous mode, a person's `^y` under supervised, and
//! they go in through the board's `Board ask` template, which names the
//! asking board and its crown's ticket. The reply is the same tool back: a
//! board that holds an open ask from the other reads the next words it sends
//! there as the answer (`asks_in`/`asks_out`), and the receiving side reads
//! them as `got_reply` and renders the `Board reply` template.
//!
//! The call runs on the writer thread, synchronously, with short timeouts:
//! a unix connect to a local daemon answers in microseconds or fails at once
//! (no socket, nobody listening), and a daemon busy for `PEER_WAIT_MS` is
//! the one case that stalls this one, for that long and no longer. An
//! action at an edge stays a fork on the writer (CLAUDE.md); the tick never
//! calls here.
//!
//! What the board shows rides the snapshot: `links` (the partners, by name
//! and path) and `board_touches` (what went where in the last ten seconds),
//! for the header's bolt and its lit word. The feed is the record.

use std::path::Path;

use mesimon_core::boardlink::{self, Links};
use mesimon_core::command::{BoardTouch, LinkView, TouchAction};
use mesimon_core::prompts::AgentPrompt;

use super::*;

/// How long a board touch rides the snapshot: the header's lit word lasts
/// a few seconds, and a board opened after that reads nothing of it.
pub(super) const BOARD_TOUCH_MS: u64 = 10_000;

/// A unix connect to a local daemon: at once, or not at all.
const PEER_CONNECT_MS: u64 = 1_000;
/// How long the other daemon may take to answer one ask.
const PEER_WAIT_MS: u64 = 5_000;

pub(super) type MachineLinksCache = super::tiers::MachineFileCache<Links>;

/// The words a peer ask carries, kept on the queue entry it became: the
/// asking board's name, and the crown's words before the template wrapped
/// them — what the card's line says.
#[derive(Debug, Clone)]
pub(super) struct FromBoard {
    pub(super) name: String,
    pub(super) words: String,
}

/// One request to another board's daemon over its `orch.sock`: connect,
/// one line out, one line back. `Err` is in words a crown can relay, and
/// a board with no daemon listening reads `is not open`.
pub(super) fn call_peer(sock: &Path, env: &Envelope) -> std::result::Result<Response, String> {
    use crate::oneshot::{one_shot, Fault};
    let write = Some(Duration::from_millis(PEER_CONNECT_MS));
    let read = Some(Duration::from_millis(PEER_WAIT_MS));
    one_shot(sock, env, write, read).map_err(|fault| match fault {
        Fault::Connect(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            "is not open; a board opens when mesimon runs in its folder, and the ask can be \
             made again then"
                .to_string()
        }
        Fault::Connect(e) => format!("could not be reached: {e}"),
        Fault::Send(e) => format!("did not take the ask: {e}"),
        Fault::Receive(e) => format!("did not answer in time: {e}"),
        Fault::Closed => "closed the connection without answering".into(),
        Fault::Unreadable(e) => format!("answered in words this build cannot read: {e}"),
    })
}

impl Daemon {
    /// This repository's root as `links.toml` spells it: `Paths::for_repo`
    /// canonicalized it, so two boards agree on each other's name.
    pub(super) fn my_root(&self) -> String {
        self.paths.repo_root.to_string_lossy().into_owned()
    }

    /// Another board may have linked or unlinked this one (T-733): one
    /// `stat`, so what this command reads is current.
    pub(super) fn refresh_links(&mut self) {
        if self.links.refresh() {
            self.broadcast();
        }
    }

    /// The boards this one reaches, for the snapshot, `list_board` and the
    /// header: by name and path, in the file's order.
    pub(super) fn links_view(&self) -> Vec<LinkView> {
        let me = self.my_root();
        self.links
            .value
            .partners(&me)
            .into_iter()
            .map(|path| LinkView { name: boardlink::board_name(&path), path })
            .collect()
    }

    /// `Command::LinkBoard` (T-733): a person links this board with the one
    /// at `path`, for both. The path is taken as typed or picked and
    /// canonicalized here, so the pair is spelled as both daemons spell
    /// their roots.
    pub(super) fn link_board(&mut self, path: String) -> Response {
        let other = match std::fs::canonicalize(path.trim()) {
            Ok(p) => p.to_string_lossy().into_owned(),
            Err(e) => return Response::Err { message: format!("{}: {e}", path.trim()) },
        };
        let me = self.my_root();
        if other == me {
            return Response::Err { message: "a board does not reach itself".into() };
        }
        let mut links = self.links.value.clone();
        if !links.link(&me, &other) {
            return Response::Err {
                message: format!("{} is linked already", boardlink::board_name(&other)),
            };
        }
        if let Err(message) = self.links.save(links) {
            return Response::Err { message };
        }
        self.broadcast();
        Response::Ok
    }

    /// `Command::UnlinkBoard` (T-733): the pair comes off the file, for both
    /// boards. Words already on a crown's card stay where they are.
    pub(super) fn unlink_board(&mut self, path: String) -> Response {
        let other = std::fs::canonicalize(path.trim())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| path.trim().to_string());
        let me = self.my_root();
        let mut links = self.links.value.clone();
        if !links.unlink(&me, &other) {
            return Response::Err {
                message: format!("{} is not linked", boardlink::board_name(&other)),
            };
        }
        if let Err(message) = self.links.save(links) {
            return Response::Err { message };
        }
        self.asks_in.remove(&other);
        self.asks_out.remove(&other);
        self.broadcast();
        Response::Ok
    }

    /// `ask_board` (T-733), from the crown on `ticket`: find the board,
    /// carry the words to its daemon, and record what happened for the card,
    /// the header and the feed. Every refusal is in words the crown relays,
    /// and names the fix a person makes.
    pub(super) fn agent_ask_board(
        &mut self,
        ticket: ulid::Ulid,
        board: String,
        text: String,
    ) -> Response {
        if !self.board.is_crowned(ticket) {
            return Response::Err { message: self.crown_refusal(ticket) };
        }
        let me = self.my_root();
        self.links.refresh();
        let other = match self.links.value.find(&me, &board) {
            Ok(p) => p,
            Err(message) => return Response::Err { message },
        };
        let name = boardlink::board_name(&other);
        // Sanitized by subtraction alone, as the person's own words are.
        let Some(text) = mesimon_core::command::sanitize_prompt(&text) else {
            return Response::Err { message: "nothing to send".into() };
        };
        // A shell line is a person's alone (T-876), on either board.
        if mesimon_core::road::is_shell(&text) {
            return Response::Err { message: mesimon_core::road::SHELL_LINE_REFUSED.into() };
        }
        let (crown_key, crown_title) = match self.board.ticket(ticket) {
            Some(t) => (t.short_key.clone(), t.title.clone()),
            None => return Response::Err { message: "no such ticket".into() },
        };
        let sock = match Paths::for_repo(Path::new(&other)) {
            Ok(p) => p.orch_sock(),
            Err(e) => return Response::Err { message: format!("{name}: {e}") },
        };
        let env = Envelope {
            principal: Principal::Peer { board: me },
            command: Command::PeerAsk { crown_key, crown_title, text: text.clone() },
        };
        let answer = match call_peer(&sock, &env) {
            Ok(Response::PeerAsked { road, crown, .. }) => Ok((road, crown)),
            Ok(Response::Err { message }) => Err(message),
            Ok(_) => Err("answered in words this build cannot read".to_string()),
            Err(why) => Err(why),
        };
        match answer {
            Err(why) => {
                self.board_touched(&other, TouchAction::Refused, Some(why.clone()));
                self.feed.board_outcome("agent", "ask_board", Some(ticket), "refused");
                self.broadcast();
                Response::Err { message: format!("{name} {why}") }
            }
            Ok((road, crown)) => {
                // Answering an ask that board made here is a reply; the
                // next words from there answer this one.
                let reply = self.asks_in.remove(&other).is_some();
                if !reply {
                    self.asks_out.insert(other.clone(), now_ms());
                }
                self.board_touched(
                    &other,
                    if reply { TouchAction::Replied } else { TouchAction::Sent },
                    None,
                );
                self.crown_said(ticket, &format!("→ {name}: {text}"));
                self.feed.board_outcome("agent", "ask_board", Some(ticket), road.word());
                self.broadcast();
                Response::AgentAskedBoard { board: name, road, crown, reply }
            }
        }
    }

    /// `Command::PeerAsk` (T-733), from the daemon of `from`: the other
    /// crown's words, parked on this board's crown's card. Refused, in words
    /// the asking crown relays, where the two are not linked, this board has
    /// no crown or its crown no agent, its crown stands at a dialog, or a
    /// person's words are queued for it already.
    pub(super) fn peer_ask(
        &mut self,
        from: String,
        crown_key: String,
        crown_title: String,
        text: String,
    ) -> Response {
        let me = self.my_root();
        let my_name = boardlink::board_name(&me);
        let from_name = boardlink::board_name(&from);
        self.links.refresh();
        if !self.links.value.linked(&me, &from) {
            return Response::Err {
                message: format!(
                    "{my_name} does not reach {from_name}; Settings → Crown → Reaches boards on \
                     either board links the two"
                ),
            };
        }
        let Some(crown) = self.board.crown_holder().map(|t| t.id) else {
            return Response::Err {
                message: format!("{my_name} has no crown; ^o on a ticket there crowns it"),
            };
        };
        let key = self.board.ticket(crown).map(|t| t.short_key.clone()).unwrap_or_default();
        let seat = self.seat_of(crown);
        if matches!(seat, QueuedSeat::Start(_)) {
            return Response::Err {
                message: format!(
                    "{my_name}'s crown {key} has no agent to read the words; a person starts one"
                ),
            };
        }
        if self.board.live_agent(crown).is_some_and(|rec| rec.observe_only()) {
            return Response::Err {
                message: format!("{my_name}'s crown's session is external; a person resumes it"),
            };
        }
        if self.board.live_agent(crown).is_some_and(|rec| {
            rec.state.question_stop()
                || rec.state == SessionState::RequiresAction { reason: Reason::Plan }
        }) {
            return Response::Err {
                message: format!(
                    "{my_name}'s crown {key} is waiting on its person at a question or a plan; \
                     the ask can be made again once it is answered"
                ),
            };
        }
        let Some(text) = mesimon_core::command::sanitize_prompt(&text) else {
            return Response::Err { message: "nothing to send".into() };
        };
        // A shell line is a person's alone (T-876): refused here whatever
        // the asking daemon's build let through.
        if mesimon_core::road::is_shell(&text) {
            return Response::Err { message: mesimon_core::road::SHELL_LINE_REFUSED.into() };
        }
        if self.queued.iter().any(|q| q.ticket == crown && q.by.is_none() && q.from_board.is_none())
        {
            return Response::Err {
                message: format!(
                    "a person's words are queued for {my_name}'s crown {key} and go first; the \
                     ask can be made again once they have gone"
                ),
            };
        }
        // Answering an ask this board made there is a reply, said as one.
        let reply = self.asks_out.remove(&from).is_some();
        let title = mesimon_core::text::scrub_text(&crown_title);
        let words = if reply {
            self.board
                .prompts
                .render(AgentPrompt::BoardReply, &[("board", &from_name), ("text", &text)])
        } else {
            self.board.prompts.render(
                AgentPrompt::BoardAsk,
                &[("board", &from_name), ("key", &crown_key), ("title", &title), ("text", &text)],
            )
        };
        let from_board = FromBoard { name: from_name.clone(), words: text.clone() };
        if let Err(message) = self.park_peer_ask(crown, seat, words, from_board) {
            return Response::Err { message };
        }
        let sends = self.board.crown_mode.sends();
        let mut held = !sends;
        if let Some(q) = self.queued.iter_mut().find(|q| q.ticket == crown) {
            q.sends = sends;
            q.deliver = Deliver::Idle;
            held = q.held_for_person();
        }
        let road = if held { AskRoad::HeldForPerson } else { AskRoad::Queued };
        self.feed.board_outcome("peer", "board_ask_received", Some(crown), road.word());
        self.board_touched(
            &from,
            if reply { TouchAction::GotReply } else { TouchAction::Received },
            None,
        );
        if !reply {
            self.asks_in.insert(from, now_ms());
        }
        if !held {
            self.drain_queue();
        }
        self.broadcast();
        Response::PeerAsked { road, crown: key, reply }
    }

    /// `park_ask` for a peer's words: the entry carries where they came
    /// from, and a peer ask already standing is replaced, as a crown's new
    /// ask replaces its last.
    fn park_peer_ask(
        &mut self,
        crown: ulid::Ulid,
        seat: QueuedSeat,
        words: String,
        from: FromBoard,
    ) -> std::result::Result<(), String> {
        self.park_ask(crown, seat, words, super::Asker::Board(from), false, false)
    }

    /// Record that words went to or came from a linked board (T-733), so
    /// the header lights its name and the bolt knows which card.
    pub(super) fn board_touched(&mut self, path: &str, action: TouchAction, words: Option<String>) {
        let now = now_ms();
        self.board_touches.retain(|t| fresh(t, now));
        self.board_touches.push(BoardTouch {
            board: boardlink::board_name(path),
            path: path.to_string(),
            action,
            words,
            crown: self.board.crown,
            at_ms: now,
        });
    }

    /// The touches still worth drawing, for the snapshot.
    pub(super) fn recent_board_touches(&self) -> Vec<BoardTouch> {
        let now = now_ms();
        self.board_touches.iter().filter(|t| fresh(t, now)).cloned().collect()
    }
}

/// Whether a touch is still worth drawing.
fn fresh(t: &BoardTouch, now: u64) -> bool {
    now.saturating_sub(t.at_ms) < BOARD_TOUCH_MS
}
