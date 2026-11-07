//! What the crown did, as a ticket's conversation says it (T-1162).
//!
//! Every crown touch (`crown_mark`: a move, an ask, a merge, a tag …) and
//! every word mesimon carried into the crown (a wake, another board's ask
//! or reply) is one `CrownAct`, which the daemon appends to the board's
//! `crown-acts.jsonl` and the ticket page reads beside the agent's own
//! transcript. A line holds board facts only, never a prompt's text: the
//! words an ask carried are already the receiving conversation's prompt,
//! and `words` is their key (`words_key`), so the page finds that prompt
//! and says whose it was instead of `you`.
//!
//! The sentences are a person's: what was done, to which ticket, and what
//! started the crown's turn (`Cause`), never a tool's name.

use serde::{Deserialize, Serialize};

/// One thing the crown did, or one road's words into it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrownAct {
    pub at_ms: u64,
    /// The crown's ticket.
    pub crown: ulid::Ulid,
    /// The ticket acted on; the crown's own for a road's words into it
    /// (`woke_crown`, `board_asked` …).
    pub ticket: ulid::Ulid,
    /// The touch's word (`moved`, `asked`, `merged` …) or a road's.
    pub act: String,
    /// The act's object, a board fact: the column, the new title, the tag,
    /// the other board's name, the wake's news.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
    /// The key of the words this act put in a conversation (`words_key`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub words: Option<u64>,
    /// What started the crown's turn, where the board knows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<Cause>,
}

/// What the crown's turn answered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "by", rename_all = "snake_case")]
pub enum Cause {
    /// A person's words, typed or sent.
    Person,
    /// The board's wake, and its news (`T-14 "…" delivered`).
    Wake { news: String },
    /// A linked board's crown, asking or replying.
    Board {
        board: String,
        #[serde(default)]
        reply: bool,
    },
}

/// The longest `detail` a line keeps.
pub const DETAIL_BYTES: usize = 240;

/// The road words into the crown, and the crown's own words out to another
/// board: acts on the crown's own ticket.
pub const WOKE_CROWN: &str = "woke_crown";
pub const BOARD_ASKED: &str = "board_asked";
pub const BOARD_REPLIED: &str = "board_replied";
pub const ASKED_BOARD: &str = "asked_board";
pub const REPLIED_BOARD: &str = "replied_board";

