// The page: pairing, or the board shell (sidebar, list, detail). Layout is
// CSS; the breakpoint only chooses which list shape to draw.
import { html, useEffect, useLayoutEffect, useRef, useState } from "./html.js";
import { Icon, Tick } from "./icons.js";
import { Shin } from "./shin.js";
import { NowList, BoardList, SentList, ColumnTabs, lastSeen } from "./lists.js";
import { Detail } from "./detail.js";
import { NewTicket, QuickNew } from "./compose.js";

// Subscribe during the commit, not after paint: a fast boot can emit before
// a deferred effect runs, and that change would never reach the page.
function useStore(store) {
  const [, setVersion] = useState(0);
  const seen = store.version;
  useLayoutEffect(() => {
    const unsubscribe = store.subscribe(() => setVersion(store.version));
    if (store.version !== seen) setVersion(store.version);
    return unsubscribe;
  }, [store]);
}

const breakpoint = () =>
  matchMedia("(max-width: 700px)").matches
    ? "phone"
    : matchMedia("(max-width: 1100px)").matches
      ? "tablet"
      : "desktop";

function useBreakpoint() {
  const [bp, setBp] = useState(breakpoint);
  useEffect(() => {
    const update = () => setBp(breakpoint());
    addEventListener("resize", update);
    return () => removeEventListener("resize", update);
  }, []);
  return bp;
}

const lightOf = (store) =>
  !store.live
    ? "dim"
    : store.board?.tickets.some((t) => t.agent?.state === "needs attention")
      ? "attn"
      : "calm";

const linkLabel = {
  live: "Live",
  connecting: "Connecting",
  asleep: "Offline",
  relay: "Relay unreachable",
  nonet: "No connection",
};

function LinkIcon({ link, size = 14 }) {
  if (link === "live") return html`<span class="dot" aria-hidden="true"></span>`;
  if (link === "connecting") return html`<${Icon} name="spinner" size=${size} cls="spin" />`;
  return html`<${Icon} name=${link === "asleep" ? "moon" : "wifiOff"} size=${size} />`;
}

function Pill({ store }) {
  const link = store.link;
  return html`<button type="button" class="pill-button" aria-label=${`Connection: ${linkLabel[link]}`}
    onClick=${() => store.openSheet(true)}>
    <span class="pill" data-link=${link}><${LinkIcon} link=${link} /><span>${linkLabel[link]}</span></span>
  </button>`;
}

function Pairing({ store, hidden }) {
  return html`<section id="onboarding" hidden=${hidden} aria-labelledby="welcome-title">
    <div class="onboarding-inner">
      <${Shin} size="medium" scale=${5} light="calm" />
      <h1 id="welcome-title">Remote Control</h1>
      <p class="onboarding-hero">Your board,<br />in your pocket.</p>
      <p class="onboarding-lede">Pair this browser with mesimon on your Mac. What passes between them is sealed end to end.</p>
      <ol class="steps">
        <li><span class="step">1</span><span>On your Mac, press <kbd>Esc</kbd>, then <strong>Sharing › Remote Control</strong>.</span></li>
        <li><span class="step">2</span><span>Choose <strong>Pair a browser</strong>.</span></li>
        <li><span class="step">3</span><span>Type the code it shows. It works once, for ten minutes.</span></li>
      </ol>
      <form id="pair-form" onSubmit=${(e) => { e.preventDefault(); store.pair(); }}>
        <label class="field">Pairing code<textarea id="code" rows="2" maxlength="80" autocomplete="off"
          autocapitalize="characters" spellcheck="false" required placeholder="XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX"
          value=${store.pairCode}
          onInput=${(e) => { store.pairCode = e.currentTarget.value; store.emit(); }}
          onKeyDown=${(e) => {
            if (e.key === "Enter" && !e.isComposing) {
              e.preventDefault();
              e.currentTarget.form.requestSubmit();
            }
          }}></textarea></label>
        <label class="field"><span>Device name <span class="muted">(optional)</span></span><input id="device-name" maxlength="64"
          placeholder="My browser" autocomplete="off" value=${store.deviceName}
          onInput=${(e) => { store.deviceName = e.currentTarget.value; }} /></label>
        <div class="pair-actions">
          <button id="cancel-pair" type="button" class="btn" hidden=${!store.canReturn} onClick=${() => store.returnToBoard()}>Back to board</button>
          <button id="pair" class="btn btn-pri" type="submit" disabled=${!store.pairReady}>Connect</button>
        </div>
      </form>
      ${!hidden && html`<p id="connection" role="status" class="onboarding-status">${store.status}</p>`}
      <p class="onboarding-note"><${Icon} name="lock" size=${14} /><span>The relay only passes sealed envelopes. It cannot read your board or your prompts.</span></p>
    </div>
  </section>`;
}

