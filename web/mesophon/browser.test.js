// Runs inside the Rust test's supervised fixture, against its real TLS relay.
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
        ignoreHTTPSErrors: true,
      }); // fixture-only self-signed TLS
      const page = await context.newPage();
      const errors = [];
      page.on("pageerror", (e) => errors.push(String(e)));
      page.on("console", (m) => {
        if (m.type() === "error") console.error(name, m.text());
      });
      try {
        await page.goto(origin);
        await page.waitForFunction(() =>
          document
            .querySelector("#connection")
            .textContent.includes("Enable Mesophon"),
        );
        const paired = await command({
          cmd: "mesophon",
          action: { action: "pair" },
        });
        assert(paired.info?.code);
        await page
          .getByLabel("Device name", { exact: true })
          .fill(`${name}-${label}`);
        await page
          .getByLabel("Pairing code", { exact: true })
          .fill(paired.info.code);
        await page
          .getByRole("button", { name: "Pair browser", exact: true })
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
        const prompt = `browser-canary-${name}-${label}`;
        await page.getByLabel("Prompt", { exact: true }).fill(prompt);
        await page
          .getByRole("button", { name: "Send prompt", exact: true })
          .click();
        await page.waitForFunction(() =>
          document.querySelector("#delivery").textContent.includes("Submitted"),
        );
        const got = await fs.readFile(
          path.join(process.env.MESOPHON_TEST_DIR, "received"),
          "utf8",
        );
        assert.equal(got.split(prompt).length - 1, 1);
        await page.reload();
        await page.waitForFunction(
          () => document.querySelector("#boards").options.length === 2,
        );
        await page.locator("#boards").selectOption({ index: 1 });
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
          document.querySelector("#preview").textContent.includes("preview-canary"),
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
