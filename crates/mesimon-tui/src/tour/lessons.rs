//! The tour's lessons (T-763): each a world to start from, the keys a
//! person would press in it, and what trying it means.
//!
//! The words follow the board's own: a lesson names a key the way its hint
//! does and says what a person sees, never how mesimon does it. A lesson
//! whose part belongs to an agent (its turn ending, a forge's pipeline, the
//! crown's moves) plays that part with `Do`, as the hooks would.

use mesimon_core::board::{SessionKind, SessionState, StopReason};
use mesimon_core::command::{ExternalItem, PrItem};
use mesimon_core::keymap::Key;

use crate::app::{App, Mode};

use super::toy::World;
use super::Step::{self, Do, Key as K, Say, Type, Wait};

pub(crate) struct Lesson {
    /// The heading the lesson sits under in the list.
    pub group: &'static str,
    pub title: &'static str,
    /// What the lesson is about, in a sentence or three.
    pub about: &'static str,
    /// The world the lesson starts from.
    pub seed: fn(&mut World),
    /// The keys, as the lesson plays them.
    pub show: &'static [Step],
    /// What Enter asks the person to do on the practice board; `None` where
    /// there is nothing to try there.
    pub ask: Option<&'static str>,
    /// The try is done: the world, or the practice board, shows what the
    /// ask asked for.
    pub done: Option<fn(&World, &App) -> bool>,
}

const LOGIN: &str = "Fix the login redirect";
const DOCS: &str = "Write the onboarding guide";
const AVATARS: &str = "Cache the avatar images";
const DARK: &str = "Dark mode toggle";
const LIMITS: &str = "Rate-limit the public API";
const SEED_TICKETS: usize = 6;

/// The board every lesson starts from unless it needs another: the four
/// columns a new board has, a few tickets, nothing running.
fn board(w: &mut World) {
    w.ticket("TODO", LOGIN);
    w.ticket("TODO", DOCS);
    w.ticket("TODO", AVATARS);
    w.ticket("IN PROGRESS", DARK);
    w.ticket("REVIEW", LIMITS);
    w.ticket("DONE", "Set up CI");
}

/// The board, the first ticket described: a brief with a link and a
/// checklist under `Summary`.
fn described(w: &mut World) {
    board(w);
    if let Some(id) = w.find(LOGIN) {
        w.describe(
            id,
            "After signing in, people land on /home instead of the page they asked for.\n\
             \n\
             See https://github.com/acme/web/issues/412\n\
             \n\
             ## Summary\n\
             - [x] Reproduce it\n\
             - [ ] Keep the return URL through the sign-in\n\
             - [ ] Test the deep links\n",
        );
    }
}

/// The board with agents at work: one running, one finished in REVIEW.
fn working(w: &mut World) {
    board(w);
    if let Some(id) = w.find(DARK) {
        w.session(id, SessionKind::Claude, SessionState::Running);
    }
    if let Some(id) = w.find(LIMITS) {
        w.worktree(id, 2);
        w.session(id, SessionKind::Claude, idle());
    }
    w.changed = false;
}

fn idle() -> SessionState {
    SessionState::Idle { stop_reason: StopReason::EndTurn }
}

/// `title`'s agent ends its turn.
fn ends_turn(w: &mut World, title: &str) {
    if let Some(id) = w.find(title) {
        w.session(id, SessionKind::Claude, idle());
    }
}

fn live(w: &World) -> usize {
    w.board.tickets.iter().filter(|t| t.archived.is_none()).count()
}

fn pr(w: &World, title: &str, state: &str, pipeline: &str) -> Option<PrItem> {
    let id = w.find(title)?;
    Some(PrItem {
        ticket: id,
        forge: "github".into(),
        repo: "acme/web".into(),
        number: 418,
        url: "https://github.com/acme/web/pull/418".into(),
        state: state.into(),
        pipeline: pipeline.into(),
        review: "approved".into(),
        ..Default::default()
    })
}

fn set_pr(w: &mut World, state: &str, pipeline: &str) {
    if let Some(p) = pr(w, LIMITS, state, pipeline) {
        w.prs = vec![p];
        w.changed = true;
    }
}