function ModeButtons({ store, board, ids = false }) {
  const mode = board?.mode || "agents";
  const needs = board ? board.tickets.filter((t) => t.agent?.state === "needs attention").length : 0;
  const unsettled = store.sent.unsettled(store.active?.pin.board);
  return html`
    <button id=${ids ? "agents-mode" : undefined} type="button" class="mode" data-mode="agents"
      aria-pressed=${String(mode === "agents")} onClick=${() => store.setMode("agents")}>
      <${Icon} name="inbox" size=${22} width=${1.8} /><span>Now</span>
      ${needs > 0 && html`<span class="badge">${needs}</span>`}
    </button>
    <button id=${ids ? "board-mode" : undefined} type="button" class="mode" data-mode="board"
      aria-pressed=${String(mode === "board")} onClick=${() => store.setMode("board")}>
      <${Icon} name="kanban" size=${22} width=${1.8} /><span>Board</span>
      ${board && html`<span class="count">${board.tickets.length}</span>`}
    </button>
    <button id=${ids ? "sent-mode" : undefined} type="button" class="mode" data-mode="sent"
      aria-pressed=${String(mode === "sent")} onClick=${() => store.setMode("sent")}>
      <${Icon} name="send" size=${22} width=${1.8} /><span>Sent</span>
      ${unsettled > 0 && html`<span class="badge badge-quiet" aria-label=${`${unsettled} not landed yet`}>${unsettled}</span>`}
    </button>`;
}

// On a phone's Board the ticket starts in the column on screen; elsewhere in
// the draft's own column, or the board's default.
function NewTicketButton({ store, board, id, cls, column }) {
  return html`<button id=${id} type="button" class=${cls} disabled=${!board}
    onClick=${() => store.openComposer(column)}>
    <${Icon} name="plus" size=${18} width=${2.4} /><span>New ticket</span></button>`;
}

// One line at a time, above the tab bar: what just happened to a ticket.
function Toast({ store }) {
  const toast = store.toast;
  return html`<div id="toast" class="toast" role="status" aria-live="polite">${toast &&
    html`<p class="toast-body" key=${toast.id}>${toast.tick && html`<${Tick} state=${toast.tick} />`}<span>${toast.text}</span></p>`}</div>`;
}

function Hop({ icon, name, state, ok }) {
  return html`<li class="hop"><span class="hop-icon"><${Icon} name=${icon} size=${18} width=${1.8} /></span>
    <span class="hop-text"><span class="hop-name">${name}</span><span class="hop-state">${state}</span></span>
    <span class=${`hop-dot${ok ? " ok" : ""}`} aria-hidden="true"></span></li>`;
}

