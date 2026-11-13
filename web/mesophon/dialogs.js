// Structured host dialogs. Payloads come from the daemon and render as text
// nodes, never HTML. Native outcomes remain the host's to observe.
import { html, useLayoutEffect, useRef, useState } from "./html.js";
import { Icon } from "./icons.js";
import { Markdown } from "./markdown.js";
import { answerBusy } from "./sessions.js";

export const DIALOG_OMITTED = "This dialog is too large to show here. Use the ticket’s Terminal tab to answer.";

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

// The option this page answered with (T-1219), marked on its card until the
// dialog is gone.
export const pickedOption = (entry, dialog, index) =>
  entry?.answer?.pick?.request === dialog?.request && entry.answer.pick.index === index;

// The whole request, for the ticket page (T-1208): a command as it would
// run, then every other field (T-1218), so nothing the approval covers is
// left off the card: a Codex command's `cwd` and `reason`, an MCP tool's
// arguments beside its `command`. Each field is its own name and value,
// never one text an agent's words could add a field to; a value that is no
// string carries its kind and is drawn as JSON, so `false` and "false"
// differ.
const KINDS = { boolean: "true or false", number: "number" };
const kindOf = (value) =>
  value === null ? "empty" : Array.isArray(value) ? "list" : KINDS[typeof value] || (typeof value === "object" ? "fields" : "");
export function requestFields(permission) {
  const input = permission?.input;
  if (!input || typeof input !== "object" || Array.isArray(input))
    return { fields: [{ name: "input", kind: kindOf(input ?? null), value: JSON.stringify(input ?? null, null, 2) }] };
  const command = typeof input.command === "string" ? input.command : undefined;
  const fields = Object.entries(input)
    .filter(([name]) => command === undefined || name !== "command")
    .map(([name, value]) => typeof value === "string"
      ? { name, kind: "", value }
      : { name, kind: kindOf(value), value: JSON.stringify(value, null, 2) });
  return { ...(command === undefined ? {} : { command }), fields };
}

// Each box is a `CodeBox` (T-1219): no field of a permission, on its card or
// its ticket page, is cut short unseen above Approve once.
export function RequestFields({ permission, cls = "attention-code" }) {
  const { command, fields } = requestFields(permission);
  return html`${command !== undefined && html`<${CodeBox} key=${`${permission.request}:command`} cls=${cls} text=${command} what="command" />`}
    ${fields.length > 0 && html`<dl class="request-fields">${fields.map(({ name, kind, value }) => html`<div class="request-field" key=${name}>
      <dt class="request-name">${name}${kind && html`<span class="request-kind"> · ${kind}</span>`}</dt>
      <dd><${CodeBox} key=${`${permission.request}:${name}`} cls=${cls} text=${value} what="field" /></dd>
    </div>`)}</dl>`}`;
}

// Whether the ticket's agent stands on a dialog this page answers on a card
// right now (T-1218): the folder trust, a permission whose remote request
// has not expired, a plan, a question the card answers. Only there does the
// host refuse the Terminal tab's keys; at any other stop they go in.
export function cardAnswers(store, agent, now = Date.now()) {
  if (!agent) return false;
  if (agent.trust) return true;
  if (agent.permission && now < agent.permission.expires_at) return true;
  const dialog = agent.dialog;
  if (!dialog) return false;
  // The host's own test where it says (T-1218): it is per provider, and a
  // Codex batch is the pane's however the page would draw it.
  if (typeof agent.card_answers === "boolean") return agent.card_answers;
  if (dialog.kind === "plan") return true;
  return answerable(dialog) || (measured(dialog) && store.answersWhole);
}

