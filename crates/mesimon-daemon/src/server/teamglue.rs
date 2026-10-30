//! Board sharing on the writer (T-215 v1). Every decision about the relay
//! is made here, on the single writer, with the board in hand: what to send,
//! what a result means, how a record becomes a ticket. The executor thread in
//! `crate::team::sync` only carries frames.
//!
//! Outgoing changes are found by diffing, not by instrumenting every
//! mutation: `team_after_broadcast` runs at the end of `broadcast()`, projects
//! the board, and queues every object whose digest moved. Incoming records
//! retain a durable revision floor and encrypted intent before content writes.
//! Publication finishes only after the content and sharing-state writes succeed.
use super::*;
use crate::team::device::DeviceFile;
use crate::team::project;
use crate::team::state::{
    LocalSnapshot, PendingInvite, PendingRecord, Published, TeamState, VerifiedMember,
};
use crate::team::sync::{Done, Job, Tag};
use mesimon_core::board::{
    note_name, sanitize_note, sanitize_title, Archived, ExecutionPolicy, NoteMeta,
};
use mesimon_core::team::{
    RecordBody, SharedObject, SyncState, TeamBoard, TeamBoardSummary, TeamDevice, TeamInfo,
    TeamMember, MEMBERS_OBJECT, RECORD_SCHEMA,
};
use mesimon_team::crypto::{self, BoardId, BoardKey, DeviceId, ObjectId, OperationId, RecordScope};
use mesimon_team::invite::InviteCode;
use mesimon_team::relay::RelayEndpoint;
use mesimon_team::wire::{
    BoardSummary, ErrorCode, Member, MemberStatus, Request, Response as Wire, Role, StoredRecord,
    MAX_SYNC_RECORDS,
};
use std::collections::HashSet;
use std::path::PathBuf;

/// Ticks between pulls (250 ms each): 3 s. `MESIMON_TEAM_SYNC_MS` overrides
/// for tests.
fn pull_every() -> u64 {
    std::env::var("MESIMON_TEAM_SYNC_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map_or(12, |ms| (ms / TICK_MS).max(1))
}
const MEMBERS_EVERY: u64 = 120;
const MAX_PENDING_RECORDS: usize = 1024;
const MAX_PENDING_BYTES: usize = 16 * 1024 * 1024;
const MAX_RETRY_RECORDS: usize = 64;
const OFFLINE_BACKOFF: u64 = 20;
/// Ticks between asking the relay when this device's grant ends: an hour.
const GRANT_EVERY: u64 = 3_600_000 / TICK_MS;
/// A grant is renewed by itself inside its last day (T-522), in seconds.
/// A paid grant runs a little past the paid period, so by then the renewal
/// charge has landed and a `Redeem` grants the next period.
const RENEW_WINDOW: u64 = 86_400;
/// Renewals by itself go at least this far apart, in seconds, and under the
/// relay's one-day floor: a grant extended a day at a time is renewed before
/// it runs out, and a cancelled one is asked a bounded number of times before
/// its no.
const RENEW_AGAIN: u64 = 6 * 3_600;
const _: () = assert!(RENEW_AGAIN < RENEW_WINDOW);
/// What a share or a join answers while board sharing is off
/// (`mesimon_core::team::enabled`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ApplyOutcome {
    Applied,
    Deferred,
    Rejected,
}

const TEAMS_OFF: &str = "board sharing is off; start mesimon with MESIMON_TEAMS=1 to use it";

pub(super) struct TeamCtx {
    jobs: Sender<Job>,
    pub(super) device: Option<DeviceFile>,
    state: Option<TeamState>,
    inflight: HashSet<Tag>,
    busy: Option<&'static str>,
    error: Option<String>,
    boards: Vec<BoardSummary>,
    sync_word: &'static str,
    synced_at_ms: Option<u64>,
    last_pull: u64,
    last_members: u64,
    backoff_until: u64,
    applying: bool,
    /// note id → (rev, ticket): what the relay has of each note body.
    note_revs: HashMap<ulid::Ulid, (u64, ulid::Ulid)>,
    /// Visible projections while an I/O-failed intent waits. An actual local
    /// change still queues; the unchanged snapshot must not echo as a new edit.
    blocked_digests: HashMap<String, Option<String>>,
    retention_full: bool,
    retry_at: usize,
    #[cfg(test)]
    before_record_finish: Option<fn(&mut Daemon)>,
    want_keys: bool,
    want_members: bool,
    /// The invite being minted, until the relay names it.
    minting: Option<(InviteCode, Role)>,
    /// The share in flight keeps its notes, or not — decided by the person
    /// and recorded on the state once the relay names the board.
    share_notes: bool,
    /// The code being redeemed, to check the owner the relay names.
    joining: Option<InviteCode>,
    /// The key of the next epoch, until the relay accepts it.
    rotating: Option<(u32, BoardKey)>,
    /// Rotate once the next member list lands: a rotation must name exactly
    /// the members the relay still counts, so it never works from a cache.
    rotate_after_members: bool,
    state_dirty: bool,
    /// The relay wants an access code at sign-in (T-515).
    code_required: bool,
    /// A write was refused for a lapsed grant; cleared when a code lands.
    lapsed: bool,
    /// The last code was accepted.
    granted: bool,
    /// A code typed at sign-in on a device the relay already knew: the
    /// register answers `Denied` (known key) and the code goes as `Redeem`.
    redeem_after_sign_in: bool,
    /// The one renewal tried by itself after a lapse, with the stored code;
    /// a second needs a person. Reset when a code is accepted.
    renewal_tried: bool,
    /// When this device's grant ends, as the relay last said (T-522): `None`
    /// until it answers `Grant`, and always on a relay from before, which
    /// cannot; `Some(None)` is a grant with no end.
    grant_until: Option<Option<u64>>,
    /// The tick `Grant` was last asked on.
    last_grant: u64,
    /// The earliest a renewal by itself may go again, in unix seconds.
    renew_after: u64,
}

impl TeamCtx {
    pub(super) fn new(tx: Sender<Msg>) -> Self {
        Self::with_jobs(crate::team::sync::spawn(move |done| {
            let _ = tx.send(Msg::Team(done));
        }))
    }

    fn with_jobs(jobs: Sender<Job>) -> Self {
        Self {
            jobs,
            device: None,
            state: None,
            inflight: HashSet::new(),
            busy: None,
            error: None,
            boards: Vec::new(),
            sync_word: "offline",
            synced_at_ms: None,
            last_pull: 0,
            last_members: 0,
            backoff_until: 0,
            applying: false,
            note_revs: HashMap::new(),
            blocked_digests: HashMap::new(),
            retention_full: false,
            retry_at: 0,
            #[cfg(test)]
            before_record_finish: None,
            want_keys: false,
            want_members: false,
            minting: None,
            share_notes: true,
            joining: None,
            rotating: None,
            rotate_after_members: false,
            state_dirty: false,
            code_required: false,
            lapsed: false,
            granted: false,
            redeem_after_sign_in: false,
            renewal_tried: false,
            grant_until: None,
            last_grant: 0,
            renew_after: 0,
        }
    }

    fn call(&mut self, tag: Tag, request: Request) {
        if self.inflight.contains(&tag) {
            return;
        }
        self.inflight.insert(tag.clone());
        let _ = self.jobs.send(Job::Call { tag, request });
    }

    fn connect(&mut self) {
        if let Some(d) = &self.device {
            let _ = self
                .jobs
                .send(Job::Connect { endpoint: d.relay.clone(), credential: d.credential.clone() });
        }
    }

    fn signed_in(&self) -> bool {
        self.device.as_ref().is_some_and(|d| d.credential.is_some())
    }

    fn board(&self) -> Option<BoardId> {
        self.state.as_ref().and_then(TeamState::board_id)
    }

    fn put_inflight(&self) -> Option<&str> {
        self.inflight.iter().find_map(|t| match t {
            Tag::Put(op) => Some(op.as_str()),
            _ => None,
        })
    }

    fn fail(&mut self, what: &str, code: ErrorCode) {
        self.busy = None;
        self.error = Some(format!("{what}: {code}"));
    }

    /// The code kept for renewing by itself (`device.toml`), if any.
    fn stored_code(&self) -> Option<String> {
        self.device.as_ref().and_then(|d| d.access_code.clone())
    }

    /// Ask when the grant ends. Only a device with a kept code asks: without
    /// one there is nothing to renew with.
    fn ask_grant(&mut self, ticks: u64) {
        if self.stored_code().is_some() {
            self.last_grant = ticks;
            self.call(Tag::Grant, Request::Grant);
        }
    }

    /// The grant's own wheel (T-522), shared board or not — Remote Control
    /// alone is reason enough: ask hourly when the grant ends, and renew by
    /// itself inside its last day, before the relay refuses the phone's mail.
    fn grant_tick(&mut self, ticks: u64, now: u64) {
        if ticks.saturating_sub(self.last_grant) >= GRANT_EVERY {
            self.ask_grant(ticks);
        }
        let ending = matches!(self.grant_until, Some(Some(until)) if now + RENEW_WINDOW >= until);
        if ending && now >= self.renew_after {
            self.renew(now);
        }
    }

    /// One renewal by itself with the kept code: a renewed subscription is
    /// the provider's word on the same key, and a friend's forever code never
    /// lapses. False when there is no code, a redeem is already out, or the
    /// last renewal failed and no code has been accepted since.
    fn renew(&mut self, now: u64) -> bool {
        let Some(code) = self.stored_code() else { return false };
        if self.renewal_tried || self.inflight.contains(&Tag::Redeem) {
            return false;
        }
        self.renewal_tried = true;
        self.renew_after = now + RENEW_AGAIN;
        self.call(Tag::Redeem, Request::Redeem { code });
        true
    }

    /// The relay refused for a lapsed grant — a write's `GrantLapsed`, or the
    /// phone's mail (`control::LAPSED`): say so, and renew once by itself.
    fn on_lapse(&mut self, now: u64) {
        self.lapsed = true;
        if !self.renew(now) {
            self.error = Some(format!("{}", ErrorCode::GrantLapsed));
        }
    }

    /// A code was accepted. The next renewal is judged by the grant's new
    /// end, so ask for it.
    fn redeemed(&mut self, ticks: u64) {
        self.busy = None;
        self.error = None;
        self.granted = true;
        self.lapsed = false;
        self.renewal_tried = false;
        self.ask_grant(ticks);
    }
}

fn err(message: impl Into<String>) -> Response {
    Response::Err { message: message.into() }
}

/// An access code as typed: trimmed, scrubbed, bounded like the relay
/// bounds it (`access::MAX_CODE_BYTES` there).
fn clean_code(code: &str) -> Result<String, &'static str> {
    let code = mesimon_core::text::scrub_text(code.trim());
    if code.is_empty() || code.len() > 128 {
        return Err("a license key is one to 128 characters");
    }
    Ok(code)
}

impl Daemon {
    /// At startup: load the identity and this board's sharing state, start
    /// the executor, and queue whatever changed while the daemon was down.
    pub(super) fn team_start(&mut self) {
        match Paths::team_device_file().and_then(|p| DeviceFile::load(&p)) {
            Ok(device) => self.team.device = device,
            Err(e) => self.team.error = Some(format!("device file: {e}")),
        }
        match TeamState::load(&self.paths.team_file()) {
            Ok(state) => self.team.state = state,
            Err(e) => {
                self.notices.push(
                    Notice::new("team_state", format!("board sharing is off: {e}"))
                        .with_path(self.paths.team_file().display()),
                );
            }
        }
        if let Some(state) = &mut self.team.state {
            if state.is_owner() && state.verified_owner.is_none() {
                state.verified_members.clear();
                if !state.members.is_empty() {
                    self.notices.push(Notice::new("team_identity", "board sharing needs fresh invitations: remove and reinvite existing members"));
                }
                if let Some(d) = &self.team.device {
                    if let Some(keys) = d.keys() {
                        state.pin_owner(keys.public(), d.display_name.clone());
                    }
                }
            }
            if state.needs_rejoin {
                self.notices.push(Notice::new(
                    "team_identity",
                    "board sharing needs a fresh invitation: leave this board and rejoin",
                ));
            }
        }
        self.team_save();
        if let Some(state) = &self.team.state {
            // What the relay already has of each note: every note whose
            // ticket object was published. Anything newer is queued below.
            for t in &self.board.tickets {
                for n in &t.notes {
                    if state.published.contains_key(&ObjectId::from(n.id).to_hex())
                        || state.pending_records.iter().any(|p| {
                            p.record.object == ObjectId::from(n.id) && p.publication.is_some()
                        })
                    {
                        self.team.note_revs.insert(n.id, (n.rev, t.id));
                    }
                }
            }
        }
        self.team_capture_blocked_digests();
        self.team.connect();
        self.team.want_members = true;
        self.team_retry_pending();
        self.team_after_broadcast();
        if self.team.signed_in() {
            self.team.call(Tag::Boards, Request::Boards);
            self.team.ask_grant(self.ticks);
        }
    }

    fn team_save(&mut self) -> bool {
        if let Some(state) = &self.team.state {
            if let Err(e) = state.save(&self.paths.team_file()) {
                self.team.error = Some(format!("saving sharing state: {e}"));
                self.team.state_dirty = true;
                return false;
            }
        }
        self.team.state_dirty = false;
        true
    }

    pub(super) fn team_defers_ticket_write(&self) -> bool {
        self.team.applying
    }

    pub(super) fn team_content_only(&self) -> bool {
        self.team.state.as_ref().is_some_and(|s| s.content_only)
    }

    /// A viewer reads and cannot edit (T-335): a command that would change
    /// a ticket or a note on a board this device only views is refused
    /// here, before it touches the board, with the reason and the way
    /// forward. The relay would refuse the record anyway (`Denied`), but by
    /// then the edit would stand on this copy alone — and the keymap has
    /// already stood every such key down, so this is the gate a second
    /// client meets, not a person.
    pub(super) fn team_read_only(&self, command: &Command) -> Option<String> {
        if !viewer_edit(command) {
            return None;
        }
        self.team_viewer_refusal()
    }

    /// The refusal a viewer's copy gives any edit, whichever road asked:
    /// the socket's commands above, and a ticket filed from a paired phone.
    pub(super) fn team_viewer_refusal(&self) -> Option<String> {
        let state = self.team.state.as_ref()?;
        (state.role() == Role::Viewer).then(|| {
            format!(
                "you view this board and cannot edit it ∙ ask {} for a contributor invite",
                state.owner_name
            )
        })
    }

