// One ticket: its agent, the dialog waiting on you, the periodic output and
// the composer. Every element keeps its place in the DOM even when empty, so
// revocation visibly clears protected content rather than removing it.
import { html, useLayoutEffect, useRef } from "./html.js";
import { Icon, Tick } from "./icons.js";
import { Shin } from "./shin.js";
import { Attention } from "./dialogs.js";
import { Headline, StartButton, StartReceipt, Tags, stateAge } from "./lists.js";

const receiptTick = (status) =>
  status === "awaiting_delivery"
    ? "clock"
    : status === "queued"
      ? "one"
      : ["submitted", "decision_sent", "input_sent"].includes(status)
        ? "two"
        : null;

// The pane's width in cells, for drawing its lines as the screen they came
// from (T-506). An older host names none: the longest line stands in, which
// is the pane's width whenever the pane laid its own lines out.
const screenCols = (entry) => {
  if (entry?.cols) return entry.cols;
  const longest = Math.max(0, ...(entry?.displayed || "").split("\n").map((line) => line.length));
  return Math.min(200, Math.max(80, longest));
};
// A line that is only a rule (Claude Code's `────` between turns, a markdown
// `---`) is drawn as one, however wide the pane was: it is the line that
// wrapped into three when the browser reflowed the pane.
const RULE = /^\s*[─━═╌╍┄┅┈┉\-_=~]{8,}\s*$/u;
const screenRows = (text) => {
  const lines = text.split("\n");
  return lines.map((line, i) =>
    RULE.test(line) ? html`<span class="screen-rule" key=${i}></span>` : i < lines.length - 1 ? `${line}\n` : line,
  );
};

function Output({ store, ticket, entry, live }) {
  const ref = useRef();
  const shown = useRef();
  useLayoutEffect(() => {
    const node = ref.current;
    if (!node || !node.getClientRects().length) return;
    const key = entry?.key;
    if (shown.current !== key || store.outputKey !== key) {
      shown.current = store.outputKey = key;
      node.scrollTop = entry?.following ? node.scrollHeight : entry?.scroll || 0;
    } else if (entry?.following) node.scrollTop = node.scrollHeight;
  });
  const text =
    entry?.displayed ||
    (entry?.receivedAt
      ? "No output in the latest preview."
      : ticket?.agent
        ? "Waiting for the first preview…"
        : ticket
          ? "No agent output."
          : "");
  const received = entry?.receivedAt
    ? `Last received ${new Date(entry.receivedAt).toLocaleTimeString()}`
    : "Nothing received yet";
  // The screen (T-506): the lines at the pane's own width, the type sized by
  // CSS so that width fills the panel, and a line the capture joined wrapped
  // back where the pane had it. Where the pane is wider than the panel can
  // show legibly, the lines reflow at the panel's width and the rules stay
  // one row each (`.screen-lines`).
  return html`<section class="output" aria-label="Output" hidden=${!ticket?.agent}>
    <div class="output-head">
      <h3 class="label">Output</h3>
      <p id="freshness">${live ? html`<span class="dot" aria-hidden="true"></span>` : null}${received}${live ? "" : " · Stale / offline"}</p>
    </div>
    <div class="output-body">
      <pre id="preview" ref=${ref} class="screen" style=${{ "--cols": screenCols(entry) }} aria-label="Agent output" tabindex="0"
        onScroll=${(e) => store.outputScrolled(e.currentTarget)}><span class="screen-lines">${screenRows(text)}</span></pre>
      <button id="latest" type="button" class="latest" hidden=${!entry || entry.following}
        onClick=${() => store.latest()}><${Icon} name="down" size=${16} /><span>${entry?.unread ? "New preview · Jump to latest" : "Jump to latest"}</span></button>
    </div>
  </section>`;
}

