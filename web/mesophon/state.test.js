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
