// Application state and every action on it. The view reads this object and
// re-renders on `emit`; nothing here touches the DOM except history, focus
// requests and the theme attribute. Transport stays in connection.js.
import { Connection } from "./connection.js";
import { BoardState } from "./board.js";
import { Sessions, answerBusy, landed } from "./sessions.js";
import { sendRefused, waitsOnYou } from "./queue.js";
import { Sent } from "./sent.js";
import { Mailbox } from "./mailbox.js";
import { Starts, startWaiting } from "./starts.js";
import { Edits } from "./edits.js";
import { NoteBook, NoteMail, NOTE_MAX_BYTES, nameOf } from "./notes.js";
import { showAlert, clearAlerts } from "./awareness.js";
import { insertToken, linked, nextNumber, picture, pieceOf, unlinked, withoutPicture } from "./pictures.js";
import { mergePage, tailAsk } from "./transcript.js";
import { Peeks, shelfItem } from "./shelf.js";

const narrow = () => matchMedia("(max-width: 700px)").matches;
const receiptOps = ["prompt", "send_now", "take_back", "permission", "dialog", "status"];
// A filed ticket's description is its first note: the host's note limit.
export const DESCRIPTION_MAX_BYTES = 32 * 1024;
// A first prompt's cap, the daemon's `PROMPT_MAX_BYTES` (T-510).
export const PROMPT_MAX_BYTES = 4096;
// The longest envelope the relay keeps (control::MAIL_BYTES).
const MAIL_BYTES = 128 * 1024;
const sentContext = (item) => `sent:${item.id}`;
const startContext = (ticket) => `start:${ticket}`;
// The tier a start or a prompt carries (T-643): a pick this ticket may make
// that is not already its own, and only to a host that takes one.
const tierPick = (store, ticket, tier) =>
  tier && tier !== ticket?.tier && store.tierChoices(ticket).some((t) => t.id === tier) ? tier : undefined;
const noteContext = (item) => `notew:${item.id}`;
// How often a page opened from the kept copy asks whether the relay is back.
const PROBE_MS = 5000;
// Opened from the home screen, not a browser tab.
const standalone = () =>
  matchMedia("(display-mode: standalone)").matches || navigator.standalone === true;
// iPhone and iPad Safari add to the home screen from the Share sheet only.
const appleTouch = () =>
  /iP(hone|ad|od)/.test(navigator.userAgent) ||
  (navigator.platform === "MacIntel" && navigator.maxTouchPoints > 1);
// A pairing code from a link: the characters a code is spelled with, bounded.
const linkCode = (text) => (text || "").trim().slice(0, 80).replace(/[^0-9A-Za-z -]/g, "");
const emptyDraft = (board) => ({
  open: false,
  board,
  title: "",
  description: "",
  column: "",
  tags: [],
  error: "",
  replaces: undefined,
  // Pictures the details name (T-670), as a note's draft holds them.
  pictures: [],
  reading: 0,
  sending: "",
});
// A draft's held thumbnails, let go with the draft.
const dropThumbs = (draft) => {
  for (const p of draft?.pictures || []) p.thumb?.close?.();
};

export class Store {
  constructor(Browser) {
    this.Browser = Browser;
    this.listeners = new Set();
    this.version = 0;
    this.boards = new Map();
    this.sessions = new Sessions();
    this.live = false;
    this.down = false; // the last attempt failed; retries keep that reading
    this.online = navigator.onLine !== false;
    this.screen = "pair";
    this.status = "Starting…";
    this.pairReady = false;
    this.pairCode = "";
    this.deviceName = "";
    this.detailOpen = false;
    this.sheetOpen = false;
    // The Settings dialog (T-548), opened from the sidebar's foot or the pill.
    this.settingsOpen = false;
    // The sidebar's board picker (T-510), open under the board's name.
    this.boardMenuOpen = false;
    // The board a Forget press in that menu asks about (T-673).
    this.forgetting = undefined;
    // The first-prompt sheet a start opens (T-510): the ticket and the words.
    this.startAsk = undefined;
    this.theme = "system";
    this.alertStatus = "";
    // The desktop sidebar folded to a rail of icons (T-506); app.js reads
    // the remembered choice in.
    this.rail = false;
    this.focus = null;
    this.outputKey = undefined;
    // The ticket page's output (T-626): the conversation (`chat`) or the
    // pane's screen (`raw`); app.js reads the remembered choice in.
    this.outputView = "chat";
    this.remembered = new Map(); // board id -> signature of the stored snapshot
    this.sent = new Sent();
    this.sentLoaded = new Set(); // boards whose stored Sent list is read
    this.composer = emptyDraft(undefined);
    this.starts = new Starts(); // agents this tab started (T-498)
    // Card edits waiting on the host (T-530), the title being written over
    // the ticket page's heading, and the column-and-tags sheet's ticket.
    this.edits = new Edits();
    this.renaming = undefined;
    this.cardSheet = undefined;
    this.editError = "";
    // Notes (T-532): what this browser read, per board, and the edits it
    // sent; the note open for reading ({ ticket, note }) and the edit sheet.
    this.noteBooks = new Map();
    this.noteMail = new NoteMail();
    this.notesLoaded = new Set(); // boards whose stored notes are read
    this.reading = undefined;
    this.noteDraft = undefined;
    // The sheet listing every note of a ticket past two (T-627).
    this.notesSheet = undefined;
    this.toast = undefined;
    this.depositing = new Set(); // ids handed to the mailbox socket, unanswered
    // The shelf (T-698): the boards this away spell asked the relay about.
    this.peeks = new Peeks();
    // True when this page came from the service worker's kept copy (T-497):
    // no socket opens, and the relay's own page replaces it when it can.
    this.kept = false;
    // Home screen (T-497): the browser's own install prompt when it offers
    // one, and whether this page already runs from the home screen.
    this.install = { prompt: undefined, standalone: standalone(), apple: appleTouch() };
  }
  subscribe(fn) {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }
  emit() {
    this.version++;
    for (const fn of this.listeners) fn();
  }
  sync() {
    this.entry =
      this.board && this.active
        ? this.sessions.get(this.active.pin.board, this.board.current)
        : undefined;
    this.emit();
  }
  entryFor(ticket) {
    return this.active && ticket
      ? this.sessions.get(this.active.pin.board, ticket)
      : undefined;
  }
  // What the header's status pill says.
  get link() {
    // The kept copy loads when the relay's page did not come in time.
    if (this.kept) return this.online ? "relay" : "nonet";
    if (this.live) return "live";
    if (!this.online) return "nonet";
    if (this.down) return this.connection?.relayReached ? "asleep" : "relay";
    return "connecting";
  }
  save() {
    return this.storage.save(this.identity);
  }
  persist() {
    this.save().catch(() => {
      this.status = "Could not save browser preferences. They may be lost on reload.";
      this.emit();
    });
  }

  // ---- remembered board ------------------------------------------------
  rememberBoard() {
    const id = this.active?.pin.board;
    // A board from the shelf (T-698) is the terminal's, as of when it wrote
    // it: kept as a live one is, stripped of what is never kept.
    if (!id || !this.board || (this.board.cached && !this.board.shelved)) return;
    const snapshot = this.board.snapshot();
    const signature = JSON.stringify(snapshot);
    if (this.remembered.get(id) === signature) return;
    this.remembered.set(id, signature);
    this.storage
      .saveBoard(id, { snapshot, savedAt: this.board.receivedAt })
      .catch(() => this.remembered.delete(id));
  }
  async restoreBoard(chosen) {
    let saved;
    try {
      saved = await this.storage.readBoard(chosen.pin.board);
    } catch {
      return;
    }
    if (!saved?.snapshot || this.boards.has(chosen.pin.board)) return;
    const board = new BoardState(chosen.selected);
    board.update(saved.snapshot, { cached: true, at: saved.savedAt });
    this.boards.set(chosen.pin.board, board);
    if (this.active === chosen && !this.board) {
      this.board = board;
      this.sync();
    }
  }
  forgetRemembered(board) {
    this.remembered.delete(board);
    this.peeks.again(board);
    this.sent.purge(board);
    this.sentLoaded.delete(board);
    this.noteBooks.delete(board);
    this.noteMail.purge(board);
    this.notesLoaded.delete(board);
    if (this.noteDraft?.board === board) this.noteDraft = undefined;
    this.reading = undefined;
    return this.storage?.dropBoards(board).catch(() => {});
  }
  async restoreSent(board) {
    if (!board || this.sentLoaded.has(board)) return;
    this.sentLoaded.add(board);
    let saved;
    try {
      saved = await this.storage.readSent(board);
    } catch {
      return;
    }
    // A revoke or forget while the read was out wins over what it found.
    if (!this.sentLoaded.has(board)) return;
    this.sent.restore(board, saved?.items);
    if (this.active?.pin.board === board && this.connection?.online) this.receipts();
    this.emit();
  }
  persistSent(board = this.active?.pin.board) {
    if (!board || !this.storage) return;
    this.storage.saveSent(board, { items: this.sent.stored(board) }).catch(() => {
      this.status = "Could not save the Sent list. It may be lost on reload.";
      this.emit();
    });
  }

  noteBook(board = this.active?.pin.board) {
    let book = this.noteBooks.get(board);
    if (!book) this.noteBooks.set(board, (book = new NoteBook()));
    return book;
  }
  async restoreNotes(board) {
    if (!board || this.notesLoaded.has(board)) return;
    this.notesLoaded.add(board);
    let saved;
    try {
      saved = await this.storage.readNotes(board);
    } catch {
      return;
    }
    if (!this.notesLoaded.has(board)) return;
    this.noteBook(board).restore(saved?.tickets);
    this.noteMail.restore(board, saved?.outbox);
    if (this.active?.pin.board === board && this.connection?.online) this.receipts();
    if (this.active?.pin.board === board && this.mailbox?.ready) this.mailReady();
    this.emit();
  }
  persistNotes(board = this.active?.pin.board) {
    if (!board || !this.storage) return;
    this.storage
      .saveNotes(board, { tickets: this.noteBook(board).stored(), outbox: this.noteMail.stored(board) })
      .catch(() => {
        this.status = "Could not save notes in this browser. They may be lost on reload.";
        this.emit();
      });
  }

