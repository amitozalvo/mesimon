// One ticket: two rows about it, then the transcript, with its notes as the
// panes beside it (T-701), and the composer. Every element keeps its place
// in the DOM even when empty, so revocation visibly clears protected content
// rather than removing it.
import { html, useLayoutEffect, useRef, useState } from "./html.js";
import { Icon, Tick } from "./icons.js";
import { Shin, animOf } from "./shin.js";
import { Attention, cardAnswers } from "./dialogs.js";
import { CrownMark, StartButton, StartReceipt, WorktreeMark, ageWords, crownTouch, exitWords, loggedOut, stateAge } from "./lists.js";
import { NotePane, PictureList, PicturePick, pastePictures } from "./notepad.js";
import { receiptTick, sentPrompt } from "./sessions.js";
import { startWaiting } from "./starts.js";
import { modeHelp, queueWords, sendRefused, waitsOnYou } from "./queue.js";
import { Chat } from "./transcript.js";
import { letterWords } from "./letters.js";
import { KEYS_TEXT_MAX_BYTES } from "./store.js";
import { swipePanes } from "./swipe.js";
import { keepAtEnd } from "./viewport.js";

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

// The ticket page's tabs (T-1208): the conversation, the agent's own
// terminal screen, and the notes. A parked agent's conversation is still
// its file, but its screen is gone, and away the screen is the terminal's
// to show (T-698): there the Terminal tab is off.
function TicketTabs({ store, ticket, panes, at }) {
  const chat = store.chatShown;
  const notes = panes.length - 1;
  const screen = store.screenShown(ticket);
  const show = (view) => {
    store.showPane(ticket.id, 0);
    store.setOutputView(view);
  };
  const tab = (id, on, label, press, off = false, count = "") => html`<button id=${id} type="button" role="tab"
    aria-selected=${String(on)} class=${on ? "on" : ""} disabled=${off} onClick=${press}>${label}${count !== "" && html`<span class="tab-count">${count}</span>`}</button>`;
  return html`<nav id="ticket-tabs" class="ticket-tabs" role="tablist" aria-label="Ticket">
    ${store.chatCapable && tab("tab-chat", at === 0 && chat, "Chat", () => show("chat"))}
    ${tab("tab-terminal", at === 0 && !chat, "Terminal", () => show("raw"), !screen && store.chatCapable)}
    ${tab("tab-notes", at > 0, "Notes", () => (notes ? store.showPane(ticket.id, at > 0 ? at : 1) : store.editNote(ticket.id)),
      !notes && !store.canWriteNotes, notes || "")}
  </nav>`;
}

// The keys a phone has no key for (T-1208), sent straight into the
// agent's pane: the desk's own keys at its prompt and its dialogs. Enter is
// the line's own ⏎ (T-1219), which sends Enter alone when the line is
// empty, so the other fourteen fit two rows of a 360 px phone, none under
// 44 px and none behind a sideways scroll.
const KEYS = [
  ["escape", "esc"], ["tab", "tab"], ["ctrl_c", "^C"], ["up", "↑"], ["down", "↓"],
  ["left", "←"], ["right", "→"], ["ctrl_r", "^R"], ["ctrl_d", "^D"], ["backspace", "⌫"],
];
const DIGITS = ["1", "2", "3", "4"];

