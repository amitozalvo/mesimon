//! Explicitly shared shell output and human-confirmed crown close requests.
use super::*;
use mesimon_core::board::ShellCloseRequest;
use mesimon_core::command::{AgentShellView, ShellOutput};

const OUTPUT_BYTES: usize = 16 * 1024;

impl Daemon {
    pub(super) fn revoke_shell_close_requests(&mut self) {
        let mut changed = false;
        for rec in &mut self.board.sessions {
            changed |= rec.shell_close_request.take().is_some();
        }
        if changed {
            self.persist_sessions();
        }
    }

    pub(super) fn agent_shells(&self, ticket: ulid::Ulid) -> Vec<AgentShellView> {
        self.board
            .sessions
            .iter()
            .filter(|r| r.ticket == ticket && r.kind == SessionKind::Bash && r.state.has_pane())
            .map(|r| AgentShellView {
                shell: r.id,
                shared: r.shell_shared,
                foreground: r.shell_shared.then(|| self.foregrounds.get(&r.id).cloned()).flatten(),
                close_requested: r
                    .shell_close_request
                    .as_ref()
                    .is_some_and(|q| q.expires_ms > now_ms()),
            })
            .collect()
    }

    /// Validate kind, ownership and the pane generation before any capture or close.
    fn shell_pane(&self, ticket: ulid::Ulid, id: uuid::Uuid) -> Result<(String, String), String> {
        let rec = self
            .board
            .sessions
            .iter()
            .find(|r| {
                r.id == id
                    && r.ticket == ticket
                    && r.kind == SessionKind::Bash
                    && r.state.has_pane()
            })
            .ok_or("no live shell on this ticket")?;
        let name = format!("={}:0.0", rec.sid16());
        let key = self.backend.pane_key(&name).map_err(|_| "shell is unavailable")?;
        if rec.pane_key.as_ref() != Some(&key) {
            return Err("shell changed; read the ticket again".into());
        }
        let pane = key
            .split_once(':')
            .map(|(_, pane)| pane.to_owned())
            .filter(|pane| pane.starts_with('%'))
            .ok_or("shell identity is unavailable")?;
        Ok((key, pane))
    }

    pub(super) fn share_shell(&mut self, id: uuid::Uuid, shared: bool) -> Response {
        let Some(rec) = self
            .board
            .sessions
            .iter_mut()
            .find(|r| r.id == id && r.kind == SessionKind::Bash && r.state.has_pane())
        else {
            return Response::Err { message: "no live shell".into() };
        };
        rec.shell_shared = shared;
        self.persist_and_notify();
        Response::Ok
    }

    fn capture_shell_output(
        &self,
        ticket: ulid::Ulid,
        id: uuid::Uuid,
    ) -> Result<ShellOutput, String> {
        let (key, pane) = self.shell_pane(ticket, id)?;
        let (raw, alternate) = self.backend.capture_shell(&pane).map_err(|e| e.to_string())?;
        if self.shell_pane(ticket, id)?.0 != key {
            return Err("shell changed during capture; try again".into());
        }
        let rows: Vec<_> = raw.lines().collect();
        let clean =
            mesimon_core::text::scrub_lines(&rows[rows.len().saturating_sub(200)..].join("\n"));
        let truncated = clean.len() > OUTPUT_BYTES || rows.len() >= 200;
        let mut from = clean.len().saturating_sub(OUTPUT_BYTES);
        while !clean.is_char_boundary(from) {
            from += 1;
        }
        Ok(ShellOutput {
            shell: id,
            captured_at_ms: now_ms(),
            source: if alternate { "terminal_screen" } else { "terminal_scrollback" }.into(),
            truncated,
            text: clean[from..].to_owned(),
        })
    }

    pub(super) fn local_shell_capture(&self, id: uuid::Uuid) -> Response {
        let Some(ticket) = self.board.sessions.iter().find(|r| r.id == id).map(|r| r.ticket) else {
            return Response::Err { message: "no live shell".into() };
        };
        self.capture_shell_output(ticket, id).map_or_else(
            |message| Response::Err { message },
            |output| Response::ShellOutput { output },
        )
    }

