// Real browsers, controlled M1 wire replies. Crypto/relay acceptance remains in
// browser.test.js; this fixture exercises timing/failure states deterministically.
import { chromium, webkit } from "playwright";
import assert from "node:assert/strict";
import http from "node:http";
import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import zlib from "node:zlib";
const root = path.dirname(fileURLToPath(import.meta.url));
// No network at all while set: every request's connection is dropped, as a
// phone with no signal sees it, service worker's requests included.
let networkDown = false;
const types = {
  html: "text/html",
  css: "text/css",
  js: "text/javascript",
  woff2: "font/woff2",
  webmanifest: "application/manifest+json",
  png: "image/png",
  wasm: "application/wasm",
};
const server = http.createServer(async (req, res) => {
  if (networkDown) {
    req.socket.destroy();
    return;
  }
  try {
    const name = new URL(req.url, "http://fixture").pathname.slice(1) || "index.html";
    const match = /^(?:vendor\/|fonts\/|icons\/|pkg\/)?[a-z0-9._-]+\.(html|css|js|woff2|webmanifest|png|wasm)$/.exec(name);
    if (!match) {
      res.writeHead(404).end();
      return;
    }
    res.setHeader("Content-Type", types[match[1]]);
    // The fixture's crypto stands in for the generated Wasm glue, served as
    // the relay would serve the real one, so a service worker can keep it.
    if (name === "pkg/mesimon_web.js") res.end(fakeCrypto);
    else if (name === "pkg/mesimon_web_bg.wasm") res.end("wasm fixture");
    else res.end(await fs.readFile(path.join(root, name)));
  } catch {
    res.writeHead(404).end();
  }
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
await fs.mkdir(path.join(root, "test-results"), { recursive: true });
const fakeCrypto = `export default async function init() {};
export class Browser {
  seed() { return 'test-seed'; } free() {}
  auth() { return JSON.stringify({kind:'auth'}); }
  pair() { return JSON.stringify({kind:'pair'}); }
  connect() { return JSON.stringify({kind:'connect'}); }
  accept() { return JSON.stringify({reply:{result:'ready',incarnation:window.fixture.incarnation,next:window.fixture.next,features:window.fixture.features}}); }
  packet(text) { return text; } open(text) { return text; }
  mail(pin, body) { return JSON.stringify({ id: crypto.randomUUID(), body: JSON.parse(body) }); }
  receipt(pin, receipt) {
    const r = JSON.parse(receipt);
    if (!r.sealed) throw new Error('not the host');
    return JSON.stringify(r.answer);
  }
}`;
function fixture() {
  const state = (window.fixture = {
    next: 1,
    features: ["permission", "dialog", "awareness", "create", "mailbox"],
    incarnation: "incarnation-a",
    prompts: [],
    requests: [],
    uploads: {},
    creates: [],
    answers: {},
    disposition: "submitted",
    createDisposition: "created",
    // A start (T-498): answered `starting` and the agent put on the ticket,
    // `rejected`, or held unanswered; `run` makes its receipt `started`.
    starts: [],
    startCommands: [],
    startDisposition: "starting",
    // The card edits (T-530): applied and answered `edited`, `rejected`
    // with `editRefusal`, or held unanswered.
    edits: [],
    editDisposition: "edited",
    editRefusal: "worktree unmerged — merge before DONE",
    refuse: false,
    sockets: [],
    // The relay's mailbox, kept across reloads the way a relay would be.
    mail: JSON.parse(localStorage.getItem("fixture-mail") || "{}"),
    deposits: [],
    mailRefusal: "",
    relayDown: localStorage.getItem("fixture-relay-down") === "1",
    saveMail() {
      localStorage.setItem("fixture-mail", JSON.stringify(this.mail));
    },
    channel() {
      return this.sockets.filter((s) => s.channel).at(-1);
    },
    mailboxes() {
      return this.sockets.filter((s) => s.mailbox && s.readyState === 1);
    },
    // The host files what waits, as it does when it comes back.
    collect() {
      for (const [id, m] of Object.entries(this.mail)) if (!m.answer) this.file(id, m);
      this.saveMail();
    },
    file(id, m) {
      const body = m.envelope.body;
      // A note's letter (T-532) is written to the note, not filed.
      if (body.kind === "note") {
        this.noteWrites.push(body);
        m.answer = this.writeNote(body);
        m.sent = true;
        for (const socket of this.mailboxes())
          socket.message({ kind: "receipt", board: "board-a", receipt: { id, sealed: true, answer: m.answer } });
        return;
      }
      const n = Object.values(this.mail).filter((x) => x.answer && x.answer.result === "created").length + 1;
      const ticket = { id: `mailed-${n}`, key: `T-${199 + n}`, title: body.title, column: body.column || "TODO", agent: null };
      this.tickets.push(ticket);
      m.answer = { result: "created", ticket: ticket.id, key: ticket.key, column: ticket.column };
      m.sent = true;
      const receipt = { id, sealed: true, answer: m.answer };
      for (const socket of this.mailboxes()) socket.message({ kind: "receipt", board: "board-a", receipt });
    },
    // A ticket's notes (T-532), by ticket id, each with its body; what the
    // page wrote, and what it told an agent.
    notes: {},
    noteWrites: [],
    told: [],
    noteRow(n) {
      return { id: n.id, name: n.name, by: n.by, at: n.at, rev: n.rev };
    },
    stampNotes() {
      for (const t of this.tickets) {
        const list = this.notes[t.id] || [];
        t.notes = list.length;
        t.noted = list.map((n) => `${n.id}.${n.rev}`).join(",");
      }
    },
    // The host's write_note: a stale revision is answered with the note.
    writeNote(r, by = "My browser") {
      const list = (this.notes[r.ticket] ||= []);
      const note = r.note && list.find((n) => n.id === r.note);
      const blank = !r.text.trim();
      if (r.note && !note)
        return blank ? { result: "note_written", ticket: r.ticket, rev: 0 } : { result: "rejected", message: "the note is gone" };
      if (note && r.rev !== undefined && r.rev !== note.rev)
        return { result: "note_stale", ticket: r.ticket, note: this.noteRow(note) };
      if (note && blank) {
        list.splice(list.indexOf(note), 1);
        this.stampNotes();
        return { result: "note_written", ticket: r.ticket, rev: 0 };
      }
      let target = note;
      if (!target) list.push((target = { id: `note-${Date.now()}-${list.length}`, rev: 0 }));
      const name = r.text.split("\n").map((l) => l.replace(/^#+\s*/, "").trim()).find(Boolean) || "";
      Object.assign(target, { name, by, at: Date.now(), rev: target.rev + 1, text: r.text });
      this.stampNotes();
      return { result: "note_written", ticket: r.ticket, note: target.id, rev: target.rev };
    },
    lines: Array.from(
      { length: 50 },
      (_, i) =>
        i === 10
          ? "─".repeat(132)
          : `line ${String(i).padStart(2, "0")} · actual-sized periodic output <script>never execute</script>`,
    ),
    tickets: Array.from({ length: 36 }, (_, i) => ({
      id: `ticket-${i}`,
      key: `T-${i}`,
      title:
        i === 0
          ? "Make remote control comfortable to use"
          : i === 2
            ? "A very long title ".repeat(15)
            : `Agent task ${i}`,
      column: i % 2 ? "TODO" : "IN PROGRESS",
      agent:
        i === 3
          ? null
          : {
              session: `session-${i}`,
              provider: i % 2 ? "codex" : "claude",
              state: i === 1 ? "needs attention" : i === 4 ? "idle" : "working",
              promptable: true,
            },
    })),
    // A transcript as the host reads it (T-626): rows oldest first, `at`
    // ten bytes apart, eight to a page; held asks wait for `releaseTranscript`.
    transcript: [],
    conversation: "conversation-a",
    transcriptAsks: [],
    transcriptHold: false,
    heldTranscript: [],
    transcriptPage(r) {
      const end = this.transcript.length ? this.transcript.at(-1).at + 10 : 0;
      const top = r.before ?? end;
      const floor = r.conversation === this.conversation && r.after != null && r.after <= top ? r.after : 0;
      const within = this.transcript.filter((row) => row.at < top && row.at >= floor);
      const rows = within.slice(-8);
      const from = within.length > 8 ? rows[0].at : floor;
      return { result: "transcript", conversation: this.conversation, rows, from, end: top,
        ...(from > 0 ? { next_before: from } : {}) };
    },
    releaseTranscript() {
      for (const release of this.heldTranscript.splice(0)) release();
    },
    snapshot() {
      return {
        result: "board",
        title: "Mesimon",
        columns: ["TODO", "IN PROGRESS", "DONE"],
        tickets: this.tickets,
        default_column: "TODO",
        column_descriptions: { TODO: "for work that can and should be done soon" },
        allowed_tags: [
          { group: 1, name: "BUG", tint: 0 },
          { group: 1, name: "FEATURE", tint: 6 },
          { group: 2, name: "QUESTION", tint: 9 },
        ],
      };
    },
    reply(reply, id = 0) {
      this.channel().message({ kind: "packet", id, reply });
    },
    // The host puts the agent on the ticket, then it takes its first prompt.
    begin(ticketId, command) {
      const ticket = this.tickets.find((t) => t.id === ticketId);
      ticket.agent = { session: `started-${command}`, provider: "claude", state: "starting", promptable: true };
      this.answers[command] = { result: "delivery", status: "starting" };
      this.update();
    },
    run(ticketId) {
      const ticket = this.tickets.find((t) => t.id === ticketId);
      ticket.agent.state = "working";
      this.answers[Number(ticket.agent.session.slice("started-".length))] = { result: "delivery", status: "started" };
      this.update();
    },
    update() {
      this.reply(this.snapshot());
    },
    // The host's rename, move, tag or workspace, as the daemon applies it.
    applyEdit(request) {
      const at = this.tickets.findIndex((t) => t.id === request.ticket);
      const ticket = this.tickets[at];
      if (request.op === "rename") ticket.title = request.title;
      if (request.op === "workspace")
        ticket.workspace = request.worktree
          ? { kind: "worktree", open: true, state: "planned" }
          : { kind: "shared", open: true };
      if (request.op === "tag") {
        const tags = (ticket.tags || []).filter((t) => t.group !== request.group);
        const tag = this.snapshot().allowed_tags.find((t) => t.group === request.group && t.name === request.name);
        if (tag) tags.push(tag);
        ticket.tags = tags.sort((a, b) => a.group - b.group);
      }
      if (request.op === "move") {
        this.tickets.splice(at, 1);
        ticket.column = request.column;
        let to = request.before ? this.tickets.findIndex((t) => t.id === request.before && t.column === ticket.column) : -1;
        if (to < 0) to = this.tickets.findLastIndex((t) => t.column === ticket.column) + 1 || this.tickets.length;
        this.tickets.splice(to, 0, ticket);
      }
    },
  });
  class Socket {
    static OPEN = 1;
    constructor() {
      this.readyState = 0;
      state.sockets.push(this);
      setTimeout(() => {
        if (state.relayDown) {
          this.readyState = 3;
          this.onclose?.();
          return;
        }
        this.readyState = 1;
        this.onopen?.();
      }, 0);
    }
    message(wire) {
      if (this.readyState === 1)
        this.onmessage?.({ data: JSON.stringify(wire) });
    }
    send(text) {
      const wire = JSON.parse(text);
      queueMicrotask(() => {
        if (wire.kind === "auth")
          this.message({ kind: "authenticated", credential: "fixture" });
        // The relay's generic answer while the host is away.
        else if (wire.kind === "connect" && state.refuse)
          this.message({ kind: "error", code: "unavailable" });
        else if (["pair", "connect"].includes(wire.kind)) {
          this.channel = true;
          this.message({ kind: "welcome", welcome: { board: "board-a" } });
        } else if (["deposit", "withdraw", "sync"].includes(wire.kind)) {
          this.mailbox = true;
          if (wire.kind === "deposit") {
            const { envelope } = wire;
            state.deposits.push(envelope.body);
            if (state.mailRefusal) {
              this.message({ kind: "refused", id: envelope.id, code: state.mailRefusal });
              return;
            }
            const m = (state.mail[envelope.id] ||= { envelope });
            this.message({ kind: "deposited", id: envelope.id });
            if (m.answer) this.message({ kind: "receipt", board: "board-a", receipt: { id: envelope.id, sealed: true, answer: m.answer } });
            else if (!state.refuse) state.file(envelope.id, m);
            state.saveMail();
          } else if (wire.kind === "withdraw") {
            const m = state.mail[wire.id];
            const removed = !!m && !m.sent && !m.answer;
            if (removed) delete state.mail[wire.id];
            state.saveMail();
            this.message({ kind: "withdrawn", id: wire.id, removed });
          } else {
            const items = wire.ids.map((id) => {
              const m = state.mail[id];
              return !m
                ? { id, stage: "gone" }
                : m.answer
                  ? { id, stage: "answered", receipt: { id, sealed: true, answer: m.answer } }
                  : { id, stage: m.sent ? "sent" : "waiting" };
            });
            this.message({ kind: "mailbox", board: wire.board, items });
          }
        } else {
          const { id, request } = wire;
          state.next = id + 1;
          state.requests.push(request);
          const answer = (reply) => {
            state.answers[id] = reply;
            this.message({ kind: "packet", id, reply });
          };
          if (state.holdAll) return;
          if (request.op === "foreground") answer({ result: "delivery", status: "observed" });
          if (request.op === "permission") answer({ result: "delivery", status: "decision_sent" });
          if (request.op === "dialog") answer(state.dialogReply || { result: "delivery", status: "input_sent" });
          if (request.op === "snapshot") answer(state.snapshot());
          if (request.op === "preview")
            answer({ result: "preview", lines: state.lines, cols: 132 });
          if (request.op === "transcript") {
            state.transcriptAsks.push(request);
            const reply = () => answer(state.transcriptPage(request));
            if (state.transcriptHold) state.heldTranscript.push(reply);
            else reply();
          }
          if (request.op === "prompt") {
            state.prompts.push(request);
            if (state.disposition === "disconnect") this.close();
            else if (state.disposition === "rejected")
              answer({
                result: "rejected",
                message: "Session is no longer promptable",
              });
            else if (state.disposition !== "hold") {
              if (state.disposition === "queued")
                state.tickets.find((t) => t.id === request.ticket).queued =
                  request.text;
              answer({ result: "delivery", status: state.disposition,
                ...(state.replaced ? { replaced: state.replaced } : {}) });
            }
          }
          if (["send_now", "take_back"].includes(request.op)) {
            const ticket = state.tickets.find(
              (t) =>
                t.id === request.ticket && t.agent?.session === request.session,
            );
            if (ticket?.queued == null)
              answer({
                result: "rejected",
                message: "nothing queued on this session",
              });
            else {
              const text = ticket.queued;
              ticket.queued = null;
              if (request.op === "send_now")
                answer({ result: "delivery", status: "submitted" });
              else if (state.holdTakeBack)
                state.releaseTakeBack = () =>
                  answer({ result: "taken_back", text });
              else answer({ result: "taken_back", text });
            }
          }
          if (request.op === "create") {
            state.creates.push(request);
            const created = () => {
              const n = state.creates.length;
              const ticket = { id: `new-${n}`, key: `T-${99 + n}`, title: request.title,
                column: request.column || "TODO", agent: null };
              state.tickets.push(ticket);
              return { result: "created", ticket: ticket.id, key: ticket.key, column: ticket.column };
            };
            if (state.createDisposition === "rejected")
              answer({ result: "rejected", message: "no such column: GONE" });
            else if (state.createDisposition === "disconnect") {
              // The host filed it; only the answer is lost with the socket.
              state.answers[id] = created();
              this.close();
            } else if (state.createDisposition !== "hold") answer(created());
          }
          if (request.op === "start") {
            state.starts.push(request);
            state.startCommands.push(id);
            if (state.startDisposition === "rejected")
              answer({ result: "rejected", message: "this ticket already has an agent" });
            else if (state.startDisposition !== "hold") {
              answer({ result: "delivery", status: "starting" });
              state.begin(request.ticket, id);
            }
          }
          if (["rename", "move", "tag", "workspace"].includes(request.op)) {
            state.edits.push(request);
            if (state.editDisposition === "rejected") answer({ result: "rejected", message: state.editRefusal });
            else if (state.editDisposition !== "hold") {
              state.applyEdit(request);
              answer({ result: "edited", ticket: request.ticket });
            }
          }
          if (request.op === "notes") {
            const list = state.notes[request.ticket] || [];
            answer({ result: "notes", ticket: request.ticket, notes: list.map((n) => state.noteRow(n)),
              ...(list[0] ? { description: list[0].text } : {}) });
          }
          if (request.op === "note") {
            const n = (state.notes[request.ticket] || []).find((x) => x.id === request.note);
            answer(n ? { result: "note", ticket: request.ticket, note: state.noteRow(n), text: n.text }
              : { result: "rejected", message: "the note is gone" });
          }
          if (request.op === "write_note") {
            state.noteWrites.push(request);
            answer(state.writeNote(request));
          }
          // The host's upload (T-629): pieces in order, each answered
          // with the upload it belongs to.
          if (request.op === "upload") {
            const id = request.upload || `pic-${Object.keys(state.uploads).length}`;
            const u = (state.uploads[id] ||= { bytes: 0, pieces: 0, complete: false });
            if (request.offset !== u.bytes || u.complete)
              answer({ result: "rejected", message: "attachment upload offset mismatch" });
            else {
              Object.assign(u, { bytes: u.bytes + atob(request.data).length, pieces: u.pieces + 1, complete: request.complete });
              answer({ result: "uploaded", upload: id });
            }
          }
          if (request.op === "tell_agent") {
            state.told.push(request);
            answer({ result: "delivery", status: "submitted" });
          }
          if (request.op === "status")
            answer(
              state.receipt
                ? { result: "delivery", status: state.receipt }
                : state.answers[request.command] || {
                    result: "delivery",
                    status: "unknown",
                  },
            );
        }
      });
    }
    close() {
      if (this.readyState === 3) return;
      this.readyState = 3;
      queueMicrotask(() => this.onclose?.());
    }
  }
  window.WebSocket = Socket;
}
// Poll until the page says yes. `page.evaluate` awaits a promise the
// predicate returns; `waitForFunction` does not (a pending Promise is
// truthy), which made every "until it is on disk" wait a no-op (T-497). A
// poll that throws, or that a reload cut short, is a no for now.
// A PNG of noise, which no encoder shrinks: a picture several upload
// pieces long once the page has made it its own PNG again.
function noisePng(width, height) {
  const chunk = (type, data) => {
    const body = Buffer.concat([Buffer.from(type), data]);
    const out = Buffer.alloc(12 + data.length);
    out.writeUInt32BE(data.length, 0);
    body.copy(out, 4);
    out.writeUInt32BE(zlib.crc32(body), 8 + data.length);
    return out;
  };
  const head = Buffer.alloc(13);
  head.writeUInt32BE(width, 0);
  head.writeUInt32BE(height, 4);
  head.set([8, 2, 0, 0, 0], 8);
  const rows = Buffer.alloc((width * 3 + 1) * height);
  let seed = 7;
  for (let i = 0; i < rows.length; i++) rows[i] = i % (width * 3 + 1) === 0 ? 0 : (seed = (Math.imul(seed, 1103515245) + 12345) >>> 0) >>> 24;
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", head),
    chunk("IDAT", zlib.deflateSync(rows)),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

async function until(page, fn, arg, timeout = 30000) {
  const deadline = Date.now() + timeout;
  let last;
  for (;;) {
    try {
      if (await page.evaluate(fn, arg)) return;
    } catch (error) {
      last = error;
    }
    if (Date.now() > deadline)
      throw new Error(`timed out waiting for ${String(fn).slice(0, 160)}${last ? `: ${last.message}` : ""}`);
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
}

// A finger's swipe as the mouse spells it (T-628): press in the middle,
// slide, let go.
async function swipe(page, locator, dx, dy = 0) {
  const box = await locator.boundingBox();
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + dx, y + dy, { steps: 8 });
  await page.mouse.up();
}

// Tickets from this browser (T-497): the sheet and Sent over a host that
// keeps mail, live and away; the clock while this browser is offline; a
// reload with tickets on their way; unsend and edit; a forged receipt; a
// refused deposit; an older host's create op; revocation.
async function ticketFlow(browser, engineName, size, viewport) {
  const context = await browser.newContext({ viewport, colorScheme: "dark", reducedMotion: "reduce", serviceWorkers: "block" });
  await context.addInitScript(fixture);
  await context.route("**/pkg/mesimon_web.js", (route) =>
    route.fulfill({ contentType: "text/javascript", body: fakeCrypto }),
  );
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(String(error)));
  const sheet = page.locator("#new-ticket-sheet");
  const mode = (name) => page.locator(`button[data-mode="${name}"]`).locator("visible=true").click();
  const openSheet = () => page.locator(size === "phone" ? "#new-ticket-fab" : "#new-ticket").click();
  const connected = () =>
    until(page, () => document.querySelector("#connection").textContent === "Connected");
  const status = (name) => page.locator(`.sent-item[data-status="${name}"]`);
  const count = (name, n) =>
    until(page, ([name, n]) => document.querySelectorAll(`.sent-item[data-status="${name}"]`).length === n, [name, n]);
  const overview = async () => {
    if (size === "phone" && (await page.locator("#back").isVisible())) await page.locator("#back").click();
  };
  const shot = (name) =>
    page.screenshot({ path: path.join(root, "test-results", `${engineName}-${size}-${name}.png`) });
  const theme = (name) =>
    page.evaluate((name) => {
      const control = document.querySelector("#theme");
      control.value = name;
      control.dispatchEvent(new Event("change"));
    }, name);
  const quick = async (title) => {
    await page.locator("#quick-title").fill(title);
    await page.locator("#quick-send").click();
  };
  try {
    await page.goto(origin);
    await until(page, () => !document.querySelector("#pair").disabled);
    await page.getByLabel("Pairing code", { exact: true }).fill("fixture-pair-code");
    await page.getByRole("button", { name: "Connect", exact: true }).click();
    await page.locator('.ticket[data-id="ticket-0"]').waitFor();

    // Live, with a host that keeps mail: sealed, handed to the relay, and
    // answered by the host's own receipt, so two ticks at once.
    await openSheet();
    await sheet.waitFor({ state: "visible" });
    assert.equal(await page.evaluate(() => document.activeElement.id), "new-title");
    assert.match(await sheet.locator(".compose-dest").textContent(), /lands right away/);
    assert(await page.locator("#send-ticket").isDisabled());
    await page.locator("#new-title").fill("Phone ticket <b>stays text</b>");
    await page.locator("#new-description").fill("Line one\nLine two");
    await sheet.getByRole("radio", { name: "IN PROGRESS", exact: true }).check();
    assert(!(await sheet.locator(".field-note").isVisible().catch(() => false)));
    await sheet.getByRole("radio", { name: "TODO", exact: true }).check();
    assert.match(await sheet.locator(".field-note").textContent(), /done soon/);
    for (const name of ["BUG", "FEATURE", "QUESTION"])
      await sheet.getByRole("button", { name, exact: true }).click();
    assert.deepEqual(await sheet.locator(".tag-chip[aria-pressed=true]").allTextContents(), ["FEATURE", "QUESTION"]);
    if (size === "phone") {
      const short = await sheet.evaluate((node) =>
        [...node.querySelectorAll("button, input, textarea")]
          .filter((n) => n.getClientRects().length && n.type !== "radio" && n.getBoundingClientRect().height < 44)
          .map((n) => n.id || n.textContent),
      );
      assert.deepEqual(short, []);
    }
    for (const name of ["graphite", "chalk"]) {
      await theme(name);
      await shot(`sheet-${name}`);
    }
    await theme("graphite");
    await page.locator("#send-ticket").click();
    await sheet.waitFor({ state: "hidden" });
    await until(page, () => document.querySelector("#toast").textContent.includes("Landed as T-200 in TODO"));
    const sealed = await page.evaluate(() => fixture.deposits.at(-1));
    assert.equal(typeof sealed.written_at, "number");
    delete sealed.written_at;
    assert.deepEqual(sealed, {
      title: "Phone ticket <b>stays text</b>",
      description: "Line one\nLine two",
      column: "TODO",
      tags: [
        { group: 1, name: "FEATURE" },
        { group: 2, name: "QUESTION" },
      ],
    });
    assert.equal(await page.evaluate(() => fixture.creates.length), 0, "a mailbox host needs no create op");
    await mode("sent");
    await count("landed", 1);
    assert.match(await status("landed").first().textContent(), /Landed as T-200 in TODO/);
    assert.equal(await page.locator(".sent-feed b").count(), 0);
    // Picked up at the desk (T-497): the host's board says so, and the
    // ticks turn teal, with when and how.
    assert.equal(await page.locator(".sent-item[data-picked]").count(), 0);
    await page.evaluate(() => {
      fixture.tickets.find((t) => t.id === "mailed-1").picked = { by: "desk", at: Date.now() };
      fixture.update();
    });
    await page.locator('.sent-item[data-picked="desk"]').waitFor();
    assert.match(await page.locator('.sent-item[data-picked="desk"]').textContent(), /Opened at your desk · \d/);
    assert.equal(await page.locator('.sent-item[data-picked="desk"] .sent-tick .tick-picked').count(), 1);
    await shot("sent-picked");
    await status("landed").first().getByRole("button", { name: /Open/ }).click();
    await until(page, () => document.querySelector("#detail .selection-key")?.textContent === "T-200");
    await overview();
    await mode("board");
    if (size === "phone") await page.locator('[data-column="TODO"]').click();
    await page.locator('.ticket[data-id="mailed-1"] .from-here').waitFor();
    if (size !== "phone") {
      await page.locator('.add-to-column[data-column="DONE"]').click();
      await sheet.waitFor({ state: "visible" });
      assert(await sheet.locator('input[name="new-column"][value="DONE"]').isChecked());
      await sheet.getByRole("button", { name: "Cancel", exact: true }).click();
      await sheet.waitFor({ state: "hidden" });
    }
    await mode("sent");
    await page.locator("#quick-column").selectOption("IN PROGRESS");
    await page.locator("#quick-title").fill("Quick one");
    await page.locator("#quick-title").press("Enter");
    await count("landed", 2);
    assert.equal(await page.evaluate(() => fixture.deposits.at(-1).column), "IN PROGRESS");
    assert.equal(await page.locator("#quick-title").inputValue(), "");

    // The terminal away: a ticket still goes, sealed, and waits at the relay
    // with one tick, where it can be taken back or edited.
    await page.evaluate(() => {
      fixture.refuse = true;
      fixture.channel().close();
    });
    await until(page, () => document.querySelector("#shell").dataset.link === "asleep");
    // The retry loop holds one line and a spinner through a refused retry,
    // never a "Disconnected" between attempts (T-639).
    const strip = await page.evaluate(
      () =>
        new Promise((done) => {
          const seen = new Set();
          const sample = setInterval(() => {
            const line = document.querySelector("#connection");
            seen.add(`${line.textContent}|${!!line.querySelector(".spin")}`);
          }, 50);
          setTimeout(() => (clearInterval(sample), done([...seen])), 3600);
        }),
    );
    assert.deepEqual(strip, ["Reconnecting… Last received view is stale.|true"]);
    await page.locator("#quick-more").click();
    await sheet.waitFor({ state: "visible" });
    assert.match(await sheet.locator(".compose-dest").textContent(), /waits at the relay/);
    await page.locator("#new-title").fill("Written while away");
    assert(!(await page.locator("#send-ticket").isDisabled()));
    await page.locator("#send-ticket").click();
    await sheet.waitFor({ state: "hidden" });
    await count("relay", 1);
    assert.match(await status("relay").first().textContent(), /Sealed at the relay/);
    await quick("Take this back");
    await quick("Edit this one");
    await count("relay", 3);
    await status("relay").filter({ hasText: "Take this back" }).getByRole("button", { name: "Unsend" }).click();
    await page.locator(".sent-gone").filter({ hasText: "Take this back" }).waitFor();
    assert.equal(await page.evaluate(() => Object.values(fixture.mail).some((m) => m.envelope.body.title === "Take this back")), false);
    await status("relay").filter({ hasText: "Edit this one" }).getByRole("button", { name: "Edit" }).click();
    await sheet.waitFor({ state: "visible" });
    assert.equal(await sheet.locator("#new-ticket-heading").textContent(), "Edit ticket");
    assert.equal(await page.locator("#new-title").inputValue(), "Edit this one");
    await page.locator("#new-title").fill("Edited while away");
    await page.locator("#send-ticket").click();
    await sheet.waitFor({ state: "hidden" });
    await count("relay", 2);
    assert.equal(await page.locator(".sent-gone").filter({ hasText: "Edit this one" }).count(), 0, "an edit replaces");
    for (const name of ["graphite", "chalk"]) {
      await theme(name);
      await shot(`sent-away-${name}`);
    }
    await theme("graphite");
    // Now and the board show what is on its way, as the same ghost card.
    await mode("agents");
    await page.locator(".ticket.card.ghost").first().waitFor();
    assert.equal(await page.locator(".ticket.card.ghost").count(), 2);
    await mode("board");
    // Both went to IN PROGRESS: the bar keeps the column the last one used.
    if (size === "phone") await page.locator('[data-column="IN PROGRESS"]').click();
    assert.equal(await page.locator(".ticket.card.ghost").count(), 2);
    await shot("board-ghosts");
    // A receipt the relay forges does not count: only the host's seal does.
    await page.evaluate(() => {
      const [id] = Object.keys(fixture.mail).filter((id) => !fixture.mail[id].answer);
      for (const socket of fixture.mailboxes())
        socket.message({ kind: "receipt", board: "board-a", receipt: { id, sealed: false, answer: { result: "created", ticket: "forged", key: "T-666", column: "TODO" } } });
    });
    await mode("sent");
    await count("relay", 2);
    assert.equal(await page.locator(".sent-feed").textContent().then((t) => t.includes("T-666")), false);

    // A reload keeps what is on its way; the relay still has it.
    await page.reload();
    if (size === "phone") await overview();
    await mode("sent");
    await count("relay", 2);
    assert.match(await page.locator(".sent-feed").textContent(), /Phone ticket <b>stays text<\/b>/);
    assert.doesNotMatch(await page.locator(".sent-feed").textContent(), /Line one/, "a landed ticket's words are not kept");
    await page.locator('.sent-item[data-picked="desk"]').waitFor();

    // The terminal back: it files what waited, and the ticks turn to two.
    await page.evaluate(() => {
      fixture.refuse = false;
      fixture.collect();
    });
    await count("landed", 4);
    await count("relay", 0);
    await connected();

    // This browser offline: the ticket stays here, a clock, until it is back.
    await context.setOffline(true);
    await page.evaluate(() => {
      for (const socket of fixture.sockets) socket.close();
    });
    await until(page, () => document.querySelector("#shell").dataset.link === "nonet");
    await page.locator("#quick-more").click();
    await sheet.waitFor({ state: "visible" });
    assert.match(await sheet.locator(".compose-dest").textContent(), /stays in this browser/);
    await page.locator("#new-title").fill("Written with no signal");
    await page.locator("#send-ticket").click();
    await sheet.waitFor({ state: "hidden" });
    await count("local", 1);
    assert.match(await status("local").first().textContent(), /In this browser/);
    await context.setOffline(false);
    await count("landed", 5);
    await connected();

    // The relay unreachable, across a reload: the sealed ticket waits in
    // this browser and goes out when the relay answers.
    await page.evaluate(() => {
      localStorage.setItem("fixture-relay-down", "1");
      fixture.relayDown = true;
      for (const socket of fixture.sockets) socket.close();
    });
    await until(page, () => document.querySelector("#shell").dataset.link === "relay");
    await page.locator("#quick-more").click();
    await sheet.waitFor({ state: "visible" });
    assert.match(await sheet.locator(".compose-dest").textContent(), /relay is out of reach/);
    await page.locator("#new-title").fill("Written with the relay away");
    await page.locator("#send-ticket").click();
    await sheet.waitFor({ state: "hidden" });
    await count("local", 1);
    // Reload once it is on disk, as a person would, never mid-write.
    await until(
      page,
      () =>
        new Promise((resolve) => {
          const open = indexedDB.open("mesophon", 1);
          open.onsuccess = () => {
            const get = open.result.transaction("device").objectStore("device").get("sent:board-a");
            get.onsuccess = () =>
              resolve((get.result?.items || []).some((i) => i.status === "local" && i.envelope));
          };
        }),
    );
    await page.reload();
    if (size === "phone") await overview();
    await mode("sent");
    await count("local", 1);
    await page.evaluate(() => {
      localStorage.removeItem("fixture-relay-down");
      fixture.relayDown = false;
    });
    await count("landed", 6);
    await connected();

    // The relay refuses: the words come back to the sheet.
    await page.evaluate(() => {
      fixture.mailRefusal = "capacity";
    });
    await quick("Refused one");
    await count("rejected", 1);
    assert.match(await status("rejected").textContent(), /too many tickets are waiting at the relay/);
    await page.evaluate(() => {
      fixture.mailRefusal = "";
    });
    await status("rejected").getByRole("button", { name: "Edit and send again" }).click();
    await sheet.waitFor({ state: "visible" });
    assert.equal(await page.locator("#new-title").inputValue(), "Refused one");
    await page.locator("#send-ticket").click();
    await count("landed", 7);
    await count("rejected", 0);

    // An older host, live, without the mailbox: the create op answers, and
    // a lost answer is recovered by its receipt, never sent again.
    await page.evaluate(() => {
      fixture.features = ["permission", "dialog", "awareness", "create"];
      fixture.channel().close();
    });
    await connected();
    await quick("Through the create op");
    await count("landed", 8);
    assert.equal(await page.evaluate(() => fixture.creates.at(-1).title), "Through the create op");
    await page.evaluate(() => {
      fixture.createDisposition = "disconnect";
    });
    await quick("Answer lost");
    await connected();
    await count("landed", 9);
    assert.equal(await page.evaluate(() => fixture.creates.filter((c) => c.title === "Answer lost").length), 1);
    await page.evaluate(() => {
      fixture.createDisposition = "created";
    });
    // Away, the older host keeps nothing for later: the sheet says so.
    await page.evaluate(() => {
      fixture.refuse = true;
      fixture.channel().close();
    });
    await until(page, () => document.querySelector("#shell").dataset.link === "asleep");
    await page.locator("#quick-more").click();
    await sheet.waitFor({ state: "visible" });
    assert.match(await sheet.locator(".compose-dest").textContent(), /kept no tickets for later/);
    await page.locator("#new-title").fill("Nowhere to go");
    assert(await page.locator("#send-ticket").isDisabled());
    await page.keyboard.press("Escape");
    await sheet.waitFor({ state: "hidden" });
    await page.evaluate(() => {
      fixture.refuse = false;
      fixture.features = ["permission", "dialog", "awareness", "create", "mailbox"];
    });
    await connected();

    if (size === "phone") {
      // A keyboard-sized viewport keeps the sheet's Send reachable.
      await page.setViewportSize({ width: 390, height: 500 });
      await page.locator("#quick-more").click();
      await sheet.waitFor({ state: "visible" });
      const send = await sheet.locator(".compose-send-top").boundingBox();
      assert(send.y >= 0 && send.y + send.height <= 500);
      assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
      await page.keyboard.press("Escape");
      await sheet.waitFor({ state: "hidden" });
      await page.setViewportSize(viewport);
    }

    // Revocation forgets what this browser sent.
    await page.evaluate(() => fixture.reply({ result: "revoked" }));
    await until(page, () => document.querySelector("#connection").textContent.includes("Access revoked"));
    const stored = await page.evaluate(
      () =>
        new Promise((resolve) => {
          const open = indexedDB.open("mesophon", 1);
          open.onsuccess = () => {
            const get = open.result.transaction("device").objectStore("device").get("sent:board-a");
            get.onsuccess = () => resolve(get.result ?? null);
          };
        }),
    );
    assert.equal(stored, null);
    assert.deepEqual(errors, []);
    console.log(`${engineName} ${size}: sheet, mailbox live and away, unsend, edit, forged receipt, reload, offline clock, refusal, older host, revocation passed`);
  } catch (error) {
    await page.screenshot({ path: path.join(root, "test-results", `${engineName}-${size}-tickets-failure.png`) });
    throw error;
  } finally {
    await context.close();
  }
}

// Starting an agent from here (T-498, T-510): from the ticket page alone,
// through a sheet that asks for the first prompt; a clock while the host
// starts one and two ticks once it runs; a parked agent woken with words; a
// refusal; a lost answer asked after, never sent again; the terminal away;
// an older host, which offers none. Then the board picker under the brand.
// Notes on the ticket page (T-532): the description under the title, a note
// read and walked, an edit saved through the mailbox with Tell, a stale save
// and Save mine, a new note and its delete, the description's own sheet, an
// edit written while the terminal is away, and the notes read kept for then.
async function notesFlow(browser, engineName, size, viewport) {
  const context = await browser.newContext({ viewport, colorScheme: "dark", reducedMotion: "reduce", serviceWorkers: "block" });
  await context.addInitScript(fixture);
  await context.addInitScript(() => {
    if (localStorage.getItem("fixture-notes")) {
      window.fixture.notes = JSON.parse(localStorage.getItem("fixture-notes"));
    } else {
      const hour = 3_600_000;
      window.fixture.notes = {
        "ticket-0": [
          { id: "desc-0", name: "Make it comfortable", by: "you", at: Date.now() - 2 * hour, rev: 1,
            text: "# Make it comfortable\n\nThe page should read well on a phone.\n\n- one\n- two\n- three\n\n```\n<script>never</script>\n```" },
          { id: "plan-0", name: "Plan: retry the train", by: "agent", at: Date.now() - 14 * 60000, rev: 2,
            text: "# Plan: retry the train\n\n1. Re-read the tip.\n2. Requeue.\n\n![shot](mesimon-attachment:01J)" },
        ],
      };
    }
    window.fixture.features.push("notes", "pictures");
    window.fixture.stampNotes();
    addEventListener("beforeunload", () => localStorage.setItem("fixture-notes", JSON.stringify(window.fixture.notes)));
  });
  await context.route("**/pkg/mesimon_web.js", (route) =>
    route.fulfill({ contentType: "text/javascript", body: fakeCrypto }),
  );
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(String(error)));
  const toast = (text) => until(page, (text) => document.querySelector("#toast").textContent.includes(text), text);
  const sheet = page.locator("#note-sheet");
  const reader = page.locator("#note-reader");
  const notes = page.locator("#notes");
  const shot = (name) =>
    page.screenshot({ path: path.join(root, "test-results", `${engineName}-${size}-${name}.png`) });
  const open = async (id) => {
    if (size === "phone" && (await page.locator("#back").isVisible())) await page.locator("#back").click();
    await page.locator(`.ticket[data-id="${id}"]`).click();
  };
  const save = async (text) => {
    await sheet.waitFor({ state: "visible" });
    await page.locator("#note-text").fill(text);
    await page.locator("#save-note").click();
    await sheet.waitFor({ state: "hidden" });
  };
  try {
    await page.goto(origin);
    await until(page, () => !document.querySelector("#pair").disabled);
    await page.getByLabel("Pairing code", { exact: true }).fill("fixture-pair-code");
    await page.getByRole("button", { name: "Connect", exact: true }).click();
    await page.locator('.ticket[data-id="ticket-0"]').waitFor();
    await open("ticket-0");

    // The description under the title, clamped while the agent works, and
    // the note as a row; nothing in a body becomes markup.
    await notes.locator(".notes-description").waitFor();
    assert.match(await notes.locator(".notes-description").textContent(), /read well on a phone/);
    assert.equal(await notes.locator("script").count(), 0);
    assert.equal(await notes.getAttribute("data-awake"), "true");
    // No heading and no pen (T-633): the description itself is the press,
    // faded at the foot because the clamp cut it.
    assert.equal(await notes.locator("h3").filter({ hasText: "Description" }).count(), 0);
    assert.equal(await notes.locator("#edit-description").count(), 0);
    assert.equal(await notes.locator(".notes-description").getAttribute("data-clipped"), "true");
    const row = notes.locator(".note-row").filter({ hasText: "Plan: retry the train" });
    assert.match(await row.textContent(), /agent · 14m/);
    if (size === "phone") {
      const short = await notes.evaluate((node) =>
        [...node.querySelectorAll("button")]
          .filter((n) => n.getClientRects().length && n.getBoundingClientRect().height < 44)
          .map((n) => n.id || n.textContent),
      );
      assert.deepEqual(short, []);
    }
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    await shot("notes-ticket");

    // A note read in place of the output, walked like the desk's Tab.
    await row.click();
    await reader.waitFor();
    assert.match(await reader.locator(".markdown").textContent(), /Re-read the tip/);
    assert.match(await reader.locator(".markdown-picture").textContent(), /open it at your desk/);
    assert.match(await reader.locator(".label").first().textContent(), /Note 1 of 1 · agent/);
    await page.locator("#note-next").click();
    await until(page, () => /Description/.test(document.querySelector("#note-reader .label").textContent));
    await page.locator("#note-prev").click();
    await until(page, () => /Note 1 of 1/.test(document.querySelector("#note-reader .label").textContent));
    await shot("notes-reader");

    // Edited and saved through the mailbox, with the revision it opened;
    // the agent awake on the ticket can be told.
    await page.locator("#edit-note").click();
    await sheet.waitFor({ state: "visible" });
    assert.match(await page.locator("#note-text").inputValue(), /Re-read the tip/);
    assert.equal(await page.locator("#note-heading").textContent(), "Edit note · T-0");
    await shot("notes-sheet");
    await save("# Plan: retry the train\n\n1. Re-read the tip first.");
    await toast("Saved");
    const sent = await page.evaluate(() => fixture.noteWrites.at(-1));
    assert.deepEqual([sent.kind, sent.note, sent.rev], ["note", "plan-0", 2]);
    await page.locator("#toast-action").click();
    await until(page, () => fixture.told.length === 1);
    assert.deepEqual(await page.evaluate(() => fixture.told[0]), { op: "tell_agent", ticket: "ticket-0", note: "plan-0" });
    await until(page, () => /tip first/.test(document.querySelector("#note-reader .markdown").textContent));

    // Changed at the terminal while the sheet was open: refused as stale,
    // the words kept, and Save mine writes them over.
    await page.locator("#edit-note").click();
    await sheet.waitFor({ state: "visible" });
    await page.evaluate(() => {
      const note = fixture.notes["ticket-0"][1];
      Object.assign(note, { rev: note.rev + 1, text: "# Plan: retry the train\n\nTheirs.", by: "agent" });
      fixture.stampNotes();
    });
    await save("# Plan: retry the train\n\nMine.");
    await toast("Not saved: agent changed it");
    await reader.locator(".note-strip-conflict").waitFor();
    assert.match(await reader.locator(".markdown").textContent(), /Mine\./);
    await page.locator("#save-mine").click();
    await toast("Saved");
    assert.match(await page.evaluate(() => fixture.notes["ticket-0"][1].text), /Mine\./);
    await reader.locator(".note-strip-conflict").waitFor({ state: "detached" });

    // A new note, then deleted: the delete asks once more.
    await page.locator("#note-back").click();
    await notes.waitFor();
    await page.locator("#add-note").click();
    assert.equal(await page.locator("#note-heading").textContent(), "New note · T-0");
    await save("Second thoughts\n\nmore");
    await toast("Saved");
    const fresh = notes.locator(".note-row").filter({ hasText: "Second thoughts" });
    await fresh.waitFor();
    await fresh.click();
    await page.locator("#edit-note").click();
    await sheet.waitFor({ state: "visible" });
    await page.locator("#delete-note").click();
    assert.match(await page.locator("#delete-note").textContent(), /Delete for good/);
    await page.locator("#delete-note").click();
    await sheet.waitFor({ state: "hidden" });
    await toast("Deleted");
    // A short drag puts the toast back; a swipe sends it off before its time.
    await swipe(page, page.locator(".toast-body"), 6);
    assert.equal(await page.locator(".toast-body").evaluate((el) => el.style.translate), "");
    await swipe(page, page.locator(".toast-body"), -160);
    await until(page, () => !document.querySelector(".toast-body"), undefined, 1500);
    await notes.waitFor();
    await fresh.waitFor({ state: "detached" });
    assert.equal(await page.evaluate(() => fixture.notes["ticket-0"].length), 2);

    // A press on the description opens it whole, and its own sheet, from
    // there, has no delete.
    await page.locator("#open-description").click();
    await reader.waitFor();
    assert.match(await reader.locator(".label").first().textContent(), /Description/);
    await page.locator("#edit-note").click();
    await sheet.waitFor({ state: "visible" });
    assert.equal(await page.locator("#note-heading").textContent(), "Description · T-0");
    assert.equal(await page.locator("#delete-note").count(), 0);
    await sheet.getByRole("button", { name: "Cancel" }).click();
    await sheet.waitFor({ state: "hidden" });
    await page.locator("#note-back").click();
    await notes.waitFor();

    // Past two notes the page lists the latest written, and All opens a
    // sheet of every one (T-627); a row there opens the note.
    await page.evaluate(() => {
      const hour = 3_600_000;
      fixture.notes["ticket-0"].push(
        { id: "old-1", name: "Older thoughts", by: "you", at: Date.now() - 3 * hour, rev: 1, text: "# Older thoughts" },
        { id: "old-2", name: "Oldest thoughts", by: "agent", at: Date.now() - 4 * hour, rev: 1, text: "# Oldest thoughts" },
      );
      fixture.stampNotes();
    });
    const notesSheet = page.locator("#notes-sheet");
    await page.locator("#all-notes").waitFor();
    assert.equal(await notes.locator(".note-row").count(), 1);
    assert.match(await notes.locator(".note-row").textContent(), /Plan: retry the train.*latest · /);
    await page.locator("#all-notes").click();
    await notesSheet.waitFor({ state: "visible" });
    assert.deepEqual(await notesSheet.locator(".note-name").allTextContents(), [
      "Plan: retry the train",
      "Older thoughts",
      "Oldest thoughts",
    ]);
    await shot("notes-all");
    await notesSheet.locator(".note-row").filter({ hasText: "Oldest thoughts" }).click();
    await notesSheet.waitFor({ state: "hidden" });
    await reader.waitFor();
    assert.match(await reader.locator(".label").first().textContent(), /Note 3 of 3/);
    await page.locator("#note-back").click();
    await page.locator("#all-notes").click();
    await notesSheet.waitFor({ state: "visible" });
    await page.locator("#notes-done").click();
    await notesSheet.waitFor({ state: "hidden" });
    assert.equal(await page.evaluate(() => document.activeElement?.id), "all-notes");
    await page.evaluate(() => {
      fixture.notes["ticket-0"].splice(2);
      fixture.stampNotes();
    });
    await page.locator("#all-notes").waitFor({ state: "detached" });

    // A picture in a new note (T-629): picked, named [Image #1] in the
    // words, shown as a thumbnail; a second one removed again; then sent
    // in pieces over the live channel ahead of the note that links it.
    await notes.waitFor();
    await page.locator("#add-note").click();
    await sheet.waitFor({ state: "visible" });
    await page.locator("#note-text").fill("Screenshot of the bug\n\n");
    const png = noisePng(240, 160);
    await page.locator("#note-picture").setInputFiles({ name: "shot.png", mimeType: "image/png", buffer: png });
    await sheet.locator(".note-pic").first().waitFor();
    assert.match(await page.locator("#note-text").inputValue(), /^Screenshot of the bug\n\n\[Image #1\]$/);
    await page.locator("#note-picture").setInputFiles({ name: "two.png", mimeType: "image/png", buffer: png });
    await until(page, () => document.querySelectorAll("#note-sheet .note-pic").length === 2);
    assert.match(await page.locator("#note-text").inputValue(), /\[Image #1\] \[Image #2\]$/);
    await shot("notes-picture");
    await page.getByRole("button", { name: "Remove Image #2" }).click();
    await until(page, () => document.querySelectorAll("#note-sheet .note-pic").length === 1);
    assert.doesNotMatch(await page.locator("#note-text").inputValue(), /Image #2/);
    await page.locator("#save-note").click();
    await sheet.waitFor({ state: "hidden" });
    await toast("Saved");
    const pictured = await page.evaluate(() => fixture.noteWrites.at(-1));
    assert.deepEqual([pictured.op, pictured.uploads], ["write_note", ["pic-0"]]);
    assert.match(pictured.text, /\[Image #1\]\(mesimon-attachment:pic-0\)/);
    const upload = await page.evaluate(() => fixture.uploads["pic-0"]);
    assert(upload.complete && upload.pieces >= 2, JSON.stringify(upload));
    assert.equal(Object.keys(await page.evaluate(() => fixture.uploads)).length, 1, "the removed picture never went up");
    const withPicture = notes.locator(".note-row").filter({ hasText: "Screenshot of the bug" });
    await withPicture.click();
    await reader.waitFor();
    assert.match(await reader.locator(".markdown-picture").textContent(), /Image #1 · open it at your desk/);
    // Picked before the cursor was ever placed, a picture goes at the end;
    // Cancel sends nothing.
    await page.locator("#edit-note").click();
    await sheet.waitFor({ state: "visible" });
    await page.locator("#note-picture").setInputFiles({ name: "three.png", mimeType: "image/png", buffer: png });
    await sheet.locator(".note-pic").first().waitFor();
    assert.match(await page.locator("#note-text").inputValue(), /\(mesimon-attachment:pic-0\) \[Image #2\]$/);
    await sheet.getByRole("button", { name: "Cancel" }).click();
    await sheet.waitFor({ state: "hidden" });
    assert.equal(Object.keys(await page.evaluate(() => fixture.uploads)).length, 1);
    await page.locator("#note-back").click();
    await notes.waitFor();
    await page.evaluate(() => {
      fixture.notes["ticket-0"].splice(2);
      fixture.stampNotes();
    });
    await withPicture.waitFor({ state: "detached" });

    // A ticket without notes offers a description.
    await open("ticket-4");
    await page.locator("#add-description").waitFor();
    assert.equal(await notes.locator(".note-row").count(), 0);

    // The terminal away: the notes read stay readable, as of when, and an
    // edit waits at the relay with one tick until it is back.
    await open("ticket-0");
    await notes.locator(".notes-description").waitFor();
    await page.evaluate(() => {
      fixture.refuse = true;
      fixture.channel().close();
    });
    await until(page, () => document.querySelector("#shell").dataset.link === "asleep");
    await until(page, () => /as of/.test(document.querySelector("#notes .label").textContent));
    await notes.locator(".note-row").first().click();
    await page.locator("#edit-note").click();
    await sheet.waitFor({ state: "visible" });
    assert.match(await sheet.locator(".compose-dest").textContent(), /Saves when your terminal is back/);
    await save("# Plan: retry the train\n\nWritten away.");
    await reader.locator(".note-strip").filter({ hasText: "Waits for your terminal" }).waitFor();
    assert.match(await reader.locator(".markdown").textContent(), /Written away/);
    await shot("notes-away");
    // A reload keeps the notes read and the edit on its way; the page
    // reopens the ticket it was on.
    await page.reload();
    await notes.locator(".notes-description").waitFor();
    assert.match(await notes.locator(".notes-description").textContent(), /read well on a phone/);
    await notes.locator(".note-row .tick").waitFor();
    await page.evaluate(() => {
      fixture.refuse = false;
      fixture.collect();
    });
    await until(page, () => document.querySelectorAll("#notes .note-row .tick").length === 0);
    assert.match(await page.evaluate(() => fixture.notes["ticket-0"][1].text), /Written away/);

    // A host that keeps no mail takes the edit over the live channel.
    await page.evaluate(() => {
      fixture.features = fixture.features.filter((f) => f !== "mailbox");
      fixture.channel().close();
    });
    await until(page, () => document.querySelector("#connection").textContent !== "Connected");
    await until(page, () => document.querySelector("#connection").textContent === "Connected");
    await notes.locator(".note-row").first().click();
    await page.locator("#edit-note").click();
    await save("# Plan: retry the train\n\nLive op.");
    await toast("Saved");
    const live = await page.evaluate(() => fixture.noteWrites.at(-1));
    assert.deepEqual([live.op, live.note, live.text.includes("Live op.")], ["write_note", "plan-0", true]);
    assert.deepEqual(errors, []);
    console.log(`${engineName} ${size}: notes read, walked, edited, told, stale, added, deleted, listed, pictured, away, kept and live passed`);
  } catch (error) {
    await page.screenshot({ path: path.join(root, "test-results", `${engineName}-${size}-notes-failure.png`) });
    throw error;
  } finally {
    await page.evaluate(() => localStorage.clear()).catch(() => {});
    await context.close();
  }
}

async function startFlow(browser, engineName, size, viewport) {
  const context = await browser.newContext({ viewport, colorScheme: "dark", reducedMotion: "reduce", serviceWorkers: "block" });
  await context.addInitScript(fixture);
  await context.addInitScript(() => {
    window.fixture.features.push("start");
    window.fixture.tickets[5].agent = null;
    window.fixture.tickets[9].agent.state = "sleeping";
  });
  await context.route("**/pkg/mesimon_web.js", (route) =>
    route.fulfill({ contentType: "text/javascript", body: fakeCrypto }),
  );
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(String(error)));
  const mode = (name) => page.locator(`button[data-mode="${name}"]`).locator("visible=true").click();
  const overview = async () => {
    if (size === "phone" && (await page.locator("#back").isVisible())) await page.locator("#back").click();
  };
  // A desktop's Board opens the ticket over a backdrop (T-510): the next
  // card is behind it until the backdrop closes the ticket.
  const closeOverlay = async () => {
    if (size === "desktop" && (await page.locator(".detail-scrim").count()))
      await page.locator(".detail-scrim").click({ position: { x: 10, y: 10 } });
  };
  const open = async (id) => {
    await overview();
    await closeOverlay();
    await page.locator(`.ticket[data-id="${id}"]`).click();
  };
  const connected = () =>
    until(page, () => document.querySelector("#connection").textContent === "Connected");
  const toast = (text) => until(page, (text) => document.querySelector("#toast").textContent.includes(text), text);
  const receiptIs = (status) =>
    until(page, (status) => document.querySelector("#detail .start-receipt")?.dataset.status === status, status);
  const shot = (name) =>
    page.screenshot({ path: path.join(root, "test-results", `${engineName}-${size}-${name}.png`) });
  const pageStart = page.locator("#detail .start-agent");
  const sheet = page.locator("#start-sheet");
  const receipt = page.locator("#detail .start-receipt");
  // The page's start opens the sheet; the words, if any, and Start.
  const start = async (text = "") => {
    await pageStart.click();
    await sheet.waitFor({ state: "visible" });
    if (text) await page.locator("#start-prompt").fill(text);
    await page.locator("#start-send").click();
    await sheet.waitFor({ state: "hidden" });
  };
  try {
    await page.goto(origin);
    await until(page, () => !document.querySelector("#pair").disabled);
    await page.getByLabel("Pairing code", { exact: true }).fill("fixture-pair-code");
    await page.getByRole("button", { name: "Connect", exact: true }).click();
    await page.locator('.ticket[data-id="ticket-0"]').waitFor();
    await mode("board");
    if (size === "phone") await page.locator('.column-tab[data-column="TODO"]').click();

    // No card offers a start (T-510): the ticket page does, over an empty
    // seat or a parked agent.
    await page.locator('.ticket[data-id="ticket-3"]').waitFor();
    assert.equal(await page.locator("[data-start]").count(), 0, "a Board card holds no start");
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    await page.locator('.ticket[data-id="ticket-3"]').click();
    await pageStart.waitFor();
    assert.equal(await pageStart.getAttribute("aria-label"), "Start agent on T-3");
    assert.equal(await page.locator("#agent-state").textContent(), "No agent on this ticket.");
    if (size === "desktop") {
      // Opened from the Board, the ticket sits on a backdrop that closes it.
      await page.locator(".detail-scrim").waitFor();
      await shot("start-overlay");
      await closeOverlay();
      await until(page, () => document.querySelector("#shell").dataset.detail === "false");
      await page.locator('.ticket[data-id="ticket-3"]').click();
      await pageStart.waitFor();
    }
    for (const theme of ["graphite", "chalk"]) {
      await page.evaluate((name) => {
        const control = document.querySelector("#theme");
        control.value = name;
        control.dispatchEvent(new Event("change"));
      }, theme);
      await shot(`start-ticket-${theme}`);
    }

    // The sheet asks for the first prompt. Blank, the request names the
    // ticket and nothing else: the words and the provider are the host's.
    await page.evaluate(() => {
      fixture.startDisposition = "hold";
    });
    await pageStart.click();
    await sheet.waitFor({ state: "visible" });
    assert.match(await page.locator("#start-heading").textContent(), /^Start agent · T-3$/);
    assert.equal(await page.evaluate(() => document.activeElement.id), "start-prompt");
    await shot("start-sheet");
    await page.locator("#start-send").click();
    await sheet.waitFor({ state: "hidden" });
    await receiptIs("sending");
    await toast("Starting an agent on T-3");
    assert.equal(await page.locator("#toast .tick circle").count(), 1, "a clock");
    assert.deepEqual(await page.evaluate(() => fixture.starts), [{ op: "start", ticket: "ticket-3" }]);
    assert(await pageStart.isDisabled(), "one start at a time");
    const command = await page.evaluate(() => fixture.startCommands.at(-1));
    await page.evaluate((id) => {
      fixture.answers[id] = { result: "delivery", status: "provisioning" };
      fixture.reply(fixture.answers[id], id);
    }, command);
    await receiptIs("provisioning");
    assert.match(await receipt.textContent(), /worktree/);
    assert.equal(await receipt.locator(".tick circle").count(), 1);
    // The agent comes up: the page shows it and offers no start.
    await page.evaluate((id) => fixture.begin("ticket-3", id), command);
    await until(
      page,
      () =>
        !document.querySelector("#detail .start-agent") &&
        document.querySelector("#agent-word")?.textContent.includes("claude · starting"),
    );
    // It took its first prompt: two ticks, found by the receipt's poll.
    await page.evaluate(() => fixture.run("ticket-3"));
    await toast("An agent is working on T-3");
    assert.equal(await page.locator("#toast .tick path").count(), 2, "two ticks");
    await receiptIs("started");
    assert.match(await receipt.textContent(), /Started from this browser · \d/);
    // A start that took sits beside the tags and the column (T-547).
    assert.equal(await page.locator("#detail .ticket-line .start-receipt").count(), 1);
    assert.equal(await receipt.locator(".tick path").count(), 2);
    assert.equal(await pageStart.count(), 0, "a ticket with an agent offers no start");
    await shot("start-started");

    // A parked agent (T-510): the page offers a wake, the composer nothing,
    // and the words go with the request.
    await open("ticket-9");
    await pageStart.waitFor();
    assert.equal(await pageStart.getAttribute("aria-label"), "Wake agent on T-9");
    assert.equal(await page.locator("#agent-state").textContent(), "codex is asleep on this ticket.");
    assert(await page.locator("#send").isDisabled(), "a parked agent has no pane to message");
    await page.evaluate(() => {
      fixture.startDisposition = "starting";
    });
    await pageStart.click();
    await sheet.waitFor({ state: "visible" });
    assert.match(await page.locator("#start-heading").textContent(), /^Wake agent · T-9$/);
    await page.locator("#start-prompt").fill("Rebase first, then run the tests.");
    assert.equal(await page.locator("#start-send").textContent(), "Wake with these words");
    await shot("wake-sheet");
    await page.locator("#start-send").click();
    await sheet.waitFor({ state: "hidden" });
    await toast("Waking the agent on T-9");
    assert.deepEqual(await page.evaluate(() => fixture.starts.at(-1)), {
      op: "start",
      ticket: "ticket-9",
      prompt: "Rebase first, then run the tests.",
    });
    await page.evaluate(() => fixture.run("ticket-9"));
    await receiptIs("started");

    // Refused by the host: why, and the button stays to try again.
    await open("ticket-5");
    await pageStart.waitFor();
    assert.equal(await page.locator("#agent-state").textContent(), "No agent on this ticket.");
    await page.evaluate(() => {
      fixture.startDisposition = "rejected";
    });
    await start();
    await receiptIs("rejected");
    assert.match(await receipt.textContent(), /Not started: this ticket already has an agent/);
    await toast("Not started: this ticket already has an agent");
    assert(await pageStart.isEnabled(), "a refusal leaves the button to try again");

    // A lost answer is asked after with `status`, never sent again.
    await page.evaluate(() => {
      fixture.startDisposition = "hold";
    });
    await start();
    await receiptIs("sending");
    assert(await pageStart.isDisabled(), "one start at a time");
    await page.evaluate(() => {
      fixture.begin("ticket-5", fixture.startCommands.at(-1));
      fixture.channel().close();
    });
    await connected();
    await receiptIs("starting");
    await page.evaluate(() => fixture.run("ticket-5"));
    await receiptIs("started");
    assert.equal(
      await page.evaluate(() => fixture.starts.filter((r) => r.ticket === "ticket-5").length),
      2,
      "the refused start and this one, never a resend",
    );

    // The terminal away: the button stays where it is, disabled, and says why.
    await page.evaluate(() => {
      fixture.tickets[7].agent = null;
      fixture.update();
    });
    await open("ticket-7");
    await pageStart.waitFor();
    await page.evaluate(() => {
      fixture.refuse = true;
      fixture.channel().close();
    });
    await until(page, () => document.querySelector("#shell").dataset.link === "asleep");
    assert(await pageStart.isDisabled());
    assert.match(await page.locator("#agent-state").textContent(), /needs your terminal back/);
    await shot("start-away");

    // An older host, live again, offers no start anywhere.
    await page.evaluate(() => {
      fixture.refuse = false;
      fixture.features = fixture.features.filter((f) => f !== "start");
    });
    await connected();
    assert.equal(await page.locator("[data-start]").count(), 0, "an older host offers no start");
    await open("ticket-7");
    assert.match(await page.locator("#agent-state").textContent(), /Start one at your terminal/);

    // The board picker (T-510): the board's name under the brand lists every
    // paired board and the way to pair one more; Escape closes it.
    await overview();
    if (size !== "desktop") await page.locator("#board-menu").click();
    await page.locator("#board-picker").click();
    await page.locator("#board-list").waitFor();
    assert.equal(
      await page.locator('#board-list .side-board[aria-current="true"] .side-board-name').textContent(),
      "Mesimon",
    );
    assert(await page.locator("#add-board").isVisible());
    assert.equal(await page.locator("#sidebar .side-title").textContent(), "mesimon");
    await shot("board-picker");
    await page.keyboard.press("Escape");
    await page.locator("#board-list").waitFor({ state: "hidden" });
    assert.equal(await page.locator("#sidebar .side-pick-name").textContent(), "Mesimon");
    assert.deepEqual(errors, []);
    console.log(`${engineName} ${size}: start from the ticket's sheet, clock, ticks, wake with words, refusal, lost answer, away, older host, board picker passed`);
  } catch (error) {
    await page.screenshot({ path: path.join(root, "test-results", `${engineName}-${size}-start-failure.png`) });
    throw error;
  } finally {
    await context.close();
  }
}

// Agent tiers from here (T-643): the start sheet offers every tier for an
// empty seat and the composer its agent's provider's alone; a pick rides
// the start or the prompt only when it changes the ticket's, a switch queues
// the words, and an older host is sent no tier and offered none.
async function tierFlow(browser, engineName, size, viewport) {
  const context = await browser.newContext({ viewport, colorScheme: "dark", reducedMotion: "reduce", serviceWorkers: "block" });
  await context.addInitScript(fixture);
  await context.addInitScript(() => {
    const f = window.fixture;
    f.features.push("start", "tiers");
    f.tiers = [
      { id: "claude", name: "claude", provider: "claude", summary: "Claude Code ∙ its own model" },
      { id: "01K", name: "coder", provider: "claude", summary: "Claude Code ∙ opus ∙ high" },
      { id: "01M", name: "quick", provider: "codex", summary: "Codex ∙ gpt-5 ∙ low" },
    ];
    for (const t of f.tickets) {
      t.tier = "claude";
      if (t.agent) t.agent.tier = t.agent.provider === "codex" ? "codex" : "claude";
    }
    const snapshot = f.snapshot.bind(f);
    f.snapshot = () => ({ ...snapshot(), ...(f.features.includes("tiers") ? { tiers: f.tiers } : {}) });
  });
  await context.route("**/pkg/mesimon_web.js", (route) =>
    route.fulfill({ contentType: "text/javascript", body: fakeCrypto }),
  );
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(String(error)));
  const mode = (name) => page.locator(`button[data-mode="${name}"]`).locator("visible=true").click();
  const overview = async () => {
    if (size === "phone" && (await page.locator("#back").isVisible())) await page.locator("#back").click();
  };
  const closeOverlay = async () => {
    if (size === "desktop" && (await page.locator(".detail-scrim").count()))
      await page.locator(".detail-scrim").click({ position: { x: 10, y: 10 } });
  };
  const shot = (name) =>
    page.screenshot({ path: path.join(root, "test-results", `${engineName}-${size}-${name}.png`) });
  const sheet = page.locator("#start-sheet");
  const tierInputs = page.locator('#start-sheet input[name="start-tier"]');
  try {
    await page.goto(origin);
    await until(page, () => !document.querySelector("#pair").disabled);
    await page.getByLabel("Pairing code", { exact: true }).fill("fixture-pair-code");
    await page.getByRole("button", { name: "Connect", exact: true }).click();
    await page.locator('.ticket[data-id="ticket-0"]').waitFor();

    // An empty seat is offered every tier, its own picked; a pick rides.
    await mode("board");
    if (size === "phone") await page.locator('.column-tab[data-column="TODO"]').click();
    await page.locator('.ticket[data-id="ticket-3"]').click();
    await page.locator("#detail .start-agent").click();
    await sheet.waitFor({ state: "visible" });
    assert.equal(await tierInputs.count(), 3);
    assert(await tierInputs.nth(0).isChecked(), "the ticket's own tier is picked");
    assert.match(await sheet.textContent(), /Claude Code ∙ its own model/);
    await sheet.locator("label.choice", { hasText: "coder" }).click();
    assert.match(await sheet.textContent(), /Claude Code ∙ opus ∙ high/);
    assert.doesNotMatch(await sheet.textContent(), /Your terminal picks the provider/);
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    await shot("tier-start-sheet");
    await page.locator("#start-send").click();
    await sheet.waitFor({ state: "hidden" });
    assert.deepEqual(await page.evaluate(() => fixture.starts.at(-1)),
      { op: "start", ticket: "ticket-3", tier: "01K" });

    // The composer offers the agent's provider's tiers; its own tier rides
    // nothing, and another one queues the words with the pick.
    await overview();
    await closeOverlay();
    await mode("agents");
    await page.locator('.ticket[data-id="ticket-0"]').click();
    const pick = page.locator("#prompt-tier");
    await pick.waitFor();
    assert.deepEqual(await pick.locator("option").allTextContents(), ["claude", "coder"]);
    assert.equal(await pick.inputValue(), "claude");
    assert(await page.locator("#steer-why").isHidden());
    await page.locator("#prompt").fill("same tier");
    await page.locator("#send").click();
    await until(page, () => fixture.prompts.length === 1);
    assert.equal(await page.evaluate(() => fixture.prompts[0].tier), undefined);
    await until(page, () => !document.querySelector("#send").disabled || !document.querySelector("#prompt").value);
    await pick.selectOption("01K");
    await until(page, () => !document.querySelector("#steer-why").hidden);
    assert.match(await page.locator("#steer-why").textContent(), /Restarts on coder when the turn ends/);
    assert(await page.locator('#prompt-mode input[value="steer"]').isDisabled());
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    await shot("tier-composer");
    await page.locator("#prompt").fill("now on coder");
    await until(page, () => !document.querySelector("#send").disabled);
    await page.locator("#send").click();
    await until(page, () => fixture.prompts.length === 2);
    const sent = await page.evaluate(() => fixture.prompts[1]);
    assert.equal(sent.tier, "01K");
    assert.equal(sent.queued, true);

    // An older host is offered no tier and sent none.
    await page.evaluate(() => {
      fixture.features = fixture.features.filter((f) => f !== "tiers");
      fixture.channel().close();
    });
    await until(page, () => document.querySelector("#connection").textContent === "Connected");
    await until(page, () => !document.querySelector("#prompt-tier"));
    assert.deepEqual(errors, []);
    console.log(`${engineName} ${size}: tiers picked for a start and a prompt, a switch queued, an older host passed`);
  } catch (error) {
    await page.screenshot({ path: path.join(root, "test-results", `${engineName}-${size}-tier-failure.png`) });
    throw error;
  } finally {
    await context.close();
  }
}

// Editing a card from here (T-530): the title written over itself, the
// line's sheet moving and tagging the ticket a press at a time, a refusal
// in the sheet's own words, drag and drop where the columns sit side by side,
// a reorder and a drop where the card already is; nothing while the
// terminal is away, and nothing from an older host.
async function editFlow(browser, engineName, size, viewport) {
  const context = await browser.newContext({ viewport, colorScheme: "dark", reducedMotion: "reduce", serviceWorkers: "block" });
  await context.addInitScript(fixture);
  await context.addInitScript(() => window.fixture.features.push("rename", "move", "tag"));
  await context.route("**/pkg/mesimon_web.js", (route) =>
    route.fulfill({ contentType: "text/javascript", body: fakeCrypto }),
  );
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(String(error)));
  const mode = (name) => page.locator(`button[data-mode="${name}"]`).locator("visible=true").click();
  const overview = async () => {
    if (size === "phone" && (await page.locator("#back").isVisible())) await page.locator("#back").click();
  };
  const closeOverlay = async () => {
    if (size === "desktop" && (await page.locator(".detail-scrim").count()))
      await page.locator(".detail-scrim").click({ position: { x: 10, y: 10 } });
  };
  const connected = () =>
    until(page, () => document.querySelector("#connection").textContent === "Connected");
  const toast = (text) => until(page, (text) => document.querySelector("#toast").textContent.includes(text), text);
  const shot = (name) =>
    page.screenshot({ path: path.join(root, "test-results", `${engineName}-${size}-${name}.png`) });
  const edits = () => page.evaluate(() => fixture.edits);
  const lastEdit = () => page.evaluate(() => fixture.edits.at(-1));
  const column = (name) => page.locator(`.column[aria-label="${name}"]`);
  const order = (name) =>
    column(name).locator(".ticket.card").evaluateAll((cards) => cards.map((c) => c.dataset.id));
  const sheet = page.locator("#card-sheet");
  try {
    await page.goto(origin);
    await until(page, () => !document.querySelector("#pair").disabled);
    await page.getByLabel("Pairing code", { exact: true }).fill("fixture-pair-code");
    await page.getByRole("button", { name: "Connect", exact: true }).click();
    await page.locator('.ticket[data-id="ticket-0"]').waitFor();
    await mode("board");
    if (size === "phone") await page.locator('.column-tab[data-column="TODO"]').click();
    await page.locator('.ticket[data-id="ticket-3"]').click();
    await page.locator("#rename").waitFor();

    // The crown (T-623): a ticket it touched says what was done and whose
    // agent did it, on its card and its page, for an hour; the ticket that
    // wears it says so.
    await page.evaluate(() => {
      fixture.tickets[0].crown = true;
      fixture.tickets[3].crowned = { action: "moved", by: "T-0", at: Date.now() - 120000 };
      fixture.update();
    });
    await until(page, () => document.querySelector("#crown-line")?.textContent.includes("T-0’s agent moved this ticket · 2m ago"));
    assert.equal(await page.locator('.ticket.card[data-id="ticket-3"] .crown-word').textContent(), "moved · 2m");
    // A phone draws one column at a time, and the crown's is the other one.
    if (size !== "phone")
      assert.equal(await page.locator('.ticket.card[data-id="ticket-0"] .ticket-title.crowned > .crown-mark:first-child').count(), 1);
    await shot("crown-touched");
    // The ticket that wears it: the crown before its title on its card and
    // its page, the title in the crown's ink, and no line about it.
    await page.evaluate(() => {
      fixture.tickets[3].crown = true;
      fixture.update();
    });
    await until(page, () => !!document.querySelector("#selection.crowned #rename > .crown-mark:first-child"));
    assert.equal(await page.locator('.ticket.card[data-id="ticket-3"] .ticket-title.crowned > .crown-mark:first-child').count(), 1);
    assert.equal(
      await page.locator("#selection").evaluate((n) => getComputedStyle(n).color),
      await page.locator("#crown-line > .icon").evaluate((n) => getComputedStyle(n).color),
      "the title is in the crown's ink",
    );
    assert.doesNotMatch(await page.locator("#detail").textContent(), /Wears the crown ·/);
    await shot("crown-worn");
    await page.evaluate(() => {
      fixture.tickets[3].crowned.at = Date.now() - 2 * 3600000;
      fixture.update();
    });
    await until(page, () => !document.querySelector("#crown-line"));
    assert.equal(await page.locator('.ticket.card[data-id="ticket-3"] .crown-word').count(), 0, "an hour on, it is not news");
    await page.evaluate(() => {
      delete fixture.tickets[0].crown;
      delete fixture.tickets[3].crown;
      delete fixture.tickets[3].crowned;
      fixture.update();
    });
    await until(page, () => !document.querySelector(".crown-word, .crown-mark, .crowned"));

    // The title writes over itself: Escape puts it back and sends nothing.
    await page.locator("#rename").click();
    assert.equal(await page.evaluate(() => document.activeElement.id), "rename-title");
    assert.equal(await page.locator("#rename-title").inputValue(), "Agent task 3");
    assert.deepEqual(
      await page.locator("#rename-title").evaluate((n) => [n.selectionStart, n.selectionEnd]),
      [12, 12],
      "the caret starts at the end",
    );
    await page.locator("#rename-title").fill("");
    assert(await page.locator("#rename-save").isDisabled(), "a ticket needs a title");
    await page.locator("#rename-title").fill("Not this");
    await page.keyboard.press("Escape");
    await page.locator("#rename-form").waitFor({ state: "detached" });
    assert.equal(await page.locator("#selection").textContent(), "Agent task 3");
    assert.deepEqual(await edits(), []);
    // Enter saves: the page wears it at once, and the host's board agrees.
    await page.locator("#rename").click();
    await page.locator("#rename-title").fill("Renamed  from the\nphone");
    await shot("rename");
    await page.keyboard.press("Enter");
    assert.equal(await page.locator("#selection").textContent(), "Renamed from the phone");
    assert.deepEqual(await lastEdit(), { op: "rename", ticket: "ticket-3", title: "Renamed from the phone" });
    if (size !== "phone")
      assert.equal(await page.locator('.ticket[data-id="ticket-3"] .ticket-title').textContent(), "Renamed from the phone");

    // The line opens the sheet. A column moves it at the press; a tag goes
    // on, another on its group replaces it, and a press on the worn one
    // takes it off.
    assert.equal(await page.locator("#card-line .chip-column").textContent(), "TODO");
    assert.equal(await page.locator("#card-line .add-tag").textContent(), "Tag");
    await page.locator("#card-line").click();
    await sheet.waitFor({ state: "visible" });
    assert.equal(await page.locator("#card-heading").textContent(), "T-3");
    assert(await sheet.getByRole("radio", { name: "TODO", exact: true }).isChecked());
    await sheet.getByRole("radio", { name: "DONE", exact: true }).check();
    assert.deepEqual(await lastEdit(), { op: "move", ticket: "ticket-3", column: "DONE" });
    await until(page, () => document.querySelector("#card-line .chip-column").textContent === "DONE");
    await sheet.getByRole("button", { name: "BUG", exact: true }).click();
    assert.deepEqual(await lastEdit(), { op: "tag", ticket: "ticket-3", group: 1, name: "BUG" });
    await sheet.getByRole("button", { name: "FEATURE", exact: true }).click();
    await sheet.getByRole("button", { name: "QUESTION", exact: true }).click();
    await until(page, () => document.querySelectorAll("#card-sheet .tag-chip[aria-pressed=true]").length === 2);
    assert.deepEqual(await sheet.locator(".tag-chip[aria-pressed=true]").allTextContents(), ["FEATURE", "QUESTION"]);
    await sheet.getByRole("button", { name: "QUESTION", exact: true }).click();
    assert.deepEqual(await lastEdit(), { op: "tag", ticket: "ticket-3", group: 2 });
    await until(page, () => document.querySelectorAll("#card-sheet .tag-chip[aria-pressed=true]").length === 1);
    if (size === "phone") {
      const short = await sheet.evaluate((node) =>
        [...node.querySelectorAll("button, input")]
          .filter((n) => n.getClientRects().length && n.type !== "radio" && n.getBoundingClientRect().height < 44)
          .map((n) => n.id || n.textContent),
      );
      assert.deepEqual(short, []);
    }
    await shot("card-sheet");

    // Refused: the sheet says why, and the board's own word puts it back.
    await page.evaluate(() => {
      fixture.editDisposition = "rejected";
    });
    // `check()` would insist it stays checked; a refusal is the board saying no.
    await sheet.getByRole("radio", { name: "IN PROGRESS", exact: true }).click();
    await until(page, () => document.querySelector("#card-sheet .compose-error")?.textContent.includes("Not moved"));
    assert.equal(
      await sheet.locator(".compose-error").textContent(),
      "Not moved: worktree unmerged — merge before DONE",
    );
    await until(page, () => document.querySelector("#card-sheet input[value=DONE]").checked);
    await shot("card-sheet-refused");
    await page.locator("#card-done").click();
    await sheet.waitFor({ state: "hidden" });
    assert.equal(await page.evaluate(() => document.activeElement.id), "card-line");
    assert.deepEqual(await page.locator("#card-line .tag").allTextContents(), ["FEATURE"]);
    await page.evaluate(() => {
      fixture.editDisposition = "edited";
    });
    assert.equal((await edits()).length, 7);

    // Where the columns sit side by side, a card drags to its place.
    if (size !== "desktop")
      assert.equal(await page.locator(".ticket.card[draggable=true]").count(), 0, "stacked columns move through the sheet");
    else {
      await overview();
      await closeOverlay();
      assert.equal(await page.locator('.ticket.card[data-id="ticket-5"]').getAttribute("draggable"), "true");
      await page.locator('.ticket.card[data-id="ticket-5"]').dragTo(page.locator('.ticket.card[data-id="ticket-0"]'), {
        targetPosition: { x: 40, y: 4 },
      });
      assert.deepEqual(await lastEdit(), { op: "move", ticket: "ticket-5", column: "IN PROGRESS", before: "ticket-0" });
      await until(page, () =>
        document.querySelector('.column[aria-label="IN PROGRESS"] .ticket.card')?.dataset.id === "ticket-5");
      await toast("Moved T-5 to IN PROGRESS");
      // A reorder in its own column is a move to its slot there, and a
      // drop where the card already is sends nothing.
      const todo = await order("TODO");
      await page.locator(`.ticket.card[data-id="${todo[1]}"]`).dragTo(column("TODO").locator(".column-label"));
      assert.deepEqual(await lastEdit(), { op: "move", ticket: todo[1], column: "TODO", before: todo[0] });
      await until(page, (want) => document.querySelector('.column[aria-label="TODO"] .ticket.card')?.dataset.id === want, todo[1]);
      const sent = (await edits()).length;
      await page.locator(`.ticket.card[data-id="${todo[1]}"]`).dragTo(column("TODO").locator(".column-label"));
      assert.equal((await edits()).length, sent, "a drop where the card already is sends nothing");
      assert.equal(await page.locator(".drop-target, .drop-before, .drop-end").count(), 0, "no marker outlives the drag");
      await shot("dragged");
    }

    // The terminal away: no title to press, no line to open.
    await overview();
    await closeOverlay();
    if (size === "phone") await page.locator('.column-tab[data-column="DONE"]').click();
    await page.locator('.ticket[data-id="ticket-3"]').click();
    await page.locator("#rename").waitFor();
    await page.evaluate(() => {
      fixture.refuse = true;
      fixture.channel().close();
    });
    await until(page, () => document.querySelector("#shell").dataset.link === "asleep");
    assert.equal(await page.locator("#rename, #card-line").count(), 0);
    assert.equal(await page.locator("#selection").textContent(), "Renamed from the phone");
    if (size === "desktop")
      assert.equal(await page.locator(".ticket.card[draggable=true]").count(), 0, "nothing drags while away");
    // An older host, live again, offers none of it.
    await page.evaluate(() => {
      fixture.refuse = false;
      fixture.features = fixture.features.filter((f) => !["rename", "move", "tag"].includes(f));
    });
    await connected();
    assert.equal(await page.locator("#rename, #card-line, .ticket.card[draggable=true]").count(), 0);
    assert.deepEqual(await page.locator("#detail .chips .chip").allTextContents(), ["DONE"]);
    assert.deepEqual(errors, []);
    console.log(`${engineName} ${size}: rename, the column-and-tags sheet, refusal, drag and drop, away and an older host passed`);
  } catch (error) {
    await page.screenshot({ path: path.join(root, "test-results", `${engineName}-${size}-edit-failure.png`) });
    throw error;
  } finally {
    await context.close();
  }
}

// Where a ticket's code lives (T-642), as the TUI says it and sets it: a
// worktree's mark on its card and its branch row on its page, a choice not
// cut yet as a chip, and the card sheet's Workspace choice while it is open,
// refused in the host's words and said rather than offered once settled.
async function workspaceFlow(browser, engineName, size, viewport) {
  const context = await browser.newContext({ viewport, colorScheme: "dark", reducedMotion: "reduce", serviceWorkers: "block" });
  await context.addInitScript(fixture);
  await context.addInitScript(() => {
    window.fixture.features.push("rename", "move", "tag", "workspace");
    window.fixture.tickets[3].workspace = { kind: "shared", open: true };
    window.fixture.tickets[5].workspace = { kind: "worktree", branch: "msmn/T-5-agent-task-5", state: "ahead", ahead: 2 };
  });
  await context.route("**/pkg/mesimon_web.js", (route) =>
    route.fulfill({ contentType: "text/javascript", body: fakeCrypto }),
  );
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(String(error)));
  const overview = async () => {
    if (size === "phone" && (await page.locator("#back").isVisible())) await page.locator("#back").click();
    if (size === "desktop" && (await page.locator(".detail-scrim").count()))
      await page.locator(".detail-scrim").click({ position: { x: 10, y: 10 } });
  };
  const shot = (name) =>
    page.screenshot({ path: path.join(root, "test-results", `${engineName}-${size}-${name}.png`) });
  const lastEdit = () => page.evaluate(() => fixture.edits.at(-1));
  const sheet = page.locator("#card-sheet");
  const mark = (id) => page.locator(`.ticket.card[data-id="${id}"] .wt-mark`);
  try {
    await page.goto(origin);
    await until(page, () => !document.querySelector("#pair").disabled);
    await page.getByLabel("Pairing code", { exact: true }).fill("fixture-pair-code");
    await page.getByRole("button", { name: "Connect", exact: true }).click();
    await page.locator('.ticket[data-id="ticket-0"]').waitFor();
    await page.locator('button[data-mode="board"]').locator("visible=true").click();
    if (size === "phone") await page.locator('.column-tab[data-column="TODO"]').click();

    // A cut worktree marks its card as the TUI's does; a shared checkout none.
    assert.equal(await mark("ticket-5").textContent(), "↑commits to merge");
    assert.match(await mark("ticket-5").getAttribute("class"), /wt-ready/);
    assert.equal(await mark("ticket-3").count(), 0);

    // Its page says the branch and what it waits on, and the sheet says
    // why the choice is settled rather than offering it.
    await page.locator('.ticket[data-id="ticket-5"]').click();
    await page.locator("#workspace-line").waitFor();
    assert.equal(await page.locator("#workspace-line .workspace-branch").textContent(), "msmn/T-5-agent-task-5");
    assert.equal(await page.locator("#workspace-line .workspace-state").textContent(), "2 to merge");
    assert.equal(await page.locator("#card-line .chip-workspace").count(), 0);
    await shot("workspace-cut");
    await page.locator("#card-line").click();
    await sheet.waitFor({ state: "visible" });
    assert(await sheet.getByRole("radio", { name: "Shared checkout", exact: true }).isDisabled());
    assert(await sheet.getByRole("radio", { name: "Own worktree", exact: true }).isChecked());
    assert.equal(await page.locator("#card-workspace .field-note").textContent(), "Its worktree is cut, so this stays.");
    await page.locator("#card-done").click();
    await sheet.waitFor({ state: "hidden" });

    // A ticket not started: its choice is a chip, and the sheet sets it at
    // the press. The card wears the pick at once.
    await overview();
    await page.locator('.ticket[data-id="ticket-3"]').click();
    await page.locator("#card-line .chip-workspace").waitFor();
    assert.equal(await page.locator("#card-line .chip-workspace").textContent(), "shared");
    assert.equal(await page.locator("#workspace-line").count(), 0);
    assert.equal(await page.locator("#card-line-hint").textContent(), "Move or tag this ticket, or choose its workspace");
    await page.locator("#card-line").click();
    await sheet.waitFor({ state: "visible" });
    assert(await sheet.getByRole("radio", { name: "Shared checkout", exact: true }).isChecked());
    await sheet.getByRole("radio", { name: "Own worktree", exact: true }).check();
    assert.deepEqual(await lastEdit(), { op: "workspace", ticket: "ticket-3", worktree: true });
    await until(page, () => document.querySelector("#card-line .chip-workspace").textContent === "worktree");
    assert.equal(await page.locator("#card-workspace .field-note").textContent(), "Its worktree is cut when its agent starts.");
    if (size !== "phone" || !(await page.locator("#back").isVisible()))
      await until(page, () => document.querySelector('.ticket.card[data-id="ticket-3"] .wt-mark')?.classList.contains("wt-dormant"));
    if (size === "phone") {
      const short = await sheet.evaluate((node) =>
        [...node.querySelectorAll("button, input")]
          .filter((n) => n.getClientRects().length && n.type !== "radio" && n.getBoundingClientRect().height < 44)
          .map((n) => n.id || n.textContent),
      );
      assert.deepEqual(short, []);
    }
    await shot("workspace-sheet");

    // Refused: the sheet says why in the host's words, and the board's own
    // word puts the pick back.
    await page.evaluate(() => {
      fixture.editDisposition = "rejected";
      fixture.editRefusal = "workspace locked — an agent is running on this ticket";
    });
    await sheet.getByRole("radio", { name: "Shared checkout", exact: true }).click();
    await until(page, () => document.querySelector("#card-sheet .compose-error")?.textContent.includes("Workspace not changed"));
    assert.equal(
      await sheet.locator(".compose-error").textContent(),
      "Workspace not changed: workspace locked — an agent is running on this ticket",
    );
    await until(page, () => document.querySelector("#card-sheet input[value=worktree]").checked);
    await page.locator("#card-done").click();
    await sheet.waitFor({ state: "hidden" });

    // Once its agent runs the host closes the choice: the chip stays, and
    // the sheet no longer offers it.
    await page.evaluate(() => {
      fixture.tickets[3].workspace.open = false;
      fixture.update();
    });
    await page.locator("#card-line").click();
    await sheet.waitFor({ state: "visible" });
    await until(page, () => document.querySelector("#card-workspace").disabled);
    assert.equal(await page.locator("#card-workspace .field-note").textContent(), "Its agent is running there, so this stays.");
    await page.locator("#card-done").click();
    await sheet.waitFor({ state: "hidden" });

    // An older host sends no workspace and takes none: nothing is said.
    await page.evaluate(() => {
      for (const t of fixture.tickets) delete t.workspace;
      fixture.features = fixture.features.filter((f) => f !== "workspace");
      fixture.channel().close();
    });
    await until(page, () => document.querySelector("#connection").textContent === "Connected" && !document.querySelector(".chip-workspace"));
    await page.locator("#card-line").click();
    await sheet.waitFor({ state: "visible" });
    assert.equal(await page.locator("#card-workspace").count(), 0);
    await page.locator("#card-done").click();
    assert.equal(await page.locator(".wt-mark, #workspace-line").count(), 0);
    assert.deepEqual(errors, []);
    console.log(`${engineName} ${size}: the worktree mark, the branch row, the workspace choice, its refusal and lock, and an older host passed`);
  } catch (error) {
    await page.screenshot({ path: path.join(root, "test-results", `${engineName}-${size}-workspace-failure.png`) });
    throw error;
  } finally {
    await context.close();
  }
}

// A pairing QR (T-497): the code arrives in the link's fragment, fills the
// field, leaves the address bar, and still waits for Connect.
// A batch, or a question that takes several choices, is answered whole on
// the ticket (T-571): radios and ticks, words in place of either, one Submit
// carrying one answer per question, the T-567 receipts, the form kept through
// a retry, and an older host leaving the dialog to the pane.
// The conversation (T-626): a page at a time from the transcript, newest at
// the bottom; scrolling up puts the page before on top without moving what
// the reader looks at; the tail ask brings only what was written since; and
// the pane's screen is one toggle away.
async function chatFlow(browser, engineName, size, viewport) {
  const context = await browser.newContext({ viewport, colorScheme: "dark", reducedMotion: "reduce", serviceWorkers: "block" });
  await context.addInitScript(fixture);
  await context.addInitScript(() => {
    window.fixture.features.push("transcript");
    // Tall enough that one page overflows the panel at every size: a page
    // that does not is followed by the one before it by itself.
    const words = (i) => i === 37
      ? `Reply ${i}: the table.\n\n| Check | Result |\n|:--|--:|\n| \`cargo ut\` | **green** |\n| clippy | ~~red~~ green |\n\n> [!NOTE]\n> quoted\n\n- [x] built\n  - nested\n\n${"Long enough to wrap across the panel. ".repeat(16)}`
      : `Reply ${i}: **done** with part ${i}.\n\n${"Long enough to wrap across the panel. ".repeat(16)}`;
    window.fixture.transcript = Array.from({ length: 40 }, (_, i) => ({
      at: i * 10,
      kind: i % 4 === 0 ? "prompt" : i % 4 === 3 ? "tool" : "reply",
      text: i % 4 === 0 ? `Prompt ${i}` : i % 4 === 3 ? `Bash cargo test ${i}` : words(i),
    }));
  });
  await context.route("**/pkg/mesimon_web.js", (route) =>
    route.fulfill({ contentType: "text/javascript", body: fakeCrypto }),
  );
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(String(error)));
  const chat = page.locator("#chat");
  const rowCount = () => page.evaluate(() => document.querySelectorAll("#chat .chat-row:not(.chat-doing)").length);
  const asks = () => page.evaluate(() => fixture.transcriptAsks);
  try {
    await page.goto(origin);
    await until(page, () => !document.querySelector("#pair").disabled);
    await page.getByLabel("Pairing code", { exact: true }).fill("fixture-pair-code");
    await page.getByRole("button", { name: "Connect", exact: true }).click();
    await page.locator('.ticket[data-id="ticket-0"]').waitFor();
    await page.evaluate(() => {
      fixture.tickets[0].agent.doing = "Run the whole suite";
      fixture.update();
    });
    await page.locator('.ticket[data-id="ticket-0"]').click();
    await until(page, () => document.querySelector("#chat")?.textContent.includes("Bash cargo test 39"));
    // The tail page: eight rows, the agent's step under the last, the
    // reply's markdown drawn, and the page before waiting above.
    assert.equal(await rowCount(), 8);
    assert((await chat.locator(".chat-doing").textContent()).includes("Run the whole suite"));
    assert(await chat.locator(".chat-reply strong").first().isVisible());
    // A reply's markdown reads as at the desk: a table, a quote, a list.
    assert.deepEqual(await chat.locator(".md-table th").allTextContents(), ["Check", "Result"]);
    assert.equal(await chat.locator(".md-table td.md-right").first().textContent(), "green");
    assert.equal(await chat.locator(".md-table td code").textContent(), "cargo ut");
    assert.equal(await chat.locator(".md-quote strong").textContent(), "Note");
    assert.equal(await chat.locator(".md-item.md-d1").textContent(), "•nested");
    // The newest row sits on the panel's floor, no gap under it.
    assert(await chat.evaluate((n) => {
      const last = [...n.querySelectorAll(".chat-row")].at(-1).getBoundingClientRect();
      return n.getBoundingClientRect().bottom - last.bottom < 24;
    }), "no gap under the newest row");
    assert((await chat.textContent()).includes("Scroll up for earlier"));
    assert.equal((await asks())[0].before, undefined);
    assert.equal(await page.locator("#preview").count(), 0, "the conversation, not the screen");
    assert(await chat.evaluate((n) => n.scrollHeight - n.clientHeight - n.scrollTop < 24), "newest at the bottom");
    // Scrolling up asks for the page before; it lands on top, and the row
    // the reader had in view stays where it was.
    await page.evaluate(() => (fixture.transcriptHold = true));
    const before = await chat.evaluate((n) => {
      n.scrollTop = 0;
      const first = n.querySelector(".chat-row");
      first.dataset.mark = "kept";
      return first.getBoundingClientRect().top - n.getBoundingClientRect().top;
    });
    await until(page, () => fixture.heldTranscript.length > 0);
    await page.evaluate(() => {
      fixture.transcriptHold = false;
      fixture.releaseTranscript();
    });
    await until(page, () => document.querySelectorAll("#chat .chat-row:not(.chat-doing)").length >= 16);
    const older = (await asks()).find((a) => a.before != null);
    assert.equal(older.before, 320, "the page before the held one");
    const after = await chat.evaluate((n) =>
      n.querySelector('[data-mark="kept"]').getBoundingClientRect().top - n.getBoundingClientRect().top);
    assert(Math.abs(after - before) < 2, `the row stayed put: ${before} → ${after}`);
    // What was written since is asked by itself and appended; the rows held
    // stay the same elements.
    await chat.evaluate((n) => (n.scrollTop = n.scrollHeight));
    const held = await rowCount();
    await page.evaluate(() => {
      fixture.transcript.push({ at: 400, kind: "reply", text: "The newest words" }, { at: 410, kind: "tool", text: "Read main.rs" });
    });
    await until(page, () => document.querySelector("#chat").textContent.includes("The newest words"));
    const tail = (await asks()).filter((a) => a.after != null).at(-1);
    assert.deepEqual([tail.after, tail.conversation], [400, "conversation-a"]);
    assert.equal(await rowCount(), held + 2);
    assert.equal(await chat.locator('[data-mark="kept"]').count(), 1, "the held rows were kept, not redrawn");
    // A prompt sent from here is a ghost under the last row, with no
    // "submitted" line, until the conversation holds it.
    await page.locator("#prompt").fill("ghost-canary");
    await page.locator('input[name="prompt-mode"][value="steer"]').check();
    await page.locator("#send").click();
    await until(page, () => document.querySelector("#chat .chat-ghost")?.textContent.includes("ghost-canary"));
    assert(await page.locator("#delivery").isHidden());
    await page.evaluate(() => fixture.transcript.push({ at: 420, kind: "prompt", text: "ghost-canary" }));
    await until(page, () => !document.querySelector("#chat .chat-ghost"));
    assert.equal(await chat.locator(".chat-prompt").last().textContent(), "ghost-canary");
    assert(await page.locator("#delivery").isHidden());
    // A new conversation (`/clear`) starts over at its tail.
    await page.evaluate(() => {
      fixture.conversation = "conversation-b";
      fixture.transcript = [{ at: 0, kind: "notice", text: "conversation cleared" }, { at: 10, kind: "prompt", text: "Fresh start" }];
    });
    await until(page, () => document.querySelector("#chat").textContent.includes("Fresh start"));
    assert.equal(await rowCount(), 2);
    assert((await chat.textContent()).includes("Start of the conversation"));
    // The raw view is the pane's screen, and the choice is remembered.
    assert.equal(await page.locator("#output-view").textContent(), "Raw");
    await page.locator("#output-view").click();
    await until(page, () => document.querySelector("#preview")?.textContent.includes("line 49"));
    assert.equal(await chat.count(), 0);
    assert.equal(await page.evaluate(() => localStorage.getItem("mesophon-output")), "raw");
    assert.equal(await page.locator("#output-view").textContent(), "Chat");
    await page.locator("#output-view").click();
    await chat.waitFor();
    // A parked agent's conversation is still its file; its screen is gone.
    await page.evaluate(() => {
      fixture.tickets[0].agent.state = "sleeping";
      fixture.update();
    });
    await until(page, () => !document.querySelector("#output-view"));
    assert(await chat.isVisible());
    assert.deepEqual(errors, []);
    console.log(`${engineName} ${size}: conversation pages, prepend, tail, new conversation and raw toggle passed`);
  } catch (error) {
    await page.screenshot({ path: path.join(root, "test-results", `${engineName}-${size}-chat-failure.png`) });
    throw error;
  } finally {
    await context.close();
  }
}

async function batchFlow(browser, engineName, size, viewport) {
  const context = await browser.newContext({ viewport, colorScheme: "dark", reducedMotion: "reduce", serviceWorkers: "block" });
  await context.addInitScript(fixture);
  await context.addInitScript(() => window.fixture.features.push("dialog_multi"));
  await context.route("**/pkg/mesimon_web.js", (route) =>
    route.fulfill({ contentType: "text/javascript", body: fakeCrypto }),
  );
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(String(error)));
  const attention = page.locator("#attention");
  const pick = (role, name) => attention.getByRole(role, { name, exact: true });
  const button = (name) => attention.getByRole("button", { name, exact: true });
  const sent = () => page.evaluate(() => fixture.requests.filter((r) => r.op === "dialog").at(-1));
  const receipt = () => page.evaluate(() => ({
    text: document.querySelector("#delivery").textContent,
    ticks: document.querySelectorAll("#delivery .tick path").length,
  }));
  const connected = () =>
    until(page, () => document.querySelector("#connection").textContent === "Connected");
  try {
    await page.goto(origin);
    await until(page, () => !document.querySelector("#pair").disabled);
    await page.getByLabel("Pairing code", { exact: true }).fill("fixture-pair-code");
    await page.getByRole("button", { name: "Connect", exact: true }).click();
    await page.locator('.ticket[data-id="ticket-0"]').waitFor();
    await page.evaluate(() => {
      const q = (question, multiSelect, ...labels) => ({ question, header: question, multiSelect,
        options: labels.map((label) => ({ label, description: `${label} paint` })) });
      fixture.tickets = [{ id: "batch", key: "T-B", title: "Three questions", column: "IN PROGRESS",
        agent: { session: "batch-session", provider: "claude", state: "needs attention", promptable: true,
          dialog: { request: "toolu_b", kind: "questions", questions: [
            q("Which color?", false, "Blue", "Green"),
            q("Which toppings?", true, "Cheese", "Olives", "Basil"),
            q("Which size?", false, "Small", "Large"),
          ] } } }];
      fixture.update();
    });
    // The needs-you card answers one question in place; this one opens the ticket.
    await page.getByRole("button", { name: "Answer in the ticket", exact: true }).click();
    await until(page, () => document.querySelector("#attention")?.textContent.includes("Which toppings?"));
    assert.equal(await attention.getByRole("radio").count(), 4);
    assert.equal(await attention.getByRole("checkbox").count(), 3);
    assert(await button("Submit answers").isDisabled(), "every question needs an answer");
    await pick("radio", "Green").click();
    await pick("checkbox", "Cheese").click();
    await pick("checkbox", "Basil").click();
    await pick("checkbox", "Olives").click();
    await pick("checkbox", "Olives").click();
    assert.equal(await pick("radio", "Green").getAttribute("aria-checked"), "true");
    assert.equal(await pick("checkbox", "Olives").getAttribute("aria-checked"), "false");
    assert(await button("Submit answers").isDisabled(), "the size is still open");
    await attention.locator('[data-question="2"] input').fill("Large please");
    await page.locator("#attention").screenshot({ path: path.join(root, "test-results", `${engineName}-${size}-batch.png`) });
    await button("Submit answers").click();
    assert.deepEqual(await sent(), {
      op: "dialog", request: "toolu_b", ticket: "batch", session: "batch-session",
      response: { answer: "answers", answers: [
        { answer: "choice", index: 1 },
        { answer: "choices", indices: [0, 2] },
        { answer: "text", text: "Large please" },
      ] },
    });
    // T-567's receipts: keys not confirmed, a reason, and the hook's word;
    // the form keeps its picks through each.
    await until(page, () => document.querySelector("#delivery").textContent.includes("not confirmed"));
    assert.equal((await receipt()).ticks, 1);
    assert.equal(await pick("checkbox", "Basil").getAttribute("aria-checked"), "true");
    await page.evaluate(() => { fixture.dialogReply = { result: "delivery", status: "unknown", reason: "answer_differs" }; });
    await button("Submit answers").click();
    await until(page, () => document.querySelector("#delivery").textContent.includes("Could not answer"));
    assert.match((await receipt()).text, /the pane's answers differ from yours, so Submit was not pressed · try again or answer in the pane/);
    assert(await button("Submit answers").isEnabled());
    await page.evaluate(() => { fixture.dialogReply = { result: "delivery", status: "answered" }; });
    await button("Submit answers").click();
    await until(page, () => document.querySelector("#delivery").textContent.includes("Answered."));
    assert.equal((await receipt()).ticks, 2);
    // Words in place of the ticks: the ticks step aside and the words go.
    await page.evaluate(() => { fixture.dialogReply = undefined; });
    await attention.locator('[data-question="1"] input').fill("Pineapple");
    assert(await pick("checkbox", "Cheese").isDisabled());
    await button("Submit answers").click();
    assert.deepEqual((await sent()).response.answers[1], { answer: "text", text: "Pineapple" });
    await button("Decline questions").click();
    assert.deepEqual((await sent()).response, { answer: "reject" });
    // One question with several choices alone: ticks and one Submit.
    await page.evaluate(() => {
      fixture.tickets[0].agent.dialog = { request: "toolu_m", kind: "questions", questions: [
        { question: "Which regions?", header: "Regions", multiSelect: true,
          options: [{ label: "Europe", description: "" }, { label: "America", description: "" }] }] };
      fixture.update();
    });
    await until(page, () => document.querySelector("#attention").textContent.includes("Which regions?"));
    await pick("checkbox", "America").click();
    await button("Submit answer").click();
    assert.deepEqual((await sent()).response, { answer: "answers", answers: [{ answer: "choices", indices: [1] }] });
    // An older host, live again, leaves the dialog to the pane.
    await page.evaluate(() => {
      fixture.refuse = true;
      fixture.channel().close();
    });
    await until(page, () => document.querySelector("#shell").dataset.link === "asleep");
    assert(await pick("checkbox", "America").isDisabled(), "away, the form waits");
    await page.evaluate(() => {
      fixture.refuse = false;
      fixture.features = fixture.features.filter((f) => f !== "dialog_multi");
    });
    await connected();
    await until(page, () => document.querySelector("#attention").textContent.includes("needs a local answer in the pane"));
    assert.equal(await attention.getByRole("checkbox").count(), 0);
    assert.deepEqual(errors, []);
    console.log(`${engineName} ${size}: a batch and a several-choice question answered whole, receipts and an older host passed`);
  } catch (error) {
    await page.screenshot({ path: path.join(root, "test-results", `${engineName}-${size}-batch-failure.png`) });
    throw error;
  } finally {
    await context.close();
  }
}

async function pairLinkFlow(browser, engineName) {
  const context = await browser.newContext({ viewport: { width: 390, height: 844 }, colorScheme: "dark", serviceWorkers: "block" });
  await context.addInitScript(fixture);
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(String(error)));
  const code = "7K2M-QX4P-0B9D-RT6W-HN3C-5VJE-8FGA-1YSZ";
  try {
    await page.goto(`${origin}/#pair=${code}`);
    await until(page, (code) => document.querySelector("#code").value === code, code);
    assert.equal(await page.evaluate(() => location.hash), "", "the code leaves the address bar");
    assert.match(await page.locator("#connection").textContent(), /filled in from your terminal’s QR code/);
    assert.equal(await page.evaluate(() => fixture.sockets.some((s) => s.channel)), false, "nothing pairs by itself");
    assert.equal(await page.evaluate(() => document.activeElement.id), "pair", "Connect is one tap away");
    await page.getByRole("button", { name: "Connect", exact: true }).click();
    await page.locator('.ticket[data-id="ticket-0"]').waitFor();
    // Scanned again with a board open: back to pairing, the board one tap away.
    await page.evaluate((code) => {
      location.hash = `pair=${code}`;
    }, code);
    await until(page, () => !document.querySelector("#onboarding").hidden);
    assert.equal(await page.locator("#code").inputValue(), code);
    assert(await page.locator("#cancel-pair").isVisible());
    await page.locator("#cancel-pair").click();
    await until(page, () => !document.querySelector("#shell").hidden && document.querySelector("#onboarding").hidden);
    assert.deepEqual(errors, []);
    console.log(`${engineName}: pairing from a QR link passed`);
  } catch (error) {
    await page.screenshot({ path: path.join(root, "test-results", `${engineName}-pair-link-failure.png`) });
    throw error;
  } finally {
    await context.close();
  }
}

// Home screen and no signal (T-497): the service worker keeps the page, so a
// load with no network opens the kept copy. It shows the remembered board,
// takes a ticket with a clock, opens no socket, and becomes the relay's own
// page when the network is back, where the ticket goes out.
async function keptFlow(browser, engineName) {
  const context = await browser.newContext({ viewport: { width: 390, height: 844 }, colorScheme: "dark", reducedMotion: "reduce" });
  await context.addInitScript(fixture);
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(String(error)));
  const sheet = page.locator("#new-ticket-sheet");
  try {
    await page.goto(origin);
    const manifest = await page.evaluate(async () => (await fetch(document.querySelector('link[rel="manifest"]').href)).json());
    assert.equal(manifest.display, "standalone");
    assert.deepEqual(manifest.icons.map((i) => i.purpose), ["any", "any", "maskable"]);
    await until(page, () => !document.querySelector("#pair").disabled);
    await page.getByLabel("Pairing code", { exact: true }).fill("fixture-pair-code");
    await page.getByRole("button", { name: "Connect", exact: true }).click();
    await page.locator('.ticket[data-id="ticket-0"]').waitFor();
    await until(page, async () =>
      !!navigator.serviceWorker.controller && (await caches.keys()).some((name) => name.startsWith("mesophon-page-")),
    );
    // The board is remembered on disk before the network goes.
    await until(
      page,
      () =>
        new Promise((resolve) => {
          const open = indexedDB.open("mesophon", 1);
          open.onsuccess = () => {
            const get = open.result.transaction("device").objectStore("device").get("board:board-a");
            get.onsuccess = () => resolve(!!get.result?.snapshot);
          };
        }),
    );
    networkDown = true;
    await page.evaluate(() => localStorage.setItem("fixture-relay-down", "1"));
    await page.reload();
    await until(page, () => document.documentElement.dataset.page === "kept");
    // The ticket that was open, from memory; back on the list, the rest.
    await until(page, () => document.querySelector("#detail .selection-key")?.textContent === "T-0");
    if (await page.locator("#back").isVisible()) await page.locator("#back").click();
    await page.locator('.ticket[data-id="ticket-0"]').waitFor();
    assert.match(await page.locator("#work-list").textContent(), /As of/);
    assert.equal(await page.locator("#shell").getAttribute("data-link"), "relay");
    assert.equal(await page.evaluate(() => fixture.sockets.length), 0, "the kept page opens no socket");
    await page.locator("#new-ticket-fab").click();
    await sheet.waitFor({ state: "visible" });
    assert.match(await sheet.locator(".compose-dest").textContent(), /stays in this browser/);
    await page.locator("#new-title").fill("Written on the kept page");
    await page.locator("#send-ticket").click();
    await sheet.waitFor({ state: "hidden" });
    await page.locator('button[data-mode="sent"]').locator("visible=true").click();
    await page.locator('.sent-item[data-status="local"]').waitFor();
    await page.screenshot({ path: path.join(root, "test-results", `${engineName}-kept-page.png`) });
    await page.evaluate(() => localStorage.removeItem("fixture-relay-down"));
    // The page asks past the worker every few seconds, and reloads itself.
    const reloaded = page.waitForEvent("load", { timeout: 20000 });
    networkDown = false;
    await reloaded;
    assert.equal(await page.evaluate(() => document.documentElement.dataset.page), undefined);
    await until(page, () => document.querySelector("#detail .selection-key")?.textContent === "T-0");
    if (await page.locator("#back").isVisible()) await page.locator("#back").click();
    await page.locator('button[data-mode="sent"]').locator("visible=true").click();
    await page.locator('.sent-item[data-status="landed"]').filter({ hasText: "Written on the kept page" }).waitFor();
    assert.deepEqual(errors, []);
    console.log(`${engineName}: the kept page opens with no network and hands its ticket over passed`);
  } catch (error) {
    await page.screenshot({ path: path.join(root, "test-results", `${engineName}-kept-failure.png`) });
    throw error;
  } finally {
    networkDown = false;
    await context.close();
  }
}

try {
  for (const [engineName, engine] of [
    ["chromium", chromium],
    ["webkit", webkit],
  ]) {
    if (process.env.MESOPHON_UX_ENGINE && process.env.MESOPHON_UX_ENGINE !== engineName) continue;
    const browser = await engine.launch();
    try {
      for (const [size, viewport] of [
        ["desktop", { width: 1440, height: 950 }],
        ["tablet", { width: 900, height: 900 }],
        ["phone", { width: 390, height: 844 }],
      ]) {
        if (process.env.MESOPHON_UX_SIZES && !process.env.MESOPHON_UX_SIZES.split(",").includes(size)) continue;
        const context = await browser.newContext({
          viewport,
          colorScheme: "dark",
          reducedMotion: "reduce",
          // `route` does not see a service worker's requests.
          serviceWorkers: "block",
        });
        await context.addInitScript(fixture);
        await context.route("**/pkg/mesimon_web.js", (route) =>
          route.fulfill({ contentType: "text/javascript", body: fakeCrypto }),
        );
        const page = await context.newPage();
        const errors = [];
        page.on("pageerror", (error) => errors.push(String(error)));
        const overview = async () => {
          if (size === "phone" && (await page.locator("#back").isVisible())) {
            await page.locator("#back").click();
            await page.locator("#search").waitFor();
          }
        };
        const select = async (id) => {
          await overview();
          await page.locator(`.ticket[data-id="ticket-${id}"]`).click();
        };
        const mode = (name) =>
          page.locator(`button[data-mode="${name}"]`).locator("visible=true").click();
        const delivery = (value) =>
          page.locator(`input[name="prompt-mode"][value="${value}"]`).check();
        const attention = (name) =>
          page.locator("#attention").getByRole("button", { name, exact: true });
        try {
          await page.goto(origin);
          await until(page, () => !document.querySelector("#pair").disabled);
          assert(await page.locator("#shell").isHidden());
          await page
            .getByLabel("Pairing code", { exact: true })
            .fill("fixture-pair-code");
          await page
            .getByRole("button", { name: "Connect", exact: true })
            .click();
          await page.locator('.ticket[data-id="ticket-0"]').waitFor();
          // The projection's newer facts (T-497): how long an agent has been
          // at it, the step it is on or its last reply line, and tags.
          await page.evaluate(() => {
            fixture.tickets[0].tags = [{ group: 1, name: "BUG", tint: 0 }];
            fixture.tickets[1].tags = [{ group: 1, name: "FEATURE", tint: 6 }];
            Object.assign(fixture.tickets[0].agent, { since: Date.now() - 5 * 60000, doing: "Bash(cargo test -p mesimon-daemon)" });
            Object.assign(fixture.tickets[4].agent, { since: Date.now() - 40 * 60000, said: "Fixed, and three tests pass." });
            Object.assign(fixture.tickets[6].agent, { state: "idle", since: Date.now() - 2 * 3600000 });
            fixture.update();
          });
          await until(page, () =>
            document.querySelector('.ticket[data-id="ticket-0"] .headline-step')?.textContent.includes("cargo test"),
          );
          assert.equal(await page.locator('.ticket[data-id="ticket-0"] .card-agent').textContent(), "claude · working · 5m");
          assert.equal(await page.locator('.ticket[data-id="ticket-4"] .headline').textContent(), "Fixed, and three tests pass.");
          assert.equal(await page.locator('.ticket[data-id="ticket-4"] .card-agent').textContent(), "claude · idle · 40m");
          // Now lists a stopped agent for an hour (T-560); the Board, always.
          assert.equal(
            await page.locator('.group:has(.ticket[data-id="ticket-4"]) .group-label').textContent(),
            "Recently idle1",
          );
          assert.equal(await page.locator('.ticket[data-id="ticket-6"]').count(), 0);
          // A ticket is one card in Now and on the Board (T-533): its tags
          // show in both, a needs-you card wears the same face, and Now adds
          // only the column, which the Board says by where the card stands.
          assert.equal(await page.locator('.ticket[data-id="ticket-0"] .tag').textContent(), "BUG");
          assert.equal(await page.locator('.ticket[data-id="ticket-0"] .ticket-meta').textContent(), "IN PROGRESS·T-0");
          assert.equal(await page.locator('.need .ticket[data-id="ticket-1"] .tag').textContent(), "FEATURE");
          assert.equal(await page.locator('.need .ticket[data-id="ticket-1"] .card-agent').textContent(), "codex · needs you");
          const anatomy = () =>
            page.locator('#tickets .ticket[data-id="ticket-0"]').evaluate((n) =>
              [n, ...n.querySelectorAll("[class]")].map((c) => c.getAttribute("class")),
            );
          const inNow = await anatomy();
          await mode("board");
          if (size === "phone") await page.locator('[data-column="IN PROGRESS"]').click();
          assert.deepEqual(await anatomy(), inNow);
          if (size === "phone") {
            // A swipe across the Board steps the columns (T-624): left for
            // the next, right for the previous, nothing past either end, and
            // a mostly vertical drag is a scroll.
            const swipe = (dx, dy = 0) =>
              page.locator("#tickets").evaluate(
                (node, [dx, dy]) => {
                  const box = node.getBoundingClientRect();
                  const x = box.left + box.width / 2;
                  const y = box.top + 120;
                  const touch = (cx, cy) => ({ clientX: cx, clientY: cy, identifier: 1, target: node });
                  const fire = (type, touches, changed) => {
                    const e = new Event(type, { bubbles: true, cancelable: true });
                    Object.defineProperty(e, "touches", { value: touches });
                    Object.defineProperty(e, "changedTouches", { value: changed });
                    node.dispatchEvent(e);
                  };
                  const start = touch(x, y);
                  fire("touchstart", [start], [start]);
                  fire("touchend", [], [touch(x + dx, y + dy)]);
                },
                [dx, dy],
              );
            const pressed = () => page.locator('.column-tab[aria-pressed="true"]').getAttribute("data-column");
            assert.equal(await page.locator("#tickets").evaluate((n) => getComputedStyle(n).touchAction), "pan-y pinch-zoom");
            await swipe(-120);
            await until(page, () => document.querySelector('.column-tab[aria-pressed="true"]')?.dataset.column === "DONE");
            await swipe(-120);
            await swipe(30, -200);
            await swipe(-20);
            assert.equal(await pressed(), "DONE", "no column past the last, and a scroll or a nudge is not a swipe");
            await swipe(120);
            await swipe(120);
            await until(page, () => document.querySelector('.column-tab[aria-pressed="true"]')?.dataset.column === "TODO");
            await swipe(120);
            assert.equal(await pressed(), "TODO", "no column before the first");
            await swipe(-120);
            await until(page, () => document.querySelector('.column-tab[aria-pressed="true"]')?.dataset.column === "IN PROGRESS");
            assert.deepEqual(await anatomy(), inNow);
          }
          assert.equal(await page.locator('.ticket[data-id="ticket-0"] .ticket-meta').textContent(), "T-0");
          assert.equal(await page.locator('.ticket[data-id="ticket-6"] .card-agent').textContent(), "claude · idle · 2h");
          await page.screenshot({ path: path.join(root, "test-results", `${engineName}-${size}-board.png`) });
          await mode("agents");
          await page.locator(".need").first().waitFor();
          await page.screenshot({ path: path.join(root, "test-results", `${engineName}-${size}-now.png`) });
          await select(0);
          assert.match(await page.locator("#detail .chips").textContent(), /BUG/);
          // The heading is the title alone; the key sits at the right of the
          // tags' line (T-510), and a tag wears its tint as its ground, the
          // TUI's chip. No provider chip: the shin and the output say so.
          assert.equal(await page.locator("#detail .selection-key").textContent(), "T-0");
          assert(!(await page.locator("#selection").textContent()).endsWith("T-0"), "the key left the heading");
          assert.deepEqual(await page.locator("#detail .chips .chip").allTextContents(), ["IN PROGRESS"]);
          assert.equal(await page.locator("#detail .agent-line").count(), 0, "a seated agent needs no line");
          const [tagGround, tint] = await page.locator("#detail .chips .tag.tint-0").evaluate((n) => {
            const hex = getComputedStyle(document.documentElement).getPropertyValue("--tag-0").trim().slice(1);
            const [r, g, b] = hex.match(/../g).map((c) => parseInt(c, 16));
            return [getComputedStyle(n).backgroundColor, `rgb(${r}, ${g}, ${b})`];
          });
          assert.equal(tagGround, tint);
          await until(page, () =>
            document.querySelector("#preview").textContent.includes("line 49"),
          );
          assert.equal(await page.locator("#preview script").count(), 0);
          // The pane as a screen (T-506): the host's width sizes the type,
          // the legend is the time alone, and there is no wrap switch.
          assert.equal(await page.locator("#preview").evaluate((n) => n.style.getPropertyValue("--cols")), "132");
          assert.equal(await page.locator("#preview .screen-rule").count(), 1);
          assert.equal(await page.locator("#wrap").count(), 0);
          await page.screenshot({ path: path.join(root, "test-results", `${engineName}-${size}-ticket.png`) });
          // No legend over the output (T-626): the connection strip says
          // when it is stale, and an older host has no view switch.
          assert.equal(await page.locator(".output .label").count(), 0);
          assert.equal(await page.locator("#output-view").count(), 0);
          await page.locator("#prompt").fill("draft for the first agent");
          await select(1);
          assert.equal(await page.locator("#prompt").inputValue(), "");
          await page.locator("#prompt").fill("draft for the second agent");
          await select(0);
          assert.equal(
            await page.locator("#prompt").inputValue(),
            "draft for the first agent",
          );
          // A periodic 50-line window must not dislodge a reader who scrolled up.
          await page.locator("#preview").evaluate((n) => {
            n.scrollTop = 100;
            n.dispatchEvent(new Event("scroll"));
          });
          const oldOutput = await page.locator("#preview").textContent();
          await page.evaluate(() => {
            fixture.lines = fixture.lines.map((line) => `new ${line}`);
          });
          await until(page, () =>
            document
              .querySelector("#latest")
              .textContent.includes("New preview"),
          );
          assert.equal(await page.locator("#preview").textContent(), oldOutput);
          await select(1);
          await select(0);
          assert(
            Math.abs(
              (await page.locator("#preview").evaluate((n) => n.scrollTop)) -
                100,
            ) < 3,
          );
          await page.locator("#latest").click();
          assert.match(
            await page.locator("#preview").textContent(),
            /^new line/,
          );
          // A delayed reply belongs to its original target, even after switching.
          await page.evaluate(() => {
            fixture.disposition = "hold";
          });
          await page.locator("#send").click();
          await select(1);
          // Reconnect resolves the original receipt; second agent's draft stays intact.
          await page.evaluate(() => {
            fixture.receipt = "submitted";
            fixture.channel().close();
          });
          await until(
            page,
            () =>
              document.querySelector("#connection").textContent === "Connected",
          );
          assert.equal(
            await page.locator("#prompt").inputValue(),
            "draft for the second agent",
          );
          await select(0);
          await until(page, () =>
            document
              .querySelector("#delivery")
              .textContent.includes("Submitted"),
          );
          assert.equal(await page.locator("#prompt").inputValue(), "");
          assert.equal(await page.evaluate(() => fixture.prompts.length), 1);
          // Lost receipt: retain text, query once connected, never replay input.
          await page.locator("#prompt").fill("uncertain delivery");
          await page.locator("#prompt").press("Enter");
          assert.equal(
            await page.locator("#prompt").inputValue(),
            "uncertain delivery\n",
          );
          await page.evaluate(() => {
            fixture.disposition = "disconnect";
            fixture.receipt = "unknown";
          });
          await page.locator("#prompt").press("Control+Enter");
          await until(page, () =>
            document
              .querySelector("#delivery")
              .textContent.includes("Delivery unknown"),
          );
          assert(await page.locator("#send").isDisabled());
          assert(await page.locator("#preview").isVisible());
          await until(
            page,
            () =>
              document.querySelector("#connection").textContent === "Connected",
          );
          assert.equal(
            await page.locator("#prompt").inputValue(),
            "uncertain delivery\n",
          );
          assert.equal(await page.evaluate(() => fixture.prompts.length), 2);
          if (engineName === "chromium" && size === "desktop") {
            // Silent host sleep: an open socket alone is not proof of freshness.
            await page.evaluate(() => {
              fixture.holdAll = true;
            });
            await until(page, () =>
              document
                .querySelector("#connection")
                .textContent.includes("Reconnecting"),
            );
            assert(await page.locator("#send").isDisabled());
            await page.evaluate(() => {
              fixture.holdAll = false;
            });
            await until(
              page,
              () =>
                document.querySelector("#connection").textContent ===
                "Connected",
            );
            assert.equal(await page.evaluate(() => fixture.prompts.length), 2);
          }
          await page.evaluate(() => {
            fixture.tickets[0].agent.session = "replacement";
            fixture.update();
          });
          await page.locator("#review-draft").waitFor();
          assert(await page.locator("#send").isDisabled());
          await page.locator("#review-draft").click();
          await page.evaluate(() => {
            fixture.disposition = "rejected";
          });
          await page.locator("#send").click();
          await until(page, () =>
            document
              .querySelector("#delivery")
              .textContent.includes("Session is no longer promptable"),
          );
          assert.equal(
            await page.locator("#prompt").inputValue(),
            "uncertain delivery\n",
          );
          // Main's queued follow-ups keep their session binding in the split UI.
          await select(1);
          assert(
            await page
              .locator('input[name="prompt-mode"][value="queue"]')
              .isChecked(),
          );
          await page.locator("#prompt").fill("queued follow-up");
          await page.evaluate(() => {
            fixture.disposition = "queued";
            fixture.receipt = undefined;
          });
          await page.locator("#send").click();
          await page.locator("#queued-row").waitFor({ state: "visible" });
          assert.equal(
            await page.locator("#queued-text").textContent(),
            "queued follow-up",
          );
          // T-568: this agent needs attention, so the host would refuse a
          // send: Send now is not offered, and an older host's words stand.
          assert(await page.locator("#send-now").isHidden());
          assert.equal(await page.locator("#queued-meta").textContent(), "Queued · waits for idle");
          assert.equal(await page.locator("#prompt").inputValue(), "");
          await page.evaluate(() => fixture.channel().close());
          await until(
            page,
            () =>
              document.querySelector("#connection").textContent === "Connected",
          );
          await until(page, () =>
            document.querySelector("#delivery").textContent.includes("Queued"),
          );
          assert.equal(
            await page.evaluate(
              () =>
                fixture.prompts.filter((p) => p.text === "queued follow-up")
                  .length,
            ),
            1,
          );
          await page.locator("#prompt").fill("newer draft");
          await page.evaluate(() => {
            fixture.holdTakeBack = true;
          });
          await page.locator("#take-back").click();
          await select(0);
          await page.evaluate(() => fixture.releaseTakeBack());
          assert.equal(
            await page.locator("#prompt").inputValue(),
            "uncertain delivery\n",
          );
          await select(1);
          assert.equal(
            await page.locator("#prompt").inputValue(),
            "newer draft",
          );
          assert.equal(
            await page.locator("#returned-text").textContent(),
            "queued follow-up",
          );
          await page.locator("#swap-returned").click();
          assert.equal(
            await page.locator("#prompt").inputValue(),
            "queued follow-up",
          );
          assert.equal(
            await page.locator("#returned-text").textContent(),
            "newer draft",
          );
          // T-568: at a stop a steer would be the dialog's answer: Steer is
          // off and says why, until the agent works again.
          assert(await page.locator('input[name="prompt-mode"][value="steer"]').isDisabled());
          assert(await page.locator("#steer-why").isVisible());
          await page.evaluate(() => {
            fixture.tickets[1].agent.state = "working";
            fixture.update();
          });
          await until(page, () => document.querySelector("#steer-why").hidden);
          await delivery("steer");
          await page.evaluate(() => {
            fixture.disposition = "submitted";
          });
          await page.locator("#send").click();
          await until(page, () =>
            document
              .querySelector("#delivery")
              .textContent.includes("Submitted"),
          );
          assert.equal(
            await page.evaluate(() => fixture.prompts.at(-1).queued),
            false,
          );
          await delivery("queue");
          await page.locator("#prompt").fill("explicit send now");
          await page.evaluate(() => {
            fixture.disposition = "queued";
          });
          await page.locator("#send").click();
          await page.locator("#queued-row").waitFor({ state: "visible" });
          await page.locator("#prompt").fill("keep this draft");
          await page.locator("#send-now").click();
          await until(page, () =>
            document
              .querySelector("#delivery")
              .textContent.includes("Submitted"),
          );
          assert.equal(
            await page.locator("#prompt").inputValue(),
            "keep this draft",
          );
          assert.deepEqual(
            await page.evaluate(() =>
              fixture.requests.filter((r) => r.op === "send_now").at(-1),
            ),
            { op: "send_now", ticket: "ticket-1", session: "session-1" },
          );
          // Search does not change selection; Board exposes tickets without agents.
          await overview();
          await page.locator("#search").fill("nothing-matches");
          assert.equal(await page.locator(".ticket").count(), 0);
          await page.locator("#search").fill("Agent task 3");
          await mode("board");
          if (size !== "phone") {
            // A column's purpose is its title's hover (T-506): no blurb under
            // the title and no "empty" line, so every column's cards start level.
            assert.equal(
              await page.locator('.column[aria-label="TODO"] .column-label').getAttribute("title"),
              "for work that can and should be done soon",
            );
            assert.equal(await page.locator(".column-about, .column-empty").count(), 0);
          }
          if (size === "phone")
            await page.locator('[data-column="TODO"]').click();
          await select(3);
          assert.equal(
            await page.locator("#agent-state").textContent(),
            "No agent on this ticket. Start one at your terminal.",
          );
          assert.equal(await page.locator("[data-start]").count(), 0, "a host without `start`");
          assert(await page.locator("#send").isDisabled());
          await overview();
          await page.locator("#search").fill("");
          await mode("agents");
          await overview();
          const lowerRow = page.locator('.ticket[data-id="ticket-20"]');
          await lowerRow.scrollIntoViewIfNeeded();
          const listPosition = await page
            .locator("#tickets")
            .evaluate((n) => n.scrollTop);
          await lowerRow.click();
          await overview();
          assert(
            Math.abs(
              (await page.locator("#tickets").evaluate((n) => n.scrollTop)) -
                listPosition,
            ) < 3,
          );
          if (size === "phone") {
            const shortControls = await page.evaluate(() =>
              [
                ...document.querySelectorAll(
                  "button, input:not([type=checkbox]), textarea, select",
                ),
              ]
                .filter(
                  (n) =>
                    n.getClientRects().length &&
                    n.getBoundingClientRect().height < 44,
                )
                .map((n) => n.id),
            );
            assert.deepEqual(shortControls, []);
          }
          await select(2); // long title and 50-line output at every width/theme
          await page.locator("#prompt").fill("A comfortable composer");
          for (const theme of ["graphite", "chalk"]) {
            await page.evaluate((theme) => {
              const control = document.querySelector("#theme");
              control.value = theme;
              control.dispatchEvent(new Event("change"));
            }, theme);
            const contrast = await page.evaluate(() => {
              const style = getComputedStyle(document.documentElement);
              const luminance = (name) => {
                const hex = style.getPropertyValue(name).trim().slice(1);
                const rgb = (
                  hex.length === 3 ? [...hex].map((c) => c + c).join("") : hex
                )
                  .match(/../g)
                  .map((c) => parseInt(c, 16) / 255)
                  .map((c) =>
                    c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4,
                  );
                return rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
              };
              return [
                ["--ink", "--bg"],
                ["--ink3", "--s1"],
                ["--ink2", "--s2"],
                ["--pri-ink", "--pri"],
                ["--attn-ink", "--attn"],
              ].map(([a, b]) => {
                const x = luminance(a),
                  y = luminance(b);
                return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
              });
            });
            assert(
              contrast.every((ratio) => ratio >= 4.5),
              `text contrast: ${contrast}`,
            );
            assert(
              await page.evaluate(
                () => document.documentElement.scrollWidth <= innerWidth,
              ),
            );
            await page.screenshot({
              path: path.join(
                root,
                "test-results",
                `${engineName}-${size}-${theme}.png`,
              ),
            });
          }
          if (size === "desktop") {
            await page.evaluate(() => {
              document.documentElement.style.fontSize = "200%";
            });
            assert(
              await page.evaluate(
                () => document.documentElement.scrollWidth <= innerWidth,
              ),
            );
            await page.locator("#send").scrollIntoViewIfNeeded();
            const zoomed = await page.locator("#send").boundingBox();
            assert(
              zoomed.y >= 0 && zoomed.y + zoomed.height <= viewport.height,
            );
            await page.screenshot({
              path: path.join(root, "test-results", `${engineName}-zoomed.png`),
            });
            await page.evaluate(() => {
              document.documentElement.style.fontSize = "";
            });
          }
          if (size === "phone") {
            await page.setViewportSize({ width: 390, height: 500 }); // keyboard-sized viewport, not a physical phone claim
            await page.locator("#prompt").focus();
            await page.waitForFunction(
              () =>
                document.querySelector("#send").getBoundingClientRect()
                  .bottom <= innerHeight,
            );
            const box = await page.locator("#send").boundingBox();
            assert(box.y >= 0 && box.y + box.height <= 500);
            assert.equal(
              await page
                .locator("#prompt")
                .evaluate((n) => getComputedStyle(n).fontSize),
              "16px",
            );
            assert(box.height >= 44);
            await page.screenshot({
              path: path.join(
                root,
                "test-results",
                `${engineName}-keyboard-viewport.png`,
              ),
            });
            await page.setViewportSize(viewport);
          }
          // Settings is a dialog (T-548): the sidebar's foot opens it, and so
          // does the pill below the desktop breakpoint; the hops are in it.
          await overview();
          const settings = page.locator("#settings-sheet");
          assert(!(await page.locator("#about .hops").isVisible()));
          if (size === "desktop") {
            const foot = await page.locator("#settings").boundingBox();
            const side = await page.locator("#sidebar").boundingBox();
            assert(side.y + side.height - (foot.y + foot.height) < 40, "Settings sits at the sidebar's foot");
            await page.locator("#settings").click();
          } else {
            await page.locator("#board-menu").click();
            await page.locator("#settings").click();
            await page.locator("#sidebar").waitFor({ state: "hidden" });
          }
          await settings.waitFor({ state: "visible" });
          assert.equal(await settings.locator("#about .hop").count(), 3);
          for (const id of ["theme", "alerts", "forget"])
            assert(await settings.locator(`#${id}`).isVisible(), id);
          await page.screenshot({ path: path.join(root, "test-results", `${engineName}-${size}-settings.png`) });
          await page.keyboard.press("Escape");
          await settings.waitFor({ state: "hidden" });
          if (size !== "desktop") {
            await page.locator(".pill-button").click();
            await settings.waitFor({ state: "visible" });
            await page.locator("#settings-done").click();
            await settings.waitFor({ state: "hidden" });
          }
          if (size === "desktop") {
            // The sidebar folds to a rail that stays folded across a reload,
            // and Settings stays at its foot as an icon.
            await page.locator("#side-toggle").click();
            await until(page, () => document.querySelector("#shell").dataset.side === "rail");
            assert((await page.locator("#sidebar").boundingBox()).width < 80);
            assert(await page.locator('#sidebar .mode[data-mode="board"]').isVisible());
            assert(await page.locator("#settings").isVisible());
            await page.screenshot({ path: path.join(root, "test-results", `${engineName}-${size}-rail.png`) });
          }
          await page.reload();
          await until(page, () =>
            document.querySelector("#detail .selection-key")?.textContent === "T-2",
          );
          assert(await page.locator("#onboarding").isHidden());
          assert.equal(
            await page.locator("html").getAttribute("data-theme"),
            "chalk",
          );
          if (size === "desktop") {
            assert.equal(await page.evaluate(() => document.querySelector("#shell").dataset.side), "rail");
            await page.locator("#side-toggle").click();
            await until(page, () => document.querySelector("#shell").dataset.side === "full");
          }
          assert.equal(await page.locator("#prompt").inputValue(), "");
          // M2 cards keep untrusted tool/plan text inert and bind each action
          // to the exact projected session and request.
          await page.evaluate(() => {
            fixture.tickets = [{ id: "m2", key: "T-M2", title: "Needs your answer", column: "IN PROGRESS",
              agent: { session: "m2-session", provider: "claude", state: "needs attention", promptable: true,
                permission: { request: "permission-1", tool: "Bash", input: { command: "<script>window.bad=true</script>" }, expires_at: Date.now() + 40000 } } }];
            fixture.update();
          });
          if (size === "phone" && !(await page.locator("#back").isVisible())) await page.locator("#tickets button").first().click();
          await until(page, () => document.querySelector("#attention").textContent.includes("Approve once"));
          assert.equal(await page.locator("#attention script").count(), 0);
          await attention("Approve once").click();
          assert.deepEqual(await page.evaluate(() => fixture.requests.filter((r) => r.op === "permission").at(-1)),
            { op: "permission", request: "permission-1", decision: "allow", ticket: "m2", session: "m2-session" });
          await page.evaluate(() => {
            fixture.tickets[0].agent.permission = null;
            fixture.tickets[0].agent.dialog = { request: "question-1", kind: "questions", questions: [{ question: "Which color?", header: "Color",
              multiSelect: false, options: [{ label: "Blue", description: "First color" }, { label: "Green", description: "Second color" }] }] };
            fixture.update();
          });
          await attention("Green").click();
          assert.deepEqual(await page.evaluate(() => fixture.requests.filter((r) => r.op === "dialog").at(-1).response), { answer: "choice", index: 1 });
          // T-567: keys nothing confirmed are one tick, and the question
          // stays answerable; a reason keeps it answerable too; only the
          // agent's own hook makes two ticks.
          const receiptLine = () => page.evaluate(() => ({
            text: document.querySelector("#delivery").textContent,
            ticks: document.querySelectorAll("#delivery .tick path").length,
          }));
          await until(page, () => document.querySelector("#delivery").textContent.includes("not confirmed"));
          assert.equal((await receiptLine()).ticks, 1);
          assert(await attention("Green").isEnabled());
          await page.evaluate(() => { fixture.dialogReply = { result: "delivery", status: "unknown", reason: "label_wrapped" }; });
          await attention("Blue").click();
          await until(page, () => document.querySelector("#delivery").textContent.includes("Could not answer"));
          assert.match((await receiptLine()).text, /the option does not read as one row on the screen · try again or answer in the pane/);
          assert.equal((await receiptLine()).ticks, 0);
          assert(await attention("Blue").isEnabled());
          await page.evaluate(() => { fixture.dialogReply = { result: "delivery", status: "answered" }; });
          await attention("Blue").click();
          await until(page, () => document.querySelector("#delivery").textContent.includes("Answered."));
          assert.equal((await receiptLine()).ticks, 2);
          await page.evaluate(() => { fixture.dialogReply = undefined; });
          await page.locator("#attention input").fill("Purple");
          await page.evaluate(() => fixture.update());
          assert.equal(await page.locator("#attention input").inputValue(), "Purple");
          await attention("Send answer").click();
          assert.equal(await page.locator("#attention input").inputValue(), "Purple");
          assert.deepEqual(await page.evaluate(() => fixture.requests.filter((r) => r.op === "dialog").at(-1).response), { answer: "text", text: "Purple" });
          await page.evaluate(() => {
            fixture.tickets[0].agent.dialog = { request: "plan-1", kind: "plan", markdown: "# Plan\\nReview <img src=x onerror=alert(1)> then implement." };
            fixture.update();
          });
          await until(page, () => document.querySelector("#attention").textContent.includes("Review plan"));
          assert.equal(await page.locator("#attention img").count(), 0);
          await page.locator("#attention").screenshot({ path: path.join(root, "test-results", `${engineName}-${size}-plan.png`) });
          await attention("Reject plan").click();
          assert.equal(await page.evaluate(() => fixture.requests.filter((r) => r.op === "dialog").at(-1).request), "plan-1");
          // T-568: words queued for the turn never lock the answer. At the
          // question Steer is off, so the words queue; the receipt says whose
          // words they replaced, and the answer still goes.
          await page.evaluate(() => {
            fixture.tickets[0].agent.dialog = { request: "question-2", kind: "questions", questions: [{ question: "Which shade?", header: "Shade",
              multiSelect: false, options: [{ label: "Light", description: "" }, { label: "Dark", description: "" }] }] };
            fixture.disposition = "queued";
            fixture.replaced = { by: "T-411" };
            fixture.update();
          });
          await until(page, () => document.querySelector("#attention").textContent.includes("Which shade?"));
          assert(await page.locator("#steer-why").isVisible());
          await page.locator("#prompt").fill("after the answer");
          await page.locator("#send").click();
          await page.locator("#queued-row").waitFor({ state: "visible" });
          assert.equal(await page.evaluate(() => fixture.prompts.at(-1).queued), true);
          await until(page, () => document.querySelector("#delivery").textContent.includes("It replaced T-411’s agent’s queued words."));
          assert(await page.locator("#send-now").isHidden());
          assert(await attention("Dark").isEnabled(), "a queued prompt does not lock the answer");
          await page.evaluate(() => { fixture.dialogReply = { result: "delivery", status: "answered" }; });
          await attention("Dark").click();
          await until(page, () => document.querySelector("#delivery").textContent.includes("Answered."));
          assert.deepEqual(await page.evaluate(() => fixture.requests.filter((r) => r.op === "dialog").at(-1)),
            { op: "dialog", request: "question-2", response: { answer: "choice", index: 1 }, ticket: "m2", session: "m2-session" });
          // The row says whose words wait and on what, as the host says it.
          const queuedMeta = (queue, agent) => page.evaluate(([queue, agent]) => {
            Object.assign(fixture.tickets[0], { queue });
            Object.assign(fixture.tickets[0].agent, agent);
            fixture.update();
          }, [queue, agent]);
          const meta = (words) => until(page, (w) => document.querySelector("#queued-meta").textContent === w, words);
          await queuedMeta({ held: "agent asked", asking: ["T-M2"] }, {});
          await meta("held · agent asked · you answer first");
          await page.screenshot({ path: path.join(root, "test-results", `${engineName}-${size}-queued-at-question.png`) });
          await queuedMeta({ held: "agent asked" }, { state: "idle", dialog: null });
          await meta("held · agent asked · you send");
          assert(await page.locator("#send-now").isVisible());
          await queuedMeta({ by: "T-411" }, {});
          await meta("T-411's agent · you send");
          await queuedMeta({ waits: ["T-3"], asking: ["T-3"] }, {});
          await meta("queued · after T-3's answer");
          await page.evaluate(() => {
            fixture.dialogReply = undefined;
            fixture.replaced = undefined;
            fixture.tickets[0].queued = null;
            fixture.tickets[0].queue = undefined;
            fixture.update();
          });
          // Connected phone browsers still receive an in-page alert when the
          // platform cannot construct a system Notification.
          await page.evaluate(async () => {
            const { showAlert } = await import("./awareness.js");
            showAlert({ result: "awareness", ticket: "other", alert: true,
              awareness: { phase: "completed", headline: "T-OTHER · <script>inert</script>", deepLink: "#ticket=other" } },
              "m2", (ticket) => { fixture.alertTarget = ticket; });
          });
          assert.equal(await page.locator("#awareness script").count(), 0);
          await page.locator("#awareness button").first().click();
          assert.equal(await page.evaluate(() => fixture.alertTarget), "other");
          assert(await page.locator("#awareness").isHidden());
          // Swiped away like a phone's own notification (T-628): sideways or
          // up, and the button pressed under the finger opens nothing.
          const alertOther = () => page.evaluate(async () => {
            const { showAlert } = await import("./awareness.js");
            fixture.alertTarget = undefined;
            showAlert({ result: "awareness", ticket: "other", alert: true,
              awareness: { phase: "completed", headline: "T-OTHER · done", deepLink: "#ticket=other" } },
              "m2", (ticket) => { fixture.alertTarget = ticket; });
          });
          const banner = page.locator("#awareness");
          await alertOther();
          await swipe(page, banner, 24, 4);
          assert(await banner.isVisible(), "a short drag snaps back");
          assert.equal(await banner.evaluate((el) => el.style.translate), "");
          await swipe(page, banner, 260, 6);
          await until(page, () => document.querySelector("#awareness").hidden);
          assert.equal(await page.evaluate(() => fixture.alertTarget), undefined);
          await alertOther();
          assert.equal(await banner.evaluate((el) => el.style.translate), "", "the last flight is undone");
          await swipe(page, banner, 0, -60);
          await until(page, () => document.querySelector("#awareness").hidden);
          assert.equal(await page.evaluate(() => fixture.alertTarget), undefined);
          // Zero and one ticket snapshots; selected identity falls back cleanly.
          await page.evaluate(() => {
            fixture.tickets = [];
            fixture.update();
          });
          await until(page, () =>
            document
              .querySelector("#tickets")
              .textContent.includes("no tickets yet"),
          );
          await page.evaluate(() => {
            fixture.tickets = [
              {
                id: "only",
                key: "T-99",
                title: "Only ticket",
                column: "TODO",
                agent: null,
              },
            ];
            fixture.update();
          });
          await until(page, () =>
            document
              .querySelector("#selection")
              .textContent.includes("Only ticket"),
          );
          await page.evaluate(() => fixture.reply({ result: "revoked" }));
          await until(page, () =>
            document
              .querySelector("#connection")
              .textContent.includes("Access revoked"),
          );
          assert.equal(await page.locator("#preview").textContent(), "");
          assert.equal(await page.locator("#queued-text").textContent(), "");
          assert.equal(await page.locator("#returned-text").textContent(), "");
          assert.equal(await page.locator("#tickets").textContent(), "");
          await page.reload();
          await until(page, () =>
            document
              .querySelector("#connection")
              .textContent.includes("Access revoked"),
          );
          assert(await page.locator("#onboarding").isVisible());
          assert.deepEqual(errors, []);
          console.log(
            `${engineName} ${size}: navigation, drafts, scrolling, receipts, reconnect, replacement, themes, empty and revoked passed`,
          );
        } catch (error) {
          await page.screenshot({
            path: path.join(
              root,
              "test-results",
              `${engineName}-${size}-failure.png`,
            ),
          });
          throw error;
        } finally {
          await context.close();
        }
        await ticketFlow(browser, engineName, size, viewport);
        await startFlow(browser, engineName, size, viewport);
        await tierFlow(browser, engineName, size, viewport);
        await editFlow(browser, engineName, size, viewport);
        await workspaceFlow(browser, engineName, size, viewport);
        await batchFlow(browser, engineName, size, viewport);
        await notesFlow(browser, engineName, size, viewport);
        await chatFlow(browser, engineName, size, viewport);
      }
      await pairLinkFlow(browser, engineName);
      await keptFlow(browser, engineName);
    } finally {
      await browser.close();
    }
  }
} finally {
  await new Promise((resolve) => server.close(resolve));
}