// The Terminal tab's keyboard (T-1208): a bar of keys, and a line typed
// into the pane, Enter after it, or each key as it is typed.
// The typed line belongs to its conversation (T-1219), kept on the session's
// entry as the composer's draft is: another ticket's or another session's
// Terminal tab never shows it, so it never goes to that agent, and it is
// there again on coming back. The page holds it in memory only.
function KeyBar({ store, ticket }) {
  const entry = store.entry;
  // A dialog the page answers on a card is answered there (T-1208): the
  // terminal refuses keys at it, so the bar rests until it is gone. At any
  // other stop the keys go in and answer the agent's own dialog (T-1218).
  const card = cardAnswers(store, ticket.agent);
  const asks = !card && ticket.agent?.state === "needs attention";
  const live = store.keysReach(ticket) && !card;
  // A typed line and its Enter on their way: no second line, no Enter.
  const typing = store.typing;
  const send = (press) => store.sendKeys(press);
  return html`<footer class="keys-area" aria-label="Keys">
    ${asks && live && html`<p id="keys-asks" class="stopped-why">These keys answer what the agent’s terminal is asking.</p>`}
    <div class="key-bar" role="group" aria-label="Keys for the agent's terminal">
      ${KEYS.map(([key, label]) => html`<button type="button" class="key" key=${key} data-key=${key} disabled=${!live}
        aria-label=${key.replace("_", " ")} onClick=${() => send({ key })}>${label}</button>`)}
      ${DIGITS.map((d) => html`<button type="button" class="key" key=${d} data-text=${d} disabled=${!live}
        onClick=${() => send({ text: d })}>${d}</button>`)}
    </div>
    <form id="keys-form" class="keys-line" onSubmit=${(e) => {
      e.preventDefault();
      if (typing || !entry) return;
      // A line goes, then its Enter once the host has it; alone, Enter.
      // A refused line keeps the field and sends no Enter, and the field
      // clears only if it still holds the line that went (T-1218): its own
      // conversation's, wherever the page is by then.
      const text = entry.keysLine || "";
      if (text)
        store.typeLine(text, (sent) => {
          if (entry.keysLine !== sent) return;
          entry.keysLine = "";
          store.emit();
        });
      else send({ key: "enter" });
    }}>
      <span class="keys-prompt" aria-hidden="true">›</span>
      <input id="keys-text" key=${entry?.key} value=${entry?.keysLine || ""} type="text" autocomplete="off" autocapitalize="off" spellcheck="false"
        enterkeyhint="send" disabled=${!live} aria-label="Type into the agent's terminal" maxlength=${KEYS_TEXT_MAX_BYTES}
        onInput=${(e) => {
          if (entry) entry.keysLine = e.currentTarget.value;
        }}
        placeholder=${live ? "Type a line, then ⏎" : card ? "The agent waits on you: answer it above" : "The agent's terminal is out of reach"} />
      <button id="keys-send" type="submit" class="send" disabled=${!live || typing} aria-label="Send the line and Enter, or Enter alone"><${Icon} name="enter" size=${18} width=${2.2} /></button>
    </form>
  </footer>`;
}

// An agent whose process ended (T-1208): why and when, and the two ways on.
// With words or pictures in the field, Resume takes the field's own road
// (T-1219), pictures uploaded first and the sending guards with it; it once
// sent the words with a picture's mark and no picture. With none, it
// resumes at once.
function StoppedLine({ store, ticket }) {
  const agent = ticket.agent;
  const entry = store.entry;
  const waiting = startWaiting(store.startOf(ticket));
  const at = agent.exit?.at;
  const when = Number.isFinite(at) ? ageWords(store.board, at) : "";
  return html`<div id="stopped-line" class="agent-line stopped-line" role="group" aria-label="Agent stopped">
    <p><span class="mark mark-stopped" aria-hidden="true"></span><b>${exitWords(agent)}${when && ` ${when === "now" ? "just now" : `${when} ago`}`}</b></p>
    <p class="stopped-why">${loggedOut(agent)
      ? `${agent.provider} was logged out. Log in to ${agent.provider} at your terminal first; then Resume picks its conversation up where it stopped, with what you type below if anything.`
      : `${agent.provider}’s process ended. Its conversation is kept: Resume picks it up where it stopped, with what you type below if anything.`}</p>
    <div class="stopped-actions">
      <button id="resume-agent" type="button" class=${loggedOut(agent) ? "btn" : "btn btn-pri"}
        disabled=${!store.canResume || waiting || !!entry?.sending || entry?.reading > 0}
        onClick=${() => (entry?.draft.trim() ? store.submitPrompt() : store.resumeAgent(ticket.id, "", entry?.tier || ""))}><${Icon} name="play" size=${15} /><span>Resume</span></button>
      <button id="fresh-agent" type="button" class="btn" disabled=${!store.canStart || waiting}
        onClick=${() => store.openStart(ticket.id)}>Start fresh</button>
    </div>
    ${!store.live && html`<p class="stopped-why">Resuming needs your terminal back.</p>`}
  </div>`;
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
  useLayoutEffect(() => keepAtEnd(ref.current, () => !!entry?.following));
  const text =
    entry?.displayed ||
    (entry?.receivedAt
      ? "No output in the latest preview."
      : ticket?.agent
        ? "Waiting for the first preview…"
        : ticket
          ? "No agent output."
          : "");
  const following = chat ? entry?.chatFollowing !== false : entry?.following;
  const unread = chat ? entry?.chatUnread : entry?.unread;
  // The dialog waiting on you is the conversation's last row (T-701): the
  // permission, the question or the plan sits where the agent stopped. On
  // the screen it sits above the lines.
  const attention = html`<${Attention} store=${store} ticket=${ticket} entry=${entry} live=${live} />`;
  // The screen (T-506): the lines at the pane's own width, the type sized by
  // CSS so that width fills the panel, and a line the capture joined wrapped
  // back where the pane had it. Where the pane is wider than the panel can
  // show legibly, the lines reflow at the panel's width and the rules stay
  // one row each (`.screen-lines`).
  return html`<section class="output" aria-label="Output" hidden=${!ticket?.agent || (ticket.agent.state === "sleeping" && !chat)}>
    ${!chat && attention}
    <div class="output-body">
      ${chat
        ? html`<${Chat} store=${store} entry=${entry} doing=${ticket?.agent?.doing} tail=${attention} />`
        : html`<pre id="preview" ref=${ref} class="screen" style=${{ "--cols": screenCols(entry) }} aria-label="Agent output" tabindex="0"
        onScroll=${(e) => store.outputScrolled(e.currentTarget)}><span class="screen-lines">${screenRows(text)}</span></pre>`}
      <button id="latest" type="button" class="latest" hidden=${!entry || following}
        onClick=${() => store.latest()}><${Icon} name="down" size=${16} /><span>${unread ? `New ${chat ? "words" : "preview"} · Jump to latest` : "Jump to latest"}</span></button>
    </div>
  </section>`;
}

