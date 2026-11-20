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
    /// The title immediately before a recorded rename. Older acts hold only the new title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_title: Option<String>,
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

const BEFORE: &str = "Before  ";
const AFTER: &str = "After   ";
pub const RENAME_BODY_BYTES: usize = BEFORE.len() + DETAIL_BYTES + 1 + AFTER.len() + DETAIL_BYTES;

/// A journaled title marks a cut inside its own detail allowance.
pub fn title_detail(title: &str) -> String {
    let clean = crate::text::scrub_text(title);
    if clean.len() <= DETAIL_BYTES {
        return clean;
    }
    format!("{}…", crate::text::cap_bytes(&clean, DETAIL_BYTES - "…".len()))
}

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
    crate::text::fnv1a(
        text.split_whitespace()
            .enumerate()
            .flat_map(|(i, word)| (i > 0).then_some(b' ').into_iter().chain(word.bytes())),
    )
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
    /// `this ticket's`, or `T-14 “Fix login”'s` (T-1263: the key alone reads as an id).
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
        "renamed" if act.previous_title.is_some() => format!("crown renamed {name}"),
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

/// One crown act as a conversation places it (T-1162).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Line {
    pub ms: u64,
    pub head: String,
    /// The other ticket's title as `head` spells it, the part a narrow row
    /// shortens (T-1263); empty where the head names none.
    pub title: String,
    /// The act's sentence, which a row of its own says where no prompt
    /// held the words (T-1289): `crown asked this ticket's agent`; `head`
    /// where the act carried none.
    pub alone: String,
    pub road: bool,
    /// Words the act carries itself (the crown's answer), else empty.
    pub body: String,
    /// Two labeled, literal title lines in the body.
    pub rename: bool,
    pub cause: Option<String>,
    /// The key of the words it put in a conversation: that prompt is
    /// marked, and the act is no row of its own.
    pub words: Option<u64>,
}

/// The acts `ticket`'s conversation says (T-1162), oldest first: on a
/// worker's, what the crown did to it and why; on the crown's own, what it
/// did to every other ticket and the words the board carried into it.
pub fn lines_of(
    board: &crate::board::Board,
    tiers: &crate::tier::Book,
    acts: &[CrownAct],
    ticket: ulid::Ulid,
) -> Vec<Line> {
    let mut out = Vec::new();
    for a in acts {
        let own = a.crown == ticket;
        if !own && a.ticket != ticket {
            continue;
        }
        // A start names its tier (T-1263): a line written before the
        // daemon wrote the name holds the tier's id, which is named here,
        // or left unsaid when this machine has no such tier.
        let named;
        let a = if a.act == "started" && a.detail.parse::<ulid::Ulid>().is_ok() {
            named = CrownAct {
                detail: tiers.get(&a.detail).map(|t| t.name).unwrap_or_default(),
                ..a.clone()
            };
            &named
        } else {
            a
        };
        let previous = a.previous_title.as_deref().filter(|_| a.act == "renamed");
        let (name, owns);
        let mut title = String::new();
        let subject = if own && a.ticket != ticket {
            let Some(t) = board.ticket(a.ticket) else { continue };
            if previous.is_some() {
                name = t.short_key.clone();
            } else {
                name = format!("{} “{}”", t.short_key, t.title);
                title = t.title.clone();
            }
            owns = format!("{name}'s");
            Subject { name: &name, own: &owns }
        } else {
            Subject { name: "this ticket", own: "this ticket's" }
        };
        // Words this conversation holds: an ask into this ticket's, a road's
        // into the crown's own. The crown's ask to another ticket went into
        // that one's conversation, not its own (T-1289).
        let words = a.words.filter(|_| a.ticket == ticket);
        let Some(said) = sentence(a, &subject) else { continue };
        let (head, title) = match words.and(words_head(a)) {
            Some(head) => (head, String::new()),
            None => (said.clone(), title),
        };
        out.push(Line {
            ms: a.at_ms,
            head,
            title,
            alone: said,
            road: is_board_road(a),
            body: match previous {
                Some(before) => format!("{BEFORE}{before}\n{AFTER}{}", a.detail),
                None => body(a).unwrap_or_default().to_string(),
            },
            rename: previous.is_some(),
            // The crown's own conversation says what started its turn.
            cause: a.cause.as_ref().filter(|_| !own).map(cause_line),
            words,
        });
    }
    out
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
            previous_title: None,
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
        assert_eq!(words_key(" fix\tthe  loop\n"), 0xd598_1c00_1e96_aff7);
        // UTF-8 bytes, an ideographic space between words: the phone's
        // `wordsKey` pins the same (T-1309).
        assert_eq!(words_key("שלום\u{3000} עולם"), 0xaeed_b9d4_d4e9_7474);
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
    fn a_rename_keeps_two_independently_bounded_titles_and_reads_older_acts() {
        let old = act("renamed", "New");
        let json = serde_json::to_value(&old).unwrap();
        assert!(json.get("previous_title").is_none());
        let legacy: CrownAct = serde_json::from_value(json).unwrap();
        assert_eq!(sentence(&legacy, &HERE).unwrap(), "crown renamed this ticket “New”");
        let long = format!("{}雪", "a".repeat(DETAIL_BYTES - 1));
        let before = title_detail(&long);
        assert_eq!(before, format!("{}…", "a".repeat(DETAIL_BYTES - "…".len())));
        let after = title_detail(&"ש".repeat(DETAIL_BYTES));
        assert!(after.ends_with('…') && after.len() <= DETAIL_BYTES);
        assert_eq!(title_detail("a\x07b"), "ab");
        assert_eq!(title_detail(&"a".repeat(DETAIL_BYTES)), "a".repeat(DETAIL_BYTES));
        let change = CrownAct { previous_title: Some(before), detail: after, ..old };
        assert_eq!(
            serde_json::from_value::<CrownAct>(serde_json::to_value(&change).unwrap()).unwrap(),
            change
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