    fn board_title(&self) -> String {
        self.paths
            .repo_root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    // ---- what the snapshot says -------------------------------------------

    pub(super) fn team_info(&self) -> TeamInfo {
        // The keys are a derivation (HKDF, Ed25519, X25519): once a snapshot.
        let me = self.team.device.as_ref().and_then(|d| d.keys()).map(|k| k.id());
        let device = self.team.device.as_ref().map(|d| TeamDevice {
            display_name: d.display_name.clone(),
            relay: d.relay.display(),
            device: me.map(|id| id.to_hex()).unwrap_or_default(),
            registered: d.credential.is_some(),
        });
        let board = self.team.state.as_ref().map(|s| TeamBoard {
            board: s.board.clone(),
            role: s.role.clone(),
            repository: !s.content_only,
            members: s
                .members
                .iter()
                .map(|m| {
                    let verified = s
                        .verified_members
                        .get(&m.device.to_hex())
                        .is_some_and(|p| p.active && p.public == m.public);
                    TeamMember {
                        device: m.device.to_hex(),
                        display_name: m.display_name.clone(),
                        role: s
                            .verified_members
                            .get(&m.device.to_hex())
                            .map_or(m.role, |p| p.role)
                            .word()
                            .into(),
                        status: m.status.word().into(),
                        pending: m.status == MemberStatus::Active
                            && m.epochs.is_empty()
                            && m.role != Role::Owner,
                        unverified: m.status == MemberStatus::Active && !verified,
                        me: Some(m.device) == me,
                    }
                })
                .collect(),
            sync: SyncState {
                // A lapse outranks the sync word: nothing goes out until a
                // code lands (`team_pump`), whatever the last pull said.
                state: if self.team.lapsed { "lapsed" } else { self.team.sync_word }.into(),
                drafts: s.outbox.len(),
                synced_at_ms: self.team.synced_at_ms,
                detail: (self.team.sync_word == "error").then(|| self.team.error.clone()).flatten(),
            },
            invite: s.invites.last().map(|i| i.code.clone()),
            owner_name: s.owner_name.clone(),
            notes_withheld: s.notes_withheld,
            edited_elsewhere: s
                .published
                .iter()
                .filter(|(_, p)| p.kind == "ticket" || p.kind == "note")
                .filter_map(|(k, p)| {
                    let by = p.by.clone()?;
                    let id = ulid::Ulid::from(ObjectId::parse(k)?);
                    Some((id.to_string(), by))
                })
                .collect(),
        });
        let boards = self
            .team
            .boards
            .iter()
            .map(|b| TeamBoardSummary {
                board: b.board.to_hex(),
                role: b.role.word().into(),
                owner_name: b.owner_name.clone(),
                root: Paths::board_root_for(&b.board.to_hex()).ok().filter(|r| r.is_dir()),
                title: self
                    .team
                    .state
                    .as_ref()
                    .filter(|s| s.board == b.board.to_hex())
                    .and_then(|s| s.title.clone()),
            })
            .collect();
        TeamInfo {
            device,
            board,
            boards,
            busy: self.team.busy.map(str::to_owned),
            error: self.team.error.clone(),
            code_required: self.team.code_required,
            lapsed: self.team.lapsed,
            granted: self.team.granted,
        }
    }

    // ---- the person's commands --------------------------------------------

    pub(super) fn team_sign_in(
        &mut self,
        relay: String,
        display_name: String,
        code: Option<String>,
    ) -> Response {
        let Some(endpoint) = RelayEndpoint::parse(&relay) else {
            return err("relay is host[:port], then a space and the pin for a self-hosted relay");
        };
        let name = mesimon_core::text::scrub_text(display_name.trim());
        if name.is_empty() || name.chars().count() > 64 {
            return err("a display name is one to sixty-four characters");
        }
        let code = match code.as_deref().map(clean_code) {
            Some(Err(message)) => return err(message),
            Some(Ok(code)) => Some(code),
            None => None,
        };
        // Keep the keys across sign-ins to the same relay: the boards this
        // device belongs to stay reachable. A new relay is a new identity.
        let known = self
            .team
            .device
            .as_ref()
            .is_some_and(|d| d.relay == endpoint && d.credential.is_some());
        let device = match self.team.device.take() {
            Some(d) if known => {
                DeviceFile { display_name: name, access_code: code.clone().or(d.access_code), ..d }
            }
            _ => DeviceFile { access_code: code.clone(), ..DeviceFile::fresh(name, endpoint) },
        };
        let Some(keys) = device.keys() else { return err("could not derive device keys") };
        if let Err(e) = Paths::team_device_file().and_then(|p| device.save(&p)) {
            return err(format!("could not save the identity: {e}"));
        }
        let display_name = device.display_name.clone();
        self.team.device = Some(device);
        self.team.error = None;
        self.team.code_required = false;
        self.team.granted = false;
        self.team.grant_until = None;
        self.team.redeem_after_sign_in = known && code.is_some();
        self.team.busy = Some("signing in");
        self.team.connect();
        // A key the relay already knows is refused with `Denied` and keeps
        // its credential; `on_team` reads that as "only the name changed"
        // and sends a code typed beside it as `Redeem`.
        self.team
            .call(Tag::SignIn, Request::Register { display_name, public: keys.public(), code });
        Response::Ok
    }

    /// An access code on the signed-in device (T-515). The code is kept in
    /// the device file so a lapse can renew by itself once.
    pub(super) fn team_redeem(&mut self, code: String) -> Response {
        if !self.team.signed_in() {
            return err("sign in to the relay first");
        }
        let code = match clean_code(&code) {
            Ok(code) => code,
            Err(message) => return err(message),
        };
        if let Some(d) = &mut self.team.device {
            d.access_code = Some(code.clone());
            let _ = Paths::team_device_file().and_then(|p| d.save(&p));
        }
        self.team.error = None;
        self.team.granted = false;
        self.team.busy = Some("redeeming");
        self.team.call(Tag::Redeem, Request::Redeem { code });
        Response::Ok
    }

    /// A write came back `GrantLapsed`, or the relay refused the phone's
    /// mail for this Mac's lapse (`control::LAPSED`, T-522).
    pub(super) fn team_on_lapse(&mut self) {
        self.team.on_lapse(now_secs());
    }

    pub(super) fn team_sign_out(&mut self) -> Response {
        if let Ok(path) = Paths::team_device_file() {
            let _ = std::fs::remove_file(path);
        }
        self.team.device = None;
        self.team.boards.clear();
        self.team.error = None;
        self.team.busy = None;
        self.team.grant_until = None;
        self.broadcast();
        Response::Ok
    }

    pub(super) fn team_share(&mut self, notes: bool) -> Response {
        if !mesimon_core::team::enabled() {
            return err(TEAMS_OFF);
        }
        if !self.team.signed_in() {
            return err("sign in to the relay first");
        }
        if self.team.state.is_some() {
            return err("this board is already shared");
        }
        if self.columns_barred {
            return err(self.barred_message("columns"));
        }
        self.team.share_notes = notes;
        self.team.error = None;
        self.team.busy = Some("sharing");
        self.team.call(Tag::Share, Request::CreateBoard);
        Response::Ok
    }

    pub(super) fn team_unshare(&mut self) -> Response {
        let Some(board) = self.team.board() else { return err("this board is not shared") };
        if !self.team.state.as_ref().is_some_and(TeamState::is_owner) {
            return err("only the owner can stop sharing; leave the board instead");
        }
        self.team.busy = Some("unsharing");
        self.team.call(Tag::Unshare, Request::Unshare { board });
        Response::Ok
    }

    pub(super) fn team_mint_invite(&mut self, role: String) -> Response {
        let Some(board) = self.team.board() else { return err("share the board first") };
        let Some(role) = Role::parse(&role).filter(|r| *r != Role::Owner) else {
            return err("an invite is for a contributor or a viewer");
        };
        if !self.team.state.as_ref().is_some_and(TeamState::is_owner) {
            return err("only the owner can invite");
        }
        let Some(keys) = self.team.device.as_ref().and_then(|d| d.keys()) else {
            return err("sign in first");
        };
        let code = InviteCode::mint(&keys.public());
        self.team.minting = Some((code.clone(), role));
        self.team.busy = Some("inviting");
        self.team.call(
            Tag::Invite,
            Request::MintInvite { board, role, secret_hash: code.secret_hash() },
        );
        Response::Ok
    }

    pub(super) fn team_revoke(&mut self, device: String) -> Response {
        let Some(board) = self.team.board() else { return err("this board is not shared") };
        let Some(device) = DeviceId::parse(&device) else { return err("no such member") };
        if !self.team.state.as_ref().is_some_and(TeamState::is_owner) {
            return err("only the owner can remove a member");
        }
        let state = self.team.state.as_mut().expect("checked board");
        if state.verified_owner.as_deref() == Some(device.to_hex().as_str()) {
            return err("the owner cannot remove itself");
        }
        if let Some(member) = state.verified_members.get_mut(&device.to_hex()) {
            member.active = false;
        }
        // Re-publish the current content under the owner's signature so a
        // later joiner can read work whose original author is now removed.
        // Keep revisions: only the projection digests need refreshing.
        for published in state.published.values_mut() {
            published.digest.clear();
        }
        self.team.note_revs.clear();
        if !self.team_save() {
            return err("could not save member removal; retry removing the member");
        }
        self.team_after_broadcast();
        self.team.busy = Some("removing");
        self.team.call(Tag::Revoke, Request::Revoke { board, device });
        Response::Ok
    }

    pub(super) fn team_join(&mut self, code: String) -> Response {
        if !mesimon_core::team::enabled() {
            return err(TEAMS_OFF);
        }
        if !self.team.signed_in() {
            return err("sign in to the relay first");
        }
        let Ok(code) = InviteCode::parse(&code) else {
            return err("an invite code is 32 letters and digits, in groups of four");
        };
        let Some(keys) = self.team.device.as_ref().and_then(|d| d.keys()) else {
            return err("sign in first");
        };
        let proof = code.proof(&keys.public());
        let secret = *code.secret();
        self.team.joining = Some(code);
        self.team.error = None;
        self.team.busy = Some("joining");
        self.team.call(Tag::Join, Request::Join { secret, proof });
        Response::Ok
    }

    pub(super) fn team_leave(&mut self) -> Response {
        let Some(board) = self.team.board() else { return err("this board is not shared") };
        if self.team.state.as_ref().is_some_and(TeamState::is_owner) {
            return err("the owner cannot leave; stop sharing instead");
        }
        self.team.busy = Some("leaving");
        self.team.call(Tag::Leave, Request::Leave { board });
        Response::Ok
    }

    pub(super) fn team_refresh(&mut self) -> Response {
        if !self.team.signed_in() {
            return err("sign in to the relay first");
        }
        self.team.call(Tag::Boards, Request::Boards);
        self.team.want_members = true;
        Response::Ok
    }

    // ---- the wheel ----------------------------------------------------------

    pub(super) fn team_tick(&mut self) {
        if !self.team.signed_in() || self.ticks < self.team.backoff_until {
            return;
        }
        self.team.grant_tick(self.ticks, now_secs());
        let Some(board) = self.team.board() else { return };
        // Board sharing off (`team::enabled`): a board shared or joined
        // earlier is not synced, so nothing is asked of the relay about it
        // and nothing it answers is applied. Unshare and leave still work.
        if !mesimon_core::team::enabled() {
            self.team.sync_word = "offline";
            return;
        }
        if self.team.state.as_ref().is_some_and(|s| s.needs_rejoin) {
            self.team.sync_word = "error";
            self.team.error = Some("leave this board and rejoin with a fresh invitation".into());
            return;
        }
        let ticks = self.ticks;
        // A join in progress is a conversation, not a heartbeat: the joiner
        // has no key yet and asks after one at every pull, and an owner with
        // an invite out looks for the joiner just as often.
        let pull_due = ticks.saturating_sub(self.team.last_pull) >= pull_every();
        if pull_due {
            self.team_retry_pending();
            if let Some(state) = &self.team.state {
                if state.current_epoch().is_none() {
                    self.team.want_members = true;
                    self.team.want_keys = true;
                } else if state.is_owner() && !state.invites.is_empty() {
                    self.team.want_members = true;
                }
            }
        }
        if self.team.want_keys && !self.team.inflight.contains(&Tag::MyKeys) {
            self.team.want_keys = false;
            self.team.call(Tag::MyKeys, Request::MyKeys { board });
        }
        if (self.team.want_members || ticks.saturating_sub(self.team.last_members) >= MEMBERS_EVERY)
            && !self.team.inflight.contains(&Tag::Members)
        {
            self.team.want_members = false;
            self.team.last_members = ticks;
            self.team.call(Tag::Members, Request::Members { board });
        }
        if pull_due
            && !self.team.inflight.contains(&Tag::Head)
            && !self.team.inflight.contains(&Tag::Sync)
        {
            self.team.last_pull = ticks;
            self.team.call(Tag::Head, Request::Head { board });
        }
        self.team_pump();
        if self.team.state_dirty {
            self.team_save();
        }
    }

    /// Send the next queued edit, if nothing is in flight and the key for
    /// the current epoch is in hand.
    fn team_pump(&mut self) {
        if self.team.put_inflight().is_some()
            || self.team.lapsed
            || self.team.sync_word == "frozen"
            || self.team.sync_word == "gone"
        {
            return;
        }
        if self.team.state_dirty && !self.team_save() {
            return;
        }
        let Some(keys) = self.team.device.as_ref().and_then(|d| d.keys()) else { return };
        let Some(state) = self.team.state.as_mut() else { return };
        let Some(board) = state.board_id() else { return };
        if state.needs_rejoin
            || !state
                .verified_members
                .get(&keys.id().to_hex())
                .is_some_and(|p| p.active && p.public == keys.public() && p.role.may_write())
        {
            return;
        }
        let Some(epoch) = state.current_epoch() else {
            self.team.want_keys = true;
            return;
        };
        let Some(key) = state.key(epoch) else { return };
        let Some(object) = state.outbox.first().map(|o| o.object.clone()) else { return };
        let expected = state.revision(&object);
        let Some(entry) = state.outbox.first_mut() else { return };
        // Expected is read now, not when queued: a pull in between may have
        // moved the object on, and the retry must name the revision it saw.
        entry.expected = expected;
        let Some(object) = ObjectId::parse(&entry.object) else {
            state.outbox.remove(0);
            return;
        };
        let Some(operation) = OperationId::parse(&entry.operation) else {
            state.outbox.remove(0);
            return;
        };
        let Some(revision) = entry.expected.unwrap_or(0).checked_add(1) else {
            self.team.error = Some("this object's revision cannot advance".into());
            return;
        };
        let scope = RecordScope { board, object, revision };
        let plaintext = serde_json::to_vec(&entry.body).unwrap_or_default();
        let record = match crypto::seal(&key, epoch, scope, &keys, &plaintext) {
            Ok(record) => record,
            Err(e) => {
                self.team.error = Some(format!("could not seal an edit: {e}"));
                state.outbox.remove(0);
                return;
            }
        };
        let expected = entry.expected;
        let tag = Tag::Put(entry.operation.clone());
        self.team.call(tag, Request::Put { board, operation, object, expected, record });
    }

    // ---- results ------------------------------------------------------------

    pub(super) fn on_team(&mut self, done: Done) {
        let Done { tag, result } = done;
        self.team.inflight.remove(&tag);
        if let Err(ErrorCode::Unavailable) = &result {
            // A renewal by itself the relay did not answer (it, or the
            // provider behind it, stumbled) was not a try: the next one
            // waits out `RENEW_AGAIN`, not a person.
            if tag == Tag::Redeem && self.team.busy != Some("redeeming") {
                self.team.renewal_tried = false;
            }
            self.team.sync_word = "offline";
            self.team.backoff_until = self.ticks + OFFLINE_BACKOFF;
            if self.team.busy.is_some() {
                self.team.busy = None;
                self.team.error = Some("the relay did not answer".into());
            }
            self.broadcast();
            return;
        }
        match (tag, result) {
            (Tag::SignIn, Ok(Wire::Registered { credential, .. })) => {
                if let Some(d) = &mut self.team.device {
                    d.credential = Some(credential);
                    let _ = Paths::team_device_file().and_then(|p| d.save(&p));
                }
                self.team.busy = None;
                self.team.granted =
                    self.team.device.as_ref().is_some_and(|d| d.access_code.is_some());
                self.team.lapsed = false;
                self.team.renewal_tried = false;
                self.team.connect();
                self.team.call(Tag::Boards, Request::Boards);
                self.team.ask_grant(self.ticks);
            }
            (Tag::SignIn, Err(ErrorCode::Denied))
                if self.team.device.as_ref().is_some_and(|d| d.credential.is_some()) =>
            {
                // Known key, kept credential: the name is all that changed —
                // and a code typed beside it goes on its own.
                self.team.busy = None;
                self.team.connect();
                if std::mem::take(&mut self.team.redeem_after_sign_in) {
                    if let Some(code) =
                        self.team.device.as_ref().and_then(|d| d.access_code.clone())
                    {
                        self.team.busy = Some("redeeming");
                        self.team.call(Tag::Redeem, Request::Redeem { code });
                    }
                }
            }
            (Tag::SignIn, Err(ErrorCode::CodeRequired)) => {
                self.team.code_required = true;
                self.team.fail("signing in", ErrorCode::CodeRequired);
            }
            (Tag::SignIn, Err(code)) => self.team.fail("signing in", code),
            (Tag::Redeem, Ok(_)) => {
                self.team.redeemed(self.ticks);
                // The edits that waited go again.
                self.team_after_broadcast();
            }
            (Tag::Redeem, Err(code)) => {
                // A code a person typed reads as its own failure; a renewal
                // by itself as the lapse it was trying to end, or — tried in
                // the grant's last day, before any refusal — as a renewal.
                if self.team.busy == Some("redeeming") {
                    self.team.fail("redeeming", code);
                } else if self.team.lapsed {
                    self.team.error = Some(format!("{}", ErrorCode::GrantLapsed));
                } else {
                    self.team.error = Some(format!("renewing: {code}"));
                }
            }
            (Tag::Grant, Ok(Wire::Grant { until })) => self.team.grant_until = Some(until),
            // A relay from before cannot say; a refusal still renews.
            (Tag::Grant, Err(_)) => {}
            (Tag::Share, Ok(Wire::BoardCreated { board })) => self.team_on_shared(board),
            (Tag::Share, Err(ErrorCode::GrantLapsed)) => {
                self.team.fail("sharing", ErrorCode::GrantLapsed);
                self.team_on_lapse();
            }
            (Tag::Share, Err(code)) => self.team.fail("sharing", code),
            (Tag::Keys, Ok(_)) => {
                if let Some((epoch, key)) = self.team.rotating.take() {
                    if let Some(state) = self.team.state.as_mut() {
                        state.add_key(epoch, &key);
                    }
                    self.team.state_dirty = true;
                }
                if self.team.busy == Some("removing") {
                    self.team.busy = None;
                }
                self.team.sync_word = "synced";
                self.team.want_members = true;
            }
            (Tag::Keys, Err(code)) => {
                self.team.rotating = None;
                self.team.want_members = true;
                self.team.fail("handing out keys", code);
            }
            (Tag::Invite, Ok(Wire::InviteMinted { invite, .. })) => {
                if let (Some((code, role)), Some(state)) =
                    (self.team.minting.take(), self.team.state.as_mut())
                {
                    state.invites.push(PendingInvite {
                        invite: invite.to_hex(),
                        secret: mesimon_team::hex::encode(code.secret()),
                        role: role.word().into(),
                        code: code.encode(),
                    });
                    self.team.state_dirty = true;
                }
                self.team.busy = None;
            }
            (Tag::Invite, Err(code)) => {
                self.team.minting = None;
                self.team.fail("inviting", code);
                if code == ErrorCode::GrantLapsed {
                    self.team_on_lapse();
                }
            }
            (Tag::Join, Ok(Wire::Joined { board, role, owner })) => {
                self.team_on_joined(board, role, owner)
            }
            (Tag::Join, Err(code)) => {
                self.team.joining = None;
                self.team.fail("joining", code);
                if code == ErrorCode::GrantLapsed {
                    self.team_on_lapse();
                }
            }
            (Tag::Revoke, Ok(_)) => {
                self.team.rotate_after_members = true;
                self.team.want_members = true;
            }
            (Tag::Revoke, Err(code)) => self.team.fail("removing", code),
            (Tag::Unshare, Ok(_)) | (Tag::Leave, Ok(_)) => {
                let _ = std::fs::remove_file(self.paths.team_file());
                self.team.state = None;
                self.team.busy = None;
                self.team.sync_word = "offline";
                self.team.call(Tag::Boards, Request::Boards);
            }
            (Tag::Unshare, Err(code)) => self.team.fail("unsharing", code),
            (Tag::Leave, Err(code)) => self.team.fail("leaving", code),
            (Tag::Boards, Ok(Wire::Boards { boards })) => self.team.boards = boards,
            (Tag::Boards, Err(code)) => self.team.error = Some(format!("listing boards: {code}")),
            (Tag::Members, Ok(Wire::Members { members })) => self.team_on_members(members),
            (Tag::Members, Err(ErrorCode::NotFound)) => self.team_gone(),
            (Tag::Members, Err(_)) => {}
            (Tag::Head, Ok(Wire::Head { head })) => {
                let Some(state) = self.team.state.as_ref() else { return };
                let board = state.board_id();
                self.team.synced_at_ms = Some(now_ms());
                if head.rotation_required && !state.is_owner() {
                    self.team.sync_word = "frozen";
                } else if self.team.sync_word != "syncing" {
                    self.team.sync_word = "synced";
                }
                // The owner heals a pending rotation whenever it sees one: a
                // removal whose rotation failed, or one asked from elsewhere.
                if head.rotation_required
                    && state.is_owner()
                    && self.team.rotating.is_none()
                    && !self.team.inflight.contains(&Tag::Keys)
                {
                    self.team.rotate_after_members = true;
                    self.team.want_members = true;
                }
                if state.current_epoch().is_none_or(|e| head.epoch > e) {
                    self.team.want_keys = true;
                }
                if head.seq > state.cursor {
                    if let Some(board) = board {
                        self.team.sync_word = "syncing";
                        let after = state.cursor;
                        self.team.call(
                            Tag::Sync,
                            Request::Sync { board, after, limit: MAX_SYNC_RECORDS },
                        );
                    }
                }
            }
            (Tag::Head, Err(ErrorCode::NotFound)) => self.team_gone(),
            (Tag::Head, Err(code)) => {
                self.team.sync_word = "error";
                self.team.error = Some(format!("relay: {code}"));
            }
            (Tag::Sync, Ok(Wire::Records { records, next, more })) => {
                self.team.retention_full = false;
                for record in records {
                    self.team_apply(record);
                }
                self.team_retry_pending();
                // Retained encrypted records and the cursor are one write.
                // Later pages can supply their keys, author directory or parent.
                let saved = self.team_save_cursor(next);
                if saved && more {
                    if let (Some(board), Some(after)) =
                        (self.team.board(), self.team.state.as_ref().map(|s| s.cursor))
                    {
                        self.team.call(
                            Tag::Sync,
                            Request::Sync { board, after, limit: MAX_SYNC_RECORDS },
                        );
                    }
                } else if saved {
                    self.team.sync_word = "synced";
                }
                self.persist_and_notify();
            }
            (Tag::Sync, Err(ErrorCode::NotFound)) => self.team_gone(),
            (Tag::Sync, Err(code)) => {
                self.team.sync_word = "error";
                self.team.error = Some(format!("relay: {code}"));
            }
            (Tag::MyKeys, Ok(Wire::Keys { wrapped })) => self.team_on_keys(wrapped),
            (Tag::MyKeys, Err(_)) => {}
            (Tag::Put(operation), result) => self.team_on_put(&operation, result),
            (tag, Ok(other)) => {
                self.team.error =
                    Some(format!("unexpected relay answer to {tag:?}: {}", other_word(&other)));
            }
        }
        if self.team.state_dirty {
            self.team_save();
        }
        self.broadcast();
    }

    fn team_gone(&mut self) {
        self.team.sync_word = "gone";
        self.team.error = Some("the owner stopped sharing this board".into());
    }

    fn team_on_shared(&mut self, board: BoardId) {
        let name = self.team.device.as_ref().map(|d| d.display_name.clone()).unwrap_or_default();
        let mut state = TeamState::new(board, Role::Owner, false, name);
        let Some(keys) = self.team.device.as_ref().and_then(|d| d.keys()) else { return };
        state.pin_owner(keys.public(), state.owner_name.clone());
        let key = BoardKey::generate();
        state.add_key(0, &key);
        state.title = Some(self.board_title());
        state.notes_withheld = !self.team.share_notes;
        self.team.state = Some(state);
        self.team.note_revs.clear();
        if !self.team_save() {
            return;
        }
        if let Some(keys) = self.team.device.as_ref().and_then(|d| d.keys()) {
            let wrapped = vec![crypto::wrap(&key, 0, board, &keys, &keys.public())];
            self.team.call(Tag::Keys, Request::PutKeys { board, epoch: 0, wrapped });
        }
        self.team.busy = None;
        self.team.sync_word = "syncing";
        self.team.want_members = true;
        self.team_after_broadcast();
        self.team.call(Tag::Boards, Request::Boards);
    }

    fn team_on_joined(&mut self, board: BoardId, role: Role, owner: Member) {
        let Some(code) = self.team.joining.take() else { return };
        if role == Role::Owner
            || owner.device != owner.public.id()
            || !code.names_owner(&owner.public)
        {
            self.team.busy = None;
            self.team.error =
                Some("the relay named an owner the invite code does not vouch for".into());
            self.team.call(Tag::Leave, Request::Leave { board });
            return;
        }
        let result = (|| -> Result<PathBuf> {
            let root = Paths::board_root_for(&board.to_hex())?;
            crate::paths::own_private_dir(&root)?;
            let paths = Paths::for_repo(&root)?;
            paths.ensure_dirs()?;
            let mut state = TeamState::new(board, role, true, owner.display_name.clone());
            state.pin_owner(owner.public, owner.display_name.clone());
            state.members = vec![owner];
            state.save(&paths.team_file())?;
            Ok(root)
        })();
        match result {
            Ok(_) => {
                self.team.busy = None;
                self.team.call(Tag::Boards, Request::Boards);
            }
            Err(e) => {
                self.team.busy = None;
                self.team.error = Some(format!("joined, but could not create the board root: {e}"));
            }
        }
    }

    /// The owner chooses recipients from durable verified authority. The
    /// transport roster may only restrict where those keys can be delivered.
    fn team_rotate(&mut self) {
        if !self.team_save() {
            return;
        }
        let Some(keys) = self.team.device.as_ref().and_then(|d| d.keys()) else { return };
        let Some(state) = self.team.state.as_ref() else { return };
        if !state.is_owner() || state.verified_owner.as_deref() != Some(keys.id().to_hex().as_str())
        {
            return;
        }
        let Some(board) = state.board_id() else { return };
        let Some(epoch) = state.current_epoch().and_then(|e| e.checked_add(1)) else { return };
        let key = BoardKey::generate();
        let wrapped = state
            .verified_members
            .values()
            .filter(|p| {
                p.active
                    && (p.public == keys.public()
                        || state.members.iter().any(|m| {
                            m.device == p.public.id()
                                && m.public == p.public
                                && m.status == MemberStatus::Active
                        }))
            })
            .map(|p| crypto::wrap(&key, epoch, board, &keys, &p.public))
            .collect();
        self.team.rotating = Some((epoch, key));
        self.team.want_members = true;
        self.team.call(Tag::Keys, Request::PutKeys { board, epoch, wrapped });
    }

    fn team_on_members(&mut self, members: Vec<Member>) {
        if !mesimon_core::team::enabled() {
            return;
        }
        self.team_accept_members(members);
    }

    /// The feature-gated receiver's state transition, also exercised without
    /// changing process-global environment in unit tests.
    fn team_accept_members(&mut self, members: Vec<Member>) {
        let Some(keys) = self.team.device.as_ref().and_then(|d| d.keys()) else { return };
        let Some(state) = self.team.state.as_mut() else { return };
        if state.needs_rejoin {
            return;
        }
        let Some(board) = state.board_id() else { return };
        let owner = state.is_owner()
            && state.verified_owner.as_deref() == Some(keys.id().to_hex().as_str());
        let mut accepted = Vec::new();
        let mut seen = HashSet::new();
        for m in members {
            let id = m.device.to_hex();
            if m.device != m.public.id()
                || !seen.insert(m.device)
                || state.verified_members.get(&id).is_some_and(|p| p.public != m.public)
            {
                self.feed.board_outcome(
                    "relay",
                    "team_member_identity",
                    None,
                    &format!("ignored inconsistent identity {id}"),
                );
                continue;
            }
            if owner
                && m.status == MemberStatus::Active
                && !state.verified_members.get(&id).is_some_and(|p| p.active)
            {
                if let Some(at) = state.invites.iter().position(|i| {
                    InviteCode::parse(&i.code).is_ok_and(|c| {
                        m.proof.is_some_and(|proof| c.verifies_proof(&m.public, &proof))
                    })
                }) {
                    let invite = state.invites.remove(at);
                    if let Some(role @ (Role::Contributor | Role::Viewer)) =
                        Role::parse(&invite.role)
                    {
                        state.verified_members.insert(
                            id,
                            VerifiedMember {
                                public: m.public,
                                role,
                                active: true,
                                display_name: m.display_name.clone(),
                            },
                        );
                    }
                }
            }
            accepted.push(m);
        }
        state.members = accepted;
        self.team.state_dirty = true;
        if !self.team_save() {
            return;
        }
        self.team_retry_pending();
        if !owner {
            return;
        }
        self.team_after_broadcast();
        if std::mem::take(&mut self.team.rotate_after_members) {
            self.team_rotate();
            return;
        }
        let state = self.team.state.as_ref().expect("sharing state");
        let Some(current) = state.current_epoch() else { return };
        let mut wrapped = Vec::new();
        for (id, p) in &state.verified_members {
            if !p.active || p.public.id() == keys.id() {
                continue;
            }
            let Some(m) = state.members.iter().find(|m| {
                m.device.to_hex() == *id && m.public == p.public && m.status == MemberStatus::Active
            }) else {
                continue;
            };
            for epoch in state.keys.keys().filter(|e| !m.epochs.contains(e)) {
                if let Some(key) = state.key(*epoch) {
                    wrapped.push(crypto::wrap(&key, *epoch, board, &keys, &p.public));
                }
            }
        }
        if !wrapped.is_empty() {
            self.team.call(Tag::Keys, Request::PutKeys { board, epoch: current, wrapped });
        }
    }

    fn team_on_keys(&mut self, wrapped: Vec<crypto::WrappedKey>) {
        let Some(keys) = self.team.device.as_ref().and_then(|d| d.keys()) else { return };
        let Some(state) = self.team.state.as_mut() else { return };
        let Some(board) = state.board_id() else { return };
        if state.needs_rejoin {
            return;
        }
        let mut added = false;
        for w in wrapped {
            if state.keys.contains_key(&w.epoch) {
                continue;
            }
            let Some(sender) = state.verified_members.get(&w.sender.to_hex()).filter(|p| {
                p.active && state.verified_owner.as_deref() == Some(w.sender.to_hex().as_str())
            }) else {
                self.team.want_members = true;
                self.team.want_keys = true;
                continue;
            };
            match crypto::unwrap(&w, board, &keys, &sender.public) {
                Ok(key) => {
                    state.add_key(w.epoch, &key);
                    added = true;
                }
                Err(e) => {
                    self.team.error = Some(format!("a key for epoch {} did not open: {e}", w.epoch))
                }
            }
        }
        if added {
            self.team.state_dirty = true;
            if self.team.sync_word == "frozen" {
                self.team.sync_word = "synced";
            }
            self.team_retry_pending();
        }
    }

    fn team_on_put(&mut self, operation: &str, result: Result<Wire, ErrorCode>) {
        let Some(state) = self.team.state.as_mut() else { return };
        let Some(at) = state.outbox.iter().position(|o| o.operation == operation) else { return };
        match result {
            Ok(Wire::Accepted { receipt }) => {
                let entry = &state.outbox[at];
                if receipt.object.to_hex() != entry.object
                    || entry.expected.unwrap_or(0).checked_add(1) != Some(receipt.revision)
                {
                    self.team.error = Some("the relay returned a mismatched edit receipt".into());
                    return;
                }
                let entry = state.outbox.remove(at);
                let kind = kind_of(&entry.body);
                // A pull may have advanced this object while the receipt
                // was in flight. Its older digest must not replace that one.
                if state.revision(&entry.object).is_none_or(|r| receipt.revision > r) {
                    state.remember_revision(&entry.object, receipt.revision);
                    if is_tombstone(&entry.body) {
                        state.forget_published(&entry.object);
                    } else {
                        state.published.insert(
                            entry.object.clone(),
                            Published {
                                revision: receipt.revision,
                                digest: project::digest(&entry.body),
                                kind,
                                by: None,
                            },
                        );
                    }
                } else {
                    // The incoming record was held off for this local edit.
                    // Keep that edit queued against the newer floor, including
                    // notes whose cached local rev will not trigger a diff.
                    state.enqueue(receipt.object, entry.body.clone(), None);
                }
                self.team.state_dirty = true;
                self.team.synced_at_ms = Some(now_ms());
                if self.team.sync_word == "error" {
                    self.team.sync_word = "synced";
                }
                if entry.dirty {
                    if let SharedObject::Note(note) = &entry.body.object {
                        let id = ulid::Ulid::from(receipt.object);
                        self.team.note_revs.remove(&id);
                        if !self
                            .board
                            .ticket(note.ticket)
                            .is_some_and(|t| t.notes.iter().any(|n| n.id == id))
                        {
                            state.enqueue(
                                receipt.object,
                                project::note_tombstone(note.ticket),
                                None,
                            );
                        }
                    }
                    self.team_after_broadcast();
                }
            }
            Err(ErrorCode::StaleRevision) => {
                // Someone else got there first: pull, apply theirs, then this
                // entry goes again against the revision it now sees.
                self.team.last_pull = 0;
            }
            Err(ErrorCode::StaleEpoch) => self.team.want_keys = true,
            Err(ErrorCode::RotationRequired) => {
                if state.is_owner() {
                    self.team.want_members = true;
                } else {
                    self.team.sync_word = "frozen";
                }
            }
            Err(ErrorCode::Denied) => {
                state.outbox.remove(at);
                self.team.error =
                    Some("this board is read-only for you; the edit stays here".into());
                self.team.state_dirty = true;
            }
            // The edit stays in the outbox: a code makes it go.
            Err(ErrorCode::GrantLapsed) => self.team_on_lapse(),
            Err(ErrorCode::NotFound) => self.team_gone(),
            Err(ErrorCode::OperationMismatch) => {
                state.outbox[at].operation = OperationId::random().to_hex();
                self.team.state_dirty = true;
            }
            Err(code) => {
                self.team.sync_word = "error";
                self.team.error = Some(format!("relay refused an edit: {code}"));
            }
            Ok(other) => {
                self.team.error =
                    Some(format!("unexpected relay answer to a put: {}", other_word(&other)))
            }
        }
    }

    // ---- outgoing: the diff -------------------------------------------------

    /// At the end of every `broadcast()`: queue whatever the relay does not
    /// have yet. Silent while a remote record is being applied, because that
    /// record's content/publication transaction has not finished yet.
    pub(super) fn team_after_broadcast(&mut self) {
        if self.team.applying || self.team.state.as_ref().is_some_and(|s| s.needs_rejoin) {
            return;
        }
        let Some(state) = self.team.state.as_ref() else { return };
        let owner = state.is_owner();
        let notes = !state.notes_withheld;
        let title = self.board_title();
        let me = self.team.device.as_ref().map(|d| d.display_name.clone()).unwrap_or_default();
        let mut objects = project::project(&self.board, owner, &title, notes, &me);
        if owner {
            objects.insert(
                ObjectId(MEMBERS_OBJECT),
                RecordBody::new(SharedObject::Members {
                    members: state.verified_members.values().map(VerifiedMember::shared).collect(),
                }),
            );
        }
        let in_flight = self.team.put_inflight().map(str::to_owned);
        let mut changed = false;
        let Some(state) = self.team.state.as_mut() else { return };
        for (id, body) in &objects {
            let digest = project::digest(body);
            if self.team.blocked_digests.get(&id.to_hex()) == Some(&Some(digest.clone())) {
                continue;
            }
            if state.published.get(&id.to_hex()).is_none_or(|p| p.digest != digest) {
                state.enqueue(*id, body.clone(), in_flight.as_deref());
                // Changed here now, whoever changed it last: the card's
                // initials come off with the same broadcast.
                if let Some(p) = state.published.get_mut(&id.to_hex()) {
                    p.by = None;
                }
                changed = true;
            }
        }
        // Tickets that are gone: a tombstone once; keep their revision floor.
        let gone: Vec<String> = state
            .published
            .iter()
            .filter(|(k, p)| {
                p.kind == "ticket"
                    && self.team.blocked_digests.get(*k) != Some(&None)
                    && ObjectId::parse(k).is_some_and(|id| !objects.contains_key(&id))
            })
            .map(|(k, _)| k.clone())
            .collect();
        for key in gone {
            if let Some(id) = ObjectId::parse(&key) {
                let title = String::new();
                let stone = project::ticket_tombstone(&mesimon_core::team::SharedTicket {
                    title,
                    column: String::new(),
                    order: String::new(),
                    created_at: String::new(),
                    created_by: String::new(),
                    archived: false,
                    notes: Vec::new(),
                    deleted: true,
                });
                if !state.outbox.iter().any(|o| o.object == key) {
                    state.enqueue(id, stone, in_flight.as_deref());
                }
                state.forget_published(&key);
                changed = true;
            }
        }
        // Notes: bodies are files, read only when a rev moved — and never
        // when the owner kept them here.
        let mut seen = HashSet::new();
        for t in self.board.tickets.iter().filter(|_| notes) {
            for n in &t.notes {
                seen.insert(n.id);
                if self.team.note_revs.get(&n.id).map(|(rev, _)| *rev) == Some(n.rev) {
                    continue;
                }
                let Ok(body) = store::read_note(&self.paths, &t.short_key, n.id) else { continue };
                state.enqueue(
                    ObjectId::from(n.id),
                    project::note_body(t.id, body),
                    in_flight.as_deref(),
                );
                if let Some(p) = state.published.get_mut(&ObjectId::from(n.id).to_hex()) {
                    p.by = None;
                }
                self.team.note_revs.insert(n.id, (n.rev, t.id));
                changed = true;
            }
        }
        let removed: Vec<(ulid::Ulid, ulid::Ulid)> = self
            .team
            .note_revs
            .iter()
            .filter(|(id, _)| !seen.contains(*id))
            .map(|(id, (_, ticket))| (*id, *ticket))
            .collect();
        for (note, ticket) in removed {
            self.team.note_revs.remove(&note);
            let key = ObjectId::from(note).to_hex();
            if state.published.contains_key(&key) {
                state.enqueue(
                    ObjectId::from(note),
                    project::note_tombstone(ticket),
                    in_flight.as_deref(),
                );
                state.forget_published(&key);
                changed = true;
            }
        }
        if changed {
            self.team.state_dirty = true;
        }
    }

    // ---- incoming: a record becomes board state ---------------------------

    fn team_save_cursor(&mut self, next: u64) -> bool {
        if self.team.retention_full {
            // The page will be fetched again; every previously retained input
            // stays intact. Other requests can still deliver missing keys.
            self.team_save();
            return false;
        }
        let Some(state) = self.team.state.as_mut() else { return false };
        let previous = state.cursor;
        state.cursor = previous.max(next);
        self.team.state_dirty = true;
        if self.team_save() {
            true
        } else {
            if let Some(state) = self.team.state.as_mut() {
                state.cursor = previous;
            }
            false
        }
    }

    fn team_retry_pending(&mut self) {
        if mesimon_core::team::enabled() {
            self.team_retry_pending_records();
        }
    }

    fn team_retry_pending_records(&mut self) {
        let mut budget = MAX_RETRY_RECORDS;
        while budget > 0 {
            let Some(state) = self.team.state.as_ref() else { return };
            let count = state.pending_records.len();
            if count == 0 {
                return;
            }
            let start = self.team.retry_at % count;
            let take = budget.min(count);
            let pending: Vec<_> = state
                .pending_records
                .iter()
                .cycle()
                .skip(start)
                .take(take)
                .map(|p| p.record.clone())
                .collect();
            for record in pending {
                self.team_receive_record(record);
                budget -= 1;
            }
            let remaining = self.team.state.as_ref().map_or(0, |s| s.pending_records.len());
            self.team.retry_at = if remaining == 0 { 0 } else { (start + take) % remaining };
            if remaining == 0 || remaining >= count {
                break;
            }
        }
    }

    fn team_can_retain(&mut self, record: &StoredRecord) -> bool {
        let Some(state) = self.team.state.as_ref() else { return false };
        if state.pending_records.iter().any(|p| p.matches(record)) {
            return true;
        }
        let bytes: usize =
            state.pending_records.iter().map(|p| p.record.record.ciphertext.len()).sum();
        if state.pending_records.len() < MAX_PENDING_RECORDS
            && record.record.ciphertext.len() <= MAX_PENDING_BYTES.saturating_sub(bytes)
        {
            return true;
        }
        self.team.retention_full = true;
        self.team.sync_word = "error";
        self.team.error = Some(format!(
            "sharing paused: retained input limit ({MAX_PENDING_RECORDS} records / {} MiB); existing records and cursor kept. Recover pending records or use a build with a higher retained limit; this page will be retried",
            MAX_PENDING_BYTES / (1024 * 1024)
        ));
        false
    }

    fn team_apply(&mut self, record: StoredRecord) {
        if mesimon_core::team::enabled() {
            self.team_receive_record(record);
        }
    }

    /// Shared ingress/finalization for content and the authenticated directory.
    /// Only this layer retains deferred input and finishes its publication.
    fn team_receive_record(&mut self, record: StoredRecord) -> ApplyOutcome {
        let board_before = self.board.clone();
        let notes_before = self.team.note_revs.clone();
        let mut outcome = self.team_apply_record(&record);
        let content_succeeded = outcome == ApplyOutcome::Applied;
        #[cfg(test)]
        if outcome == ApplyOutcome::Applied {
            if let Some(fail) = self.team.before_record_finish.take() {
                fail(self);
            }
        }
        let can_retain = outcome != ApplyOutcome::Deferred || self.team_can_retain(&record);
        let Some(state) = self.team.state.as_mut() else { return outcome };
        match outcome {
            ApplyOutcome::Deferred => {
                if can_retain && !state.pending_records.iter().any(|p| p.matches(&record)) {
                    state.pending_records.push(PendingRecord {
                        record: record.clone(),
                        publication: None,
                        note_rev: None,
                        local_before: None,
                        local_after: None,
                        local_intermediate: Vec::new(),
                        applied_at: None,
                    });
                }
                self.team.state_dirty = true;
            }
            ApplyOutcome::Applied => {
                // A directory applied directly may already have committed all
                // its state and have no retained entry to finalize.
                if let Some(entry) =
                    state.pending_records.iter().find(|p| p.matches(&record)).cloned()
                {
                    let before_finish = state.clone();
                    if let Some(publication) = entry.publication {
                        state.published.insert(record.object.to_hex(), publication);
                    }
                    state.pending_records.retain(|p| !p.matches(&record));
                    self.team.state_dirty = true;
                    if !self.team_save() {
                        self.team.state = Some(before_finish);
                        outcome = ApplyOutcome::Deferred;
                    }
                }
            }
            ApplyOutcome::Rejected => {
                let before = state.pending_records.len();
                state.pending_records.retain(|p| !p.matches(&record));
                self.team.state_dirty |= before != state.pending_records.len();
            }
        }
        if outcome == ApplyOutcome::Deferred {
            // Failed content work restores its visible snapshot. Successful
            // content stays visible if only finalization failed; the saved
            // intent/floor already protects it across a restart.
            if !content_succeeded {
                self.board = board_before;
                self.team.note_revs = notes_before;
            }
            self.team_capture_blocked_digests();
        } else if !self.team.state.as_ref().is_some_and(|s| {
            s.pending_records
                .iter()
                .any(|p| p.record.object == record.object && p.publication.is_some())
        }) {
            self.team.blocked_digests.remove(&record.object.to_hex());
        }
        outcome
    }

    fn team_capture_blocked_digests(&mut self) {
        let Some(state) = self.team.state.as_ref() else { return };
        let title = self.board_title();
        let me = self.team.device.as_ref().map(|d| d.display_name.clone()).unwrap_or_default();
        let objects =
            project::project(&self.board, state.is_owner(), &title, !state.notes_withheld, &me);
        for entry in state.pending_records.iter().filter(|p| p.publication.is_some()) {
            self.team
                .blocked_digests
                .entry(entry.record.object.to_hex())
                .or_insert_with(|| objects.get(&entry.record.object).map(project::digest));
        }
    }

    fn team_local_body(&self, object: ObjectId, body: &RecordBody) -> Option<RecordBody> {
        let me = self.team.device.as_ref().map(|d| d.display_name.as_str()).unwrap_or_default();
        match &body.object {
            SharedObject::Ticket(shared) => Some(
                self.board
                    .ticket(ulid::Ulid::from(object))
                    .map(|t| {
                        project::ticket_body(
                            t,
                            self.team.state.as_ref().is_none_or(|s| !s.notes_withheld),
                            me,
                        )
                    })
                    .unwrap_or_else(|| project::ticket_tombstone(shared)),
            ),
            SharedObject::Note(note) => {
                let Some(t) = self.board.ticket(note.ticket) else {
                    return Some(project::note_tombstone(note.ticket));
                };
                if !t.notes.iter().any(|n| n.id == ulid::Ulid::from(object)) {
                    return Some(project::note_tombstone(note.ticket));
                }
                store::read_note(&self.paths, &t.short_key, ulid::Ulid::from(object))
                    .ok()
                    .map(|text| project::note_body(note.ticket, text))
            }
            _ => None,
        }
    }

    fn team_local_snapshot(&self, object: ObjectId, body: &RecordBody) -> Option<LocalSnapshot> {
        let id = ulid::Ulid::from(object);
        match &body.object {
            SharedObject::Ticket(_) => {
                let local = self.team_local_body(object, body)?;
                let path = self.board.ticket(id).map(|t| {
                    self.paths
                        .board_dir
                        .join("board/tickets")
                        .join(&t.short_key)
                        .join("ticket.toml")
                });
                Some(LocalSnapshot {
                    digest: project::digest(&local),
                    stamp: file_stamp(path.as_deref()),
                })
            }
            SharedObject::Note(note) => {
                let Some(ticket) = self.board.ticket(note.ticket) else {
                    return Some(LocalSnapshot {
                        digest: local_fingerprint(&(None::<String>, None::<NoteMeta>)),
                        stamp: "missing".into(),
                    });
                };
                let path = self
                    .paths
                    .board_dir
                    .join("board/tickets")
                    .join(&ticket.short_key)
                    .join("notes")
                    .join(format!("{id}.md"));
                let text = match store::read_note(&self.paths, &ticket.short_key, id) {
                    Ok(text) => Some(text),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                    Err(_) => return None,
                };
                let meta = ticket.notes.iter().find(|n| n.id == id);
                Some(LocalSnapshot {
                    digest: local_fingerprint(&(text, meta)),
                    stamp: file_stamp(Some(&path)),
                })
            }
            _ => None,
        }
    }

    fn team_expected_local_digest(
        &self,
        object: ObjectId,
        body: &RecordBody,
        by: &Principal,
        note_rev: Option<u64>,
        at: &str,
    ) -> (Option<String>, Vec<String>) {
        let id = ulid::Ulid::from(object);
        match &body.object {
            SharedObject::Note(note) => {
                let old = self
                    .board
                    .ticket(note.ticket)
                    .and_then(|t| t.notes.iter().find(|n| n.id == id))
                    .cloned();
                let text = (!note.deleted).then(|| sanitize_note(&note.body));
                let intermediate = local_fingerprint(&(text.clone(), old.clone()));
                let meta = if note.deleted {
                    None
                } else {
                    let author = by.note_author();
                    Some(NoteMeta {
                        id,
                        name: note_name(text.as_deref().unwrap_or_default()),
                        rev: note_rev.unwrap_or(1),
                        created_at: old
                            .as_ref()
                            .map(|n| n.created_at.clone())
                            .unwrap_or_else(|| at.into()),
                        created_by: old
                            .as_ref()
                            .map(|n| n.created_by.clone())
                            .unwrap_or_else(|| author.clone()),
                        edited_at: at.into(),
                        edited_by: author,
                    })
                };
                (Some(local_fingerprint(&(text, meta))), vec![intermediate])
            }
            SharedObject::Ticket(shared) => {
                let current = self.board.ticket(id);
                let me =
                    self.team.device.as_ref().map(|d| d.display_name.as_str()).unwrap_or_default();
                let mut next = if let Some(t) = current {
                    let SharedObject::Ticket(t) = project::ticket_body(
                        t,
                        self.team.state.as_ref().is_none_or(|s| !s.notes_withheld),
                        me,
                    )
                    .object
                    else {
                        unreachable!()
                    };
                    t
                } else {
                    mesimon_core::team::SharedTicket {
                        created_by: if shared.created_by.starts_with("member:") {
                            shared.created_by.clone()
                        } else {
                            by.note_author()
                        },
                        notes: Vec::new(),
                        deleted: false,
                        ..shared.clone()
                    }
                };
                if shared.deleted {
                    if current.is_none() {
                        return (
                            Some(project::digest(&project::ticket_tombstone(shared))),
                            Vec::new(),
                        );
                    }
                    next.archived = true;
                    return (
                        Some(project::digest(&RecordBody::new(SharedObject::Ticket(next)))),
                        Vec::new(),
                    );
                }
                let column = if self.board.column(&shared.column).is_some() {
                    shared.column.clone()
                } else {
                    self.board.landing_column().unwrap_or_default()
                };
                if current.is_none()
                    || (!next.archived
                        && !shared.archived
                        && self.column_gate(id, &column).is_none())
                {
                    next.column = column;
                }
                next.title = sanitize_title(&shared.title);
                next.order = shared.order.clone();
                next.archived = shared.archived;
                if !shared.notes.is_empty() {
                    next.notes.sort_by_key(|id| {
                        shared.notes.iter().position(|n| n == id).unwrap_or(usize::MAX)
                    });
                }
                (Some(project::digest(&RecordBody::new(SharedObject::Ticket(next)))), Vec::new())
            }
            _ => (None, Vec::new()),
        }
    }

    fn team_apply_record(&mut self, record: &StoredRecord) -> ApplyOutcome {
        let Some(keys) = self.team.device.as_ref().and_then(|d| d.keys()) else {
            return ApplyOutcome::Deferred;
        };
        let Some(state) = self.team.state.as_ref() else { return ApplyOutcome::Rejected };
        let Some(board) = state.board_id() else { return ApplyOutcome::Rejected };
        if state.needs_rejoin {
            return ApplyOutcome::Rejected;
        }
        // Own echoes are accounted for by the local receipt. A claimed
        // self author must not authorize a new inbound floor on its own.
        if record.record.author == keys.id() {
            return ApplyOutcome::Rejected;
        }
        let Some(author) = state.verified_members.get(&record.record.author.to_hex()).cloned()
        else {
            self.team.want_members = true;
            return ApplyOutcome::Deferred;
        };
        // A missing directory may supply an unknown author's identity. A
        // known revoked or read-only identity has already been decided: its
        // rejected records must not become eligible after a later re-invite.
        if !author.active || !author.role.may_write() {
            return ApplyOutcome::Rejected;
        }
        let Some(key) = state.key(record.record.epoch) else {
            self.team.want_keys = true;
            return ApplyOutcome::Deferred;
        };
        let scope = RecordScope { board, object: record.object, revision: record.revision };
        let plaintext = match crypto::open(&key, scope, &record.record, &author.public) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.team.error =
                    Some(format!("a record from {} did not verify: {e}", author.display_name));
                return ApplyOutcome::Rejected;
            }
        };
        let Ok(body) = serde_json::from_slice::<RecordBody>(&plaintext) else {
            self.team.error =
                Some(format!("a record from {} is not readable", author.display_name));
            return ApplyOutcome::Rejected;
        };
        if body.schema > RECORD_SCHEMA {
            self.notices.retain(|n| n.kind != "team_schema");
            self.notices.push(Notice::new(
                "team_schema",
                "a teammate runs a newer mesimon; update to see their changes",
            ));
            return ApplyOutcome::Deferred;
        }
        let from_owner =
            state.verified_owner.as_deref() == Some(record.record.author.to_hex().as_str());
        if matches!(
            body.object,
            SharedObject::Members { .. }
                | SharedObject::Board { .. }
                | SharedObject::Columns { .. }
        ) && !from_owner
        {
            return ApplyOutcome::Rejected;
        }
        if let SharedObject::Members { members } = &body.object {
            if record.object != ObjectId(MEMBERS_OBJECT)
                || state.is_owner()
                || record.revision <= state.membership_revision
            {
                return ApplyOutcome::Rejected;
            }
            let mut verified = std::collections::BTreeMap::new();
            for m in members {
                let Some(pin) = VerifiedMember::from_shared(m) else {
                    return ApplyOutcome::Rejected;
                };
                if state.verified_members.get(&m.device).is_some_and(|p| p.public != pin.public)
                    || verified.insert(m.device.clone(), pin).is_some()
                {
                    return ApplyOutcome::Rejected;
                }
            }
            let owner_id = record.record.author.to_hex();
            if verified.get(&owner_id) != Some(&author)
                || verified.iter().any(|(id, p)| p.role == Role::Owner && *id != owner_id)
            {
                return ApplyOutcome::Rejected;
            }
            let old = self.team.state.clone();
            let state = self.team.state.as_mut().expect("sharing state");
            state.role = verified
                .get(&keys.id().to_hex())
                .filter(|p| p.active && p.public == keys.public())
                .map_or(Role::Viewer, |p| p.role)
                .word()
                .into();
            state.verified_members = verified;
            state.membership_revision = record.revision;
            if !self.team_save() {
                self.team.state = old;
                return ApplyOutcome::Deferred;
            }
            return ApplyOutcome::Applied;
        }
        if record.object == ObjectId(MEMBERS_OBJECT) {
            return ApplyOutcome::Rejected;
        }
        let object = record.object.to_hex();
        if let Some(published) = state.revision(&object) {
            let retained = record.revision == published
                && state
                    .pending_records
                    .iter()
                    .any(|p| p.matches(record) && p.publication.is_some());
            if record.revision <= published && !retained {
                self.feed.board_detail(
                    "mesimon",
                    "team_revision_refused",
                    "refused",
                    &format!(
                        "object {object}: received revision {}, published revision {published}",
                        record.revision
                    ),
                );
                return ApplyOutcome::Rejected;
            }
        }
        let name = mesimon_core::text::scrub_text(&author.display_name);
        let member = Principal::Remote { member: name.clone() };
        // Defer dependencies before accepting a content intent. Broadcasts
        // during application are suppressed until publication is finalized.
        let key = record.object.to_hex();
        let mut pending = state.outbox.iter().any(|o| o.object == key);
        let retained = state.pending_records.iter().find(|p| p.matches(record)).cloned();
        // A note can arrive before its parent. It has not been accepted
        // yet: publishing it here would make its own retry stale.
        if !pending
            && matches!(&body.object, SharedObject::Note(note)
                if !note.deleted && self.board.ticket(note.ticket).is_none())
        {
            return ApplyOutcome::Deferred;
        }
        if !pending
            && self.columns_barred
            && matches!(&body.object, SharedObject::Ticket(t)
                if !t.deleted && self.board.ticket(ulid::Ulid::from(record.object)).is_none())
        {
            return ApplyOutcome::Deferred;
        }
        let local_before = self.team_local_snapshot(record.object, &body);
        if matches!(body.object, SharedObject::Ticket(_) | SharedObject::Note(_))
            && local_before.is_none()
        {
            self.team.error = Some(format!(
                "local content for shared object {key} could not be inspected; record retained for retry"
            ));
            return ApplyOutcome::Deferred;
        }
        let already_applied = retained.as_ref().is_some_and(|p| {
            p.publication.is_some()
                && local_before.as_ref().is_some_and(|s| p.local_after.as_ref() == Some(&s.digest))
        });
        if let Some(intent) = retained.as_ref().filter(|p| p.publication.is_some() && !pending) {
            if let Some(current) = &local_before {
                if intent.local_before.as_ref() != Some(current)
                    && intent.local_after.as_ref() != Some(&current.digest)
                    && !intent.local_intermediate.contains(&current.digest)
                {
                    // A durable local write happened after this intent. It may
                    // have outlived an unsaved outbox: reconstruct that edit.
                    if let Some(local) = self.team_local_body(record.object, &body) {
                        let state = self.team.state.as_mut().expect("sharing state");
                        if !matches!(local.object, SharedObject::Note(_)) || !state.notes_withheld {
                            state.enqueue(record.object, local, None);
                        }
                        pending = true;
                    }
                }
            }
        }
        if !self.team_can_retain(record) {
            return ApplyOutcome::Deferred;
        }
        let publication = Published {
            revision: record.revision,
            digest: project::digest(&body),
            kind: kind_of(&body),
            by: Some(name),
        };
        let note_rev = match &body.object {
            SharedObject::Note(note) if !note.deleted => Some(
                self.board
                    .ticket(note.ticket)
                    .and_then(|t| t.notes.iter().find(|n| n.id == ulid::Ulid::from(record.object)))
                    .map_or(1, |n| n.rev.saturating_add(1)),
            ),
            _ => None,
        };
        let applied_at =
            retained.as_ref().and_then(|p| p.applied_at.clone()).unwrap_or_else(now_iso);
        let target_rev = retained.as_ref().and_then(|p| p.note_rev).or(note_rev);
        let (local_after, local_intermediate) =
            self.team_expected_local_digest(record.object, &body, &member, target_rev, &applied_at);
        let state = self.team.state.as_mut().expect("sharing state");
        state.remember_revision(&key, record.revision);
        if let Some(entry) = state.pending_records.iter_mut().find(|p| p.matches(record)) {
            if entry.publication.is_none() {
                entry.publication = Some(publication);
                entry.note_rev = target_rev;
                entry.local_before = local_before;
                entry.local_after = local_after;
                entry.local_intermediate = local_intermediate;
                entry.applied_at = Some(applied_at.clone());
            }
        } else {
            state.pending_records.push(PendingRecord {
                record: record.clone(),
                publication: Some(publication),
                note_rev: target_rev,
                local_before,
                local_after,
                local_intermediate,
                applied_at: Some(applied_at.clone()),
            });
        }
        self.team.state_dirty = true;
        // The floor and exact recoverable intent precede every content write.
        if !self.team_save() {
            return ApplyOutcome::Deferred;
        }
        if pending || already_applied {
            return ApplyOutcome::Applied;
        }
        self.team.applying = true;
        let result = match body.object {
            SharedObject::Ticket(shared) => {
                self.team_apply_ticket(ulid::Ulid::from(record.object), shared, &member)
            }
            SharedObject::Note(note) => {
                let rev = self
                    .team
                    .state
                    .as_ref()
                    .and_then(|s| s.pending_records.iter().find(|p| p.matches(record)))
                    .and_then(|p| p.note_rev);
                self.team_apply_note(
                    ulid::Ulid::from(record.object),
                    note,
                    &member,
                    rev,
                    &applied_at,
                )
            }
            SharedObject::Columns { names } => {
                self.team_apply_columns(names);
                store::save_columns(&self.paths, &self.board)
            }
            SharedObject::Members { .. } => unreachable!("directory applied above"),
            SharedObject::Board { title } => {
                if let Some(state) = self.team.state.as_mut() {
                    state.title = Some(sanitize_title(&title));
                }
                Ok(())
            }
        };
        self.team.applying = false;
        match result {
            Ok(()) => ApplyOutcome::Applied,
            Err(error) => {
                self.team.error = Some(format!("applying shared object {key}: {error}"));
                ApplyOutcome::Deferred
            }
        }
    }

    fn team_apply_ticket(
        &mut self,
        id: ulid::Ulid,
        shared: mesimon_core::team::SharedTicket,
        by: &Principal,
    ) -> Result<()> {
        let now = now_iso();
        let exists = self.board.ticket(id).is_some();
        if shared.deleted {
            if exists && !self.board.ticket(id).is_some_and(Ticket::is_archived) {
                let by_word = by.note_author();
                if let Some(t) = self.board.ticket_mut(id) {
                    t.archived =
                        Some(Archived { at: now, by: by_word, until: None, needs_you: false });
                }
                self.feed.board(by.actor(), "team_archive", Some(id));
            }
            if let Some(ticket) = self.board.ticket(id) {
                store::save_ticket(&self.paths, ticket)?;
            }
            return Ok(());
        }
        let column = if self.board.column(&shared.column).is_some() {
            shared.column.clone()
        } else {
            self.board.landing_column().unwrap_or_default()
        };
        if !exists {
            if self.columns_barred {
                anyhow::bail!("columns are unavailable");
            }
            self.board.next_key += 1;
            let t = Ticket {
                id,
                short_key: format!("{}{}", mesimon_core::board::KEY_PREFIX, self.board.next_key),
                title: sanitize_title(&shared.title),
                column,
                order: shared.order.clone(),
                created_at: shared.created_at.clone(),
                // The record's own author word — `member:<name>` since the
                // projection spells every maker that way (T-335) — so every
                // copy projects the same bytes and none echoes the ticket
                // back. A record from before that spelling names its
                // signer, as it did.
                created_by: if shared.created_by.starts_with("member:") {
                    shared.created_by.clone()
                } else {
                    by.note_author()
                },
                created_from: None,
                entered_at: Some(now.clone()),
                previous_column: None,
                picked: None,
                woke_at: None,
                manual_merge: false,
                // Never the local composer's automation: a teammate's ticket
                // is data until the owner starts it.
                execution_policy: ExecutionPolicy::OwnerOnly,
                tier: None,
                import_origin: None,
                envelope: None,
                raised: None,
                workspace: None,
                tags: Vec::new(),
                notes: Vec::new(),
                archived: shared.archived.then(|| Archived {
                    at: now.clone(),
                    by: by.note_author(),
                    until: None,
                    needs_you: false,
                }),
            };
            store::save_ticket(&self.paths, &t)?;
            self.board.tickets.push(t);
            store::save_columns(&self.paths, &self.board)?;
            self.feed.board(by.actor(), "team_create", Some(id));
            self.broadcast();
            return Ok(());
        }
        let (from, was_archived) =
            self.board.ticket(id).map(|t| (t.column.clone(), t.is_archived())).unwrap_or_default();
        if from != column && !was_archived && !shared.archived {
            if let Err(why) = self.place_ticket(id, &column, Position::Top, by, None, "team") {
                // The board's own rules held (the DONE gate, say). The next
                // diff sends the local truth back.
                self.feed.board(by.actor(), &format!("team_move_refused:{why}"), Some(id));
            }
        }
        let by_word = by.note_author();
        let order = shared.order.clone();
        // A teammate's reorder is a move by hand (T-1035).
        if self.board.ticket(id).is_some_and(|t| t.order != order) {
            self.hand_moved(id);
        }
        let title = sanitize_title(&shared.title);
        let notes = shared.notes.clone();
        let archive = shared.archived;
        if let Some(t) = self.board.ticket_mut(id) {
            t.title = title;
            t.order = order;
            if archive && t.archived.is_none() {
                t.archived = Some(Archived { at: now, by: by_word, until: None, needs_you: false });
            } else if !archive && t.archived.is_some() {
                t.archived = None;
            }
            if !notes.is_empty() {
                t.notes
                    .sort_by_key(|n| notes.iter().position(|id| *id == n.id).unwrap_or(usize::MAX));
            }
        }
        store::save_ticket(&self.paths, self.board.ticket(id).expect("existing ticket"))?;
        self.feed.board(by.actor(), "team_update", Some(id));
        self.broadcast();
        Ok(())
    }

    fn team_apply_note(
        &mut self,
        note: ulid::Ulid,
        shared: mesimon_core::team::SharedNote,
        by: &Principal,
        target_rev: Option<u64>,
        applied_at: &str,
    ) -> Result<()> {
        let Some(t) = self.board.ticket(shared.ticket) else { return Ok(()) };
        let mut updated = t.clone();
        let key = t.short_key.clone();
        let ticket = t.id;
        if shared.deleted {
            store::delete_note(&self.paths, &key, note)?;
            updated.notes.retain(|n| n.id != note);
            store::save_ticket(&self.paths, &updated)?;
            *self.board.ticket_mut(ticket).expect("existing ticket") = updated;
            self.team.note_revs.remove(&note);
            self.broadcast();
            return Ok(());
        }
        let text = sanitize_note(&shared.body);
        store::save_note(&self.paths, &key, note, &text)?;
        let name = note_name(&text);
        let now = applied_at.to_owned();
        let author = by.note_author();
        let rev = target_rev.expect("note content intent carries its local revision");
        match updated.notes.iter_mut().find(|n| n.id == note) {
            Some(n) => {
                n.name = name;
                n.rev = rev;
                n.edited_at = now;
                n.edited_by = author;
            }
            None => {
                updated.notes.push(NoteMeta {
                    id: note,
                    name,
                    rev,
                    created_at: now.clone(),
                    created_by: author.clone(),
                    edited_at: now,
                    edited_by: author,
                });
            }
        }
        store::save_ticket(&self.paths, &updated)?;
        *self.board.ticket_mut(ticket).expect("existing ticket") = updated;
        self.team.note_revs.insert(note, (rev, ticket));
        self.feed.board(by.actor(), "team_note", Some(ticket));
        self.broadcast();
        Ok(())
    }

    /// On a joined board the owner's columns are the columns: missing ones
    /// are added and the order is theirs. Extra local columns stay.
    fn team_apply_columns(&mut self, names: Vec<String>) {
        if !self.team_content_only() {
            return;
        }
        let mut changed = false;
        let mut previous: Option<String> = None;
        for name in names.iter().filter(|n| !n.trim().is_empty()) {
            if self.board.column(name).is_none()
                && self.board.add_column(name.clone(), previous.as_deref()).is_ok()
            {
                changed = true;
            }
            if let Some(prev) = &previous {
                let order: Vec<String> =
                    self.board.columns.iter().map(|c| c.name.clone()).collect();
                let (Some(p), Some(n)) =
                    (order.iter().position(|c| c == prev), order.iter().position(|c| c == name))
                else {
                    continue;
                };
                if n < p {
                    let after_prev = order.get(p + 1).cloned();
                    if self.board.reorder_column(name, after_prev.as_deref()).is_ok() {
                        changed = true;
                    }
                }
            }
            previous = Some(name.clone());
        }
        if changed {
            self.persist_columns();
            self.broadcast();
        }
    }
}

