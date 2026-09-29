// Runs inside the Rust test's supervised fixture, against its real relay.
import { chromium, webkit } from "playwright";
import assert from "node:assert/strict";
import net from "node:net";
import fs from "node:fs/promises";
import path from "node:path";
const origin = process.env.MESOPHON_TEST_ORIGIN;
assert(
  origin && process.env.MESOPHON_TEST_SOCKET,
  "run through the mesophon Rust acceptance test",
);
const localHTTP = origin.startsWith("http:");
function command(command) {
  return new Promise((resolve, reject) => {
    const sock = net.createConnection(process.env.MESOPHON_TEST_SOCKET);
    let data = "";
    sock.setTimeout(10000, () => sock.destroy(new Error("daemon timeout")));
    sock.on("error", reject);
    sock.on("connect", () =>
      sock.write(
        JSON.stringify({ principal: { kind: "local" }, command }) + "\n",
      ),
    );
    sock.on("data", (chunk) => {
      data += chunk;
      const at = data.indexOf("\n");
      if (at >= 0) {
        sock.end();
        resolve(JSON.parse(data.slice(0, at)));
      }
    });
  });
}
for (const [name, engine] of [
  ["chromium", chromium],
  ["webkit", webkit],
]) {
  const browser = await engine.launch();
  try {
    for (const [label, viewport] of [
      ["desktop", { width: 1200, height: 800 }],
      ["phone", { width: 390, height: 844 }],
    ]) {
      const context = await browser.newContext({
        viewport,
        ignoreHTTPSErrors: !localHTTP,
      }); // fixture-only self-signed TLS
      const page = await context.newPage();
      const errors = [];
      page.on("pageerror", (e) => errors.push(String(e)));
      page.on("console", (m) => {
        if (m.type() === "error") console.error(name, m.text());
      });
      try {
        await page.goto(origin);
        assert.equal(await page.title(), "Remote Control");
        assert(
          await page
            .getByRole("heading", { name: "Remote Control", exact: true })
            .isVisible(),
        );
        assert(await page.evaluate(() => isSecureContext));
        if (localHTTP) {
          const rejected = await context.request.get(origin, {
            headers: { Host: "attacker.example" },
          });
          assert.equal(rejected.status(), 403);
          const crossOrigin = await context.request.get(`${origin}/control`, {
            headers: {
              Origin: "http://attacker.example",
              Connection: "Upgrade",
              Upgrade: "websocket",
              "Sec-WebSocket-Version": "13",
              "Sec-WebSocket-Key": "dGhlIHNhbXBsZSBub25jZQ==",
            },
          });
          assert.equal(crossOrigin.status(), 403);
        }
        await page.waitForFunction(() =>
          document
            .querySelector("#connection")
            .textContent.includes("Enable Remote Control"),
        );
        const paired = await command({
          cmd: "mesophon",
          action: { action: "pair" },
        });
        assert(paired.info?.code);
        await page
          .getByLabel("Device name (optional)", { exact: true })
          .fill(`${name}-${label}`);
        await page
          .getByLabel("Pairing code", { exact: true })
          .fill(paired.info.code);
        await page
          .getByRole("button", { name: "Connect", exact: true })
          .click();
        await page
          .getByRole("button", { name: /private-ticket-canary/ })
          .waitFor();
        await page
          .getByRole("button", { name: /private-ticket-canary/ })
          .click();
        await page.waitForFunction(() =>
          document
            .querySelector("#preview")
            .textContent.includes("preview-canary"),
        );
        assert.equal(await page.locator("#preview script").count(), 0);
        assert(await page.getByRole("radio", { name: "Queue", exact: true }).isChecked());
        const prompt = `browser-canary-${name}-${label}`;
        await page.getByLabel("Prompt", { exact: true }).fill(prompt);
        await page
          .getByRole("button", { name: "Queue prompt", exact: true })
          .click();
        await page.locator("#queued-row").waitFor({ state: "visible" });
        assert.equal(await page.locator("#queued-text").textContent(), prompt);
        await page
          .getByRole("button", { name: "Take back", exact: true })
          .click();
        await page.waitForFunction(
          (text) => document.querySelector("#prompt").value === text,
          prompt,
        );
        await page
          .getByRole("button", { name: "Queue prompt", exact: true })
          .click();
        await page.locator("#queued-row").waitFor({ state: "visible" });
        await page
          .getByRole("button", { name: "Send now", exact: true })
          .click();
        await page.waitForFunction(() =>
          document.querySelector("#delivery").textContent.includes("Submitted"),
        );
        // Submitted acknowledges input delivery, not provider execution. The
        // stub writes its receipt before echoing; observing the echo makes
        // the subsequent exactly-once receipt check independent of timing.
        await page.waitForFunction(
          (text) =>
            document.querySelector("#preview").textContent.includes(text),
          prompt,
        );
        const got = await fs.readFile(
          path.join(process.env.MESOPHON_TEST_DIR, "received"),
          "utf8",
        );
        assert.equal(got.split(prompt).length - 1, 1);
        await page.reload();
        // Pair-once restoration must not require a selector or Connect.
        await page.waitForFunction(() =>
          document
            .querySelector("#preview")
            .textContent.includes("preview-canary"),
        );
        assert(await page.locator("#onboarding").isHidden());
        assert.equal(
          await page.getByLabel("Prompt", { exact: true }).inputValue(),
          "",
        );
        // A ticket from this browser lands on the board, and starts nothing.
        const title = `browser-ticket-canary-${name}-${label}`;
        if (label === "phone") {
          await page.locator("#back").click();
          await page.locator("#new-ticket-fab").click();
        } else await page.locator("#new-ticket").click();
        await page.getByLabel("Title", { exact: true }).fill(title);
        await page.locator("#new-description").fill(`brief-canary for ${title}`);
        await page
          .getByRole("button", { name: "Send ticket", exact: true })
          .click();
        await page.waitForFunction(() =>
          document.querySelector("#toast").textContent.includes("Landed as"),
        );
        const { board } = await command({ cmd: "snapshot" });
        const filed = board.tickets.find((t) => t.title === title);
        assert(filed, "the filed ticket is on the board");
        assert.match(filed.created_by, /^device:/);
        assert(!board.sessions.some((s) => s.ticket === filed.id), "no agent started");
        // The page asks the fixture to act for the host, and waits for it.
        const dir = process.env.MESOPHON_TEST_DIR;
        const ask = async (asked, answered, words = "") => {
          await fs.writeFile(path.join(dir, asked), words);
          for (let i = 0; i < 600; i++) {
            try {
              await fs.rm(path.join(dir, answered));
              return;
            } catch {
              await new Promise((resolve) => setTimeout(resolve, 100));
            }
          }
          throw new Error(`the fixture never wrote ${answered}`);
        };
        // Started from here (T-498, T-510): from the ticket's page, through
        // the sheet that asks for the first prompt, sent blank. A clock until
        // the session takes its first prompt, the ticket's own words, then
        // two ticks, and its output on the page.
        await page.locator(`button[data-mode="board"]`).locator("visible=true").click();
        await page.locator(`.ticket[data-id="${filed.id}"]`).click();
        await page.locator("#detail .start-agent").click();
        await page.locator("#start-send").click();
        await page.waitForFunction(() =>
          document.querySelector("#toast").textContent.includes("Starting an agent on"),
        );
        let started;
        for (let i = 0; i < 100 && !started; i++) {
          const { board } = await command({ cmd: "snapshot" });
          started = board.sessions.find((s) => s.ticket === filed.id);
          if (!started) await new Promise((resolve) => setTimeout(resolve, 100));
        }
        assert.equal(started?.kind, "claude", "the board's provider started on it");
        await ask("agent-run", "agent-ran", started.id);
        await page.waitForFunction(
          () => document.querySelector("#detail .start-receipt")?.dataset.status === "started",
          null,
          { timeout: 30000 },
        );
        await page.waitForFunction(
          () => document.querySelector("#preview").textContent.includes("preview-canary"),
          null,
          { timeout: 30000 },
        );
        const brief = await fs.readFile(path.join(dir, "received"), "utf8");
        assert.equal(brief.split(`brief-canary for ${title}`).length - 1, 1, "its brief, once");
        const after = (await command({ cmd: "snapshot" })).board;
        assert.equal(after.sessions.filter((s) => s.ticket === filed.id).length, 1, "one agent");
        // Back to Now, as the page was, with the ticket's panel closed.
        await page.locator(label === "phone" ? "#back" : "#close-detail").click();
        await page.locator(`button[data-mode="agents"]`).locator("visible=true").click();
        if ((name === "chromium" && label === "desktop") || (name === "webkit" && label === "phone")) {
          // The terminal away (T-497): a ticket waits at the relay, sealed,
          // with one tick, and lands once, when the terminal is back.
          await ask("host-stop", "host-stopped");
          await page.waitForFunction(() => document.querySelector("#shell").dataset.link === "asleep");
          const away = `browser-away-canary-${name}-${label}`;
          await page.locator(label === "phone" ? "#new-ticket-fab" : "#new-ticket").click();
          assert.match(await page.locator(".compose-dest").textContent(), /waits at the relay/);
          await page.getByLabel("Title", { exact: true }).fill(away);
          await page.getByRole("button", { name: "Send ticket", exact: true }).click();
          await page.locator(".waiting-row").filter({ hasText: away }).waitFor();
          await page.waitForFunction(() =>
            document.querySelector("#toast").textContent.includes("waits at the relay"),
          );
          await ask("host-start", "host-started");
          await page.locator(".waiting-row").filter({ hasText: away }).waitFor({ state: "detached", timeout: 30000 });
          const back = (await command({ cmd: "snapshot" })).board;
          const landed = back.tickets.filter((t) => t.title === away);
          assert.equal(landed.length, 1, "filed once");
          assert.match(landed[0].envelope, /^[0-9a-f]{32}$/);
          await page.waitForFunction(() => document.querySelector("#shell").dataset.link === "live", null, {
            timeout: 30000,
          });
        }
        const info = (
          await command({ cmd: "mesophon", action: { action: "status" } })
        ).info;
        const grant = info.devices.find(
          (d) => d.name === `${name}-${label}`,
        ).grant;
        await page.screenshot({
          path: path.join(
            process.env.MESOPHON_TEST_DIR,
            `${name}-${label}.png`,
          ),
          fullPage: true,
        });
        await command({ cmd: "mesophon", action: { action: "revoke", grant } });
        await page.waitForFunction(() =>
          document
            .querySelector("#connection")
            .textContent.includes("Access revoked"),
        );
        assert(await page.locator("#send").isDisabled());
        assert.equal(await page.locator("#preview").textContent(), "");
        assert.equal(await page.locator("#tickets").textContent(), "");
        assert.deepEqual(errors, []);
        console.log(
          `${name} ${label}: pair, encrypted preview, prompt, remembered reconnect, filed ticket, started agent, away ticket, revoke passed`,
        );
      } catch (error) {
        console.error(
          name,
          label,
          await page.locator("#connection").textContent(),
          errors,
        );
        throw error;
      } finally {
        await context.close();
      }
    }
  } finally {
    await browser.close();
  }
}