// What the field says over an empty or a parked seat (T-709), before any
// words: the press starts or wakes, and why it cannot when it cannot.
function startPlaceholder(store, ticket, asleep) {
  const who = asleep ? ticket.agent.provider : "an agent";
  if (!store.live) return `${asleep ? "Waking" : "Starting"} ${who} needs your terminal back.`;
  if (startWaiting(store.startOf(ticket))) return `${asleep ? "Waking" : "Starting"} ${who}…`;
  return asleep ? `Wake ${who} with these words…` : "Start an agent with these words…";
}

function Composer({ store, ticket, entry, live }) {
  // A parked agent has no pane to type at: the field is its wake (T-709).
  const asleep = ticket?.agent?.state === "sleeping";
  // A stopped agent (T-1208) has no pane either: the field resumes it.
  const stopped = store.stopped(ticket);
  const agent = asleep || stopped ? undefined : ticket?.agent;
  // The desk's Shift+Enter on an empty or a parked seat (T-709): the same
  // field, and the press starts or wakes the agent on the words. A host
  // that starts no agents from here keeps the field off, as before.
  const starts = store.composerStarts(ticket);
  const startOff = !store.canComposeStart;
  // The terminal is away (T-881): the words are sealed for the relay and
  // queue when it is back. One waits per ticket.
  const mails = !starts && store.mailsPrompt(ticket);
  const letter = store.letterOf(ticket);
  const acting = entry?.receipt?.waiting && entry.receipt.status !== "queued";
  const queueOff = !live || !agent?.promptable || ticket?.queued == null || !!acting;
  // At a dialog a steer would be its answer (T-568): Steer is off, and the
  // words queue, until the agent no longer waits on you. A tier switch
  // (T-643) restarts the agent at its turn's end, so its words queue too.
  const tiers = agent || starts ? store.tierChoices(ticket) : [];
  const pick = store.promptTier(ticket);
  const switching = !!agent && store.switchesTier(ticket);
  const steerOff = waitsOnYou(agent) || switching;
  // A Codex agent still stopping (T-1218) takes no words yet (T-1219): the
  // field keeps them, and says why Send waits.
  const stopping = agent?.state === "stopping";
  const mode = steerOff ? "queue" : entry?.mode || "queue";
  const sending = !!entry?.sending;
  // A picture still being read is not in the words yet (T-1219): Send waits
  // for it, as the New ticket sheet and a note's Save do, or the words would
  // go without it.
  const reading = entry?.reading > 0;
  const sendOff = sending || reading || !!letter || (starts
    ? startOff || !entry?.draft.trim()
    : mails
      ? !entry?.draft.trim()
      : !live || !agent?.promptable || !!entry?.review || !!entry?.receipt?.waiting ||
        !!entry?.answer?.waiting || !entry?.draft.trim());
  const fieldOff = sending || (starts ? startOff : !agent);
  // Stop (T-1208): an empty field over a working turn is the desk's Escape.
  const stops = !!agent && ["working", "starting"].includes(agent.state) && !entry?.draft.trim() && store.keysReach(ticket);
  // A picture with the words (T-853): offered wherever the words can go,
  // to a host that keeps it with the ticket.
  const pictures = !!entry && !fieldOff && store.canAskPictures;
  const caret = useRef();
  const mark = (e) => {
    caret.current = e.currentTarget.selectionStart;
  };
  const tick = receiptTick(entry?.latest?.status);
  // In the conversation a sent prompt is its ghost, then its own row
  // (T-626): no line says it was submitted.
  const ghost = store.chatShown && !!sentPrompt(entry) && entry.latest === entry.receipt;
  // One line until there are words (T-701): the field alone with its arrow,
  // and the delivery row beneath once something is typed.
  const open = !!entry?.draft;
  // The field grows to its words (`field-sizing: content`); an engine
  // without it is measured by hand.
  const field = useRef();
  useLayoutEffect(() => {
    const node = field.current;
    if (!node || CSS.supports("field-sizing", "content")) return;
    node.style.height = "";
    node.style.height = `${node.scrollHeight}px`;
  }, [entry?.draft]);
  return html`<footer class="composer-area">
    <section id="letter-row" class="bubble-row" aria-label="Message waiting for your terminal" hidden=${!letter}>
      <div class="bubble">
        <pre id="letter-text" dir="auto">${letter?.text || ""}</pre>
        <p class="bubble-meta"><${Icon} name=${letter?.status === "unknown" ? "shield" : "moon"} size=${13} /><span id="letter-meta">${letter ? letterWords(letter) : ""}</span>${letter
          && html`<${Tick} state=${letter.status === "local" ? "clock" : letter.status === "relay" ? "one" : ""} />`}</p>
      </div>
      <div class="bubble-actions">
        <button id="letter-back" type="button" class="btn btn-quiet" onClick=${() => store.retractPrompt(letter.id)}>${letter?.status === "unknown" ? "Dismiss" : "Take back"}</button>
      </div>
    </section>
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
        : starts
          ? `${asleep ? `Wake ${ticket.agent.provider}` : "Start an agent"} on ${ticket.key} with the first prompt`
          : "No agent selected"}</p>
      <div class="composer" data-open=${String(open)} data-starts=${String(starts)}>
        <textarea id="prompt" ref=${field} aria-label=${starts ? "First prompt" : "Prompt"} aria-describedby="target" rows="1" dir="auto"
          placeholder=${mails ? `Message ${agent.provider} · it waits for your terminal…` : stopping ? `${agent.provider} is stopping · words wait here…` : agent ? `Message ${agent.provider}…` : starts ? (stopped && store.resumes ? `Resume ${ticket.agent.provider} with these words…` : startPlaceholder(store, ticket, asleep)) : "No agent to message"}
          required disabled=${fieldOff}
          value=${entry?.draft || ""} onInput=${(e) => {
            mark(e);
            store.setDraft(e.currentTarget.value);
          }}
          onClick=${mark} onKeyUp=${mark} onSelect=${mark}
          onPaste=${(e) => pastePictures(e, pictures, (files, at) => store.addPromptPictures(files, at), caret)}
          onKeyDown=${(e) => {
            mark(e);
            if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && !e.isComposing) {
              e.preventDefault();
              e.currentTarget.form.requestSubmit();
            }
          }}></textarea>
        ${entry?.pictures.length > 0 && html`<div class="composer-pics"><${PictureList} draft=${entry} onRemove=${(n) => store.removePromptPicture(n)} /></div>`}
        ${pictures && html`<${PicturePick} id="prompt-picture" draft=${entry} caret=${caret} label="Picture"
          onAdd=${(files, at) => store.addPromptPictures(files, at)} />`}
        <div class="composer-row" hidden=${!open}>
          ${mails
            ? html`<p id="away-what" class="start-what"><${Icon} name="moon" size=${13} /><span>${letter
                ? "One message waits for this ticket · take it back to change it."
                : `Waits for your terminal, then queues for ${agent.provider}.`}</span></p>`
            : starts
            ? html`<p id="start-what" class="start-what"><${Icon} name="play" size=${13} /><span>${asleep
                ? `Wakes ${ticket.agent.provider}, then says this.`
                : stopped && store.resumes
                  ? `Resumes ${ticket.agent.provider}’s conversation, then says this.`
                  : "Starts an agent on these words."}</span></p>`
            : html`<fieldset id="prompt-mode" class="segmented" disabled=${!agent}>
            <legend class="sr-only">Delivery</legend>
            <label class=${mode === "queue" ? "on" : ""}><input type="radio" name="prompt-mode" value="queue"
              checked=${mode === "queue"} onChange=${() => store.setDelivery("queue")} /><${Icon} name="hourglass" size=${14} /><span>Queue</span></label>
            <label class=${`${mode === "steer" ? "on" : ""}${steerOff ? " off" : ""}`}><input type="radio" name="prompt-mode" value="steer"
              checked=${mode === "steer"} disabled=${steerOff} onChange=${() => store.setDelivery("steer")} /><${Icon} name="zap" size=${14} /><span>Steer</span></label>
          </fieldset>`}
          ${tiers.length > 0 && !mails && html`<label class="tier-pick"><span class="sr-only">Agent tier</span>
            <select id="prompt-tier" value=${pick} onChange=${(e) => store.setPromptTier(e.currentTarget.value)}>
              ${tiers.map((t) => html`<option key=${t.id} value=${t.id}>${t.name}</option>`)}
            </select></label>`}
          ${!starts && !mails && html`<p class="mode-help">${modeHelp(mode, agent)}</p>`}
        </div>
        <button id="stop" type="button" class="send send-stop" hidden=${!stops} aria-label="Stop the turn"
          onClick=${() => store.stopTurn()}><${Icon} name="stop" size=${16} /></button>
        <button id="send" type="submit" class="send" hidden=${stops} disabled=${sendOff}
          aria-label=${starts ? (asleep ? "Wake agent with these words" : "Start agent with these words") : mails ? "Queue for when your terminal is back" : mode === "steer" ? "Send prompt" : "Queue prompt"}>${starts
            ? html`<${Icon} name="play" size=${18} />`
            : html`<${Icon} name="up" size=${20} width=${2.2} />`}</button>
        <p id="steer-why" class="steer-why" hidden=${!(steerOff || stopping) || !open || mails}>${stopping
          ? `${agent.provider} is stopping: these words wait here until it has stopped.`
          : switching
          ? `Restarts on ${store.board?.tierNamed(pick)?.name || pick} when the turn ends, then sends.`
          : "Steer is off while the agent waits on you · answer it first. Queue still works."}</p>
      </div>
      <p id="delivery" role="status" hidden=${!sending && !entry?.error && (ghost || (!tick && !entry?.delivery))}>${!sending && !entry?.error && tick && html`<${Tick} state=${tick} />`}<span>${entry?.sending || entry?.error || entry?.delivery || ""}</span></p>
    </form>
  </footer>`;
}

