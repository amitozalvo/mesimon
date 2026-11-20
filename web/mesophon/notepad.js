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
import { extract } from "./summary.js";

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

// What a note's row says of its body: its first words past a heading, the
// desk's picture marks counted, never drawn.
const PICTURE_MARK = /!?\[[^\]]*\]\(mesimon-attachment:[^)]*\)/g;
function previewOf(body) {
  const pictures = (body.match(PICTURE_MARK) || []).length;
  const text = body.replace(PICTURE_MARK, "");
  // The first four lines that say something; the rest of a long note is
  // never split or trimmed.
  const kept = [];
  for (let from = 0; from <= text.length && kept.length < 4; ) {
    let to = text.indexOf("\n", from);
    if (to < 0) to = text.length;
    const line = text.slice(from, to).trim();
    if (line && !/^#{1,6}(\s|$)/.test(line) && !/^(```|~~~)/.test(line)) kept.push(line);
    from = to + 1;
  }
  const words = kept.join(" ").replace(/[*_`>#]+/g, "").slice(0, 240);
  return { words, pictures };
}

// The Notes tab (T-1208): the ticket's Summary boxes in one card, ticked
// here as the desk ticks them, then every note as a row (its name, who and
// when, its first words) that opens its pane. A box of a note this browser
// has an edit of on its way waits for that edit.
export function NotesList({ store, ticket, panes }) {
  const board = store.active.pin.board;
  const book = store.noteBook();
  const away = !store.live;
  const writes = store.canWriteNotes;
  // Each row with its body, read once: a note's is this browser's edit on
  // its way, else the host's; a fresh note's is its own text.
  const rows = panes
    .map((pane, at) => {
      if (pane.kind === "fresh") return { pane, at, pending: pane.item, body: pane.item.text };
      if (pane.kind !== "note") return { pane, at };
      const pending = store.noteMail.pending(board, ticket.id, pane.row.id);
      return { pane, at, pending, body: pending ? pending.text : book.body(ticket.id, pane.row.id) };
    })
    .filter(({ pane }) => pane.kind !== "transcript");
  const boxes = [];
  for (const { pane, pending, body } of rows) {
    if (pane.kind !== "note") continue;
    for (const box of extract(body || "")) boxes.push({ ...box, note: pane.row.id, busy: !!pending });
  }
  const done = boxes.filter((b) => b.done).length;
  return html`<div id="notes-list" class="notes-list">
    ${boxes.length > 0 && html`<section id="summary-card" class="summary-card" aria-label="Summary">
      <h3><span>Summary</span><span class="summary-count">${done}/${boxes.length}</span></h3>
      ${boxes.map((box) => html`<button type="button" class=${`summary-box${box.done ? " done" : ""}`} key=${`${box.note}:${box.line}`}
        role="checkbox" aria-checked=${String(box.done)} disabled=${!writes || box.busy}
        onClick=${() => store.tickSummary(ticket.id, box.note, box.line)}>
        <span class="summary-mark" aria-hidden="true">${box.done && html`<${Icon} name="check" size=${13} width=${3} />`}</span>
        <span class="summary-text" dir="auto">${box.text}</span></button>`)}
    </section>`}
    <div class="note-rows">${rows.map(({ pane, at, pending, body }) => {
      if (pane.kind === "unread")
        return html`<p class="notes-empty" key="unread">${pane.count} ${pane.count === 1 ? "note" : "notes"} · ${away ? "needs your terminal" : "loading…"}</p>`;
      const fresh = pane.kind === "fresh";
      const said = body === undefined ? undefined : previewOf(body);
      const name = fresh ? pane.item.name || "New note" : pane.title;
      return html`<button type="button" class="note-row" key=${pane.key} data-note=${fresh ? undefined : pane.row.id}
        onClick=${() => store.showPane(ticket.id, at)}>
        <span class="note-row-head"><b dir="auto">${name}</b><span class="note-row-by">${fresh
          ? html`<${PendingMark} item=${pane.item} /><span>on its way</span>`
          : html`${pending && html`<${PendingMark} item=${pending} />`}<span>${pane.row.by} · ${ago(pane.row.at)}</span>`}</span></span>
        ${said === undefined
          ? html`<span class="note-row-words">${away ? "Needs your terminal." : "Loading…"}</span>`
          : html`${said.words && html`<span class="note-row-words" dir="auto">${said.words}</span>`}
            ${said.pictures > 0 && html`<span class="note-row-pics"><${Icon} name="image" size=${13} /><span>${said.pictures} ${said.pictures === 1 ? "picture" : "pictures"}</span></span>`}`}
      </button>`;
    })}</div>
    ${writes && html`<div class="notes-list-foot">
      <button id="list-add-note" type="button" class="btn" onClick=${() => store.editNote(ticket.id)}>
        <${Icon} name="plus" size=${16} /><span>New note</span></button>
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
    <${PictureList} draft=${draft} onRemove=${onRemove} />
    ${can && html`<${PicturePick} id=${id} draft=${draft} caret=${caret} onAdd=${onAdd}>
      <span>${draft.reading > 0 ? "Reading…" : "Picture"}</span><//>`}
  </div>`;
}

// The pictures a draft holds, each with its number and a remove button.
// While they go up (T-1312) each wears a ring that fills as its pieces are
// taken, in place of the remove button, and says nothing under the field.
export function PictureList({ draft, onRemove }) {
  if (!draft.pictures.length) return null;
  const busy = !!draft.sending;
  return html`<ul class="note-pic-list" aria-label=${draft.pictures.some(p => p.name) ? "Attachments" : "Pictures"}>
    ${draft.pictures.map((p) => html`<li key=${p.n} class="note-pic" data-up=${p.up === undefined ? undefined : String(p.up >= 1 ? "done" : p.up > 0 ? "going" : "waiting")}>
      ${p.name ? html`<span class="file-thumb"><${Icon} name="file" size=${22} /></span>` : html`<${Thumb} bitmap=${p.thumb} />`}
      ${p.up !== undefined && html`<${Going} label=${p.label || `Image #${p.n}`} up=${p.up} />`}
      <span class="note-pic-name" title=${p.name}>${p.label || `Image #${p.n}`}</span>
      ${p.name && html`<span class="note-pic-name">${p.bytes.length < 1024 ? p.bytes.length + " B" : Math.ceil(p.bytes.length / 1024) + " KiB"}</span>`}
      ${p.up === undefined && html`<button type="button" class="note-pic-drop" aria-label=${`Remove ${p.label || `Image #${p.n}`}`} disabled=${busy}
        onClick=${() => onRemove(p.n)}><${Icon} name="x" size=${14} /></button>`}
    </li>`)}
  </ul>`;
}

// How much of a picture went, as a ring over its thumbnail: the track, and
// the arc from the top, clockwise. A picture waiting its turn shows the
// track alone.
const RING = 2 * Math.PI * 11;
function Going({ label, up }) {
  const done = Math.max(0, Math.min(1, up));
  return html`<span class="note-pic-going" role="progressbar" aria-label=${`Sending ${label}`}
      aria-valuemin="0" aria-valuemax="100" aria-valuenow=${Math.round(done * 100)}>
    <svg viewBox="0 0 28 28" width="28" height="28" aria-hidden="true">
      <circle class="ring-track" cx="14" cy="14" r="11" />
      <circle class="ring-arc" cx="14" cy="14" r="11" transform="rotate(-90 14 14)"
        stroke-dasharray=${RING.toFixed(2)} stroke-dashoffset=${(RING * (1 - done)).toFixed(2)} />
    </svg>
  </span>`;
}

// The file input that picks pictures into a draft, dressed as a button:
// the picture icon and whatever `children` says beside it. A picker the
// person dismisses fires a `cancel` that bubbles, and every sheet it sits
// in takes a `cancel` for Escape and closes (T-1307): it stops here.
export function PicturePick({ id, draft, caret, onAdd, label, capture, anyFile, children }) {
  const busy = !!draft.sending;
  return html`<label class="btn btn-quiet note-pic-add" data-busy=${String(busy || draft.reading > 0)}
    aria-label=${label}>
    <input id=${id} type="file" accept=${anyFile ? undefined : "image/*"} multiple=${!capture} capture=${capture} class="sr-only" disabled=${busy}
      onCancel=${(e) => e.stopPropagation()}
      onChange=${async (e) => {
        const files = [...e.currentTarget.files];
        e.currentTarget.value = "";
        const at = await onAdd(files, caret.current);
        if (at !== undefined) caret.current = at;
      }} />
    <${Icon} name=${anyFile ? "file" : "image"} size=${17} />${children}</label>`;
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
          onPaste=${(e) => pastePictures(e, store.offersNotePictures, (files, at) => store.addPictures(files, at), caret)}
          onKeyDown=${(e) => {
            if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && !e.isComposing) {
              e.preventDefault();
              e.currentTarget.form.requestSubmit();
            }
          }}></textarea></label>
      </div>
      <footer class="compose-foot note-sheet-foot">
        <${PictureBar} id="note-picture" can=${store.offersNotePictures} draft=${draft} caret=${caret}
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
