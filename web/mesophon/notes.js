// A ticket's notes (T-532): what this browser read of them, and the edits it
// sent. A note read here stays readable while the terminal is away, marked
// with when it was read; revoke and Forget this browser drop it with the
// remembered board. An edit goes as a ticket does: sealed for the relay's
// mailbox (a clock, then one tick) when the host collects mail, else over
// the live channel; the host's answer settles it.

// The host's note limit (`board::NOTE_MAX_BYTES`).
export const NOTE_MAX_BYTES = 32 * 1024;
// How many tickets' notes a board keeps for reading while away; the ones
// read longest ago go first.
export const KEEP_TICKETS = 40;
const statuses = ["sending", "local", "relay", "stale", "rejected", "unknown"];
// On their way: the note shows the words and a tick.
const waitingStatuses = ["sending", "local", "relay"];

const newId = () =>
  globalThis.crypto?.randomUUID?.() ?? `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
const isRow = (r) =>
  r && typeof r.id === "string" && typeof r.name === "string" && Number.isInteger(r.rev);

// The notes of one board's tickets, as this browser last read them.
export class NoteBook {
  constructor() {
    this.tickets = new Map();
  }
  entry(ticket) {
    return this.tickets.get(ticket);
  }
  #ensure(ticket) {
    let entry = this.tickets.get(ticket);
    if (!entry) this.tickets.set(ticket, (entry = { stamp: undefined, rows: [], bodies: {}, at: 0 }));
    return entry;
  }
  // The host's list of a ticket's notes, `stamp` the board's digest of them
  // when asked. Bodies of notes no longer listed, or since rewritten, go.
  // `shelved` when the list is the relay's copy (T-698): read, never kept
  // past the page, which asks the relay again.
  listed(ticket, reply, stamp, at = Date.now(), shelved = false) {
    const entry = this.#ensure(ticket);
    entry.rows = (reply.notes || []).filter(isRow);
    entry.stamp = stamp;
    entry.at = at;
    entry.shelved = shelved;
    const kept = {};
    for (const row of entry.rows) {
      const body = entry.bodies[row.id];
      if (body && body.rev === row.rev) kept[row.id] = body;
    }
    entry.bodies = kept;
    const first = entry.rows[0];
    if (first && typeof reply.description === "string") entry.bodies[first.id] = { rev: first.rev, text: reply.description };
  }
  // One note's body, and its row as it stands now. Read live, the entry is
  // this browser's to keep, whatever the shelf gave before.
  read(ticket, reply, at = Date.now(), shelved = false) {
    if (!isRow(reply.note) || typeof reply.text !== "string") return;
    const entry = this.#ensure(ticket);
    entry.at = at;
    if (!shelved) entry.shelved = false;
    const index = entry.rows.findIndex((r) => r.id === reply.note.id);
    if (index >= 0) entry.rows[index] = reply.note;
    entry.bodies[reply.note.id] = { rev: reply.note.rev, text: reply.text };
  }
  // A write this browser made to note `id` landed: its words are the
  // note's now, or the note is gone. A new note's row comes with the list.
  written(ticket, id, reply, text, name) {
    const entry = this.tickets.get(ticket);
    if (!entry) return;
    if (!reply.note) {
      entry.rows = entry.rows.filter((r) => r.id !== id);
      delete entry.bodies[id];
      return;
    }
    const row = entry.rows.find((r) => r.id === reply.note);
    if (row) Object.assign(row, { rev: reply.rev, at: Date.now(), name: name || row.name });
    entry.bodies[reply.note] = { rev: reply.rev, text };
  }
  // The note's body when this browser holds its current revision.
  body(ticket, id) {
    const entry = this.tickets.get(ticket);
    const row = entry?.rows.find((r) => r.id === id);
    const body = row && entry.bodies[row.id];
    return body && body.rev === row.rev ? body.text : undefined;
  }
  // Whether the list matches the board's digest of it.
  current(ticket, stamp) {
    const entry = this.tickets.get(ticket);
    return !!entry && entry.stamp === (stamp || "");
  }
  // What survives a reload: the newest KEEP_TICKETS tickets' rows and the
  // bodies read of them.
  stored() {
    return [...this.tickets.entries()]
      .filter(([, e]) => !e.shelved)
      .sort((a, b) => b[1].at - a[1].at)
      .slice(0, KEEP_TICKETS)
      .map(([ticket, e]) => ({ ticket, stamp: e.stamp, rows: e.rows, bodies: e.bodies, at: e.at }));
  }
  restore(stored) {
    if (!Array.isArray(stored)) return;
    for (const s of stored) {
      if (!s || typeof s.ticket !== "string" || this.tickets.has(s.ticket) || !Array.isArray(s.rows)) continue;
      const rows = s.rows.filter(isRow);
      const bodies = {};
      for (const row of rows) {
        const body = s.bodies?.[row.id];
        if (body && body.rev === row.rev && typeof body.text === "string") bodies[row.id] = body;
      }
      this.tickets.set(s.ticket, {
        stamp: typeof s.stamp === "string" ? s.stamp : undefined,
        rows,
        bodies,
        at: Number.isFinite(s.at) ? s.at : 0,
      });
    }
  }
}

// Edits this browser sent, every board's: one item per save, until the host
// answers. A landed one leaves; a refused, stale or unknown one stays, with
// its words, until the reader saves again or lets it go.
export class NoteMail {
  constructor() {
    this.items = [];
  }
  get(id) {
    return this.items.find((i) => i.id === id);
  }
  forBoard(board) {
    return this.items.filter((i) => i.board === board).sort((a, b) => a.at - b.at);
  }
  // `envelope` sealed for the mailbox, or none for the live op. `uploads`
  // are the pictures the host holds for it (T-629), kept in memory only:
  // the host lets them go ten minutes after, so a reload has none to name.
  add(board, { ticket, key = "", note, name = "", rev, text, uploads = [] }, at = Date.now(), envelope = undefined) {
    const item = {
      uploads,
      id: envelope?.id ?? newId(),
      board,
      ticket,
      key,
      note,
      name,
      rev,
      text,
      at,
      status: envelope ? "local" : "sending",
      envelope,
      command: undefined,
      incarnation: undefined,
      message: "",
      stale: undefined,
    };
    this.items.push(item);
    return item;
  }
  sent(item, command, incarnation) {
    item.command = command;
    item.incarnation = incarnation;
  }
  deposited(item) {
    if (item.status !== "local") return;
    item.status = "relay";
    item.envelope = undefined;
  }
  gone(item) {
    if (item.status === "relay") item.status = "unknown";
  }
  // The host's answer, to the live op, a status query or a sealed receipt.
  // Returns "landed" when the words are the note's now.
  reply(item, reply) {
    if (!waitingStatuses.includes(item.status)) return item.status;
    item.envelope = undefined;
    if (reply.result === "note_written") {
      item.status = "landed";
      // A delete lands with no id: the item keeps the one it deleted.
      item.note = reply.note ?? item.note;
      item.rev = reply.rev;
    } else if (reply.result === "note_stale" && reply.note) {
      item.status = "stale";
      item.stale = reply.note;
    } else if (reply.result === "rejected") {
      item.status = "rejected";
      item.message = reply.message || "";
    } else if (reply.result === "delivery" && reply.status === "unknown") item.status = "unknown";
    return item.status;
  }
  // The edit a note is waiting on, or that did not land: the newest one.
  pending(board, ticket, note) {
    return this.forBoard(board)
      .filter((i) => i.ticket === ticket && i.note === note && statuses.includes(i.status))
      .at(-1);
  }
  // New notes on their way to a ticket.
  fresh(board, ticket) {
    return this.forBoard(board).filter((i) => i.ticket === ticket && !i.note && statuses.includes(i.status));
  }
  waiting(board) {
    return this.forBoard(board).filter((i) => ["local", "relay"].includes(i.status));
  }
  unresolved(board) {
    return this.items.filter((i) => i.board === board && i.status === "sending" && i.command !== undefined);
  }
  remove(id) {
    this.items = this.items.filter((i) => i.id !== id);
  }
  purge(board) {
    this.items = this.items.filter((i) => i.board !== board);
  }
  stored(board) {
    return this.forBoard(board)
      .filter((i) => statuses.includes(i.status))
      .map(({ id, ticket, key, note, name, rev, text, at, status, envelope, command, incarnation, message, stale }) => ({
        id,
        ticket,
        key,
        note,
        name,
        rev,
        text,
        at,
        status,
        envelope: status === "local" ? envelope : undefined,
        command,
        incarnation,
        message,
        stale,
      }));
  }
  restore(board, stored) {
    if (!Array.isArray(stored)) return;
    for (const s of stored) {
      if (!s || typeof s.id !== "string" || typeof s.ticket !== "string" || typeof s.text !== "string") continue;
      if (!statuses.includes(s.status) || this.get(s.id)) continue;
      const envelope = s.envelope && typeof s.envelope.id === "string" ? s.envelope : undefined;
      // A local edit whose sealed copy was lost cannot go out any more.
      const status = s.status === "local" && !envelope ? "unknown" : s.status;
      this.items.push({
        id: s.id,
        board,
        ticket: s.ticket,
        key: typeof s.key === "string" ? s.key : "",
        note: typeof s.note === "string" ? s.note : undefined,
        name: typeof s.name === "string" ? s.name : "",
        rev: Number.isInteger(s.rev) ? s.rev : undefined,
        text: s.text,
        at: Number.isFinite(s.at) ? s.at : 0,
        status,
        envelope: status === "local" ? envelope : undefined,
        command: Number.isInteger(s.command) ? s.command : undefined,
        incarnation: typeof s.incarnation === "string" ? s.incarnation : undefined,
        message: typeof s.message === "string" ? s.message : "",
        stale: isRow(s.stale) ? s.stale : undefined,
      });
    }
  }
}

// A note's name, as the host derives it: the first line that says something.
export const nameOf = (text) =>
  (text.split("\n").map((l) => l.replace(/^#+\s*/, "").trim()).find(Boolean) || "").slice(0, 120);

// How long ago, in a card's few characters.
export function ago(at, now = Date.now()) {
  if (!at) return "";
  const minutes = Math.floor((now - at) / 60000);
  if (minutes < 1) return "now";
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  return hours < 24 ? `${hours}h` : `${Math.floor(hours / 24)}d`;
}
