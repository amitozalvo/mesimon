// The coordinator owns no fixture. Each child owns one browser, one server
// (including its offline switch), and the screenshots for its engine and size.
import { spawn, spawnSync } from "node:child_process";
import { mkdir, mkdtemp } from "node:fs/promises";
import path from "node:path";

function selection(variable, choices) {
  const value = process.env[variable];
  if (value === undefined) return choices;
  const names = value.split(",").map((name) => name.trim());
  for (const name of names) {
    if (!choices.includes(name)) {
      throw new Error(`Unknown ${variable} value "${name}". Choose from ${choices.join(", ")}`);
    }
  }
  return choices.filter((choice) => names.includes(choice));
}

function themeFilesChanged(root) {
  // Include commits on the ticket and staged/unstaged edits. On main itself
  // check the last commit; a checkout without the base gets the full sweep.
  const git = (...args) => spawnSync("git", args, { cwd: root, encoding: "utf8" });
  const base = git("merge-base", "main", "HEAD");
  if (base.status !== 0) return true;
  const ref = git("branch", "--show-current").stdout?.trim() === "main" ? "HEAD^" : base.stdout.trim();
  return git("diff", "--quiet", ref, "--", "style.css", "themes.js").status !== 0;
}

function finished(child) {
  return new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("close", (code, signal) => {
      if (code === 0) resolve();
      else reject(new Error(`${child.spawnargs.join(" ")} ended with ${signal || code}`));
    });
  });
}

export async function runMatrix(root, engineNames, bySize) {
  const suite = path.join(root, "ux.test.js");
  if (!process.env.MESIMON_TEST_RUN) {
    // Keep the longstanding direct node invocation bounded too. Forward a
    // targeted TERM/INT to the supervisor so it can clean up its descendants.
    const deadline = Number(process.env.MESOPHON_UX_JOBS) === 1 ? ["--timeout", "3600"] : [];
    const child = spawn("python3", [
      "-B", path.resolve(root, "../../ci/test-run.py"), ...deadline, "--", process.execPath, suite,
    ], { stdio: "inherit" });
    const stop = (signal) => child.kill(signal);
    process.on("SIGTERM", stop);
    process.on("SIGINT", stop);
    try {
      await finished(child);
    } finally {
      process.off("SIGTERM", stop);
      process.off("SIGINT", stop);
    }
    return;
  }

  const sizes = Object.keys(bySize);
  const allFlows = [...new Set(Object.values(bySize).flatMap(Object.keys))];
  const engines = selection("MESOPHON_UX_ENGINE", engineNames);
  const selectedSizes = selection("MESOPHON_UX_SIZES", sizes);
  const flows = selection("MESOPHON_UX_FLOW", allFlows);
  const jobs = Number(process.env.MESOPHON_UX_JOBS ?? 6);
  if (!Number.isSafeInteger(jobs) || jobs < 1) throw new Error("MESOPHON_UX_JOBS must be a positive integer");

  const cells = [];
  for (const engine of engines) {
    for (const size of selectedSizes) {
      const names = Object.keys(bySize[size]).filter((name) => flows.includes(name));
      if (names.length) cells.push({ engine, size, names });
    }
  }
  const total = engineNames.length * sizes.reduce((sum, size) => sum + Object.keys(bySize[size]).length, 0);
  const count = cells.reduce((sum, cell) => sum + cell.names.length, 0);
  if (!count) throw new Error("No UX cases match the selected flows and viewports");
  const skipped = allFlows.filter((name) => !cells.some((cell) => cell.names.includes(name)));
  console.log(`Selected ${count}/${total} cases (${total - count} skipped); ${Math.min(jobs, cells.length)} concurrent cells.`);
  if (skipped.length) console.log(`Skipped flows: ${skipped.join(", ")}`);
  const navigation = cells.filter((cell) => cell.names.includes("navigation"));
  const sweep = navigation.length && (process.env.MESOPHON_UX_THEMES === "all" || themeFilesChanged(root))
    ? navigation : navigation.slice(0, 1);
  if (sweep.length) console.log(`15-theme sweep: ${sweep.map(({ engine, size }) => engine + "/" + size).join(", ")}`);

  const resultsRoot = path.join(root, "test-results");
  await mkdir(resultsRoot, { recursive: true });
  const screenshots = await mkdtemp(path.join(resultsRoot, "run-"));
  console.log(`Screenshots: ${screenshots}`);
  const queue = cells.values();
  async function work() {
    for (const cell of queue) {
      const { engine, size, names } = cell;
      await finished(spawn(process.execPath, [suite, "--cell", screenshots, engine, size,
        sweep.includes(cell) ? "all" : "chalk", ...names], { stdio: "inherit" }));
    }
  }
  const start = performance.now();
  const results = await Promise.allSettled(Array.from({ length: Math.min(jobs, cells.length) }, work));
  const failures = results.filter((result) => result.status === "rejected");
  if (failures.length) throw new AggregateError(failures.map((result) => result.reason), "UX matrix failed");
  console.log(`Passed ${count}/${total} UX cases (${total - count} skipped) in ${((performance.now() - start) / 1000).toFixed(1)} s.`);
}
