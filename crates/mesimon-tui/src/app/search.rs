//! Keep the inputs a rank actually reads. A busy worker finishes its pass,
//! then takes the latest query and inputs still wanted by the open picker.
use super::{App, Mode, Screen, Search, CORPUS_BACKOFF};
use mesimon_core::search::{Hit, PullRequest, Rows, Searcher};
use std::sync::{mpsc, Arc};

#[derive(Default)]
pub(super) struct Ranking {
    matcher: Option<Searcher>,
    pub(super) active: Option<mpsc::Receiver<(Searcher, Vec<Hit>)>>,
    started: u64,
    wanted: u64,
}

impl App {
    /// Called on snapshot and archive landings. Compare the same ordered
    /// rows the matcher consumes, including note revisions and forge text.
    pub(super) fn refresh_search_rows(&mut self) -> bool {
        let Mode::Search(s) = &self.mode else { return false };
        let archived = s.archived;
        let prs = self.all_prs().map(|p| PullRequest {
            ticket: p.ticket,
            text: format!("{} {} {} {} {}", p.label(), p.title, p.repo, p.state, p.url),
        });
        let rows = Rows::new(&self.board, archived, prs);
        if s.rows.as_deref() == Some(&rows) {
            return self.corpus_refused.is_some_and(|t| t.elapsed() >= CORPUS_BACKOFF)
                && self.poll_bodies(archived);
        }
        if let Mode::Search(s) = &mut self.mode {
            s.rows = Some(Arc::new(rows));
        }
        self.poll_bodies(archived);
        true
    }

    pub(super) fn search_board_changed(&mut self) {
        if self.refresh_search_rows() {
            self.research(true);
        }
    }

    pub(super) fn research(&mut self, keep: bool) {
        if matches!(&self.mode, Mode::Search(s) if s.rows.is_none()) {
            self.refresh_search_rows();
        }
        let Mode::Search(s) = &mut self.mode else { return };
        if !keep {
            s.held = None;
            s.accept = false;
        } else if s.pending.is_none() {
            // While a query edit is pending, the visible cursor still belongs
            // to the old query. A snapshot must preserve the edit's top pick.
            s.held = s.selected().map(|h| h.id);
        }
        self.ranking.wanted += 1;
        s.pending = Some(self.ranking.wanted);
        // Keep the last completed list and cursor on screen until land_ranking
        // replaces them together. Enter waits on the pending generation.
        self.start_ranking();
    }

    fn start_ranking(&mut self) {
        let Mode::Search(s) = &self.mode else { return };
        let (Some(rows), Some(id)) = (&s.rows, s.pending) else { return };
        if self.ranking.active.is_some() {
            return;
        }
        let mut matcher = self.ranking.matcher.take().unwrap_or_default();
        // Keep tiny boards immediate. A populated board or corpus always
        // ranks on the worker, including on the first letter and on paste.
        if self.board.tickets.len() <= 64 && self.corpus.0.len() <= 8 {
            let hits = matcher.rank_rows(rows, s.query.as_str(), self.corpus.as_ref());
            self.ranking.matcher = Some(matcher);
            self.land_ranking(hits);
            return;
        }

        let rows = Arc::clone(rows);
        let corpus = Arc::clone(&self.corpus);
        let query = s.query.as_str().to_owned();
        let (send, recv) = mpsc::channel();
        self.ranking.active = Some(recv);
        self.ranking.started = id;
        std::thread::spawn(move || {
            let hits = matcher.rank_rows(&rows, &query, corpus.as_ref());
            let _ = send.send((matcher, hits));
        });
    }

    pub(super) fn poll_ranking(&mut self) -> bool {
        let Some(active) = &self.ranking.active else { return false };
        let result = match active.try_recv() {
            Err(mpsc::TryRecvError::Empty) => return false,
            result => result,
        };
        self.ranking.active = None;
        if let Ok((matcher, hits)) = result {
            self.ranking.matcher = Some(matcher);
            if matches!(&self.mode, Mode::Search(s) if s.pending == Some(self.ranking.started)) {
                self.land_ranking(hits);
                return true;
            }
        }
        // All intermediate keystrokes collapse into the open picker's
        // current query. Closing the picker drops its interest entirely.
        let pending = matches!(&self.mode, Mode::Search(s) if s.pending.is_some());
        self.start_ranking();
        pending
    }

    fn land_ranking(&mut self, hits: Vec<Hit>) {
        let Mode::Search(s) = &mut self.mode else { return };
        s.pending = None;
        s.hits = hits;
        let recent_tickets = &self.recent_tickets;
        // Nothing typed: the pages opened this run, newest first, instead of
        // the whole board (T-355). The rank pass still decides what is IN —
        // a ticket deleted since, or archived while `tab` hides the archive,
        // is not a row — and the first keystroke is the board again.
        s.recent = s.query.as_str().trim().is_empty()
            && recent_tickets.iter().any(|id| s.hits.iter().any(|h| h.id == *id));
        if s.recent {
            let mut hits = std::mem::take(&mut s.hits);
            s.hits = recent_tickets
                .iter()
                .filter_map(|id| hits.iter().position(|h| h.id == *id).map(|i| hits.swap_remove(i)))
                .collect();
        }
        s.idx = s
            .held
            .take()
            .and_then(|id| s.hits.iter().position(|h| h.id == id))
            .unwrap_or(0)
            .min(s.hits.len().saturating_sub(1));
        if std::mem::take(&mut s.accept) && s.selected().is_some() {
            let Mode::Search(s) = std::mem::replace(&mut self.mode, Mode::Normal) else { return };
            self.accept_search(&s);
        }
    }

    pub(super) fn accept_search(&mut self, s: &Search) {
        let Some(hit) = s.selected() else { return };
        if hit.archived {
            self.screen = Screen::Ticket { ticket: hit.id, rail_idx: 0 };
        } else {
            self.select_ticket(hit.id);
            self.header_focus = false;
        }
    }
}
