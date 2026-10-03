// A ticket's notes on its page (T-532): the description and the note rows
// under the title (past two, the latest and a sheet of all, T-627), a note
// read in place of the agent's output, and the edit sheet. Bodies render as
// text nodes through `Markdown`, never as HTML.
import { html, useLayoutEffect, useRef } from "./html.js";
import { Icon, Tick } from "./icons.js";
import { Markdown } from "./markdown.js";
import { NOTE_MAX_BYTES, ago } from "./notes.js";

const clock = (at) => new Date(at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
const waitTick = { sending: "clock", local: "clock", relay: "one" };
// Past this many notes besides the description, the page lists only the
// latest, and All opens the rest in a sheet (T-627).
const ROWS = 2;

// What one note's own edit, if any, says about it.
function PendingMark({ item }) {
  if (!item) return null;
  if (waitTick[item.status]) return html`<${Tick} state=${waitTick[item.status]} />`;
  return html`<span class="note-flag">Not saved</span>`;
}

function NoteRowButton({ store, ticket, row, pending, latest = false }) {
  return html`<li><button type="button" class="note-row" onClick=${() => store.openNote(ticket.id, row.id)}>
    <${Icon} name="file" size=${18} />
    <span class="note-row-text"><span class="note-name" dir="auto">${row.name || "Untitled"}</span>
      <span class="note-meta">${latest ? "latest · " : ""}${row.by} · ${ago(row.at)}</span></span>
    <${PendingMark} item=${pending} />
    <${Icon} name="chevronRight" size=${16} cls="note-go" />
  </button></li>`;
}

// The description under the title, with no heading of its own (T-633): the
// whole of it is one press that opens it in the reader, where Edit is. It
// fades out at the foot exactly when the clamp hides some of it, measured,
// since a line count cannot know the width.
function Description({ body, onOpen }) {
  const ref = useRef();
  useLayoutEffect(() => {
    const box = ref.current;
    if (!box) return;
    const measure = () => {
      box.dataset.clipped = String(box.scrollHeight > box.clientHeight + 1);
    };
    measure();
    const watch = new ResizeObserver(measure);
    watch.observe(box);
    if (box.firstElementChild) watch.observe(box.firstElementChild);
    return () => watch.disconnect();
  });
  return html`<div class="notes-description" ref=${ref}>
    <${Markdown} text=${body} />
    <button id="open-description" type="button" class="notes-description-open" aria-label="Open the description"
      onClick=${onOpen}></button>
  </div>`;
}

export function NotesCard({ store, ticket }) {
  if (!ticket || !store.notesHere) return null;
  const board = store.active.pin.board;
  const entry = store.noteBook().entry(ticket.id);
  const rows = entry?.rows || [];
  const count = rows.length || ticket.notes || 0;
  const writes = store.canWriteNotes;
  const fresh = store.noteMail.fresh(board, ticket.id);
  if (!count && !fresh.length && !writes) return null;
  const description = rows[0];
  const body = description && store.noteBook().body(ticket.id, description.id);
  const away = !store.live;
  const asOf = away && entry?.at ? `as of ${clock(entry.at)}` : "";
  const awake = ticket.agent && ticket.agent.state !== "sleeping";
  const descPending = description && store.noteMail.pending(board, ticket.id, description.id);
  const others = rows.slice(1);
  // Past ROWS, the latest written stays, and any whose own edit is on its
  // way or did not save, so that is never out of sight.
  const pendingOf = (row) => store.noteMail.pending(board, ticket.id, row.id);
  const brief = others.length > ROWS;
  const latest = brief ? others.reduce((a, b) => ((b.at || 0) >= (a.at || 0) ? b : a)) : undefined;
  const shown = brief ? others.filter((row) => row === latest || pendingOf(row)) : others;
  return html`<section id="notes" class="notes-card" aria-label="Description and notes" data-awake=${String(!!awake)}>
    ${description
      ? html`${(asOf || descPending) && html`<div class="notes-meta">
          <span class="label">${asOf}</span><${PendingMark} item=${descPending} />
        </div>`}
        ${body !== undefined
          ? html`<${Description} body=${body} onOpen=${() => store.openNote(ticket.id, description.id)} />`
          : html`<p class="notes-empty">${away ? "Needs your terminal." : "Loading…"}</p>`}`
      : !count && writes && !fresh.length &&
        html`<button id="add-description" type="button" class="btn btn-quiet notes-add-first"
          onClick=${() => store.editNote(ticket.id)}><${Icon} name="plus" size=${16} /><span>Add a description</span></button>`}
    ${!description && count > 0 && html`<p class="notes-empty">${count} ${count === 1 ? "note" : "notes"} · ${away ? "needs your terminal" : "loading…"}</p>`}
    ${(others.length > 0 || fresh.length > 0 || (description && writes)) && html`<div class="notes-list">
      <div class="notes-head">
        <h3 class="label">Notes${others.length ? ` · ${others.length}` : ""}</h3>
        ${writes && description && html`<button id="add-note" type="button" class="btn btn-quiet notes-add"
          onClick=${() => store.editNote(ticket.id)}><${Icon} name="plus" size=${16} /><span>Note</span></button>`}
        ${brief && html`<button id="all-notes" type="button" class="btn btn-quiet notes-add" aria-haspopup="dialog"
          onClick=${() => store.openNotesSheet(ticket.id)}><span>All</span><${Icon} name="chevronRight" size=${16} /></button>`}
      </div>
      <ul class="note-rows">
        ${shown.map((row) => html`<${NoteRowButton} key=${row.id} store=${store} ticket=${ticket} row=${row}
          pending=${pendingOf(row)} latest=${row === latest} />`)}
        ${fresh.map((item) => html`<li key=${item.id}><div class="note-row note-row-fresh">
          <${Icon} name="file" size=${18} />
          <span class="note-row-text"><span class="note-name" dir="auto">${item.name || "New note"}</span>
            <span class="note-meta">${item.status === "stale" || item.status === "rejected" || item.status === "unknown"
              ? item.message || "Not saved"
              : "New"}</span></span>
          <${PendingMark} item=${item} />
          ${["local", "relay"].includes(item.status)
            ? html`<button type="button" class="btn btn-quiet" onClick=${() => store.retractNote(item.id)}>Unsend</button>`
            : !waitTick[item.status] &&
              html`<button type="button" class="btn btn-quiet" onClick=${() => store.dropNote(item.id)}>Discard</button>`}
        </div></li>`)}
      </ul>
    </div>`}
  </section>`;
}

// Every note of a ticket past ROWS (T-627), the description aside, in their
// order: a row opens the note in place of the page, as the card's rows do.
export function NotesSheet({ store }) {
  const ref = useRef();
  const ticket = store.notesSheet && store.board?.tickets.find((t) => t.id === store.notesSheet);
  useLayoutEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (ticket && !dialog.open) dialog.showModal();
    if (!ticket && dialog.open) dialog.close();
  });
  if (!ticket) return html`<dialog id="notes-sheet" class="compose" ref=${ref}></dialog>`;
  const board = store.active.pin.board;
  const others = (store.noteBook().entry(ticket.id)?.rows || []).slice(1);
  const writes = store.canWriteNotes;
  return html`<dialog id="notes-sheet" class="compose notes-sheet" ref=${ref} aria-labelledby="notes-heading"
      onCancel=${(e) => {
        e.preventDefault();
        store.closeNotesSheet();
      }}
      onClose=${() => store.closeNotesSheet()}
      onClick=${(e) => {
        if (e.target === e.currentTarget) store.closeNotesSheet();
      }}>
    <div class="compose-form">
      <header class="compose-head">
        ${writes
          ? html`<button id="notes-sheet-add" type="button" class="btn btn-quiet notes-add" onClick=${() => store.editNote(ticket.id)}>
              <${Icon} name="plus" size=${16} /><span>Note</span></button>`
          : html`<span></span>`}
        <h2 id="notes-heading">Notes · ${ticket.key}</h2>
        <button id="notes-done" type="button" class="btn btn-quiet compose-send-top" onClick=${() => store.closeNotesSheet()}>Done</button>
      </header>
      <ul class="note-rows notes-sheet-rows">
        ${others.map((row) => html`<${NoteRowButton} key=${row.id} store=${store} ticket=${ticket} row=${row}
          pending=${store.noteMail.pending(board, ticket.id, row.id)} />`)}
      </ul>
    </div>
  </dialog>`;
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

