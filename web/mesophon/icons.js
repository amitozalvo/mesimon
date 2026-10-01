// Stroke icons drawn from Lucide's geometry (ISC), inlined so the strict CSP
// needs no image source. Colour always comes from `currentColor`.
import { html } from "./html.js";

const paths = {
  back: "m12 19-7-7 7-7M19 12H5",
  bell: "M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9M10.3 21a1.94 1.94 0 0 0 3.4 0",
  check: "M20 6 9 17l-5-5",
  chevronDown: "m6 9 6 6 6-6",
  chevronRight: "m9 18 6-6-6-6",
  circleCheck: "M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20m-3-10 2 2 4-4",
  cloud: "M17.5 19H9a7 7 0 1 1 6.71-9h1.79a4.5 4.5 0 1 1 0 9Z",
  down: "M12 5v14m7-7-7 7-7-7",
  download: "M12 15V3m9 12v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4m4-5 5 5 5-5",
  file: "M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7ZM14 2v4a2 2 0 0 0 2 2h4M16 13H8m8 4H8m2-8H8",
  hourglass:
    "M5 22h14M5 2h14m-2 20v-4.17a2 2 0 0 0-.59-1.42L12 12l-4.41 4.41A2 2 0 0 0 7 17.83V22M7 2v4.17a2 2 0 0 0 .59 1.42L12 12l4.41-4.41A2 2 0 0 0 17 6.17V2",
  inbox:
    "M22 12h-6l-2 3h-4l-2-3H2m3.45-6.89L2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z",
  kanban: "M6 3h12a3 3 0 0 1 3 3v12a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3V6a3 3 0 0 1 3-3m2 4v7m4-7v4m4-4v9",
  leave: "M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4m7 14 5-5-5-5m5 5H9",
  lock: "M5 11h14a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2m2 0V7a5 5 0 0 1 10 0v4",
  moon: "M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z",
  pencil:
    "M21.17 6.81a1 1 0 0 0-3.99-3.99L3.84 16.17a2 2 0 0 0-.5.83l-1.32 4.35a.5.5 0 0 0 .62.62l4.35-1.32a2 2 0 0 0 .83-.5zM15 5l4 4",
  panelLeft: "M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2M9 3v18",
  play: "M5 5a2 2 0 0 1 3.008-1.728l11.997 6.998a2 2 0 0 1 .003 3.458l-12 7A2 2 0 0 1 5 19z",
  plus: "M12 5v14M5 12h14",
  search: "M11 19a8 8 0 1 0 0-16 8 8 0 0 0 0 16m10 2-4.3-4.3",
  send: "M14.54 21.69a.5.5 0 0 0 .94-.03l6.5-19a.5.5 0 0 0-.64-.64l-19 6.5a.5.5 0 0 0-.02.94l7.93 3.18a2 2 0 0 1 1.11 1.11zm7.31-19.54L10.91 13.09",
  sliders: "M21 4h-7m-4 0H3m18 8h-9m-4 0H3m18 8h-5m-4 0H3M14 2v4m-6 4v4m8 4v4",
  shield:
    "M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z",
  smartphone: "M7 2h10a2 2 0 0 1 2 2v16a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2m5 16h.01",
  spinner: "M21 12a9 9 0 1 1-6.22-8.56",
  terminal: "m4 17 6-6-6-6m8 14h8",
  up: "m5 12 7-7 7 7M12 19V5",
  wifiOff:
    "M12 20h.01M8.5 16.43a5 5 0 0 1 7 0M5 12.86a10 10 0 0 1 5.17-2.69M19 12.86a10 10 0 0 0-2-1.52M2 8.82a15 15 0 0 1 4.18-2.64M22 8.82a15 15 0 0 0-11.29-3.76M2 2l20 20",
  x: "M18 6 6 18M6 6l12 12",
  zap: "M4 14a1 1 0 0 1-.78-1.63l9.9-10.2a.5.5 0 0 1 .86.46l-1.92 6.02A1 1 0 0 0 13 10h7a1 1 0 0 1 .78 1.63l-9.9 10.2a.5.5 0 0 1-.86-.46l1.92-6.02A1 1 0 0 0 11 14z",
};

export function Icon({ name, size = 20, width = 2, cls = "" }) {
  return html`<svg class=${`icon ${cls}`} width=${size} height=${size} viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width=${width} stroke-linecap="round" stroke-linejoin="round"><path d=${paths[name]} /></svg>`;
}

// Delivery ticks, one shape language for every receipt: a clock while this
// browser holds it, one tick when the next hop holds it, two when it arrived,
// and two in teal once it was picked up at the desk (T-497).
export function Tick({ state }) {
  if (state === "clock")
    return html`<svg class="tick" width="14" height="14" viewBox="0 0 14 14" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><circle cx="7" cy="7" r="5.6" /><path d="M7 4.2V7l1.9 1.3" /></svg>`;
  if (!["one", "two", "picked"].includes(state)) return null;
  return html`<svg class=${state === "picked" ? "tick tick-picked" : "tick"} width="18" height="12" viewBox="0 0 18 12" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M1.5 6.4 4.9 9.8 11.4 2.2" />${state !== "one" && html`<path d="M6.9 8.4 8.3 9.8 14.8 2.2" />`}</svg>`;
}
