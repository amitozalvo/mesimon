// Drafts, receipts and reading positions never cross board/ticket/session keys.
export class Sessions {
  constructor() {
    this.entries = new Map();
    this.targets = new Map();
  }
  get(board, ticket) {
    if (!ticket) return undefined;
    const target = JSON.stringify([board, ticket.id]);
    const key = JSON.stringify([
      board,
      ticket.id,
      ticket.agent?.session || null,
    ]);
    if (!this.entries.has(key)) {
      const previous = this.entries.get(this.targets.get(target));
      this.entries.set(key, {
        key,
        board,
        ticket: ticket.id,
        session: ticket.agent?.session,
        draft: previous?.draft || "",
        review: !!previous?.draft,
        delivery: "",
        mode: "queue",
        returned: undefined,
        receipt: undefined,
        output: "",
        displayed: "",
        receivedAt: undefined,
        scroll: 0,
        following: true,
        unread: false,
      });
    }
    this.targets.set(target, key);
    return this.entries.get(key);
  }
  lost() {
    for (const entry of this.entries.values())
      if (entry.receipt?.waiting) {
        entry.receipt.waiting = false;
        this.retain(entry, entry.receipt.text);
        entry.receipt.restored = true;
        entry.delivery =
          "Delivery unknown. Check the agent before sending again.";
      }
  }
  retain(entry, text, session = entry.session) {
    if (!text) return;
    if (!entry.draft || entry.draft === text) {
      entry.draft = text;
      if (session !== entry.session) entry.review = true;
    } else entry.returned = { text, session };
  }
  sent(entry, id, incarnation, op = "prompt", text = entry.draft) {
    entry.receipt = {
      id,
      incarnation,
      text,
      op,
      status: "awaiting_delivery",
      waiting: true,
      unresolved: true,
    };
    entry.delivery = "Sending…";
  }
  reply(entry, reply) {
    const receipt = entry?.receipt;
    if (!receipt) return;
    if (reply.result === "taken_back") {
      receipt.waiting = receipt.unresolved = false;
      entry.delivery = "Taken back. Edit or discard the prompt.";
      const target = JSON.stringify([entry.board, entry.ticket]);
      const current = this.entries.get(this.targets.get(target)) || entry;
      this.retain(current, reply.text, entry.session);
      return;
    }
    const status = reply.result === "rejected" ? "rejected" : reply.status;
    receipt.status = status;
    receipt.waiting = ["queued", "awaiting_delivery"].includes(status);
    receipt.unresolved = receipt.waiting;
    entry.delivery =
      {
        queued: "Queued · waiting for idle.",
        awaiting_delivery: "Sending… Awaiting delivery.",
        decision_sent: "Decision sent. Check the output for the agent’s response.",
        input_sent: "Answer keys sent. Check the output to confirm the result.",
        submitted:
          "Submitted to the agent. This confirms input delivery, not completion.",
        rejected: reply.message
          ? `Rejected: ${reply.message}`
          : "Prompt rejected. Check the agent before trying again.",
        unknown: "Delivery unknown. Check the agent before sending again.",
      }[status] || "Delivery unknown. Check the agent before sending again.";
    if (
      ["queued", "submitted"].includes(status) &&
      (receipt.op === "prompt" || receipt.restored) &&
      entry.draft === receipt.text
    )
      entry.draft = "";
    if (["unknown", "rejected"].includes(status))
      this.retain(entry, receipt.text);
  }
  purge(board) {
    for (const [key, entry] of this.entries)
      if (entry.board === board) this.entries.delete(key);
    for (const [target, key] of this.targets)
      if (!this.entries.has(key)) this.targets.delete(target);
  }
}
