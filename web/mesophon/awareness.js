// Receives only authenticated, decrypted daemon projections.
export function shouldAlert(reply, visibleTicket) {
  return reply.result === "awareness" && reply.alert === true &&
    reply.ticket !== visibleTicket &&
    ["waiting_for_approval", "waiting_for_input", "completed", "failed"].includes(reply.awareness?.phase);
}
export function alertTitle(phase) {
  return { waiting_for_approval: "Approval needed", waiting_for_input: "Your input is needed",
    completed: "Turn completed", failed: "Agent failed" }[phase] || "Agent update";
}
let nativeNotice;
export function clearAlerts() {
  nativeNotice?.close();
  nativeNotice = undefined;
  const banner = document.getElementById("awareness");
  banner.hidden = true;
  banner.replaceChildren();
}
export function showAlert(reply, visibleTicket, navigate) {
  if (!shouldAlert(reply, visibleTicket)) return;
  clearAlerts();
  const banner = document.getElementById("awareness");
  const open = document.createElement("button");
  open.type = "button";
  open.textContent = `${alertTitle(reply.awareness.phase)} · ${reply.awareness.headline}`;
  open.onclick = () => { navigate(reply.ticket); clearAlerts(); };
  const dismiss = document.createElement("button");
  dismiss.type = "button";
  dismiss.textContent = "Dismiss";
  dismiss.onclick = clearAlerts;
  banner.append(open, dismiss);
  banner.hidden = false;
  if (typeof Notification === "undefined" || Notification.permission !== "granted") return;
  try {
    const notice = nativeNotice = new Notification(alertTitle(reply.awareness.phase), {
      body: reply.awareness.headline,
      tag: reply.awareness.deepLink,
    });
    notice.onclick = () => { window.focus(); navigate(reply.ticket); notice.close(); };
  } catch { /* This browser does not support notifications from an open page. */ }
}
