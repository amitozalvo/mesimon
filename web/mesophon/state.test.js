import { test } from "node:test";
import assert from "node:assert/strict";
import { Sessions, answerBusy, receiptTick, reasonText } from "./sessions.js";
import { afterWords, queueWords, sendRefused, waitsOnYou } from "./queue.js";
import { BoardState, RECENT_MS } from "./board.js";
import { answerable, dialogForm, formAnswers, measured } from "./dialogs.js";
import { mergePage, tailAsk } from "./transcript.js";
const ticket = (id, session) => ({
  id,
  key: id,
  title: `Ticket ${id}`,
  column: "TODO",
  agent: session ? { session, state: "working", provider: "claude" } : null,
});
test("drafts are isolated across tickets, sessions and boards; replacements require review", () => {
  const sessions = new Sessions();
  const first = sessions.get("board-a", ticket("one", "session-a"));
  first.draft = "only for one";
  assert.equal(sessions.get("board-a", ticket("two", "session-b")).draft, "");
  assert.equal(sessions.get("board-b", ticket("one", "session-a")).draft, "");
  const replaced = sessions.get("board-a", ticket("one", "session-c"));
  assert.equal(replaced.draft, "only for one");
  assert.equal(replaced.review, true);
  assert.equal(sessions.get("board-a", ticket("one", "session-a")), first);
});
test("delivery acknowledgement clears only the submitted draft, not newer edits", () => {
  const sessions = new Sessions();
  const entry = sessions.get("board", ticket("one", "session"));
  entry.draft = "sent";
  sessions.sent(entry, 42, "incarnation");
  sessions.reply(entry, { result: "delivery", status: "awaiting_delivery" });
  assert.equal(entry.draft, "sent");
  assert.equal(entry.receipt.waiting, true);
  entry.draft = "next draft";
  sessions.reply(entry, { result: "delivery", status: "submitted" });
  assert.equal(entry.draft, "next draft");
  assert.equal(entry.receipt.waiting, false);
});
test("disconnect preserves input and queryable receipts without a waiting lock", () => {
  const sessions = new Sessions();
  const entry = sessions.get("board", ticket("one", "session"));
  entry.draft = "uncertain";
  sessions.sent(entry, 42, "incarnation");
  sessions.lost();
  assert.equal(entry.draft, "uncertain");
  assert.equal(entry.receipt.waiting, false);
  assert.equal(entry.receipt.unresolved, true);
  assert.match(entry.delivery, /Delivery unknown/);
  sessions.reply(entry, { result: "rejected", message: "Session replaced" });
  assert.equal(entry.draft, "uncertain");
  assert.match(entry.delivery, /Session replaced/);
});
test("revocation purges only the affected board's protected state", () => {
  const sessions = new Sessions();
  sessions.get("a", ticket("one", "a")).draft = "private";
  const keep = sessions.get("b", ticket("one", "a"));
  sessions.purge("a");
  assert.equal(sessions.entries.size, 1);
  assert.equal(sessions.get("b", ticket("one", "a")), keep);
  assert.equal(sessions.get("a", ticket("one", "a")).draft, "");
});
test("empty, single, many projections preserve selected identity under filters and reordering", () => {
  const board = new BoardState();
  board.update({ title: "Empty", columns: ["TODO"], tickets: [] });
  assert.equal(board.current, undefined);
  assert.deepEqual(board.visible(), []);
  const rows = [ticket("one", "a"), ticket("two", "b"), ticket("three")];
  rows[1].agent.state = "needs attention";
  board.update({ title: "Board", columns: ["TODO"], tickets: rows });
  assert.deepEqual(
    board.visible().map((t) => t.id),
    ["two", "one"],
  );
  board.selected = "two";
  board.search = "one";
  assert.deepEqual(
    board.visible().map((t) => t.id),
    ["one"],
  );
  assert.equal(board.current.id, "two");
  board.update({
    title: "Board",
    columns: ["TODO"],
    tickets: [...rows].reverse(),
  });
  assert.equal(board.current.id, "two");
  board.search = "";
  board.filter = "attention";
  assert.deepEqual(
    board.visible().map((t) => t.id),
    ["two"],
  );
  board.mode = "board";
  assert.equal(board.visible().length, 3);
  board.update({ title: "Board", columns: ["TODO"], tickets: [rows[0]] });
  assert.equal(board.current.id, "one");
});
test("queued receipts survive reconnect and never clear newer drafts", () => {
  const sessions = new Sessions();
  const entry = sessions.get("board", ticket("one", "session"));
  assert.equal(entry.mode, "queue");
  entry.draft = "waiting words";
  sessions.sent(entry, 42, "incarnation");
  sessions.reply(entry, { result: "delivery", status: "queued" });
  assert.equal(entry.draft, "");
  assert.equal(entry.receipt.waiting, true);
  sessions.lost();
  assert.equal(entry.draft, "waiting words");
  assert.equal(entry.receipt.unresolved, true);
  entry.draft = "next words";
  sessions.reply(entry, { result: "delivery", status: "queued" });
  assert.equal(entry.draft, "next words");
  sessions.sent(entry, 43, "incarnation", "send_now", "waiting words");
  sessions.reply(entry, { result: "delivery", status: "submitted" });
  assert.equal(entry.draft, "next words");
});
test("take-back receipts preserve a newer draft and stay bound to the original ticket", () => {
  const sessions = new Sessions();
  const entry = sessions.get("board", ticket("one", "session"));
  entry.draft = "draft being edited";
  sessions.sent(entry, 43, "incarnation", "take_back", "waiting words");
  const other = sessions.get("board", ticket("two", "other"));
  other.draft = "other ticket's words";
  sessions.reply(entry, { result: "taken_back", text: "waiting words" });
  assert.equal(entry.draft, "draft being edited");
  assert.deepEqual(entry.returned, {
    text: "waiting words",
    session: "session",
  });
  assert.equal(other.draft, "other ticket's words");
  assert.equal(entry.receipt.unresolved, false);
});
test("late take-back after session replacement retains the text for review", () => {
  const sessions = new Sessions();
  const old = sessions.get("board", ticket("one", "old"));
  sessions.sent(old, 44, "incarnation", "take_back", "old session's words");
  const current = sessions.get("board", ticket("one", "new"));
  sessions.reply(old, { result: "taken_back", text: "old session's words" });
  assert.equal(current.draft, "old session's words");
  assert.equal(current.review, true);
});
test("a queue cancelled by the host returns the unsent words to the draft", () => {
  const sessions = new Sessions();
  const entry = sessions.get("board", ticket("one", "session"));
  entry.draft = "not delivered";
  sessions.sent(entry, 45, "incarnation");
  sessions.reply(entry, { result: "delivery", status: "queued" });
  assert.equal(entry.draft, "");
  sessions.reply(entry, { result: "delivery", status: "rejected" });
  assert.equal(entry.draft, "not delivered");
  assert.equal(entry.receipt.waiting, false);
});