export function NoteReader({ store, ticket }) {
  const reading = store.reading;
  const board = store.active.pin.board;
  const entry = store.noteBook().entry(ticket.id);
  const rows = entry?.rows || [];
  const at = rows.findIndex((r) => r.id === reading.note);
  const row = rows[at];
  const pending = store.noteMail.pending(board, ticket.id, reading.note);
  const body = pending ? pending.text : store.noteBook().body(ticket.id, reading.note);
  const away = !store.live;
  const what = at === 0 ? "Description" : `Note ${at} of ${rows.length - 1}`;
  const editable = store.canWriteNotes && (body !== undefined || !!pending) && pending?.status !== "sending";
  return html`<article id="note-reader" class="note-reader" aria-labelledby="note-title">
    <header class="note-reader-head">
      <button id="note-back" type="button" class="btn btn-quiet note-back" onClick=${() => store.closeNote()}>
        <${Icon} name="back" size=${20} /><span>${ticket.key}</span></button>
      ${store.canWriteNotes && html`<button id="edit-note" type="button" class="btn" disabled=${!editable}
        onClick=${() => store.editNote(ticket.id, reading.note)}><${Icon} name="pencil" size=${17} /><span>Edit</span></button>`}
    </header>
    <div class="note-reader-body">
      <p class="label">${what}${row ? ` · ${row.by} · ${ago(row.at)}` : ""}${away && entry?.at ? ` · as of ${clock(entry.at)}` : ""}</p>
      <h2 id="note-title" tabindex="-1" class="sr-only">${row?.name || what}</h2>
      <${PendingStrip} store=${store} item=${pending} />
      ${body !== undefined
        ? html`<${Markdown} text=${body} />`
        : html`<p class="notes-empty">${row ? (away ? "Needs your terminal." : "Loading…") : "This note is gone."}</p>`}
    </div>
    ${rows.length > 1 && at >= 0 && html`<footer class="note-reader-foot">
      <button id="note-prev" type="button" class="btn btn-quiet" onClick=${() => store.walkNote(-1)}>
        <${Icon} name="back" size=${18} /><span>Previous</span></button>
      <button id="note-next" type="button" class="btn btn-quiet note-next" onClick=${() => store.walkNote(1)}>
        <span>Next</span><${Icon} name="chevronRight" size=${18} /></button>
    </footer>`}
  </article>`;
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

function Pictures({ store, draft, caret }) {
  const can = store.canAddPictures;
  if (!can && !draft.pictures.length) return null;
  const busy = !!draft.sending;
  return html`<div class="note-pics">
    ${draft.pictures.length > 0 && html`<ul class="note-pic-list" aria-label="Pictures">
      ${draft.pictures.map((p) => html`<li key=${p.n} class="note-pic">
        <${Thumb} bitmap=${p.thumb} />
        <span class="note-pic-name">Image #${p.n}</span>
        <button type="button" class="note-pic-drop" aria-label=${`Remove Image #${p.n}`} disabled=${busy}
          onClick=${() => store.removePicture(p.n)}><${Icon} name="x" size=${14} /></button>
      </li>`)}
    </ul>`}
    ${can && html`<label class="btn btn-quiet note-pic-add" data-busy=${String(busy || draft.reading > 0)}>
      <input id="note-picture" type="file" accept="image/*" multiple class="sr-only" disabled=${busy}
        onChange=${async (e) => {
          const files = [...e.currentTarget.files];
          e.currentTarget.value = "";
          const at = await store.addPictures(files, caret.current);
          if (at !== undefined) caret.current = at;
        }} />
      <${Icon} name="image" size=${17} /><span>${draft.reading > 0 ? "Reading…" : "Picture"}</span></label>`}
  </div>`;
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
          onPaste=${async (e) => {
            const files = [...(e.clipboardData?.files || [])].filter((f) => f.type.startsWith("image/"));
            if (!files.length || !store.canAddPictures) return;
            e.preventDefault();
            const at = await store.addPictures(files, e.currentTarget.selectionStart);
            if (at !== undefined) caret.current = at;
          }}
          onKeyDown=${(e) => {
            if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && !e.isComposing) {
              e.preventDefault();
              e.currentTarget.form.requestSubmit();
            }
          }}></textarea></label>
      </div>
      <footer class="compose-foot note-sheet-foot">
        <${Pictures} store=${store} draft=${draft} caret=${caret} />
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
