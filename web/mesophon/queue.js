// A ticket's queued words as the page says them, and what a send meets
// (T-568). The words are the board's own grammar (the TUI's owed row):
// whose they are, whether they wait on your send, and whose turn or answer
// they wait on. A host before T-568 sends the words alone.

// The agent waits on a person: a dialog or a permission is up, or the host
// says it needs attention. A paste there would land in the dialog as its
// answer, so Steer and Send now wait for the answer; Queue does not.
export const waitsOnYou = (agent) =>
  !!agent && (!!agent.dialog || !!agent.permission || agent.state === "needs attention");

// The host refuses Send now while the agent waits on you, its own question
// read stale included (`asking` names the ticket's own key then).
export const sendRefused = (ticket) =>
  waitsOnYou(ticket?.agent) || !!ticket?.queue?.asking?.includes(ticket.key);

// Whose words wait for the turn's end, as `waits` and `asking` name them:
// `after T-3`, `after T-3 +1`, `after its turn` when only its own agent
// works. A question does not end by itself: an asking key reads `T-3's
// answer`, or `your answer` for its own, and is named first.
export function afterWords(waits = [], asking = [], own = "") {
  const asks = (key) => asking.includes(key);
  const others = waits.filter((key) => key !== own);
  const first = others.findIndex(asks);
  if (first > 0) others.unshift(...others.splice(first, 1));
  const name = (key) => (asks(key) ? `${key}'s answer` : key);
  if (!others.length)
    return !waits.length ? undefined : asks(own) ? "after your answer" : "after its turn";
  return others.length === 1 ? `after ${name(others[0])}` : `after ${name(others[0])} +${others.length - 1}`;
}

// The queued row's words. Held words wait on your send: a crown's, named
// by its ticket, or a person's the host held on a question. The rest go by
// themselves once what they wait on is done.
export function queueWords(ticket) {
  const queue = ticket?.queue;
  if (!queue) return "Queued · waits for idle";
  if (queue.held != null || (queue.by != null && !queue.sends)) {
    const lead = queue.by != null ? `${queue.by}'s agent` : `held · ${queue.held}`;
    return `${lead} · ${sendRefused(ticket) ? "you answer first" : "you send"}`;
  }
  const lead = queue.by != null ? `${queue.by}'s agent · queued` : "queued";
  return `${lead} · ${afterWords(queue.waits, queue.asking, ticket.key) || "sends next"}`;
}

// Whose queued words a prompt took the place of, from its receipt.
export function replacedWords(replaced) {
  const by = replaced?.by;
  if (!by) return "";
  if (by === "you") return "It replaced your queued words.";
  if (by === "held") return "It replaced the words held for the agent’s question.";
  return `It replaced ${by}’s agent’s queued words.`;
}