    pub(super) fn agent_read_shell(
        &self,
        session: uuid::Uuid,
        own: ulid::Ulid,
        shell: uuid::Uuid,
        key: Option<String>,
    ) -> Response {
        let target = match key {
            Some(key) => match self.crown_target(own, &key, false) {
                Ok(t) => t,
                Err(message) => return Response::Err { message },
            },
            None => own,
        };
        let by = Principal::Agent { session };
        if let Decision::Deny { reason } =
            authorize(&by, &Action::ReadShell, &Resource::Shell { ticket: target, id: shell })
        {
            return Response::Err { message: reason };
        }
        if !self.board.sessions.iter().any(|r| {
            r.id == shell && r.ticket == target && r.kind == SessionKind::Bash && r.shell_shared
        }) {
            return Response::Err { message: "no shared shell on this ticket; the person shares it with ^s or Shift+Enter on its ticket-page row".into() };
        }
        self.capture_shell_output(target, shell).map_or_else(
            |message| Response::Err { message },
            |output| Response::ShellOutput { output },
        )
    }

    pub(super) fn shell_close_ready(&self, rec: &SessionRecord) -> Result<(), String> {
        let (_, pane) = self.shell_pane(rec.ticket, rec.id)?;
        if self.focus_held() == Some(Focus::Session(rec.id))
            || self
                .backend
                .has_clients(&format!("={}", rec.sid16()))
                .map_err(|_| "could not inspect shell clients")?
        {
            return Err("shell is attached; return to the board, or exit it yourself".into());
        }
        let facts = self.backend.pane_facts().map_err(|_| "could not inspect shell activity")?;
        let fact = facts
            .iter()
            .find(|p| p.session_name == rec.sid16() && !p.pane_dead)
            .ok_or("shell activity is unknown; open it and exit it yourself")?;
        let kids = live_children(fact.pane_pid)?;
        if !kids.is_empty() {
            return Err(format!(
                "shell has live children ({}); open it, finish or stop its program, then exit",
                kids.join(", ")
            ));
        }
        if mesimon_core::board::foreground_of(
            &fact.current_command,
            rec.argv.first().map(String::as_str).unwrap_or(""),
        )
        .is_some()
        {
            return Err(
                "shell is running a program; open it, finish or stop the program, then exit".into(),
            );
        }
        // A second identity read also covers a process sample taken across a pane replacement.
        if self.shell_pane(rec.ticket, rec.id)?.1 != pane {
            return Err("shell changed; read the ticket again".into());
        }
        Ok(())
    }

    pub(super) fn request_close_shell(
        &mut self,
        session: uuid::Uuid,
        own: ulid::Ulid,
        shell: uuid::Uuid,
        key: &str,
    ) -> Response {
        if self.board.crown_holder().map(|t| t.id) != Some(own) {
            return Response::Err { message: "only the crown can request shell closure".into() };
        }
        let target = match self.crown_target(own, key, false) {
            Ok(t) => t,
            Err(message) => return Response::Err { message },
        };
        let by = Principal::Agent { session };
        if let Decision::Deny { reason } = authorize(
            &by,
            &Action::RequestCloseShell,
            &Resource::Shell { ticket: target, id: shell },
        ) {
            return Response::Err { message: reason };
        }
        let (pane, _) = match self.shell_pane(target, shell) {
            Ok(p) => p,
            Err(message) => return Response::Err { message },
        };
        let Some(rec) = self.board.sessions.iter().find(|r| r.id == shell) else {
            return no_such_ticket();
        };
        if let Err(message) = self.shell_close_ready(rec) {
            return Response::Err { message };
        }
        // Repeated requests keep the same approval and deadline.
        if !rec
            .shell_close_request
            .as_ref()
            .is_some_and(|q| q.expires_ms > now_ms() && q.crown == own && q.pane == pane)
        {
            let request = ShellCloseRequest {
                id: uuid::Uuid::new_v4(),
                crown: own,
                pane,
                expires_ms: now_ms() + 300_000,
            };
            if let Some(rec) = self.board.sessions.iter_mut().find(|r| r.id == shell) {
                rec.shell_close_request = Some(request);
            }
            self.feed.board(by.actor(), "request_close_shell", Some(target));
            self.crown_touched(own, target, "close shell?");
            self.persist_and_notify();
        }
        self.agent_ticket_view(target)
            .map_or_else(no_such_ticket, |ticket| Response::AgentTicket { ticket })
    }

