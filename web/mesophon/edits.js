// The card edits this tab sent (T-530): a rename, a move, a tag or a
// workspace (T-642), by command id, until the host answers. While one waits
// the board wears it, so a snapshot the host took before it had the edit
// cannot put the card back for a moment. The answer, or a dropped
// connection, ends it, and the next snapshot is the truth. Never replayed and never kept: after a drop or
// a reload the board itself says what took.
export class Edits {
  constructor() {
    this.items = new Map(); // command id -> { board, ticket, key, op, patch, before }
  }
  sent(command, item) {
    this.items.set(command, item);
    return item;
  }
  take(command) {
    const item = this.items.get(command);
    this.items.delete(command);
    return item;
  }
  waiting(board, ticket, op) {
    return [...this.items.values()].some(
      (i) => i.board === board && i.ticket === ticket && (!op || i.op === op),
    );
  }
  lost() {
    this.items.clear();
  }
  purge(board) {
    for (const [command, item] of this.items) if (item.board === board) this.items.delete(command);
  }
  // A host's tickets wearing what waits, oldest first. A move takes the card
  // to its slot, as the host will: before `before` when that card is in the
  // column, else after the column's last card.
  wear(board, tickets, items = this.items.values()) {
    const worn = tickets.slice();
    for (const item of items) {
      if (item.board !== board) continue;
      const at = worn.findIndex((t) => t.id === item.ticket);
      if (at < 0) continue;
      const ticket = { ...worn[at], ...item.patch };
      if (item.op !== "move") {
        worn[at] = ticket;
        continue;
      }
      worn.splice(at, 1);
      let to = item.before ? worn.findIndex((t) => t.id === item.before && t.column === ticket.column) : -1;
      if (to < 0) {
        const last = worn.findLastIndex((t) => t.column === ticket.column);
        to = last < 0 ? worn.length : last + 1;
      }
      worn.splice(to, 0, ticket);
    }
    return worn;
  }
}
