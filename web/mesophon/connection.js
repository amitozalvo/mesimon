// Owns only transport, authentication, request correlation and reconnect.
// A reconnect may query receipts; it never retains or replays request bodies.
export class Connection {
  constructor({ Browser, identity, save, onState, onReady, onReply, onLost }) {
    Object.assign(this, {
      Browser,
      identity,
      save,
      onState,
      onReady,
      onReply,
      onLost,
    });
    this.generation = 0;
    this.pending = new Map();
    this.processing = Promise.resolve();
    this.online = false;
  }
  stop() {
    ++this.generation;
    clearTimeout(this.retry);
    clearTimeout(this.deadline);
    this.online = false;
    this.socket?.close();
    this.pending.clear();
    this.crypto?.free();
    this.crypto = undefined;
    this.onLost();
  }
  request(body, context) {
    if (!this.online || this.socket?.readyState !== WebSocket.OPEN) return;
    const id = this.next++;
    try {
      this.socket.send(
        this.crypto.packet(
          JSON.stringify({ incarnation: this.incarnation, id, request: body }),
        ),
      );
      this.pending.set(id, { body, context, at: Date.now() });
      return id;
    } catch {
      this.socket.close();
      return undefined;
    }
  }
  has(op) {
    return [...this.pending.values()].some((p) => p.body.op === op);
  }
  tick() {
    if ([...this.pending.values()].some((p) => Date.now() - p.at > 10000)) {
      this.online = false;
      this.onLost();
      this.onState(
        "offline",
        "Host not responding. Last received view is stale. Reconnecting…",
      );
      this.socket?.close();
    }
  }
  async connect(entry, code, attempt = 0) {
    this.stop();
    const gen = this.generation;
    this.entry = entry;
    this.onState(
      "connecting",
      code ? "Pairing…" : "Connecting… Last received view is stale.",
    );
    this.crypto = new this.Browser(this.identity.seed);
    const ws = (this.socket = new WebSocket(
      `${location.origin.replace(/^http/, "ws")}/control`,
    ));
    let terminal = false;
    this.deadline = setTimeout(() => ws.close(), 12000);
    ws.onopen = () => {
      if (gen === this.generation)
        ws.send(
          this.crypto.auth(
            this.identity.credential || undefined,
            this.identity.name || "My browser",
          ),
        );
    };
    ws.onmessage = (event) => {
      this.processing = this.processing
        .then(async () => {
          if (gen !== this.generation) return;
          const wire = JSON.parse(event.data);
          if (wire.kind === "authenticated") {
            if (wire.credential) {
              this.identity.credential = wire.credential;
              await this.save();
              if (gen !== this.generation) return;
            }
            ws.send(
              code
                ? this.crypto.pair(code)
                : this.crypto.connect(JSON.stringify(entry.pin)),
            );
          } else if (wire.kind === "welcome") {
            const ready = JSON.parse(
              this.crypto.accept(
                JSON.stringify(wire.welcome),
                code || undefined,
                code ? undefined : JSON.stringify(entry.pin),
              ),
            ).reply;
            if (ready.result !== "ready")
              throw new Error("Unsupported handshake");
            if (code) {
              entry = { pin: wire.welcome, title: "Paired board" };
              this.identity.boards = this.identity.boards.filter(
                (b) => b.pin.board !== entry.pin.board,
              );
              this.identity.boards.push(entry);
              this.identity.lastBoard = entry.pin.board;
              await this.save();
              if (gen !== this.generation) return;
              this.entry = entry;
              code = undefined;
            }
            clearTimeout(this.deadline);
            this.incarnation = ready.incarnation;
            this.next = ready.next;
            this.online = true;
            this.onState("loading", "Connected. Loading board…");
            this.onReady(entry);
          } else if (wire.kind === "packet") {
            const { id, reply } = JSON.parse(this.crypto.open(event.data));
            const original = this.pending.get(id);
            this.pending.delete(id);
            if (reply.result === "revoked") {
              terminal = true;
              this.stop();
              this.onState(
                "revoked",
                "Access revoked. Pair again from the host to restore access.",
              );
            } else this.onReply(reply, original, id);
          } else if (wire.kind === "error") {
            // The relay deliberately cannot distinguish an offline host from a
            // removed grant. Only an authenticated Revoked clears protected data.
            ws.close();
          }
        })
        .catch(() => {
          if (gen !== this.generation) return;
          terminal = true;
          this.stop();
          this.onState(
            "unverified",
            "Connection did not verify. Pair again from the host.",
          );
        });
    };
    ws.onclose = () => {
      if (gen !== this.generation || terminal) return;
      ++this.generation;
      clearTimeout(this.deadline);
      this.online = false;
      this.pending.clear();
      this.onLost();
      this.onState(
        "offline",
        "Disconnected. Last received view is stale. Waiting for the host…",
      );
      if (entry) this.retry = setTimeout(() => this.connect(entry), 3000);
      else if (code && attempt < 3)
        this.retry = setTimeout(
          () => this.connect(undefined, code, attempt + 1),
          1000,
        );
      else
        this.onState(
          "unpaired",
          "Pairing did not complete. Generate a new code on the host and try again.",
        );
    };
    ws.onerror = () => ws.close();
  }
}