function Composer({ store, ticket, entry, live }) {
  const agent = ticket?.agent;
  const acting = entry?.receipt?.waiting && entry.receipt.status !== "queued";
  const queueOff = !live || !agent?.promptable || ticket?.queued == null || !!acting;
  const mode = entry?.mode || "queue";
  const sendOff =
    !live || !agent?.promptable || !!entry?.review || !!entry?.receipt?.waiting || !entry?.draft.trim();
  const tick = receiptTick(entry?.receipt?.status);
  return html`<footer class="composer-area">
    <section id="queued-row" class="bubble-row" aria-label="Queued prompt" hidden=${ticket?.queued == null}>
      <div class="bubble">
        <pre id="queued-text" dir="auto">${ticket?.queued || ""}</pre>
        <p class="bubble-meta"><${Icon} name="hourglass" size=${13} /><span>Queued · waits for idle</span><${Tick} state="one" /></p>
      </div>
      <div class="bubble-actions">
        <button id="take-back" type="button" class="btn btn-quiet" disabled=${queueOff} onClick=${() => store.queueAction("take_back")}>Take back</button>
        <button id="send-now" type="button" class="btn" disabled=${queueOff} onClick=${() => store.queueAction("send_now")}>Send now</button>
      </div>
    </section>
    <section id="returned-row" class="returned" aria-label="Retained prompt text" hidden=${!entry?.returned}>
      <p class="returned-note">Retained text · your current draft is unchanged</p>
      <pre id="returned-text" dir="auto">${entry?.returned?.text || ""}</pre>
      <button id="swap-returned" type="button" class="btn" onClick=${() => store.swapReturned()}>Swap with draft</button>
    </section>
    <form id="prompt-form" onSubmit=${(e) => { e.preventDefault(); store.submitPrompt(); }}>
      <p id="session-warning" hidden=${!entry?.review || !agent}>
        <span>Session changed. Review the retained draft before sending to this session.</span>
        <button id="review-draft" type="button" class="btn" onClick=${() => store.reviewDraft()}>Use draft for this session</button>
      </p>
      <p id="target" class="sr-only">${agent
        ? `Message ${agent.provider} on ${ticket.key} · session ${agent.session.slice(0, 8)}`
        : "No agent selected"}</p>
      <div class="composer">
        <textarea id="prompt" aria-label="Prompt" aria-describedby="target" rows="2" dir="auto"
          placeholder=${agent ? `Message ${agent.provider}…` : "No agent to message"} required disabled=${!agent}
          value=${entry?.draft || ""} onInput=${(e) => store.setDraft(e.currentTarget.value)}
          onKeyDown=${(e) => {
            if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && !e.isComposing) {
              e.preventDefault();
              e.currentTarget.form.requestSubmit();
            }
          }}></textarea>
        <div class="composer-row">
          <fieldset id="prompt-mode" class="segmented" disabled=${!agent}>
            <legend class="sr-only">Delivery</legend>
            <label class=${mode === "queue" ? "on" : ""}><input type="radio" name="prompt-mode" value="queue"
              checked=${mode === "queue"} onChange=${() => store.setDelivery("queue")} /><${Icon} name="hourglass" size=${14} /><span>Queue</span></label>
            <label class=${mode === "steer" ? "on" : ""}><input type="radio" name="prompt-mode" value="steer"
              checked=${mode === "steer"} onChange=${() => store.setDelivery("steer")} /><${Icon} name="zap" size=${14} /><span>Steer</span></label>
          </fieldset>
          <p class="mode-help">${mode === "steer" ? "Goes in now, mid-turn." : "Waits for the turn to end."}</p>
          <button id="send" type="submit" class="send" disabled=${sendOff}
            aria-label=${mode === "steer" ? "Send prompt" : "Queue prompt"}><${Icon} name="up" size=${20} width=${2.2} /></button>
        </div>
      </div>
      <p id="delivery" role="status">${tick && html`<${Tick} state=${tick} />`}<span>${entry?.delivery || ""}</span></p>
    </form>
  </footer>`;
}

// What the ticket page says when no agent is on it (T-498).
function noAgent(store) {
  if (!store.startsAgents || (store.live && !store.canStart))
    return "No agent on this ticket. Start one at your terminal.";
  return store.live ? "No agent on this ticket." : "No agent on this ticket. Starting one needs your terminal back.";
}

export function Detail({ store, bp }) {
  const ticket = store.board?.current;
  const entry = store.entry;
  const live = store.live;
  const agent = ticket?.agent;
  const light = agent?.state === "needs attention" ? "attn" : ["starting", "working"].includes(agent?.state) ? "calm" : "dim";
  const since = store.board && stateAge(store.board, agent);
  const start = store.startOf(ticket);
  return html`<article id="detail" aria-labelledby="selection">
    <header class="detail-head">
      <button id="back" type="button" class="icon-btn" aria-label="Back" onClick=${() => store.back()}><${Icon} name="back" size=${22} /></button>
      <button id="close-detail" type="button" class="icon-btn" aria-label="Close" onClick=${() => store.back()}><${Icon} name="x" size=${20} /></button>
      <div class="detail-title">
        <h2 id="selection" tabindex="-1" dir="auto">${ticket
          ? html`<span class="selection-title">${ticket.title}</span>${" "}<span class="selection-key">${ticket.key}</span>`
          : "Select a ticket"}</h2>
        <div class="chips">
          ${ticket && html`<span class="chip">${ticket.column}</span>`}
          ${agent && html`<span class="chip">${agent.provider}</span>`}
          ${ticket && html`<${Tags} ticket=${ticket} />`}
        </div>
      </div>
    </header>
    <div class="detail-scroll">
      <div class=${`agent-card${agent ? "" : " agent-none"}`} hidden=${!ticket}>
        <${Shin} scale=${3} light=${light} mood=${agent ? "awake" : "asleep"} />
        <div class="agent-words">
          <p id="agent-state">${agent
            ? `${agent.provider} · ${agent.state}${since ? ` · ${since}` : ""} · ${ticket.column}`
            : ticket
              ? noAgent(store)
              : "Choose an agent, or open Board to see all tickets."}</p>
          <${Headline} agent=${agent} />
          <${StartReceipt} item=${start} agent=${agent} />
        </div>
        ${ticket && !agent && store.startsAgents && html`<${StartButton} store=${store} ticket=${ticket} />`}
        ${agent?.state === "needs attention" && html`<span class="mark mark-attn pulse" aria-hidden="true"></span>`}
        ${["starting", "working"].includes(agent?.state) && html`<${Icon} name="spinner" size=${18} width=${2.4} cls="spin" />`}
      </div>
      <${Attention} store=${store} ticket=${ticket} entry=${entry} live=${live} />
      <${Output} store=${store} ticket=${ticket} entry=${entry} live=${live} />
      ${!ticket && html`<div class="detail-empty"><${Shin} size="medium" scale=${4} /><p>Pick a ticket to read its agent’s output and send it a prompt.</p></div>`}
    </div>
    <${Composer} store=${store} ticket=${ticket} entry=${entry} live=${live} />
  </article>`;
}