test("awareness alerts follow daemon phases and silence the visible ticket", async () => {
  const { shouldAlert } = await import("./awareness.js");
  const event = { result: "awareness", ticket: "one", alert: true, awareness: { phase: "completed" } };
  assert.equal(shouldAlert(event, "one"), false);
  assert.equal(shouldAlert(event, "two"), true);
  assert.equal(shouldAlert({ ...event, alert: false }, "two"), false);
  for (const phase of ["running", "starting", "stale"]) {
    assert.equal(shouldAlert({ ...event, awareness: { phase } }, null), false);
  }
});

test("dialog and approval receipts preserve drafts and never imply execution", () => {
  const sessions = new Sessions();
  const entry = sessions.get("board-a", ticket("one", "session-a"));
  entry.draft = "later follow-up";
  sessions.sent(entry, 7, "host", "permission", "");
  sessions.reply(entry, { result: "delivery", status: "decision_sent" });
  assert.equal(entry.answer.unresolved, false);
  assert.equal(entry.draft, "later follow-up");
  assert.match(entry.delivery, /Decision sent/);
  sessions.sent(entry, 8, "host", "dialog", "");
  sessions.lost();
  assert.match(entry.delivery, /unknown/);
  assert.equal(entry.draft, "later follow-up");
});

test("a dialog answer is answered only on the hook's word, and an unknown one says why (T-567)", () => {
  const sessions = new Sessions();
  const entry = sessions.get("board-a", ticket("one", "session-a"));
  sessions.sent(entry, 1, "host", "dialog", "");
  sessions.reply(entry, { result: "delivery", status: "awaiting_delivery" });
  assert.equal(entry.answer.waiting, true, "the card's buttons wait");
  assert.equal(receiptTick(entry.answer.status), "clock");
  // The agent's own hook said the dialog took it: two ticks.
  sessions.reply(entry, { result: "delivery", status: "answered" });
  assert.equal(entry.answer.unresolved, false);
  assert.equal(entry.delivery, "Answered.");
  assert.equal(receiptTick(entry.answer.status), "two");
  // Keys went in and nothing confirmed them: one tick, and the buttons live.
  sessions.sent(entry, 2, "host", "dialog", "");
  sessions.reply(entry, { result: "delivery", status: "input_sent" });
  assert.equal(entry.answer.waiting, false);
  assert.equal(entry.delivery, "Keys sent, not confirmed · check the pane.");
  assert.equal(receiptTick(entry.answer.status), "one");
  // No key could be chosen: no tick, the reason in words, try again.
  sessions.sent(entry, 3, "host", "dialog", "");
  sessions.reply(entry, { result: "delivery", status: "unknown", reason: "label_wrapped; cursor moved" });
  assert.equal(entry.answer.waiting, false);
  assert.equal(
    entry.delivery,
    "Could not answer: the option does not read as one row on the screen; the selection already moved · try again or answer in the pane.",
  );
  assert.equal(receiptTick(entry.answer.status), null);
  // An older host names no reason, and a word this page does not know shows as it came.
  sessions.sent(entry, 4, "host", "dialog", "");
  sessions.reply(entry, { result: "delivery", status: "unknown" });
  assert.match(entry.delivery, /Delivery unknown/);
  assert.equal(reasonText("pane_gone"), "pane gone");
});

test("a queued prompt never locks the answer buttons; the answer keeps its own receipt (T-568)", () => {
  const sessions = new Sessions();
  const entry = sessions.get("board-a", ticket("one", "session-a"));
  entry.draft = "after the question";
  sessions.sent(entry, 1, "host");
  sessions.reply(entry, { result: "delivery", status: "queued" });
  assert.equal(entry.receipt.waiting, true, "the words wait for the turn");
  assert.equal(answerBusy(entry), false, "a queued prompt is not input on its way");
  sessions.sent(entry, 2, "host", "dialog", "");
  assert.equal(answerBusy(entry), true, "an answer on its way holds the buttons");
  assert.equal(entry.receipt.id, 1, "the queued prompt keeps its receipt");
  sessions.reply(entry, { result: "delivery", status: "answered" }, 2);
  assert.equal(answerBusy(entry), false);
  assert.equal(entry.delivery, "Answered.");
  assert.equal(receiptTick(entry.latest.status), "two");
  // The prompt's own receipt moves quietly: the line says what was sent last.
  sessions.reply(entry, { result: "delivery", status: "queued" }, 1);
  assert.equal(entry.delivery, "Answered.");
  // Cancelled by the host after all: the words come back to the draft.
  sessions.reply(entry, { result: "delivery", status: "rejected" }, 1);
  assert.equal(entry.draft, "after the question");
  // A prompt going into the pane is input on its way.
  entry.draft = "steer";
  sessions.sent(entry, 3, "host");
  assert.equal(answerBusy(entry), true);
  sessions.reply(entry, { result: "delivery", status: "submitted" }, 3);
  assert.equal(answerBusy(entry), false);
  // A drop restores what was waiting, the queued prompt's words included.
  entry.draft = "kept words";
  sessions.sent(entry, 4, "host");
  sessions.reply(entry, { result: "delivery", status: "queued" }, 4);
  sessions.sent(entry, 5, "host", "dialog", "");
  sessions.lost();
  assert.equal(answerBusy(entry), false);
  assert.equal(entry.draft, "kept words");
  assert.match(entry.delivery, /Delivery unknown/);
});

