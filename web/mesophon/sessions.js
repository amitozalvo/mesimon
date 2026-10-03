// Drafts, receipts and reading positions never cross board/ticket/session keys.
import { replacedWords } from "./queue.js";

// A dialog's or a permission's answer keeps its own receipt beside the
// prompt's (T-568): a prompt queued for the turn's end is not input on its
// way, and the answer buttons wait only on an answer, or on a prompt that
// is going into the pane.
const answerOps = ["dialog", "permission"];
const slotOf = (op) => (answerOps.includes(op) ? "answer" : "receipt");
export const answerBusy = (entry) =>
  !!entry?.answer?.waiting ||
  (!!entry?.receipt?.waiting && entry.receipt.status === "awaiting_delivery");

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
  // A batch or a several-choice question (T-571).
  tick_not_taken: "an option did not take its tick",
  answer_differs: "the pane's answers differ from yours, so Submit was not pressed",
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
        // The prompt's receipt, the answer's, and whichever was sent last:
        // the delivery line and its ticks are that one's.
        receipt: undefined,
        answer: undefined,
        latest: undefined,
        output: "",
        displayed: "",
        cols: undefined,
        receivedAt: undefined,
        scroll: 0,
        following: true,
        unread: false,
        // The conversation (T-626): the pages held, whether the page before
        // is on its way, and the reader's place in it.
        chat: undefined,
        chatOlder: false,
        chatWantsOlder: false,
        chatError: "",
        chatScroll: 0,
        chatFollowing: true,
        chatUnread: false,
        chatAt: undefined,
      });
    }
    this.targets.set(target, key);
    return this.entries.get(key);
  }
  lost() {
    for (const entry of this.entries.values())
      for (const receipt of [entry.answer, entry.receipt])
        if (receipt?.waiting) {
          receipt.waiting = false;
          this.retain(entry, receipt.text);
          receipt.restored = true;
          entry.latest = receipt;
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
    const receipt = {
      id,
      incarnation,
      text,
      op,
      status: "awaiting_delivery",
      waiting: true,
      unresolved: true,
    };
    entry[slotOf(op)] = entry.latest = receipt;
    entry.delivery = "Sending…";
  }
  // The host's answer for the command `id`, the latest sent by default. A
  // receipt that is not the latest changes quietly: the line keeps saying
  // what was sent last.
  reply(entry, reply, id = entry?.latest?.id) {
    const receipt = [entry?.receipt, entry?.answer].find((r) => r && r.id === id);
    if (!receipt) return;
    const said = receipt === entry.latest;
    if (reply.result === "taken_back") {
      receipt.waiting = receipt.unresolved = false;
      if (said) entry.delivery = "Taken back. Edit or discard the prompt.";
      const target = JSON.stringify([entry.board, entry.ticket]);
      const current = this.entries.get(this.targets.get(target)) || entry;
      this.retain(current, reply.text, entry.session);
      return;
    }
    const status = reply.result === "rejected" ? "rejected" : reply.status;
    receipt.status = status;
    receipt.waiting = ["queued", "awaiting_delivery"].includes(status);
    receipt.unresolved = receipt.waiting;
    if (reply.replaced) receipt.replaced = reply.replaced;
    if (said) entry.delivery = deliveryWords(status, reply, receipt);
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

// A receipt's line. The queued row says what the words wait on (T-568);
// the line says they are queued, and whose words they took the place of.
function deliveryWords(status, reply, receipt) {
  let words =
    {
      queued: "Queued.",
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
    words = `Could not answer: ${reasonText(reply.reason)} · try again or answer in the pane.`;
  const replaced = replacedWords(receipt.replaced);
  return replaced && !["rejected", "unknown"].includes(status) ? `${words} ${replaced}` : words;
}