  // ---- navigation --------------------------------------------------------
  // On a phone the ticket is a screen of its own and the system's Back closes
  // it (T-667): every way it opens (a tap, a notification's link, the
  // selection remembered at boot or on a board switch) leaves an entry to go
  // back from, or Back leaves the app.
  detail(open) {
    if (open && !this.detailOpen) this.outputKey = undefined;
    this.detailOpen = open;
    if (!open) this.focus = "row";
    if (open && narrow() && !history.state?.detail) history.pushState({ detail: true }, "");
  }
  select(id) {
    if (!this.board) return;
    if (this.board.selected !== id) this.renaming = undefined;
    this.reading = undefined;
    this.board.selected = id;
    this.active.selected = id;
    this.persist();
    this.detail(true);
    this.focus = "selection";
    this.sync();
    this.preview();
    this.foreground();
    this.loadNotes();
  }
  back() {
    this.reading = undefined;
    if (history.state?.detail) history.back();
    else this.detail(false);
    this.focus = "row";
    this.sync();
  }
  popstate(state) {
    this.detail(!!state?.detail);
    if (!state?.note) this.reading = undefined;
    this.sync();
  }
  setMode(mode) {
    if (!this.board) return;
    this.board.mode = mode;
    // Wide screens show the board as columns; a card opens the side panel.
    if (!narrow()) this.detailOpen = false;
    this.sync();
  }
  setSearch(value) {
    if (!this.board) return;
    this.board.search = value;
    this.board.scroll[this.board.mode] = 0;
    this.sync();
  }
  setColumn(column) {
    if (!this.board) return;
    this.board.column = column;
    this.board.scroll.board = 0;
    this.sync();
  }
  // A phone's swipe on the Board (T-624): the next or the previous column,
  // stopping at either end.
  stepColumn(step) {
    const columns = this.board?.columns || [];
    const next = columns[columns.indexOf(this.board?.column) + step];
    if (next !== undefined) this.setColumn(next);
  }
  openSheet(open) {
    this.sheetOpen = open;
    this.boardMenuOpen = false;
    this.forgetting = undefined;
    this.emit();
  }
  openSettings(open) {
    if (this.settingsOpen === open) return;
    this.settingsOpen = open;
    this.sheetOpen = false;
    this.boardMenuOpen = false;
    this.emit();
  }
  openBoardMenu(open) {
    if (this.boardMenuOpen === open) return;
    this.boardMenuOpen = open;
    this.forgetting = undefined;
    this.emit();
  }
  askForget(board) {
    this.forgetting = board;
    this.emit();
  }
  // One paired board forgotten on this browser (T-673): its pairing and
  // everything kept for it go, and the others stay. Nothing tells its host,
  // which may be long gone; a live one still lists this browser until it is
  // revoked there.
  async forgetBoard(id) {
    this.forgetting = undefined;
    const chosen = this.identity?.boards.find((b) => b.pin.board === id);
    if (!chosen) return this.emit();
    const shown = this.active === chosen;
    if (shown) {
      this.connection.stop();
      this.mailbox?.want(false);
      clearAlerts();
      this.live = false;
      this.active = this.board = this.entry = undefined;
    }
    this.sessions.purge(id);
    this.starts.purge(id);
    this.edits.purge(id);
    this.boards.delete(id);
    const dropped = this.forgetRemembered(id);
    if (this.composer.board === id) this.composer = emptyDraft(undefined);
    if (this.returnBoard === chosen) this.returnBoard = undefined;
    this.identity.boards = this.identity.boards.filter((b) => b !== chosen);
    if (this.identity.lastBoard === id) this.identity.lastBoard = undefined;
    this.boardMenuOpen = false;
    const name = chosen.title || "Paired board";
    try {
      await Promise.all([this.save(), dropped]);
      this.status = `Forgot ${name} on this browser.`;
    } catch {
      this.status = `Could not forget ${name}. Clear this site's browser storage.`;
    }
    if (shown) {
      const next = this.identity.boards.find((b) => !b.revoked) || this.identity.boards[0];
      if (next) return this.openBoard(next);
      this.showPairing();
      this.pairReady = true;
    }
    this.emit();
  }
  navigateTicket(boardId, ticket) {
    const chosen = this.identity?.boards.find((b) => b.pin.board === boardId && !b.revoked);
    if (!chosen) return;
    chosen.selected = ticket;
    if (this.active?.pin.board === boardId && this.board?.tickets.some((t) => t.id === ticket))
      this.select(ticket);
    else {
      this.boards.delete(boardId);
      this.openBoard(chosen);
      this.detail(true);
      this.sync();
    }
  }
  visibleTicket() {
    return !document.hidden &&
      document.hasFocus() &&
      this.board?.current &&
      (!narrow() || this.detailOpen)
      ? this.board.current.id
      : null;
  }

  // ---- host requests -----------------------------------------------------
  // The output's next ask: what the conversation gained since the last one,
  // or the pane's screen in the raw view and from a host with no transcript.
  preview() {
    const current = this.board?.current;
    if (document.hidden || !current?.agent || !this.entry) return;
    if (this.chatShown) {
      if (!this.connection.has("transcript"))
        this.connection.request(
          { op: "transcript", ticket: current.id, session: current.agent.session, ...tailAsk(this.entry.chat) },
          this.entry.key,
        );
    } else if (current.agent.state !== "sleeping" && !this.connection.has("preview"))
      this.connection.request(
        { op: "preview", ticket: current.id, session: current.agent.session },
        this.entry.key,
      );
  }
  // Does this host read the conversation (T-626)?
  // Away, a conversation this page holds still reads (T-698): one read
  // live before, or the shelf's newest page.
  get chatCapable() {
    return !!this.connection?.features?.includes("transcript") || (!this.live && !!this.entry?.chat);
  }
  // Away, the screen is the terminal's to show: the conversation held reads.
  get chatShown() {
    return this.chatCapable && (this.outputView !== "raw" || (!this.live && !!this.entry?.chat));
  }
  // The page before the held part of the conversation, one ask at a time;
  // asked again once the ask in flight lands.
  older() {
    const entry = this.entry;
    const current = this.board?.current;
    if (!entry?.chat || entry.chat.floor == null || entry.chatOlder || !current?.agent || !this.live) return;
    if (this.connection.has("transcript")) {
      entry.chatWantsOlder = true;
      return;
    }
    entry.chatWantsOlder = false;
    const id = this.connection.request(
      { op: "transcript", ticket: current.id, session: current.agent.session, before: entry.chat.floor },
      entry.key,
    );
    if (id === undefined) return;
    entry.chatOlder = true;
    this.emit();
  }
  // `older`, from a render: after it.
  olderSoon() {
    queueMicrotask(() => this.older());
  }
  setOutputView(view) {
    if (this.outputView === view) return;
    this.outputView = view;
    try {
      localStorage.setItem("mesophon-output", view);
    } catch {
      /* The choice still holds for this page. */
    }
    this.sync();
    this.preview();
  }
  foreground() {
    const c = this.connection;
    if (c?.online && c.features?.includes("awareness") && !c.has("foreground"))
      c.request({ op: "foreground", ticket: this.visibleTicket() });
  }
  refresh() {
    if (!this.connection.has("snapshot")) this.connection.request({ op: "snapshot" });
  }
  receipts() {
    const c = this.connection;
    for (const session of this.sessions.entries.values()) {
      if (session.board !== this.active?.pin.board) continue;
      // The prompt's receipt and the answer's (T-568), each asked after
      // while no request about it is on its way.
      for (const receipt of [session.receipt, session.answer]) {
        if (!receipt?.unresolved) continue;
        if (receipt.incarnation !== c.incarnation) {
          this.sessions.reply(session, { result: "delivery", status: "unknown" }, receipt.id);
        } else if (
          ![...c.pending.entries()].some(
            ([id, p]) =>
              p.context === session.key &&
              receiptOps.includes(p.body.op) &&
              (id === receipt.id || p.body.command === receipt.id),
          )
        ) {
          c.request({ op: "status", command: receipt.id }, session.key);
        }
      }
    }
    // A ticket sent before a drop or a reload: ask what became of it, once.
    // A restarted host kept no receipt, so its answer is unknown.
    if (!c?.online) return;
    for (const item of this.sent.unresolved(this.active?.pin.board)) {
      if (item.incarnation !== c.incarnation)
        this.onSentReply(item.id, { result: "delivery", status: "unknown" });
      else if (![...c.pending.values()].some((p) => p.context === sentContext(item)))
        c.request({ op: "status", command: item.command }, sentContext(item));
    }
    // A note saved over the live channel before a drop or a reload (T-532).
    for (const item of this.noteMail.unresolved(this.active?.pin.board)) {
      if (item.incarnation !== c.incarnation)
        this.onNoteReply(item.id, { result: "delivery", status: "unknown" });
      else if (![...c.pending.values()].some((p) => p.context === noteContext(item)))
        c.request({ op: "status", command: item.command }, noteContext(item));
    }
    // A start is followed until its session runs: the host's receipt moves
    // on its own, so it is asked after on every tick until it settles.
    for (const item of this.starts.unresolved(this.active?.pin.board)) {
      if (item.incarnation !== c.incarnation)
        this.onStartReply(item.ticket, { result: "delivery", status: "unknown" });
      else if (![...c.pending.values()].some((p) => p.context === startContext(item.ticket)))
        c.request({ op: "status", command: item.command }, startContext(item.ticket));
    }
  }
  // Permission and dialog answers, bound to the exact ticket, session and request.
  sendInteraction(body, target) {
    const c = this.connection;
    if (!this.live || !c.online || !c.features?.includes(body.op) || !target || answerBusy(target))
      return;
    const id = c.request(body, target.key);
    if (id === undefined) return;
    this.sessions.sent(target, id, c.incarnation, body.op, "");
    this.sync();
  }

