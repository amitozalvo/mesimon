// Boot: load the Wasm crypto and the device identity, then hand the page to
// the store and the view. Window-level events are wired here and nowhere else.
import init, { Browser } from "./pkg/mesimon_web.js";
import { openIdentity } from "./identity.js";
import { Store } from "./store.js";
import { html, render } from "./html.js";
import { App } from "./shell.js";

const store = new Store(Browser);
try {
  store.theme = localStorage.getItem("mesophon-theme") || "system";
} catch {
  /* System appearance remains usable when preferences are unavailable. */
}
document.documentElement.dataset.theme = store.theme;

function viewport() {
  document.documentElement.style.setProperty(
    "--viewport-height",
    `${window.visualViewport?.height || innerHeight}px`,
  );
}
window.visualViewport?.addEventListener("resize", viewport);
addEventListener("resize", viewport);
viewport();

render(html`<${App} store=${store} />`, document.getElementById("app"));

addEventListener("hashchange", () => {
  const link = new URLSearchParams(location.hash.slice(1));
  store.navigateTicket(link.get("board"), link.get("ticket"));
});
addEventListener("popstate", (event) => store.popstate(event.state));
addEventListener("keydown", (event) => {
  if (event.key === "Escape" && store.sheetOpen) {
    store.openSheet(false);
    document.getElementById("board-menu")?.focus();
  }
});
for (const event of ["focus", "blur"]) addEventListener(event, () => store.foreground());
addEventListener("online", () => store.setOnline(true));
addEventListener("offline", () => store.setOnline(false));
document.addEventListener("visibilitychange", () => store.visibility());
setInterval(() => store.tick(), 2000);

try {
  await init();
  const storage = await openIdentity();
  let identity = await storage.read();
  if (!identity) {
    const crypto = new Browser();
    identity = { seed: crypto.seed(), boards: [] };
    crypto.free();
    await storage.save(identity);
  }
  store.boot(storage, identity);
} catch {
  store.status =
    "Could not load the browser module or device storage. Check the deployment and browser storage permissions.";
  store.emit();
}
