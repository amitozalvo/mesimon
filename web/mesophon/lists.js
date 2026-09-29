// The work list: Now (agents grouped by the host's state word), Board
// (every ticket, by column) and Sent (the tickets this browser filed). Rows
// are buttons; the pressed one is selected.
import { html } from "./html.js";
import { Icon, Tick } from "./icons.js";
import { Shin } from "./shin.js";
import { answerable, requestSummary } from "./dialogs.js";

const clock = (at) => new Date(at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
export const lastSeen = (board) => (board?.receivedAt ? clock(board.receivedAt) : "");

// A ticket this browser sent carries a small phone mark on the board.
function FromHere({ store, ticket }) {
  return store.sentHere().has(ticket.id)
    ? html`<span class="from-here" title="Sent from this browser"><${Icon} name="smartphone" size=${13} /><span class="sr-only">Sent from this browser</span></span>`
    : null;
}

function StateMark({ ticket }) {
  const state = ticket.agent?.state;
  if (state === "needs attention") return html`<span class="mark mark-attn" aria-hidden="true"></span>`;
  if (state === "starting" || state === "working") return html`<${Icon} name="spinner" size=${16} width=${2.4} cls="spin mark-icon" />`;
  if (!ticket.agent) return html`<span class="mark mark-none" aria-hidden="true"></span>`;
  return html`<span class="mark mark-idle" aria-hidden="true"></span>`;
}

function Row({ store, ticket, board, live }) {
  const pressed = ticket.id === board.selected;
  const agent = ticket.agent;
  return html`<button type="button" class="ticket row" data-id=${ticket.id} aria-pressed=${String(pressed)}
    onClick=${() => store.select(ticket.id)}>
    <${StateMark} ticket=${ticket} />
    <span class="row-text">
      <span class="ticket-title" dir="auto">${ticket.title}</span>
      <span class="ticket-meta"><span class="ticket-key">${ticket.key}</span>${agent ? ` · ${agent.provider} · ${agent.state}` : " · No agent"} · ${ticket.column}<${FromHere} store=${store} ticket=${ticket} /></span>
    </span>
    <${Icon} name="chevronRight" size=${16} cls="row-go" />
  </button>`;
}

// A needs-you card answers what it can in place; the rest opens the ticket.
function NeedCard({ store, ticket, board, live }) {
  const pressed = ticket.id === board.selected;
  const { permission, dialog, session } = ticket.agent;
  const entry = (permission || dialog) ? store.entryFor(ticket) : undefined;
  const off = !live || !!entry?.receipt?.waiting;
  const send = (body) => store.sendInteraction({ ...body, ticket: ticket.id, session }, entry);
  const kind = permission ? "needs approval" : dialog?.kind === "plan" ? "has a plan" : dialog ? "has a question" : "needs you";
  const question = answerable(dialog) ? dialog.questions[0] : undefined;
  const expired = permission && Date.now() >= permission.expires_at;
  return html`<article class="need">
    <button type="button" class="ticket need-open" data-id=${ticket.id} aria-pressed=${String(pressed)}
      onClick=${() => store.select(ticket.id)}>
      <span class="ticket-meta"><span class="mark mark-attn" aria-hidden="true"></span><span class="ticket-key">${ticket.key}</span> · ${ticket.agent.provider} ${kind}</span>
      <span class="ticket-title" dir="auto">${ticket.title}</span>
    </button>
    ${permission && html`<div class="need-body">
      <p class="need-line"><${Icon} name="shield" size=${15} cls="attn-ink" /><span>Wants to use ${permission.tool}</span></p>
      <pre class="need-code">${requestSummary(permission)}</pre>
    </div>`}
    ${question && html`<div class="need-body">
      <p class="need-question" dir="auto">${question.question}</p>
      <div class="options">${question.options.map((option, index) => {
        const described = option.description ? `need-${dialog.request}-${index}` : undefined;
        return html`<button type="button" class="option" disabled=${off} aria-label=${option.label} aria-describedby=${described}
          onClick=${() => send({ op: "dialog", request: dialog.request, response: { answer: "choice", index } })}>
          <span class="option-label">${option.label}</span>
          ${described && html`<span class="option-desc" id=${described}>${option.description}</span>`}
        </button>`;
      })}</div>
    </div>`}
    ${!live
      ? html`<p class="need-note"><${Icon} name="moon" size=${14} /><span>Answer this when your terminal is back.</span></p>`
      : permission
        ? html`<div class="need-actions">
            <button type="button" class="btn" disabled=${off || expired}
              onClick=${() => send({ op: "permission", request: permission.request, decision: "deny" })}>Deny</button>
            <button type="button" class="btn btn-attn" disabled=${off || expired}
              onClick=${() => send({ op: "permission", request: permission.request, decision: "allow" })}>Approve once</button>
          </div>`
        : dialog?.kind === "plan" || (dialog && !question)
          ? html`<div class="need-actions"><button type="button" class="btn btn-attn" onClick=${() => store.select(ticket.id)}>${dialog.kind === "plan" ? "Review plan" : "Answer in the ticket"}</button></div>`
          : null}
    ${entry?.delivery && html`<p class="need-delivery" role="status">${entry.delivery}</p>`}
  </article>`;
}

function Group({ label, count, attn, children }) {
  return html`<section class="group">
    <h3 class="group-label">${attn && html`<span class="mark mark-attn pulse" aria-hidden="true"></span>`}<span>${label}</span><span class="group-count">${count}</span></h3>
    ${children}
  </section>`;
}

// Shown while this view is not live: what is known, and since when.
function Asleep({ store }) {
  const link = store.link;
  if (link === "live" || link === "connecting") return null;
  const nonet = link === "nonet" || link === "relay";
  const seen = lastSeen(store.board);
  return html`<section class="hero" aria-label="Connection">
    <div class="hero-top">
      <${Shin} size="medium" scale=${3} mood=${nonet ? "surprised" : "asleep"} light="dim" />
      <div>
        <h3>${link === "nonet" ? "You’re offline" : link === "relay" ? "The relay is out of reach" : "Your terminal is out of reach"}</h3>
        <p class="hero-sub">${seen ? `Last seen ${seen}` : "Not seen yet"}</p>
      </div>
    </div>
    <p class="hero-body">${link === "asleep"
      ? "Your Mac may be asleep, or mesimon isn’t running. This is the board as it was then. Answering, prompting and sending tickets need your terminal back."
      : "This browser can’t reach the relay right now. This is the board as it was when it last could."}</p>
  </section>`;
}

export function NowList({ store, board, live }) {
  const { needs, working, idle } = board.sections();
  const empty = !needs.length && !working.length && !idle.length;
  return html`
    <${Asleep} store=${store} />
    ${needs.length > 0 && html`<${Group} label="Needs you" count=${needs.length} attn=${true}>
      <div class="needs">${needs.map((t) => html`<${NeedCard} key=${t.id} store=${store} ticket=${t} board=${board} live=${live} />`)}</div>
    </${Group}>`}
    ${working.length > 0 && html`<${Group} label="Working" count=${working.length}>
      <div class="rows">${working.map((t) => html`<${Row} key=${t.id} store=${store} ticket=${t} board=${board} live=${live} />`)}</div>
    </${Group}>`}
    ${idle.length > 0 && html`<${Group} label="Idle" count=${idle.length}>
      <div class="rows">${idle.map((t) => html`<${Row} key=${t.id} store=${store} ticket=${t} board=${board} live=${live} />`)}</div>
    </${Group}>`}
    ${empty && html`<p class="empty">${board.search
      ? "No tickets match your search."
      : board.tickets.length
        ? "No agents here. Open Board to see every ticket."
        : "This board has no tickets yet."}</p>`}
  `;
}

function Card({ store, ticket, board }) {
  const pressed = ticket.id === board.selected;
  const agent = ticket.agent;
  const needs = agent?.state === "needs attention";
  return html`<button type="button" class=${`ticket card${needs ? " card-attn" : ""}`} data-id=${ticket.id}
    aria-pressed=${String(pressed)} onClick=${() => store.select(ticket.id)}>
    <span class="ticket-meta"><span class="ticket-key">${ticket.key}</span><${FromHere} store=${store} ticket=${ticket} /></span>
    <span class="ticket-title" dir="auto">${ticket.title}</span>
    ${agent && html`<span class=${`card-agent${needs ? " attn-ink" : ""}`}><${StateMark} ticket=${ticket} /><span>${agent.provider} · ${agent.state}</span></span>`}
  </button>`;
}

// Phone: one column at a time. Tablet: columns stacked. Desktop: side by side.
export function BoardList({ store, board, bp }) {
  const tickets = board.visible();
  const columns = bp === "phone" ? board.columns.filter((c) => c === board.column) : board.columns;
  if (!tickets.length && board.search) return html`<p class="empty">No tickets match your search.</p>`;
  if (!board.tickets.length) return html`<p class="empty">This board has no tickets yet.</p>`;
  return html`<div class=${bp === "desktop" ? "kanban" : bp === "phone" ? "stack single" : "stack"}>${columns.map((column) => {
    const group = tickets.filter((t) => t.column === column);
    const about = board.columnDescriptions[column];
    return html`<section class="column" key=${column} aria-label=${column}>
      <h3 class="column-label"><span>${column}</span><span class="group-count">${group.length}</span></h3>
      ${about && html`<p class="column-about" dir="auto">${about}</p>`}
      <div class="cards">${group.length
        ? group.map((t) => html`<${Card} key=${t.id} store=${store} ticket=${t} board=${board} />`)
        : html`<p class="empty column-empty">No tickets in this column.</p>`}</div>
      ${bp !== "phone" && html`<button type="button" class="add-to-column" data-column=${column}
        onClick=${() => store.openComposer(column)}><${Icon} name="plus" size=${16} /><span>Add to ${column}</span></button>`}
    </section>`;
  })}</div>`;
}

const dayOf = (at) => {
  const day = new Date(at);
  day.setHours(0, 0, 0, 0);
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  const days = Math.round((today - day) / 86400000);
  return days === 0
    ? "Today"
    : days === 1
      ? "Yesterday"
      : day.toLocaleDateString([], { weekday: "short", day: "numeric", month: "short" });
};

function SentItem({ store, board, item }) {
  const onBoard = !!item.ticket && board.tickets.some((t) => t.id === item.ticket);
  const tick = item.status === "sending" ? "clock" : item.status === "landed" ? "two" : null;
  const said = { sending: "Sending", landed: "On your board", unknown: "Delivery unknown", rejected: "Not created" }[item.status];
  return html`<article class="sent-item" data-status=${item.status} data-id=${item.id} aria-label=${`${item.title}, ${said}`}>
    <div class="sent-bubble">
      <p class="sent-title" dir="auto">${item.title}</p>
      ${item.description && html`<p class="sent-desc" dir="auto">${item.description}</p>`}
      <p class="sent-meta">
        <span>→ ${item.column}</span>
        ${item.tags.map((t) => html`<span class=${`sent-tag tint-${t.tint}`} key=${`${t.group}:${t.name}`}><span class="tag-dot" aria-hidden="true"></span>${t.name}</span>`)}
        <span>${clock(item.at)}</span>
        ${tick && html`<span class="sent-tick"><${Tick} state=${tick} /><span class="sr-only">${said}</span></span>`}
      </p>
    </div>
    ${item.status === "landed" &&
    (onBoard
      ? html`<button type="button" class="sent-sys" onClick=${() => store.openSent(item.id)}>
          <${Tick} state="two" /><span>Landed as ${item.key} in ${item.column}</span><span class="sent-open">Open</span></button>`
      : html`<p class="sent-sys"><${Tick} state="two" /><span>Landed as ${item.key} in ${item.column}</span></p>`)}
    ${(item.status === "unknown" || item.status === "rejected") && html`<div class="sent-problem" role="group" aria-label=${said}>
      <p>${item.status === "unknown"
        ? "Delivery unknown. The terminal may have created it: check the board before you send it again."
        : `Not created: ${item.message || "the terminal refused it."}`}</p>
      <div class="sent-actions">
        <button type="button" class="btn btn-quiet" onClick=${() => store.discardSent(item.id)}>Discard</button>
        <button type="button" class="btn" onClick=${() => store.editSent(item.id)}>Edit and send again</button>
      </div>
    </div>`}
  </article>`;
}

// Oldest first, the newest by the bar, the way a conversation reads.
export function SentList({ store, board }) {
  const items = store.sent.forBoard(store.active?.pin.board);
  if (!items.length)
    return html`<div class="sent-empty">
      <${Shin} size="medium" scale=${4} />
      <h3>Nothing sent yet</h3>
      <p>Tickets you write here land on your board, quietly: no agent starts until you start one at your terminal.</p>
    </div>`;
  let day;
  return html`<div class="sent-feed">
    <p class="sent-legend"><span><${Tick} state="clock" />Sending</span><span><${Tick} state="two" />On your board</span></p>
    ${items.map((item) => {
      const label = dayOf(item.at);
      const heading = label !== day && html`<h3 class="sent-day" key=${`day-${label}`}>${label}</h3>`;
      day = label;
      return [heading, html`<${SentItem} key=${item.id} store=${store} board=${board} item=${item} />`];
    })}
  </div>`;
}

export function ColumnTabs({ store, board }) {
  return html`<div class="column-tabs" role="group" aria-label="Column">${board.columns.map((column) => {
    const count = board.visible().filter((t) => t.column === column).length;
    return html`<button type="button" class="column-tab" data-column=${column} aria-pressed=${String(column === board.column)}
      onClick=${() => store.setColumn(column)}><span>${column}</span><span class="group-count">${count}</span></button>`;
  })}</div>`;
}