test("a receipt says whose queued words the prompt replaced (T-568)", () => {
  const sessions = new Sessions();
  const entry = sessions.get("board-a", ticket("one", "session-a"));
  const say = (id, replaced, status = "queued") => {
    entry.draft = `words ${id}`;
    sessions.sent(entry, id, "host");
    sessions.reply(entry, { result: "delivery", status, replaced });
    return entry.delivery;
  };
  assert.equal(say(1, { by: "T-411" }), "Queued. It replaced T-411’s agent’s queued words.");
  assert.equal(say(2, { by: "you" }), "Queued. It replaced your queued words.");
  assert.equal(say(3, { by: "held" }), "Queued. It replaced the words held for the agent’s question.");
  assert.match(say(4, { by: "you" }, "submitted"), /^Submitted to the agent\..* It replaced your queued words\.$/);
  assert.equal(say(5, undefined), "Queued.");
});

test("the queued row names whose words wait and on what (T-568)", () => {
  const row = (queue, state = "idle", extra = {}) =>
    queueWords({ key: "T-3", queued: "words", queue, agent: { state, ...extra } });
  // A host before T-568 sends the words alone.
  assert.equal(row(undefined), "Queued · waits for idle");
  // The crown's words wait on your send, and on your answer first.
  assert.equal(row({ by: "T-411" }), "T-411's agent · you send");
  assert.equal(row({ by: "T-411", asking: ["T-3"] }, "needs attention"), "T-411's agent · you answer first");
  // Words the host held on a question.
  assert.equal(row({ held: "agent asked" }), "held · agent asked · you send");
  assert.equal(row({ held: "agent asked", asking: ["T-3"] }, "needs attention"), "held · agent asked · you answer first");
  // Words that go by themselves.
  assert.equal(row({ waits: ["T-5"], asking: ["T-5"] }), "queued · after T-5's answer");
  assert.equal(row({ waits: ["T-2", "T-5"], asking: ["T-5"] }), "queued · after T-5's answer +1");
  assert.equal(row({ waits: ["T-2", "T-5"] }), "queued · after T-2 +1");
  assert.equal(row({ waits: ["T-3"] }, "working"), "queued · after its turn");
  assert.equal(row({}), "queued · sends next");
  assert.equal(row({ by: "T-411", sends: true, waits: ["T-5"] }), "T-411's agent · queued · after T-5");
  assert.equal(afterWords(["T-3"], ["T-3"], "T-3"), "after your answer");
  assert.equal(afterWords([], [], "T-3"), undefined);
});

test("Steer and Send now wait while the agent waits on you; Queue does not (T-568)", () => {
  const at = (agent, queue) => ({ key: "T-3", queued: "words", queue, agent });
  assert.equal(waitsOnYou({ state: "working" }), false);
  assert.equal(waitsOnYou({ state: "needs attention" }), true);
  assert.equal(waitsOnYou({ state: "working", dialog: { kind: "plan" } }), true);
  assert.equal(waitsOnYou({ state: "working", permission: { tool: "Bash" } }), true);
  assert.equal(waitsOnYou(undefined), false);
  assert.equal(sendRefused(at({ state: "idle" }, {})), false);
  assert.equal(sendRefused(at({ state: "needs attention" }, {})), true);
  // A question the host reads stale still refuses the send.
  assert.equal(sendRefused(at({ state: "unknown" }, { held: "agent asked", asking: ["T-3"] })), true);
  assert.equal(sendRefused(at({ state: "idle" }, { waits: ["T-5"], asking: ["T-5"] })), false);
});

test("a batch is answered whole: one answer per question, words in place of picks (T-571)", () => {
  const q = (question, multiSelect, ...labels) => ({
    question, header: question, multiSelect, options: labels.map((label) => ({ label, description: "" })),
  });
  const dialog = { request: "r1", kind: "questions", questions: [
    q("Which color?", false, "Blue", "Green"),
    q("Which toppings?", true, "Cheese", "Olives", "Basil"),
    q("Which size?", false, "Small", "Large"),
  ] };
  assert.equal(answerable(dialog), false, "the card's one-tap answer is for one single-choice question");
  assert.equal(measured(dialog), true);
  assert.equal(measured({ ...dialog, questions: [dialog.questions[0], dialog.questions[0]] }), false, "twins");
  assert.equal(measured({ ...dialog, questions: [...dialog.questions, ...dialog.questions.slice(0, 2)] }), false, "five");
  assert.equal(measured({ request: "p", kind: "plan", markdown: "x" }), false);
  const entry = {};
  const form = dialogForm(entry, dialog);
  assert.equal(formAnswers(dialog, form), null, "nothing picked yet");
  form.picks[0] = [1];
  form.picks[1] = [2, 0];
  assert.equal(formAnswers(dialog, form), null, "the third question still open");
  form.texts[2] = "  Large please ";
  assert.deepEqual(formAnswers(dialog, form), [
    { answer: "choice", index: 1 },
    { answer: "choices", indices: [0, 2] },
    { answer: "text", text: "Large please" },
  ]);
  // Words replace a question's picks; blank words do not.
  form.texts[1] = "Pineapple";
  assert.deepEqual(formAnswers(dialog, form)[1], { answer: "text", text: "Pineapple" });
  form.texts[1] = "   ";
  assert.deepEqual(formAnswers(dialog, form)[1], { answer: "choices", indices: [0, 2] });
  // The same request keeps the form through a retry; a new one starts afresh.
  assert.equal(dialogForm(entry, dialog), form);
  assert.deepEqual(dialogForm(entry, { ...dialog, request: "r2" }).picks, [[], [], []]);
  assert.equal(
    reasonText("answer_differs; cursor moved"),
    "the pane's answers differ from yours, so Submit was not pressed; the selection already moved",
  );
  assert.equal(reasonText("tick_not_taken"), "an option did not take its tick");
});