  // ---- starting an agent (T-498) --------------------------------------------
  // Whether this board's host starts agents from here, as it said when last
  // live: the button stays in place, disabled, while the terminal is away.
  get startsAgents() {
    return !!this.active?.starts;
  }
  get canStart() {
    return this.live && !!this.connection?.features?.includes("start");
  }
  // Whether the live host takes a tier with a start or a prompt (T-643):
  // an older one would drop the peer on the field.
  get picksTiers() {
    return this.live && !!this.connection?.features?.includes("tiers");
  }
  // The tiers this ticket may pick here, or none when there is no choice.
  tierChoices(ticket) {
    const ring = this.picksTiers && ticket ? this.board?.tierRing(ticket) || [] : [];
    return ring.length > 1 ? ring : [];
  }
  startOf(ticket) {
    return ticket && this.active ? this.starts.get(this.active.pin.board, ticket.id) : undefined;
  }
  // The seat a start reaches (T-510): empty, or a parked agent it wakes.
  startable(ticket) {
    return !!ticket && (!ticket.agent || ticket.agent.state === "sleeping");
  }
  // The words come first (T-510), as at the desk's Shift+Enter field: a
  // sheet asks for the first turn, and a blank one is the ticket's own
  // title and description, or a plain wake.
  openStart(id) {
    const ticket = this.board?.tickets.find((t) => t.id === id);
    if (!this.startable(ticket) || !this.canStart || startWaiting(this.startOf(ticket))) return;
    this.startAsk = { ticket: id, text: "", tier: ticket.tier || "" };
    this.emit();
  }
  closeStart() {
    if (!this.startAsk) return;
    this.startAsk = undefined;
    this.emit();
  }
  setStartText(text) {
    if (!this.startAsk) return;
    this.startAsk.text = text;
    this.emit();
  }
  setStartTier(tier) {
    if (!this.startAsk) return;
    this.startAsk.tier = tier;
    this.emit();
  }
  confirmStart() {
    const ask = this.startAsk;
    if (!ask) return;
    this.startAsk = undefined;
    this.startAgent(ask.ticket, ask.text, ask.tier);
  }
  // The host picks the provider from the ticket's tier; the words ride only
  // when there are any, so an older host, which knows no `prompt`, still
  // takes a blank start, and a tier only when it changes the ticket's.
  startAgent(id, text = "", tier = "") {
    const board = this.active?.pin.board;
    const ticket = this.board?.tickets.find((t) => t.id === id);
    if (!board || !this.startable(ticket) || !this.canStart || startWaiting(this.startOf(ticket))) return;
    const c = this.connection;
    const prompt = text.trim();
    const pick = tierPick(this, ticket, tier);
    const command = c.request(
      { op: "start", ticket: id, ...(prompt ? { prompt } : {}), ...(pick ? { tier: pick } : {}) },
      startContext(id),
    );
    if (command === undefined) {
      this.say("Not started: the connection dropped. Try again when your terminal is back.");
      this.emit();
      return;
    }
    this.starts.sent(board, id, command, c.incarnation, ticket.key);
    this.say(ticket.agent ? `Waking the agent on ${ticket.key}…` : `Starting an agent on ${ticket.key}…`, "clock");
    this.emit();
  }
  onStartReply(id, reply) {
    const item = this.starts.get(this.active?.pin.board, id);
    if (!item) return;
    const before = item.status;
    this.starts.reply(item, reply);
    if (item.status !== before) {
      if (item.status === "started") this.say(`An agent is working on ${item.key}`, "two");
      else if (item.status === "rejected") this.say(`Not started: ${item.message || "the terminal refused it"}`);
      else if (item.status === "unknown") this.say("Start unknown. Check the board before you start it again.");
      // The board shows the new agent; its output follows.
      this.refresh();
    }
    this.emit();
  }

  // Whether the live host answers a batch, or a question that takes several
  // choices, whole (T-571): an older one would drop the peer on `answers`.
  get answersWhole() {
    return this.live && !!this.connection?.features?.includes("dialog_multi");
  }

  // ---- card edits (T-530) ---------------------------------------------------
  // Whether the live host takes this edit: an older one would drop the peer
  // on an op it does not know, so the page offers none while it is away.
  canEdit(op) {
    return this.live && !!this.connection?.features?.includes(op);
  }
  editWaiting(ticket, op) {
    return !!ticket && !!this.active && this.edits.waiting(this.active.pin.board, ticket.id, op);
  }
  // Send one edit and wear it on the board until the host answers.
  edit(ticket, op, body, patch, extra = {}) {
    const board = this.active?.pin.board;
    if (!board || !ticket || !this.canEdit(op)) return false;
    const command = this.connection.request({ op, ticket: ticket.id, ...body }, "edit");
    if (command === undefined) {
      this.editError = "Not sent: the connection dropped. Try again when your terminal is back.";
      this.say(this.editError);
      this.emit();
      return false;
    }
    this.editError = "";
    const item = this.edits.sent(command, { board, ticket: ticket.id, key: ticket.key, op, patch, ...extra });
    this.board.tickets = this.edits.wear(board, this.board.tickets, [item]);
    this.sync();
    return true;
  }
  onEditReply(command, reply) {
    const item = this.edits.take(command);
    if (!item) return;
    if (reply.result === "rejected") {
      const what = { rename: "Not renamed", move: "Not moved", tag: "Tags not changed", workspace: "Workspace not changed" }[
        item.op
      ];
      this.editError = `${what}: ${reply.message || "the terminal refused it."}`;
      this.say(this.editError);
    } else if (reply.result === "edited" && item.moved) this.say(`Moved ${item.key} to ${item.patch.column}`, "two");
    // The board that follows is the truth, the refusal's included.
    this.refresh();
    this.emit();
  }
  // The title, written over the ticket page's heading.
  startRename() {
    const ticket = this.board?.current;
    if (!ticket || !this.canEdit("rename") || this.editWaiting(ticket, "rename")) return;
    this.renaming = { ticket: ticket.id, text: ticket.title };
    this.focus = "rename";
    this.emit();
  }
  setRename(text) {
    if (!this.renaming) return;
    this.renaming.text = text;
    this.emit();
  }
  cancelRename() {
    if (!this.renaming) return;
    this.renaming = undefined;
    this.focus = "selection";
    this.emit();
  }
  confirmRename() {
    const ask = this.renaming;
    const ticket = ask && this.board?.tickets.find((t) => t.id === ask.ticket);
    const title = ask?.text.replace(/\s+/g, " ").trim();
    if (!ticket || !title) return;
    this.renaming = undefined;
    this.focus = "selection";
    if (title === ticket.title || !this.edit(ticket, "rename", { title }, { title })) this.emit();
  }
  // To another column, or to a slot in one: before the card `before` names,
  // or at the column's end. A drop where the card already is sends nothing.
  moveTicket(id, column, before = null) {
    const ticket = this.board?.tickets.find((t) => t.id === id);
    if (!ticket || !this.board.columns.includes(column) || before === id) return;
    const moved = ticket.column !== column;
    if (!moved) {
      const others = this.board.tickets.filter((t) => t.column === column && t.id !== id);
      const slot = before ? others.findIndex((t) => t.id === before) : others.length;
      const now = this.board.tickets.filter((t) => t.column === column).findIndex((t) => t.id === id);
      if (slot < 0 || slot === now) return;
    }
    this.edit(ticket, "move", { column, ...(before ? { before } : {}) }, { column }, { before, moved });
  }
  // One tag per group, as on the board: another on the group replaces it,
  // the worn one comes off. Only the board's own tags are offered.
  toggleTicketTag(id, tag) {
    const ticket = this.board?.tickets.find((t) => t.id === id);
    if (!ticket) return;
    const worn = (ticket.tags || []).some((t) => t.group === tag.group && t.name === tag.name);
    const tags = (ticket.tags || []).filter((t) => t.group !== tag.group);
    if (!worn) tags.push({ group: tag.group, name: tag.name, tint: tag.tint });
    tags.sort((a, b) => a.group - b.group);
    this.edit(ticket, "tag", { group: tag.group, ...(worn ? {} : { name: tag.name }) }, { tags });
  }
  // Its own worktree or the shared checkout (T-642), while the choice is
  // open: the card wears the pick at once, a worktree as one not cut yet.
  setWorkspace(id, worktree) {
    const ticket = this.board?.tickets.find((t) => t.id === id);
    const ws = ticket?.workspace;
    if (!ws?.open || (ws.kind === "worktree") === worktree) return;
    const workspace = { ...ws, kind: worktree ? "worktree" : "shared", state: worktree ? "planned" : "" };
    this.edit(ticket, "workspace", { worktree }, { workspace });
  }
  // The sheet that moves a ticket, sets its tags and, while it is open,
  // its workspace, opened from its line.
  openCardSheet(id) {
    const ticket = this.board?.tickets.find((t) => t.id === id);
    const workspace = this.canEdit("workspace") && !!ticket?.workspace?.open;
    if (!ticket || !(this.canEdit("move") || this.canEdit("tag") || workspace)) return;
    this.cardSheet = id;
    this.editError = "";
    this.emit();
  }
  closeCardSheet() {
    if (!this.cardSheet) return;
    this.cardSheet = undefined;
    this.editError = "";
    this.focus = "card-line";
    this.emit();
  }

