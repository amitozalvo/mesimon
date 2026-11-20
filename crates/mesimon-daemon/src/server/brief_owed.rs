//! T-1290: the crown starts an agent and asks it at once, deliver now. The
//! seat is idle at its `SessionStart` with the start's brief still owed (on
//! the mod road its bridge has not polled; on the hook set its composer has
//! not painted), so the ask is refused before any key and kept on the queue.
//! The brief then goes in, and its `UserPromptSubmit` is the board's own
//! words, not a person's: the ask survives it and goes after the brief's
//! turn. The reason the crown reads names the brief, never a person.

use super::*;
use crate::server::queued_person_push_tests::{daemon, prompt_frame, worker_ticket};

/// A `UserPromptSubmit` by the seat's own road.
fn prompt(d: &Daemon, sid: uuid::Uuid, text: &str) -> HookFrame {
    prompt_frame(sid, d.board.session(sid).unwrap().road, text)
}

fn feed(d: &mut Daemon) -> String {
    d.feed.flush().unwrap();
    std::fs::read_to_string(d.paths.activity_log()).unwrap_or_default()
}

fn kept_lines(d: &mut Daemon) -> Vec<String> {
    feed(d).lines().filter(|l| l.contains("\"queued_ask_kept\"")).map(str::to_string).collect()
}

/// The fixture's daemon with its seat just up (`Idle{Unknown}`, what
/// `SessionStart` sets), the start's brief armed and still parked, and the
/// queued ask a crown's `deliver now`.
fn at_start(road: Road, f: impl FnOnce(&mut Daemon, ulid::Ulid, uuid::Uuid)) {
    let dir = std::env::temp_dir()
        .join(format!("msmn-t1290-{road:?}-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]));
    let (mut d, _, _rx) = daemon(&dir);
    let sid = d.board.sessions[0].id;
    let ticket = d.board.sessions[0].ticket;
    let crown = ulid::Ulid::new();
    d.board.tickets.push(worker_ticket(ticket, "IN PROGRESS", "a"));
    let state = SessionState::Idle { stop_reason: StopReason::Unknown };
    let rec = &mut d.board.sessions[0];
    rec.state = state.clone();
    rec.pane_key = Some("1:%0".into());
    rec.road = road;
    rec.started_by = Some(crown);
    rec.pending_submit = true;
    d.machines.insert(sid, Machine::new(state, 1_000));
    let brief = Parked { text: String::new(), brief: true, title: road == Road::Mod };
    let mut owed = Owed::launch(ticket, brief, Ack::PROMPT);
    owed.mod_road = road == Road::Mod;
    owed.next_press = Some(now_ms() + 60_000);
    d.owed.insert(sid, owed);
    let q = &mut d.queued[0];
    q.by = Some(crown);
    q.sends = true;
    q.deliver = Deliver::Now;
    q.queued_at = now_ms();
    f(&mut d, ticket, sid);
    drop(d);
    let _ = std::fs::remove_dir_all(dir);
}

/// The brief goes in as its road sends it: down the mod's `submit`, or
/// pasted into the painted composer. Its words are the ticket's.
fn brief_goes_in(d: &mut Daemon, sid: uuid::Uuid) {
    let owed = d.owed.get_mut(&sid).unwrap();
    owed.sent = owed.parked.take();
    owed.sent_at = Some(now_ms() + 1);
    if owed.mod_road {
        owed.frame = Some("f1".into());
    }
}

#[test]
fn an_ask_at_a_start_waits_for_the_brief_and_survives_its_prompt() {
    for road in [Road::Mod, Road::Hooks] {
        at_start(road, |d, ticket, sid| {
            let Response::Err { message } = d.send_queued_ask(ticket) else {
                panic!("{road:?}: the brief is owed, so nothing is sent ahead of it");
            };
            assert!(message.contains("brief has not gone in yet"), "{road:?}: {message}");
            assert!(!message.contains("person"), "{road:?}: no person is there: {message}");
            assert!(message.contains("the words stay queued"), "{road:?}: {message}");
            assert_eq!(d.queued.len(), 1, "{road:?}: kept");
            let kept = kept_lines(d);
            assert_eq!(kept.len(), 1, "{road:?}: {kept:?}");
            assert!(!kept[0].contains(A_PERSON_ATTACHED), "{road:?}: {kept:?}");
            assert!(kept[0].contains("brief"), "{road:?}: {kept:?}");

            brief_goes_in(d, sid);
            d.on_hook(prompt(d, sid, "worker"));
            assert!(!d.owed.contains_key(&sid), "{road:?}: the brief's own ack");
            assert_eq!(d.queued.len(), 1, "{road:?}: the brief talked past no ask");
            let fed = feed(d);
            assert!(!fed.contains("queued_ask_dropped_by_hand"), "{road:?}: {fed}");

            // A person's own prompt after it still talks past the ask.
            d.on_hook(prompt(d, sid, "a person's words"));
            assert!(d.queued.is_empty(), "{road:?}: a person's prompt drops it");
        });
    }
}

/// A person's words a start carried under the brief (T-294) are theirs:
/// their ack talks past an ask queued before them, as before (T-894).
#[test]
fn a_start_carrying_a_person_s_words_still_talks_past_the_ask() {
    at_start(Road::Hooks, |d, _ticket, sid| {
        let owed = d.owed.get_mut(&sid).unwrap();
        owed.parked.as_mut().unwrap().text = "do the tests first".into();
        brief_goes_in(d, sid);
        d.on_hook(prompt(d, sid, "worker\n\ndo the tests first"));
        assert!(d.queued.is_empty(), "the person's words went in after the ask was queued");
    });
}
