// Application state and every action on it. The view reads this object and
// re-renders on `emit`; nothing here touches the DOM except history, focus
// requests and the theme attribute. Transport stays in connection.js.
import { Connection } from "./connection.js";
import { BoardState } from "./board.js";
import { Sessions } from "./sessions.js";
import { showAlert, clearAlerts } from "./awareness.js";

const narrow = () => matchMedia("(max-width: 700px)").matches;
const receiptOps = ["prompt", "send_now", "take_back", "permission", "dialog", "status"];

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
    return this.storage?.dropBoards(board).catch(() => {});
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
  pair() {
    const code = this.pairCode.trim();
    if (!code || !this.connection) return;
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
    this.sync();
    this.connection.connect(chosen);
  }
  async forget() {
    this.connection.stop();
    this.sessions.entries.clear();
    this.sessions.targets.clear();
    this.boards.clear();
    this.remembered.clear();
    this.active = this.board = this.entry = this.returnBoard = undefined;
    clearAlerts();
    const crypto = new this.Browser();
    this.identity = { seed: crypto.seed(), boards: [] };
    crypto.free();
    this.connection.identity = this.identity;
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
        if (state === "revoked") {
          this.active.revoked = true;
          revocationSaved = this.save();
        }
      }
      this.board = this.entry = undefined;
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
    this.board = this.boards.get(chosen.pin.board);
    this.pairCode = "";
    this.pairReady = true;
    this.screen = "shell";
    if (!this.board) this.restoreBoard(chosen);
    this.sync();
    this.refresh();
    this.receipts();
  }
  onLost() {
    this.live = false;
    this.sessions.lost();
    this.sync();
  }
  onReply(reply, original, id) {
    if (original?.body.op === "foreground") return;
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
    this.emit();
  }
  async boot(storage, identity) {
    this.storage = storage;
    this.identity = identity;
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
    const link = new URLSearchParams(location.hash.slice(1));
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