// The heading, two lines of it at most (T-701), and the rename it opens
// (T-530): a press on the title writes over it, whole, Enter saves and
// Escape puts it back.
function Title({ store, ticket, open = false }) {
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
  return html`<div class="title-row">
    <h2 id="selection" class=${`${renames || open ? "" : "clamp"}${ticket?.crown ? " crowned" : ""}`} tabindex="-1" dir="auto">${!ticket
      ? "Select a ticket"
      : renames
        ? html`<button id="rename" type="button" class="title-button clamp" aria-describedby="rename-hint"
            onClick=${() => store.startRename()}><${CrownMark} ticket=${ticket} size=${18} />${ticket.title}</button>`
        : html`<${CrownMark} ticket=${ticket} size=${18} />${ticket.title}`}</h2>
    ${ticket && html`<span class="selection-key">${ticket.key}</span>`}
    ${renames && html`<span id="rename-hint" class="sr-only">Rename</span>`}
  </div>`;
}

// Whether the card sheet offers this ticket's workspace (T-642): a host
// that takes the choice, and a choice still open, as the TUI's Shift+Tab.
export const choosesWorkspace = (store, ticket) => store.canEdit("workspace") && !!ticket?.workspace?.open;

// The ticket page's tags on one line (T-1219): the tags that fit whole, then
// "+N" for the rest, never one cut mid-word (a single tag too long for the
// line ends in an ellipsis). The room is the line's less its other chips;
// every tag stays in the page, the ones that do not fit out of the flow, so
// each is measured again as the line widens. The card sheet (or, on a host
// that edits nothing, the line opened in place) shows every one.
const TAG_GAP = 8;
const MORE_ROOM = 34;
function LineTags({ ticket, all }) {
  const tags = ticket.tags || [];
  const ref = useRef();
  const [fit, setFit] = useState(tags.length);
  useLayoutEffect(() => {
    const node = ref.current;
    const line = node?.parentElement;
    if (!node || !line) return;
    const measure = () => {
      // A tag squeezed to its ellipsis is measured at its whole width.
      const widths = [...node.querySelectorAll(".tag")].map((k) => Math.max(k.getBoundingClientRect().width, k.scrollWidth));
      const style = getComputedStyle(line);
      const gap = parseFloat(style.columnGap) || 0;
      let room = line.clientWidth - (parseFloat(style.paddingLeft) || 0) - (parseFloat(style.paddingRight) || 0);
      for (const other of line.children)
        if (other !== node && other.getClientRects().length) room -= other.getBoundingClientRect().width + gap;
      let n = 0;
      let used = 0;
      while (n < widths.length) {
        const next = used + (n ? TAG_GAP : 0) + widths[n];
        if (next + (n + 1 < widths.length ? MORE_ROOM : 0) > room) break;
        used = next;
        n++;
      }
      setFit(Math.max(1, n));
    };
    measure();
    if (typeof ResizeObserver !== "function") return;
    const watch = new ResizeObserver(measure);
    watch.observe(line);
    return () => watch.disconnect();
  }, [tags.map((t) => t.name).join("\n")]);
  if (!tags.length) return null;
  const shown = all ? tags.length : Math.min(fit, tags.length);
  return html`<span class="line-tags" ref=${ref}><span class="tags">${tags.map((t, i) => html`<span class=${`tag tint-${t.tint}${i < shown ? "" : " tag-off"}`} key=${`${t.group}:${t.name}`}>${t.name}</span>`)}</span>${shown < tags.length && html`<span class="tags-more">+${tags.length - shown}<span class="sr-only"> more</span></span>`}</span>`;
}