    pub(super) fn confirm_close_shell(
        &mut self,
        id: uuid::Uuid,
        request: uuid::Uuid,
        close: bool,
    ) -> Response {
        let Some(rec) =
            self.board.sessions.iter().find(|r| r.id == id && r.kind == SessionKind::Bash)
        else {
            return Response::Err { message: "no live shell".into() };
        };
        let Some(q) = rec.shell_close_request.as_ref().filter(|q| q.id == request) else {
            return Response::Err {
                message: "close request changed; read the ticket again".into(),
            };
        };
        if close {
            if q.expires_ms <= now_ms()
                || self.board.crown_holder().map(|t| t.id) != Some(q.crown)
                || self.shell_pane(rec.ticket, id).map(|(key, _)| key != q.pane).unwrap_or(true)
            {
                return Response::Err {
                    message: "close request expired or shell changed; nothing closed".into(),
                };
            }
            if let Err(message) = self.sleep_one(id, false, Park::Deliberate) {
                return Response::Err { message };
            }
        } else if let Some(rec) = self.board.sessions.iter_mut().find(|r| r.id == id) {
            rec.shell_close_request = None;
        }
        self.persist_and_notify();
        Response::Ok
    }

    pub(super) fn shell_blocker(&self, ticket: ulid::Ulid) -> Option<String> {
        if self
            .board
            .sessions
            .iter()
            .any(|r| r.ticket == ticket && r.kind == SessionKind::Bash && r.state.has_pane())
        {
            return Some("shell still open — select its row on the ticket page; x closes it and loses its terminal history. If a program is running, open the shell, finish or stop it, then exit".into());
        }
        self.terminals.contains_key(&Some(ticket)).then(|| "terminal still open — ! opens it; finish or stop its program, then exit before parking or archiving this ticket".into())
    }
}

/// Only pgrep's explicit no-match result proves an empty child list.
pub(super) fn parse_children(status: Option<i32>, bytes: &[u8]) -> Result<Vec<String>, String> {
    let unknown =
        || "could not inspect shell jobs; open the shell and exit it yourself".to_string();
    let text = std::str::from_utf8(bytes).map_err(|_| unknown())?;
    if status == Some(1) && text.trim().is_empty() {
        return Ok(Vec::new());
    }
    if status != Some(0) || text.trim().is_empty() {
        return Err(unknown());
    }
    text.lines()
        .map(|line| {
            let mut words = line.split_whitespace();
            if !words.next().and_then(|p| p.parse::<u32>().ok()).is_some_and(|pid| pid > 0) {
                return Err(unknown());
            }
            words
                .next()
                .map(mesimon_core::text::scrub_text)
                .filter(|name| !name.is_empty())
                .ok_or_else(unknown)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_malformed_process_evidence_is_not_an_idle_shell() {
        assert_eq!(parse_children(Some(1), b"").unwrap(), Vec::<String>::new());
        assert_eq!(parse_children(Some(0), b"42 vim\n43 sleep\n").unwrap(), ["vim", "sleep"]);
        for (status, text) in [
            (Some(2), &b""[..]),
            (None, &b""[..]),
            (Some(0), &b""[..]),
            (Some(0), &b"42"[..]),
            (Some(0), &b"not-a-pid vim"[..]),
            (Some(1), &b"42 vim"[..]),
        ] {
            assert!(parse_children(status, text).is_err());
        }
    }
}
