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

// A list that follows its newest line stays on it when its box or its rows
// change size without a render (T-1219): the keyboard opening shrinks the
// box, a larger text size or a picture loading grows a row. A reader who
// scrolled up stays where they are. A box that shrinks fires a scroll
// before the resize is seen, its offset unchanged or moved by the browser's
// scroll anchoring, and the list's own handler reads that as the reader
// leaving the end: a scroll that comes while the box is not the height last
// seen is the resize's, and the resize judges the end by what was known
// before it. What each list knows lives with its element, and its scroll
// listener with it: a render that subscribes again (the list's own scroll
// handler can cause one, mid-event) never drops a scroll.
const ends = new WeakMap();
export function keepAtEnd(node, following) {
  if (!node || typeof ResizeObserver !== "function") return undefined;
  let end = ends.get(node);
  if (!end) {
    end = { top: node.scrollTop, height: node.clientHeight, atEnd: following(), following };
    ends.set(node, end);
    node.addEventListener("scroll", () => {
      if (node.clientHeight !== end.height || node.scrollTop === end.top) return;
      end.top = node.scrollTop;
      end.atEnd = end.following();
    });
  }
  end.following = following;
  const watch = new ResizeObserver(() => {
    end.height = node.clientHeight;
    if (end.atEnd && node.getClientRects().length) node.scrollTop = node.scrollHeight;
    end.top = node.scrollTop;
  });
  watch.observe(node);
  for (const child of node.children) watch.observe(child);
  return () => watch.disconnect();
}
