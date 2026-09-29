// Real browsers, controlled M1 wire replies. Crypto/relay acceptance remains in
// browser.test.js; this fixture exercises timing/failure states deterministically.
import { chromium, webkit } from "playwright";
import assert from "node:assert/strict";
import http from "node:http";
import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
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
    creates: [],
    answers: {},
    disposition: "submitted",
    createDisposition: "created",
    // A start (T-498): answered `starting` and the agent put on the ticket,
    // `rejected`, or held unanswered; `run` makes its receipt `started`.
    starts: [],
    startCommands: [],
    startDisposition: "starting",
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
      const n = Object.values(this.mail).filter((x) => x.answer).length + 1;
      const body = m.envelope.body;
      const ticket = { id: `mailed-${n}`, key: `T-${199 + n}`, title: body.title, column: body.column || "TODO", agent: null };
      this.tickets.push(ticket);
      m.answer = { result: "created", ticket: ticket.id, key: ticket.key, column: ticket.column };
      m.sent = true;
      const receipt = { id, sealed: true, answer: m.answer };
      for (const socket of this.mailboxes()) socket.message({ kind: "receipt", board: "board-a", receipt });
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
          if (["permission", "dialog"].includes(request.op)) answer({ result: "delivery", status: "input_sent" });
          if (request.op === "snapshot") answer(state.snapshot());
          if (request.op === "preview")
            answer({ result: "preview", lines: state.lines, cols: 132 });
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
              answer({ result: "delivery", status: state.disposition });
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
    // Now and the board show what is on its way.
    await mode("agents");
    await page.locator(".waiting-row").first().waitFor();
    assert.equal(await page.locator(".waiting-row").count(), 2);
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

// A pairing QR (T-497): the code arrives in the link's fragment, fills the
// field, leaves the address bar, and still waits for Connect.
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
            Object.assign(fixture.tickets[0].agent, { since: Date.now() - 5 * 60000, doing: "Bash(cargo test -p mesimon-daemon)" });
            Object.assign(fixture.tickets[4].agent, { since: Date.now() - 2 * 3600000, said: "Fixed, and three tests pass." });
            fixture.update();
          });
          await until(page, () =>
            document.querySelector('.ticket[data-id="ticket-0"] .headline-step')?.textContent.includes("cargo test"),
          );
          assert.match(await page.locator('.ticket[data-id="ticket-0"] .ticket-meta').textContent(), / · 5m · /);
          assert.equal(await page.locator('.ticket[data-id="ticket-4"] .headline').textContent(), "Fixed, and three tests pass.");
          assert.match(await page.locator('.ticket[data-id="ticket-4"] .ticket-meta').textContent(), / · 2h · /);
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
          assert.doesNotMatch(await page.locator("#freshness").textContent(), /Periodic|50 lines/);
          assert.match(
            await page.locator("#freshness").textContent(),
            /^Last received/,
          );
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
          assert.match(await page.locator("#freshness").textContent(), /Stale/);
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
                .textContent.includes("Disconnected"),
            );
            assert(await page.locator("#send").isDisabled());
            assert.match(
              await page.locator("#freshness").textContent(),
              /Stale/,
            );
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
          if (size === "desktop") {
            // The hops sit under Settings, closed until asked (T-506), and the
            // sidebar folds to a rail that stays folded across a reload.
            assert.equal(await page.locator("#about .hop").count(), 3);
            assert(!(await page.locator("#about .hops").isVisible()));
            await page.locator("#side-toggle").click();
            await until(page, () => document.querySelector("#shell").dataset.side === "rail");
            assert((await page.locator("#sidebar").boundingBox()).width < 80);
            assert(await page.locator('#sidebar .mode[data-mode="board"]').isVisible());
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
