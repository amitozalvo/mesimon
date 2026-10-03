// One ticket: its agent, the dialog waiting on you, the periodic output and
// the composer. Every element keeps its place in the DOM even when empty, so
// revocation visibly clears protected content rather than removing it.
import { html, useLayoutEffect, useRef } from "./html.js";
import { Icon, Tick } from "./icons.js";
import { Shin } from "./shin.js";
import { Attention } from "./dialogs.js";
import { StartButton, StartReceipt, Tags, stateAge } from "./lists.js";
import { NotesCard, NoteReader } from "./notepad.js";
import { receiptTick } from "./sessions.js";
import { queueWords, sendRefused, waitsOnYou } from "./queue.js";
import { Chat } from "./transcript.js";

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

// The conversation or the pane's screen (T-626), one toggle between them. A
// parked agent's conversation is still its file; its screen is gone.
function OutputView({ store, ticket }) {
  const chat = store.chatShown;
  const asleep = ticket?.agent?.state === "sleeping";
  return html`<fieldset id="output-view" class="segmented segmented-small">
    <legend class="sr-only">Show</legend>
    <label class=${chat ? "on" : ""}><input type="radio" name="output-view" value="chat"
      checked=${chat} onChange=${() => store.setOutputView("chat")} /><span>Chat</span></label>
    <label class=${`${chat ? "" : "on"}${asleep ? " off" : ""}`}><input type="radio" name="output-view" value="raw"
      checked=${!chat} disabled=${asleep} onChange=${() => store.setOutputView("raw")} /><span>Raw</span></label>
  </fieldset>`;
}

function Output({ store, ticket, entry, live }) {
  const chat = store.chatShown;
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
  const at = chat ? entry?.chatAt : entry?.receivedAt;
  const received = at ? `Last received ${new Date(at).toLocaleTimeString()}` : "Nothing received yet";
  const following = chat ? entry?.chatFollowing !== false : entry?.following;
  const unread = chat ? entry?.chatUnread : entry?.unread;
  // The screen (T-506): the lines at the pane's own width, the type sized by
  // CSS so that width fills the panel, and a line the capture joined wrapped
  // back where the pane had it. Where the pane is wider than the panel can
  // show legibly, the lines reflow at the panel's width and the rules stay
  // one row each (`.screen-lines`).
  return html`<section class="output" aria-label="Output" hidden=${!ticket?.agent || (ticket.agent.state === "sleeping" && !chat)}>
    <div class="output-head">
      <h3 class="label">${chat ? "Conversation" : "Output"}</h3>
      ${store.chatCapable && html`<${OutputView} store=${store} ticket=${ticket} />`}
      <p id="freshness">${live ? html`<span class="dot" aria-hidden="true"></span>` : null}${received}${live ? "" : " · Stale / offline"}</p>
    </div>
    <div class="output-body">
      ${chat
        ? html`<${Chat} store=${store} entry=${entry} doing=${ticket?.agent?.doing} />`
        : html`<pre id="preview" ref=${ref} class="screen" style=${{ "--cols": screenCols(entry) }} aria-label="Agent output" tabindex="0"
        onScroll=${(e) => store.outputScrolled(e.currentTarget)}><span class="screen-lines">${screenRows(text)}</span></pre>`}
      <button id="latest" type="button" class="latest" hidden=${!entry || following}
        onClick=${() => store.latest()}><${Icon} name="down" size=${16} /><span>${unread ? `New ${chat ? "words" : "preview"} · Jump to latest` : "Jump to latest"}</span></button>
    </div>
  </section>`;
}

