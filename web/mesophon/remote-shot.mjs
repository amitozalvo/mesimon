// The README's and mesimon.dev's Remote Control pictures (T-637), taken from
// this page on ux.test.js's fixture: the real client, a seeded board, no
// relay. `node remote-shot.mjs ../../assets/demo` writes remote.png (the Now
// screen with a permission on its card) and remote-conversation.png (a
// ticket page with the agent's conversation), both at a phone's 390 px and
// twice the density. Playwright's Chromium must be installed (`npm ci`).
import { chromium } from "playwright";
import http from "node:http";
import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
const root = path.dirname(fileURLToPath(import.meta.url));
const out = process.argv[2] || path.join(root, "test-results");
const src = await fs.readFile(path.join(root, "ux.test.js"), "utf8");
const fixtureSrc = src.slice(src.indexOf("function fixture() {"), src.indexOf("\n// Poll until the page says yes."));
const cryptoAt = src.indexOf("const fakeCrypto = `");
const fakeCrypto = src.slice(cryptoAt + "const fakeCrypto = `".length, src.indexOf("`;", cryptoAt));
const types = { html: "text/html", css: "text/css", js: "text/javascript", woff2: "font/woff2", webmanifest: "application/manifest+json", png: "image/png", wasm: "application/wasm" };
const server = http.createServer(async (req, res) => {
  try {
    const name = new URL(req.url, "http://fixture").pathname.slice(1) || "index.html";
    const match = /^(?:vendor\/|fonts\/|icons\/|pkg\/)?[a-z0-9._-]+\.(html|css|js|woff2|webmanifest|png|wasm)$/.exec(name);
    if (!match) { res.writeHead(404).end(); return; }
    res.setHeader("Content-Type", types[match[1]]);
    if (name === "pkg/mesimon_web.js") res.end(fakeCrypto);
    else if (name === "pkg/mesimon_web_bg.wasm") res.end("wasm fixture");
    else res.end(await fs.readFile(path.join(root, name)));
  } catch { res.writeHead(404).end(); }
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const origin = `http://127.0.0.1:${server.address().port}`;
const browser = await chromium.launch();
const context = await browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, colorScheme: "dark", reducedMotion: "reduce", serviceWorkers: "block" });
await context.addInitScript({ content: fixtureSrc + "\nfixture();\nwindow.fixture.features.push('transcript');" });
const page = await context.newPage();
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
await page.goto(origin);
await page.waitForFunction(() => !document.querySelector("#pair").disabled);
await page.getByLabel("Pairing code", { exact: true }).fill("fixture-pair-code");
await page.getByRole("button", { name: "Connect", exact: true }).click();
await page.locator('.ticket[data-id="ticket-0"]').waitFor();
await page.evaluate(() => {
  const now = Date.now();
  fixture.tickets = [
    { id: "t12", key: "T-12", title: "Add a --version flag", column: "TODO", tags: [{ group: 1, name: "FEATURE" }],
      agent: { session: "s12", provider: "claude", state: "needs attention", promptable: true, since: now - 40000,
        permission: { request: "p1", tool: "Bash", input: { command: "cargo test -p mesimon-core" }, expires_at: now + 600000 } } },
    { id: "t9", key: "T-9", title: "Export the stats table as CSV", column: "IN PROGRESS", tags: [{ group: 1, name: "BUG" }],
      agent: { session: "s9", provider: "claude", state: "working", promptable: true, since: now - 5 * 60000, doing: "Edit(crates/core/src/stats.rs)" } },
    { id: "t11", key: "T-11", title: "Rename the settings menu", column: "IN PROGRESS",
      agent: { session: "s11", provider: "codex", state: "working", promptable: true, since: now - 2 * 60000 } },
    { id: "t7", key: "T-7", title: "Keyboard layouts: Hebrew brackets", column: "IN PROGRESS",
      agent: { session: "s7", provider: "claude", state: "idle", promptable: true, since: now - 25 * 60000, said: "Done, and the goldens are reminted." } },
    { id: "t14", key: "T-14", title: "Write the release notes", column: "TODO", agent: null },
  ];
  fixture.transcript = [
    { at: 0, kind: "prompt", text: "Export the stats table as CSV. Scripts scrape the table today; print the same numbers when --csv is passed, one row per link." },
    { at: 10, kind: "tool", text: "Read crates/core/src/stats.rs" },
    { at: 20, kind: "tool", text: "Grep --csv" },
    { at: 30, kind: "reply", text: "The table is built in `stats::render`, so the flag goes on the same path. I will add a `--csv` writer beside it and a test on a three-link fixture." },
    { at: 40, kind: "tool", text: "Edit crates/core/src/stats.rs" },
    { at: 50, kind: "tool", text: "Edit crates/core/src/cli.rs" },
    { at: 60, kind: "tool", text: "Bash(cargo test -p mesimon-core stats)" },
  ];
  fixture.update();
});
await page.waitForFunction(() => document.body.textContent.includes("Approve once"));
await page.waitForTimeout(400);
await page.screenshot({ path: path.join(out, "remote.png") });
await page.locator('.ticket[data-id="t9"]').first().click();
await page.waitForFunction(() => document.body.textContent.includes("three-link fixture"), null, { timeout: 10000 }).catch(() => {});
await page.waitForTimeout(600);
await page.screenshot({ path: path.join(out, "remote-conversation.png") });
console.log("wrote", out, "errors", JSON.stringify(errors));
await browser.close();
server.close();
