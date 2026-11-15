// Receives only authenticated, decrypted daemon projections.
import { swipeAway } from "./swipe.js";
export function shouldAlert(reply, visibleTicket) {
  return reply.result === "awareness" && reply.alert === true &&
    reply.ticket !== visibleTicket &&
    ["waiting_for_approval", "waiting_for_input", "completed", "failed"].includes(reply.awareness?.phase);
}
export function alertTitle(phase) {
  return { waiting_for_approval: "Approval needed", waiting_for_input: "Your input is needed",
    completed: "Turn completed", failed: "Agent failed" }[phase] || "Agent update";
}
// A finished turn is news, not a call (T-1268): its banner goes by itself
// after a few seconds. A call for approval, for input or a failure stays
// until it is opened, dismissed or swiped away.
export const PASSING_MS = 5000;
export function passes(phase) {
  return phase === "completed";
}
let nativeNotice;
let swipeArmed;
let passTimer;
// Only the banner: a system notice stays in the phone's list until read.
function hideBanner() {
  clearTimeout(passTimer);
  passTimer = undefined;
  const banner = document.getElementById("awareness");
  banner.hidden = true;
  banner.replaceChildren();
}
export function clearAlerts() {
  nativeNotice?.close();
  nativeNotice = undefined;
  hideBanner();
}
export function showAlert(reply, visibleTicket, navigate) {
  if (!shouldAlert(reply, visibleTicket)) return;
  clearAlerts();
  const banner = document.getElementById("awareness");
  // Swiped away, it is dismissed; the last swipe's flight is undone here.
  if (!swipeArmed) swipeArmed = swipeAway(banner, clearAlerts, { up: true });
  banner.style.translate = "";
  banner.style.opacity = "";
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
  if (passes(reply.awareness.phase)) passTimer = setTimeout(hideBanner, PASSING_MS);
  if (typeof Notification === "undefined" || Notification.permission !== "granted") return;
  try {
    const notice = nativeNotice = new Notification(alertTitle(reply.awareness.phase), {
      body: reply.awareness.headline,
      tag: reply.awareness.deepLink,
    });
    notice.onclick = () => { window.focus(); navigate(reply.ticket); notice.close(); };
  } catch { /* This browser does not support notifications from an open page. */ }
}
