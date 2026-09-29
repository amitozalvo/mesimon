// Agents this browser started (T-498), per board and ticket: a clock while
// the host starts one (`sending`, then `provisioning` while a worktree is
// cut, then `starting`), two ticks once its session runs (`started`), or why
// not (`rejected`, `unknown`). Kept in this tab only, as a prompt's receipt
// is: after a reload the board itself says whether the agent is there.
const waitingStatuses = ["sending", "provisioning", "starting"];
const hostStatuses = ["provisioning", "starting", "started"];

export const startWaiting = (item) => waitingStatuses.includes(item?.status);

export class Starts {
  constructor() {
    this.items = new Map();
  }
  key(board, ticket) {
    return JSON.stringify([board, ticket]);
  }
  get(board, ticket) {
    return this.items.get(this.key(board, ticket));
  }
  // A new press replaces whatever the last one on this ticket came to.
  sent(board, ticket, command, incarnation, key = "") {
    const item = { board, ticket, key, command, incarnation, status: "sending", message: "", at: Date.now() };
    this.items.set(this.key(board, ticket), item);
    return item;
  }
  // The host's answer, to the start op or a status query. A settled start
  // stays settled: a late `status` cannot take two ticks back.
  reply(item, reply) {
    if (!startWaiting(item)) return;
    if (reply.result === "rejected") {
      item.status = "rejected";
      item.message = reply.message || "";
    } else if (reply.result === "delivery" && hostStatuses.includes(reply.status)) item.status = reply.status;
    else item.status = "unknown";
    if (item.status === "started") item.at = Date.now();
  }
  unresolved(board) {
    return [...this.items.values()].filter((i) => i.board === board && startWaiting(i));
  }
  purge(board) {
    for (const [key, item] of this.items) if (item.board === board) this.items.delete(key);
  }
}
