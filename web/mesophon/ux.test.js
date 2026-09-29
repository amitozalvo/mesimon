// Real browsers, controlled M1 wire replies. Crypto/relay acceptance remains in
// browser.test.js; this fixture exercises timing/failure states deterministically.
import { chromium, webkit } from "playwright";
import assert from "node:assert/strict";
import http from "node:http";
import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
const root = path.dirname(fileURLToPath(import.meta.url));
const server = http.createServer(async (req, res) => {
  try {
    const name = req.url === "/" ? "index.html" : req.url.slice(1);
    if (!/^(?:vendor\/|fonts\/)?[a-z0-9.-]+\.(html|css|js|woff2)$/.test(name)) {
      res.writeHead(404).end();
      return;
    }
    res.setHeader(
      "Content-Type",
      name.endsWith("js")
        ? "text/javascript"
        : name.endsWith("css")
          ? "text/css"
          : name.endsWith("woff2")
            ? "font/woff2"
            : "text/html",
    );
    res.end(await fs.readFile(path.join(root, name)));
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
}`;
function fixture() {
  const state = (window.fixture = {
    next: 1,
    features: ["permission", "dialog", "awareness"],
    incarnation: "incarnation-a",
    prompts: [],
    requests: [],
    answers: {},
    disposition: "submitted",
    sockets: [],
    lines: Array.from(
      { length: 50 },
      (_, i) =>
        `line ${String(i).padStart(2, "0")} · actual-sized periodic output <script>never execute</script>`,
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
      };
    },
    reply(reply, id = 0) {
      this.sockets.at(-1).message({ kind: "packet", id, reply });
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
        else if (["pair", "connect"].includes(wire.kind))
          this.message({ kind: "welcome", welcome: { board: "board-a" } });
        else {
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
            answer({ result: "preview", lines: state.lines });
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
async function until(page, fn) {
  await page.waitForFunction(fn);
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
          await select(0);
          await until(page, () =>
            document.querySelector("#preview").textContent.includes("line 49"),
          );
          assert.equal(await page.locator("#preview script").count(), 0);
          assert.match(
            await page.locator("#freshness").textContent(),
            /Last received/,
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
            fixture.sockets.at(-1).close();
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
          await page.evaluate(() => fixture.sockets.at(-1).close());
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
          if (size === "phone")
            await page.locator('[data-column="TODO"]').click();
          await select(3);
          assert.match(
            await page.locator("#agent-state").textContent(),
            /No live agent/,
          );
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
          await page.reload();
          await until(page, () =>
            document
              .querySelector("#selection")
              .textContent.startsWith("T-2 ·"),
          );
          assert(await page.locator("#onboarding").isHidden());
          assert.equal(
            await page.locator("html").getAttribute("data-theme"),
            "chalk",
          );
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
      }
    } finally {
      await browser.close();
    }
  }
} finally {
  await new Promise((resolve) => server.close(resolve));
}
