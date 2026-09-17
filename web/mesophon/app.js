import init, { Browser } from "./pkg/mesimon_web.js";
import { openIdentity } from "./identity.js";
import { Connection } from "./connection.js";
import { BoardState } from "./board.js";
import { Sessions } from "./sessions.js";
import { $, View } from "./view.js";
import { renderDialogs, clearDialogs } from "./dialogs.js";
import { showAlert, clearAlerts } from "./awareness.js";

let identity,
  storage,
  connection,
  active,
  board,
  entry,
  returnBoard,
  live = false;
const boards = new Map();
const sessions = new Sessions();
const view = new View(select);
const save = () => storage.save(identity);
function persist() {
  save().catch(() => {
    $("connection").textContent =
      "Could not save browser preferences. They may be lost on reload.";
  });
}
function render() {
  if (!board) return;
  entry = sessions.get(active.pin.board, board.current);
  view.list(board);
  view.detail(board.current, entry, live);
  renderDialogs(board.current, entry, live, sendInteraction);
}
function sendInteraction(body, target) {
  if (!live || !connection.online || !connection.features?.includes(body.op) || entry !== target || target.receipt?.waiting) return;
  const id = connection.request(body, target.key);
  if (id === undefined) return;
  sessions.sent(target, id, connection.incarnation, body.op, "");
  render();
}
function detail(open, push = false) {
  if (open && board && $("tickets").getClientRects().length)
    board.scroll[board.mode] = $("tickets").scrollTop;
  if (open && document.body.dataset.detail !== "true")
    view.entryKey = undefined;
  document.body.dataset.detail = String(open);
  if (!open && board) $("tickets").scrollTop = board.scroll[board.mode];
  if (!open)
    $("tickets")
      .querySelector('[aria-pressed="true"]')
      ?.focus({ preventScroll: true });
  if (
    push &&
    matchMedia("(max-width: 700px)").matches &&
    !history.state?.detail
  )
    history.pushState({ detail: true }, "");
}
function select(id) {
  view.capture(entry);
  board.selected = id;
  active.selected = id;
  persist();
  detail(true, true);
  render();
  $("selection").focus({ preventScroll: true });
  preview();
  foreground();
}
function preview() {
  if (!document.hidden && board?.current?.agent && !connection.has("preview"))
    connection.request(
      {
        op: "preview",
        ticket: board.current.id,
        session: board.current.agent.session,
      },
      entry.key,
    );
}
function navigateTicket(boardId, ticket) {
  const chosen = identity?.boards.find((b) => b.pin.board === boardId && !b.revoked);
  if (!chosen) return;
  chosen.selected = ticket;
  if (active?.pin.board === boardId && board?.tickets.some((t) => t.id === ticket)) select(ticket);
  else {
    boards.delete(boardId);
    openBoard(chosen);
    detail(true);
  }
}
addEventListener("hashchange", () => {
  const link = new URLSearchParams(location.hash.slice(1));
  navigateTicket(link.get("board"), link.get("ticket"));
});
function visibleTicket() {
  return !document.hidden && document.hasFocus() && board?.current &&
    (!matchMedia("(max-width: 700px)").matches || document.body.dataset.detail === "true")
    ? board.current.id : null;
}
function foreground() {
  if (connection?.online && connection.features?.includes("awareness") && !connection.has("foreground"))
    connection.request({ op: "foreground", ticket: visibleTicket() });
}
function refresh() {
  if (!connection.has("snapshot")) connection.request({ op: "snapshot" });
}
function receipts() {
  for (const session of sessions.entries.values()) {
    const receipt = session.receipt;
    if (session.board !== active?.pin.board || !receipt?.unresolved) continue;
    if (receipt.incarnation !== connection.incarnation) {
      sessions.reply(session, { result: "delivery", status: "unknown" });
    } else if (
      ![...connection.pending.values()].some(
        (p) =>
          p.context === session.key &&
          ["prompt", "send_now", "take_back", "permission", "dialog", "status"].includes(p.body.op),
      )
    ) {
      connection.request({ op: "status", command: receipt.id }, session.key);
    }
  }
}
function showPairing() {
  if (active && !active.revoked) returnBoard = active;
  $("onboarding").hidden = false;
  $("shell").hidden = true;
  $("cancel-pair").hidden = !returnBoard || !!returnBoard.revoked;
  $("code").focus();
}
function openBoard(chosen) {
  view.capture(entry);
  live = false;
  active = chosen;
  board = boards.get(chosen.pin.board);
  entry = undefined;
  view.clear();
  clearDialogs();
  clearAlerts();
  view.boards(identity, active);
  $("sidebar").classList.remove("open");
  $("board-menu").setAttribute("aria-expanded", "false");
  identity.lastBoard = chosen.pin.board;
  persist();
  if (chosen.revoked) {
    connection.stop();
    showPairing();
    $("connection").textContent =
      "Access revoked. Pair again from the host to restore access.";
    return;
  }
  $("onboarding").hidden = true;
  $("shell").hidden = false;
  $("board-title").textContent = chosen.title || "Paired board";
  detail(!!chosen.selected);
  render();
  connection.connect(chosen);
}
async function onState(state, message) {
  $("connection").textContent = state === "revoked" ? "Removing access…" : message;
  let revocationSaved;
  if (state === "revoked" || state === "unverified") {
    if (active) {
      sessions.purge(active.pin.board);
      boards.delete(active.pin.board);
      if (state === "revoked") {
        active.revoked = true;
        revocationSaved = save();
      }
    }
    board = entry = undefined;
    view.clear();
    clearDialogs();
    clearAlerts();
    showPairing();
  }
  if (["unpaired", "revoked", "unverified"].includes(state))
    $("pair").disabled = false;
  render();
  if (revocationSaved) {
    try {
      await revocationSaved;
      $("connection").textContent = message;
    } catch {
      $("connection").textContent = `${message} Could not save the access-removed marker.`;
    }
  }
}

