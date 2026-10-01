// Drafts, receipts and reading positions never cross board/ticket/session keys.

// A receipt's ticks: a clock while it is on its way, one when it went in and
// nothing confirmed it, two when the far side said it arrived. A dialog's
// keys are `input_sent` until the agent's own hook says `answered` (T-567).
export const receiptTick = (status) =>
  status === "awaiting_delivery"
    ? "clock"
    : ["queued", "input_sent"].includes(status)
      ? "one"
      : ["submitted", "decision_sent", "answered"].includes(status)
        ? "two"
        : null;

// Why the host could not answer a dialog, in words (T-567). A word this page
// does not know is shown as it came.
const reasonWords = {
  label_not_found: "the option is not on the screen",
  label_wrapped: "the option does not read as one row on the screen",
  shape_unrecognised: "the screen does not show this question",
  deadline: "it took too long",
  state_changed: "the agent moved on",
  pane_unreachable: "the pane did not take the keys",
  "cursor moved": "the selection already moved",
};
export const reasonText = (reason) =>
  String(reason)
    .split("; ")
    .map((word) => reasonWords[word] || word.replaceAll("_", " "))
    .join("; ");

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
        cols: undefined,
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
        answered: "Answered.",
        input_sent: "Keys sent, not confirmed · check the pane.",
        submitted:
          "Submitted to the agent. This confirms input delivery, not completion.",
        rejected: reply.message
          ? `Rejected: ${reply.message}`
          : "Prompt rejected. Check the agent before trying again.",
        unknown: "Delivery unknown. Check the agent before sending again.",
      }[status] || "Delivery unknown. Check the agent before sending again.";
    if (status === "unknown" && reply.reason)
      entry.delivery = `Could not answer: ${reasonText(reply.reason)} · try again or answer in the pane.`;
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
