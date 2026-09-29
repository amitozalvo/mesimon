import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { execFileSync } from "node:child_process";

const source = path.dirname(fileURLToPath(import.meta.url));
const stage = path.resolve(source, "../../ci/stage-mesophon.sh");
test("the deployable browser package contains every referenced file and no test harness", async () => {
  const scratch = await fs.mkdtemp(path.join(os.tmpdir(), "mesophon-assets-"));
  try {
    const fixture = path.join(scratch, "source");
    const output = path.join(scratch, "runtime");
    await fs.mkdir(fixture);
    for (const name of await fs.readdir(source)) {
      if (/\.(html|css|js)$/.test(name))
        await fs.copyFile(path.join(source, name), path.join(fixture, name));
    }
    for (const dir of ["vendor", "fonts"])
      await fs.cp(path.join(source, dir), path.join(fixture, dir), { recursive: true });
    // Packaging does not depend on crypto execution. The browser and relay
    // suites separately exercise the real Wasm build and encrypted transport.
    await fs.mkdir(path.join(fixture, "pkg"));
    await fs.writeFile(
      path.join(fixture, "pkg/mesimon_web.js"),
      "// generated facade\n",
    );
    await fs.writeFile(
      path.join(fixture, "pkg/mesimon_web_bg.wasm"),
      "wasm fixture",
    );
    await fs.mkdir(path.join(fixture, "node_modules"));
    await fs.writeFile(path.join(fixture, "package.json"), "{}");
    execFileSync("sh", [stage, output, fixture]);
    const visited = new Set();
    // Follow HTML src/href, JavaScript imports and CSS url() references.
    async function follow(name) {
      if (visited.has(name)) return;
      visited.add(name);
      const content = await fs.readFile(path.join(output, name), "utf8");
      const references = name.endsWith(".html")
        ? [...content.matchAll(/(?:src|href)="\.\/([^"]+)"/g)]
        : name.endsWith(".css")
          ? [...content.matchAll(/url\((?!data:)([^)"']+)\)/g)]
          : name.endsWith(".js")
            ? [...content.matchAll(/(?:from\s*|import\s*)["']\.\/([^"']+)["']/g)]
            : [];
      for (const [, reference] of references)
        await follow(path.join(path.dirname(name), reference));
    }
    await follow("index.html");
    for (const name of [
      "connection.js",
      "store.js",
      "shell.js",
      "compose.js",
      "sent.js",
      "mailbox.js",
      "vendor/preact.module.js",
      "vendor/hooks.module.js",
      "vendor/htm.module.js",
      "fonts/plex-sans-latin.woff2",
      "fonts/plex-sans-hebrew-400.woff2",
      "fonts/plex-mono-500-latin.woff2",
    ])
      assert(visited.has(name), `${name} is not reachable from index.html`);
    for (const licence of ["vendor/LICENSE-preact", "vendor/LICENSE-htm", "fonts/OFL.txt"])
      await fs.access(path.join(output, licence));
    assert.equal(
      await fs.readFile(path.join(output, "pkg/mesimon_web_bg.wasm"), "utf8"),
      "wasm fixture",
    );
    assert(
      !(await fs.readdir(output)).some(
        (name) =>
          name.endsWith(".test.js") ||
          ["node_modules", "package.json"].includes(name),
      ),
    );
    assert.throws(
      () => execFileSync("sh", [stage, output, fixture], { stdio: "pipe" }),
      /Command failed/,
    );
  } finally {
    await fs.rm(scratch, { recursive: true, force: true });
  }
});
