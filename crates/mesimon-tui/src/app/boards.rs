//! Linked boards in the TUI (T-733): the Settings › Crown › Reaches boards
//! list, the header's word for a linked board, and the bolt between the
//! crown's card and that word.
//!
//! The list's rows are the boards this one reaches (from the snapshot's
//! `links`), then the boards open on this machine that it could reach (the
//! `boards` walk, run when the list opens), then a row that takes a folder
//! typed by hand, for a board that is not open. Enter on an open board
//! links it, on a linked one unlinks it on the second press, on the folder
//! row opens the field. The daemon writes the machine's `links.toml`, so a
//! link made here reads on the other board at its next look.
//!
//! A board touch off the snapshot (`BoardTouch`) lights the linked board's
//! name in the header with what happened (`sent ask to relay`, `relay
//! asks`, `relay replied`, `replied to relay`, or why an ask was refused)
//! for `LINK_LIT_MS`, and strikes T-544's bolt between the crown's card and
//! that name: upward for words that left, downward for words that came.

use super::*;
use crate::boards::OpenBoard;
use crate::strike::LinkStrike;
use mesimon_core::command::{BoardTouch, LinkView};
use mesimon_core::text::scrub_cells;

/// How long a typed folder may be: a path, generously.
const FOLDER_MAX: usize = 1024;

/// One row of the boards list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardRow {
    /// A board this one reaches.
    Linked(LinkView),
    /// A board open on this machine, not linked yet.
    Open(OpenBoard),
    /// The walk for open boards has not answered yet.
    Scanning,
    /// Type a folder.
    Folder,
}

/// The header's lit word for a board touch (T-733), from the moment this
/// board first saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LinkLit {
    pub board: String,
    pub words: String,
    /// A refusal lights in the plain ink, not the crown's.
    pub refused: bool,
    pub seen: u64,
}

/// What the header says for a touch: the board's name and what happened.
fn link_words(t: &BoardTouch) -> String {
    let board = scrub_cells(&t.board, false);
    match t.action.as_str() {
        "sent" => format!("sent ask to {board}"),
        "replied" => format!("replied to {board}"),
        "received" => format!("{board} asks"),
        "got_reply" => format!("{board} replied"),
        "refused" => match &t.words {
            Some(why) => format!("{board} {}", scrub_cells(why, false)),
            None => format!("{board} refused"),
        },
        _ => board,
    }
}

impl App {
    /// The list's rows, in the order drawn.
    pub(crate) fn board_rows(&self) -> Vec<BoardRow> {
        let mut rows: Vec<BoardRow> = self.links.iter().cloned().map(BoardRow::Linked).collect();
        match &self.open_boards {
            None => rows.push(BoardRow::Scanning),
            Some(open) => rows.extend(
                open.iter()
                    .filter(|o| !self.links.iter().any(|l| l.path == o.path))
                    .cloned()
                    .map(BoardRow::Open),
            ),
        }
        rows.push(BoardRow::Folder);
        rows
    }

    /// A row's label and detail, in the list's words. `armed` is a linked
    /// row's first press.
    pub(crate) fn board_row_words(&self, row: &BoardRow, armed: bool) -> (String, String) {
        match row {
            BoardRow::Linked(l) => (
                scrub_cells(&l.name, false),
                if armed {
                    format!("{} ∙ enter again confirms", scrub_cells(&l.path, false))
                } else {
                    format!(
                        "{} ∙ reaches this board, and this board reaches it",
                        scrub_cells(&l.path, false)
                    )
                },
            ),
            BoardRow::Open(o) => {
                let crown = match &o.crown {
                    Some(key) => format!("crown {}", scrub_cells(key, false)),
                    None => "no crown yet".to_string(),
                };
                (
                    format!("{}  open now", scrub_cells(&o.name, false)),
                    format!("{} ∙ {crown}", scrub_cells(&o.path, false)),
                )
            }
            BoardRow::Scanning => ("looking for open boards…".into(), String::new()),
            BoardRow::Folder => {
                ("a folder…".into(), "the repository of a board that is not open now".into())
            }
        }
    }

    /// The boards list's words on `Ctx`: the door's label and Enter's word.
    pub(super) fn fill_boards_ctx(&self, c: &mut Ctx) {
        c.crown_links = self.links.iter().map(|l| l.name.clone()).collect::<Vec<_>>().join(" ");
        if let Mode::Boards { idx, typing: None, armed } = &self.mode {
            c.boards_enter_word = match self.board_rows().get(*idx) {
                Some(BoardRow::Linked(_)) => {
                    if *armed {
                        "confirm unlink"
                    } else {
                        "unlink"
                    }
                }
                Some(BoardRow::Open(_)) => "link",
                Some(BoardRow::Folder) => "type a folder",
                Some(BoardRow::Scanning) | None => "",
            };
        }
    }

    /// Open the list and go looking for open boards, through the seam
    /// `lib.rs::run` set; a test app has none and lists nothing open.
    pub(super) fn open_boards_list(&mut self) {
        self.mode = Mode::Boards { idx: 0, typing: None, armed: false };
        match self.board_scanner {
            Some(scan) => {
                self.open_boards = None;
                self.boards_rx = Some(scan(self.repo_root.clone()));
            }
            None => self.open_boards = Some(Vec::new()),
        }
    }

