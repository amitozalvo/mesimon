// The work list: Now (agents grouped by the host's state word, a stopped
// one only for an hour, T-560; then the tickets created within the hour,
// T-668), Board (every ticket, by column) and Sent
// (the tickets this browser filed). A ticket is the same card in Now and on
// the Board (T-533); a card is a button, and the pressed one is selected.
import { html, useLayoutEffect, useRef, useState } from "./html.js";
import { Icon, Tick } from "./icons.js";
import { Shin } from "./shin.js";
import { answerable, requestSummary } from "./dialogs.js";
import { startWaiting } from "./starts.js";
import { answerBusy, receiptTick } from "./sessions.js";
import { RECENT_MS } from "./board.js";

const clock = (at) => new Date(at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
export const lastSeen = (board) => (board?.receivedAt ? clock(board.receivedAt) : "");
// How long ago `at` was, by the host's clock, in the board's short words. A
// remembered board says none: it is not now.
export function ageWords(board, at) {
  if (board.cached || !Number.isFinite(at)) return "";
  const seconds = Math.max(0, (Date.now() - at) / 1000);
  return seconds < 60
    ? "now"
    : seconds < 3600
      ? `${Math.floor(seconds / 60)}m`
      : seconds < 86400
        ? `${Math.floor(seconds / 3600)}h`
        : `${Math.floor(seconds / 86400)}d`;
}
// How long an agent has been in its state (T-497).
export const stateAge = (board, agent) => ageWords(board, agent?.since);

// What the crown last did to a ticket (T-623), while it is news: the host
// keeps it an hour, and so does a page left open. A remembered board has
// none.
export function crownTouch(board, ticket) {
  const touch = ticket?.crowned;
  if (board.cached || !touch?.action || !(Date.now() - touch.at < RECENT_MS)) return undefined;
  return touch;
}

// The word for what the crown last did to a ticket (T-623), and how long
// ago, where the TUI's card says it.
function CrownWord({ board, ticket }) {
  const touch = crownTouch(board, ticket);
  if (!touch) return null;
  const age = ageWords(board, touch.at);
  return html`<span class="crown-word"><${Icon} name="crown" size=${13} width=${2.2} /><span>${touch.action}${age && ` · ${age}`}</span></span>`;
}

// The ticket that wears the crown has it before its title, and its title in
// the crown's ink (T-623), as the TUI's card draws its holder.
export function CrownMark({ ticket, size }) {
  if (!ticket?.crown) return null;
  return html`<${Icon} name="crown" size=${size} width=${2.2} cls="crown-mark" /><span class="sr-only">Wears the crown: </span>`;
}

// A worktree's state as the TUI's card marks it (T-642): one character
// beside the branch glyph, and the register it is drawn in. A shared
// checkout has no mark.
const WT_MARK = {
  planned: ["·", "dormant", "worktree, cut when an agent starts"],
  provisioning: ["…", "quiet", "worktree being cut"],
  error: ["×", "err", "worktree failed"],
  evicted: ["–", "quiet", "worktree removed"],
  conflict: ["!", "err", "branch shared with another worktree"],
  merged: ["✓", "quiet", "worktree merged"],
  behind: ["↓", "ready", "main moved, rebase first"],
  ahead: ["↑", "ready", "commits to merge"],
  clean: ["", "quiet", "worktree"],
};
// With `count` (the ticket page's line, T-701) the glyph to merge carries
// how many commits, and the title says what the page's branch row said.
export function WorktreeMark({ ticket, count = false }) {
  const ws = ticket.workspace;
  // A worktree chosen and not cut yet is planned, whether or not the host
  // says so.
  const mark = WT_MARK[ws?.state || (ws?.kind === "worktree" && !ws.branch ? "planned" : "")];
  if (!mark) return null;
  const glyph = count && ws.state === "ahead" && ws.ahead ? `${mark[0]}${ws.ahead}` : mark[0];
  const words = count ? [mark[2], ws.branch, worktreeWords(ws)].filter(Boolean).join(" · ") : mark[2];
  return html`<span class=${`wt-mark wt-${mark[1]}`} title=${words}><${Icon} name="branch" size=${13} width=${2.2} />${glyph && html`<span aria-hidden="true">${glyph}</span>`}<span class="sr-only">${words}</span></span>`;
}

// The ticket page's words for its worktree (T-642), the TUI's branch row:
// what it is waiting on or what is wrong, the progress or init detail.
export function worktreeWords(ws) {
  const words = {
    provisioning: "being cut",
    error: "failed · see your terminal",
    evicted: "removed",
    conflict: "branch shared!",
    merged: "merged",
    behind: "main moved",
    ahead: `${ws.ahead || ""} to merge`.trim(),
  }[ws.state];
  return [words, ws.detail].filter(Boolean).join(" · ");
}

// The ticket's tags as the TUI paints them (T-506): the name on a ground of
// its tint, in the page's ground ink.
export function Tags({ ticket }) {
  const tags = ticket.tags || [];
  if (!tags.length) return null;
  return html`<span class="tags">${tags.map((t) => html`<span class=${`tag tint-${t.tint}`} key=${`${t.group}:${t.name}`}>${t.name}</span>`)}</span>`;
}

// What the agent is on (a tool step, mono) or last said, one line (T-497).
// Live only: a remembered board never has it.
export function Headline({ agent }) {
  if (agent?.doing) return html`<span class="headline headline-step">› ${agent.doing}</span>`;
  if (agent?.said) return html`<span class="headline" dir="auto">${agent.said}</span>`;
  return null;
}

// Starting an agent from here (T-498): a clock while the host starts it,
// two ticks once its session runs.
const startTick = { sending: "clock", provisioning: "clock", starting: "clock", started: "two" };
const startWords = (item) =>
  ({
    sending: "Asking your terminal…",
    provisioning: "Setting up the ticket’s worktree…",
    starting: "Starting the agent…",
    started: `Started from this browser · ${clock(item.at)}`,
    rejected: `Not started: ${item.message || "the terminal refused it."}`,
    unknown: "Start unknown. Check the board before you start it again.",
  })[item.status];

// A start's receipt. Two ticks are news only while its agent is there.
export function StartReceipt({ item, agent }) {
  if (!item || (item.status === "started" && !agent)) return null;
  const tick = startTick[item.status];
  return html`<p class="start-receipt" role="status" data-status=${item.status}>${tick && html`<${Tick} state=${tick} />`}<span>${startWords(item)}</span></p>`;
}

// Enabled while the terminal is live and nothing is starting here already.
// On a parked agent it is the wake (T-510). The ticket page alone holds
// it: a Board card opens the ticket, and the words are asked for there.
export function StartButton({ store, ticket }) {
  const waiting = startWaiting(store.startOf(ticket));
  const asleep = ticket.agent?.state === "sleeping";
  return html`<button type="button" class="btn start-agent" data-start=${ticket.id}
    aria-label=${`${asleep ? "Wake" : "Start"} agent on ${ticket.key}`} disabled=${!store.canStart || waiting}
    onClick=${() => store.openStart(ticket.id)}><${Icon} name="play" size=${16} /><span>${asleep ? "Wake agent" : "Start agent"}</span></button>`;
}

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

// What a needs-you agent needs, in the words a person would use; any other
// agent's own state word.
function stateWord(agent) {
  if (agent.state !== "needs attention") return agent.state;
  return agent.permission
    ? "needs approval"
    : agent.dialog?.kind === "plan"
      ? "has a plan"
      : agent.dialog
        ? "has a question"
        : "needs you";
}

// A ticket, drawn the same wherever it is listed (T-533): the title; its
// tags, with the key at the right (and the column, where the list around it
// is not that column); the agent's state and age; and, live, what it is on.
// An archived ticket (T-665) says so where its column would be.
function Face({ store, ticket, board, column, archived }) {
  const agent = ticket.agent;
  const since = stateAge(board, agent);
  return html`<span class=${`ticket-title${ticket.crown ? " crowned" : ""}`} dir="auto"><${CrownMark} ticket=${ticket} size=${15} />${ticket.title}</span>
    <span class="card-line"><${Tags} ticket=${ticket} /><span class="ticket-meta"><${CrownWord} board=${board} ticket=${ticket} />${(column || archived) && html`<span class=${archived ? "archived-word" : undefined}>${archived ? "Archived" : ticket.column}</span><span aria-hidden="true">·</span>`}<${WorktreeMark} ticket=${ticket} /><span class="ticket-key">${ticket.key}</span><${FromHere} store=${store} ticket=${ticket} /></span></span>
    ${agent && html`<span class=${`card-agent${agent.state === "needs attention" ? " attn-ink" : ""}`}><${StateMark} ticket=${ticket} /><span>${agent.provider} · ${stateWord(agent)}${since && ` · ${since}`}</span></span>`}
    <${Headline} agent=${agent} />`;
}

// On a desktop's Board a card drags (T-530): `drag` is its column's drop
// state, absent where nothing drags.
function Card({ store, ticket, board, column, drag }) {
  const needs = ticket.agent?.state === "needs attention";
  const cls = `ticket card${needs ? " card-attn" : ""}${drag?.before === ticket.id ? " drop-before" : ""}`;
  return html`<button type="button" class=${cls} data-id=${ticket.id}
    aria-pressed=${String(ticket.id === board.selected)} onClick=${() => store.select(ticket.id)}
    draggable=${drag ? "true" : undefined}
    onDragStart=${drag && ((e) => {
      dragged = ticket.id;
      e.dataTransfer.effectAllowed = "move";
      e.dataTransfer.setData("text/plain", ticket.key);
    })}
    onDragEnd=${drag && (() => {
      dragged = undefined;
      drag.end();
    })}>
    <${Face} store=${store} ticket=${ticket} board=${board} column=${column} />
  </button>`;
}

// A ticket written here that has not landed yet (T-497), dashed; it opens Sent.
function Ghost({ store, item, column }) {
  return html`<button type="button" class="ticket card ghost" data-id=${item.id} onClick=${() => store.setMode("sent")}>
    <span class="ticket-meta"><span>New</span><${Tick} state=${tickOf[item.status]} /><span>${item.status === "relay" ? "At the relay" : "In this browser"} · ${clock(item.at)}${column ? ` · → ${item.column}` : ""}</span></span>
    <span class="ticket-title" dir="auto">${item.title}</span>
  </button>`;
}

// A needs-you card is a card that answers what it can in place; the rest
// opens the ticket.
function NeedCard({ store, ticket, board, live }) {
  const pressed = ticket.id === board.selected;
  const { permission, dialog, session } = ticket.agent;
  const entry = (permission || dialog) ? store.entryFor(ticket) : undefined;
  const off = !live || answerBusy(entry);
  const send = (body) => store.sendInteraction({ ...body, ticket: ticket.id, session }, entry);
  const question = answerable(dialog) ? dialog.questions[0] : undefined;
  const expired = permission && Date.now() >= permission.expires_at;
  const tick = receiptTick(entry?.latest?.status);
  return html`<article class="need">
    <button type="button" class="ticket need-open" data-id=${ticket.id} aria-pressed=${String(pressed)}
      onClick=${() => store.select(ticket.id)}>
      <${Face} store=${store} ticket=${ticket} board=${board} column=${true} />
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
    ${entry?.delivery && html`<p class="need-delivery" role="status">${tick && html`<${Tick} state=${tick} />`}<span>${entry.delivery}</span></p>`}
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

// Tickets written here that have not landed yet (T-497): what waits in this
// browser or at the relay, as the Board's ghosts. One opens Sent.
function Waiting({ store }) {
  const waiting = store.sent.waiting(store.active?.pin.board);
  if (!waiting.length) return null;
  return html`<${Group} label="Waiting to land" count=${waiting.length}>
    <div class="cards">${waiting.map((item) => html`<${Ghost} key=${item.id} store=${store} item=${item} column=${true} />`)}</div>
  </${Group}>`;
}

export function NowList({ store, board, live }) {
  const { needs, working, idle, created } = board.sections();
  const empty = !needs.length && !working.length && !idle.length && !created.length;
  return html`
    <${Asleep} store=${store} />
    <${Waiting} store=${store} />
    ${needs.length > 0 && html`<${Group} label="Needs you" count=${needs.length} attn=${true}>
      <div class="needs">${needs.map((t) => html`<${NeedCard} key=${t.id} store=${store} ticket=${t} board=${board} live=${live} />`)}</div>
    </${Group}>`}
    ${working.length > 0 && html`<${Group} label="Working" count=${working.length}>
      <div class="cards">${working.map((t) => html`<${Card} key=${t.id} store=${store} ticket=${t} board=${board} column=${true} />`)}</div>
    </${Group}>`}
    ${idle.length > 0 && html`<${Group} label="Recently idle" count=${idle.length}>
      <div class="cards">${idle.map((t) => html`<${Card} key=${t.id} store=${store} ticket=${t} board=${board} column=${true} />`)}</div>
    </${Group}>`}
    ${created.length > 0 && html`<${Group} label="Recently created" count=${created.length}>
      <div class="cards">${created.map((t) => html`<${Card} key=${t.id} store=${store} ticket=${t} board=${board} column=${true} />`)}</div>
    </${Group}>`}
    ${empty && html`<p class="empty">${board.search
      ? "No tickets match your search."
      : board.tickets.length
        ? "Nothing in the last hour. Open Board to see every ticket."
        : "This board has no tickets yet."}</p>`}
  `;
}

// The card a drag holds (T-530); one at a time.
let dragged;
// Where a drop over a column lands: before the first card whose middle is
// below the pointer, or at the column's end.
const dropBefore = (column, y) => {
  for (const node of column.querySelectorAll(".ticket.card[data-id]:not(.ghost)")) {
    if (node.dataset.id === dragged) continue;
    const rect = node.getBoundingClientRect();
    if (y < rect.top + rect.height / 2) return node.dataset.id;
  }
  return null;
};

// Phone: one column at a time. Tablet: columns stacked. Desktop: side by
// side, and there a card drags to another column or to another place in its
// own (T-530). Everywhere, the line on the ticket's page moves it.
export function BoardList({ store, board, bp }) {
  const [drop, setDrop] = useState(null);
  const drags = bp === "desktop" && store.canEdit("move");
  const tickets = board.visible();
  const columns = bp === "phone" ? board.columns.filter((c) => c === board.column) : board.columns;
  if (!tickets.length && board.search) return html`<p class="empty">No tickets match your search.</p>`;
  if (!board.tickets.length && !store.sent.waiting(store.active?.pin.board).length)
    return html`<p class="empty">This board has no tickets yet.</p>`;
  return html`<div class=${bp === "desktop" ? "kanban" : bp === "phone" ? "stack single" : "stack"}>${columns.map((column) => {
    const group = tickets.filter((t) => t.column === column);
    const ghosts = store.sent.waiting(store.active?.pin.board).filter((i) => i.column === column);
    // What the column is for shows on the title's hover (T-506), so every
    // column's first card starts at the same height.
    const about = board.columnDescriptions[column];
    const here = drop?.column === column ? drop : undefined;
    const drag = drags ? { before: here?.before, end: () => setDrop(null) } : undefined;
    const over = drags
      ? {
          onDragOver: (e) => {
            if (!dragged) return;
            e.preventDefault();
            e.dataTransfer.dropEffect = "move";
            const before = dropBefore(e.currentTarget, e.clientY);
            if (here?.before !== before || !here) setDrop({ column, before });
          },
          onDragLeave: (e) => {
            if (here && !e.currentTarget.contains(e.relatedTarget)) setDrop(null);
          },
          onDrop: (e) => {
            if (!dragged) return;
            e.preventDefault();
            const id = dragged;
            const before = dropBefore(e.currentTarget, e.clientY);
            dragged = undefined;
            setDrop(null);
            store.moveTicket(id, column, before);
          },
        }
      : {};
    return html`<section class=${`column${here ? " drop-target" : ""}`} key=${column} aria-label=${column} ...${over}>
      <h3 class="column-label" title=${about || undefined}><span>${column}</span><span class="group-count">${group.length}</span></h3>
      <div class=${`cards${here && here.before === null ? " drop-end" : ""}`}>
        ${ghosts.map((item) => html`<${Ghost} key=${item.id} store=${store} item=${item} />`)}
        ${group.map((t) => html`<${Card} key=${t.id} store=${store} ticket=${t} board=${board} drag=${drag} />`)}
      </div>
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

const tickOf = { sending: "clock", local: "clock", relay: "one", landed: "two" };
// Picked up at the desk (T-497): the two ticks turn teal.
const tickFor = (item) => (item.status === "landed" && item.picked ? "picked" : tickOf[item.status]);
const pickedLine = (item) =>
  `${item.picked.by === "agent" ? "An agent started on it" : "Opened at your desk"} · ${clock(item.picked.at)}`;
const saidOf = {
  sending: "Sending",
  local: "In this browser",
  relay: "At the relay",
  landed: "On your board",
  unknown: "Delivery unknown",
  rejected: "Not created",
  withdrawn: "Unsent",
};

function SentItem({ store, board, item }) {
  const said = saidOf[item.status];
  if (item.status === "withdrawn")
    return html`<p class="sent-gone" data-id=${item.id}>You unsent “<span dir="auto">${item.title}</span>”</p>`;
  // A landed ticket is the board's own card while the host has it (T-665):
  // its column, tags and agent as they are now, and on the board it opens
  // the ticket. One no longer on the board keeps the words it was sent with.
  const landed = item.status === "landed" && !!item.ticket;
  const where = landed ? board.whereIs(item.ticket) : {};
  const tick = tickFor(item);
  // Still on its way: it can be edited or taken back until the host has it.
  const waiting = item.status === "local" || item.status === "relay";
  const bubble = html`
    <span class="sent-title" dir="auto">${item.title}</span>
    ${item.description && html`<span class="sent-desc" dir="auto">${item.description}</span>`}
    <span class="sent-meta">
      <span>→ ${item.column}</span>
      ${item.tags.map((t) => html`<span class=${`tag tint-${t.tint}`} key=${`${t.group}:${t.name}`}>${t.name}</span>`)}
      <span>${clock(item.at)}</span>
      ${tick && !landed && html`<span class="sent-tick" title=${item.picked ? "Picked up" : said}><${Tick} state=${tick} /><span class="sr-only">${item.picked ? "Picked up" : said}</span></span>`}
    </span>`;
  const sys = (words) => html`<p class="sent-sys"><span class="sent-tick" title=${item.picked ? "Picked up" : said}><${Tick} state=${tick} /><span class="sr-only">${item.picked ? "Picked up" : said}</span></span><span>${words}</span></p>`;
  return html`<article class="sent-item" data-status=${item.status} data-picked=${item.picked?.by} data-id=${item.id}
    data-where=${landed ? (where.archived ? "archived" : where.ticket ? "board" : "gone") : undefined}
    aria-label=${`${item.title}, ${item.picked ? "picked up" : said}`}>
    ${where.archived
      ? html`<div class="ticket card card-archived" data-id=${where.ticket.id}><${Face} store=${store} ticket=${where.ticket} board=${board} archived=${true} /></div>`
      : where.ticket
        ? html`<${Card} store=${store} ticket=${where.ticket} board=${board} column=${true} />`
        : html`<div class="sent-bubble">${bubble}</div>`}
    ${waiting && html`<div class="sent-waiting" role="group" aria-label=${said}>
      <p>${item.status === "local"
        ? store.link === "nonet"
          ? "In this browser. It goes out when you’re back online."
          : "In this browser. It goes out when the relay answers."
        : "Sealed at the relay. It lands when your terminal is back."}</p>
      <div class="sent-actions">
        <button type="button" class="btn btn-quiet" onClick=${() => store.unsendSent(item.id)}>Unsend</button>
        <button type="button" class="btn btn-quiet" onClick=${() => store.editSent(item.id)}>Edit</button>
      </div>
    </div>`}
    ${landed && sys(`Landed as ${item.key} in ${item.column} · ${clock(item.at)}${where.ticket ? "" : " · no longer on the board"}`)}
    ${item.picked && html`<p class="sent-sys sent-picked"><${Tick} state="picked" /><span>${pickedLine(item)}</span></p>`}
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
    ${items.map((item) => {
      const label = dayOf(item.at);
      const heading = label !== day && html`<h3 class="sent-day" key=${`day-${label}`}>${label}</h3>`;
      day = label;
      return [heading, html`<${SentItem} key=${item.id} store=${store} board=${board} item=${item} />`];
    })}
  </div>`;
}

export function ColumnTabs({ store, board }) {
  // A swipe can bring a column whose tab is out of sight: keep it in view.
  const strip = useRef();
  useLayoutEffect(() => {
    const node = strip.current;
    const tab = node?.querySelector('[aria-pressed="true"]');
    if (!tab) return;
    const left = tab.getBoundingClientRect().left - node.getBoundingClientRect().left + node.scrollLeft;
    const pad = 16;
    if (left - pad < node.scrollLeft) node.scrollLeft = left - pad;
    else if (left + tab.offsetWidth + pad > node.scrollLeft + node.clientWidth)
      node.scrollLeft = left + tab.offsetWidth + pad - node.clientWidth;
  }, [board.column]);
  return html`<div class="column-tabs" role="group" aria-label="Column" ref=${strip}>${board.columns.map((column) => {
    const count = board.visible().filter((t) => t.column === column).length;
    return html`<button type="button" class="column-tab" data-column=${column} aria-pressed=${String(column === board.column)}
      title=${board.columnDescriptions[column] || undefined}
      onClick=${() => store.setColumn(column)}><span>${column}</span><span class="group-count">${count}</span></button>`;
  })}</div>`;
}
