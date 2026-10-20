// Words for an agent written while the terminal is away (T-881), sealed for
// the relay's mailbox as a ticket is (T-497): one per ticket, a clock while
// it is in this browser, one tick at the relay, and gone once the terminal
// queued it, its words then the ticket's queued row. A refusal gives the
// words back to the box; a letter the relay lost reads unknown until it is
// dismissed, since it may have reached the agent.

const kept = ["local", "relay", "unknown"];

export class PromptMail {
  constructor() {
    this.items = [];
  }
  get(id) {
    return this.items.find((i) => i.id === id);
  }
  forBoard(board) {
    return this.items.filter((i) => i.board === board).sort((a, b) => a.at - b.at);
  }
  // The letter a ticket's composer shows, if one is on its way or lost.
  of(board, ticket) {
    return this.items.find((i) => i.board === board && i.ticket === ticket);
  }
  add(board, { ticket, key = "", session, provider = "", text }, envelope, at = Date.now()) {
    const item = { id: envelope.id, board, ticket, key, session, provider, text, at, status: "local", envelope };
    this.items.push(item);
    return item;
  }
  deposited(item) {
    if (item.status !== "local") return;
    item.status = "relay";
    item.envelope = undefined;
  }
  gone(item) {
    if (item.status === "relay") item.status = "unknown";
  }
  waiting(board) {
    return this.forBoard(board).filter((i) => i.status === "local");
  }
  atRelay(board) {
    return this.forBoard(board).filter((i) => i.status === "relay");
  }
  remove(id) {
    this.items = this.items.filter((i) => i.id !== id);
  }
  purge(board) {
    this.items = this.items.filter((i) => i.board !== board);
  }
  stored(board) {
    return this.forBoard(board).map(({ id, ticket, key, session, provider, text, at, status, envelope }) => ({
      id,
      ticket,
      key,
      session,
      provider,
      text,
      at,
      status,
      envelope: status === "local" ? envelope : undefined,
    }));
  }
  restore(board, stored) {
    if (!Array.isArray(stored)) return;
    for (const s of stored) {
      if (!s || typeof s.id !== "string" || typeof s.ticket !== "string" || typeof s.text !== "string") continue;
      if (!kept.includes(s.status) || this.get(s.id) || this.of(board, s.ticket)) continue;
      const envelope = s.envelope && typeof s.envelope.id === "string" ? s.envelope : undefined;
      // A sealed copy lost from storage cannot go out any more.
      const status = s.status === "local" && !envelope ? "unknown" : s.status;
      this.items.push({
        id: s.id,
        board,
        ticket: s.ticket,
        key: typeof s.key === "string" ? s.key : "",
        session: typeof s.session === "string" ? s.session : undefined,
        provider: typeof s.provider === "string" ? s.provider : "",
        text: s.text,
        at: Number.isFinite(s.at) ? s.at : 0,
        status,
        envelope: status === "local" ? envelope : undefined,
      });
    }
  }
}

// What the waiting bubble says under the words.
export const letterWords = (item) =>
  item.status === "local"
    ? "Waits in this browser · goes to the relay when it answers"
    : item.status === "relay"
      ? "Waits at the relay · queues when your terminal is back"
      : "Delivery unknown · check the agent before sending again";