    /// The walk's answer, when it comes (`tick`). True when the list changed.
    pub(crate) fn poll_boards(&mut self) -> bool {
        let Some(rx) = &self.boards_rx else { return false };
        let found = match rx.try_recv() {
            Ok(list) => list,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => Vec::new(),
        };
        self.boards_rx = None;
        self.open_boards = Some(found);
        true
    }

    /// Enter on the list: link, arm and unlink, or open the folder field.
    pub(super) fn boards_act(&mut self) -> Result<()> {
        let Mode::Boards { idx, typing: None, armed } = self.mode.clone() else { return Ok(()) };
        match self.board_rows().get(idx).cloned() {
            Some(BoardRow::Linked(l)) => {
                if !armed {
                    self.mode = Mode::Boards { idx, typing: None, armed: true };
                    self.status = format!("unlink {} ∙ enter again confirms", l.name);
                    return Ok(());
                }
                match self.client.request(Command::UnlinkBoard { path: l.path })? {
                    Response::Err { message } => self.status = message,
                    _ => {
                        self.refresh()?;
                        self.status = format!("{} unlinked ∙ for both boards", l.name);
                    }
                }
                let n = self.board_rows().len().max(1);
                self.mode = Mode::Boards { idx: idx.min(n - 1), typing: None, armed: false };
            }
            Some(BoardRow::Open(o)) => {
                self.link(o.path)?;
                self.mode = Mode::Boards { idx: 0, typing: None, armed: false };
            }
            Some(BoardRow::Folder) => {
                self.mode =
                    Mode::Boards { idx, typing: Some(EditBuffer::new(FOLDER_MAX)), armed: false };
                self.status = "the path of a board's repository ∙ enter links it".into();
            }
            Some(BoardRow::Scanning) | None => {}
        }
        Ok(())
    }

    /// One link, and what the status says of it.
    fn link(&mut self, path: String) -> Result<()> {
        match self.client.request(Command::LinkBoard { path })? {
            Response::Err { message } => self.status = message,
            _ => {
                self.refresh()?;
                let name = self.links.last().map(|l| l.name.clone()).unwrap_or_default();
                self.status =
                    format!("{name} linked ∙ its crown and this board's reach each other");
            }
        }
        Ok(())
    }

    /// The folder being typed: the raw key edits the buffer, then Enter
    /// links it and Esc drops it, against `Scope::Input`.
    pub(super) fn key_board_field(&mut self, code: KeyCode, mods: KeyModifiers) -> Result<()> {
        let Mode::Boards { typing: Some(buf), .. } = &mut self.mode else { return Ok(()) };
        edit_buffer_key(buf, code, mods);
        let ctx = self.ctx();
        let verb = crate::keys::to_key_text(code, mods)
            .and_then(|k| keymap::resolve(Scope::Input, k, &ctx));
        match verb {
            Some(Verb::Save | Verb::SaveStart) => self.commit_board_field(),
            Some(Verb::Cancel) => {
                if let Mode::Boards { typing, .. } = &mut self.mode {
                    *typing = None;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn commit_board_field(&mut self) -> Result<()> {
        let Mode::Boards { idx, typing: Some(buf), .. } = self.mode.clone() else { return Ok(()) };
        let typed = buf.as_str().trim().to_string();
        if typed.is_empty() {
            self.mode = Mode::Boards { idx, typing: None, armed: false };
            return Ok(());
        }
        // `~` is the shell's, not the daemon's.
        let path = match (typed.strip_prefix("~/"), std::env::var_os("HOME")) {
            (Some(rest), Some(home)) => Path::new(&home).join(rest).to_string_lossy().into_owned(),
            _ => typed,
        };
        let before = self.links.len();
        self.link(path)?;
        if self.links.len() > before {
            self.mode = Mode::Boards { idx: 0, typing: None, armed: false };
        }
        Ok(())
    }

    /// The board touches off a snapshot (T-733): a touch first seen fresh
    /// lights the header's word and strikes once; one already past its beat
    /// — a board opened after it — does nothing.
    pub(super) fn absorb_board_touches(&mut self, touches: Vec<BoardTouch>) {
        let now = (self.now)();
        for t in &touches {
            if self
                .board_touches
                .iter()
                .any(|k| k.at_ms == t.at_ms && k.path == t.path && k.action == t.action)
            {
                continue;
            }
            if now.saturating_sub(t.at_ms) >= crate::theme::LINK_LIT_MS {
                continue;
            }
            self.link_lit = Some(LinkLit {
                board: t.board.clone(),
                words: link_words(t),
                refused: t.action == "refused",
                seen: now,
            });
            if t.action != "refused" {
                self.link_strikes.push(LinkStrike {
                    board: t.board.clone(),
                    up: matches!(t.action.as_str(), "sent" | "replied"),
                    crown: t.crown,
                    at_ms: t.at_ms,
                    seen: now,
                });
            }
        }
        self.link_strikes.retain(|s| now.saturating_sub(s.seen) < crate::strike::BOLT_MS);
        self.board_touches = touches;
    }

    /// The header's lit word, while it lasts.
    pub(crate) fn link_lit(&self) -> Option<&LinkLit> {
        let lit = self.link_lit.as_ref()?;
        ((self.now)().saturating_sub(lit.seen) < crate::theme::LINK_LIT_MS).then_some(lit)
    }

    /// Is a bolt between the crown and a board's name mid-flight?
    pub(crate) fn link_striking(&self) -> bool {
        let now = mesimon_core::clock::now_ms();
        self.motion()
            && matches!(self.screen, Screen::Board)
            && self.link_strikes.iter().any(|s| s.moving(now))
    }
}
