// Writing a ticket: the New ticket sheet and Sent's one-line bar share one
// draft in the store. A ticket lands quietly: nothing here starts an agent.
import { html, useLayoutEffect, useRef } from "./html.js";
import { Icon } from "./icons.js";
import { DESCRIPTION_MAX_BYTES } from "./store.js";

// Where the ticket is going, and whether it can get there now.
function destination(store) {
  if (store.canFile) return { link: "live", text: "Your terminal is live, so it lands right away." };
  if (store.live)
    return { link: "old", text: "This terminal’s mesimon is too old to take tickets from here. Update it on your Mac." };
  return { link: "away", text: "Your terminal is out of reach. Keep writing, and send it when the terminal is back." };
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
  const ready = store.canFile && !!draft.title.trim();
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
          <${Icon} name=${dest.link === "live" ? "terminal" : dest.link === "old" ? "shield" : "moon"} size=${16} />
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
        <p class="compose-quiet"><${Icon} name="moon" size=${15} /><span>It lands quietly. No agent starts until you start one at your terminal.</span></p>
      </div>
      <footer class="compose-foot">
        ${draft.error && html`<p class="compose-error" role="alert">${draft.error}</p>`}
        <button id="send-ticket" type="submit" class="btn btn-pri compose-send" disabled=${!ready}>
          <${Icon} name="send" size=${18} /><span>Send ticket</span></button>
        <p class="compose-note">${store.canFile ? "It lands on your board in a moment." : "Sending needs your terminal."}</p>
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
  const ready = store.canFile && !!draft.title.trim();
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
