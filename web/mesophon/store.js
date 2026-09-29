// Application state and every action on it. The view reads this object and
// re-renders on `emit`; nothing here touches the DOM except history, focus
// requests and the theme attribute. Transport stays in connection.js.
import { Connection } from "./connection.js";
import { BoardState } from "./board.js";
import { Sessions } from "./sessions.js";
import { Sent } from "./sent.js";
import { Mailbox } from "./mailbox.js";
import { showAlert, clearAlerts } from "./awareness.js";

const narrow = () => matchMedia("(max-width: 700px)").matches;
const receiptOps = ["prompt", "send_now", "take_back", "permission", "dialog", "status"];
// A filed ticket's description is its first note: the host's note limit.
export const DESCRIPTION_MAX_BYTES = 32 * 1024;
// The longest envelope the relay keeps (control::MAIL_BYTES).
const MAIL_BYTES = 128 * 1024;
const sentContext = (item) => `sent:${item.id}`;
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
});

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
    this.theme = "system";
    this.alertStatus = "Alerts need this browser to stay connected.";
    this.wrap = true;
    this.focus = null;
    this.outputKey = undefined;
    this.remembered = new Map(); // board id -> signature of the stored snapshot
    this.sent = new Sent();
    this.sentLoaded = new Set(); // boards whose stored Sent list is read
    this.composer = emptyDraft(undefined);
    this.toast = undefined;
    this.depositing = new Set(); // ids handed to the mailbox socket, unanswered
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
    if (!id || !this.board || this.board.cached) return;
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
    this.sent.purge(board);
    this.sentLoaded.delete(board);
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

  // ---- navigation --------------------------------------------------------
  detail(open, push = false) {
    if (open && !this.detailOpen) this.outputKey = undefined;
    this.detailOpen = open;
    if (!open) this.focus = "row";
    if (push && narrow() && !history.state?.detail) history.pushState({ detail: true }, "");
  }
  select(id) {
    if (!this.board) return;
    this.board.selected = id;
    this.active.selected = id;
    this.persist();
    this.detail(true, true);
    this.focus = "selection";
    this.sync();
    this.preview();
    this.foreground();
  }
  back() {
    if (history.state?.detail) history.back();
    else this.detail(false);
    this.focus = "row";
    this.sync();
  }
  popstate(state) {
    this.detail(!!state?.detail);
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
  openSheet(open) {
    this.sheetOpen = open;
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
  preview() {
    const current = this.board?.current;
    if (!document.hidden && current?.agent && this.entry && !this.connection.has("preview"))
      this.connection.request(
        { op: "preview", ticket: current.id, session: current.agent.session },
        this.entry.key,
      );
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
      const receipt = session.receipt;
      if (session.board !== this.active?.pin.board || !receipt?.unresolved) continue;
      if (receipt.incarnation !== c.incarnation) {
        this.sessions.reply(session, { result: "delivery", status: "unknown" });
      } else if (
        ![...c.pending.values()].some(
          (p) => p.context === session.key && receiptOps.includes(p.body.op),
        )
      ) {
        c.request({ op: "status", command: receipt.id }, session.key);
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
  }
  // Permission and dialog answers, bound to the exact ticket, session and request.
  sendInteraction(body, target) {
    const c = this.connection;
    if (!this.live || !c.online || !c.features?.includes(body.op) || !target || target.receipt?.waiting)
      return;
    const id = c.request(body, target.key);
    if (id === undefined) return;
    this.sessions.sent(target, id, c.incarnation, body.op, "");
    this.sync();
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
  // The one draft, for the board on screen; another board starts afresh.
  draft() {
    const board = this.active?.pin.board;
    if (this.composer.board !== board) this.composer = emptyDraft(board);
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
    if (!title || !board) return;
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
    if (this.collects) return this.sealTicket(board, draft, ticket);
    const item = this.sent.add(board, ticket);
    const c = this.connection;
    const id = c.request(
      { op: "create", ...ticket, tags: ticket.tags.map(({ group, name }) => ({ group, name })) },
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
    Object.assign(draft, { open: false, title: "", description: "", tags: [], error: "", replaces: undefined });
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
    Object.assign(draft, { open: false, title: "", description: "", tags: [], error: "", replaces: undefined });
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
    const asked = this.sent.forBoard(board).filter((i) => i.status === "relay").map((i) => i.id).slice(-128);
    this.mailbox.send({ kind: "sync", board, ids: asked });
    for (const item of this.sent.waiting(board)) this.deposit(item);
    this.emit();
  }
  onMail(wire) {
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
        const mine = this.sent.get(state.id);
        if (!mine) continue;
        if (state.stage === "answered" && state.receipt) this.onReceipt(mine, state.receipt);
        else if (state.stage === "gone") {
          this.sent.gone(mine);
          this.persistSent(mine.board);
        }
      }
    } else if (wire.kind === "receipt" && wire.board === this.active?.pin.board) {
      const mine = wire.receipt?.id && this.sent.get(wire.receipt.id);
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
    this.onSentReply(item.id, body);
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
    Object.assign(draft, {
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
  openSent(id) {
    const item = this.sent.get(id);
    if (item?.ticket && this.board?.tickets.some((t) => t.id === item.ticket)) this.select(item.ticket);
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
  say(text, tick) {
    if (this.board?.mode === "sent") return;
    clearTimeout(this.toastTimer);
    const toast = (this.toast = { id: (this.toast?.id || 0) + 1, text, tick });
    this.toastTimer = setTimeout(() => {
      if (this.toast === toast) {
        this.toast = undefined;
        this.emit();
      }
    }, 2800);
  }

  // ---- pairing and boards ------------------------------------------------
  showPairing() {
    if (this.active && !this.active.revoked) this.returnBoard = this.active;
    this.screen = "pair";
    this.sheetOpen = false;
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
    this.sheetOpen = false;
    this.composer.open = false;
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
    this.remembered.clear();
    this.sent = new Sent();
    this.sentLoaded.clear();
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
    if (state === "offline") this.down = true;
    this.status = state === "revoked" ? "Removing access…" : message;
    let revocationSaved;
    if (state === "revoked" || state === "unverified") {
      if (this.active) {
        this.sessions.purge(this.active.pin.board);
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
    if (!!chosen.collects !== collects) {
      chosen.collects = collects;
      this.persist();
    }
    this.board = this.boards.get(chosen.pin.board);
    this.pairCode = "";
    this.pairReady = true;
    this.screen = "shell";
    if (!this.board) this.restoreBoard(chosen);
    this.restoreSent(chosen.pin.board);
    this.sync();
    this.refresh();
    this.receipts();
    this.mailbox?.want(true);
    if (this.mailbox?.ready) this.mailReady();
  }
  onLost() {
    this.live = false;
    this.sessions.lost();
    this.sync();
  }
  onReply(reply, original, id) {
    if (original?.body.op === "foreground") return;
    if (typeof original?.context === "string" && original.context.startsWith("sent:")) {
      this.onSentReply(original.context.slice("sent:".length), reply);
      return;
    }
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
      this.board.update(reply);
      this.notePickups(reply.tickets);
      if (this.active.title !== reply.title || this.active.selected !== this.board.selected) {
        this.active.title = reply.title;
        this.active.selected = this.board.selected;
        this.persist();
      }
      this.live = true;
      this.down = false;
      this.phase = "live";
      this.status = "Connected";
      this.rememberBoard();
      this.sync();
      this.preview();
      this.foreground();
    } else if (reply.result === "preview" && original?.body.op === "preview") {
      const session = this.sessions.entries.get(original.context);
      if (session) {
        session.output = reply.lines.join("\n");
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
      if (session?.receipt?.id === command && command !== undefined) this.sessions.reply(session, reply);
      else if (reply.result === "rejected" && original?.body.op === "preview" && session) {
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
  latest() {
    const entry = this.entry;
    if (!entry) return;
    entry.following = true;
    entry.unread = false;
    entry.displayed = entry.output;
    this.sync();
  }
  setWrap(wrap) {
    this.wrap = wrap;
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
  queueAction(op) {
    const entry = this.entry;
    const current = this.board?.current;
    if (!this.live || !current?.agent?.promptable || current.queued == null ||
      (entry.receipt?.waiting && entry.receipt.status !== "queued"))
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
    if (!this.live || !this.board?.current?.agent?.promptable || !entry?.draft.trim() ||
      entry.review || entry.receipt?.waiting)
      return;
    if (new TextEncoder().encode(entry.draft).length > 4096) {
      entry.delivery = "Prompt must fit in 4096 UTF-8 bytes.";
      this.sync();
      return;
    }
    const id = this.connection.request(
      { op: "prompt", ticket: entry.ticket, session: entry.session, text: entry.draft, queued: entry.mode === "queue" },
      entry.key,
    );
    if (id !== undefined) this.sessions.sent(entry, id, this.connection.incarnation);
    else entry.delivery = "Delivery unknown. Check the agent before sending again.";
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
      this.phase = "checking";
      this.status = "Checking connection… Last received view is stale.";
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
      if (this.composer.open) return;
      try {
        const response = await fetch("./manifest.webmanifest", { cache: "no-store" });
        if (response.ok && !this.composer.open) location.reload();
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