  // ---- new tickets -------------------------------------------------------
  // Whether a live host takes the create op now (phase 2's road).
  get canFile() {
    return this.live && !!this.connection?.features?.includes("create");
  }
  // Whether this board's host collects mail (T-497), as it said when last
  // live: then a ticket can be written and sealed at any time.
  get collects() {
    return !!this.active?.collects;
  }
  get canSend() {
    return this.collects || this.canFile;
  }
  // Whether a ticket's details can carry a picture now (T-670): it goes up
  // live, never through the mailbox, to a host that takes it before the
  // ticket exists.
  get canFilePictures() {
    return this.canFile && !!this.connection?.features?.includes("filed_pictures");
  }
  // The one draft, for the board on screen; another board starts afresh.
  draft() {
    const board = this.active?.pin.board;
    if (this.composer.board !== board) {
      dropThumbs(this.composer);
      this.composer = emptyDraft(board);
    }
    const draft = this.composer;
    if (this.board && !this.board.columns.includes(draft.column)) draft.column = this.board.landing();
    if (this.board)
      draft.tags = draft.tags.filter((t) =>
        this.board.allowedTags.some((a) => a.group === t.group && a.name === t.name),
      );
    return draft;
  }
  openComposer(column) {
    if (!this.board) return;
    const draft = this.draft();
    if (column && this.board.columns.includes(column)) draft.column = column;
    draft.error = "";
    draft.open = true;
    this.sheetOpen = false;
    this.emit();
  }
  closeComposer() {
    if (!this.composer.open) return;
    this.composer.open = false;
    this.emit();
  }
  setComposer(field, value) {
    if (this.composer.sending) return;
    this.draft()[field] = value;
    this.composer.error = "";
    this.emit();
  }
  // One tag per group, as on the board: picking another replaces it.
  toggleTag(tag) {
    const draft = this.draft();
    const worn = draft.tags.some((t) => t.group === tag.group && t.name === tag.name);
    draft.tags = draft.tags.filter((t) => t.group !== tag.group);
    if (!worn) draft.tags.push(tag);
    this.emit();
  }
  sendTicket() {
    const draft = this.draft();
    const board = this.active?.pin.board;
    const title = draft.title.trim();
    if (!title || !board || draft.sending || draft.reading) return;
    if (!this.canSend) {
      draft.error = this.live
        ? "This terminal’s mesimon is too old to take tickets from here. Update it, then send again."
        : "Your terminal is out of reach, and its mesimon does not keep tickets while it is away.";
      this.emit();
      return;
    }
    if (new TextEncoder().encode(draft.description).length > DESCRIPTION_MAX_BYTES) {
      draft.error = "Details must fit in 32 KiB.";
      this.emit();
      return;
    }
    const ticket = { title, description: draft.description, column: draft.column, tags: draft.tags.slice() };
    const held = new Map(draft.pictures.map((p) => [p.n, p]));
    const pictures = unlinked(ticket.description).filter((n) => held.has(n)).map((n) => held.get(n));
    if (pictures.length) {
      if (!this.canFilePictures) {
        draft.error = "Pictures need your terminal online.";
        this.emit();
        return;
      }
      draft.error = "";
      return this.sendPicturedTicket(board, draft, ticket, pictures);
    }
    if (this.collects) return this.sealTicket(board, draft, ticket);
    this.fileTicket(board, draft, ticket);
  }
  // The pictures the details name go up one by one, then the ticket that
  // links them (T-670), which the host files whole or not at all.
  async sendPicturedTicket(board, draft, ticket, pictures) {
    const current = () => this.composer === draft;
    const ids = await this.sendPictures(draft, pictures, undefined, current);
    if (!ids || !current()) return;
    const description = linked(ticket.description, ids);
    if (new TextEncoder().encode(description).length > DESCRIPTION_MAX_BYTES) {
      draft.error = "Details must fit in 32 KiB.";
      this.emit();
      return;
    }
    this.fileTicket(board, draft, ticket, description, [...ids.values()]);
  }
  // Sent live. Sent keeps the words as written, without the pictures'
  // links: the host lets a refused ticket's pictures go.
  fileTicket(board, draft, ticket, description = ticket.description, uploads = []) {
    const item = this.sent.add(board, ticket);
    const c = this.connection;
    const id = c.request(
      { op: "create", ...ticket, description, tags: ticket.tags.map(({ group, name }) => ({ group, name })),
        ...(uploads.length ? { uploads } : {}) },
      sentContext(item),
    );
    if (id === undefined) {
      // Nothing left this browser: the draft stays, nothing is listed.
      this.sent.remove(item.id);
      draft.error = "Not sent: the connection dropped. Send it again when the terminal is back.";
      this.emit();
      return;
    }
    this.sent.sent(item, id, c.incarnation);
    if (draft.replaces) this.sent.remove(draft.replaces);
    // The column stays for the next one; a run of tickets often shares it.
    dropThumbs(draft);
    Object.assign(draft, { open: false, title: "", description: "", tags: [], error: "", replaces: undefined, pictures: [] });
    this.persistSent(board);
    this.say("Sending to your board…", "clock");
    this.emit();
  }
  onSentReply(id, reply) {
    const item = this.sent.get(id);
    if (!item) return;
    const before = item.status;
    this.sent.reply(item, reply);
    if (item.status !== before) {
      this.persistSent(item.board);
      if (item.status === "landed") this.say(`Landed as ${item.key} in ${item.column}`, "two");
      else if (item.status === "rejected") this.say(`Not created: ${item.message || "the terminal refused it"}`);
      else this.say("Delivery unknown. Check the board before sending it again.");
    }
    if (item.status === "landed") this.refresh();
    this.emit();
  }
  // A ticket this browser filed was picked up at the desk (T-497): opened
  // there, or an agent started on it. Its ticks turn teal, once.
  notePickups(tickets) {
    const board = this.active?.pin.board;
    let last;
    for (const item of this.sent.forBoard(board)) {
      const picked = item.ticket && tickets.find((t) => t.id === item.ticket)?.picked;
      if (picked && this.sent.pickedUp(item, picked)) last = item;
    }
    if (!last) return;
    this.persistSent(board);
    this.say(
      last.picked.by === "agent" ? `An agent started on ${last.key}` : `${last.key} was opened at your desk`,
      "picked",
    );
  }
  // ---- the mailbox (T-497): tickets for a host that may be away -----------
  // Sealed here and now, so it can wait in this browser, then at the relay,
  // for as long as the host is away. Only the host can open it.
  sealTicket(board, draft, ticket) {
    let envelope;
    try {
      const body = { ...ticket, tags: ticket.tags.map(({ group, name }) => ({ group, name })), written_at: Date.now() };
      envelope = JSON.parse(this.crypto.mail(JSON.stringify(this.active.pin), JSON.stringify(body)));
    } catch {
      draft.error = "Could not seal the ticket in this browser.";
      this.emit();
      return;
    }
    if (JSON.stringify(envelope).length > MAIL_BYTES) {
      draft.error = "Too long to wait at the relay. Shorten the details, or send it while your terminal is live.";
      this.emit();
      return;
    }
    const item = this.sent.add(board, ticket, Date.now(), envelope);
    if (draft.replaces) this.sent.remove(draft.replaces);
    dropThumbs(draft);
    Object.assign(draft, { open: false, title: "", description: "", tags: [], error: "", replaces: undefined, pictures: [] });
    this.persistSent(board);
    if (!this.deposit(item))
      this.say(
        this.link === "nonet"
          ? "Saved in this browser. It goes out when you’re back online."
          : "Saved in this browser. It goes out when the relay answers.",
        "clock",
      );
    else if (this.live) this.say("Sending to your board…", "clock");
    this.emit();
  }
  // Hand a sealed ticket to the relay, once per socket.
  deposit(item) {
    if (item.status !== "local" || !item.envelope || this.depositing.has(item.id)) return false;
    if (!this.mailbox?.send({ kind: "deposit", board: this.active.pin.board, envelope: item.envelope })) return false;
    this.depositing.add(item.id);
    return true;
  }
  // The mailbox socket is up: ask what became of what is at the relay, and
  // hand over what waited in this browser.
  mailReady() {
    this.depositing.clear();
    const board = this.active?.pin.board;
    if (!board || !this.sent) return;
    const asked = [...this.sent.forBoard(board), ...this.noteMail.forBoard(board)]
      .filter((i) => i.status === "relay")
      .map((i) => i.id)
      .slice(-128);
    this.mailbox.send({ kind: "sync", board, ids: asked });
    for (const item of this.sent.waiting(board)) this.deposit(item);
    for (const item of this.noteMail.waiting(board)) this.deposit(item);
    this.peeks.again(board);
    this.peekShelf();
    this.emit();
  }
  // The terminal is out of reach: ask the relay for what it left this
  // browser (T-698), once a spell.
  peekShelf() {
    const board = this.active?.pin.board;
    if (!board || this.live || !this.down || this.kept || !this.mailbox?.ready) return;
    if (this.peeks.want(board)) this.mailbox.send({ kind: "peek", board });
  }
  // The shelf's items, opened with the host key this browser pinned: the
  // board first, since a conversation is laid on its agent.
  onShelf(items) {
    if (this.live || !this.active) return;
    const pin = JSON.stringify(this.active.pin);
    const opened = [];
    for (const item of Array.isArray(items) ? items : []) {
      try {
        const body = shelfItem(JSON.parse(this.crypto.shelf(pin, JSON.stringify(item))));
        if (body) opened.push(body);
      } catch {
        // Not the terminal's seal: not the terminal's words.
      }
    }
    opened.sort((a, b) => Number(b.kind === "board") - Number(a.kind === "board"));
    for (const item of opened) this.takeShelved(item);
    this.sync();
  }
  // Each lands where a live answer would, unless this page holds newer.
  takeShelved(item) {
    const board = this.active.pin.board;
    if (item.kind === "board") {
      if (this.board && item.at <= (this.board.receivedAt || 0)) return;
      if (!this.board) {
        this.board = new BoardState(this.active.selected);
        this.boards.set(board, this.board);
      }
      this.board.update(item.board, { cached: true, at: item.at, shelved: true });
      this.rememberBoard();
    } else if (item.kind === "notes") {
      const book = this.noteBook();
      if ((book.entry(item.ticket)?.at || 0) >= item.at) return;
      const stamp = this.board?.tickets.find((t) => t.id === item.ticket)?.noted;
      book.listed(item.ticket, item.notes, stamp, item.at, true);
      for (const body of item.bodies) book.read(item.ticket, body, item.at, true);
    } else if (item.kind === "transcript") {
      const ticket = this.board?.tickets.find((t) => t.id === item.ticket);
      if (ticket?.agent?.session !== item.session) return;
      const entry = this.sessions.get(board, ticket);
      if (entry.chat && (entry.chatAt || 0) >= item.at) return;
      entry.chat = mergePage(undefined, {}, item.page);
      entry.chatAt = item.at;
      entry.chatError = "";
    }
  }
  onMail(wire) {
    const note = wire.id && this.noteMail.get(wire.id);
    if (note) return this.onNoteMail(wire, note);
    const item = wire.id && this.sent.get(wire.id);
    if (wire.kind === "deposited" && item) {
      this.depositing.delete(item.id);
      const before = item.status;
      this.sent.deposited(item);
      if (before !== item.status) {
        this.persistSent(item.board);
        if (!this.live) this.say("Sent. It waits at the relay, sealed.", "one");
      }
    } else if (wire.kind === "refused" && item) {
      this.depositing.delete(item.id);
      // Transient: it stays in this browser and goes again with the socket.
      if (wire.code === "unavailable") return;
      const why = {
        denied: "this browser may no longer leave tickets for this board. Pair it again.",
        lapsed: "your terminal's access to the relay has lapsed. Enter a new license key in its Sharing dialog.",
        capacity: "too many tickets are waiting at the relay.",
        too_large: "it is too long to wait at the relay. Send it while your terminal is live.",
      }[wire.code] || "the relay did not keep it.";
      this.sent.reply(item, { result: "rejected", message: why });
      this.persistSent(item.board);
      this.say(`Not sent: ${why}`);
    } else if (wire.kind === "withdrawn" && item) {
      const editing = item.editing;
      item.editing = false;
      if (wire.removed) {
        this.sent.withdrawn(item);
        this.persistSent(item.board);
        if (editing) this.editWords(item, editing);
        else this.say("Unsent. The relay deleted it.");
      } else this.say("Too late to take back: your terminal has it.");
    } else if (wire.kind === "mailbox" && wire.board === this.active?.pin.board) {
      for (const state of wire.items || []) {
        const mine = this.sent.get(state.id) || this.noteMail.get(state.id);
        if (!mine) continue;
        if (state.stage === "answered" && state.receipt) this.onReceipt(mine, state.receipt);
        else if (state.stage === "gone" && this.noteMail.get(mine.id)) {
          this.noteMail.gone(mine);
          this.persistNotes(mine.board);
        } else if (state.stage === "gone") {
          this.sent.gone(mine);
          this.persistSent(mine.board);
        }
      }
    } else if (wire.kind === "shelf" && wire.board === this.active?.pin.board) {
      this.onShelf(wire.items);
    } else if (wire.kind === "receipt" && wire.board === this.active?.pin.board) {
      const mine = wire.receipt?.id && (this.sent.get(wire.receipt.id) || this.noteMail.get(wire.receipt.id));
      if (mine) this.onReceipt(mine, wire.receipt);
    }
    this.emit();
  }
  // Only the host's own seal counts: a receipt that does not open under the
  // pinned host key is not an answer, whoever sent it.
  onReceipt(item, receipt) {
    let body;
    try {
      body = JSON.parse(this.crypto.receipt(JSON.stringify(this.active.pin), JSON.stringify(receipt)));
    } catch {
      return;
    }
    if (this.noteMail.get(item.id)) this.onNoteReply(item.id, body);
    else this.onSentReply(item.id, body);
  }
  // Take a ticket back before the host has it: in this browser at once, at
  // the relay on its word.
  unsendSent(id) {
    const item = this.sent.get(id);
    if (!item) return;
    if (item.status === "local") {
      this.sent.withdrawn(item);
      this.persistSent(item.board);
      this.say("Unsent. It never left this browser.");
      this.emit();
    } else if (item.status === "relay") {
      if (!this.mailbox?.send({ kind: "withdraw", board: this.active.pin.board, id }))
        this.say("Unsending needs the relay. Try again when you’re back online.");
      this.emit();
    }
  }
  editWords(item, words = item) {
    const draft = this.draft();
    if (draft.sending) return;
    dropThumbs(draft);
    Object.assign(draft, {
      pictures: [],
      title: words.title,
      description: words.description,
      column: words.column,
      tags: words.tags.slice(),
      error: "",
      replaces: item.id,
    });
    this.openComposer(this.board?.columns.includes(words.column) ? words.column : undefined);
  }
  // Unknown or refused: back to the sheet, and the old entry goes once the
  // new one is sent. Still waiting: taken back first, then edited.
  editSent(id) {
    const item = this.sent.get(id);
    if (!item || ["landed", "sending", "withdrawn"].includes(item.status)) return;
    if (item.status === "local") {
      const words = { title: item.title, description: item.description, column: item.column, tags: item.tags };
      this.sent.withdrawn(item);
      this.persistSent(item.board);
      return this.editWords(item, words);
    }
    if (item.status === "relay") {
      const words = { title: item.title, description: item.description, column: item.column, tags: item.tags };
      if (this.mailbox?.send({ kind: "withdraw", board: this.active.pin.board, id })) item.editing = words;
      else this.say("Editing needs the relay. Try again when you’re back online.");
      this.emit();
      return;
    }
    this.editWords(item);
  }
  discardSent(id) {
    const item = this.sent.get(id);
    if (!item || ["sending", "local", "relay"].includes(item.status)) return;
    this.sent.remove(id);
    this.persistSent(item.board);
    this.emit();
  }
  // The tickets this browser filed, by id; one set per render.
  sentHere() {
    if (this.hereAt !== this.version) {
      this.hereAt = this.version;
      const board = this.active?.pin.board;
      this.here = new Set(this.sent.forBoard(board).filter((i) => i.ticket).map((i) => i.ticket));
    }
    return this.here;
  }
  // A toast says what became of a ticket; Sent shows it in the feed itself.
  // The caller emits.
  say(text, tick, action) {
    if (this.board?.mode === "sent" && !action) return;
    clearTimeout(this.toastTimer);
    const toast = (this.toast = { id: (this.toast?.id || 0) + 1, text, tick, action });
    this.toastTimer = setTimeout(() => {
      if (this.toast === toast) {
        this.toast = undefined;
        this.emit();
      }
    }, action ? 6000 : 2800);
  }
  // Swiped away (T-628): gone without its button's action.
  dismissToast() {
    clearTimeout(this.toastTimer);
    this.toast = undefined;
    this.emit();
  }
  // The toast's button, once.
  toastAction() {
    const action = this.toast?.action;
    this.toast = undefined;
    this.emit();
    action?.run();
  }

