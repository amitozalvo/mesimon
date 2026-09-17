// Structured host dialogs. Native outcomes remain the host's to observe.
let shown;
export function clearDialogs() {
  shown = undefined;
  document.getElementById("attention").replaceChildren();
}
export function renderDialogs(ticket, entry, live, send) {
  const root = document.getElementById("attention");
  const permission = ticket?.agent?.permission;
  const dialog = ticket?.agent?.dialog;
  const busy = !!entry?.receipt?.waiting;
  const expired = permission && Date.now() >= permission.expires_at;
  const key = JSON.stringify([entry?.key, permission, dialog, live, busy, !!expired]);
  if (key === shown) return;
  shown = key;
  root.replaceChildren();
  root.hidden = !permission && !dialog;
  if (root.hidden) return;
  const append = (tag, text, parent = root) => {
    const el = document.createElement(tag);
    el.textContent = text;
    parent.append(el);
    return el;
  };
  const button = (label, body, enabled = true) => {
    const el = append("button", label);
    el.type = "button";
    el.disabled = !live || busy || !enabled;
    el.onclick = () => send({ ...body, ticket: ticket.id, session: ticket.agent.session }, entry);
    return el;
  };
  if (permission) {
    append("h3", `Permission · ${permission.tool}`);
    append("pre", JSON.stringify(permission.input, null, 2));
    append("p", expired ? "Remote request expired. Check the pane." : "Approve this request once, or deny it.");
    for (const decision of ["allow", "deny"])
      button(decision === "allow" ? "Approve once" : "Deny", { op: "permission", request: permission.request, decision }, !expired);
    return;
  }
  const answer = (label, response, enabled = true) => button(label, { op: "dialog", request: dialog.request, response }, enabled);
  if (dialog.kind === "plan") {
    append("h3", "Review plan");
    append("pre", dialog.markdown);
    answer("Accept · approve edits manually", { answer: "accept" });
    answer("Reject plan", { answer: "reject" });
  } else if (dialog.kind === "questions") {
    const supported = dialog.questions.length === 1 && !dialog.questions[0].multiSelect;
    for (const question of dialog.questions) {
      append("h3", question.question);
      question.options.forEach((option, index) => {
        answer(option.label, { answer: "choice", index }, supported);
        if (option.description) append("p", option.description);
      });
    }
    if (supported) {
      const label = append("label", "Your answer");
      const input = document.createElement("input");
      input.type = "text";
      input.maxLength = 1000;
      input.autocomplete = "off";
      input.value = entry?.dialogDraft?.request === dialog.request ? entry.dialogDraft.text : "";
      input.oninput = () => { entry.dialogDraft = { request: dialog.request, text: input.value }; };
      label.append(input);
      const submit = answer("Send answer", { answer: "text", text: "" });
      submit.onclick = () => {
        if (input.value.trim()) send({ op: "dialog", ticket: ticket.id, session: ticket.agent.session,
          request: dialog.request, response: { answer: "text", text: input.value } }, entry);
      };
      answer("Decline question", { answer: "reject" });
    } else append("p", "This dialog shape needs a local answer in the pane.");
  }
  append("p", "The host checks the visible dialog before sending keys. Check the output if delivery is unknown.");
}
