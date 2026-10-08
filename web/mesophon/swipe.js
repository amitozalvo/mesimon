// A notice goes with a swipe, as a phone's own notifications do (T-628):
// sideways either way, and with `up` upward too. A press that never moves
// past the slop stays a tap on whatever it pressed. The drag rides the
// `translate` property, so a notice's own `transform` keeps placing it.
const SLOP = 10; // px before a press is a drag
const FAR = 0.35; // of the notice's size, or a flick
const FLICK = 0.5; // px per ms

export function swipeAway(el, dismiss, { up = false } = {}) {
  let press, axis, offset, swallow;
  const reset = () => {
    el.classList.remove("swiping");
    el.style.translate = "";
    el.style.opacity = "";
  };
  const down = (event) => {
    swallow = false;
    if (!event.isPrimary || event.button > 0) return;
    press = { x: event.clientX, y: event.clientY, at: event.timeStamp, id: event.pointerId };
    axis = undefined;
    offset = 0;
  };
  const move = (event) => {
    if (!press || event.pointerId !== press.id) return;
    const x = event.clientX - press.x;
    const y = event.clientY - press.y;
    if (!axis) {
      if (Math.hypot(x, y) < SLOP) return;
      if (Math.abs(x) >= Math.abs(y)) axis = "x";
      else if (up && y < 0) axis = "y";
      else {
        press = undefined;
        return;
      }
      el.setPointerCapture?.(event.pointerId);
      el.classList.add("swiping");
    }
    offset = axis === "x" ? x : Math.min(0, y);
    const size = axis === "x" ? el.offsetWidth : el.offsetHeight;
    el.style.translate = axis === "x" ? `${offset}px 0` : `0 ${offset}px`;
    el.style.opacity = String(Math.max(0.2, 1 - Math.abs(offset) / (size || 1)));
  };
  const end = (event) => {
    if (!press || event.pointerId !== press.id) return;
    const dragged = axis;
    const speed = Math.abs(offset) / Math.max(1, event.timeStamp - press.at);
    press = undefined;
    if (!dragged) return;
    // The click a drag ends in is not a tap on a button inside.
    swallow = true;
    const size = dragged === "x" ? el.offsetWidth : el.offsetHeight;
    const gone = event.type === "pointerup" &&
      (Math.abs(offset) > size * FAR || (speed > FLICK && Math.abs(offset) > SLOP * 3));
    el.classList.remove("swiping");
    if (!gone) {
      reset();
      return;
    }
    const away = Math.sign(offset) * (size + 48);
    el.style.translate = dragged === "x" ? `${away}px 0` : `0 ${away}px`;
    el.style.opacity = "0";
    setTimeout(dismiss, 160);
  };
  const click = (event) => {
    if (!swallow) return;
    swallow = false;
    event.preventDefault();
    event.stopPropagation();
  };
  el.addEventListener("pointerdown", down);
  el.addEventListener("pointermove", move);
  el.addEventListener("pointerup", end);
  el.addEventListener("pointercancel", end);
  el.addEventListener("click", click, true);
  return () => {
    el.removeEventListener("pointerdown", down);
    el.removeEventListener("pointermove", move);
    el.removeEventListener("pointerup", end);
    el.removeEventListener("pointercancel", end);
    el.removeEventListener("click", click, true);
    reset();
  };
}

// The ticket page's panes (T-701): a sideways drag past the slop follows the
// finger, and past FAR of the width, or a flick, `step` is told which way;
// an upright drag is the pane's own scroll. The track is `el`'s first
// child, placed by `--pane` and nudged by `--drag` while the finger is down.
export function swipePanes(el, step) {
  let press, axis, offset;
  const track = () => el.firstElementChild;
  const settle = () => {
    track()?.style.removeProperty("--drag");
    el.classList.remove("swiping");
  };
  const down = (event) => {
    if (!event.isPrimary || event.button > 0) return;
    press = { x: event.clientX, y: event.clientY, at: event.timeStamp, id: event.pointerId };
    axis = undefined;
    offset = 0;
  };
  const move = (event) => {
    if (!press || event.pointerId !== press.id) return;
    const x = event.clientX - press.x;
    const y = event.clientY - press.y;
    if (!axis) {
      if (Math.hypot(x, y) < SLOP) return;
      if (Math.abs(x) >= Math.abs(y) * 1.5) axis = "x";
      else {
        press = undefined;
        return;
      }
      el.setPointerCapture?.(event.pointerId);
      el.classList.add("swiping");
    }
    offset = x;
    track()?.style.setProperty("--drag", `${offset}px`);
  };
  const end = (event) => {
    if (!press || event.pointerId !== press.id) return;
    const dragged = axis;
    const speed = Math.abs(offset) / Math.max(1, event.timeStamp - press.at);
    press = undefined;
    settle();
    if (!dragged) return;
    const size = el.offsetWidth || 1;
    const gone = event.type === "pointerup" &&
      (Math.abs(offset) > size * FAR || (speed > FLICK && Math.abs(offset) > SLOP * 3));
    if (gone) step(offset < 0 ? 1 : -1);
  };
  el.addEventListener("pointerdown", down);
  el.addEventListener("pointermove", move);
  el.addEventListener("pointerup", end);
  el.addEventListener("pointercancel", end);
  return () => {
    el.removeEventListener("pointerdown", down);
    el.removeEventListener("pointermove", move);
    el.removeEventListener("pointerup", end);
    el.removeEventListener("pointercancel", end);
    settle();
  };
}