function Sidebar({ store, bp }) {
  const open = store.sheetOpen && bp !== "desktop";
  const link = store.link;
  const seen = lastSeen(store.board);
  const terminal = {
    live: "Live",
    connecting: "Connecting…",
    asleep: seen ? `Out of reach · last seen ${seen}` : "Out of reach",
    relay: "Unknown while the relay is unreachable",
    nonet: "Unknown while this browser is offline",
  }[link];
  const boards = store.identity?.boards || [];
  return html`
    ${open && html`<button type="button" class="scrim" aria-label="Close boards and settings" onClick=${() => store.openSheet(false)}></button>`}
    <aside id="sidebar" class=${open ? "open" : ""} aria-label="Boards and settings">
      <div class="side-brand">
        <${Shin} scale=${3} light=${lightOf(store)} />
        <span class="side-brand-text"><span class="side-title">${store.board?.title || store.active?.title || "mesimon"}</span><span class="side-sub">Remote Control</span></span>
        <button type="button" class="icon-btn side-close" aria-label="Close" onClick=${() => store.openSheet(false)}><${Icon} name="x" size=${20} /></button>
      </div>
      <nav class="side-nav" aria-label="View"><${ModeButtons} store=${store} board=${store.board} /></nav>
      <section class="side-section" aria-label="Paired boards">
        <h2 class="label">Paired boards</h2>
        <ul class="side-boards">${boards.map((b) => html`<li key=${b.pin.board}>
          <button type="button" class="side-board" aria-current=${String(b === store.active)} onClick=${() => store.switchBoard(b.pin.board)}>
            <span class=${`hop-dot${b === store.active && store.live ? " ok" : ""}`} aria-hidden="true"></span>
            <span class="side-board-name">${b.title || "Paired board"}</span>
            <span class="side-board-state">${b.revoked ? "Access removed" : b === store.active ? linkLabel[link] : ""}</span>
          </button></li>`)}</ul>
        <button id="add-board" type="button" class="btn btn-quiet" onClick=${() => store.showPairing()}><${Icon} name="plus" size=${16} /><span>Pair another board</span></button>
      </section>
      <section class="side-section" aria-label="Connection">
        <h2 class="label">Connection</h2>
        <ol class="hops">
          <${Hop} icon="smartphone" name="This browser" state=${store.online ? "Online" : "No connection"} ok=${store.online} />
          <${Hop} icon="cloud" name="Relay" state=${link === "relay" ? "Unreachable" : link === "nonet" ? "Unknown" : "Reachable"} ok=${!["relay", "nonet"].includes(link)} />
          <${Hop} icon="terminal" name="Your terminal" state=${terminal} ok=${link === "live"} />
        </ol>
        ${link === "asleep" && html`<p class="side-note">Your Mac may be asleep, or mesimon isn’t running. If this lasts, check that the board still lists this browser under Remote Control.</p>`}
        <p class="side-note"><${Icon} name="lock" size=${13} /><span>End-to-end encrypted. The relay routes sealed envelopes it cannot read.</span></p>
      </section>
      <section class="side-section settings" aria-label="Settings">
        <h2 class="label">Settings</h2>
        <label class="field">Appearance<select id="theme" value=${store.theme} onChange=${(e) => store.setTheme(e.currentTarget.value)}>
          <option value="system">System</option>
          <option value="graphite">Graphite</option>
          <option value="chalk">Chalk</option>
        </select></label>
        <button id="alerts" type="button" class="btn btn-quiet" onClick=${() => store.enableAlerts()}><${Icon} name="bell" size=${16} /><span>Enable connected-browser alerts</span></button>
        <p id="alert-status" class="side-note">${store.alertStatus}</p>
        <button id="forget" type="button" class="btn btn-quiet btn-danger" onClick=${() => store.forget()}><${Icon} name="leave" size=${16} /><span>Forget this browser</span></button>
      </section>
    </aside>`;
}