function Composer({ store, ticket, entry, live }) {
  // A parked agent has no pane to type at: the sheet's wake is its road.
  const agent = ticket?.agent?.state === "sleeping" ? undefined : ticket?.agent;
  const acting = entry?.receipt?.waiting && entry.receipt.status !== "queued";
  const queueOff = !live || !agent?.promptable || ticket?.queued == null || !!acting;
  // At a dialog a steer would be its answer (T-568): Steer is off, and the
  // words queue, until the agent no longer waits on you.
  const steerOff = waitsOnYou(agent);
  const mode = steerOff ? "queue" : entry?.mode || "queue";
  const sendOff = !live || !agent?.promptable || !!entry?.review || !!entry?.receipt?.waiting ||
    !!entry?.answer?.waiting || !entry?.draft.trim();
  const tick = receiptTick(entry?.latest?.status);
  return html`<footer class="composer-area">
    <section id="queued-row" class="bubble-row" aria-label="Queued prompt" hidden=${ticket?.queued == null}>
      <div class="bubble">
        <pre id="queued-text" dir="auto">${ticket?.queued || ""}</pre>
        <p class="bubble-meta"><${Icon} name="hourglass" size=${13} /><span id="queued-meta">${queueWords(ticket)}</span><${Tick} state="one" /></p>
      </div>
      <div class="bubble-actions">
        <button id="take-back" type="button" class="btn btn-quiet" disabled=${queueOff} onClick=${() => store.queueAction("take_back")}>Take back</button>
        <button id="send-now" type="button" class="btn" disabled=${queueOff} hidden=${sendRefused(ticket)}
          onClick=${() => store.queueAction("send_now")}>Send now</button>
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
            <label class=${`${mode === "steer" ? "on" : ""}${steerOff ? " off" : ""}`}><input type="radio" name="prompt-mode" value="steer"
              checked=${mode === "steer"} disabled=${steerOff} onChange=${() => store.setDelivery("steer")} /><${Icon} name="zap" size=${14} /><span>Steer</span></label>
          </fieldset>
          <p class="mode-help">${mode === "steer" ? "Goes in now, mid-turn." : "Waits for the turn to end."}</p>
          <button id="send" type="submit" class="send" disabled=${sendOff}
            aria-label=${mode === "steer" ? "Send prompt" : "Queue prompt"}><${Icon} name="up" size=${20} width=${2.2} /></button>
        </div>
        <p id="steer-why" class="steer-why" hidden=${!steerOff}>Steer is off while the agent waits on you · answer it first. Queue still works.</p>
      </div>
      <p id="delivery" role="status" hidden=${!tick && !entry?.delivery}>${tick && html`<${Tick} state=${tick} />`}<span>${entry?.delivery || ""}</span></p>
    </form>
  </footer>`;
}

// The heading, and the rename it opens (T-530): a press on the title
// writes over it, Enter saves and Escape puts it back.
function Title({ store, ticket }) {
  const ask = ticket && store.renaming?.ticket === ticket.id ? store.renaming : undefined;
  const field = useRef();
  // The caret starts at the end: a rename is most often a tweak.
  useLayoutEffect(() => {
    const node = field.current;
    if (node) node.setSelectionRange(node.value.length, node.value.length);
  }, [ask?.ticket]);
  if (ask)
    return html`<form id="rename-form" class="rename" onSubmit=${(e) => {
      e.preventDefault();
      store.confirmRename();
    }}>
      <textarea id="rename-title" ref=${field} aria-label=${`Title of ${ticket.key}`} rows="2" dir="auto" maxlength="2048"
        enterkeyhint="done" value=${ask.text} onInput=${(e) => store.setRename(e.currentTarget.value)}
        onKeyDown=${(e) => {
          if (e.key === "Enter" && !e.isComposing) {
            e.preventDefault();
            e.currentTarget.form.requestSubmit();
          } else if (e.key === "Escape") {
            e.preventDefault();
            store.cancelRename();
          }
        }}></textarea>
      <div class="rename-actions">
        <button id="rename-cancel" type="button" class="btn btn-quiet" onClick=${() => store.cancelRename()}>Cancel</button>
        <button id="rename-save" type="submit" class="btn btn-pri" disabled=${!ask.text.trim()}>Save</button>
      </div>
    </form>`;
  const renames = !!ticket && store.canEdit("rename") && !store.editWaiting(ticket, "rename");
  return html`<h2 id="selection" tabindex="-1" dir="auto">${!ticket
    ? "Select a ticket"
    : renames
      ? html`<button id="rename" type="button" class="title-button" aria-describedby="rename-hint"
          onClick=${() => store.startRename()}>${ticket.title}<${Icon} name="pencil" size=${16} cls="title-pencil" /></button>`
      : ticket.title}</h2>
    ${renames && html`<span id="rename-hint" class="sr-only">Rename</span>`}`;
}

