// Tickets this browser sent, per board: what was asked and what the host
// answered. A clock while the answer is out, two ticks once the ticket is on
// the board. Kept beside the remembered board; a landed ticket keeps its
// title and key, never its description.
export const KEEP = 50;
const statuses = ["sending", "landed", "unknown", "rejected"];

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
  add(board, { title, description = "", column, tags = [] }, at = Date.now()) {
    const item = {
      id: newId(),
      board,
      title,
      description,
      column,
      tags,
      at,
      status: "sending",
      command: undefined,
      incarnation: undefined,
      key: "",
      ticket: "",
      message: "",
    };
    this.items.push(item);
    return item;
  }
  sent(item, command, incarnation) {
    item.command = command;
    item.incarnation = incarnation;
  }
  // The host's answer to the create, or to a later status query for it.
  reply(item, reply) {
    if (item.status !== "sending") return;
    if (reply.result === "created") {
      // The description stays in this page's memory; `stored` drops it.
      Object.assign(item, {
        status: "landed",
        key: reply.key,
        ticket: reply.ticket,
        column: reply.column || item.column,
      });
    } else if (reply.result === "rejected") {
      item.status = "rejected";
      item.message = reply.message || "";
    } else if (reply.result === "delivery" && reply.status === "unknown") {
      item.status = "unknown";
    }
  }
  // Sent, not yet answered: a reconnect asks the host what became of them.
  unresolved(board) {
    return this.items.filter((i) => i.board === board && i.status === "sending" && i.command !== undefined);
  }
  // What needs the reader: an answer still out, or one that did not land.
  unsettled(board) {
    return this.items.filter((i) => i.board === board && i.status !== "landed").length;
  }
  remove(id) {
    this.items = this.items.filter((i) => i.id !== id);
  }
  purge(board) {
    this.items = this.items.filter((i) => i.board !== board);
  }
  // What survives a reload: every unsettled item whole, and the newest KEEP
  // landed ones without a description.
  stored(board) {
    const mine = this.forBoard(board);
    const landed = mine.filter((i) => i.status === "landed").slice(-KEEP);
    return mine
      .filter((i) => i.status !== "landed" || landed.includes(i))
      .map(({ id, title, description, column, tags, at, status, command, incarnation, key, ticket, message }) => ({
        id,
        title,
        description: status === "landed" ? "" : description,
        column,
        tags: tags.map(({ group, name, tint }) => ({ group, name, tint })),
        at,
        status,
        command,
        incarnation,
        key,
        ticket,
        message,
      }));
  }
  // Put back what the last page kept, beside anything sent since it loaded.
  restore(board, stored) {
    if (!Array.isArray(stored)) return;
    for (const s of stored) {
      if (!s || typeof s.id !== "string" || typeof s.title !== "string" || !statuses.includes(s.status)) continue;
      if (this.get(s.id)) continue;
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
        status: s.status,
        command: Number.isInteger(s.command) ? s.command : undefined,
        incarnation: typeof s.incarnation === "string" ? s.incarnation : undefined,
        key: typeof s.key === "string" ? s.key : "",
        ticket: typeof s.ticket === "string" ? s.ticket : "",
        message: typeof s.message === "string" ? s.message : "",
      });
    }
  }
}
