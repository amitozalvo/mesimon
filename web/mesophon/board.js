// The bounded host projection and local list preferences, one instance per board.
const phase = (ticket) =>
  ticket.agent?.state === "needs attention"
    ? "needs"
    : ["starting", "working"].includes(ticket.agent?.state)
      ? "working"
      : "idle";

export class BoardState {
  constructor(selected) {
    this.selected = selected;
    this.tickets = [];
    this.columns = [];
    this.mode = "agents";
    this.filter = "all";
    this.search = "";
    this.column = "";
    this.scroll = { agents: 0, board: 0 };
    this.receivedAt = undefined;
    // True while the view comes from this browser's memory, not the host.
    this.cached = false;
  }
  update(reply, { cached = false, at = Date.now() } = {}) {
    Object.assign(this, {
      title: reply.title,
      tickets: reply.tickets,
      columns: reply.columns,
      cached,
      receivedAt: at,
    });
    if (!this.columns.includes(this.column))
      this.column = this.columns[0] || "";
    if (!this.tickets.some((t) => t.id === this.selected))
      this.selected = (
        this.tickets.find((t) => t.agent) || this.tickets[0]
      )?.id;
  }
  get current() {
    return this.tickets.find((t) => t.id === this.selected);
  }
  visible() {
    const query = this.search.toLocaleLowerCase().trim();
    const visible = this.tickets.filter(
      (t) =>
        (this.mode === "board" || t.agent) &&
        (this.mode !== "agents" ||
          this.filter === "all" ||
          (this.filter === "running"
            ? ["starting", "working"].includes(t.agent?.state)
            : t.agent?.state === "needs attention")) &&
        (!query ||
          [t.key, t.title, t.column, t.agent?.provider, t.agent?.state]
            .join(" ")
            .toLocaleLowerCase()
            .includes(query)),
    );
    // Use only the state supplied by the host; no inferred approval/question.
    if (this.mode === "agents")
      visible.sort(
        (a, b) =>
          Number(b.agent.state === "needs attention") -
          Number(a.agent.state === "needs attention"),
      );
    return visible;
  }
  // Now's three groups, by the host's own state word and nothing else.
  sections() {
    const groups = { needs: [], working: [], idle: [] };
    for (const ticket of this.visible()) groups[phase(ticket)].push(ticket);
    return groups;
  }
  // What this browser may remember: no queued text, tool input or dialogs.
  snapshot() {
    return {
      title: this.title,
      columns: this.columns,
      tickets: this.tickets.map(({ id, key, title, column, agent }) => ({
        id,
        key,
        title,
        column,
        agent: agent && {
          session: agent.session,
          provider: agent.provider,
          state: agent.state,
          promptable: false,
        },
      })),
    };
  }
}