  // ---- notes (T-532) ---------------------------------------------------------
  // Whether this board's host has notes, as it said when last live.
  get notesHere() {
    return !!this.active?.notes;
  }
  // An edit goes through the mailbox when the host collects mail, live or
  // away, as a ticket does; else over the live channel, while it is live.
  get canWriteNotes() {
    return this.notesHere && (this.collects || (this.live && !!this.connection?.features?.includes("notes")));
  }
  // Whether a picture can go into a note now (T-629): it goes up over the
  // live channel, never through the mailbox, to a host that takes it.
  get canAddPictures() {
    return this.canWriteNotes && this.live && !!this.connection?.features?.includes("pictures");
  }
  // The notes of the ticket on screen, asked for again only when the board's
  // digest of them moved; then the body the page shows and does not hold.
  loadNotes() {
    const c = this.connection;
    const ticket = this.board?.current;
    if (!ticket || !this.live || this.board.cached || !c?.features?.includes("notes")) return;
    if (narrow() && !this.detailOpen) return;
    const book = this.noteBook();
    const stamp = ticket.noted || "";
    if (!ticket.notes) {
      if (!book.current(ticket.id, "")) book.listed(ticket.id, { notes: [] }, "");
    } else if (!book.current(ticket.id, stamp) && !c.has("notes"))
      c.request({ op: "notes", ticket: ticket.id }, `notes:${ticket.id}:${stamp}`);
    const rows = book.entry(ticket.id)?.rows || [];
    const wanted = this.reading?.ticket === ticket.id ? this.reading.note : rows[0]?.id;
    if (wanted && rows.some((r) => r.id === wanted) && book.body(ticket.id, wanted) === undefined && !c.has("note"))
      c.request({ op: "note", ticket: ticket.id, note: wanted }, `note:${ticket.id}`);
  }
  onNotes(context, reply) {
    const [, ticket, stamp = ""] = context.split(":");
    if (reply.result === "notes") {
      this.noteBook().listed(ticket, reply, stamp);
      this.persistNotes();
      this.loadNotes();
    }
    this.emit();
  }
  onNote(context, reply) {
    const ticket = context.slice("note:".length);
    if (reply.result === "note") {
      this.noteBook().read(ticket, reply);
      this.persistNotes();
    } else if (reply.result === "rejected") {
      // Gone since the list was read: read the list again.
      const entry = this.noteBook().entry(ticket);
      if (entry) entry.stamp = undefined;
    }
    this.emit();
  }
  // Every note of a ticket, in a sheet (T-627): the ticket page shows only
  // the latest once it has more than two.
  openNotesSheet(ticket) {
    if (!this.board?.tickets.some((t) => t.id === ticket)) return;
    this.notesSheet = ticket;
    this.loadNotes();
    this.emit();
  }
  closeNotesSheet() {
    if (!this.notesSheet) return;
    this.notesSheet = undefined;
    this.focus = "all-notes";
    this.emit();
  }
  // A note opened, or one written, from the sheet closes it first.
  openNote(ticket, note) {
    this.notesSheet = undefined;
    this.reading = { ticket, note };
    if (narrow() && !history.state?.note) history.pushState({ detail: true, note: true }, "");
    this.focus = "note";
    this.loadNotes();
    this.emit();
  }
  // Closed at once: `history.back` lands later, and a second close before it
  // would pop the ticket too.
  closeNote() {
    if (!this.reading) return;
    this.reading = undefined;
    if (history.state?.note) history.back();
    this.emit();
  }
  // Prev and Next walk the ticket's notes in order, the description first,
  // and come round again, as the desk's Tab does in its note editor.
  walkNote(step) {
    const rows = this.noteBook().entry(this.reading?.ticket)?.rows || [];
    const at = rows.findIndex((r) => r.id === this.reading?.note);
    if (at < 0 || rows.length < 2) return;
    this.reading = { ticket: this.reading.ticket, note: rows[(at + step + rows.length) % rows.length].id };
    this.loadNotes();
    this.emit();
  }
  // The edit sheet, on the words the note has, or on an edit of this
  // browser's that did not land. One still on its way is taken back first.
  editNote(ticket, note) {
    const board = this.active?.pin.board;
    const t = this.board?.tickets.find((x) => x.id === ticket);
    if (!board || !t || !this.canWriteNotes) return;
    this.notesSheet = undefined;
    const pending = note ? this.noteMail.pending(board, ticket, note) : undefined;
    if (pending && ["local", "relay"].includes(pending.status)) return this.retractNote(pending.id, true);
    if (pending?.status === "sending") return;
    const book = this.noteBook();
    const rows = book.entry(ticket)?.rows || [];
    const row = rows.find((r) => r.id === note);
    let text = "";
    if (pending) text = pending.text;
    else if (note) {
      text = book.body(ticket, note);
      if (text === undefined) return;
    }
    this.noteDraft = {
      board,
      ticket,
      key: t.key,
      note,
      rev: pending?.stale?.rev ?? row?.rev ?? pending?.rev,
      text,
      error: "",
      confirmDelete: false,
      description: !!note && rows[0]?.id === note,
      replaces: pending?.id,
      pictures: [],
      reading: 0,
      sending: "",
    };
    this.emit();
  }
  closeNoteSheet() {
    if (!this.noteDraft) return;
    dropThumbs(this.noteDraft);
    this.noteDraft = undefined;
    this.emit();
  }
  setNoteText(text) {
    if (!this.noteDraft || this.noteDraft.sending) return;
    Object.assign(this.noteDraft, { text, error: "", confirmDelete: false });
    this.emit();
  }
  // Pictures picked or pasted into the sheet (T-629), each named
  // `[Image #N]` at `at` in the words (the end without one), as the
  // desk's editor names one. Answers where the next one would go.
  async addPictures(files, at) {
    const draft = this.noteDraft;
    if (!draft || !this.canAddPictures) return;
    return this.readPictures(draft, "text", files, at, () => this.noteDraft === draft);
  }
  // The same for a new ticket's details (T-670).
  async addTicketPictures(files, at) {
    const draft = this.draft();
    if (!this.canFilePictures) return;
    return this.readPictures(draft, "description", files, at, () => this.composer === draft);
  }
  // Each file read into `draft`, named in its `field` while `current()`.
  async readPictures(draft, field, files, at, current) {
    if (draft.sending) return;
    for (const file of files) {
      draft.reading += 1;
      this.emit();
      let made;
      try {
        made = await picture(file);
      } catch (e) {
        draft.error = e.message;
      }
      draft.reading -= 1;
      if (!current() || draft.sending) {
        made?.thumb.close?.();
        return undefined;
      }
      if (made) {
        const n = nextNumber(draft[field], draft.pictures.map((p) => p.n));
        const placed = insertToken(draft[field], at, n);
        Object.assign(draft, { [field]: placed.text, error: "", confirmDelete: false });
        at = placed.at;
        draft.pictures.push({ n, ...made });
      }
      this.emit();
    }
    return at;
  }
  removePicture(n) {
    this.dropPicture(this.noteDraft, "text", n);
  }
  removeTicketPicture(n) {
    this.dropPicture(this.composer, "description", n);
  }
  dropPicture(draft, field, n) {
    if (!draft || draft.sending) return;
    const at = draft.pictures.findIndex((p) => p.n === n);
    if (at < 0) return;
    draft.pictures[at].thumb?.close?.();
    draft.pictures.splice(at, 1);
    draft[field] = withoutPicture(draft[field], n);
    this.emit();
  }
  // One request answered as a promise, for an upload's pieces in turn. A
  // dropped connection answers nothing, so it rejects the one waiting.
  ask(body) {
    return new Promise((resolve, reject) => {
      const id = this.connection?.request(body, (reply) => {
        this.asking = undefined;
        resolve(reply);
      });
      if (id === undefined) reject(new Error("the connection dropped"));
      else this.asking = reject;
    });
  }
  // One picture's pieces in order, for a note on `ticket`, or with none
  // for a ticket not filed yet (T-670).
  async uploadPicture(bytes, ticket, current) {
    let upload;
    let offset = 0;
    while (offset < bytes.length) {
      if (!current()) throw new Error("cancelled");
      const { data, end } = pieceOf(bytes, offset);
      const reply = await this.ask({ op: "upload", ...(ticket ? { ticket } : {}), ...(upload ? { upload } : {}),
        offset, data, complete: end >= bytes.length });
      if (reply.result !== "uploaded") throw new Error(reply.message || "the terminal refused it");
      upload = reply.upload;
      offset = end;
    }
    return upload;
  }
  // `pictures` up one by one, saying so on `draft`: their host ids by
  // number, or nothing once one failed, which the draft says.
  async sendPictures(draft, pictures, ticket, current) {
    const ids = new Map();
    try {
      for (const [i, p] of pictures.entries()) {
        draft.sending = pictures.length > 1 ? `Sending picture ${i + 1} of ${pictures.length}…` : "Sending picture…";
        this.emit();
        ids.set(p.n, await this.uploadPicture(p.bytes, ticket, current));
      }
    } catch (e) {
      draft.sending = "";
      if (current()) {
        draft.error = `Picture not sent: ${e.message}.`;
        this.emit();
      }
      return undefined;
    }
    draft.sending = "";
    return ids;
  }
  // The pictures the words name go up one by one, then the note that
  // links them, in one write the host keeps whole or not at all.
  async savePictured(draft, pictures) {
    const words = draft.text;
    const current = () => this.noteDraft === draft;
    const ids = await this.sendPictures(draft, pictures, draft.ticket, current);
    if (!ids || !current()) return;
    const text = linked(words, ids);
    if (new TextEncoder().encode(text).length > NOTE_MAX_BYTES) {
      draft.error = "Notes must fit in 32 KiB.";
      return this.emit();
    }
    this.finishSave(draft, text, [...ids.values()]);
  }
  // Delete asks once more, in place.
  deleteNote() {
    const draft = this.noteDraft;
    if (!draft?.note || draft.description) return;
    if (!draft.confirmDelete) {
      draft.confirmDelete = true;
      this.emit();
      return;
    }
    this.saveNote(true);
  }
  saveNote(deleting = false) {
    const draft = this.noteDraft;
    if (!draft || draft.sending || draft.reading) return;
    const text = deleting ? "" : draft.text;
    const fail = (error) => {
      draft.error = error;
      this.emit();
    };
    if (!deleting && !text.trim()) return fail(draft.note ? "Empty. Delete the note instead." : "Nothing to save.");
    if (new TextEncoder().encode(text).length > NOTE_MAX_BYTES) return fail("Notes must fit in 32 KiB.");
    if (!this.canWriteNotes) return fail("Saving needs your terminal.");
    const held = new Map(draft.pictures.map((p) => [p.n, p]));
    const pictures = deleting ? [] : unlinked(text).filter((n) => held.has(n)).map((n) => held.get(n));
    if (pictures.length) {
      if (!this.canAddPictures) return fail("Pictures need your terminal online.");
      draft.error = "";
      return this.savePictured(draft, pictures);
    }
    this.finishSave(draft, text);
  }
  finishSave(draft, text, uploads = []) {
    const deleting = !text;
    const error = this.sendNote(draft.board, {
      ticket: draft.ticket,
      key: draft.key,
      note: draft.note,
      name: nameOf(text),
      rev: draft.rev,
      text,
      uploads,
    });
    if (error) {
      draft.error = error;
      return this.emit();
    }
    dropThumbs(draft);
    if (draft.replaces) this.noteMail.remove(draft.replaces);
    this.noteDraft = undefined;
    if (deleting && this.reading?.note === draft.note) this.closeNote();
    this.persistNotes(draft.board);
    this.emit();
  }
  // Sealed for the mailbox, or over the live channel: an error, or nothing.
  sendNote(board, words) {
    const uploads = words.uploads || [];
    if (uploads.length && !this.canAddPictures) return "Pictures need your terminal online.";
    if (this.collects && !uploads.length) {
      let envelope;
      try {
        const body = { kind: "note", ticket: words.ticket, note: words.note, text: words.text, rev: words.rev,
          written_at: Date.now() };
        envelope = JSON.parse(this.crypto.mail(JSON.stringify(this.active.pin), JSON.stringify(body)));
      } catch {
        return "Could not seal the note in this browser.";
      }
      if (JSON.stringify(envelope).length > MAIL_BYTES) return "Too long to wait at the relay.";
      const item = this.noteMail.add(board, words, Date.now(), envelope);
      if (this.deposit(item)) this.say("Saving…", "clock");
      else this.say("Saved in this browser. It goes out when you’re back online.", "clock");
      return "";
    }
    const c = this.connection;
    const item = this.noteMail.add(board, words);
    const id = c.request(
      { op: "write_note", ticket: words.ticket, note: words.note, text: words.text, rev: words.rev,
        ...(uploads.length ? { uploads } : {}) },
      noteContext(item),
    );
    if (id === undefined) {
      this.noteMail.remove(item.id);
      return "Not saved: the connection dropped.";
    }
    this.noteMail.sent(item, id, c.incarnation);
    this.say("Saving…", "clock");
    return "";
  }
  onNoteReply(id, reply) {
    const item = this.noteMail.get(id);
    if (!item) return;
    const before = item.status;
    const deleted = item.note;
    const status = this.noteMail.reply(item, reply);
    if (status === before) return this.emit();
    const book = this.noteBook(item.board);
    if (status === "landed") {
      this.noteMail.remove(item.id);
      book.written(item.ticket, deleted, reply, item.text, item.name);
      if (!reply.note && this.reading?.note === deleted) this.closeNote();
      const agent = this.board?.tickets.find((t) => t.id === item.ticket)?.agent;
      const awake = this.live && agent?.promptable && agent.state !== "sleeping";
      if (reply.note && awake)
        this.say("Saved", "two", { label: `Tell ${agent.provider}`, run: () => this.tellAgent(item.ticket, reply.note) });
      else this.say(reply.note ? "Saved" : "Deleted", "two");
      this.refresh();
    } else if (status === "stale") {
      const entry = book.entry(item.ticket);
      const at = entry?.rows.findIndex((r) => r.id === item.stale.id) ?? -1;
      if (at >= 0) entry.rows[at] = item.stale;
      this.say(`Not saved: ${item.stale.by} changed it`);
      this.loadNotes();
    } else if (status === "rejected") this.say(`Not saved: ${item.message || "the terminal refused it"}`);
    else if (status === "unknown") this.say("Save unknown. Check the note before saving again.");
    this.persistNotes(item.board);
    this.emit();
  }
  // A stale edit (T-532): keep the words the note has now, or save these
  // over them. A refused or unknown one is let go the same way.
  dropNote(id) {
    const item = this.noteMail.get(id);
    if (!item || ["sending", "local", "relay"].includes(item.status)) return;
    this.noteMail.remove(id);
    this.persistNotes(item.board);
    this.loadNotes();
    this.emit();
  }
  saveMine(id) {
    const item = this.noteMail.get(id);
    if (item?.status !== "stale" || !this.canWriteNotes) return;
    const { ticket, key, note, name, text, uploads } = item;
    const error = this.sendNote(item.board, { ticket, key, note, name, text, uploads, rev: item.stale.rev });
    if (error) this.say(error);
    else this.noteMail.remove(id);
    this.persistNotes(item.board);
    this.emit();
  }
  // Take back an edit still on its way: here at once, at the relay on its
  // word. With `edit`, its words go back to the sheet.
  retractNote(id, edit = false) {
    const item = this.noteMail.get(id);
    if (!item) return;
    if (item.status === "local") {
      this.noteMail.remove(id);
      this.persistNotes(item.board);
      if (edit) this.reopenNote(item);
      else this.say("Unsent.");
      this.emit();
    } else if (item.status === "relay") {
      if (this.mailbox?.send({ kind: "withdraw", board: this.active.pin.board, id })) item.editing = edit;
      else this.say("Taking it back needs the relay.");
      this.emit();
    }
  }
  reopenNote(item) {
    const rows = this.noteBook(item.board).entry(item.ticket)?.rows || [];
    this.noteDraft = {
      board: item.board,
      ticket: item.ticket,
      key: item.key,
      note: item.note,
      rev: item.rev,
      text: item.text,
      error: "",
      confirmDelete: false,
      description: !!item.note && rows[0]?.id === item.note,
      replaces: undefined,
    };
  }
  onNoteMail(wire, item) {
    if (wire.kind === "deposited") {
      this.depositing.delete(item.id);
      const before = item.status;
      this.noteMail.deposited(item);
      if (before !== item.status) {
        this.persistNotes(item.board);
        if (!this.live) this.say("Waits at the relay for your terminal.", "one");
      }
    } else if (wire.kind === "refused") {
      this.depositing.delete(item.id);
      if (wire.code === "unavailable") return;
      this.noteMail.reply(item, { result: "rejected", message: "the relay did not keep it" });
      this.persistNotes(item.board);
      this.say("Not sent: the relay did not keep it.");
    } else if (wire.kind === "withdrawn") {
      const edit = item.editing;
      item.editing = false;
      if (wire.removed) {
        this.noteMail.remove(item.id);
        this.persistNotes(item.board);
        if (edit) this.reopenNote(item);
        else this.say("Unsent.");
      } else this.say("Too late: your terminal has it.");
    }
    this.emit();
  }
  tellAgent(ticket, note) {
    const c = this.connection;
    if (!this.live || c?.request({ op: "tell_agent", ticket, note }, `tell:${ticket}`) === undefined)
      this.say("Not sent: your terminal is out of reach.");
    this.emit();
  }
  onTold(reply) {
    if (reply.result === "delivery") this.say("Told", "two");
    else if (reply.result === "rejected") this.say(`Not told: ${reply.message}`);
    this.emit();
  }

