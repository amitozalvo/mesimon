// The only place the vendored Preact and htm are imported. Plain modules, no
// build step: the relay serves these files as they are (see vendor/README.md).
import { h, render } from "./vendor/preact.module.js";
import htm from "./vendor/htm.module.js";

export const html = htm.bind(h);
export { render };
export {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "./vendor/hooks.module.js";
