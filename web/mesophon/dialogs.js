// Structured host dialogs. Payloads come from the daemon and render as text
// nodes, never HTML. Native outcomes remain the host's to observe.
import { html } from "./html.js";
import { Icon } from "./icons.js";
import { Markdown } from "./markdown.js";
import { answerBusy } from "./sessions.js";

// The single-choice question a card answers in place, one tap an answer.
export const answerable = (dialog) =>
  dialog?.kind === "questions" && dialog.questions.length === 1 && !dialog.questions[0].multiSelect;

// A dialog the host's walk was measured for (T-571): up to four questions,
// no two reading alike, since the host tells them apart by their words.
export const measured = (dialog) => {
  if (dialog?.kind !== "questions") return false;
  const words = dialog.questions.map((q) => String(q.question).replace(/\s+/g, ""));
  return words.length >= 1 && words.length <= 4 && !words.includes("") && new Set(words).size === words.length;
};

// The form's state for one dialog (T-571): per question the options picked,
// in the order they were, and the words typed. A new request starts afresh.
export function dialogForm(entry, dialog) {
  if (entry.dialogForm?.request !== dialog.request)
    entry.dialogForm = {
      request: dialog.request,
      picks: dialog.questions.map(() => []),
      texts: dialog.questions.map(() => ""),
    };
  return entry.dialogForm;
}

// The answer the form holds: per question its words when any are typed, else
// its ticked options (several) or its picked option (one). `null` until every
// question has one.
export function formAnswers(dialog, form) {
  const answers = dialog.questions.map((question, i) => {
    const text = (form?.texts?.[i] || "").trim();
    if (text) return { answer: "text", text };
    const picks = form?.picks?.[i] || [];
    if (!picks.length) return null;
    return question.multiSelect
      ? { answer: "choices", indices: [...picks].sort((a, b) => a - b) }
      : { answer: "choice", index: picks[0] };
  });
  return answers.every(Boolean) ? answers : null;
}

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
  // An answer on its way, never a prompt queued for the turn (T-568).
  const busy = answerBusy(entry);
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
  // Offline it is drawn disabled; a live host too old for `answers` leaves
  // the dialog to the pane.
  if (!supported && measured(dialog) && (!live || store.answersWhole))
    return html`<${QuestionsForm} store=${store} dialog=${dialog} entry=${entry} off=${off} send=${send} />`;
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

// Every question of a batch, or one that takes several choices, answered
// whole (T-571): radios for one choice, ticks for several, words in place of
// either, and one Submit. The host walks the pane's dialog tab by tab and
// presses its own Submit once; the receipt is the agent's hook, as for one
// question. The form keeps what was picked through a retry.
function QuestionsForm({ store, dialog, entry, off, send }) {
  const form = dialogForm(entry, dialog);
  const answers = formAnswers(dialog, form);
  const pick = (q, index) => {
    const picks = form.picks[q];
    if (!dialog.questions[q].multiSelect) form.picks[q] = [index];
    else if (picks.includes(index)) picks.splice(picks.indexOf(index), 1);
    else picks.push(index);
    store.emit();
  };
  const several = dialog.questions.length > 1;
  return html`<section id="attention" class="attention" aria-label="Agent needs your answer">
    ${dialog.questions.map((question, q) => {
      const multi = !!question.multiSelect;
      const head = `att-${dialog.request}-q${q}`;
      const typed = !!form.texts[q].trim();
      return html`<div class="attention-question" data-question=${q}>
        <h3 class="attention-head" id=${head}><${Icon} name="bell" size=${18} cls="attn-ink" /><span dir="auto">${question.question}</span></h3>
        ${several || multi ? html`<p class="attention-note">${multi ? "Tick any that apply." : "Pick one."}</p>` : null}
        <div class="options" role=${multi ? "group" : "radiogroup"} aria-labelledby=${head}>
          ${question.options.map((option, index) => {
            const described = option.description ? `${head}-${index}` : undefined;
            const on = !typed && form.picks[q].includes(index);
            return html`<button type="button" class="option option-pick" role=${multi ? "checkbox" : "radio"}
              aria-checked=${String(on)} aria-label=${option.label} aria-describedby=${described}
              disabled=${off || typed} onClick=${() => pick(q, index)}>
              <span class="option-mark" aria-hidden="true"></span>
              <span class="option-text">
                <span class="option-label">${option.label}</span>
                ${described && html`<span class="option-desc" id=${described}>${option.description}</span>`}
              </span>
            </button>`;
          })}
        </div>
        <label class="field">${multi ? "Or in your own words, in place of the ticks" : "Or in your own words"}<input type="text"
          maxlength="1000" autocomplete="off" value=${form.texts[q]} disabled=${off}
          onInput=${(e) => {
            form.texts[q] = e.currentTarget.value;
            store.emit();
          }} /></label>
      </div>`;
    })}
    <p class="attention-note">${several
      ? "Every question goes in one Submit, as the terminal sends them."
      : "Your ticks go in one Submit."}</p>
    <div class="attention-actions">
      <button type="button" class="btn" disabled=${off}
        onClick=${() => send({ op: "dialog", request: dialog.request, response: { answer: "reject" } })}>Decline ${several ? "questions" : "question"}</button>
      <button type="button" class="btn btn-attn" disabled=${off || !answers}
        onClick=${() => answers && send({ op: "dialog", request: dialog.request, response: { answer: "answers", answers } })}>Submit ${several ? "answers" : "answer"}</button>
    </div>
  </section>`;
}