// The ticket's line (T-510, T-701): the shin while an agent holds the seat,
// then the tags, the column and the worktree's glyph, and with a host that
// takes the card edits (T-530) the whole of it one button that opens the
// sheet moving and tagging it, where the branch and its state are spelled
// out. The glyph is the card's (`WorktreeMark`), with the count to merge.
// On a host that edits nothing the line opens in place (T-1219): the whole
// title and every tag.
function TicketLine({ store, ticket, open, setOpen }) {
  const moves = store.canEdit("move");
  const tags = store.canEdit("tag") && store.board.allowedTags.length > 0;
  const add = tags && !ticket.tags?.length;
  const chooses = choosesWorkspace(store, ticket);
  const archives = store.canEdit("archive");
  const crowns = store.canEdit("crown");
  const chips = html`<${LineTags} ticket=${ticket} all=${open} />${add && html`<span class="chip chip-quiet add-tag"><${Icon} name="plus" size=${12} width=${2.4} /><span>Tag</span></span>`}<span class="chip chip-column">${ticket.column}${moves && html`<${Icon} name="chevronDown" size=${12} width=${2.4} />`}</span><${WorktreeMark} ticket=${ticket} count=${true} />`;
  if (!moves && !tags && !chooses && !archives && !crowns)
    return html`<button id="line-open" type="button" class="chips chips-edit" aria-expanded=${String(open)}
      aria-label=${open ? "Show less" : "Show the whole title and every tag"} onClick=${() => setOpen(!open)}>${chips}</button>`;
  const verbs = [moves && "move", tags && "tag", crowns && "crown", archives && "archive"].filter(Boolean);
  const said = verbs.length > 1 ? `${verbs.slice(0, -1).join(", ")} or ${verbs.at(-1)}` : verbs[0] || "";
  const edits = said && `${said[0].toUpperCase()}${said.slice(1)} this ticket`;
  const hint = !chooses ? edits : edits ? `${edits}, or choose its workspace` : "Choose this ticket’s workspace";
  return html`<button id="card-line" type="button" class="chips chips-edit" aria-haspopup="dialog" aria-describedby="card-line-hint"
      onClick=${() => store.openCardSheet(ticket.id)}>${chips}</button>
    <span id="card-line-hint" class="sr-only">${hint}</span>`;
}

