// The relay's mailbox (T-497): this browser's own socket, apart from the
// host channel, where tickets wait for an away host. It opens whether or not
// the host is reachable and carries only sealed envelopes and their answers.
// The store decides what to send; this owns the socket and its retries.
export class Mailbox {
  constructor({ crypto, identity, onReady, onFrame, onDown }) {
    Object.assign(this, { crypto, identity, onReady, onFrame, onDown });
    this.generation = 0;
    this.ready = false;
    this.wanted = false;
    this.delay = 3000;
  }
  // Keep a socket open while a paired board is on screen.
  want(wanted) {
    this.wanted = wanted;
    if (!wanted) this.stop();
    else if (!this.socket) this.connect();
  }
  // Try now rather than at the next retry: the network came back.
  poke() {
    if (!this.wanted || this.ready) return;
    clearTimeout(this.retry);
    this.delay = 3000;
    this.stop(false);
    this.connect();
  }
  stop(forget = true) {
    ++this.generation;
    clearTimeout(this.retry);
    this.ready = false;
    const socket = this.socket;
    this.socket = undefined;
    socket?.close();
    if (forget) this.delay = 3000;
  }
  send(frame) {
    if (!this.ready || this.socket?.readyState !== WebSocket.OPEN) return false;
    try {
      this.socket.send(JSON.stringify(frame));
      return true;
    } catch {
      this.socket.close();
      return false;
    }
  }
  connect() {
    if (!this.identity.credential || navigator.onLine === false) return;
    const gen = ++this.generation;
    const ws = (this.socket = new WebSocket(`${location.origin.replace(/^http/, "ws")}/control`));
    const deadline = setTimeout(() => ws.close(), 12000);
    ws.onopen = () => {
      if (gen === this.generation)
        ws.send(this.crypto.auth(this.identity.credential, this.identity.name || "My browser"));
    };
    ws.onmessage = (event) => {
      if (gen !== this.generation) return;
      let wire;
      try {
        wire = JSON.parse(event.data);
      } catch {
        ws.close();
        return;
      }
      if (wire.kind === "authenticated") {
        clearTimeout(deadline);
        this.ready = true;
        this.delay = 3000;
        this.onReady();
      } else if (this.ready) this.onFrame(wire);
    };
    ws.onclose = () => {
      clearTimeout(deadline);
      if (gen !== this.generation) return;
      const was = this.ready;
      this.ready = false;
      this.socket = undefined;
      if (was) this.onDown();
      if (this.wanted) {
        this.retry = setTimeout(() => this.connect(), this.delay);
        this.delay = Math.min(this.delay * 2, 30000);
      }
    };
    ws.onerror = () => ws.close();
  }
}