  // ---- pairing and boards ------------------------------------------------
  showPairing() {
    if (this.active && !this.active.revoked) this.returnBoard = this.active;
    this.screen = "pair";
    this.sheetOpen = this.settingsOpen = false;
    this.boardMenuOpen = false;
    this.focus = "code";
    this.emit();
  }
  get canReturn() {
    return !!this.returnBoard && !this.returnBoard.revoked;
  }
  returnToBoard() {
    if (this.returnBoard) this.openBoard(this.returnBoard);
  }
  // A pairing QR opened this page (T-497): the code is filled in and the
  // link forgotten, and pairing still waits for Connect. A link can come
  // from anyone; the person's tap is what pairs.
  pairFromLink(text) {
    const code = linkCode(text);
    history.replaceState(history.state, "", location.pathname + location.search);
    if (!code) return;
    this.pairCode = code;
    this.showPairing();
    this.status = this.kept
      ? "Code filled in. Pairing needs a connection: Connect when you’re back online."
      : "Code filled in from your terminal’s QR code. Connect to pair.";
    this.focus = "pair";
    this.emit();
  }
  pair() {
    const code = this.pairCode.trim();
    if (!code || !this.connection) return;
    if (this.kept) {
      this.status = "Pairing needs a connection. Try again when you’re back online.";
      this.emit();
      return;
    }
    this.identity.name = this.deviceName.trim() || "My browser";
    this.active = this.board = this.entry = undefined;
    this.live = false;
    clearAlerts();
    this.pairReady = false;
    this.emit();
    this.connection.connect(undefined, code);
  }
  switchBoard(id) {
    const chosen = this.identity.boards.find((b) => b.pin.board === id);
    if (chosen) this.openBoard(chosen);
  }
  openBoard(chosen) {
    this.live = false;
    this.down = false;
    this.active = chosen;
    this.board = this.boards.get(chosen.pin.board);
    this.entry = undefined;
    this.outputKey = undefined;
    this.sheetOpen = this.settingsOpen = false;
    this.boardMenuOpen = false;
    this.composer.open = false;
    this.startAsk = undefined;
    this.renaming = this.cardSheet = this.notesSheet = undefined;
    clearAlerts();
    this.identity.lastBoard = chosen.pin.board;
    this.persist();
    if (chosen.revoked) {
      this.connection.stop();
      this.showPairing();
      this.status = "Access revoked. Pair again from the host to restore access.";
      this.emit();
      return;
    }
    this.screen = "shell";
    this.detail(!!chosen.selected);
    if (!this.board) this.restoreBoard(chosen);
    this.restoreSent(chosen.pin.board);
    this.restoreNotes(chosen.pin.board);
    this.reading = undefined;
    this.noteDraft = undefined;
    this.sync();
    if (this.kept) return;
    this.connection.connect(chosen);
    this.mailbox?.want(true);
    if (this.mailbox?.ready) this.mailReady();
  }
  async forget() {
    this.connection.stop();
    this.sessions.entries.clear();
    this.sessions.targets.clear();
    this.boards.clear();
    this.forgetting = undefined;
    this.remembered.clear();
    this.sent = new Sent();
    this.sentLoaded.clear();
    this.starts = new Starts();
    this.edits = new Edits();
    this.renaming = this.cardSheet = this.notesSheet = undefined;
    this.noteBooks.clear();
    this.noteMail = new NoteMail();
    this.notesLoaded.clear();
    this.reading = this.noteDraft = undefined;
    this.composer = emptyDraft(undefined);
    this.active = this.board = this.entry = this.returnBoard = undefined;
    clearAlerts();
    this.mailbox?.want(false);
    const crypto = new this.Browser();
    this.identity = { seed: crypto.seed(), boards: [] };
    crypto.free();
    this.connection.identity = this.identity;
    this.crypto?.free?.();
    this.crypto = new this.Browser(this.identity.seed);
    if (this.mailbox) Object.assign(this.mailbox, { crypto: this.crypto, identity: this.identity });
    try {
      await Promise.all([this.save(), this.storage.dropBoards()]);
      this.status = "Browser forgotten. Pair again to connect.";
    } catch {
      this.status = "Could not forget the saved identity. Clear this site's browser storage.";
    }
    this.showPairing();
    this.pairReady = true;
    this.emit();
  }

