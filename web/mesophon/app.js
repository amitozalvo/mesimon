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
  store.rail = localStorage.getItem("mesophon-sidebar") === "rail";
  store.outputView = localStorage.getItem("mesophon-output") === "raw" ? "raw" : "chat";
} catch {
  /* System appearance remains usable when preferences are unavailable. */
}
document.documentElement.dataset.theme = store.theme;
// Opened from the service worker's kept copy (sw.js), not from the relay:
// it shows what this browser remembers and talks to nothing until the
// relay's own page can be had (T-497).
store.kept = document.documentElement.dataset.page === "kept";

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
  if (link.has("pair")) store.pairFromLink(link.get("pair"));
  else store.navigateTicket(link.get("board"), link.get("ticket"));
});
addEventListener("popstate", (event) => store.popstate(event.state));
addEventListener("keydown", (event) => {
  if (event.key !== "Escape") return;
  if (store.boardMenuOpen) {
    store.openBoardMenu(false);
    document.getElementById("board-picker")?.focus();
  } else if (store.sheetOpen) {
    store.openSheet(false);
    document.getElementById("board-menu")?.focus();
  }
});
// The board picker (T-510) closes on a press anywhere else.
addEventListener("pointerdown", (event) => {
  if (store.boardMenuOpen && !event.target.closest?.("#board-picker, #board-list")) store.openBoardMenu(false);
});
for (const event of ["focus", "blur"]) addEventListener(event, () => store.foreground());
addEventListener("online", () => store.setOnline(true));
addEventListener("offline", () => store.setOnline(false));
document.addEventListener("visibilitychange", () => store.visibility());
setInterval(() => store.tick(), 2000);
// A browser that can put the page on the home screen says so once; the
// Settings button asks for it when the person does.
addEventListener("beforeinstallprompt", (event) => {
  event.preventDefault();
  store.installable(event);
});
addEventListener("appinstalled", () => store.installable(undefined, true));
// The kept copy for opening with no signal. A browser without service
// workers, or one that refuses this one, just has no offline page.
navigator.serviceWorker?.register("./sw.js").catch(() => {});

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
