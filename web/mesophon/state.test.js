import { test } from "node:test";
import assert from "node:assert/strict";
import { Sessions } from "./sessions.js";
import { BoardState } from "./board.js";
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
  assert.equal(entry.receipt.unresolved, false);
  assert.equal(entry.draft, "later follow-up");
  assert.match(entry.delivery, /Decision sent/);
  sessions.sent(entry, 8, "host", "dialog", "");
  sessions.lost();
  assert.match(entry.delivery, /unknown/);
  assert.equal(entry.draft, "later follow-up");
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
