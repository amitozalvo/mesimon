// A small markdown reading for plans and notes: headings, lists, fenced code,
// code spans and bold become elements whose children are plain text, never
// HTML. A picture is named, not fetched (T-532). Anything else is a paragraph.
import { html } from "./html.js";
import { Icon } from "./icons.js";

const PICTURE = /^!\[[^\]]*\]\([^)]*\)$/;

export function blocks(text) {
  const out = [];
  let list;
  let fence;
  for (const raw of text.split("\n")) {
    const line = raw.trimEnd();
    if (fence) {
      if (/^\s*(```|~~~)/.test(line)) fence = undefined;
      else fence.lines.push(raw);
      continue;
    }
    if (/^\s*(```|~~~)/.test(line)) {
      list = undefined;
      out.push((fence = { code: true, lines: [] }));
      continue;
    }
    const item = line.match(/^\s*(?:[-*+]|\d+[.)])\s+(.*)$/);
    if (item) {
      const ordered = /^\s*\d/.test(line);
      if (!list || list.ordered !== ordered) out.push((list = { ordered, items: [] }));
      list.items.push(item[1]);
      continue;
    }
    list = undefined;
    const heading = line.match(/^#{1,6}\s+(.*)$/);
    if (heading) out.push({ heading: heading[1] });
    else if (PICTURE.test(line.trim())) out.push({ picture: true });
    else if (line.trim()) out.push({ text: line.replace(/^\s*>\s?/, "") });
  }
  return out;
}

const inline = (text) =>
  text.split(/(`[^`]+`|\*\*[^*]+\*\*)/).map((part) =>
    part.length > 2 && part.startsWith("`") && part.endsWith("`")
      ? html`<code>${part.slice(1, -1)}</code>`
      : part.length > 4 && part.startsWith("**") && part.endsWith("**")
        ? html`<strong>${part.slice(2, -2)}</strong>`
        : part,
  );

export function Markdown({ text }) {
  return html`<div class="markdown" dir="auto">${blocks(text).map((b) =>
    b.code
      ? html`<pre class="markdown-code">${b.lines.join("\n")}</pre>`
      : b.picture
        ? html`<p class="markdown-picture"><${Icon} name="image" size=${16} /><span>Picture · open it at your desk</span></p>`
        : b.heading !== undefined
          ? html`<h4>${inline(b.heading)}</h4>`
          : b.items
            ? b.ordered
              ? html`<ol>${b.items.map((i) => html`<li>${inline(i)}</li>`)}</ol>`
              : html`<ul>${b.items.map((i) => html`<li>${inline(i)}</li>`)}</ul>`
            : html`<p>${inline(b.text)}</p>`,
  )}</div>`;
}