  // ---- connection callbacks ---------------------------------------------
  async onState(state, message) {
    this.phase = state;
    if (state === "reconnecting") {
      this.down = true;
      this.peekShelf();
    }
    this.status = state === "revoked" ? "Removing access…" : message;
    let revocationSaved;
    if (state === "revoked" || state === "unverified") {
      if (this.active) {
        this.sessions.purge(this.active.pin.board);
        this.starts.purge(this.active.pin.board);
        this.edits.purge(this.active.pin.board);
        this.boards.delete(this.active.pin.board);
        this.forgetRemembered(this.active.pin.board);
        if (this.composer.board === this.active.pin.board) this.composer = emptyDraft(undefined);
        if (state === "revoked") {
          this.active.revoked = true;
          revocationSaved = this.save();
        }
      }
      this.board = this.entry = undefined;
      this.mailbox?.want(false);
      clearAlerts();
      this.showPairing();
    }
    if (["unpaired", "revoked", "unverified"].includes(state)) this.pairReady = true;
    this.sync();
    if (revocationSaved) {
      try {
        await revocationSaved;
        this.status = message;
      } catch {
        this.status = `${message} Could not save the access-removed marker.`;
      }
      this.emit();
    }
  }
  onReady(chosen) {
    this.active = chosen;
    this.returnBoard = undefined;
    // What the host said about itself, kept for when it is away (T-497).
    const collects = !!this.connection.features?.includes("mailbox");
    const starts = !!this.connection.features?.includes("start");
    const notes = !!this.connection.features?.includes("notes");
    if (!!chosen.collects !== collects || !!chosen.starts !== starts || !!chosen.notes !== notes) {
      chosen.collects = collects;
      chosen.starts = starts;
      chosen.notes = notes;
      this.persist();
    }
    this.board = this.boards.get(chosen.pin.board);
    this.pairCode = "";
    this.pairReady = true;
    this.screen = "shell";
    if (!this.board) this.restoreBoard(chosen);
    this.restoreSent(chosen.pin.board);
    this.restoreNotes(chosen.pin.board);
    this.sync();
    this.refresh();
    this.receipts();
    this.mailbox?.want(true);
    if (this.mailbox?.ready) this.mailReady();
  }
  onLost() {
    this.live = false;
    this.asking?.(new Error("the connection dropped"));
    this.asking = undefined;
    this.sessions.lost();
    // Whatever took, the next board says; nothing is sent again.
    this.edits.lost();
    this.sync();
  }
  onReply(reply, original, id) {
    if (original?.body.op === "foreground") return;
    if (typeof original?.context === "function") return original.context(reply);
    if (typeof original?.context === "string" && original.context.startsWith("sent:")) {
      this.onSentReply(original.context.slice("sent:".length), reply);
      return;
    }
    if (typeof original?.context === "string" && original.context.startsWith("start:")) {
      this.onStartReply(original.context.slice("start:".length), reply);
      return;
    }
    if (original?.context === "edit") {
      this.onEditReply(id, reply);
      return;
    }
    const context = typeof original?.context === "string" ? original.context : "";
    if (context.startsWith("notes:")) return this.onNotes(context, reply);
    if (context.startsWith("note:")) return this.onNote(context, reply);
    if (context.startsWith("notew:")) return this.onNoteReply(context.slice("notew:".length), reply);
    if (context.startsWith("tell:")) return this.onTold(reply);
    if (reply.result === "awareness") {
      const originBoard = this.active?.pin.board;
      showAlert(reply, this.visibleTicket(), (ticket) => this.navigateTicket(originBoard, ticket));
      this.refresh();
      return;
    }
    if (reply.result === "changed") {
      this.refresh();
      return;
    }
    if (reply.result === "board") {
      if (!this.board) {
        this.board = new BoardState(this.active.selected);
        this.boards.set(this.active.pin.board, this.board);
      }
      this.board.update({ ...reply, tickets: this.edits.wear(this.active.pin.board, reply.tickets) });
      this.notePickups(reply.tickets);
      if (this.active.title !== reply.title || this.active.selected !== this.board.selected) {
        this.active.title = reply.title;
        this.active.selected = this.board.selected;
        this.persist();
      }
      this.live = true;
      this.down = false;
      this.peeks.again(this.active.pin.board);
      this.phase = "live";
      this.status = "Connected";
      this.rememberBoard();
      this.sync();
      this.preview();
      this.foreground();
      this.loadNotes();
    } else if (reply.result === "transcript" && original?.body.op === "transcript") {
      const entry = this.sessions.entries.get(original.context);
      if (entry) {
        const before = entry.chat;
        entry.chat = mergePage(before, original.body, reply);
        entry.chatAt = Date.now();
        entry.chatError = "";
        if (landed(entry)) entry.receipt.landed = true;
        if (original.body.before != null) entry.chatOlder = false;
        else if (!entry.chatFollowing && before && entry.chat.rows.length !== before.rows.length)
          entry.chatUnread = true;
      }
      this.sync();
      if (entry?.chatWantsOlder) this.older();
    } else if (reply.result === "preview" && original?.body.op === "preview") {
      const session = this.sessions.entries.get(original.context);
      if (session) {
        session.output = reply.lines.join("\n");
        session.cols = Number.isInteger(reply.cols) && reply.cols > 0 ? reply.cols : undefined;
        session.receivedAt = Date.now();
        if (session.following) session.displayed = session.output;
        else session.unread = session.output !== session.displayed;
      }
      this.sync();
    } else if (["delivery", "rejected", "taken_back"].includes(reply.result)) {
      const session = this.sessions.entries.get(original?.context);
      const command = ["prompt", "send_now", "take_back", "permission", "dialog"].includes(original?.body.op)
        ? id
        : original?.body.op === "status"
          ? original.body.command
          : undefined;
      if (command !== undefined && [session?.receipt, session?.answer].some((r) => r?.id === command))
        this.sessions.reply(session, reply, command);
      else if (reply.result === "rejected" && original?.body.op === "transcript" && session) {
        session.chatOlder = false;
        if (!session.chat) session.chatError = `Conversation unavailable: ${reply.message}`;
      } else if (reply.result === "rejected" && original?.body.op === "preview" && session) {
        session.displayed = `Preview unavailable: ${reply.message}`;
        // A rejected preview can indicate session replacement; refresh identity.
        this.refresh();
      }
      this.sync();
      if (["delivery", "taken_back"].includes(reply.result) || ["send_now", "take_back"].includes(original?.body.op))
        this.refresh();
    }
  }

