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
    this.scroll = { agents: 0, board: 0, sent: 0 };
    this.receivedAt = undefined;
    // True while the view comes from this browser's memory, not the host.
    this.cached = false;
    // What the New ticket sheet offers; absent from an older host's reply.
    this.defaultColumn = "";
    this.columnDescriptions = {};
    this.allowedTags = [];
  }
  update(reply, { cached = false, at = Date.now() } = {}) {
    Object.assign(this, {
      title: reply.title,
      tickets: reply.tickets,
      columns: reply.columns,
      defaultColumn: reply.default_column || "",
      columnDescriptions: reply.column_descriptions || {},
      allowedTags: Array.isArray(reply.allowed_tags) ? reply.allowed_tags : [],
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
          [t.key, t.title, t.column, t.agent?.provider, t.agent?.state, ...(t.tags || []).map((g) => g.name)]
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
  // The column a new ticket starts in: the host's default, else the first.
  landing() {
    return this.columns.includes(this.defaultColumn) ? this.defaultColumn : this.columns[0] || "";
  }
  // What this browser may remember: keys, titles, columns, tags and agent
  // states. No queued text, tool input, dialogs, or the agent's step and
  // reply line (T-497): those are output, shown live and never kept.
  snapshot() {
    return {
      title: this.title,
      columns: this.columns,
      default_column: this.defaultColumn,
      column_descriptions: this.columnDescriptions,
      allowed_tags: this.allowedTags.map(({ group, name, tint }) => ({ group, name, tint })),
      tickets: this.tickets.map(({ id, key, title, column, agent, tags }) => ({
        id,
        key,
        title,
        column,
        tags: (tags || []).map(({ group, name, tint }) => ({ group, name, tint })),
        agent: agent && {
          session: agent.session,
          provider: agent.provider,
          state: agent.state,
          since: agent.since,
          promptable: false,
        },
      })),
    };
  }
}
