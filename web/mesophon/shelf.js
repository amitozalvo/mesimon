// The shelf (T-698): what the terminal left at the relay for this browser
// to read while it is away. Each item is sealed to this browser and opened
// with the host key it pinned at pairing, so the relay can serve one late or
// not at all, never one of its own. Each is an answer the live channel also
// gives (the board, a ticket's notes, the newest page of a conversation),
// written at `at`; the store lays it where a live answer would go, unless
// what is there is newer. Nothing here is kept past the page: a reload asks
// the relay again.

// One opened item, checked: its kind, when it was written, what it holds.
export function shelfItem(body) {
  if (!body || !Number.isFinite(body.at)) return undefined;
  if (body.kind === "board" && body.board?.result === "board" && Array.isArray(body.board.tickets))
    return { kind: "board", at: body.at, board: body.board };
  if (body.kind === "notes" && typeof body.ticket === "string" && body.notes?.result === "notes")
    return {
      kind: "notes",
      at: body.at,
      ticket: body.ticket,
      notes: body.notes,
      bodies: (Array.isArray(body.bodies) ? body.bodies : []).filter((b) => b?.result === "note"),
    };
  if (
    body.kind === "transcript" &&
    typeof body.ticket === "string" &&
    typeof body.session === "string" &&
    body.page?.result === "transcript"
  )
    return { kind: "transcript", at: body.at, ticket: body.ticket, session: body.session, page: body.page };
  return undefined;
}

// What was asked of the relay, per board, this away spell: a peek goes
// once until the terminal is live again or the relay's socket comes back.
export class Peeks {
  constructor() {
    this.asked = new Set();
  }
  want(board) {
    if (!board || this.asked.has(board)) return false;
    this.asked.add(board);
    return true;
  }
  again(board) {
    if (board) this.asked.delete(board);
    else this.asked.clear();
  }
}
