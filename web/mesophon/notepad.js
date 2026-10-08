// A ticket's notes on its page (T-532): since T-701 each is a pane beside
// the transcript, the description first, and the edit sheet. Bodies render
// as text nodes through `Markdown`, never as HTML.
import { html, useLayoutEffect, useRef } from "./html.js";
import { Icon, Tick } from "./icons.js";
import { Markdown } from "./markdown.js";

// A person's note reads a newline as a break, an agent's as a space, as the
// desk's `Newline::of_note` reads them (T-626).
const byPerson = (row) => !!row && row.by !== "agent";
import { NOTE_MAX_BYTES, ago } from "./notes.js";

const clock = (at) => new Date(at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
const waitTick = { sending: "clock", local: "clock", relay: "one" };
// What one note's own edit, if any, says about it.
function PendingMark({ item }) {
  if (!item) return null;
  if (waitTick[item.status]) return html`<${Tick} state=${waitTick[item.status]} />`;
  return html`<span class="note-flag">Not saved</span>`;
}

// One's own edit of the open note, as it stands: on its way, or not saved.
function PendingStrip({ store, item }) {
  if (!item) return null;
  if (waitTick[item.status])
    return html`<div class="note-strip" role="status">
      <${Tick} state=${waitTick[item.status]} /><span>${item.status === "relay" ? "Waits for your terminal" : "Saving…"}</span>
      ${["local", "relay"].includes(item.status) &&
        html`<button type="button" class="btn btn-quiet" onClick=${() => store.retractNote(item.id)}>Unsend</button>`}
    </div>`;
  if (item.status === "stale")
    return html`<div class="note-strip note-strip-conflict" role="alert">
      <p><strong>${item.stale.by} changed it</strong> at ${clock(item.stale.at)}. Your words are below.</p>
      <div class="note-strip-actions">
        <button id="keep-theirs" type="button" class="btn" onClick=${() => store.dropNote(item.id)}>Keep theirs</button>
        <button id="save-mine" type="button" class="btn btn-pri" disabled=${!store.canWriteNotes}
          onClick=${() => store.saveMine(item.id)}>Save mine</button>
      </div>
    </div>`;
  return html`<div class="note-strip note-strip-conflict" role="alert">
    <p>${item.status === "unknown" ? "Save unknown." : `Not saved: ${item.message || "refused"}.`}</p>
    <div class="note-strip-actions">
      <button type="button" class="btn" onClick=${() => store.dropNote(item.id)}>Discard</button>
    </div>
  </div>`;
}

// One note as a pane beside the transcript (T-701): the whole of it, who
// wrote it and when, one's own edit of it as it stands, and the presses
// that edit it or add the next. A note on its way (`fresh`) is a pane too,
// with its tick, until the host lists it. Bodies render as text nodes
// through `Markdown`, never as HTML.
export function NotePane({ store, ticket, pane, shown }) {
  const board = store.active.pin.board;
  const entry = store.noteBook().entry(ticket.id);
  const away = !store.live;
  const asOf = away && entry?.at ? ` · as of ${clock(entry.at)}` : "";
  if (pane.kind === "fresh") {
    const item = pane.item;
    const unsent = ["local", "relay"].includes(item.status);
    return html`<div class="note-pane" data-fresh="true">
      <p class="note-pane-meta"><${PendingMark} item=${item} /><span>${item.status === "stale" || item.status === "rejected" || item.status === "unknown"
        ? item.message || "Not saved"
        : unsent ? "On its way · waits for your terminal" : "Saving…"}</span></p>
      <div class="note-pane-body"><${Markdown} text=${item.text} breaks=${true} /></div>
      <div class="note-pane-foot">
        ${unsent
          ? html`<button type="button" class="btn btn-quiet" onClick=${() => store.retractNote(item.id)}>Unsend</button>`
          : !waitTick[item.status] &&
            html`<button type="button" class="btn btn-quiet" onClick=${() => store.dropNote(item.id)}>Discard</button>`}
      </div>
    </div>`;
  }
  if (pane.kind === "unread")
    return html`<div class="note-pane"><p class="notes-empty">${pane.count} ${pane.count === 1 ? "note" : "notes"} · ${away ? "needs your terminal" : "loading…"}</p></div>`;
  const row = pane.row;
  const pending = store.noteMail.pending(board, ticket.id, row.id);
  const body = pending ? pending.text : store.noteBook().body(ticket.id, row.id);
  const editable = store.canWriteNotes && (body !== undefined || !!pending) && pending?.status !== "sending";
  const writes = store.canWriteNotes;
  return html`<div class="note-pane" data-note=${row.id}>
    <p class="note-pane-meta"><span>${row.by} · ${ago(row.at)}${asOf}</span></p>
    <${PendingStrip} store=${store} item=${pending} />
    ${body !== undefined
      ? html`<div class="note-pane-body"><${Markdown} text=${body} breaks=${byPerson(row)} /></div>`
      : html`<p class="notes-empty">${away ? "Needs your terminal." : "Loading…"}</p>`}
    ${writes && html`<div class="note-pane-foot">
      <button id=${shown ? "edit-note" : undefined} type="button" class="btn" disabled=${!editable}
        onClick=${() => store.editNote(ticket.id, row.id)}><${Icon} name="pencil" size=${16} /><span>Edit</span></button>
      <button id=${shown ? "add-note" : undefined} type="button" class="btn btn-quiet notes-add" onClick=${() => store.editNote(ticket.id)}>
        <${Icon} name="plus" size=${16} /><span>Note</span></button>
    </div>`}
  </div>`;
}

// A picked picture as the sheet shows it (T-629): its bitmap drawn on a
// canvas, which the relay's CSP admits where a `blob:` image is refused.
function Thumb({ bitmap }) {
  const ref = useRef();
  useLayoutEffect(() => {
    const c = ref.current;
    if (!c || !bitmap) return;
    c.width = bitmap.width;
    c.height = bitmap.height;
    c.getContext("2d").drawImage(bitmap, 0, 0);
  }, [bitmap]);
  return html`<canvas ref=${ref} class="note-pic-thumb" aria-hidden="true"></canvas>`;
}

// A draft's pictures and the button that picks more: the note sheet's
// (T-629) and the New ticket sheet's details (T-670). `onAdd` answers
// where the next picture goes, which `caret` keeps.
export function PictureBar({ id, can, draft, caret, onAdd, onRemove }) {
  if (!can && !draft.pictures.length) return null;
  const busy = !!draft.sending;
  return html`<div class="note-pics">
    ${draft.pictures.length > 0 && html`<ul class="note-pic-list" aria-label="Pictures">
      ${draft.pictures.map((p) => html`<li key=${p.n} class="note-pic">
        <${Thumb} bitmap=${p.thumb} />
        <span class="note-pic-name">Image #${p.n}</span>
        <button type="button" class="note-pic-drop" aria-label=${`Remove Image #${p.n}`} disabled=${busy}
          onClick=${() => onRemove(p.n)}><${Icon} name="x" size=${14} /></button>
      </li>`)}
    </ul>`}
    ${can && html`<label class="btn btn-quiet note-pic-add" data-busy=${String(busy || draft.reading > 0)}>
      <input id=${id} type="file" accept="image/*" multiple class="sr-only" disabled=${busy}
        onChange=${async (e) => {
          const files = [...e.currentTarget.files];
          e.currentTarget.value = "";
          const at = await onAdd(files, caret.current);
          if (at !== undefined) caret.current = at;
        }} />
      <${Icon} name="image" size=${17} /><span>${draft.reading > 0 ? "Reading…" : "Picture"}</span></label>`}
  </div>`;
}

// A pasted image goes in as a picked one; anything else pastes as text.
export function pastePictures(e, can, onAdd, caret) {
  const files = [...(e.clipboardData?.files || [])].filter((f) => f.type.startsWith("image/"));
  if (!files.length || !can) return;
  e.preventDefault();
  onAdd(files, e.currentTarget.selectionStart).then((at) => {
    if (at !== undefined) caret.current = at;
  });
}

export function NoteSheet({ store }) {
  const ref = useRef();
  // Where the person last left the cursor in this draft's words, which a
  // picked picture goes to; none yet, and it goes at the end.
  const caret = useRef();
  const opened = useRef();
  const draft = store.noteDraft;
  if (opened.current !== draft) {
    opened.current = draft;
    caret.current = undefined;
  }
  const mark = (e) => {
    caret.current = e.currentTarget.selectionStart;
  };
  useLayoutEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (draft && !dialog.open) dialog.showModal();
    if (!draft && dialog.open) dialog.close();
  });
  if (!draft) return html`<dialog id="note-sheet" class="compose" ref=${ref}></dialog>`;
  const bytes = new TextEncoder().encode(draft.text).length;
  const heading = draft.description ? "Description" : draft.note ? "Edit note" : store.noteBook().entry(draft.ticket)?.rows.length ? "New note" : "Description";
  const away = !store.live;
  return html`<dialog id="note-sheet" class="compose note-sheet" ref=${ref} aria-labelledby="note-heading"
      onCancel=${(e) => {
        e.preventDefault();
        store.closeNoteSheet();
      }}
      onClose=${() => store.closeNoteSheet()}
      onClick=${(e) => {
        if (e.target === e.currentTarget) store.closeNoteSheet();
      }}>
    <form class="compose-form" onSubmit=${(e) => {
      e.preventDefault();
      store.saveNote();
    }}>
      <header class="compose-head">
        <button type="button" class="btn btn-quiet" onClick=${() => store.closeNoteSheet()}>Cancel</button>
        <h2 id="note-heading">${heading} · ${draft.key}</h2>
        <button id="save-note" type="submit" class="btn btn-quiet compose-send-top"
          disabled=${!draft.text.trim() || !!draft.sending || draft.reading > 0}>Save</button>
      </header>
      <div class="compose-body note-sheet-body">
        ${away && html`<p class="compose-dest"><${Icon} name="moon" size=${16} /><span>Saves when your terminal is back.</span></p>`}
        <label class="field note-field"><span class="sr-only">Note</span><textarea id="note-text" dir="auto" autofocus
          readOnly=${!!draft.sending} onClick=${mark} onKeyUp=${mark} onSelect=${mark}
          aria-invalid=${String(bytes > NOTE_MAX_BYTES)} placeholder="Markdown works."
          value=${draft.text} onInput=${(e) => {
            mark(e);
            store.setNoteText(e.currentTarget.value);
          }}
          onPaste=${(e) => pastePictures(e, store.canAddPictures, (files, at) => store.addPictures(files, at), caret)}
          onKeyDown=${(e) => {
            if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && !e.isComposing) {
              e.preventDefault();
              e.currentTarget.form.requestSubmit();
            }
          }}></textarea></label>
      </div>
      <footer class="compose-foot note-sheet-foot">
        <${PictureBar} id="note-picture" can=${store.canAddPictures} draft=${draft} caret=${caret}
          onAdd=${(files, at) => store.addPictures(files, at)} onRemove=${(n) => store.removePicture(n)} />
        ${draft.sending && html`<p class="note-sending" role="status"><${Tick} state="clock" /><span>${draft.sending}</span></p>`}
        ${draft.error && html`<p class="compose-error" role="alert">${draft.error}</p>`}
        <div class="note-sheet-row">
          ${draft.note && !draft.description
            ? html`<button id="delete-note" type="button" class="btn btn-quiet btn-danger" onClick=${() => store.deleteNote()}>
                <${Icon} name="trash" size=${17} /><span>${draft.confirmDelete ? "Delete for good" : "Delete"}</span></button>`
            : html`<span></span>`}
          <span class="note-bytes" data-over=${String(bytes > NOTE_MAX_BYTES)}>${(bytes / 1024).toFixed(1)} of 32 KiB</span>
        </div>
      </footer>
    </form>
  </dialog>`;
}