test("Now groups agents by the host's own state word", () => {
  const board = new BoardState();
  const rows = [ticket("one", "a"), ticket("two", "b"), ticket("three", "c"), ticket("four")];
  rows[1].agent.state = "needs attention";
  rows[2].agent.state = "idle";
  board.update({ title: "Board", columns: ["TODO"], tickets: rows });
  const { needs, working, idle } = board.sections();
  assert.deepEqual(needs.map((t) => t.id), ["two"]);
  assert.deepEqual(working.map((t) => t.id), ["one"]);
  assert.deepEqual(idle.map((t) => t.id), ["three"]);
  board.search = "three";
  assert.deepEqual(board.sections().idle.map((t) => t.id), ["three"]);
  assert.equal(board.sections().working.length, 0);
});

test("Now keeps a stopped agent for an hour; the Board keeps it always (T-560)", () => {
  const now = Date.now();
  const rows = ["fresh", "stale", "parked", "working", "needs", "older-host"].map((id) => ticket(id, id));
  Object.assign(rows[0].agent, { state: "idle", since: now - RECENT_MS + 60000 });
  Object.assign(rows[1].agent, { state: "idle", since: now - RECENT_MS - 60000 });
  Object.assign(rows[2].agent, { state: "sleeping", since: now - 3 * RECENT_MS });
  Object.assign(rows[3].agent, { state: "working", since: now - 3 * RECENT_MS });
  Object.assign(rows[4].agent, { state: "needs attention", since: now - 3 * RECENT_MS });
  rows[5].agent.state = "exited";
  const board = new BoardState();
  board.update({ title: "Board", columns: ["TODO"], tickets: rows });
  const { needs, working, idle } = board.sections();
  assert.deepEqual(needs.map((t) => t.id), ["needs"]);
  assert.deepEqual(working.map((t) => t.id), ["working"]);
  assert.deepEqual(idle.map((t) => t.id), ["fresh", "older-host"]);
  assert.equal(board.visible().length, 4);
  board.search = "stale";
  assert.deepEqual(board.visible(), []);
  board.search = "";
  board.mode = "board";
  assert.equal(board.visible().length, 6);
  // A remembered board is measured at the moment it was seen: what was
  // recent then is listed, however long ago that was.
  const remembered = new BoardState();
  remembered.update(board.snapshot(), { cached: true, at: now - 5 * RECENT_MS });
  for (const t of remembered.tickets) if (t.agent.since) t.agent.since -= 5 * RECENT_MS;
  assert.deepEqual(remembered.sections().idle.map((t) => t.id), ["fresh", "older-host"]);
});

test("a remembered board keeps no prompt text, tool input or dialog", () => {
  const board = new BoardState();
  const row = ticket("one", "session");
  row.queued = "private follow-up words";
  row.agent.promptable = true;
  row.agent.permission = { request: "r", tool: "Bash", input: { command: "secret" }, expires_at: 1 };
  row.agent.dialog = { request: "d", kind: "plan", markdown: "private plan" };
  board.update({ title: "Board", columns: ["TODO"], tickets: [row] });
  const text = JSON.stringify(board.snapshot());
  for (const secret of ["private follow-up", "secret", "private plan"])
    assert(!text.includes(secret), `${secret} leaked into the remembered board`);
  const restored = new BoardState();
  restored.update(board.snapshot(), { cached: true, at: 42 });
  assert.equal(restored.cached, true);
  assert.equal(restored.receivedAt, 42);
  assert.equal(restored.tickets[0].agent.promptable, false);
  assert.equal(restored.tickets[0].agent.session, "session");
  restored.update({ title: "Board", columns: ["TODO"], tickets: [row] });
  assert.equal(restored.cached, false);
});