function onReply(reply, original, id) {
  if (original?.body.op === "foreground") return;
  if (reply.result === "awareness") {
    const originBoard = active?.pin.board;
    showAlert(reply, visibleTicket(), (ticket) => navigateTicket(originBoard, ticket));
    refresh();
    return;
  }
  if (reply.result === "changed") {
    refresh();
    return;
  }
  if (reply.result === "board") {
    view.capture(entry);
    if (!board) {
      board = new BoardState(active.selected);
      boards.set(active.pin.board, board);
    }
    board.update(reply);
    if (active.title !== reply.title || active.selected !== board.selected) {
      active.title = reply.title;
      active.selected = board.selected;
      persist();
      view.boards(identity, active);
    }
    live = true;
    if ($("connection").textContent !== "Connected")
      $("connection").textContent = "Connected";
    render();
    preview();
    foreground();
  } else if (reply.result === "preview" && original?.body.op === "preview") {
    const session = sessions.entries.get(original.context);
    if (session) {
      session.output = reply.lines.join("\n");
      session.receivedAt = Date.now();
      if (session.following) session.displayed = session.output;
      else session.unread = session.output !== session.displayed;
    }
    render();
  } else if (["delivery", "rejected", "taken_back"].includes(reply.result)) {
    const session = sessions.entries.get(original?.context);
    const command = ["prompt", "send_now", "take_back", "permission", "dialog"].includes(
      original?.body.op,
    )
      ? id
      : original?.body.op === "status"
        ? original.body.command
        : undefined;
    if (session?.receipt?.id === command && command !== undefined)
      sessions.reply(session, reply);
    else if (
      reply.result === "rejected" &&
      original?.body.op === "preview" &&
      session
    ) {
      session.displayed = `Preview unavailable: ${reply.message}`;
      // A rejected preview can indicate session replacement; refresh identity.
      refresh();
    }
    render();
    if (
      ["delivery", "taken_back"].includes(reply.result) ||
      ["send_now", "take_back"].includes(original?.body.op)
    )
      refresh();
  }
}
$("pair-form").onsubmit = (event) => {
  event.preventDefault();
  const code = $("code").value.trim();
  if (!code || !connection) return;
  view.capture(entry);
  identity.name = $("device-name").value.trim() || "My browser";
  active = board = entry = undefined;
  live = false;
  view.clear();
  clearDialogs();
  clearAlerts();
  $("pair").disabled = true;
  connection.connect(undefined, code);
};
$("boards").onchange = () => {
  const chosen = identity.boards.find((b) => b.pin.board === $("boards").value);
  if (chosen) openBoard(chosen);
};
$("add-board").onclick = showPairing;
$("cancel-pair").onclick = () => {
  if (returnBoard) openBoard(returnBoard);
};
$("board-menu").onclick = () => {
  const open = $("sidebar").classList.toggle("open");
  $("board-menu").setAttribute("aria-expanded", String(open));
};
$("back").onclick = () => {
  view.capture(entry);
  if (history.state?.detail) history.back();
  else detail(false);
  $("tickets")
    .querySelector('[aria-pressed="true"]')
    ?.focus({ preventScroll: true });
};
addEventListener("keydown", (event) => {
  if (event.key === "Escape" && $("sidebar").classList.contains("open")) {
    $("sidebar").classList.remove("open");
    $("board-menu").setAttribute("aria-expanded", "false");
    $("board-menu").focus();
  }
});
addEventListener("popstate", (event) => {
  view.capture(entry);
  detail(!!event.state?.detail);
  render();
});
$("search").oninput = () => {
  if (board) {
    board.search = $("search").value;
    board.scroll[board.mode] = 0;
    render();
  }
};
$("filter").onchange = () => {
  if (board) {
    board.filter = $("filter").value;
    board.scroll.agents = 0;
    render();
  }
};
$("column").onchange = () => {
  if (board) {
    board.column = $("column").value;
    board.scroll.board = 0;
    render();
  }
};
for (const mode of ["agents", "board"])
  $(mode + "-mode").onclick = () => {
    if (board) {
      board.scroll[board.mode] = $("tickets").scrollTop;
      board.mode = mode;
      render();
    }
  };