function WorkList({ store, bp }) {
  const board = store.board;
  const mode = board?.mode || "agents";
  const sent = mode === "sent" ? store.sent.forBoard(store.active?.pin.board) : [];
  const list = useRef();
  // A list hidden behind the phone's detail view forgets its scroll offset;
  // put back the reader's place whenever the list shows again. Sent reads
  // like a conversation: it opens at the newest, by the bar.
  useLayoutEffect(() => {
    const node = list.current;
    if (!node || !board || !node.getClientRects().length) return;
    node.scrollTop = mode === "sent" ? node.scrollHeight : board.scroll[mode];
  }, [board, mode, store.detailOpen, bp, sent.length]);
  const plural = (n, word) => `${n} ${word}${n === 1 ? "" : "s"}`;
  const count = !board
    ? ""
    : mode === "agents"
      ? plural(board.visible().length, "agent")
      : mode === "board"
        ? plural(board.visible().length, "ticket")
        : `${sent.length} sent`;
  const loading = store.screen === "shell" && store.active && !board;
  return html`<section id="work-list" aria-label="Work list">
    <div class="list-tools">
      <div class="list-heading">
        <h2 id="list-title">${{ agents: "Now", board: "Board", sent: "Sent" }[mode]}</h2>
        <span id="count">${count}</span>
        ${board?.cached && html`<span class="chip chip-quiet">As of ${lastSeen(board)}</span>`}
        ${bp !== "phone" && html`<${NewTicketButton} store=${store} board=${board} id="new-ticket" cls="btn btn-pri new-ticket" />`}
      </div>
      ${mode === "sent"
        ? html`<p class="list-sub">Tickets from this browser to your board.</p>`
        : html`<label class="search"><${Icon} name="search" size=${16} /><span class="sr-only">Find a ticket</span>
            <input id="search" type="search" placeholder="Find a ticket…" value=${board?.search || ""}
              onInput=${(e) => store.setSearch(e.currentTarget.value)} /></label>`}
      ${mode === "board" && bp === "phone" && board && html`<${ColumnTabs} store=${store} board=${board} />`}
    </div>
    <nav id="tickets" aria-label=${mode === "sent" ? "Sent tickets" : "Tickets"} ref=${list}
      onScroll=${(e) => {
        if (board && e.currentTarget.getClientRects().length) board.scroll[mode] = e.currentTarget.scrollTop;
      }}>
      ${board
        ? mode === "agents"
          ? html`<${NowList} store=${store} board=${board} live=${store.live} />`
          : mode === "board"
            ? html`<${BoardList} store=${store} board=${board} bp=${bp} />`
            : html`<${SentList} store=${store} board=${board} />`
        : loading && html`<div class="loading"><${Shin} size="medium" scale=${4} mood="awake" light="dim" /><p>Reaching your board…</p></div>`}
    </nav>
    ${board && mode === "sent" && html`<${QuickNew} store=${store} />`}
    ${board && mode !== "sent" && bp === "phone" && html`<${NewTicketButton} store=${store} board=${board}
      id="new-ticket-fab" cls="fab" column=${mode === "board" ? board.column : undefined} />`}
    <nav id="modes" aria-label="View"><${ModeButtons} store=${store} board=${board} ids=${true} /></nav>
  </section>`;
}

function Shell({ store, bp, hidden }) {
  const board = store.board;
  const mode = board?.mode || "agents";
  return html`<div id="shell" hidden=${hidden} data-mode=${mode} data-detail=${String(store.detailOpen)} data-link=${store.link}>
    <${Sidebar} store=${store} bp=${bp} />
    <header id="board-header">
      <button id="board-menu" type="button" aria-expanded=${String(store.sheetOpen)} aria-controls="sidebar"
        onClick=${() => store.openSheet(!store.sheetOpen)}>
        <${Shin} light=${lightOf(store)} />
        <span id="board-title">${board?.title || store.active?.title || "Paired board"}</span>
        <${Icon} name="chevronDown" size=${16} cls="muted" />
      </button>
      <${Pill} store=${store} />
    </header>
    ${!hidden && html`<p id="connection" role="status" class=${store.status === "Connected" ? "sr-only" : "strip"}>
      ${store.status !== "Connected" && html`<${LinkIcon} link=${store.link} />`}<span>${store.status}</span></p>`}
    <div class="workspace">
      <${WorkList} store=${store} bp=${bp} />
      <${Detail} store=${store} bp=${bp} />
    </div>
  </div>`;
}

export function App({ store }) {
  useStore(store);
  const bp = useBreakpoint();
  useLayoutEffect(() => {
    const want = store.focus;
    if (!want) return;
    store.focus = null;
    const target =
      want === "selection"
        ? document.getElementById("selection")
        : want === "row"
          ? document.querySelector('#tickets .ticket[aria-pressed="true"]')
          : document.getElementById(want === "prompt" ? "prompt" : "code");
    if (target?.getClientRects().length) target.focus({ preventScroll: true });
  });
  return html`
    <${Pairing} store=${store} hidden=${store.screen !== "pair"} />
    <${Shell} store=${store} bp=${bp} hidden=${store.screen === "pair"} />
    <${NewTicket} store=${store} />
    <${Toast} store=${store} />`;
}