// What the crown did, in a sentence (T-623), by the card's word. A word
// this page does not know is said as the host spells it.
const CROWN_DID = {
  moved: "moved this ticket",
  renamed: "renamed this ticket",
  tagged: "tagged this ticket",
  note: "wrote a note here",
  workspace: "set this ticket’s workspace",
  created: "filed this ticket",
  started: "started this agent",
  woken: "woke this agent",
  parked: "parked this agent",
  asked: "queued words for this agent",
  sent: "sent this agent words",
  answered: "answered this agent",
  "accepted plan": "accepted this agent’s plan",
  merged: "merged this ticket",
};
export function crownSentence(touch) {
  // A worker's news woke the crown: `by` is the worker.
  if (touch.action === "woke") return touch.by ? `${touch.by}’s news woke this agent` : "A worker’s news woke this agent";
  const did = CROWN_DID[touch.action] || touch.action;
  return touch.by ? `${touch.by}’s agent ${did}` : `The crown ${did}`;
}

// A ticket the crown touched says what it did, at the transcript's top (T-623).
function CrownLine({ store, ticket }) {
  const touch = ticket && crownTouch(store.board, ticket);
  if (!touch) return null;
  const age = ageWords(store.board, touch.at);
  return html`<div id="crown-line" class="crown-line">
    <${Icon} name="crown" size=${15} width=${2.2} />
    <p>${crownSentence(touch)}${age && ` · ${age === "now" ? "just now" : `${age} ago`}`}</p>
  </div>`;
}

