// Tickets this browser sent, per board: what was asked and what became of
// it. Through the relay's mailbox (T-497) a ticket is `local` (a clock:
// sealed in this browser), then `relay` (one tick: kept for the host), then
// `landed` (two ticks: the host's own receipt), and a landed one is
// `picked` up once it was opened at the desk or an agent started on it
// (teal ticks). A live host without the mailbox answers the create op
// instead, `sending` until it does. Kept beside the remembered board; a
// landed ticket keeps its title and key, never its description.
export const KEEP = 50;
const statuses = ["sending", "local", "relay", "landed", "unknown", "rejected", "withdrawn"];
// Waiting for the host: shown on Now and as ghosts on the board.
const waitingStatuses = ["local", "relay"];
// Settled for good: kept to a bound, without their words.
const settled = ["landed", "withdrawn"];

const newId = () =>
  globalThis.crypto?.randomUUID?.() ?? `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;

export class Sent {
  constructor() {
    this.items = [];
  }
  forBoard(board) {
    return this.items.filter((i) => i.board === board).sort((a, b) => a.at - b.at);
  }
  get(id) {
    return this.items.find((i) => i.id === id);
  }
  // `envelope` sealed for the mailbox, or none for the live create op.
  add(board, { title, description = "", column, tags = [] }, at = Date.now(), envelope = undefined) {
    const item = {
      id: envelope?.id ?? newId(),
      board,
      title,
      description,
      column,
      tags,
      at,
      status: envelope ? "local" : "sending",
      envelope,
      command: undefined,
      incarnation: undefined,
      key: "",
      ticket: "",
      message: "",
      picked: undefined,
    };
    this.items.push(item);
    return item;
  }
  sent(item, command, incarnation) {
    item.command = command;
    item.incarnation = incarnation;
  }
  // The relay kept it: the one tick. The sealed copy is no longer needed.
  deposited(item) {
    if (item.status !== "local") return;
    item.status = "relay";
    item.envelope = undefined;
  }
  // Taken back before the host had it: a line in the record, no words.
  withdrawn(item) {
    Object.assign(item, { status: "withdrawn", envelope: undefined, description: "" });
  }
  // The relay no longer holds a ticket it kept: nobody can say what became of it.
  gone(item) {
    if (item.status === "relay") item.status = "unknown";
  }
  // The host's answer: to the create op, a status query or a sealed receipt.
  reply(item, reply) {
    if (!["sending", "local", "relay"].includes(item.status)) return;
    if (reply.result === "created") {
      // The description stays in this page's memory; `stored` drops it.
      Object.assign(item, {
        status: "landed",
        key: reply.key,
        ticket: reply.ticket,
        column: reply.column || item.column,
        envelope: undefined,
      });
    } else if (reply.result === "rejected") {
      item.status = "rejected";
      item.message = reply.message || "";
      item.envelope = undefined;
    } else if (reply.result === "delivery" && reply.status === "unknown") {
      item.status = "unknown";
    }
  }
  // Picked up at the desk, as the host's board says: once, and only a
  // ticket that landed. Returns whether it changed.
  pickedUp(item, picked) {
    const known = picked && typeof picked.by === "string" && Number.isFinite(picked.at);
    if (item.status !== "landed" || item.picked || !known) return false;
    item.picked = { by: picked.by, at: picked.at };
    return true;
  }
  // Sent over the live channel, not yet answered: a reconnect asks.
  unresolved(board) {
    return this.items.filter((i) => i.board === board && i.status === "sending" && i.command !== undefined);
  }
  // Not on the board yet, but on its way: this browser or the relay has it.
  waiting(board) {
    return this.forBoard(board).filter((i) => waitingStatuses.includes(i.status));
  }
  // What needs the reader: still on its way, or did not land.
  unsettled(board) {
    return this.items.filter((i) => i.board === board && !settled.includes(i.status)).length;
  }
  remove(id) {
    this.items = this.items.filter((i) => i.id !== id);
  }
  purge(board) {
    this.items = this.items.filter((i) => i.board !== board);
  }
  // What survives a reload: every unsettled item whole (a local one with its
  // sealed envelope, to deposit later), and the newest KEEP settled ones
  // without their words.
  stored(board) {
    const mine = this.forBoard(board);
    const kept = mine.filter((i) => settled.includes(i.status)).slice(-KEEP);
    return mine
      .filter((i) => !settled.includes(i.status) || kept.includes(i))
      .map(({ id, title, description, column, tags, at, status, envelope, command, incarnation, key, ticket, message, picked }) => ({
        id,
        title,
        description: settled.includes(status) ? "" : description,
        column,
        tags: tags.map(({ group, name, tint }) => ({ group, name, tint })),
        at,
        status,
        envelope: status === "local" ? envelope : undefined,
        command,
        incarnation,
        key,
        ticket,
        message,
        picked,
      }));
  }
  // Put back what the last page kept, beside anything sent since it loaded.
  restore(board, stored) {
    if (!Array.isArray(stored)) return;
    for (const s of stored) {
      if (!s || typeof s.id !== "string" || typeof s.title !== "string" || !statuses.includes(s.status)) continue;
      if (this.get(s.id)) continue;
      const envelope = s.envelope && typeof s.envelope.id === "string" ? s.envelope : undefined;
      // A local ticket whose sealed copy was lost cannot go out any more.
      const status = s.status === "local" && !envelope ? "unknown" : s.status;
      this.items.push({
        id: s.id,
        board,
        title: s.title,
        description: typeof s.description === "string" ? s.description : "",
        column: typeof s.column === "string" ? s.column : "",
        tags: Array.isArray(s.tags)
          ? s.tags.filter((t) => t && typeof t.name === "string" && Number.isInteger(t.group))
          : [],
        at: Number.isFinite(s.at) ? s.at : 0,
        status,
        envelope: status === "local" ? envelope : undefined,
        command: Number.isInteger(s.command) ? s.command : undefined,
        incarnation: typeof s.incarnation === "string" ? s.incarnation : undefined,
        key: typeof s.key === "string" ? s.key : "",
        ticket: typeof s.ticket === "string" ? s.ticket : "",
        message: typeof s.message === "string" ? s.message : "",
        picked:
          status === "landed" && typeof s.picked?.by === "string" && Number.isFinite(s.picked?.at)
            ? { by: s.picked.by, at: s.picked.at }
            : undefined,
      });
    }
  }
}