// The ticket's tags and column (T-510), and with a host that takes the
// card edits (T-530) one button that opens the sheet moving and tagging it.
function TicketLine({ store, ticket }) {
  const moves = store.canEdit("move");
  const tags = store.canEdit("tag") && store.board.allowedTags.length > 0;
  const add = tags && !ticket.tags?.length;
  const chips = html`<${Tags} ticket=${ticket} />${add && html`<span class="chip chip-quiet add-tag"><${Icon} name="plus" size=${12} width=${2.4} /><span>Tag</span></span>`}<span class="chip chip-column">${ticket.column}${moves && html`<${Icon} name="chevronDown" size=${12} width=${2.4} />`}</span>`;
  if (!moves && !tags) return html`<div class="chips">${chips}</div>`;
  return html`<button id="card-line" type="button" class="chips chips-edit" aria-haspopup="dialog" aria-describedby="card-line-hint"
      onClick=${() => store.openCardSheet(ticket.id)}>${chips}</button>
    <span id="card-line-hint" class="sr-only">${moves && tags ? "Move or tag this ticket" : moves ? "Move this ticket" : "Tag this ticket"}</span>`;
}

// What the ticket page says over an empty or a parked seat (T-498, T-510).
function seatWords(store, agent) {
  const what = agent ? `${agent.provider} is asleep on this ticket.` : "No agent on this ticket.";
  const it = agent ? "it" : "one";
  if (!store.startsAgents || (store.live && !store.canStart))
    return `${what} ${agent ? "Wake" : "Start"} ${it} at your terminal.`;
  return store.live ? what : `${what} ${agent ? "Waking" : "Starting"} ${it} needs your terminal back.`;
}

export function Detail({ store, bp }) {
  const ticket = store.board?.current;
  const entry = store.entry;
  const live = store.live;
  const agent = ticket?.agent;
  const asleep = agent?.state === "sleeping";
  const light = agent?.state === "needs attention" ? "attn" : ["starting", "working"].includes(agent?.state) ? "calm" : "dim";
  const since = store.board && stateAge(store.board, agent);
  const start = store.startOf(ticket);
  // The seat a start reaches (T-510): empty, or a parked agent it wakes.
  const seatOpen = !!ticket && (!agent || asleep);
  // A note open for reading takes the page (T-532); Back returns to it.
  if (ticket && store.reading?.ticket === ticket.id)
    return html`<article id="detail" aria-labelledby="note-title"><${NoteReader} store=${store} ticket=${ticket} /></article>`;
  // The head (T-510): the shin, the title, then one line with the tags and
  // the column at its left and the key at its right, as the TUI's page. A
  // start that took sits beside the column (T-547); one still on its way, or
  // refused, stays under the button it came from.
  const started = start?.status === "started";
  return html`<article id="detail" aria-labelledby="selection">
    <header class="detail-head">
      <button id="back" type="button" class="icon-btn" aria-label="Back" onClick=${() => store.back()}><${Icon} name="back" size=${22} /></button>
      <button id="close-detail" type="button" class="icon-btn" aria-label="Close" onClick=${() => store.back()}><${Icon} name="x" size=${20} /></button>
      <div class="detail-shin" hidden=${!ticket}>
        <${Shin} scale=${3} light=${light} mood=${agent && !asleep ? "awake" : "asleep"} />
        ${agent && html`<span id="agent-word" class="sr-only">${agent.provider} · ${agent.state}${since ? ` · ${since}` : ""}</span>`}
      </div>
      <div class="detail-title">
        <${Title} store=${store} ticket=${ticket} />
        ${ticket && html`<div class="ticket-line">
          <${TicketLine} store=${store} ticket=${ticket} />
          ${started && html`<${StartReceipt} item=${start} agent=${agent} />`}
          <span class="selection-key">${ticket.key}</span>
        </div>`}
      </div>
    </header>
    <div class="detail-scroll">
      ${seatOpen && html`<div class="agent-line" role="group" aria-label="Agent">
        <p id="agent-state">${seatWords(store, agent)}</p>
        ${store.startsAgents && html`<${StartButton} store=${store} ticket=${ticket} />`}
      </div>`}
      ${ticket && !started && html`<${StartReceipt} item=${start} agent=${agent} />`}
      <${Attention} store=${store} ticket=${ticket} entry=${entry} live=${live} />
      <${NotesCard} store=${store} ticket=${ticket} />
      <${Output} store=${store} ticket=${ticket} entry=${entry} live=${live} />
      ${!ticket && html`<div class="detail-empty"><${Shin} size="medium" scale=${4} /><p>Pick a ticket to read its agent’s output and send it a prompt.</p></div>`}
    </div>
    <${Composer} store=${store} ticket=${ticket} entry=${entry} live=${live} />
  </article>`;
}