export function Attention({ store, ticket, entry, live }) {
  const permission = ticket?.agent?.permission;
  const dialog = ticket?.agent?.dialog;
  const trust = ticket?.agent?.trust;
  const omitted = ticket?.agent?.dialog_omitted;
  if (!permission && !dialog && !trust && !omitted) return html`<section id="attention" hidden></section>`;
  // An answer on its way, never a prompt queued for the turn (T-568).
  const busy = answerBusy(entry);
  const off = !live || busy;
  const send = (body) =>
    store.sendInteraction({ ...body, ticket: ticket.id, session: ticket.agent.session }, entry);
  if (trust) return html`<${TrustCard} ticket=${ticket} trust=${trust} off=${off} send=${send} />`;
  if (permission) {
    const expired = Date.now() >= permission.expires_at;
    return html`<section id="attention" class="attention" aria-label="Agent needs your answer">
      <h3 class="attention-head"><${Icon} name="shield" size=${18} cls="attn-ink" /><span>Permission · ${permission.tool}</span></h3>
      <${RequestFields} permission=${permission} cls="attention-code" />
      <p class="attention-note">${expired
        ? "This request expired here: answer it at your terminal."
        : "Here or at the terminal, whichever answers first wins."}</p>
      <div class="attention-actions">
        <button type="button" class="btn" disabled=${off || expired}
          onClick=${() => send({ op: "permission", request: permission.request, decision: "deny" })}>Deny</button>
        <button type="button" class="btn btn-attn" disabled=${off || expired}
          onClick=${() => send({ op: "permission", request: permission.request, decision: "allow" })}>Approve once</button>
      </div>
    </section>`;
  }
  if (omitted && !dialog)
    return html`<section id="attention" class="attention" aria-label="Agent needs your answer">
      <p class="attention-note">${DIALOG_OMITTED}</p>
    </section>`;
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
  // A host that says its card cannot answer this dialog (T-1218) leaves it
  // to the pane: no form, no option that sends.
  const pane = ticket.agent.card_answers === false;
  const supported = !pane && answerable(dialog);
  // Offline it is drawn disabled; a live host too old for `answers` leaves
  // the dialog to the pane.
  if (!pane && !supported && measured(dialog) && (!live || store.answersWhole))
    return html`<${QuestionsForm} store=${store} dialog=${dialog} entry=${entry} off=${off} send=${send} />`;
  const draft = entry?.dialogDraft?.request === dialog.request ? entry.dialogDraft.text : "";
  return html`<section id="attention" class="attention" aria-label="Agent needs your answer">
    ${dialog.questions.map((question) => html`<div class="attention-question">
      <h3 class="attention-head"><${Icon} name="bell" size=${18} cls="attn-ink" /><span>${question.question}</span></h3>
      <div class="options">
        ${question.options.map((option, index) => {
          const described = option.description ? `att-${dialog.request}-${index}` : undefined;
          const picked = pickedOption(entry, dialog, index);
          return html`<button type="button" class=${`option${picked ? " option-picked" : ""}`} aria-label=${option.label} aria-describedby=${described}
            aria-pressed=${picked ? "true" : undefined} ...${answer({ answer: "choice", index }, supported)}>
            <span class="option-label">${option.label}${picked && html`<${Icon} name="check" size=${15} width=${2.4} />`}</span>
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
      : html`<p class="attention-note">${store.takesKeys
          ? "This card cannot answer it: the Terminal tab’s keys can, or your terminal."
          : "This card cannot answer it: answer it at your terminal."}</p>`}
  </section>`;
}

// The folder trust dialog a harness shows before it starts (T-844), with its
// one answer. The host presses the dialog's own Yes in the pane, the screen
// read before every key, as the person would at the keyboard; the brief
// then goes in by itself. "No, exit" ends the agent and stays the pane's.
function TrustCard({ ticket, trust, off, send }) {
  const harness = ticket.agent.provider === "codex" ? "Codex" : "Claude Code";
  return html`<section id="attention" class="attention" aria-label="Agent needs your answer">
    <h3 class="attention-head"><${Icon} name="shield" size=${18} cls="attn-ink" /><span>Trust this folder</span></h3>
    ${trust.folder && html`<${CodeBox} key=${trust.request} cls="attention-code" text=${trust.folder} what="folder" />`}
    <p class="attention-note">${harness} asks before it works in a folder for the first time. You started this agent from mesimon; accepting presses Yes in its terminal, as you would there, and it goes on with its brief. To decline, answer in the terminal: No ends the agent.</p>
    <div class="attention-actions"><${TrustAccept} trust=${trust} off=${off} send=${send} /></div>
  </section>`;
}

// The one accept of a trust dialog (T-844): the ticket's card and the Now
// list press the same Yes, with the same frame.
export function TrustAccept({ trust, off, send }) {
  return html`<button type="button" class="btn btn-attn" disabled=${off}
    onClick=${() => send({ op: "dialog", request: trust.request, response: { answer: "accept" } })}>Yes, I trust this folder</button>`;
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

// What a person approves is never cut short unseen (T-1219): a box too small
// for its text fades at its foot and says so, and one press shows the whole
// of it, on the card that asks. Measured after each layout and on resize, so
// a larger text size or a turned phone says so too.
export function CodeBox({ text, cls, what = "text" }) {
  const box = useRef();
  const [cut, setCut] = useState(false);
  const [whole, setWhole] = useState(false);
  useLayoutEffect(() => {
    const node = box.current;
    if (!node) return;
    const measure = () => setCut(!whole && node.scrollHeight > node.clientHeight + 1);
    measure();
    if (typeof ResizeObserver !== "function") return;
    const watch = new ResizeObserver(measure);
    watch.observe(node);
    return () => watch.disconnect();
  }, [text, whole]);
  return html`<div class="code-box">
    <pre ref=${box} class=${`${cls}${whole ? " whole" : cut ? " cut" : ""}`} tabindex=${cut ? "0" : undefined}>${text}</pre>
    ${(cut || whole) && html`<button type="button" class="code-more" aria-expanded=${String(whole)} onClick=${() => setWhole(!whole)}>
      <${Icon} name="chevronDown" size=${14} width=${2.4} cls=${whole ? "flip" : ""} /><span>${whole ? "Show less" : `Only part shown · show the whole ${what}`}</span></button>`}
  </div>`;
}