pub(crate) static LESSONS: &[Lesson] = &[
    // ---- tickets ----------------------------------------------------------
    Lesson {
        group: "Tickets",
        title: "Create a ticket",
        about: "A ticket is one piece of work, and the board is its columns. o opens a new \
                ticket in the column the cursor is in: type its title, and enter files it.",
        seed: board,
        show: &[
            Say("o opens a new ticket"),
            K(Key::Char('o')),
            Say("type its title"),
            Type("Add a password reset"),
            Say("enter files it"),
            K(Key::Enter),
            Wait(600),
        ],
        ask: Some("press o, type a title, then enter"),
        done: Some(|w, _| w.board.tickets.len() > SEED_TICKETS),
    },
    Lesson {
        group: "Tickets",
        title: "Rename a ticket",
        about: "r on a card puts its title in a field: enter keeps the new one, esc keeps \
                the old.",
        seed: board,
        show: &[
            Say("r renames the card under the cursor"),
            K(Key::Char('r')),
            Say("change the title"),
            K(Key::Ctrl('u')),
            Type("Fix the login redirect loop"),
            Say("enter keeps it"),
            K(Key::Enter),
        ],
        ask: Some("press r on a card, change its title, then enter"),
        done: Some(|w, _| w.find(LOGIN).is_none()),
    },
    Lesson {
        group: "Tickets",
        title: "Describe a ticket",
        about: "tab on a card opens its description: the brief an agent reads before it \
                starts. It is markdown; ^v pastes a picture from the clipboard into it, and \
                ^s saves.",
        seed: board,
        show: &[
            Say("tab opens the description"),
            K(Key::Tab),
            Say("write the brief"),
            Type("People land on /home after signing in."),
            K(Key::Enter),
            Type("Keep the page they asked for."),
            Say("^s saves it"),
            K(Key::Ctrl('s')),
            Wait(600),
        ],
        ask: Some("press tab on a card, write a line, then ^s"),
        done: Some(|w, _| w.board.tickets.iter().any(|t| !t.notes.is_empty())),
    },
    Lesson {
        group: "Tickets",
        title: "Checklists",
        about: "A Summary heading in the description, with - [ ] lines under it, is a \
                checklist: the card shows how far along it is. ^j opens it, and space ticks \
                a box.",
        seed: described,
        show: &[
            Say("the card counts its checklist"),
            Wait(900),
            Say("^j opens it"),
            K(Key::Ctrl('j')),
            K(Key::Char('j')),
            Say("space ticks a box"),
            K(Key::Space),
            Wait(400),
            Say("esc closes it"),
            K(Key::Esc),
        ],
        ask: Some("press ^j on the first card, then space on a box"),
        done: Some(|w, _| w.notes.values().any(|n| n.matches("- [x]").count() > 1)),
    },
    Lesson {
        group: "Tickets",
        title: "Tags",
        about: "^t opens the tags: pick one with the arrows and enter puts it on the card. \
                A tag's group is a number, and that number on the board cycles the card \
                through the group's tags.",
        seed: board,
        show: &[
            Say("^t opens the tags"),
            K(Key::Ctrl('t')),
            Say("enter puts one on the card"),
            K(Key::Enter),
            Say("esc closes them"),
            K(Key::Esc),
            K(Key::Char('j')),
            Say("1 cycles the next card through group 1"),
            K(Key::Char('1')),
            K(Key::Char('1')),
        ],
        ask: Some("press ^t on a card and enter on a tag, or press 1"),
        done: Some(|w, _| w.board.tickets.iter().any(|t| !t.tags.is_empty())),
    },
    Lesson {
        group: "Tickets",
        title: "Copy a ticket's title",
        about: "c copies the card's key, c again its title, and a third c both: what an \
                agent elsewhere, a commit or a message needs to name it.",
        seed: board,
        show: &[
            Say("c copies the key"),
            K(Key::Char('c')),
            Say("c again copies the title"),
            K(Key::Char('c')),
            Say("and again, both"),
            K(Key::Char('c')),
        ],
        ask: Some("press c on a card"),
        done: Some(|_, app| app.status.contains("copied")),
    },
    Lesson {
        group: "Tickets",
        title: "Duplicate a ticket",
        about: "y y copies the card, its description and tags with it, right under it.",
        seed: described,
        show: &[
            Say("y, then y again, duplicates the card"),
            K(Key::Char('y')),
            K(Key::Char('y')),
            Wait(600),
        ],
        ask: Some("press y twice on a card"),
        done: Some(|w, _| w.board.tickets.len() > SEED_TICKETS),
    },
    Lesson {
        group: "Tickets",
        title: "Archive a ticket",
        about: "a a takes the card off the board and keeps it: the menu's Archived tickets \
                lists every one, and a there puts it back.",
        seed: board,
        show: &[
            Say("a, then a again, archives the card"),
            K(Key::Char('a')),
            K(Key::Char('a')),
            Wait(600),
            Say("the menu's Archived tickets lists it"),
            K(Key::Esc),
            K(Key::Char('j')),
            K(Key::Enter),
            Say("a there puts it back"),
            K(Key::Char('a')),
            Wait(600),
        ],
        ask: Some("press a twice on a card"),
        done: Some(|w, _| live(w) < SEED_TICKETS),
    },
    Lesson {
        group: "Tickets",
        title: "Delete a ticket",
        about: "d d deletes the card for good, its notes with it. u undoes a move or an edit, \
                but not this: archive what you may want back.",
        seed: board,
        show: &[
            Say("d, then d again, deletes the card"),
            K(Key::Char('d')),
            K(Key::Char('d')),
            Wait(600),
        ],
        ask: Some("press d twice on a card"),
        done: Some(|w, _| w.board.tickets.len() < SEED_TICKETS),
    },
    // ---- moving -----------------------------------------------------------
    Lesson {
        group: "Moving",
        title: "Move a card",
        about: "> moves the card to the next column and < to the one before; press it twice. \
                The cursor stays where it was, so the next card is already under it.",
        seed: board,
        show: &[
            Say("> picks the card up"),
            K(Key::Char('>')),
            Say("> again puts it in the next column"),
            K(Key::Char('>')),
            Wait(400),
            Say("the cursor stays, so > > moves the next one too"),
            K(Key::Char('>')),
            K(Key::Char('>')),
        ],
        ask: Some("move a card to IN PROGRESS with > >"),
        done: Some(|w, _| w.board.column_tickets("IN PROGRESS").len() > 1),
    },
    Lesson {
        group: "Moving",
        title: "Order the cards",
        about: "A column is a priority list, top first. J and K move the card down and up, \
                H and L to the column beside it, and the cursor goes with it. . does the \
                last move again.",
        seed: board,
        show: &[
            Say("J moves the card down"),
            K(Key::Char('J')),
            Say(". does it again"),
            K(Key::Char('.')),
            Say("K moves it back up"),
            K(Key::Char('K')),
            Say("L takes it to the next column"),
            K(Key::Char('L')),
        ],
        ask: Some("press J or K on a card"),
        done: Some(|w, _| w.board.column_tickets("TODO").first().is_none_or(|t| t.title != LOGIN)),
    },
    Lesson {
        group: "Moving",
        title: "Automatic moves",
        about: "Each column says where its cards go when their agent starts working and \
                when it is done. On a new board, a card leaves TODO for IN PROGRESS when its \
                agent starts, and lands in REVIEW when it finishes: the board says what \
                needs you without your moving a thing.",
        seed: board,
        show: &[
            Say("start an agent on the card"),
            K(Key::Enter),
            K(Key::Enter),
            K(Key::Char('q')),
            Say("it is working: the card moved to IN PROGRESS"),
            Wait(2_200),
            Do(|w| ends_turn(w, LOGIN)),
            Say("its turn ended: the card is in REVIEW, waiting for you"),
            Wait(1_500),
        ],
        ask: None,
        done: None,
    },
    // ---- finding ----------------------------------------------------------
    Lesson {
        group: "Finding",
        title: "Search",
        about: "/ searches every ticket by its key, title and description, archived ones \
                too with tab; enter goes to the one under the cursor.",
        seed: described,
        show: &[
            Say("/ opens the search"),
            K(Key::Char('/')),
            Say("type a few letters of anything"),
            Type("avatar"),
            Say("enter goes to it"),
            K(Key::Enter),
        ],
        ask: Some("press /, type part of a title, then enter"),
        done: Some(|_, app| {
            !matches!(app.mode, Mode::Search(_))
                && app.status.is_empty()
                && app.selected_title().is_some_and(|t| t != LOGIN)
        }),
    },
    Lesson {
        group: "Finding",
        title: "Open a link",
        about: "Every link in a ticket's title and description is a key away: ^k lists them \
                and enter opens one, and ^K opens the first at once.",
        seed: described,
        show: &[
            Say("^k lists the card's links"),
            K(Key::Ctrl('k')),
            Wait(600),
            Say("enter opens one"),
            K(Key::Enter),
        ],
        ask: Some("press ^k on the first card"),
        done: Some(|_, app| {
            matches!(app.mode, Mode::Links { .. }) || app.status.starts_with("opens")
        }),
    },
    // ---- agents -----------------------------------------------------------
    Lesson {
        group: "Agents",
        title: "Quick start",
        about: "A ticket and its agent in one go: o, its title, then enter files it and \
                enter again starts an agent on it, the title as the brief (shift+enter does \
                both where your terminal sends it). On your own board you are then in the \
                agent's session; the board is a key away.",
        seed: board,
        show: &[
            Say("o, then the title"),
            K(Key::Char('o')),
            Type("Add a password reset"),
            Say("enter files it"),
            K(Key::Enter),
            Wait(300),
            Say("enter again starts its agent"),
            K(Key::Enter),
            Wait(1_000),
            Say("it is working, and the card moved to IN PROGRESS"),
            Wait(1_500),
        ],
        ask: Some("press o, type a title, then enter twice"),
        done: Some(|w, _| w.board.sessions.iter().any(|s| s.kind.is_agent())),
    },
    Lesson {
        group: "Agents",
        title: "Start an agent",
        about: "On a card with no agent, enter opens the ticket's page: enter on + agent \
                session starts one there, its title and description as the brief. On a card \
                whose agent is working, enter goes straight to it.",
        seed: board,
        show: &[
            Say("enter opens the ticket's page"),
            K(Key::Enter),
            Wait(600),
            Say("enter on + agent session starts one"),
            K(Key::Enter),
            Wait(1_200),
            Say("q goes back to the board"),
            K(Key::Char('q')),
            Wait(1_200),
        ],
        ask: Some("press enter on a card, then enter on + agent session"),
        done: Some(|w, _| w.board.sessions.iter().any(|s| s.kind.is_agent())),
    },
    Lesson {
        group: "Agents",
        title: "Agent tiers",
        about: "A tier is the agent, model and effort a ticket starts on. ^n steps the card \
                through your tiers, so the hard bug gets the deep one and the rename the \
                quick one. Settings, then Agents, makes them.",
        seed: board,
        show: &[
            Say("^n gives the card the next tier"),
            K(Key::Ctrl('n')),
            Wait(500),
            Say("and the one after"),
            K(Key::Ctrl('n')),
            Wait(500),
        ],
        ask: Some("press ^n on a card"),
        done: Some(|w, _| w.board.tickets.iter().any(|t| t.tier.is_some())),
    },
    Lesson {
        group: "Agents",
        title: "Worktrees",
        about: "A ticket on a worktree gets a branch and a checkout of its own, so agents \
                never edit the same files. shift+tab picks it before the agent starts; when \
                the work is done, m merges the branch.",
        seed: board,
        show: &[
            Say("shift+tab puts the ticket on a worktree"),
            K(Key::BackTab),
            Wait(500),
            Say("its agent starts on a branch of its own"),
            K(Key::Enter),
            K(Key::Enter),
            K(Key::Char('q')),
            Do(|w| {
                if let Some(id) = w.find(LOGIN) {
                    w.worktree(id, 3);
                }
            }),
            Say("it commits on the branch"),
            Wait(1_200),
            Do(|w| ends_turn(w, LOGIN)),
            Say("done: m merges the branch"),
            K(Key::Char('m')),
            Wait(700),
            K(Key::Char('m')),
            Wait(1_200),
            Say("merged: the card says so"),
            K(Key::Esc),
            Wait(900),
        ],
        ask: Some("press shift+tab on a card that has not started"),
        done: Some(|w, _| {
            w.board
                .tickets
                .iter()
                .any(|t| t.workspace == Some(mesimon_core::board::WorkspaceStrategy::Worktree))
        }),
    },
    Lesson {
        group: "Agents",
        title: "The merge train",
        about: "With the train on, a finished branch merges by itself: a column's settings \
                say whether the train asks its agent to rebase there, or merges there too. \
                On a new board it merges from REVIEW. Settings turns it on.",
        seed: working,
        show: &[
            Say("REVIEW holds a finished branch"),
            K(Key::Char('l')),
            K(Key::Char('l')),
            Wait(900),
            Do(|w| {
                if let Some(id) = w.find(LIMITS) {
                    w.merged(id);
                }
            }),
            Say("the train merges it"),
            Wait(1_200),
            Do(|w| {
                if let Some(id) = w.find(LIMITS) {
                    w.arrive(id, "DONE");
                }
            }),
            Say("and it is done"),
            Wait(1_200),
        ],
        ask: None,
        done: None,
    },
    Lesson {
        group: "Agents",
        title: "Pull requests",
        about: "A branch pushed to GitHub or GitLab shows its pull request on the card: \
                open, its checks, merged. ^h lists them. A column can wait for a pull \
                request to merge before its cards move on.",
        seed: working,
        show: &[
            K(Key::Char('l')),
            K(Key::Char('l')),
            Do(|w| set_pr(w, "open", "running")),
            Say("the card shows its pull request, checks running"),
            Wait(1_500),
            Do(|w| set_pr(w, "open", "passed")),
            Say("the checks passed"),
            Wait(1_200),
            Say("^h lists the ticket's pull requests"),
            K(Key::Ctrl('h')),
            Wait(1_200),
            K(Key::Esc),
            Do(|w| set_pr(w, "merged", "passed")),
            Say("merged on the forge"),
            Wait(1_200),
            Do(|w| {
                if let Some(id) = w.find(LIMITS) {
                    w.merged(id);
                    w.arrive(id, "DONE");
                }
            }),
            Say("so the card moves on to DONE"),
            Wait(1_500),
        ],
        ask: Some("press ^h on the card in REVIEW"),
        done: Some(|_, app| matches!(app.mode, Mode::PullRequests { .. })),
    },
    Lesson {
        group: "Agents",
        title: "The crown",
        about: "^o crowns a ticket: its agent may then file, move, start and merge every \
                other ticket on the board. One crown at a time; ^o again takes it back.",
        seed: working,
        show: &[
            K(Key::Char('l')),
            Say("^o crowns the card's ticket"),
            K(Key::Ctrl('o')),
            Wait(1_200),
            Do(|w| {
                if let Some(id) = w.find(DOCS) {
                    w.arrive(id, "IN PROGRESS");
                    w.session(id, SessionKind::Claude, SessionState::Running);
                }
            }),
            Say("its agent starts the next ticket"),
            Wait(1_500),
            Say("^o again takes the crown back"),
            K(Key::Ctrl('o')),
            Wait(600),
        ],
        ask: Some("press ^o on a card"),
        done: Some(|w, _| w.board.crown.is_some()),
    },
    Lesson {
        group: "Agents",
        title: "Adopt a session",
        about: "An agent you started outside mesimon in this repository can join the board: \
                the menu's External sessions lists them, and enter adopts one onto a ticket \
                of its own.",
        seed: |w| {
            board(w);
            w.external = vec![ExternalItem {
                id: uuid::Uuid::from_u128(0x7e57),
                provider: Default::default(),
                conversation_id: String::new(),
                cwd: "/practice".into(),
                transcript_path: String::new(),
                mtime_ms: mesimon_core::clock::now_ms().saturating_sub(240_000),
                preview: Some("Profile the slow search page".into()),
                name: None,
                running_elsewhere: false,
            }];
        },
        show: &[
            Say("esc opens the menu"),
            K(Key::Esc),
            Wait(400),
            Say("External sessions lists the agents started elsewhere"),
            K(Key::Enter),
            Wait(700),
            Say("enter adopts one onto a ticket"),
            K(Key::Enter),
            Wait(900),
        ],
        ask: Some("open External sessions from esc, then enter"),
        done: Some(|w, _| w.board.tickets.len() > SEED_TICKETS),
    },
    // ---- the board --------------------------------------------------------
    Lesson {
        group: "The board",
        title: "Columns",
        about: "O adds a column beside the cursor's. enter on a column's name opens its \
                settings: where its cards go when an agent starts or finishes, whether new \
                tickets may be filed there, the train's reach.",
        seed: board,
        show: &[
            Say("O adds a column beside this one"),
            K(Key::Char('O')),
            Type("BLOCKED"),
            K(Key::Enter),
            Say("its settings open with it"),
            Wait(600),
            K(Key::Char('j')),
            K(Key::Char('j')),
            K(Key::Char('j')),
            Say("Filed here: may o file a ticket in it"),
            Wait(900),
            K(Key::Char('j')),
            K(Key::Char('j')),
            K(Key::Char('j')),
            K(Key::Char('j')),
            K(Key::Char('j')),
            K(Key::Char('j')),
            Say("Going out: where its cards go when agents start and finish"),
            Wait(1_200),
            K(Key::Esc),
            Say("enter on a column's name opens them again"),
            K(Key::Enter),
            Wait(900),
            K(Key::Esc),
        ],
        ask: Some("press O and name a column"),
        done: Some(|w, _| w.board.columns.len() > 4),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_lesson_has_a_place_and_words() {
        let mut groups: Vec<&str> = Vec::new();
        for l in LESSONS {
            assert!(!l.about.is_empty() && !l.show.is_empty(), "{}", l.title);
            assert_eq!(l.ask.is_some(), l.done.is_some(), "{}: an ask needs a done", l.title);
            // A group's lessons sit together, so the list's headings are
            // each drawn once.
            if groups.last() != Some(&l.group) {
                assert!(!groups.contains(&l.group), "{} is split", l.group);
                groups.push(l.group);
            }
        }
    }
}