/// The words' key, as both ends see them: the text as sent, and the
/// transcript's row of it, compare whitespace-blind. FNV-1a, stable across
/// builds (a line outlives the binary that wrote it).
pub fn words_key(text: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut first = true;
    for word in text.split_whitespace() {
        if !first {
            h ^= u64::from(b' ');
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        first = false;
        for b in word.bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

/// Whose words a conversation's prompt was, when an act carried them: the
/// message's head.
pub fn words_head(act: &CrownAct) -> Option<String> {
    Some(match act.act.as_str() {
        "asked" | "sent" => "crown asked".into(),
        WOKE_CROWN => "the board woke the crown".into(),
        BOARD_ASKED => format!("the {} board's crown asked", act.detail),
        BOARD_REPLIED => format!("the {} board's crown replied", act.detail),
        _ => return None,
    })
}

/// How the ticket an act touched is named on the page reading it: `this
/// ticket` on its own page, its key and title on the crown's.
pub struct Subject<'a> {
    /// `this ticket`, or `T-14 “Fix login”`.
    pub name: &'a str,
    /// `this ticket's`, or `T-14's`.
    pub own: &'a str,
}

/// The act as its row says it, without its time or its mark (`♛`, or
/// `↯` for words across boards, `is_board_road`): `crown moved this
/// ticket to REVIEW`. `None` for a touch with nothing to say on a page
/// (the bolt's `woke`, which a `woke_crown` line says).
pub fn sentence(act: &CrownAct, s: &Subject) -> Option<String> {
    let d = act.detail.as_str();
    let (name, own) = (s.name, s.own);
    let said = match act.act.as_str() {
        "moved" if !d.is_empty() => format!("crown moved {name} to {d}"),
        "moved" => format!("crown moved {name}"),
        "renamed" if !d.is_empty() => format!("crown renamed {name} “{d}”"),
        "renamed" => format!("crown renamed {name}"),
        "tagged" if !d.is_empty() => format!("crown tagged {name} {d}"),
        "tagged" => format!("crown tagged {name}"),
        "untagged" => format!("crown took {d} off {name}"),
        "note" => format!("crown wrote a note on {name}"),
        "workspace" if !d.is_empty() => format!("crown set {own} workspace to {d}"),
        "workspace" => format!("crown set {own} workspace"),
        "archived" => format!("crown archived {name}"),
        "restored" => format!("crown restored {name}"),
        "started" if !d.is_empty() => format!("crown started {own} agent on {d}"),
        "started" => format!("crown started {own} agent"),
        "woken" => format!("crown woke {own} agent"),
        "parked" => format!("crown parked {own} agent"),
        "watched" => format!("crown watches {name}"),
        "unwatched" => format!("crown stopped watching {name}"),
        "merged" if !d.is_empty() => format!("crown merged {own} branch into {d}"),
        "merged" => format!("crown merged {own} branch"),
        "asked" | "sent" => format!("crown asked {own} agent"),
        "created" => format!("crown filed {name}"),
        "held" => format!("crown held {name} out of the merge train"),
        "returned" => format!("crown gave {name} back to the merge train"),
        "close shell?" => format!("crown asked to close {own} shell"),
        "accepted plan" => format!("crown accepted {own} plan"),
        "answered" => format!("crown answered {own} question"),
        "sent plan back" => format!("crown sent {own} plan back"),
        ASKED_BOARD => format!("crown asked the {d} board"),
        REPLIED_BOARD => format!("crown replied to the {d} board"),
        WOKE_CROWN => "the board woke the crown".into(),
        BOARD_ASKED => format!("the {d} board's crown asked"),
        BOARD_REPLIED => format!("the {d} board's crown replied"),
        _ => return None,
    };
    Some(said)
}

/// The words a row says under its sentence, where the act carries some a
/// person decided on: the crown's answer to a question.
pub fn body(act: &CrownAct) -> Option<&str> {
    matches!(act.act.as_str(), "answered" | "sent plan back")
        .then_some(act.detail.as_str())
        .filter(|d| !d.is_empty())
}

/// The line under an act's row on a worker's page: what the crown was
/// answering when it did it.
pub fn cause_line(cause: &Cause) -> String {
    match cause {
        Cause::Person => "after you asked the crown".into(),
        Cause::Wake { news } if news.is_empty() => "after the board woke the crown".into(),
        Cause::Wake { news } => format!("after the board told the crown: {news}"),
        Cause::Board { board, reply: false } => format!("after the {board} board's crown asked"),
        Cause::Board { board, reply: true } => format!("after the {board} board's crown replied"),
    }
}

/// Whether the act is words across boards, which wear `↯` and strike the
/// title bar's boards.
pub fn is_board_road(act: &CrownAct) -> bool {
    matches!(act.act.as_str(), ASKED_BOARD | REPLIED_BOARD | BOARD_ASKED | BOARD_REPLIED)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn act(word: &str, detail: &str) -> CrownAct {
        CrownAct {
            at_ms: 1,
            crown: ulid::Ulid(1),
            ticket: ulid::Ulid(2),
            act: word.into(),
            detail: detail.into(),
            words: None,
            cause: None,
        }
    }

    const HERE: Subject = Subject { name: "this ticket", own: "this ticket's" };

    #[test]
    fn the_key_is_whitespace_blind_and_tells_words_apart() {
        assert_eq!(words_key("fix  the\nloop "), words_key("fix the loop"));
        assert_ne!(words_key("fix the loop"), words_key("fix the loops"));
        assert_ne!(words_key("ab c"), words_key("a bc"));
        // Stable across builds: a line outlives the binary that wrote it.
        assert_eq!(words_key(""), 0xcbf2_9ce4_8422_2325);
    }

    #[test]
    fn every_touch_word_reads_as_a_persons_sentence() {
        let there = Subject { name: "T-14 “Fix login”", own: "T-14's" };
        let cases = [
            ("moved", "REVIEW", "crown moved this ticket to REVIEW"),
            ("renamed", "Fix login loop", "crown renamed this ticket “Fix login loop”"),
            ("tagged", "BUG", "crown tagged this ticket BUG"),
            ("untagged", "BUG", "crown took BUG off this ticket"),
            ("started", "Opus", "crown started this ticket's agent on Opus"),
            ("merged", "main", "crown merged this ticket's branch into main"),
            ("held", "", "crown held this ticket out of the merge train"),
            ("asked", "", "crown asked this ticket's agent"),
        ];
        for (word, detail, want) in cases {
            assert_eq!(sentence(&act(word, detail), &HERE).as_deref(), Some(want), "{word}");
        }
        assert_eq!(
            sentence(&act("moved", "DONE"), &there).as_deref(),
            Some("crown moved T-14 “Fix login” to DONE")
        );
        assert_eq!(
            sentence(&act(ASKED_BOARD, "relay"), &HERE).as_deref(),
            Some("crown asked the relay board")
        );
        // The bolt's own word says nothing on a page.
        assert_eq!(sentence(&act("woke", ""), &HERE), None);
        // No sentence names a tool.
        for word in ["moved", "asked", "merged", "held", "parked", "note", "watched"] {
            let s = sentence(&act(word, "x"), &HERE).unwrap();
            assert!(!s.contains('_'), "{s}");
        }
    }

    #[test]
    fn the_prompt_an_act_carried_says_whose_it_was() {
        assert_eq!(words_head(&act("asked", "")).as_deref(), Some("crown asked"));
        assert_eq!(
            words_head(&act(BOARD_REPLIED, "relay")).as_deref(),
            Some("the relay board's crown replied")
        );
        assert_eq!(words_head(&act("moved", "DONE")), None);
    }

    #[test]
    fn a_cause_names_who_started_the_crowns_turn() {
        assert_eq!(cause_line(&Cause::Person), "after you asked the crown");
        assert_eq!(
            cause_line(&Cause::Board { board: "relay".into(), reply: false }),
            "after the relay board's crown asked"
        );
        assert_eq!(
            cause_line(&Cause::Wake { news: "T-14 “Fix” delivered".into() }),
            "after the board told the crown: T-14 “Fix” delivered"
        );
    }

    #[test]
    fn a_line_from_an_older_build_reads_with_what_it_held() {
        let line = r#"{"at_ms":5,"crown":"01ARZ3NDEKTSV4RRFFQ69G5FAV","ticket":"01ARZ3NDEKTSV4RRFFQ69G5FAV","act":"moved"}"#;
        let a: CrownAct = serde_json::from_str(line).unwrap();
        assert_eq!((a.detail.as_str(), a.words, a.cause), ("", None, None));
        let round = serde_json::to_string(&CrownAct {
            cause: Some(Cause::Board { board: "relay".into(), reply: true }),
            ..a
        })
        .unwrap();
        assert!(
            round.contains(r#""cause":{"by":"board","board":"relay","reply":true}"#),
            "{round}"
        );
    }
}