/// The commands a viewer may not send: everything that writes a ticket or a
/// note (`Command::meta` names the ticket) and the two that mint one. Tags
/// are local and stay a viewer's own; so are the columns, the sessions the
/// board cannot have anyway, and every preference.
fn viewer_edit(command: &Command) -> bool {
    if matches!(
        command,
        Command::CreateTicket { .. }
            | Command::CreateTicketWithNote { .. }
            | Command::ImportTicket { .. }
            | Command::MoveTicket { .. }
            | Command::ArchiveAll
            // Names no single ticket (T-378), so `meta` alone would let it by.
            | Command::PromptColumn { .. }
    ) {
        return true;
    }
    if matches!(
        command,
        Command::SetTag { .. }
            | Command::SeenTicket { .. }
            | Command::LowerHand { .. }
            | Command::OpenedTicket { .. }
    ) {
        return false;
    }
    let meta = command.meta();
    meta.subject.is_some() && !matches!(meta.action, mesimon_core::authorize::Action::Read)
}

fn local_fingerprint(value: &impl serde::Serialize) -> String {
    use sha2::Digest;
    mesimon_team::hex::encode(&sha2::Sha256::digest(serde_json::to_vec(value).unwrap_or_default()))
}

fn file_stamp(path: Option<&std::path::Path>) -> String {
    use std::os::unix::fs::MetadataExt;
    path.and_then(|p| std::fs::metadata(p).ok())
        .and_then(|m| {
            m.modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|t| format!("{}:{}:{}:{}", m.dev(), m.ino(), t.as_nanos(), m.len()))
        })
        .unwrap_or_else(|| "missing".into())
}

