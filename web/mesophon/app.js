import init, { Browser } from "./pkg/mesimon_web.js";

const $ = (id) => document.getElementById(id);
const node = (tag, text) => {
  const n = document.createElement(tag);
  n.textContent = text;
  return n;
};
let db,
  identity,
  crypto,
  socket,
  generation = 0,
  active,
  incarnation,
  next = 1,
  online = false;
let tickets = [],
  selected,
  lastPrompt,
  pending = new Map(),
  reconnect,
  processing = Promise.resolve();
const setConnection = (text) => {
  $("connection").textContent = text;
};
function endConnection(message) {
  ++generation;
  active = undefined;
  online = false;
  clearTimeout(reconnect);
  socket?.close();
  pending.clear();
  $("workspace").hidden = true;
  $("send").disabled = true;
  setConnection(message);
}
const delivery = (status) => {
  $("delivery").textContent =
    {
      awaiting_delivery: "Awaiting delivery…",
      submitted: "Submitted to the agent.",
      rejected: "Prompt rejected.",
      unknown:
        "Delivery outcome unknown. Check the agent before sending again.",
    }[status] || status;
};
function storage() {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open("mesophon", 1);
    req.onupgradeneeded = () => req.result.createObjectStore("device");
    req.onerror = () => reject(req.error);
    req.onsuccess = () => resolve(req.result);
  });
}
function stored() {
  return new Promise((resolve, reject) => {
    const req = db.transaction("device").objectStore("device").get("identity");
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}
function save() {
  return new Promise((resolve, reject) => {
    const tx = db.transaction("device", "readwrite");
    tx.objectStore("device").put(identity, "identity");
    tx.oncomplete = resolve;
    tx.onerror = () => reject(tx.error);
    tx.onabort = () => reject(tx.error);
  });
}
function boards() {
  const chosen = active?.pin?.board;
  $("boards").replaceChildren(node("option", "Choose a board"));
  $("boards").firstChild.value = "";
  for (const entry of identity.boards) {
    const opt = node("option", entry.title || "Paired board");
    opt.value = entry.pin.board;
    $("boards").append(opt);
  }
  $("boards").value = chosen || "";
}
function request(body, context) {
  if (!online || socket?.readyState !== WebSocket.OPEN) return;
  const id = next++;
  pending.set(id, { body, context, at: Date.now() });
  socket.send(
    crypto.packet(JSON.stringify({ incarnation, id, request: body })),
  );
  return id;
}
function refresh() {
  if (![...pending.values()].some((p) => p.body.op === "snapshot"))
    request({ op: "snapshot" });
}
function preview() {
  if (
    !document.hidden &&
    selected?.agent &&
    ![...pending.values()].some((p) => p.body.op === "preview")
  ) {
    request(
      { op: "preview", ticket: selected.id, session: selected.agent.session },
      selected.agent.session,
    );
  }
}
function select(ticket) {
  if (
    selected?.id !== ticket.id ||
    selected?.agent?.session !== ticket.agent?.session
  )
    $("preview").textContent = "";
  selected = ticket;
  $("selection").textContent = `${ticket.key} · ${ticket.title}`;
  $("agent-state").textContent = ticket.agent
    ? `${ticket.agent.provider} · ${ticket.agent.state}`
    : "No live agent";
  $("send").disabled =
    !online || !ticket.agent?.promptable || !!lastPrompt?.waiting;
  for (const b of $("tickets").querySelectorAll("button"))
    b.setAttribute("aria-pressed", String(b.dataset.id === ticket.id));
  preview();
}
async function answer(answer) {
  const { id, reply } = answer;
  const original = pending.get(id);
  pending.delete(id);
  if (reply.result === "changed") {
    refresh();
    return;
  }
  if (reply.result === "revoked") {
    endConnection("Access revoked. Pair again from the host.");
    return;
  }
  if (reply.result === "board") {
    tickets = reply.tickets;
    $("workspace").hidden = false;
    $("board-title").textContent = reply.title;
    $("tickets").replaceChildren();
    for (const column of reply.columns) {
      $("tickets").append(node("h3", column));
      for (const ticket of tickets.filter((t) => t.column === column)) {
        const b = node("button", `${ticket.key} · ${ticket.title}`);
        b.type = "button";
        b.dataset.id = ticket.id;
        b.onclick = () => select(ticket);
        $("tickets").append(b);
      }
    }
    if (active && active.title !== reply.title) {
      active.title = reply.title;
      await save();
      boards();
    }
    const current = tickets.find((t) => t.id === selected?.id);
    if (current) select(current);
    else {
      selected = undefined;
      $("selection").textContent = "Select a ticket";
      $("preview").textContent = "";
      $("agent-state").textContent = "";
      $("send").disabled = true;
    }
  } else if (reply.result === "preview") {
    if (selected?.agent?.session === original?.context)
      $("preview").textContent = reply.lines.join("\n");
  } else if (reply.result === "delivery") {
    if (
      (original?.body.op === "prompt"
        ? id
        : original?.body.op === "status"
          ? original.body.command
          : undefined) === lastPrompt?.id &&
      lastPrompt
    ) {
      delivery(reply.status);
      if (lastPrompt) lastPrompt.waiting = reply.status === "awaiting_delivery";
      if (
        original.body.op === "prompt" &&
        ["submitted", "awaiting_delivery"].includes(reply.status)
      )
        $("prompt").value = "";
      if (selected)
        $("send").disabled =
          !online || !selected.agent?.promptable || !!lastPrompt?.waiting;
    }
  } else if (reply.result === "rejected") {
    if (
      (original?.body.op === "prompt"
        ? id
        : original?.body.op === "status"
          ? original.body.command
          : undefined) === lastPrompt?.id &&
      lastPrompt
    ) {
      delivery(`Rejected: ${reply.message}`);
      if (lastPrompt) lastPrompt.waiting = false;
      if (selected) $("send").disabled = !online || !selected.agent?.promptable;
    } else if (
      original?.body.op === "preview" &&
      selected?.agent?.session === original.context
    )
      $("preview").textContent = reply.message;
  }
}
function disconnect() {
  online = false;
  $("send").disabled = true;
  if (
    [...pending.values()].some((p) => p.body.op === "prompt") ||
    lastPrompt?.waiting
  )
    delivery("unknown");
  pending.clear();
  setConnection("Disconnected. Waiting for the host…");
}
async function connect(entry, code, pairingAttempt = 0) {
  clearTimeout(reconnect);
  const gen = ++generation;
  if (lastPrompt && lastPrompt.board !== entry?.pin?.board) {
    lastPrompt = undefined;
    $("delivery").textContent = "";
    $("prompt").value = "";
  }
  online = false;
  $("send").disabled = true;
  socket?.close();
  pending.clear();
  selected = undefined;
  $("workspace").hidden = true;
  active = entry;
  crypto?.free();
  crypto = new Browser(identity.seed);
  const ws = new WebSocket(
    `${location.origin.replace(/^https:/, "wss:")}/control`,
  );
  socket = ws;
  setConnection(code ? "Pairing…" : "Connecting…");
  ws.onopen = () =>
    ws.send(
      crypto.auth(
        identity.credential || undefined,
        $("device-name").value.trim() || "My browser",
      ),
    );
  ws.onmessage = (event) => {
    processing = processing
      .then(async () => {
        if (gen !== generation) return;
        const wire = JSON.parse(event.data);
        if (wire.kind === "authenticated") {
          if (wire.credential) {
            identity.credential = wire.credential;
            await save();
          }
          ws.send(
            code
              ? crypto.pair(code)
              : crypto.connect(JSON.stringify(entry.pin)),
          );
        } else if (wire.kind === "welcome") {
          const welcome = JSON.stringify(wire.welcome);
          const ready = JSON.parse(
            crypto.accept(
              welcome,
              code || undefined,
              code ? undefined : JSON.stringify(entry.pin),
            ),
          ).reply;
          if (ready.result !== "ready")
            throw new Error("Unsupported host handshake");
          if (code) {
            entry = { pin: wire.welcome, title: "Paired board" };
            identity.boards = identity.boards.filter(
              (b) => b.pin.board !== entry.pin.board,
            );
            identity.boards.push(entry);
            active = entry;
            await save();
            code = undefined;
            $("code").value = "";
            boards();
          }
          incarnation = ready.incarnation;
          next = ready.next;
          online = true;
          setConnection("Connected");
          refresh();
          if (lastPrompt?.incarnation === incarnation)
            request({ op: "status", command: lastPrompt.id });
          else if (lastPrompt) {
            lastPrompt.waiting = false;
            delivery("unknown");
          }
        } else if (wire.kind === "packet")
          await answer(JSON.parse(crypto.open(event.data)));
        else if (wire.kind === "error") {
          setConnection("Host unavailable, or access no longer granted.");
          ws.close();
        }
      })
      .catch(() => {
        if (gen === generation) {
          endConnection("Connection did not verify. Pair again from the host.");
        }
      });
  };
  ws.onclose = () => {
    if (gen !== generation) return;
    disconnect();
    if (active) reconnect = setTimeout(() => connect(active), 3000);
    else if (code && pairingAttempt < 3)
      reconnect = setTimeout(
        () => connect(undefined, code, pairingAttempt + 1),
        1000,
      );
    else if (code)
      setConnection(
        "Pairing did not complete. Generate a new code on the host and try again.",
      );
  };
  ws.onerror = () => {
    if (gen === generation)
      setConnection(
        "Cannot reach the relay. Check its address and certificate.",
      );
  };
}
$("pair-form").onsubmit = async (event) => {
  event.preventDefault();
  const code = $("code").value.trim();
  if (!code) {
    setConnection("Enter the code from the host’s Mesophon dialog.");
    return;
  }
  await connect(undefined, code);
};
$("connect").onclick = () => {
  const entry = identity.boards.find((b) => b.pin.board === $("boards").value);
  if (entry) connect(entry);
};
$("prompt-form").onsubmit = (event) => {
  event.preventDefault();
  if (!online || !selected?.agent?.promptable || lastPrompt?.waiting) return;
  const text = $("prompt").value;
  if (new TextEncoder().encode(text).length > 4096) {
    delivery("Prompt must fit in 4096 UTF-8 bytes.");
    return;
  }
  const id = request({
    op: "prompt",
    ticket: selected.id,
    session: selected.agent.session,
    text,
  });
  if (id) {
    lastPrompt = { id, incarnation, board: active.pin.board, waiting: true };
    delivery("awaiting_delivery");
    $("send").disabled = true;
  }
};
$("forget").onclick = async () => {
  ++generation;
  active = undefined;
  clearTimeout(reconnect);
  socket?.close();
  online = false;
  pending.clear();
  lastPrompt = undefined;
  crypto?.free();
  crypto = new Browser();
  identity = { seed: crypto.seed(), boards: [] };
  await save();
  $("workspace").hidden = true;
  $("prompt").value = "";
  $("preview").textContent = "";
  $("code").value = "";
  boards();
  setConnection("Device forgotten. Pair again to connect.");
};
setInterval(() => {
  if (online && !document.hidden) {
    preview();
    if (
      lastPrompt?.waiting &&
      ![...pending.values()].some(
        (p) => p.body.op === "status" || p.body.op === "prompt",
      )
    )
      request({ op: "status", command: lastPrompt.id });
  }
  for (const [id, p] of pending)
    if (Date.now() - p.at > 10000) {
      pending.delete(id);
      if (p.body.op === "prompt" && lastPrompt?.id === id) {
        delivery("unknown");
        lastPrompt.waiting = false;
        if (selected)
          $("send").disabled = !online || !selected.agent?.promptable;
      }
    }
}, 2000);
try {
  await init();
  db = await storage();
  identity = await stored();
  crypto = new Browser(identity?.seed);
  if (!identity) {
    identity = { seed: crypto.seed(), boards: [] };
    await save();
  }
  boards();
  setConnection("Enable Mesophon on the host, then pair with its code.");
} catch {
  setConnection(
    "Could not load the browser module or device storage. Check the deployment and browser storage permissions.",
  );
}
