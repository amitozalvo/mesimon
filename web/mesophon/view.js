export const $ = (id) => document.getElementById(id);
const node = (tag, text, className) => {
  const n = document.createElement(tag);
  n.textContent = text;
  if (className) n.className = className;
  return n;
};
export class View {
  constructor(select) {
    this.select = select;
  }
  boards(identity, active) {
    $("boards").replaceChildren(
      ...identity.boards.map((entry) => {
        const opt = node(
          "option",
          `${entry.title || "Paired board"}${entry.revoked ? " · Access removed" : ""}`,
        );
        opt.value = entry.pin.board;
        return opt;
      }),
    );
    $("boards").value = active?.pin.board || "";
  }
  list(board) {
    $("board-title").textContent = board.title || "Paired board";
    $("list-title").textContent =
      board.mode === "agents" ? "Your agents" : "Board";
    $("search").value = board.search;
    $("filter").value = board.filter;
    $("filter-label").hidden = board.mode !== "agents";
    $("column-label").hidden = board.mode !== "board";
    $("agents-mode").setAttribute(
      "aria-pressed",
      String(board.mode === "agents"),
    );
    $("board-mode").setAttribute(
      "aria-pressed",
      String(board.mode === "board"),
    );
    const tickets = board.visible();
    $("count").textContent =
      `${tickets.length} ${board.mode === "agents" ? "agent" : "ticket"}${tickets.length === 1 ? "" : "s"}`;
    const signature = JSON.stringify([
      tickets,
      board.selected,
      board.mode,
      board.columns,
      board.column,
    ]);
    if (this.listSignature === signature) return;
    this.listSignature = signature;
    const focused = $("tickets").contains(document.activeElement)
      ? document.activeElement.dataset.id
      : undefined;
    $("column").replaceChildren(
      ...board.columns.map((column) => {
        const opt = node("option", column);
        opt.value = column;
        return opt;
      }),
    );
    $("column").value = board.column;
    const row = (ticket) => {
      const b = node("button", "", "ticket");
      b.type = "button";
      b.dataset.id = ticket.id;
      b.setAttribute("aria-pressed", String(ticket.id === board.selected));
      const top = node("span", "", "ticket-top");
      top.append(
        node("span", ticket.key, "ticket-key"),
        node("span", ticket.agent?.provider || "No agent"),
      );
      const meta = node("span", "", "ticket-meta");
      meta.append(
        node("span", ticket.agent?.state || "No live agent", "ticket-state"),
        node("span", ticket.column),
      );
      b.append(top, node("span", ticket.title, "ticket-title"), meta);
      b.onclick = () => this.select(ticket.id);
      return b;
    };
    if (board.mode === "board") {
      $("tickets").replaceChildren(
        ...board.columns.map((column) => {
          const section = node(
            "section",
            "",
            `board-column${column === board.column ? " chosen-column" : ""}`,
          );
          section.append(node("h3", column));
          const group = tickets.filter((t) => t.column === column);
          section.append(
            ...(group.length
              ? group.map(row)
              : [node("p", "No tickets in this column.", "empty")]),
          );
          return section;
        }),
      );
    } else $("tickets").replaceChildren(...tickets.map(row));
    if (!tickets.length)
      $("tickets").append(
        node(
          "p",
          board.search
            ? "No tickets match your search."
            : board.tickets.length
              ? "No agents here. Open Board to see every ticket."
              : "This board has no tickets yet.",
          "empty",
        ),
      );
    $("tickets").scrollTop = board.scroll[board.mode];
    if (focused)
      [...$("tickets").querySelectorAll("button")]
        .find((b) => b.dataset.id === focused)
        ?.focus({ preventScroll: true });
  }
  capture(entry) {
    if (
      !entry ||
      this.entryKey !== entry.key ||
      !$("preview").getClientRects().length
    )
      return;
    entry.scroll = $("preview").scrollTop;
  }
  detail(ticket, entry, live) {
    const changed = this.entryKey !== entry?.key;
    this.entryKey = entry?.key;
    $("selection").textContent = ticket
      ? `${ticket.key} · ${ticket.title}`
      : "Select a ticket";
    $("agent-state").textContent = ticket?.agent
      ? `${ticket.agent.provider} · ${ticket.agent.state} · ${ticket.column}`
      : ticket
        ? "No live agent. Start an agent from the host to send input."
        : "Choose an agent, or open Board to see all tickets.";
    $("target").textContent = ticket?.agent
      ? `Message ${ticket.agent.provider} on ${ticket.key} · session ${ticket.agent.session.slice(0, 8)}`
      : "No agent selected";
    const text =
      entry?.displayed ||
      (entry?.receivedAt
        ? "No output in the latest preview."
        : ticket?.agent
          ? "Waiting for the first preview…"
          : "No agent output.");
    if ($("preview").textContent !== text) $("preview").textContent = text;
    if (changed)
      $("preview").scrollTop = entry?.following
        ? $("preview").scrollHeight
        : entry?.scroll || 0;
    else if (entry?.following)
      $("preview").scrollTop = $("preview").scrollHeight;
    if ($("prompt").value !== (entry?.draft || ""))
      $("prompt").value = entry?.draft || "";
    $("prompt").disabled = !ticket?.agent;
    $("prompt-mode").value = entry?.mode || "queue";
    $("send").textContent =
      entry?.mode === "steer" ? "Send prompt ↑" : "Queue prompt";
    $("queued-row").hidden = ticket?.queued == null;
    $("queued-text").textContent = ticket?.queued || "";
    const acting = entry?.receipt?.waiting && entry.receipt.status !== "queued";
    $("send-now").disabled = $("take-back").disabled =
      !live || !ticket?.agent?.promptable || ticket?.queued == null || !!acting;
    $("returned-row").hidden = !entry?.returned;
    $("returned-text").textContent = entry?.returned?.text || "";
    $("send").disabled =
      !live ||
      !ticket?.agent?.promptable ||
      !!entry?.review ||
      !!entry?.receipt?.waiting ||
      !entry?.draft.trim();
    $("session-warning").hidden = !entry?.review || !ticket?.agent;
    const delivery = entry?.delivery || "";
    if ($("delivery").textContent !== delivery)
      $("delivery").textContent = delivery;
    $("latest").hidden = !entry || entry.following;
    $("latest").textContent = entry?.unread
      ? "↓ New preview · Jump to latest"
      : "↓ Jump to latest";
    $("freshness").textContent =
      `Periodic preview · up to 50 lines${entry?.receivedAt ? ` · Last received ${new Date(entry.receivedAt).toLocaleTimeString()}` : " · Nothing received yet"}${live ? "" : " · Stale / offline"}`;
  }
  clear() {
    this.entryKey = undefined;
    this.listSignature = undefined;
    $("tickets").replaceChildren();
    $("preview").textContent = "";
    $("prompt").value = "";
    $("delivery").textContent = "";
    $("selection").textContent = "Select a ticket";
    $("agent-state").textContent = "";
    $("target").textContent = "No agent selected";
    $("freshness").textContent = "Nothing received yet";
    $("session-warning").hidden = true;
    $("latest").hidden = true;
    $("send").disabled = true;
    $("queued-row").hidden = $("returned-row").hidden = true;
    $("queued-text").textContent = $("returned-text").textContent = "";
    $("send-now").disabled = $("take-back").disabled = true;
  }
}