// What the ticket page says over an empty or a parked seat (T-498, T-510).
function seatWords(store, agent) {
  const what = agent ? `${agent.provider} is asleep on this ticket.` : "No agent on this ticket.";
  const it = agent ? "it" : "one";
  if (!store.startsAgents || (store.live && !store.canStart))
    return `${what} ${agent ? "Wake" : "Start"} ${it} at your terminal.`;
  return store.live ? what : `${what} ${agent ? "Waking" : "Starting"} ${it} needs your terminal back.`;
}

// The divider between the ticket and its panes (T-701): the pane's name at
// the left, what a swipe reaches at the right, and both are presses too.
// On the transcript it says how many notes lie to the right; on a note,
// which one this is of them.
function PaneBar({ store, ticket, panes, at }) {
  const notes = panes.length - 1;
  const last = at === panes.length - 1;
  const writes = store.canWriteNotes;
  const name = at === 0 ? "Transcript" : panes[at].name;
  const rhs = at === 0
    ? notes
      ? `${notes} ${notes === 1 ? "note" : "notes"}`
      : writes
        ? "note"
        : ""
    : `${at}/${notes}`;
  const next = () => (at === 0 && !notes ? store.editNote(ticket.id) : store.showPane(ticket.id, at + 1));
  return html`<div id="pane-bar" class="pane-bar" data-pane=${at}>
    <button id="pane-name" type="button" class="pane-name" disabled=${at === 0} aria-label=${at === 0 ? name : `${name} · back`}
      onClick=${() => store.showPane(ticket.id, at - 1)}>
      ${at > 0 && html`<span class="pane-arrow" aria-hidden="true">‹</span>`}<span dir="auto">${name}</span></button>
    <span class="pane-rule" aria-hidden="true"></span>
    <button id="pane-next" type="button" class="pane-next" disabled=${last && !(at === 0 && !notes && writes)} hidden=${!rhs}
      aria-label=${at === 0 ? (notes ? `${rhs} · swipe left to read them` : "Add a description") : last ? rhs : `${rhs} · next note`}
      onClick=${next}>${at === 0 && !notes && writes && html`<${Icon} name="plus" size=${11} width=${2.6} />`}<span>${rhs}</span>${(at === 0 ? notes > 0 : !last) && html`<span class="pane-arrow" aria-hidden="true">›</span>`}</button>
  </div>`;
}

