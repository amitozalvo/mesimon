// Writing a ticket: the New ticket sheet and Sent's one-line bar share one
// draft in the store. A ticket lands quietly: nothing here starts an agent.
import { html, useLayoutEffect, useRef } from "./html.js";
import { Icon } from "./icons.js";
import { DESCRIPTION_MAX_BYTES, PROMPT_MAX_BYTES } from "./store.js";

// Where the ticket is going, and how it gets there from here (T-497): live,
// through the relay's mailbox while the terminal is away, or kept in this
// browser until there is a connection.
function destination(store) {
  if (store.live && store.canSend)
    return { link: "live", icon: "terminal", text: "Your terminal is live, so it lands right away.",
      note: "It lands on your board in a moment." };
  if (store.collects && (store.link === "nonet" || store.link === "relay"))
    return { link: "held", icon: "wifiOff",
      text: store.link === "nonet"
        ? "No connection. It stays in this browser and goes out when you’re back online."
        : "The relay is out of reach. It stays in this browser and goes out when the relay is back.",
      note: "Saved in this browser. It goes out by itself." };
  if (store.collects)
    return { link: "away", icon: "moon",
      text: "Your terminal is out of reach. It waits at the relay, sealed, and lands when the terminal is back.",
      note: "You can edit or unsend it until it lands." };
  if (store.live)
    return { link: "old", icon: "shield",
      text: "This terminal’s mesimon is too old to take tickets from here. Update it on your Mac.",
      note: "Sending needs a newer mesimon." };
  return { link: "old", icon: "moon",
    text: "Your terminal is out of reach, and when it was last live its mesimon kept no tickets for later. Update it, then open this page while it’s live.",
    note: "Sending needs your terminal." };
}

function Tags({ store, draft, board }) {
  const groups = [];
  for (const tag of board.allowedTags) {
    const group = groups.find((g) => g.group === tag.group);
    if (group) group.tags.push(tag);
    else groups.push({ group: tag.group, tags: [tag] });
  }
  if (!groups.length) return null;
  return html`<fieldset class="choices">
    <legend>Tags <span class="muted">· one per group</span></legend>
    ${groups.map((g) => html`<div class="choice-row" key=${g.group} role="group" aria-label=${`Tag group ${g.group}`}>
      ${g.tags.map((tag) => {
        const worn = draft.tags.some((t) => t.group === tag.group && t.name === tag.name);
        return html`<button type="button" class=${`tag-chip tint-${tag.tint}`} aria-pressed=${String(worn)}
          onClick=${() => store.toggleTag(tag)}><span class="tag-dot" aria-hidden="true"></span><span>${tag.name}</span></button>`;
      })}
    </div>`)}
  </fieldset>`;
}

export function NewTicket({ store }) {
  const ref = useRef();
  const draft = store.draft();
  const board = store.board;
  useLayoutEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (draft.open && board && !dialog.open) dialog.showModal();
    if ((!draft.open || !board) && dialog.open) dialog.close();
  });
  if (!board) return html`<dialog id="new-ticket-sheet" class="compose" ref=${ref}></dialog>`;
  const dest = destination(store);
  const ready = store.canSend && !!draft.title.trim();
  const about = board.columnDescriptions[draft.column];
  const oversize = new TextEncoder().encode(draft.description).length > DESCRIPTION_MAX_BYTES;
  return html`<dialog id="new-ticket-sheet" class="compose" ref=${ref} aria-labelledby="new-ticket-heading"
      onCancel=${(e) => {
        e.preventDefault();
        store.closeComposer();
      }}
      onClose=${() => store.closeComposer()}
      onClick=${(e) => {
        if (e.target === e.currentTarget) store.closeComposer();
      }}>
    <form class="compose-form" onSubmit=${(e) => {
      e.preventDefault();
      store.sendTicket();
    }}>
      <header class="compose-head">
        <button type="button" class="btn btn-quiet" onClick=${() => store.closeComposer()}>Cancel</button>
        <h2 id="new-ticket-heading">${draft.replaces ? "Edit ticket" : "New ticket"}</h2>
        <button type="submit" class="btn btn-quiet compose-send-top" disabled=${!ready}>Send</button>
      </header>
      <div class="compose-body">
        <p class="compose-dest" data-link=${dest.link}>
          <${Icon} name=${dest.icon} size=${16} />
          <span><strong>To ${board.title || "your board"}.</strong> ${dest.text}</span>
        </p>
        <label class="field">Title<input id="new-title" type="text" dir="auto" maxlength="500" autocomplete="off"
          enterkeyhint="send" required autofocus placeholder="What needs doing?" value=${draft.title}
          onInput=${(e) => store.setComposer("title", e.currentTarget.value)} /></label>
        <label class="field"><span>Details <span class="muted">(optional)</span></span><textarea id="new-description"
          rows="4" dir="auto" placeholder="Details, links, what done looks like. Markdown works." value=${draft.description}
          aria-invalid=${String(oversize)}
          onInput=${(e) => store.setComposer("description", e.currentTarget.value)}
          onKeyDown=${(e) => {
            if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && !e.isComposing) {
              e.preventDefault();
              e.currentTarget.form.requestSubmit();
            }
          }}></textarea></label>
        ${oversize && html`<p class="compose-error">Details must fit in 32 KiB.</p>`}
        <fieldset class="choices">
          <legend>Column</legend>
          <div class="choice-row">${board.columns.map((column) => html`<label class="choice" key=${column}>
            <input type="radio" name="new-column" value=${column} checked=${column === draft.column}
              onChange=${() => store.setComposer("column", column)} /><span>${column}</span></label>`)}</div>
          ${about && html`<p class="field-note" dir="auto">${about}</p>`}
        </fieldset>
        <${Tags} store=${store} draft=${draft} board=${board} />
      </div>
      <footer class="compose-foot">
        ${draft.error && html`<p class="compose-error" role="alert">${draft.error}</p>`}
        <button id="send-ticket" type="submit" class="btn btn-pri compose-send" disabled=${!ready}>
          <${Icon} name="send" size=${18} /><span>Send ticket</span></button>
        <p class="compose-note">${dest.note}</p>
      </footer>
    </form>
  </dialog>`;
}