test("Sent keeps an answer's key and forgets a landed description across a reload", async () => {
  const { Sent, KEEP } = await import("./sent.js");
  const sent = new Sent();
  const tag = { group: 1, name: "BUG", tint: 0 };
  const item = sent.add("board-a", { title: "Fix it", description: "private brief", column: "TODO", tags: [tag] }, 1);
  assert.equal(item.status, "sending");
  assert.deepEqual(sent.unresolved("board-a"), [], "nothing is asked about before it is sent");
  sent.sent(item, 9, "incarnation");
  assert.deepEqual(sent.unresolved("board-a"), [item]);
  assert.equal(sent.unsettled("board-a"), 1);
  sent.reply(item, { result: "created", ticket: "t-1", key: "T-7", column: "BACKLOG" });
  assert.equal(item.status, "landed");
  assert.equal(item.column, "BACKLOG", "where the host put it");
  assert.equal(item.description, "private brief", "the page still shows it");
  sent.reply(item, { result: "rejected", message: "late" });
  assert.equal(item.status, "landed", "a late answer never unlands a ticket");
  const refused = sent.add("board-a", { title: "Refused", description: "keep me", column: "GONE" }, 2);
  sent.sent(refused, 10, "incarnation");
  sent.reply(refused, { result: "rejected", message: "no such column: GONE" });
  const unknown = sent.add("board-a", { title: "Lost", description: "keep me too", column: "TODO" }, 3);
  sent.sent(unknown, 11, "incarnation");
  sent.reply(unknown, { result: "delivery", status: "unknown" });
  assert.equal(sent.unsettled("board-a"), 2);
  const stored = JSON.parse(JSON.stringify(sent.stored("board-a")));
  assert(!JSON.stringify(stored).includes("private brief"), "a landed description is not stored");
  assert.equal(stored.find((s) => s.title === "Refused").description, "keep me");
  const reloaded = new Sent();
  reloaded.restore("board-a", [...stored, null, { id: 4, title: "bad" }, { id: "x", title: "t", status: "weird" }]);
  assert.deepEqual(reloaded.forBoard("board-a").map((i) => i.title), ["Fix it", "Refused", "Lost"]);
  assert.equal(reloaded.get(item.id).key, "T-7");
  assert.deepEqual(reloaded.get(item.id).tags, [tag]);
  reloaded.restore("board-a", stored);
  assert.equal(reloaded.forBoard("board-a").length, 3, "restoring twice adds nothing");
  reloaded.purge("board-a");
  assert.equal(reloaded.forBoard("board-a").length, 0);
  const many = new Sent();
  for (let i = 0; i < KEEP + 5; i++) many.sent(many.add("b", { title: `t${i}`, column: "TODO" }, i), i + 1, "inc");
  for (const i of many.items) many.reply(i, { result: "created", ticket: i.title, key: i.title, column: "TODO" });
  const kept = many.stored("b");
  assert.equal(kept.length, KEEP);
  assert.equal(kept[0].title, "t5", "the oldest landed tickets go first");
});

test("the remembered board keeps the New ticket sheet's facts", () => {
  const board = new BoardState();
  board.update({
    title: "Board",
    columns: ["BACKLOG", "TODO"],
    tickets: [],
    default_column: "TODO",
    column_descriptions: { TODO: "soon" },
    allowed_tags: [{ group: 1, name: "BUG", tint: 0, extra: "dropped" }],
  });
  assert.equal(board.landing(), "TODO");
  const restored = new BoardState();
  restored.update(JSON.parse(JSON.stringify(board.snapshot())), { cached: true });
  assert.equal(restored.landing(), "TODO");
  assert.deepEqual(restored.columnDescriptions, { TODO: "soon" });
  assert.deepEqual(restored.allowedTags, [{ group: 1, name: "BUG", tint: 0 }]);
  const older = new BoardState();
  older.update({ title: "Old host", columns: ["TODO", "DONE"], tickets: [] });
  assert.equal(older.landing(), "TODO", "an older host's board lands in its first column");
  assert.deepEqual(older.allowedTags, []);
});

test("a mailed ticket goes clock, one tick, two ticks, and keeps its seal only while local", async () => {
  const { Sent } = await import("./sent.js");
  const sent = new Sent();
  const envelope = { id: "envelope-1", wrapped: {}, record: {} };
  const item = sent.add("board", { title: "Away", description: "words", column: "TODO" }, 1, envelope);
  assert.equal(item.id, "envelope-1", "the relay's id is the item's");
  assert.equal(item.status, "local");
  assert.deepEqual(sent.waiting("board"), [item]);
  let stored = sent.stored("board");
  assert.deepEqual(stored[0].envelope, envelope, "a reload can still send it");
  sent.deposited(item);
  assert.equal(item.status, "relay");
  assert.equal(item.envelope, undefined);
  stored = sent.stored("board");
  assert.equal(stored[0].envelope, undefined);
  assert.equal(stored[0].description, "words", "kept for Edit while it waits");
  sent.reply(item, { result: "created", ticket: "t", key: "T-9", column: "TODO" });
  assert.equal(item.status, "landed");
  assert.deepEqual(sent.waiting("board"), []);

  const back = sent.add("board", { title: "Back", description: "gone words", column: "TODO" }, 2, { id: "envelope-2" });
  sent.withdrawn(back);
  assert.equal(back.status, "withdrawn");
  assert.equal(back.description, "");
  assert.equal(sent.unsettled("board"), 0, "unsent is settled");
  const lost = sent.add("board", { title: "Lost", column: "TODO" }, 3, { id: "envelope-3" });
  sent.deposited(lost);
  sent.gone(lost);
  assert.equal(lost.status, "unknown");
  const reloaded = new Sent();
  reloaded.restore("board", [
    ...JSON.parse(JSON.stringify(sent.stored("board"))),
    { id: "envelope-4", title: "Sealed copy lost", status: "local", at: 4 },
  ]);
  assert.deepEqual(
    reloaded.forBoard("board").map((i) => [i.title, i.status]),
    [["Away", "landed"], ["Back", "withdrawn"], ["Lost", "unknown"], ["Sealed copy lost", "unknown"]],
  );
});

test("a landed ticket is picked up once, keeps it across a reload, and nothing else is", async () => {
  const { Sent } = await import("./sent.js");
  const sent = new Sent();
  const item = sent.add("board", { title: "From the phone", column: "TODO" }, 1, { id: "env-1" });
  const desk = { by: "desk", at: 1_790_000_000_000 };
  assert.equal(sent.pickedUp(item, desk), false, "not on the board yet");
  sent.deposited(item);
  sent.reply(item, { result: "created", ticket: "t-1", key: "T-1", column: "TODO" });
  assert.equal(sent.pickedUp(item, { by: "desk" }), false, "no time, no pickup");
  assert.equal(sent.pickedUp(item, desk), true);
  assert.equal(sent.pickedUp(item, { by: "agent", at: 2 }), false, "once: the first road wins");
  assert.deepEqual(item.picked, desk);
  const again = new Sent();
  again.restore("board", JSON.parse(JSON.stringify(sent.stored("board"))));
  assert.deepEqual(again.get("env-1").picked, desk);
  // A stored pickup on a ticket that never landed is not believed.
  const forged = new Sent();
  forged.restore("board", [{ id: "x", title: "t", status: "relay", picked: desk }]);
  assert.equal(forged.get("x").picked, undefined);
});