export function Detail({ store, bp }) {
  const ticket = store.board?.current;
  const entry = store.entry;
  const live = store.live;
  const agent = ticket?.agent;
  const asleep = agent?.state === "sleeping";
  const since = store.board && stateAge(store.board, agent);
  const start = store.startOf(ticket);
  // The seat a start reaches (T-510): empty, or a parked agent it wakes.
  const seatOpen = !!ticket && (!agent || asleep);
  // The panes (T-701): the transcript, then the notes in their order, the
  // description first, and a note on its way last. Which one is shown is
  // the store's, so a swipe, the bar and Back agree.
  const panes = ticket ? store.panesOf(ticket) : [{ kind: "transcript", name: "Transcript" }];
  const at = ticket ? store.paneAt(ticket) : 0;
  const track = useRef();
  useLayoutEffect(() => {
    const node = track.current;
    if (!node || !ticket) return;
    return swipePanes(node, (step) => store.showPane(ticket.id, at + step));
  }, [ticket?.id, at]);
  const anim = animOf(agent);
  // The header opened in place (T-1219), for one ticket at a time.
  const [opened, setOpened] = useState();
  const open = !!ticket && opened === ticket.id;
  return html`<article id="detail" aria-labelledby="selection" data-head=${open ? "open" : undefined}>
    <header class="detail-head">
      <button id="back" type="button" class="icon-btn" aria-label="Back" onClick=${() => store.back()}><${Icon} name="back" size=${22} /></button>
      <button id="close-detail" type="button" class="icon-btn" aria-label="Close" onClick=${() => store.back()}><${Icon} name="x" size=${20} /></button>
      <div class="detail-title">
        <${Title} store=${store} ticket=${ticket} open=${open} />
        ${ticket && html`<div class="ticket-line">
          <span class="detail-shin" hidden=${!agent}>
            ${agent && html`<${Shin} scale=${2} anim=${anim} />
              <span id="agent-word" class="sr-only">${agent.provider} · ${agent.state}${since ? ` · ${since}` : ""}</span>`}
          </span>
          <${TicketLine} store=${store} ticket=${ticket} open=${open} setOpen=${(on) => setOpened(on ? ticket.id : undefined)} />
        </div>`}
      </div>
    </header>
    ${ticket && html`<${TicketTabs} store=${store} ticket=${ticket} panes=${panes} at=${at} />`}
    ${ticket && at > 0 && html`<${PaneBar} store=${store} ticket=${ticket} panes=${panes} at=${at} />`}
    <div class="panes" ref=${track}>
      <div class="pane-track" style=${{ "--pane": at }}>
        <section class="pane pane-transcript" aria-label="Transcript" aria-hidden=${String(at !== 0)} inert=${at !== 0 || undefined}>
          <div class="detail-scroll">
            <${CrownLine} store=${store} ticket=${ticket} />
            ${store.stopped(ticket) && html`<${StoppedLine} store=${store} ticket=${ticket} />`}
            ${agent?.state === "stopping" && html`<div id="stopping-line" class="agent-line stopping-line" role="status">
              <p><${Icon} name="spinner" size=${15} width=${2.4} cls="spin" /><b>Stopping…</b></p>
              <p class="stopped-why">${agent.provider} is stopping. Words typed below wait here until it has stopped; then Resume or Start fresh takes them.</p>
            </div>`}
            ${seatOpen && html`<div class="agent-line" role="group" aria-label="Agent">
              <p id="agent-state">${seatWords(store, agent)}</p>
              ${store.startsAgents && html`<${StartButton} store=${store} ticket=${ticket} />`}
            </div>`}
            ${ticket && html`<${StartReceipt} item=${start} agent=${agent} />`}
            <${Output} store=${store} ticket=${ticket} entry=${entry} live=${live} />
            ${!ticket && html`<div class="detail-empty"><${Shin} size="medium" scale=${4} /><p>Pick a ticket to read its agent’s output and send it a prompt.</p></div>`}
          </div>
        </section>
        ${ticket && panes.slice(1).map((pane, i) => html`<section class="pane pane-note" key=${pane.key} aria-label=${pane.name}
            aria-hidden=${String(at !== i + 1)} inert=${at !== i + 1 || undefined}>
          <${NotePane} store=${store} ticket=${ticket} pane=${pane} shown=${at === i + 1} />
        </section>`)}
      </div>
    </div>
    ${ticket && at === 0 && !store.chatShown && store.keysReach(ticket)
      ? html`<${KeyBar} store=${store} ticket=${ticket} />`
      : html`<${Composer} store=${store} ticket=${ticket} entry=${entry} live=${live} />`}
  </article>`;
}