// Sent's own composer, a message bar: a title and a column, and the sheet
// one tap away for details and tags.
export function QuickNew({ store }) {
  const board = store.board;
  if (!board) return null;
  const draft = store.draft();
  const extras = !!draft.description.trim() || draft.tags.length > 0;
  const ready = store.canSend && !!draft.title.trim();
  return html`<form id="quick-new" class="quick-new" onSubmit=${(e) => {
    e.preventDefault();
    store.sendTicket();
  }}>
    <label class="quick-column"><span class="sr-only">Column</span>
      <select id="quick-column" value=${draft.column} onChange=${(e) => store.setComposer("column", e.currentTarget.value)}>
        ${board.columns.map((column) => html`<option key=${column} value=${column}>${column}</option>`)}
      </select></label>
    <label class="quick-title"><span class="sr-only">New ticket title</span>
      <input id="quick-title" type="text" dir="auto" maxlength="500" autocomplete="off" enterkeyhint="send"
        placeholder="New ticket…" value=${draft.title}
        onInput=${(e) => store.setComposer("title", e.currentTarget.value)} /></label>
    <button id="quick-more" type="button" class="icon-btn quick-more" data-extras=${String(extras)}
      aria-label=${extras ? "More options, details or tags set" : "More options"} onClick=${() => store.openComposer()}>
      <${Icon} name="sliders" size=${20} /></button>
    <button id="quick-send" type="submit" class="send" aria-label="Send ticket" disabled=${!ready}>
      <${Icon} name="up" size=${20} width=${2.2} /></button>
    ${draft.error && !draft.open && html`<p class="quick-error" role="alert">${draft.error}</p>`}
  </form>`;
}

// The first turn's words (T-510): the desk's Shift+Enter field as a sheet.
// Blank, an empty seat starts on the ticket's title and description and a
// parked agent wakes with nothing to say; the host picks the provider.
export function StartSheet({ store }) {
  const ref = useRef();
  const ask = store.startAsk;
  const ticket = ask && store.board?.tickets.find((t) => t.id === ask.ticket);
  useLayoutEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (ask && ticket && !dialog.open) dialog.showModal();
    if ((!ask || !ticket) && dialog.open) dialog.close();
  });
  if (!ask || !ticket) return html`<dialog id="start-sheet" class="compose" ref=${ref}></dialog>`;
  const asleep = ticket.agent?.state === "sleeping";
  const words = !!ask.text.trim();
  return html`<dialog id="start-sheet" class="compose" ref=${ref} aria-labelledby="start-heading"
      onCancel=${(e) => {
        e.preventDefault();
        store.closeStart();
      }}
      onClose=${() => store.closeStart()}
      onClick=${(e) => {
        if (e.target === e.currentTarget) store.closeStart();
      }}>
    <form class="compose-form" onSubmit=${(e) => {
      e.preventDefault();
      store.confirmStart();
    }}>
      <header class="compose-head">
        <button type="button" class="btn btn-quiet" onClick=${() => store.closeStart()}>Cancel</button>
        <h2 id="start-heading">${asleep ? "Wake agent" : "Start agent"} · ${ticket.key}</h2>
        <span></span>
      </header>
      <div class="compose-body">
        <p class="start-title" dir="auto">${ticket.title}</p>
        <label class="field"><span>First prompt <span class="muted">(optional)</span></span><textarea id="start-prompt"
          rows="4" dir="auto" maxlength=${PROMPT_MAX_BYTES} autofocus
          placeholder=${asleep ? "What to say when it wakes." : "What to do first. Empty sends the title and details."}
          value=${ask.text} onInput=${(e) => store.setStartText(e.currentTarget.value)}
          onKeyDown=${(e) => {
            if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && !e.isComposing) {
              e.preventDefault();
              e.currentTarget.form.requestSubmit();
            }
          }}></textarea></label>
        <p class="field-note">${asleep
          ? "Empty wakes it and says nothing, as the board’s wake does."
          : "Empty starts it on the ticket’s title and details, as the board’s Shift+Enter does. Your terminal picks the provider."}</p>
      </div>
      <footer class="compose-foot">
        <button id="start-send" type="submit" class="btn btn-pri compose-send">
          <${Icon} name="play" size=${18} /><span>${asleep ? (words ? "Wake with these words" : "Wake agent") : words ? "Start with these words" : "Start on the title"}</span></button>
      </footer>
    </form>
  </dialog>`;
}