  // ---- reading the output --------------------------------------------------
  outputScrolled(node) {
    const entry = this.entry;
    if (!entry || !node.getClientRects().length) return;
    const wasFollowing = entry.following;
    const wasUnread = entry.unread;
    entry.scroll = node.scrollTop;
    entry.following = node.scrollHeight - node.clientHeight - entry.scroll < 24;
    // A paused 50-line window stays frozen until the reader explicitly follows.
    if (entry.following && entry.unread) {
      entry.displayed = entry.output;
      entry.unread = false;
    }
    if (wasFollowing !== entry.following || wasUnread !== entry.unread) this.emit();
  }
  chatScrolled(node) {
    const entry = this.entry;
    if (!entry || !node.getClientRects().length) return;
    const was = [entry.chatFollowing, entry.chatUnread];
    entry.chatScroll = node.scrollTop;
    entry.chatFollowing = node.scrollHeight - node.clientHeight - node.scrollTop < 24;
    if (entry.chatFollowing) entry.chatUnread = false;
    if (node.scrollTop < 240) this.older();
    if (was[0] !== entry.chatFollowing || was[1] !== entry.chatUnread) this.emit();
  }
  latest() {
    const entry = this.entry;
    if (!entry) return;
    if (this.chatShown) {
      entry.chatFollowing = true;
      entry.chatUnread = false;
      this.sync();
      return;
    }
    entry.following = true;
    entry.unread = false;
    entry.displayed = entry.output;
    this.sync();
  }
  setRail(rail) {
    this.rail = rail;
    if (rail) this.boardMenuOpen = false;
    try {
      localStorage.setItem("mesophon-sidebar", rail ? "rail" : "full");
    } catch {
      /* The fold still holds for this page. */
    }
    this.emit();
  }

  // ---- composer ------------------------------------------------------------
  setDraft(text) {
    if (!this.entry) return;
    this.entry.draft = text;
    this.emit();
  }
  reviewDraft() {
    if (!this.entry) return;
    this.entry.review = false;
    this.focus = "prompt";
    this.sync();
  }
  setDelivery(mode) {
    if (!this.entry) return;
    this.entry.mode = mode;
    this.sync();
  }
  // The composer's tier pick (T-643), the desk ask field's `^n`: it rides
  // the next prompt, and a pick the agent did not launch on restarts it on
  // that tier when its turn ends, with the words.
  setPromptTier(tier) {
    if (!this.entry) return;
    this.entry.tier = tier;
    this.sync();
  }
  // The tier the composer shows: its pick, else the ticket's own.
  promptTier(ticket) {
    return this.entry?.tier || ticket?.tier || "";
  }
  // Whether the next prompt restarts the agent on another tier.
  switchesTier(ticket) {
    const pick = this.promptTier(ticket);
    const agent = ticket?.agent;
    return !!pick && !!agent?.tier && this.tierChoices(ticket).length > 0 && pick !== agent.tier;
  }
  queueAction(op) {
    const entry = this.entry;
    const current = this.board?.current;
    if (!this.live || !current?.agent?.promptable || current.queued == null ||
      (entry.receipt?.waiting && entry.receipt.status !== "queued") ||
      (op === "send_now" && sendRefused(current)))
      return;
    const id = this.connection.request({ op, ticket: entry.ticket, session: entry.session }, entry.key);
    if (id !== undefined) this.sessions.sent(entry, id, this.connection.incarnation, op, current.queued);
    else entry.delivery = "Delivery unknown. Check the agent before trying again.";
    this.sync();
  }
  swapReturned() {
    const entry = this.entry;
    if (!entry?.returned) return;
    const returned = entry.returned;
    entry.returned = entry.draft ? { text: entry.draft, session: entry.session } : undefined;
    entry.draft = returned.text;
    entry.review ||= returned.session !== entry.session;
    this.focus = "prompt";
    this.sync();
  }
  submitPrompt() {
    const entry = this.entry;
    const agent = this.board?.current?.agent;
    if (!this.live || !agent?.promptable || !entry?.draft.trim() ||
      entry.review || entry.receipt?.waiting || entry.answer?.waiting)
      return;
    if (new TextEncoder().encode(entry.draft).length > 4096) {
      entry.delivery = "Prompt must fit in 4096 UTF-8 bytes.";
      this.sync();
      return;
    }
    const ticket = this.board.current;
    const pick = tierPick(this, ticket, entry.tier);
    const id = this.connection.request(
      // Steer is off while the agent waits on you (T-568), and while a tier
      // switch waits for the turn's end (T-643): the words queue.
      { op: "prompt", ticket: entry.ticket, session: entry.session, text: entry.draft,
        queued: entry.mode === "queue" || waitsOnYou(agent) || this.switchesTier(ticket),
        ...(pick ? { tier: pick } : {}) },
      entry.key,
    );
    if (id !== undefined) {
      this.sessions.sent(entry, id, this.connection.incarnation);
      // The pick is the ticket's now; the next board says so.
      entry.tier = undefined;
    } else entry.delivery = "Delivery unknown. Check the agent before sending again.";
    this.sync();
  }

  // ---- preferences -----------------------------------------------------------
  setTheme(theme) {
    this.theme = theme;
    document.documentElement.dataset.theme = theme;
    try {
      localStorage.setItem("mesophon-theme", theme);
    } catch {
      /* In-memory choice still applies. */
    }
    this.emit();
  }
  // The browser offered to install the page, or installed it.
  installable(prompt, installed = false) {
    this.install.prompt = prompt;
    if (installed) this.install.standalone = true;
    this.emit();
  }
  async installApp() {
    const prompt = this.install.prompt;
    if (!prompt) return;
    this.install.prompt = undefined;
    this.emit();
    try {
      await prompt.prompt();
    } catch {
      /* The browser declined to show it; nothing to undo. */
    }
  }
  async enableAlerts() {
    if (typeof Notification === "undefined") {
      this.alertStatus = "Updates appear here while connected. System notifications are unavailable in this browser.";
      this.emit();
      return;
    }
    const permission = await Notification.requestPermission();
    this.alertStatus = permission === "granted"
      ? "Alerts enabled while this browser stays connected."
      : "Updates appear here while connected. System notifications are disabled in browser settings.";
    this.emit();
  }

  // ---- lifecycle -------------------------------------------------------------
  visibility() {
    this.foreground();
    if (!document.hidden && this.connection) {
      this.live = false;
      // A retry loop already says what it is doing (T-639).
      if (this.phase !== "reconnecting") {
        this.phase = "checking";
        this.status = "Checking connection… Last received view is stale.";
      }
      this.sync();
      this.connection.tick();
      this.refresh();
      this.preview();
    }
  }
  tick() {
    if (!this.connection) return;
    this.connection.tick();
    this.foreground();
    if (!document.hidden && this.connection.online) {
      this.refresh();
      this.preview();
      this.receipts();
      this.loadNotes();
    }
  }
  setOnline(online) {
    this.online = online;
    if (online) this.mailbox?.poke();
    this.emit();
  }
  // A page from the kept copy asks the network (past the service worker)
  // whether the relay's page can be had, and becomes it when it can: never
  // while a ticket is being written, whose words live only in this page.
  watchForRelay() {
    const probe = async () => {
      if (this.composer.open || this.noteDraft) return;
      try {
        const response = await fetch("./manifest.webmanifest", { cache: "no-store" });
        if (response.ok && !this.composer.open && !this.noteDraft) location.reload();
      } catch {
        /* Still out of reach. */
      }
    };
    setInterval(probe, PROBE_MS);
    addEventListener("online", probe);
  }
  async boot(storage, identity) {
    this.storage = storage;
    this.identity = identity;
    // Keys for sealing tickets and opening receipts, and the mailbox socket.
    this.crypto = new this.Browser(identity.seed);
    this.mailbox = new Mailbox({
      crypto: this.crypto,
      identity,
      onReady: () => this.mailReady(),
      onFrame: (wire) => this.onMail(wire),
      onDown: () => {
        this.depositing.clear();
        this.emit();
      },
    });
    this.connection = new Connection({
      Browser: this.Browser,
      identity,
      save: () => this.save(),
      onState: (state, message) => this.onState(state, message),
      onReply: (reply, original, id) => this.onReply(reply, original, id),
      onLost: () => this.onLost(),
      onReady: (chosen) => this.onReady(chosen),
    });
    this.pairReady = true;
    if (this.kept) {
      this.status = "Opened without the relay: the board as this browser remembers it. It reconnects by itself.";
      this.watchForRelay();
    }
    const link = new URLSearchParams(location.hash.slice(1));
    if (link.has("pair")) {
      const remembered = identity.boards.find((b) => b.pin.board === identity.lastBoard && !b.revoked);
      if (remembered) this.returnBoard = remembered;
      return this.pairFromLink(link.get("pair"));
    }
    const linked = identity.boards.find((b) => b.pin.board === link.get("board") && !b.revoked);
    if (linked && link.get("ticket")) linked.selected = link.get("ticket");
    const remembered =
      linked || identity.boards.find((b) => b.pin.board === identity.lastBoard) || identity.boards[0];
    if (remembered) {
      this.openBoard(remembered);
      if (linked) {
        this.detail(true);
        this.sync();
      }
    } else {
      this.status = "Enable Remote Control on the host, then pair with its code.";
      this.focus = "code";
      this.emit();
    }
  }
}
