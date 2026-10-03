// The page's service worker (T-497). It keeps a whole copy of the page's own
// files, so the page opens with no signal: the remembered board, Sent, and a
// ticket sealed in this browser until the relay can take it. Whenever the
// network answers, the page is the relay's own, as fresh as without this
// worker; the kept copy is only for when it does not. A WebSocket never
// passes through a service worker, so the host channel and the mailbox are
// untouched.

const PREFIX = "mesophon-page-";
// Every file the page is made of, the page itself first. `assets.test.js`
// checks this is exactly what index.html reaches.
const PAGE = [
  "./",
  "./style.css",
  "./app.js",
  "./awareness.js",
  "./board.js",
  "./compose.js",
  "./connection.js",
  "./detail.js",
  "./dialogs.js",
  "./edits.js",
  "./html.js",
  "./icons.js",
  "./identity.js",
  "./lists.js",
  "./mailbox.js",
  "./markdown.js",
  "./notes.js",
  "./notepad.js",
  "./pictures.js",
  "./queue.js",
  "./sent.js",
  "./sessions.js",
  "./shell.js",
  "./shin.js",
  "./starts.js",
  "./store.js",
  "./vendor/preact.module.js",
  "./vendor/hooks.module.js",
  "./vendor/htm.module.js",
  "./fonts/plex-sans-latin.woff2",
  "./fonts/plex-sans-latin-ext.woff2",
  "./fonts/plex-sans-hebrew-400.woff2",
  "./fonts/plex-sans-hebrew-500.woff2",
  "./fonts/plex-sans-hebrew-600.woff2",
  "./fonts/plex-mono-400-latin.woff2",
  "./fonts/plex-mono-400-latin-ext.woff2",
  "./fonts/plex-mono-500-latin.woff2",
  "./fonts/plex-mono-500-latin-ext.woff2",
  "./pkg/mesimon_web.js",
  "./pkg/mesimon_web_bg.wasm",
  "./manifest.webmanifest",
  "./icons/icon-192.png",
  "./icons/icon-512.png",
  "./icons/maskable-512.png",
  "./icons/apple-touch-icon.png",
];
// Where the kept copy notes which build of the page it holds.
const VERSION = "./.version";
// How long a page load waits for the network before the kept copy opens.
const WAIT_MS = 6000;
// With no Last-Modified or ETag to compare, how often the copy is renewed.
const STALE_MS = 60 * 60 * 1000;

// How each open page was served, by client: `kept` pages take every file
// from the same copy, so a page is never half one build and half another.
const served = new Map();

self.addEventListener("install", (event) => {
  event.waitUntil(keep().then(() => self.skipWaiting()));
});
self.addEventListener("activate", (event) => {
  event.waitUntil(self.clients.claim());
});

self.addEventListener("fetch", (event) => {
  const request = event.request;
  if (request.method !== "GET" || new URL(request.url).origin !== self.location.origin) return;
  // The kept page asking whether the relay is back: the network, always.
  if (request.cache === "no-store") return;
  event.respondWith(request.mode === "navigate" ? open(event) : file(event));
});

// The newest whole copy's name. Names carry the time they were made.
async function current() {
  const names = (await caches.keys()).filter((name) => name.startsWith(PREFIX)).sort();
  return names.at(-1);
}

const versionOf = (response) => response.headers.get("last-modified") || response.headers.get("etag") || "";

// A new copy of every file, fetched past the HTTP cache. It is put under a
// new name only once every file answered, and the older copies go after it.
async function keep() {
  const responses = await Promise.all(
    PAGE.map(async (url) => {
      const response = await fetch(url, { cache: "no-cache" });
      if (!response.ok) throw new Error(`${url} answered ${response.status}`);
      return response;
    }),
  );
  const name = `${PREFIX}${Date.now()}`;
  const cache = await caches.open(name);
  try {
    await Promise.all(PAGE.map((url, i) => cache.put(url, responses[i])));
    await cache.put(VERSION, new Response(versionOf(responses[0])));
  } catch (error) {
    await caches.delete(name);
    throw error;
  }
  for (const old of await caches.keys())
    if (old.startsWith(PREFIX) && old !== name) await caches.delete(old);
}

// After a page load the network answered: renew the copy when the relay
// serves another build, or, when it cannot say, once an hour.
async function renew(response) {
  const name = await current();
  if (name) {
    const kept = await (await caches.open(name)).match(VERSION);
    const version = versionOf(response);
    const fresh = version
      ? (await kept?.text()) === version
      : Date.now() - Number(name.slice(PREFIX.length)) < STALE_MS;
    if (fresh) return;
  }
  await keep();
}

// A page load: the relay's page when the network answers in time, else the
// kept copy, marked so the page knows it was not the relay's this time.
async function open(event) {
  const network = fetch(event.request);
  // A load the kept copy answered may still fail later; nobody waits on it.
  network.catch(() => {});
  let response;
  try {
    response = await Promise.race([network, new Promise((resolve) => setTimeout(resolve, WAIT_MS))]);
  } catch {
    response = undefined;
  }
  for (const id of served.keys()) if (!(await self.clients.get(id))) served.delete(id);
  if (response) {
    served.set(event.resultingClientId, "network");
    event.waitUntil(renew(response).catch(() => {}));
    return response;
  }
  const name = await current();
  const kept = name && (await (await caches.open(name)).match("./"));
  if (!kept) return network;
  served.set(event.resultingClientId, "kept");
  const headers = new Headers(kept.headers);
  headers.delete("content-length");
  const page = (await kept.text()).replace("<html", '<html data-page="kept"');
  return new Response(page, { status: 200, statusText: "OK", headers });
}

// Any other file: from the page's own copy when the page came from it, else
// from the network, falling back to the copy.
async function file(event) {
  const name = await current();
  const cache = name && (await caches.open(name));
  if (served.get(event.clientId) === "kept" && cache) {
    const kept = await cache.match(event.request, { ignoreSearch: true });
    if (kept) return kept;
  }
  try {
    return await fetch(event.request);
  } catch (error) {
    const kept = cache && (await cache.match(event.request, { ignoreSearch: true }));
    if (kept) return kept;
    throw error;
  }
}
