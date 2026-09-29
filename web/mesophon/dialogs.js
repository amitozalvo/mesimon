// Structured host dialogs. Payloads come from the daemon and render as text
// nodes, never HTML. Native outcomes remain the host's to observe.
import { html } from "./html.js";
import { Icon } from "./icons.js";

// A tiny markdown reading for plans: headings, lists and code spans become
// elements whose children are plain text. Anything else is a paragraph.
function Markdown({ text }) {
  const blocks = [];
  let list;
  for (const raw of text.split("\n")) {
    const line = raw.trimEnd();
    const item = line.match(/^\s*(?:[-*]|\d+[.)])\s+(.*)$/);
    if (item) {
      const ordered = /^\s*\d/.test(line);
      if (!list || list.ordered !== ordered) blocks.push((list = { ordered, items: [] }));
      list.items.push(item[1]);
      continue;
    }
    list = undefined;
    const heading = line.match(/^#{1,6}\s+(.*)$/);
    if (heading) blocks.push({ heading: heading[1] });
    else if (line.trim()) blocks.push({ text: line });
  }
  const inline = (text) =>
    text.split(/(`[^`]+`)/).map((part) =>
      part.length > 2 && part.startsWith("`") && part.endsWith("`")
        ? html`<code>${part.slice(1, -1)}</code>`
        : part,
    );
  return html`<div class="markdown">${blocks.map((b) =>
    b.heading !== undefined
      ? html`<h4>${inline(b.heading)}</h4>`
      : b.items
        ? b.ordered
          ? html`<ol>${b.items.map((i) => html`<li>${inline(i)}</li>`)}</ol>`
          : html`<ul>${b.items.map((i) => html`<li>${inline(i)}</li>`)}</ul>`
        : html`<p>${inline(b.text)}</p>`,
  )}</div>`;
}

// The single-choice question the host can answer by keys; anything else is
// answered in the pane.
export const answerable = (dialog) =>
  dialog?.kind === "questions" && dialog.questions.length === 1 && !dialog.questions[0].multiSelect;

// A one-line description of a tool request, for cards and headings.
export function requestSummary(permission) {
  const input = permission?.input;
  if (input && typeof input.command === "string") return input.command;
  if (input && typeof input.file_path === "string") return input.file_path;
  return JSON.stringify(input ?? {});
}

export function Attention({ store, ticket, entry, live }) {
  const permission = ticket?.agent?.permission;
  const dialog = ticket?.agent?.dialog;
  if (!permission && !dialog) return html`<section id="attention" hidden></section>`;
  const busy = !!entry?.receipt?.waiting;
  const off = !live || busy;
  const send = (body) =>
    store.sendInteraction({ ...body, ticket: ticket.id, session: ticket.agent.session }, entry);
  if (permission) {
    const expired = Date.now() >= permission.expires_at;
    return html`<section id="attention" class="attention" aria-label="Agent needs your answer">
      <h3 class="attention-head"><${Icon} name="shield" size=${18} cls="attn-ink" /><span>Permission · ${permission.tool}</span></h3>
      <pre class="attention-code">${JSON.stringify(permission.input, null, 2)}</pre>
      <p class="attention-note">${expired
        ? "Remote request expired. Check the pane."
        : "Approve this request once, or deny it. The terminal’s own dialog stays open too, and whichever answers first wins."}</p>
      <div class="attention-actions">
        <button type="button" class="btn" disabled=${off || expired}
          onClick=${() => send({ op: "permission", request: permission.request, decision: "deny" })}>Deny</button>
        <button type="button" class="btn btn-attn" disabled=${off || expired}
          onClick=${() => send({ op: "permission", request: permission.request, decision: "allow" })}>Approve once</button>
      </div>
    </section>`;
  }
  const answer = (response, enabled = true) => ({
    disabled: off || !enabled,
    onClick: () => send({ op: "dialog", request: dialog.request, response }),
  });
  if (dialog.kind === "plan")
    return html`<section id="attention" class="attention" aria-label="Agent needs your answer">
      <h3 class="attention-head"><${Icon} name="file" size=${18} cls="attn-ink" /><span>Review plan</span></h3>
      <div class="attention-plan"><${Markdown} text=${dialog.markdown} /></div>
      <p class="attention-note">Accepting keeps every edit asking for your approval. Never auto-accept.</p>
      <div class="attention-actions">
        <button type="button" class="btn" ...${answer({ answer: "reject" })}>Reject plan</button>
        <button type="button" class="btn btn-attn" ...${answer({ answer: "accept" })}>Accept · approve edits manually</button>
      </div>
    </section>`;
  const supported = answerable(dialog);
  const draft = entry?.dialogDraft?.request === dialog.request ? entry.dialogDraft.text : "";
  return html`<section id="attention" class="attention" aria-label="Agent needs your answer">
    ${dialog.questions.map((question) => html`<div class="attention-question">
      <h3 class="attention-head"><${Icon} name="bell" size=${18} cls="attn-ink" /><span>${question.question}</span></h3>
      <div class="options">
        ${question.options.map((option, index) => {
          const described = option.description ? `att-${dialog.request}-${index}` : undefined;
          return html`<button type="button" class="option" aria-label=${option.label} aria-describedby=${described}
            ...${answer({ answer: "choice", index }, supported)}>
            <span class="option-label">${option.label}</span>
            ${described && html`<span class="option-desc" id=${described}>${option.description}</span>`}
          </button>`;
        })}
      </div>
    </div>`)}
    ${supported
      ? html`<label class="field">Your answer<input type="text" maxlength="1000" autocomplete="off" value=${draft}
          onInput=${(e) => {
            entry.dialogDraft = { request: dialog.request, text: e.currentTarget.value };
          }} /></label>
        <div class="attention-actions">
          <button type="button" class="btn" ...${answer({ answer: "reject" })}>Decline question</button>
          <button type="button" class="btn btn-attn" disabled=${off}
            onClick=${() => {
              const text = entry?.dialogDraft?.request === dialog.request ? entry.dialogDraft.text : "";
              if (text.trim()) send({ op: "dialog", request: dialog.request, response: { answer: "text", text } });
            }}>Send answer</button>
        </div>`
      : html`<p class="attention-note">This dialog shape needs a local answer in the pane.</p>`}
  </section>`;
}
