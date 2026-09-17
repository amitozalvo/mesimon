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
        assert.equal(await page.locator("#prompt-mode").inputValue(), "queue");
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
          `${name} ${label}: pair, encrypted preview, prompt, remembered reconnect, revoke passed`,
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