$("tickets").onscroll = () => {
  if (board && $("tickets").getClientRects().length)
    board.scroll[board.mode] = $("tickets").scrollTop;
};
$("preview").onscroll = () => {
  if (!entry || !$("preview").getClientRects().length) return;
  entry.scroll = $("preview").scrollTop;
  entry.following =
    $("preview").scrollHeight - $("preview").clientHeight - entry.scroll < 24;
  // A paused 50-line window stays frozen until the reader explicitly follows.
  if (entry.following && entry.unread) {
    entry.displayed = entry.output;
    entry.unread = false;
  }
  $("latest").hidden = entry.following;
};
$("latest").onclick = () => {
  if (entry) {
    entry.following = true;
    entry.unread = false;
    entry.displayed = entry.output;
    render();
  }
};
$("wrap").onchange = () =>
  $("preview").classList.toggle("no-wrap", !$("wrap").checked);
$("prompt").oninput = () => {
  if (entry) {
    entry.draft = $("prompt").value;
    view.detail(board.current, entry, live);
  }
};
$("review-draft").onclick = () => {
  if (entry) {
    entry.review = false;
    render();
    $("prompt").focus();
  }
};
$("prompt").onkeydown = (event) => {
  if (
    event.key === "Enter" &&
    (event.ctrlKey || event.metaKey) &&
    !event.isComposing
  ) {
    event.preventDefault();
    $("prompt-form").requestSubmit();
  }
};
$("prompt-mode").onchange = () => {
  if (entry) {
    entry.mode = $("prompt-mode").value;
    render();
  }
};
function queueAction(op) {
  if (
    !live ||
    !board?.current?.agent?.promptable ||
    board.current.queued == null ||
    (entry.receipt?.waiting && entry.receipt.status !== "queued")
  )
    return;
  const id = connection.request(
    { op, ticket: entry.ticket, session: entry.session },
    entry.key,
  );
  if (id !== undefined)
    sessions.sent(entry, id, connection.incarnation, op, board.current.queued);
  else
    entry.delivery = "Delivery unknown. Check the agent before trying again.";
  render();
}
$("send-now").onclick = () => queueAction("send_now");
$("take-back").onclick = () => queueAction("take_back");
$("swap-returned").onclick = () => {
  if (!entry?.returned) return;
  const returned = entry.returned;
  entry.returned = entry.draft
    ? { text: entry.draft, session: entry.session }
    : undefined;
  entry.draft = returned.text;
  entry.review ||= returned.session !== entry.session;
  render();
  $("prompt").focus();
};
$("prompt-form").onsubmit = (event) => {
  event.preventDefault();
  if (
    !live ||
    !board?.current?.agent?.promptable ||
    !entry?.draft.trim() ||
    entry.review ||
    entry.receipt?.waiting
  )
    return;
  if (new TextEncoder().encode(entry.draft).length > 4096) {
    entry.delivery = "Prompt must fit in 4096 UTF-8 bytes.";
    render();
    return;
  }
  const id = connection.request(
    {
      op: "prompt",
      ticket: entry.ticket,
      session: entry.session,
      text: entry.draft,
      queued: entry.mode === "queue",
    },
    entry.key,
  );
  if (id !== undefined) sessions.sent(entry, id, connection.incarnation);
  else
    entry.delivery = "Delivery unknown. Check the agent before sending again.";
  render();
};
$("forget").onclick = async () => {
  connection.stop();
  sessions.entries.clear();
  sessions.targets.clear();
  boards.clear();
  active = board = entry = returnBoard = undefined;
  view.clear();
  clearDialogs();
  clearAlerts();
  const crypto = new Browser();
  identity = { seed: crypto.seed(), boards: [] };
  crypto.free();
  connection.identity = identity;
  try {
    await save();
    $("connection").textContent = "Browser forgotten. Pair again to connect.";
  } catch {
    $("connection").textContent =
      "Could not forget the saved identity. Clear this site's browser storage.";
  }
  view.boards(identity);
  showPairing();
  $("pair").disabled = false;
};
try {
  const theme = localStorage.getItem("mesophon-theme") || "system";
  document.documentElement.dataset.theme = theme;
  $("theme").value = theme;
} catch {
  /* System appearance remains usable when preferences are unavailable. */
}
$("theme").onchange = () => {
  document.documentElement.dataset.theme = $("theme").value;
  try {
    localStorage.setItem("mesophon-theme", $("theme").value);
  } catch {
    /* In-memory choice still applies. */
  }
};
function viewport() {
  document.documentElement.style.setProperty(
    "--viewport-height",
    `${window.visualViewport?.height || innerHeight}px`,
  );
}
window.visualViewport?.addEventListener("resize", viewport);
addEventListener("resize", viewport);
viewport();
for (const event of ["focus", "blur"]) addEventListener(event, foreground);
$("alerts").onclick = async () => {
  if (typeof Notification === "undefined") {
    $("alert-status").textContent = "Updates appear here while connected. System notifications are unavailable in this browser.";
    return;
  }
  const permission = await Notification.requestPermission();
  $("alert-status").textContent = permission === "granted"
    ? "Alerts enabled while this browser stays connected."
    : "Updates appear here while connected. System notifications are disabled in browser settings.";
};
document.addEventListener("visibilitychange", () => {
  foreground();
  if (!document.hidden && connection) {
    live = false;
    render();
    $("connection").textContent =
      "Checking connection… Last received view is stale.";
    connection.tick();
    refresh();
    preview();
  }
});
setInterval(() => {
  if (!connection) return;
  connection.tick();
  foreground();
  if (!document.hidden && connection.online) {
    refresh();
    preview();
    receipts();
  }
}, 2000);
try {
  await init();
  storage = await openIdentity();
  identity = await storage.read();
  if (!identity) {
    const crypto = new Browser();
    identity = { seed: crypto.seed(), boards: [] };
    crypto.free();
    await save();
  }
  connection = new Connection({
    Browser,
    identity,
    save,
    onState,
    onReply,
    onLost: () => {
      live = false;
      sessions.lost();
      render();
    },
    onReady: (chosen) => {
      active = chosen;
      returnBoard = undefined;
      board = boards.get(chosen.pin.board);
      $("code").value = "";
      $("pair").disabled = false;
      $("onboarding").hidden = true;
      $("shell").hidden = false;
      view.boards(identity, active);
      render();
      refresh();
      receipts();
    },
  });
  $("pair").disabled = false;
  const link = new URLSearchParams(location.hash.slice(1));
  const linked = identity.boards.find((b) => b.pin.board === link.get("board") && !b.revoked);
  if (linked && link.get("ticket")) linked.selected = link.get("ticket");
  const remembered = linked ||
    identity.boards.find((b) => b.pin.board === identity.lastBoard) ||
    identity.boards[0];
  if (remembered) { openBoard(remembered); if (linked) detail(true); }
  else
    $("connection").textContent =
      "Enable Remote Control on the host, then pair with its code.";
} catch {
  $("connection").textContent =
    "Could not load the browser module or device storage. Check the deployment and browser storage permissions.";
}