fn is_tombstone(body: &RecordBody) -> bool {
    match &body.object {
        SharedObject::Ticket(ticket) => ticket.deleted,
        SharedObject::Note(note) => note.deleted,
        _ => false,
    }
}

fn kind_of(body: &RecordBody) -> String {
    match &body.object {
        SharedObject::Ticket(_) => "ticket",
        SharedObject::Note(_) => "note",
        SharedObject::Columns { .. } => "columns",
        SharedObject::Board { .. } => "board",
        SharedObject::Members { .. } => "members",
    }
    .into()
}

fn other_word(r: &Wire) -> &'static str {
    match r {
        Wire::Registered { .. } => "registered",
        Wire::Device { .. } => "device",
        Wire::Boards { .. } => "boards",
        Wire::BoardCreated { .. } => "board_created",
        Wire::Ok => "ok",
        Wire::InviteMinted { .. } => "invite_minted",
        Wire::Joined { .. } => "joined",
        Wire::Members { .. } => "members",
        Wire::Keys { .. } => "keys",
        Wire::Head { .. } => "head",
        Wire::Accepted { .. } => "accepted",
        Wire::Records { .. } => "records",
        Wire::Error { .. } => "error",
        Wire::ControlInfo { .. } => "control_info",
        Wire::ControlMail { .. } => "control_mail",
        Wire::ControlShelf { .. } => "control_shelf",
        Wire::Grant { .. } => "grant",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesimon_team::wire::Credential;
    use std::sync::mpsc::{channel, Receiver};

    fn owner_daemon(dir: &std::path::Path) -> (Daemon, Receiver<Job>) {
        let (mut d, _, _) = super::super::queued_person_push_tests::daemon(dir);
        let (team, rx) = signed_in(None);
        d.team = team;
        let keys = d.team.device.as_ref().unwrap().keys().unwrap();
        let mut state = TeamState::new(BoardId::random(), Role::Owner, false, "Owner".into());
        state.pin_owner(keys.public(), "Owner".into());
        state.add_key(0, &BoardKey::generate());
        d.team.state = Some(state);
        (d, rx)
    }

    fn row(keys: &crypto::DeviceKeys, epochs: Vec<u32>) -> Member {
        Member {
            device: keys.id(),
            public: keys.public(),
            display_name: "Peer".into(),
            role: Role::Contributor,
            status: MemberStatus::Active,
            proof: None,
            epochs,
        }
    }

    fn invite_member(d: &mut Daemon, keys: &crypto::DeviceKeys, role: Role) -> Member {
        let owner = d.team.device.as_ref().unwrap().keys().unwrap();
        let code = InviteCode::mint(&owner.public());
        d.team.state.as_mut().unwrap().invites.push(PendingInvite {
            invite: "invite".into(),
            secret: String::new(),
            role: role.word().into(),
            code: code.encode(),
        });
        Member { proof: Some(code.proof(&keys.public())), ..row(keys, vec![u32::MAX]) }
    }

    fn key_requests(rx: &Receiver<Job>) -> Vec<Vec<crypto::WrappedKey>> {
        rx.try_iter()
            .filter_map(|job| match job {
                Job::Call { request: Request::PutKeys { wrapped, .. }, .. } => Some(wrapped),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn unknown_member_epochs_never_authorize_key_delivery() {
        let dir = tempfile::tempdir().unwrap();
        let (mut d, rx) = owner_daemon(dir.path());
        let unknown = crypto::DeviceKeys::generate();
        d.team_accept_members(vec![row(&unknown, vec![u32::MAX])]);
        assert!(key_requests(&rx).is_empty());
        assert!(!d
            .team
            .state
            .as_ref()
            .unwrap()
            .verified_members
            .contains_key(&unknown.id().to_hex()));
        assert!(d.team_info().board.unwrap().members[0].unverified);
    }

    #[test]
    fn rotation_required_wraps_only_persisted_verified_keys() {
        let dir = tempfile::tempdir().unwrap();
        let (mut d, rx) = owner_daemon(dir.path());
        let peer = crypto::DeviceKeys::generate();
        let member = invite_member(&mut d, &peer, Role::Contributor);
        d.team_accept_members(vec![member]);
        assert_eq!(key_requests(&rx)[0][0].recipient, peer.id());
        d.team.inflight.remove(&Tag::Keys);
        let unknown = crypto::DeviceKeys::generate();
        d.on_team(Done {
            tag: Tag::Head,
            result: Ok(Wire::Head {
                head: mesimon_team::wire::Head { seq: 0, epoch: 0, rotation_required: true },
            }),
        });
        d.team_accept_members(vec![row(&peer, vec![0]), row(&unknown, vec![u32::MAX])]);
        let sent = key_requests(&rx);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].len(), 2);
        assert!(sent[0].iter().all(|w| w.epoch == 1 && w.recipient != unknown.id()));
        let disk = TeamState::load(&d.paths.team_file()).unwrap().unwrap();
        assert!(sent[0].iter().all(|w| disk.verified_members.contains_key(&w.recipient.to_hex())));
    }

    #[test]
    fn proofs_pin_both_keys_and_local_role_before_wrapping() {
        let dir = tempfile::tempdir().unwrap();
        let (mut d, rx) = owner_daemon(dir.path());
        let peer = crypto::DeviceKeys::generate();
        let mut member = invite_member(&mut d, &peer, Role::Viewer);
        member.role = Role::Owner;
        d.team_accept_members(vec![member.clone(), member.clone()]);
        let disk = TeamState::load(&d.paths.team_file()).unwrap().unwrap();
        assert_eq!(disk.verified_members[&peer.id().to_hex()].role, Role::Viewer);
        assert!(disk.invites.is_empty());
        let sent = key_requests(&rx);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].len(), 1);
        assert!(crypto::unwrap(
            &sent[0][0],
            disk.board_id().unwrap(),
            &peer,
            &d.team.device.as_ref().unwrap().keys().unwrap().public()
        )
        .is_ok());
        d.team.inflight.clear();
        member.public.kex = crypto::DeviceKeys::generate().public().kex;
        d.team_accept_members(vec![member]);
        assert!(key_requests(&rx).is_empty());
        assert!(d.team.state.as_ref().unwrap().members.is_empty());
        assert_eq!(
            d.team.state.as_ref().unwrap().verified_members[&peer.id().to_hex()].public,
            peer.public()
        );
    }

    #[test]
    fn failed_trust_save_emits_no_wrap_or_rotation() {
        let dir = tempfile::tempdir().unwrap();
        let (mut d, rx) = owner_daemon(dir.path());
        let peer = crypto::DeviceKeys::generate();
        let member = invite_member(&mut d, &peer, Role::Contributor);
        std::fs::create_dir(d.paths.team_file()).unwrap();
        d.team_accept_members(vec![member]);
        d.team_rotate();
        assert!(key_requests(&rx).is_empty());
        assert!(d.team.state_dirty);
        assert!(d.team.error.is_some());
    }

    fn record(
        d: &Daemon,
        author: &crypto::DeviceKeys,
        object: ObjectId,
        revision: u64,
        body: RecordBody,
    ) -> StoredRecord {
        let state = d.team.state.as_ref().unwrap();
        let scope = RecordScope { board: state.board_id().unwrap(), object, revision };
        StoredRecord {
            object,
            revision,
            seq: revision,
            record: crypto::seal(
                &state.key(0).unwrap(),
                0,
                scope,
                author,
                &serde_json::to_vec(&body).unwrap(),
            )
            .unwrap(),
        }
    }

    fn peer_ticket() -> RecordBody {
        RecordBody::new(SharedObject::Ticket(mesimon_core::team::SharedTicket {
            title: "peer edit".into(),
            column: "TODO".into(),
            order: "a0".into(),
            created_at: "@1".into(),
            created_by: "peer".into(),
            archived: false,
            notes: vec![],
            deleted: false,
        }))
    }

    #[test]
    fn joined_board_accepts_peer_authors_only_through_owner_directory() {
        let dir = tempfile::tempdir().unwrap();
        let (mut d, _) = owner_daemon(dir.path());
        let owner = crypto::DeviceKeys::generate();
        let peer = crypto::DeviceKeys::generate();
        let state = d.team.state.as_mut().unwrap();
        state.role = Role::Contributor.word().into();
        state.content_only = true;
        state.verified_members.clear();
        state.pin_owner(owner.public(), "Owner".into());
        let peer_pin = VerifiedMember {
            public: peer.public(),
            display_name: "Peer".into(),
            role: Role::Contributor,
            active: true,
        };
        let owner_pin = state.verified_members[&owner.id().to_hex()].clone();
        let directory = |active, role| {
            RecordBody::new(SharedObject::Members {
                members: vec![
                    owner_pin.shared(),
                    VerifiedMember { active, role, ..peer_pin.clone() }.shared(),
                ],
            })
        };
        d.team_accept_members(vec![row(&peer, vec![0])]);
        let object = ObjectId::random();
        d.team_apply_record(&record(&d, &peer, object, 1, peer_ticket()));
        assert!(d.board.ticket(ulid::Ulid::from(object)).is_none());
        // Even a correctly signed directory from a peer is not owner authority.
        d.team_apply_record(&record(
            &d,
            &peer,
            ObjectId(MEMBERS_OBJECT),
            1,
            directory(true, Role::Contributor),
        ));
        assert!(!d.team.state.as_ref().unwrap().verified_members.contains_key(&peer.id().to_hex()));
        d.team_apply_record(&record(
            &d,
            &owner,
            ObjectId::random(),
            1,
            directory(true, Role::Contributor),
        ));
        assert!(!d.team.state.as_ref().unwrap().verified_members.contains_key(&peer.id().to_hex()));
        d.team_apply_record(&record(
            &d,
            &owner,
            ObjectId(MEMBERS_OBJECT),
            1,
            directory(true, Role::Contributor),
        ));
        d.team_apply_record(&record(&d, &peer, object, 1, peer_ticket()));
        assert_eq!(d.board.ticket(ulid::Ulid::from(object)).unwrap().title, "peer edit");
        d.team_apply_record(&record(
            &d,
            &peer,
            ObjectId(mesimon_core::team::BOARD_OBJECT),
            1,
            RecordBody::new(SharedObject::Board { title: "not the owner".into() }),
        ));
        assert_ne!(d.team.state.as_ref().unwrap().title.as_deref(), Some("not the owner"));
        // Downgrade, then revocation, are the signed owner's decisions.
        for (revision, active, role) in [(2, true, Role::Viewer), (3, false, Role::Contributor)] {
            d.team_apply_record(&record(
                &d,
                &owner,
                ObjectId(MEMBERS_OBJECT),
                revision,
                directory(active, role),
            ));
            d.team.state = TeamState::load(&d.paths.team_file()).unwrap();
            d.team_apply_record(&record(
                &d,
                &owner,
                ObjectId(MEMBERS_OBJECT),
                1,
                directory(true, Role::Contributor),
            ));
            let next = ObjectId::random();
            assert_eq!(
                d.team_apply_record(&record(&d, &peer, next, 1, peer_ticket())),
                ApplyOutcome::Rejected
            );
            assert!(d.board.ticket(ulid::Ulid::from(next)).is_none());
            assert_eq!(d.team.state.as_ref().unwrap().membership_revision, revision);
        }
    }

    #[test]
    fn local_revocation_survives_an_active_relay_row() {
        let dir = tempfile::tempdir().unwrap();
        let (mut d, rx) = owner_daemon(dir.path());
        let peer = crypto::DeviceKeys::generate();
        let member = invite_member(&mut d, &peer, Role::Contributor);
        d.team_accept_members(vec![member]);
        key_requests(&rx);
        d.team.inflight.clear();
        assert!(matches!(d.team_revoke(peer.id().to_hex()), Response::Ok));
        d.team_accept_members(vec![row(&peer, vec![u32::MAX])]);
        d.team_rotate();
        assert!(key_requests(&rx).iter().flatten().all(|w| w.recipient != peer.id()));
        let rejected = ObjectId::random();
        assert_eq!(
            d.team_apply_record(&record(&d, &peer, rejected, 1, peer_ticket())),
            ApplyOutcome::Rejected
        );
        assert!(!d
            .team
            .state
            .as_ref()
            .unwrap()
            .pending_records
            .iter()
            .any(|r| r.record.object == rejected));
        assert!(!d.team.state.as_ref().unwrap().verified_members[&peer.id().to_hex()].active);
        assert_eq!(
            d.team.state.as_ref().unwrap().verified_members,
            TeamState::load(&d.paths.team_file()).unwrap().unwrap().verified_members
        );
    }

    #[test]
    fn directory_save_outcomes_preserve_authority_and_leave_retry_to_the_caller() {
        let dir = tempfile::tempdir().unwrap();
        let (mut d, _) = owner_daemon(dir.path());
        let owner = crypto::DeviceKeys::generate();
        let peer = crypto::DeviceKeys::generate();
        let state = d.team.state.as_mut().unwrap();
        state.role = "contributor".into();
        state.verified_members.clear();
        state.pin_owner(owner.public(), "Owner".into());
        let old = state.verified_members.clone();
        let body = RecordBody::new(SharedObject::Members {
            members: vec![
                old[&owner.id().to_hex()].shared(),
                VerifiedMember {
                    public: peer.public(),
                    display_name: "Peer".into(),
                    role: Role::Contributor,
                    active: true,
                }
                .shared(),
            ],
        });
        let directory = record(&d, &owner, ObjectId(MEMBERS_OBJECT), 9, body);
        let peer_object = ObjectId::random();
        let peer_record = record(&d, &peer, peer_object, 1, peer_ticket());
        assert_eq!(d.team_apply_record(&peer_record), ApplyOutcome::Deferred);
        assert!(
            d.team.state.as_ref().unwrap().pending_records.is_empty(),
            "the receiver does not retain input itself"
        );
        assert_eq!(d.team_receive_record(peer_record), ApplyOutcome::Deferred);
        let old_state = d.team.state.clone();
        std::fs::create_dir(d.paths.team_file()).unwrap();
        assert_eq!(d.team_apply_record(&directory), ApplyOutcome::Deferred);
        assert_eq!(d.team.state, old_state, "authority, revision and cursor roll back together");
        assert_eq!(
            d.team.state.as_ref().unwrap().pending_records.len(),
            1,
            "directory retention belongs to the shared caller"
        );
        std::fs::remove_dir(d.paths.team_file()).unwrap();
        assert_eq!(d.team_apply_record(&directory), ApplyOutcome::Applied);
        let saved = TeamState::load(&d.paths.team_file()).unwrap().unwrap();
        assert_eq!(saved.membership_revision, 9);
        assert!(saved.verified_members.contains_key(&peer.id().to_hex()));
        assert_eq!(
            d.team.state.as_ref().unwrap().pending_records.len(),
            1,
            "no recursive retry from the directory receiver"
        );
        assert!(d.board.ticket(ulid::Ulid::from(peer_object)).is_none());
        d.team_retry_pending_records();
        assert!(d.team.state.as_ref().unwrap().pending_records.is_empty());
        assert!(d.board.ticket(ulid::Ulid::from(peer_object)).is_some());
        assert_eq!(d.team_apply_record(&directory), ApplyOutcome::Rejected);
        assert_eq!(d.team.state.as_ref().unwrap().membership_revision, 9);
    }

    #[test]
    fn a_claimed_self_author_does_not_authorize_an_inbound_floor() {
        let dir = tempfile::tempdir().unwrap();
        let (mut d, _) = owner_daemon(dir.path());
        let other = crypto::DeviceKeys::generate();
        let mut incoming = record(&d, &other, ObjectId::random(), u64::MAX, peer_ticket());
        incoming.record.author = d.team.device.as_ref().unwrap().keys().unwrap().id();
        let before = d.team.state.clone();
        assert_eq!(d.team_apply_record(&incoming), ApplyOutcome::Rejected);
        assert_eq!(d.team.state, before);
        assert!(d.team.state.as_ref().unwrap().pending_records.is_empty());
    }

    #[test]
    fn key_receipts_require_the_pinned_owner_and_join_requires_matching_identity() {
        let dir = tempfile::tempdir().unwrap();
        let (mut d, rx) = owner_daemon(dir.path());
        let owner = crypto::DeviceKeys::generate();
        let stranger = crypto::DeviceKeys::generate();
        let me = d.team.device.as_ref().unwrap().keys().unwrap();
        let state = d.team.state.as_mut().unwrap();
        state.role = "contributor".into();
        state.verified_members.clear();
        state.pin_owner(owner.public(), "Owner".into());
        state.members = vec![Member { role: Role::Owner, ..row(&stranger, vec![0, 1]) }];
        let board = state.board_id().unwrap();
        let key = BoardKey::generate();
        d.team_on_keys(vec![crypto::wrap(&key, 1, board, &stranger, &me.public())]);
        assert!(d.team.state.as_ref().unwrap().key(1).is_none());
        d.team_on_keys(vec![crypto::wrap(&key, 1, board, &owner, &me.public())]);
        assert_eq!(d.team.state.as_ref().unwrap().key(1), Some(key));
        d.team.joining = Some(InviteCode::mint(&owner.public()));
        d.team_on_joined(
            board,
            Role::Contributor,
            Member { device: stranger.id(), ..row(&owner, vec![]) },
        );
        assert!(rx
            .try_iter()
            .any(|job| matches!(job, Job::Call { request: Request::Leave { .. }, .. })));
        assert!(d.team.error.is_some());
    }

    fn combined_joined_daemon(
        dir: &std::path::Path,
        owner: &crypto::DeviceKeys,
    ) -> (Daemon, Receiver<Job>) {
        let (mut d, jobs) = owner_daemon(dir);
        let state = d.team.state.as_mut().unwrap();
        state.role = Role::Contributor.word().into();
        state.content_only = true;
        state.verified_members.clear();
        state.pin_owner(owner.public(), "Owner".into());
        assert!(d.team_save());
        store::save_columns(&d.paths, &d.board).unwrap();
        (d, jobs)
    }

    fn combined_directory(
        d: &Daemon,
        owner: &crypto::DeviceKeys,
        peer: &crypto::DeviceKeys,
        active: bool,
    ) -> RecordBody {
        let owner_pin =
            d.team.state.as_ref().unwrap().verified_members[&owner.id().to_hex()].clone();
        let local = d.team.device.as_ref().unwrap().keys().unwrap();
        RecordBody::new(SharedObject::Members {
            members: vec![
                owner_pin.shared(),
                VerifiedMember {
                    public: local.public(),
                    display_name: "Local".into(),
                    role: Role::Contributor,
                    active: true,
                }
                .shared(),
                VerifiedMember {
                    public: peer.public(),
                    display_name: "Peer".into(),
                    role: Role::Contributor,
                    active,
                }
                .shared(),
            ],
        })
    }

    // Reload only durable state; do not accidentally save the old process's RAM.
    // Reset the entire TeamCtx so volatile pending/cursor/cache state cannot make
    // these tests pass. The local device identity survives as it does on restart.
    // These cases do not need startup network traffic or note-projection caches.
    fn combined_reload(d: &mut Daemon) -> Receiver<Job> {
        let device = d.team.device.clone();
        let state = TeamState::load(&d.paths.team_file()).unwrap();
        let board = store::load(&d.paths).unwrap().board;
        let (mut team, jobs) = signed_in(None);
        team.device = device;
        team.state = state;
        d.team = team;
        d.board = board;
        jobs
    }

    #[test]
    fn combined_deferred_directory_survives_cursor_reload_and_applies_revocation() {
        let dir = tempfile::tempdir().unwrap();
        let owner = crypto::DeviceKeys::generate();
        let peer = crypto::DeviceKeys::generate();
        let (mut d, _jobs) = combined_joined_daemon(dir.path(), &owner);
        let active = record(
            &d,
            &owner,
            ObjectId(MEMBERS_OBJECT),
            1,
            combined_directory(&d, &owner, &peer, true),
        );
        assert_eq!(d.team_receive_record(active.clone()), ApplyOutcome::Applied);
        let mut revoked = record(
            &d,
            &owner,
            ObjectId(MEMBERS_OBJECT),
            2,
            combined_directory(&d, &owner, &peer, false),
        );
        revoked.seq = 40;
        let path = d.paths.team_file();
        let backup = path.with_extension("run2-backup");
        std::fs::rename(&path, &backup).unwrap();
        std::fs::create_dir(&path).unwrap();

        assert_eq!(d.team_receive_record(revoked.clone()), ApplyOutcome::Deferred);
        let state = d.team.state.as_ref().unwrap();
        assert_eq!(state.membership_revision, 1);
        assert!(state.verified_members[&peer.id().to_hex()].active);
        assert!(state.pending_records.iter().any(|p| p.matches(&revoked)));
        let prior_cursor = state.cursor;
        assert!(!d.team_save_cursor(40));
        assert_eq!(d.team.state.as_ref().unwrap().cursor, prior_cursor);

        std::fs::remove_dir(&path).unwrap();
        std::fs::rename(&backup, &path).unwrap();
        // Persist page progress and deferred input without applying it first.
        assert!(d.team_save_cursor(40));
        let _reloaded_jobs = combined_reload(&mut d);
        let state = d.team.state.as_ref().unwrap();
        assert_eq!(state.cursor, 40);
        assert_eq!(state.membership_revision, 1);
        assert!(state.pending_records.iter().any(|p| p.matches(&revoked)));
        d.team_retry_pending_records();
        let saved = TeamState::load(&path).unwrap().unwrap();
        assert_eq!(saved.cursor, 40);
        assert_eq!(saved.membership_revision, 2);
        assert!(!saved.verified_members[&peer.id().to_hex()].active);
        assert!(saved.pending_records.is_empty());

        let _reloaded_jobs = combined_reload(&mut d);
        let mut replay = active;
        replay.seq = 10_000;
        assert_eq!(d.team_receive_record(replay), ApplyOutcome::Rejected);
        let object = ObjectId::random();
        let edit = record(&d, &peer, object, 50, peer_ticket());
        assert_eq!(d.team_receive_record(edit), ApplyOutcome::Rejected);
        assert!(d.board.ticket(ulid::Ulid::from(object)).is_none());
        let state = d.team.state.as_ref().unwrap();
        assert_eq!(state.membership_revision, 2);
        assert!(!state.verified_members[&peer.id().to_hex()].active);
        assert_eq!(state.revision(&object.to_hex()), None);
    }

    #[test]
    fn combined_unknown_peer_note_waits_for_directory_and_parent_then_applies_once() {
        let dir = tempfile::tempdir().unwrap();
        let owner = crypto::DeviceKeys::generate();
        let peer = crypto::DeviceKeys::generate();
        let (mut d, _jobs) = combined_joined_daemon(dir.path(), &owner);
        let ticket = ulid::Ulid::new();
        let note = ulid::Ulid::new();
        let object = ObjectId::from(note);
        let mut incoming = record(
            &d,
            &peer,
            object,
            7,
            project::note_body(ticket, "peer note after catch-up".into()),
        );
        incoming.seq = 10;
        assert_eq!(d.team_receive_record(incoming.clone()), ApplyOutcome::Deferred);
        assert_eq!(d.team.state.as_ref().unwrap().revision(&object.to_hex()), None);
        assert!(d.team_save_cursor(10));
        let _reloaded_jobs = combined_reload(&mut d);
        assert_eq!(d.team.state.as_ref().unwrap().cursor, 10);
        assert!(d
            .team
            .state
            .as_ref()
            .unwrap()
            .pending_records
            .iter()
            .any(|p| p.matches(&incoming)));

        let mut directory = record(
            &d,
            &owner,
            ObjectId(MEMBERS_OBJECT),
            1,
            combined_directory(&d, &owner, &peer, true),
        );
        directory.seq = 20;
        assert_eq!(d.team_receive_record(directory), ApplyOutcome::Applied);
        d.team_retry_pending_records();
        assert!(d.board.ticket(ticket).is_none());
        assert_eq!(d.team.state.as_ref().unwrap().revision(&object.to_hex()), None);
        assert!(d
            .team
            .state
            .as_ref()
            .unwrap()
            .pending_records
            .iter()
            .any(|p| p.matches(&incoming)));
        assert!(d.team_save_cursor(20));
        let _reloaded_jobs = combined_reload(&mut d);

        let mut parent = record(&d, &peer, ObjectId::from(ticket), 3, peer_ticket());
        parent.seq = 30;
        assert_eq!(d.team_receive_record(parent), ApplyOutcome::Applied);
        d.team_retry_pending_records();
        let t = d.board.ticket(ticket).unwrap();
        assert_eq!(t.notes.len(), 1);
        assert_eq!(t.notes[0].id, note);
        assert_eq!(t.notes[0].rev, 1);
        assert_eq!(
            store::read_note(&d.paths, &t.short_key, note).unwrap(),
            "peer note after catch-up"
        );
        assert_eq!(d.team.state.as_ref().unwrap().revision(&object.to_hex()), Some(7));
        assert!(d.team.state.as_ref().unwrap().pending_records.is_empty());
        assert!(d.team_save_cursor(30));
        let _reloaded_jobs = combined_reload(&mut d);
        incoming.seq = 10_000;
        assert_eq!(d.team_receive_record(incoming), ApplyOutcome::Rejected);
        let mut older =
            record(&d, &peer, object, 6, project::note_body(ticket, "older body".into()));
        older.seq = 10_001;
        assert_eq!(d.team_receive_record(older), ApplyOutcome::Rejected);
        d.team_retry_pending_records();
        let t = d.board.ticket(ticket).unwrap();
        assert_eq!(t.notes.len(), 1);
        assert_eq!(t.notes[0].rev, 1);
        assert_eq!(
            store::read_note(&d.paths, &t.short_key, note).unwrap(),
            "peer note after catch-up"
        );
        assert_eq!(d.team.state.as_ref().unwrap().revision(&object.to_hex()), Some(7));
    }

    #[test]
    fn combined_older_directory_and_content_cannot_replace_reloaded_state_at_later_sequences() {
        let dir = tempfile::tempdir().unwrap();
        let owner = crypto::DeviceKeys::generate();
        let peer = crypto::DeviceKeys::generate();
        let (mut d, _jobs) = combined_joined_daemon(dir.path(), &owner);
        let old_directory = record(
            &d,
            &owner,
            ObjectId(MEMBERS_OBJECT),
            1,
            combined_directory(&d, &owner, &peer, true),
        );
        assert_eq!(d.team_receive_record(old_directory.clone()), ApplyOutcome::Applied);
        let current_directory = record(
            &d,
            &owner,
            ObjectId(MEMBERS_OBJECT),
            2,
            combined_directory(&d, &owner, &peer, true),
        );
        assert_eq!(d.team_receive_record(current_directory), ApplyOutcome::Applied);
        let object = ObjectId::random();
        let old_content = record(&d, &peer, object, 4, peer_ticket());
        assert_eq!(d.team_receive_record(old_content.clone()), ApplyOutcome::Applied);
        let mut latest = peer_ticket();
        if let SharedObject::Ticket(ticket) = &mut latest.object {
            ticket.title = "newer durable content".into();
        }
        let current = record(&d, &peer, object, 8, latest);
        assert_eq!(d.team_receive_record(current), ApplyOutcome::Applied);
        assert!(d.team_save_cursor(100));
        let _reloaded_jobs = combined_reload(&mut d);
        let verified_before = d.team.state.as_ref().unwrap().verified_members.clone();
        let published_before = d.team.state.as_ref().unwrap().published[&object.to_hex()].clone();

        let mut replay = old_directory;
        replay.seq = 1_000;
        assert_eq!(d.team_receive_record(replay), ApplyOutcome::Rejected);
        let mut replay = old_content;
        replay.seq = 1_001;
        assert_eq!(d.team_receive_record(replay), ApplyOutcome::Rejected);
        let state = d.team.state.as_ref().unwrap();
        assert_eq!(state.membership_revision, 2);
        assert_eq!(state.verified_members, verified_before);
        assert!(state.verified_members[&peer.id().to_hex()].active);
        assert_eq!(state.revision(&object.to_hex()), Some(8));
        assert_eq!(state.published[&object.to_hex()], published_before);
        assert_eq!(
            d.board.ticket(ulid::Ulid::from(object)).unwrap().title,
            "newer durable content"
        );
        assert!(state.pending_records.is_empty());
        // Rejected replay can still advance the transport cursor, never the facts.
        assert!(d.team_save_cursor(1_001));
        let _reloaded_jobs = combined_reload(&mut d);
        let state = d.team.state.as_ref().unwrap();
        assert_eq!(state.cursor, 1_001);
        assert_eq!(state.membership_revision, 2);
        assert_eq!(state.revision(&object.to_hex()), Some(8));
        assert_eq!(
            d.board.ticket(ulid::Ulid::from(object)).unwrap().title,
            "newer durable content"
        );
    }

    const NOW: u64 = 1_790_000_000;

    /// A signed-in context whose calls land on the returned receiver.
    fn signed_in(code: Option<&str>) -> (TeamCtx, Receiver<Job>) {
        let (jobs, rx) = channel();
        let mut team = TeamCtx::with_jobs(jobs);
        let relay = RelayEndpoint::parse("relay.example:9000").unwrap();
        team.device = Some(DeviceFile {
            credential: Some(Credential::generate()),
            access_code: code.map(Into::into),
            ..DeviceFile::fresh("Dana".into(), relay)
        });
        (team, rx)
    }

    /// The calls sent since the last look, by tag.
    fn sent(rx: &Receiver<Job>) -> Vec<Tag> {
        rx.try_iter()
            .filter_map(|job| match job {
                Job::Call { tag, .. } => Some(tag),
                Job::Connect { .. } => None,
            })
            .collect()
    }

    /// The relay answered `tag`, and nothing on the context changed: how
    /// a failed renewal looks from here (the daemon's arm only words it).
    fn answered(team: &mut TeamCtx, tag: &Tag) {
        team.inflight.remove(tag);
    }

    /// T-522: inside the grant's last day the kept code goes by itself,
    /// once; outside it, or with nothing to go on, nothing does. A failed
    /// renewal is not retried by the clock.
    #[test]
    fn a_grant_in_its_last_day_is_renewed_once_on_the_tick() {
        let (mut team, rx) = signed_in(Some("MSMN-1"));
        team.grant_until = Some(Some(NOW + RENEW_WINDOW + 60));
        team.grant_tick(1, NOW);
        assert_eq!(sent(&rx), [], "a day and a minute left: not yet");

        team.grant_until = Some(Some(NOW + 3_600));
        team.grant_tick(2, NOW);
        assert_eq!(sent(&rx), [Tag::Redeem]);
        team.grant_tick(3, NOW + 1);
        assert_eq!(sent(&rx), [], "one in flight is enough");
        answered(&mut team, &Tag::Redeem);
        team.grant_tick(4, NOW + RENEW_AGAIN);
        assert_eq!(sent(&rx), [], "a failed renewal waits for a person");

        // A grant that has already run out, learned at connect, renews too.
        let (mut team, rx) = signed_in(Some("MSMN-1"));
        team.grant_until = Some(Some(NOW - 60));
        team.grant_tick(1, NOW);
        assert_eq!(sent(&rx), [Tag::Redeem]);

        for (code, until) in [
            (Some("MSMN-1"), Some(None)), // a grant with no end
            (Some("MSMN-1"), None),       // a relay from before: never said
            (None, Some(Some(NOW + 60))), // no code kept to renew with
        ] {
            let (mut team, rx) = signed_in(code);
            team.grant_until = until;
            team.grant_tick(1, NOW);
            assert_eq!(sent(&rx), [], "{code:?} {until:?}");
        }
    }

    /// A renewal that lands does not make the next one immediate: a grant
    /// the provider extends a day at a time (past due, cancelling) is asked
    /// again after `RENEW_AGAIN`, before it runs out, and not before.
    #[test]
    fn a_renewal_that_lands_is_followed_by_one_per_interval() {
        let (mut team, rx) = signed_in(Some("MSMN-1"));
        team.grant_until = Some(Some(NOW + 3_600));
        team.grant_tick(1, NOW);
        assert_eq!(sent(&rx), [Tag::Redeem]);
        answered(&mut team, &Tag::Redeem);
        team.redeemed(2);
        assert_eq!(sent(&rx), [Tag::Grant], "the new end is asked for");
        assert!(!team.lapsed && team.granted && !team.renewal_tried);

        // Past due: the relay's floor, a day from the redeem.
        answered(&mut team, &Tag::Grant);
        team.grant_until = Some(Some(NOW + RENEW_WINDOW));
        team.grant_tick(3, NOW + 1);
        team.grant_tick(4, NOW + RENEW_AGAIN - 1);
        assert_eq!(sent(&rx), []);
        team.grant_tick(5, NOW + RENEW_AGAIN);
        assert_eq!(sent(&rx), [Tag::Redeem]);
    }

    /// The phone's mail refused for this Mac's lapse is the same edge as a
    /// refused write: one renewal by itself, and after it fails a second
    /// refusal only says so. A code a person enters starts the count again.
    #[test]
    fn a_mail_refusal_renews_once_and_a_failed_renewal_is_not_retried() {
        let (mut team, rx) = signed_in(Some("MSMN-1"));
        team.on_lapse(NOW);
        assert!(team.lapsed);
        assert_eq!(sent(&rx), [Tag::Redeem]);
        assert_eq!(team.error, None, "the renewal speaks for itself");

        answered(&mut team, &Tag::Redeem);
        team.on_lapse(NOW + 5);
        assert_eq!(sent(&rx), [], "a second refusal after a failed renewal");
        assert_eq!(team.error.as_deref(), Some(ErrorCode::GrantLapsed.to_string().as_str()));

        team.redeemed(9);
        assert_eq!(sent(&rx), [Tag::Grant]);
        team.on_lapse(NOW + 10);
        assert_eq!(sent(&rx), [Tag::Redeem], "an accepted code resets the one try");

        let (mut team, rx) = signed_in(None);
        team.on_lapse(NOW);
        assert_eq!(sent(&rx), [], "nothing kept to renew with");
        assert!(team.lapsed && team.error.is_some());
    }

    /// The grant's end is asked hourly, and only by a device that kept a
    /// code: there is nothing to renew without one.
    #[test]
    fn the_grant_is_asked_hourly_with_a_kept_code() {
        let (mut team, rx) = signed_in(Some("MSMN-1"));
        team.grant_tick(GRANT_EVERY - 1, NOW);
        assert_eq!(sent(&rx), []);
        team.grant_tick(GRANT_EVERY, NOW);
        assert_eq!(sent(&rx), [Tag::Grant]);
        answered(&mut team, &Tag::Grant);
        team.grant_tick(2 * GRANT_EVERY - 1, NOW);
        assert_eq!(sent(&rx), []);

        let (mut team, rx) = signed_in(None);
        team.grant_tick(GRANT_EVERY, NOW);
        assert_eq!(sent(&rx), []);
    }

    /// A viewer may read, tag, and mark a card seen; every write to a
    /// ticket or a note is refused, and so is minting one.
    #[test]
    fn a_viewer_may_read_and_tag_and_nothing_else() {
        let id = ulid::Ulid::new();
        let edits = [
            Command::CreateTicket {
                column: "TODO".into(),
                title: "x".into(),
                workspace: None,
                tier: None,
            },
            Command::CreateTicketWithNote {
                column: "TODO".into(),
                title: "x".into(),
                workspace: None,
                text: String::new(),
                uploads: Vec::new(),
                tags: Vec::new(),
                tier: None,
            },
            Command::RenameTicket { id, title: "y".into() },
            Command::MoveTicket { id, column: "DOING".into(), before: None },
            Command::ArchiveTicket { id },
            Command::UnarchiveTicket { id },
            Command::DeleteTicket { id, discard_worktree: false },
            Command::WriteNote { ticket: id, note: None, text: "n".into(), rev: None },
            Command::DuplicateTicket { id },
            Command::ArchiveAll,
        ];
        for c in &edits {
            assert!(viewer_edit(c), "{} should be refused for a viewer", c.wire_name());
        }
        let reads = [
            Command::Snapshot,
            Command::ReadNote { ticket: id, note: id },
            Command::SetTag { id, group: 1, name: Some("FEATURE".into()) },
            Command::SeenTicket { id },
            Command::OpenedTicket { id },
            Command::TeamRefresh,
            Command::LeaveBoard,
            Command::SetStatusLine { top: true },
        ];
        for c in &reads {
            assert!(!viewer_edit(c), "{} is a viewer's own", c.wire_name());
        }
    }
}

#[cfg(test)]
#[path = "teamglue_tests.rs"]
mod record_tests;