test("a remembered board keeps tags and since, never the agent's step or reply", () => {
  const board = new BoardState();
  board.update({
    title: "Board",
    columns: ["TODO"],
    tickets: [
      {
        id: "one",
        key: "T-1",
        title: "Ticket",
        column: "TODO",
        tags: [{ group: 1, name: "BUG", tint: 3, extra: "dropped" }],
        picked: { by: "desk", at: 5 },
        agent: { session: "s", provider: "claude", state: "working", promptable: true,
          since: 1_790_000_000_000, doing: "Bash(cat secrets)", said: "I read it." },
      },
    ],
  });
  const kept = JSON.parse(JSON.stringify(board.snapshot())).tickets[0];
  assert.deepEqual(kept.tags, [{ group: 1, name: "BUG", tint: 3 }]);
  assert.equal(kept.agent.since, 1_790_000_000_000);
  assert.equal(kept.agent.doing, undefined);
  assert.equal(kept.agent.said, undefined);
  assert.equal(JSON.stringify(kept).includes("secrets"), false);
  const restored = new BoardState();
  restored.update(JSON.parse(JSON.stringify(board.snapshot())), { cached: true });
  restored.search = "bug";
  assert.equal(restored.visible().length, 1, "a tag's name finds its ticket, remembered too");
});
test("a start goes clock to two ticks, settles once, and never crosses boards", async () => {
  const { Starts, startWaiting } = await import("./starts.js");
  const starts = new Starts();
  const item = starts.sent("board-a", "one", 7, "incarnation", "T-1");
  assert.equal(item.status, "sending");
  assert.equal(starts.get("board-b", "one"), undefined);
  starts.reply(item, { result: "delivery", status: "provisioning" });
  assert.equal(item.status, "provisioning");
  starts.reply(item, { result: "delivery", status: "starting" });
  assert(startWaiting(item));
  assert.deepEqual(starts.unresolved("board-a"), [item]);
  starts.reply(item, { result: "delivery", status: "started" });
  assert.equal(item.status, "started");
  assert.deepEqual(starts.unresolved("board-a"), []);
  // A late or lost answer cannot take two ticks back.
  starts.reply(item, { result: "delivery", status: "unknown" });
  assert.equal(item.status, "started");
  // A word the page does not know, from a newer host, is unknown, not two.
  const other = starts.sent("board-a", "two", 8, "incarnation");
  starts.reply(other, { result: "delivery", status: "submitted" });
  assert.equal(other.status, "unknown");
  const refused = starts.sent("board-a", "three", 9, "incarnation");
  starts.reply(refused, { result: "rejected", message: "this ticket already has an agent" });
  assert.deepEqual([refused.status, refused.message], ["rejected", "this ticket already has an agent"]);
  // A new press replaces what the last one came to.
  assert.equal(starts.sent("board-a", "three", 10, "incarnation").status, "sending");
  starts.sent("board-b", "one", 1, "incarnation");
  starts.purge("board-a");
  assert.equal(starts.get("board-a", "one"), undefined);
  assert.equal(starts.get("board-b", "one").command, 1);
});
test("a card edit is worn until the host answers, in its slot, and never crosses boards", async () => {
  const { Edits } = await import("./edits.js");
  const edits = new Edits();
  const card = (id, column) => ({ id, key: id.toUpperCase(), title: id, column, tags: [] });
  const host = [card("a", "TODO"), card("b", "TODO"), card("c", "DONE"), card("d", "DONE")];
  const order = (tickets) => tickets.map((t) => `${t.id}:${t.column}`);
  edits.sent(1, { board: "board-a", ticket: "a", op: "rename", patch: { title: "Renamed" } });
  edits.sent(2, { board: "board-a", ticket: "b", op: "move", patch: { column: "DONE" }, before: "d" });
  edits.sent(3, { board: "board-a", ticket: "c", op: "tag", patch: { tags: [{ group: 1, name: "BUG", tint: 0 }] } });
  edits.sent(4, { board: "board-b", ticket: "a", op: "move", patch: { column: "DONE" }, before: null });
  const worn = edits.wear("board-a", host);
  assert.deepEqual(order(worn), ["a:TODO", "c:DONE", "b:DONE", "d:DONE"]);
  assert.equal(worn[0].title, "Renamed");
  assert.deepEqual(worn[1].tags, [{ group: 1, name: "BUG", tint: 0 }]);
  assert.equal(host[0].title, "a", "the host's reply is not written over");
  // Wearing it twice lands it in the same place: a snapshot taken before
  // the host had it, and one taken after.
  assert.deepEqual(order(edits.wear("board-a", worn)), order(worn));
  assert(edits.waiting("board-a", "b", "move") && !edits.waiting("board-a", "b", "rename"));
  assert(!edits.waiting("board-b", "b"));
  // Without a card to go before, a move lands at its column's end, and an
  // empty column takes it at the end of the list.
  assert.deepEqual(order(edits.wear("board-b", host)), ["b:TODO", "c:DONE", "d:DONE", "a:DONE"]);
  const empty = new Edits();
  empty.sent(1, { board: "x", ticket: "a", op: "move", patch: { column: "REVIEW" }, before: "gone" });
  assert.deepEqual(order(empty.wear("x", host)), ["b:TODO", "c:DONE", "d:DONE", "a:REVIEW"]);
  // The answer ends it; a dropped connection or a revoke ends them all.
  assert.equal(edits.take(1).op, "rename");
  assert.equal(edits.take(1), undefined);
  assert.equal(edits.wear("board-a", host)[0].title, "a");
  edits.purge("board-a");
  assert.deepEqual([...edits.items.keys()], [4]);
  edits.lost();
  assert.equal(edits.items.size, 0);
});
test("a ticket's notes keep the bodies read at their revision, and only those", async () => {
  const { NoteBook, KEEP_TICKETS } = await import("./notes.js");
  const book = new NoteBook();
  const row = (id, rev) => ({ id, name: `note ${id}`, by: "you", at: 5, rev });
  book.listed("t", { notes: [row("d", 1), row("n", 2)], description: "the brief" }, "s1");
  assert.equal(book.body("t", "d"), "the brief", "the list carries the description");
  assert.equal(book.body("t", "n"), undefined, "another note is read alone");
  assert.equal(book.current("t", "s1"), true);
  assert.equal(book.current("t", "s2"), false, "a moved digest asks again");
  book.read("t", { note: row("n", 2), text: "second" });
  assert.equal(book.body("t", "n"), "second");
  // Rewritten elsewhere: the old body is not the note any more.
  book.listed("t", { notes: [row("d", 1), row("n", 3)] }, "s2");
  assert.equal(book.body("t", "d"), "the brief", "an unchanged body stays");
  assert.equal(book.body("t", "n"), undefined, "a rewritten one goes");
  // This browser's own write landed, then a delete.
  book.written("t", "n", { result: "note_written", note: "n", rev: 4 }, "mine", "mine");
  assert.equal(book.body("t", "n"), "mine");
  book.written("t", "n", { result: "note_written", rev: 0 }, "", "");
  assert.deepEqual(book.entry("t").rows.map((r) => r.id), ["d"]);
  // Kept across a reload, newest first and bounded; a malformed entry is dropped.
  book.entry("t").at = 1;
  for (let i = 0; i < KEEP_TICKETS + 5; i++) book.listed(`x${i}`, { notes: [row("a", 1)], description: "x" }, "s", 100 + i);
  const stored = JSON.parse(JSON.stringify(book.stored()));
  assert.equal(stored.length, KEEP_TICKETS);
  assert.equal(stored.some((s) => s.ticket === "t"), false, "the one read longest ago went");
  const back = new NoteBook();
  back.restore([...stored, { ticket: "bad", rows: [{ id: 1 }] }, null]);
  assert.equal(back.body(`x${KEEP_TICKETS + 4}`, "a"), "x");
  assert.deepEqual(back.entry("bad").rows, []);
});
test("a note edit settles once on the host's word and keeps its words until then", async () => {
  const { NoteMail } = await import("./notes.js");
  const mail = new NoteMail();
  const words = { ticket: "t", key: "T-1", note: "n", name: "n", rev: 2, text: "mine" };
  const sealed = mail.add("board", words, 1, { id: "env-1" });
  assert.equal(sealed.status, "local");
  assert.equal(mail.pending("board", "t", "n"), sealed);
  assert.equal(mail.pending("other", "t", "n"), undefined, "never across boards");
  mail.deposited(sealed);
  assert.equal(sealed.status, "relay");
  assert.equal(sealed.envelope, undefined, "the relay holds the sealed copy now");
  const stale = { id: "n", name: "n", by: "agent", at: 9, rev: 3 };
  assert.equal(mail.reply(sealed, { result: "note_stale", ticket: "t", note: stale }), "stale");
  assert.equal(mail.reply(sealed, { result: "note_written", note: "n", rev: 4 }), "stale", "settled once");
  assert.deepEqual(sealed.stale, stale);
  assert.equal(sealed.text, "mine", "the words stay for Save mine");
  const live = mail.add("board", { ...words, note: undefined, rev: undefined, text: "fresh" });
  mail.sent(live, 7, "inc");
  assert.deepEqual(mail.unresolved("board"), [live]);
  assert.deepEqual(mail.fresh("board", "t"), [live]);
  assert.equal(mail.reply(live, { result: "note_written", note: "new", rev: 1 }), "landed");
  assert.equal(live.note, "new");
  // A delete lands with no note id; the id it deleted is the caller's.
  const gone = mail.add("board", { ...words, text: "" });
  mail.sent(gone, 8, "inc");
  assert.equal(mail.reply(gone, { result: "note_written", rev: 0 }), "landed");
  assert.equal(gone.note, "n");
  // A reload keeps what is unsettled, and a local edit without its seal is unknown.
  const local = mail.add("board", words, 2, { id: "env-2" });
  const stored = JSON.parse(JSON.stringify(mail.stored("board")));
  assert.deepEqual(stored.map((s) => s.id).sort(), ["env-1", "env-2"]);
  const back = new NoteMail();
  back.restore("board", [...stored.map((s) => (s.id === local.id ? { ...s, envelope: undefined } : s)), { id: 5 }]);
  assert.equal(back.get("env-2").status, "unknown");
  assert.equal(back.get("env-1").status, "stale");
  assert.equal(back.items.length, 2);
});
test("notes read as text: fences, pictures and markers never become markup", async () => {
  const { blocks } = await import("./markdown.js");
  const out = blocks("# Plan\n\n- one\n- two\n\n```\n<script>x</script>\n```\n![shot](mesimon-attachment:01J)\n> quoted <b>");
  assert.deepEqual(out, [
    { heading: "Plan" },
    { ordered: false, items: ["one", "two"] },
    { code: true, lines: ["<script>x</script>"] },
    { picture: "Picture" },
    { text: "quoted <b>" },
  ]);
  // The desk's own picture link, alone on a line or inside one (T-629).
  const id = "01JZZZZZZZZZZZZZZZZZZZZZZZ";
  assert.deepEqual(blocks(`[Image #2](mesimon-attachment:${id})\nsee [Image #3](mesimon-attachment:${id}) here`), [
    { picture: "Image #2" },
    { text: `see [Image #3](mesimon-attachment:${id}) here` },
  ]);
  const { nameOf, ago } = await import("./notes.js");
  assert.equal(nameOf("\n\n## Plan: retry\nmore"), "Plan: retry");
  assert.equal(ago(0), "");
  assert.equal(ago(1000, 1000 + 30_000), "now");
  assert.equal(ago(1000, 1000 + 3 * 3_600_000), "3h");
});
test("a note's pictures are named in the draft and linked only once sent", async () => {
  const { unlinked, nextNumber, linked, withoutPicture, insertToken, pieceOf, PICTURE_CHUNK_BYTES } = await import("./pictures.js");
  const id = "01JZZZZZZZZZZZZZZZZZZZZZZZ";
  const text = `Before [Image #1](mesimon-attachment:${id}) then [Image #3] and [Image #2], [Image #3] again.`;
  assert.deepEqual(unlinked(text), [3, 2]);
  assert.equal(nextNumber(text), 4);
  assert.equal(nextNumber("none", [7]), 8);
  assert.equal(
    linked(text, new Map([[3, "A"]])),
    `Before [Image #1](mesimon-attachment:${id}) then [Image #3](mesimon-attachment:A) and [Image #2], [Image #3](mesimon-attachment:A) again.`,
  );
  assert.equal(withoutPicture(text, 3), `Before [Image #1](mesimon-attachment:${id}) then and [Image #2], again.`);
  assert.equal(withoutPicture(text, 1), text, "a sent picture's link stays");
  assert.deepEqual(insertToken("ab", 1, 4), { text: "a [Image #4] b", at: 13 });
  assert.deepEqual(insertToken("", undefined, 1), { text: "[Image #1]", at: 10 });
  assert.deepEqual(insertToken("line\n", 5, 2), { text: "line\n[Image #2]", at: 15 });
  const bytes = Uint8Array.from({ length: PICTURE_CHUNK_BYTES + 5 }, (_, i) => i % 251);
  const first = pieceOf(bytes, 0);
  assert.equal(first.end, PICTURE_CHUNK_BYTES);
  assert.deepEqual(Uint8Array.from(Buffer.from(first.data, "base64")), bytes.subarray(0, PICTURE_CHUNK_BYTES));
  const last = pieceOf(bytes, first.end);
  assert.deepEqual([last.end, Buffer.from(last.data, "base64").length], [bytes.length, 5]);
});
test("a remembered board keeps each ticket's note count and digest", () => {
  const board = new BoardState();
  board.update({ title: "B", columns: ["TODO"], tickets: [{ ...ticket("one"), notes: 2, noted: "abc" }, ticket("two")] });
  const [one, two] = JSON.parse(JSON.stringify(board.snapshot())).tickets;
  assert.deepEqual([one.notes, one.noted, two.notes, two.noted], [2, "abc", 0, ""]);
});

