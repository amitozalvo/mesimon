// The bounded host projection and local list preferences, one instance per board.
const phase = (ticket) =>
  ticket.agent?.state === "needs attention"
    ? "needs"
    : ["starting", "working"].includes(ticket.agent?.state)
      ? "working"
      : "idle";
// How long a stopped agent stays on Now (T-560): about an hour.
export const RECENT_MS = 3600000;
// Whether Now lists this agent: one that needs you or works, always; any
// other only while its state is under an hour old at `now`. An age it cannot
// tell (an older host sends no since) keeps the agent.
const recent = (ticket, now) => {
  const age = now - ticket.agent?.since;
  return phase(ticket) !== "idle" || !(age >= RECENT_MS);
};

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
    // The agent tiers a start or a prompt may pick (T-643): live only, as
    // the pick is, so a remembered board has none.
    this.tiers = [];
    // The tickets paired browsers filed that are archived now (T-665):
    // Sent's own, never on the board. Absent from an older host.
    this.archived = [];
  }
  update(reply, { cached = false, at = Date.now() } = {}) {
    Object.assign(this, {
      title: reply.title,
      tickets: reply.tickets,
      columns: reply.columns,
      defaultColumn: reply.default_column || "",
      columnDescriptions: reply.column_descriptions || {},
      allowedTags: Array.isArray(reply.allowed_tags) ? reply.allowed_tags : [],
      tiers: Array.isArray(reply.tiers) ? reply.tiers : [],
      archived: Array.isArray(reply.archived) ? reply.archived : [],
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
  // Where a ticket this browser filed is now (T-665): on the board, archived,
  // or neither (deleted, or a host that does not say).
  whereIs(id) {
    const live = this.tickets.find((t) => t.id === id);
    if (live) return { ticket: live };
    const archived = this.archived.find((t) => t.id === id);
    return archived ? { ticket: archived, archived: true } : {};
  }
  visible() {
    const query = this.search.toLocaleLowerCase().trim();
    // A remembered board is measured at the moment it describes, not now.
    const now = this.cached ? this.receivedAt : Date.now();
    const visible = this.tickets.filter(
      (t) =>
        (this.mode === "board" || t.agent) &&
        (this.mode !== "agents" || recent(t, now)) &&
        (this.mode !== "agents" ||
          this.filter === "all" ||
          (this.filter === "running"
            ? ["starting", "working"].includes(t.agent?.state)
            : t.agent?.state === "needs attention")) &&
        (!query ||
          [t.key, t.title, t.column, t.agent?.provider, t.agent?.state, t.workspace?.branch, ...(t.tags || []).map((g) => g.name)]
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
  // The tiers a ticket may pick (T-643), the desk's `^n` ring: a seat that
  // holds an agent, awake or parked, keeps its provider, so it is offered
  // that provider's tiers alone.
  tierRing(ticket) {
    const provider = ticket?.agent?.provider;
    return this.tiers.filter((t) => !provider || t.provider === provider);
  }
  tierNamed(id) {
    return this.tiers.find((t) => t.id === id);
  }
  // The column a new ticket starts in: the host's default, else the first.
  landing() {
    return this.columns.includes(this.defaultColumn) ? this.defaultColumn : this.columns[0] || "";
  }
  // What this browser may remember: keys, titles, columns, tags, note counts,
  // the crown's seat, the worktree's branch and state and agent states.
  // Whether the workspace may still change is the live host's to say. No queued text, tool input, dialogs,
  // the agent's step and reply line (T-497) or the crown's last touch: those
  // are output, shown live and never kept.
  snapshot() {
    const card = ({ id, key, title, column, tags }) => ({
      id,
      key,
      title,
      column,
      tags: (tags || []).map(({ group, name, tint }) => ({ group, name, tint })),
    });
    return {
      // Sent's archived tickets (T-665), as their cards read: no agent.
      ...(this.archived.length && { archived: this.archived.map(card) }),
      title: this.title,
      columns: this.columns,
      default_column: this.defaultColumn,
      column_descriptions: this.columnDescriptions,
      allowed_tags: this.allowedTags.map(({ group, name, tint }) => ({ group, name, tint })),
      tickets: this.tickets.map(({ id, key, title, column, agent, tags, notes, noted, crown, workspace }) => ({
        id,
        key,
        title,
        column,
        // Who wears the crown is a fact of the board (T-623); what it last
        // did is news, shown live and never kept.
        ...(crown === true && { crown }),
        tags: (tags || []).map(({ group, name, tint }) => ({ group, name, tint })),
        // How many notes, and their digest (T-532): the notes this browser
        // read are kept apart, in `notes:<grant>`.
        notes: Number.isInteger(notes) ? notes : 0,
        noted: typeof noted === "string" ? noted : "",
        // Where its code lives (T-642), as the card marks it.
        ...(workspace && {
          workspace: {
            kind: workspace.kind,
            ...(workspace.branch && { branch: workspace.branch }),
            ...(workspace.state && { state: workspace.state }),
            ...(workspace.ahead && { ahead: workspace.ahead }),
          },
        }),
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
