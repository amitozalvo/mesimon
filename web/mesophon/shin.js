// The shin, drawn from assets/mascot/shin.txt (the TUI's own sprite source):
// top edges catch the light, the underside falls away, and the right arm's
// head is the status light. Colours are theme tokens set in style.css.
import { html } from "./html.js";

const sprites = {
  small: {
    box: "0 0 12 10",
    body: "M0 0h2v1h-2zM4 0h2v1h-2zM9 0h2v1h-2zM1 1h1v1h-1zM3 1h2v1h-2zM9 1h2v1h-2zM1 2h3v1h-3zM9 2h2v1h-2zM0 3h12v1h-12zM0 4h12v1h-12zM0 5h12v1h-12zM0 6h12v1h-12zM0 7h12v1h-12zM0 8h12v1h-12zM1 9h10v1h-10z",
    hi: "M0 0h2v1h-2zM4 0h2v1h-2zM9 0h2v1h-2zM3 1h1v1h-1zM2 2h1v1h-1zM0 3h1v1h-1zM4 3h5v1h-5zM11 3h1v1h-1z",
    shade: "M4 1h1v1h-1zM0 8h1v1h-1zM11 8h1v1h-1zM1 9h10v1h-10z",
    blush: "M1 7h1v1h-1zM10 7h1v1h-1z",
    eyes: "M3 5h1v1h-1zM9 5h1v1h-1zM2 6h2v1h-2zM8 6h2v1h-2z",
    glint: "M2 5h1v1h-1zM8 5h1v1h-1z",
    closed: "M2 6h2v1h-2zM8 6h2v1h-2z",
    mouth: "M5 7h2v1h-2z",
    surprised: "M5 7h2v1h-2z",
    light: "M9 0h1v1h-1z",
  },
  medium: {
    box: "0 0 20 16",
    body: "M1 0h3v1h-3zM13 0h3v1h-3zM1 1h4v1h-4zM7 1h3v1h-3zM13 1h4v1h-4zM2 2h3v1h-3zM6 2h4v1h-4zM14 2h3v1h-3zM3 3h6v1h-6zM14 3h3v1h-3zM3 4h5v1h-5zM14 4h3v1h-3zM2 5h16v1h-16zM1 6h18v1h-18zM0 7h20v1h-20zM0 8h20v1h-20zM0 9h20v1h-20zM0 10h20v1h-20zM0 11h20v1h-20zM0 12h20v1h-20zM0 13h20v1h-20zM1 14h18v1h-18zM3 15h14v1h-14z",
    hi: "M1 0h3v1h-3zM13 0h3v1h-3zM4 1h1v1h-1zM7 1h3v1h-3zM16 1h1v1h-1zM6 2h1v1h-1zM5 3h1v1h-1zM2 5h1v1h-1zM8 5h6v1h-6zM17 5h1v1h-1zM1 6h1v1h-1zM18 6h1v1h-1zM0 7h1v1h-1zM19 7h1v1h-1z",
    shade: "M1 1h1v1h-1zM13 1h1v1h-1zM2 2h1v1h-1zM9 2h1v1h-1zM8 3h1v1h-1zM0 13h1v1h-1zM19 13h1v1h-1zM1 14h2v1h-2zM17 14h2v1h-2zM3 15h14v1h-14z",
    blush: "M1 12h2v1h-2zM17 12h2v1h-2z",
    eyes: "M5 8h2v1h-2zM14 8h2v1h-2zM4 9h3v1h-3zM13 9h3v1h-3zM4 10h2v1h-2zM13 10h2v1h-2z",
    glint: "M4 8h1v1h-1zM13 8h1v1h-1zM6 10h1v1h-1zM15 10h1v1h-1z",
    closed: "M4 9h1v1h-1zM6 9h1v1h-1zM13 9h1v1h-1zM15 9h1v1h-1zM5 10h1v1h-1zM14 10h1v1h-1z",
    mouth: "M8 12h1v1h-1zM11 12h1v1h-1zM9 13h2v1h-2z",
    flat: "M9 13h2v1h-2z",
    surprised: "M9 12h2v1h-2zM9 13h2v1h-2z",
    light: "M14 0h1v1h-1z",
  },
};

// mood: awake | asleep | surprised. light: calm | attn | dim.
export function Shin({ size = "small", scale = 2, mood = "awake", light = "calm" }) {
  const s = sprites[size];
  const [, , w, h] = s.box.split(" ").map(Number);
  const asleep = mood === "asleep";
  return html`<span class=${`shin shin-${mood}`} aria-hidden="true">
    <svg width=${w * scale} height=${h * scale} viewBox=${s.box} shape-rendering="crispEdges">
      <path class="sh-body" d=${s.body} />
      <path class="sh-hi" d=${s.hi} />
      <path class="sh-shade" d=${s.shade} />
      <path class="sh-blush" d=${s.blush} />
      ${asleep
        ? html`<path class="sh-eye" d=${s.closed} />`
        : html`<g class="sh-eyes"><path class="sh-eye" d=${s.eyes} /><path class="sh-glint" d=${s.glint} /></g>`}
      <path class="sh-eye" d=${asleep ? s.flat || s.mouth : mood === "surprised" ? s.surprised : s.mouth} />
      <path class=${`sh-light sh-light-${light}`} d=${s.light} />
    </svg>
    ${asleep && html`<span class="shin-z">z</span><span class="shin-z shin-z2">Z</span>`}
  </span>`;
}