// T-626: the conversation's pages join by byte offset.
test("transcript pages join: what was written since appends, an earlier page goes on top", () => {
  const row = (at, text = `r${at}`) => ({ at, kind: "reply", text });
  let chat = mergePage(undefined, {}, { conversation: "a", rows: [row(40), row(50)], from: 40, end: 60, next_before: 40 });
  assert.deepEqual([chat.floor, chat.end, chat.rows.length], [40, 60, 2]);
  assert.deepEqual(tailAsk(chat), { after: 60, conversation: "a" });
  assert.deepEqual(tailAsk(undefined), {});
  // Nothing new: the page stays as it was.
  assert.deepEqual(mergePage(chat, tailAsk(chat), { conversation: "a", rows: [], from: 60, end: 60 }).rows, chat.rows);
  // New rows append, and only rows past `from` are replaced.
  chat = mergePage(chat, tailAsk(chat), { conversation: "a", rows: [row(60), row(70)], from: 60, end: 80 });
  assert.deepEqual(chat.rows.map((r) => r.at), [40, 50, 60, 70]);
  const replaced = mergePage(chat, { after: 50, conversation: "a" }, { conversation: "a", rows: [row(50, "again")], from: 50, end: 80 });
  assert.deepEqual(replaced.rows.map((r) => r.text), ["r40", "again"]);
  // The page before joins on top where the held part begins; one that does
  // not end there is not this conversation's, and is dropped.
  chat = mergePage(chat, { before: 40 }, { conversation: "a", rows: [row(10), row(20)], from: 10, end: 40, next_before: 10 });
  assert.deepEqual([chat.floor, chat.rows.map((r) => r.at)], [10, [10, 20, 40, 50, 60, 70]]);
  assert.equal(mergePage(chat, { before: 10 }, { conversation: "a", rows: [row(0)], from: 0, end: 5 }), chat);
  chat = mergePage(chat, { before: 10 }, { conversation: "a", rows: [row(0)], from: 0, end: 10 });
  assert.equal(chat.floor, null, "the file's start");
  // More written than a page holds: a gap, so the tail starts over.
  const gap = mergePage(chat, tailAsk(chat), { conversation: "a", rows: [row(900)], from: 900, end: 910, next_before: 900 });
  assert.deepEqual([gap.rows.length, gap.floor], [1, 900]);
  // A new conversation (`/clear`, `/resume`) starts over too.
  const fresh = mergePage(chat, tailAsk(chat), { conversation: "b", rows: [row(0)], from: 0, end: 10 });
  assert.deepEqual([fresh.conversation, fresh.rows.length, fresh.floor], ["b", 1, null]);
});
