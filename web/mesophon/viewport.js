// The page is the visual viewport (T-700). `body` is sized to it, not to the
// layout viewport, because a software keyboard on a phone shrinks only the
// visual one on iOS Safari: `100dvh` and `interactive-widget=resizes-content`
// change nothing there. Safari then scrolls the visual viewport down the
// layout viewport to show the focused field, so a body that is the right
// height but still at the top sat above what the person saw, by up to the
// keyboard's height: the whole page pushed out of view. `--viewport-top` is
// that scroll, and `body` moves down by it.
export function viewportVars(visual, fallbackHeight) {
  const height = visual?.height || fallbackHeight;
  // pageTop is the visible top in the page's own coordinates: the layout
  // viewport's scroll plus the visual viewport's offset within it.
  const top = Math.max(0, visual?.pageTop || 0);
  return { "--viewport-height": `${height}px`, "--viewport-top": `${top}px` };
}

// A keyboard is at least this much shorter than the window; a collapsed
// address bar changes both heights together and a pinch-zoom is the one
// other thing that parts them, where a dropped inset costs nothing.
export const KEYBOARD_MIN_PX = 100;

// Whether something covers the bottom of the window: the visual viewport is
// shorter than the layout viewport, a keyboard in practice. iOS reports
// `safe-area-inset-bottom` for the home indicator all the while, though the
// keyboard sits on it, so the page kept a band of padding under the composer.
export function keyboardUp(visual, windowHeight) {
  return !!visual?.height && windowHeight - visual.height > KEYBOARD_MIN_PX;
}

// Keeps the two variables on the root element as the viewport moves. iOS
// reports the keyboard's shrink on `resize` and its scroll on `scroll`, so
// both are followed. Set through the CSSOM, outside the page's CSP.
export function followViewport(win = window, root = win.document.documentElement) {
  const apply = () => {
    for (const [name, value] of Object.entries(viewportVars(win.visualViewport, win.innerHeight))) {
      root.style.setProperty(name, value);
    }
    // `--safe-bottom` (style.css) is the bottom inset, 0 while the keyboard is up.
    if (keyboardUp(win.visualViewport, win.innerHeight)) root.dataset.keyboard = "up";
    else delete root.dataset.keyboard;
  };
  win.visualViewport?.addEventListener("resize", apply);
  win.visualViewport?.addEventListener("scroll", apply);
  win.addEventListener("resize", apply);
  apply();
}
