// Markdown for plans, notes and the agent's replies, read as the desk reads
// it (`tui/src/rich.rs`, T-626): headings, paragraphs and hard breaks,
// quotes holding blocks (a GitHub alert names itself), nested and task lists,
// fenced code, pipe tables with their alignment, thematic breaks, and inline
// code, strong, emphasis, struck text, links (a bare web address and
// `<https://…>` too, T-704) and `<br>`. Every element's
// children are plain text, never HTML. A picture is named, not fetched
// (T-532): a markdown image, or the desk's `[Image #N](mesimon-attachment:…)`
// (T-629), alone on its line or inside one. Reference links, footnotes,
// setext headings, indented code and HTML (bar `<br>`) are text, as there.
import { html, useRef, useState } from "./html.js";
import { Icon } from "./icons.js";

const PICTURE = /^!\[[^\]]*\]\([^)]*\)$|^\[Image #\d+\]\(mesimon-attachment:[^)]*\)$/;
const ATTACHED = /^\[(Image #\d+)\]\(mesimon-attachment:[^)]*\)$/;

// ---- blocks ------------------------------------------------------------------

// `breaks`: a newline inside a paragraph is a break (a person's note, typed
// into a field that wraps for them) rather than a space (an agent's reply,
// wrapped by hand), as the desk's `Newline` says.
export function blocks(src, breaks = false) {
  const out = [];
  let pending;
  let fence;
  const lines = src.split("\n").map((l) => l.replace(/\r$/, ""));
  let at = 0;
  while (at < lines.length) {
    const raw = lines[at++];
    // Inside a fence every line is verbatim until the closing marker.
    if (fence) {
      const t = raw.trim();
      if (t.startsWith(fence.ch.repeat(fence.len)) && [...t].every((c) => c === fence.ch)) {
        out.push({ code: fence.rows });
        fence = undefined;
      } else fence.rows.push(raw);
      continue;
    }
    const opened = fenceOpen(raw);
    if (opened) {
      pending = flush(pending, out, breaks);
      fence = { ...opened, rows: [] };
      continue;
    }
    const trimmed = raw.trimEnd();
    const body = trimmed.trimStart();
    const indent = trimmed.length - body.length;
    if (!body) {
      pending = flush(pending, out, breaks);
      out.push({ blank: true });
      continue;
    }
    if (isBreak(body)) {
      pending = flush(pending, out, breaks);
      out.push({ rule: true });
      continue;
    }
    const head = heading(body);
    if (head) {
      pending = flush(pending, out, breaks);
      out.push({ head: head.level, runs: inline(head.text) });
      continue;
    }
    const align = tableHead(body, lines[at]);
    if (align) {
      pending = flush(pending, out, breaks);
      at++; // the delimiter row
      const row = (line) => {
        const cs = cells(line).map(inline);
        while (cs.length < align.length) cs.push([]);
        return cs.slice(0, align.length);
      };
      const rows = [];
      // The table runs to the first line that cannot be a row of it.
      while (at < lines.length) {
        const next = lines[at].trim();
        if (!next || !next.includes("|") || fenceOpen(next)) break;
        rows.push(row(next));
        at++;
      }
      out.push({ table: { head: row(body), align, rows } });
      continue;
    }
    if (body.startsWith("|")) {
      pending = flush(pending, out, breaks);
      out.push({ raw: body });
      continue;
    }
    if (PICTURE.test(body)) {
      pending = flush(pending, out, breaks);
      out.push({ picture: body.match(ATTACHED)?.[1] || "Picture" });
      continue;
    }
    const item = listMarker(body);
    if (item) {
      pending = flush(pending, out, breaks);
      // Two source spaces a level, capped: a deep tree would spend the
      // whole column on indent.
      const depth = Math.min(Math.floor(indent / 2), 3);
      const { task, text } = taskBox(item.text);
      pending = { item: { marker: item.marker, depth, task }, lines: [lineText(raw, text)] };
      continue;
    }
    const quoted = quoteLine(raw.trimStart());
    if (quoted !== undefined) {
      if (pending?.quote) pending.quote.push(quoted);
      else {
        pending = flush(pending, out, breaks);
        pending = { quote: [quoted] };
      }
      continue;
    }
    // Plain text: markdown's lazy continuation. It belongs to whatever
    // paragraph is open (a list item's next line, a quote's), else starts one.
    if (pending?.quote && pending.quote.at(-1).trim()) pending.quote.push(raw.trimStart());
    else if (pending?.para || pending?.item) pending.lines.push(lineText(raw, body));
    else {
      pending = flush(pending, out, breaks);
      pending = { para: true, lines: [lineText(raw, body)] };
    }
  }
  if (fence) out.push({ code: fence.rows }); // unterminated: show it anyway
  flush(pending, out, breaks);
  return out;
}

function flush(pending, out, breaks) {
  if (!pending) return undefined;
  if (pending.para) out.push({ para: inline(joined(pending.lines, breaks)) });
  else if (pending.quote) {
    const lines = [...pending.quote];
    // A GitHub alert (`> [!NOTE]`) names itself on its own first line.
    const kind = alert(lines[0]);
    if (kind) lines[0] = `**${kind}**\\`;
    out.push({ quote: blocks(lines.join("\n"), breaks) });
  } else if (pending.item) {
    const runs = inline(joined(pending.lines, breaks));
    // A ticked item is read last, like struck text.
    if (pending.item.task === true) for (const r of runs) r.dead = true;
    out.push({ item: pending.item, runs });
  }
  return undefined;
}

// A paragraph's lines as one string: joined by a space, or by a break where
// `breaks` says so; a line ending in its own hard break gets no second one.
function joined(lines, breaks) {
  let s = "";
  for (const l of lines) {
    if (s && !s.endsWith("\n")) s += breaks ? "\n" : " ";
    s += l;
  }
  return s;
}

// CommonMark's hard break, two trailing spaces or a trailing backslash,
// spelt `\n`.
function lineText(raw, body) {
  if (body.endsWith("\\") && !body.endsWith("\\\\")) return `${body.slice(0, -1)}\n`;
  if (raw.endsWith("  ")) return `${body}\n`;
  return body;
}

function taskBox(text) {
  for (const [mark, done] of [["[ ] ", false], ["[x] ", true], ["[X] ", true]])
    if (text.startsWith(mark)) return { task: done, text: text.slice(mark.length) };
  return { task: undefined, text };
}

function alert(line) {
  const m = line.trim().match(/^\[!(\w+)\]$/);
  return m && ["Note", "Tip", "Important", "Warning", "Caution"].find((k) => k.toLowerCase() === m[1].toLowerCase());
}

function fenceOpen(line) {
  const t = line.trimStart();
  for (const ch of ["`", "~"]) {
    let n = 0;
    while (t[n] === ch) n++;
    if (n >= 3) return { ch, len: n };
  }
  return undefined;
}

function isBreak(body) {
  return ["-", "*", "_"].some((ch) => [...body].filter((c) => c === ch).length >= 3 && [...body].every((c) => c === ch || c === " "));
}

// A table's header row, when the line under it is a delimiter row with as
// many cells. Both carry a pipe, or `text` over `---` would be a table.
function tableHead(body, next) {
  if (next === undefined) return undefined;
  next = next.trim();
  if (!body.includes("|") || !next.includes("|")) return undefined;
  const align = [];
  for (const c of cells(next)) {
    const dashes = c.replace(/^:+/, "").replace(/:+$/, "");
    if (!dashes || !/^-+$/.test(dashes)) return undefined;
    align.push(c.startsWith(":") && c.endsWith(":") ? "center" : c.endsWith(":") ? "right" : "left");
  }
  return cells(body).length === align.length ? align : undefined;
}

// A row's cells, trimmed: split on every pipe but an escaped one, the outer
// pipes optional.
function cells(line) {
  let t = line.trim();
  if (t.startsWith("|")) t = t.slice(1);
  if (t.endsWith("|") && !t.slice(0, -1).endsWith("\\")) t = t.slice(0, -1);
  const out = [];
  let cur = "";
  let escaped = false;
  for (const c of t) {
    if (c === "|" && !escaped) {
      out.push(cur.trim());
      cur = "";
    } else cur += c;
    escaped = c === "\\" && !escaped;
  }
  out.push(cur.trim());
  return out;
}

function heading(body) {
  const m = body.match(/^(#{1,6}) (.*)$/);
  return m && { level: m[1].length, text: m[2].replace(/[# ]+$/, "") };
}

function quoteLine(body) {
  if (!body.startsWith(">")) return undefined;
  const rest = body.slice(1);
  return rest.startsWith(" ") ? rest.slice(1) : rest;
}

function listMarker(body) {
  const bullet = body.match(/^[-*+] (.*)$/s);
  if (bullet) return { marker: undefined, text: bullet[1] };
  const ordered = body.match(/^(\d{1,3}[.)]) (.*)$/s);
  return ordered ? { marker: ordered[1], text: ordered[2] } : undefined;
}

// ---- inline ------------------------------------------------------------------

const PUNCT = /[!-/:-@[-`{-~]/;
const space = (c) => c === undefined || /\s/.test(c);
const word = (c) => c !== undefined && /[\p{L}\p{N}]/u.test(c);

function runLen(chars, i) {
  let n = 0;
  while (chars[i + n] === chars[i]) n++;
  return n;
}
function delimLen(chars, i) {
  const n = runLen(chars, i);
  if (chars[i] === "~") return n >= 2 ? 2 : 0;
  return n >= 2 ? 2 : 1;
}
// A delimiter opens only when it hugs the text to its right, and `_` never
// inside a word: `snake_case_names` stay as they are.
function opens(chars, i, len) {
  const ok = !space(chars[i + len]);
  return chars[i] === "_" ? ok && !word(chars[i - 1]) : ok;
}
function closes(chars, i, len) {
  const ok = i > 0 && !space(chars[i - 1]);
  return chars[i] === "_" ? ok && !word(chars[i + len]) : ok;
}
function hasCloser(chars, from, ch, len) {
  for (let i = from; i < chars.length; i++)
    if (chars[i] === ch && runLen(chars, i) >= len && closes(chars, i, len)) return true;
  return false;
}
const flagOf = (ch, len) => (ch === "~" ? "dead" : len === 2 ? "strong" : "em");

export function inline(s) {
  const chars = Array.from(s);
  const out = [];
  let cur = "";
  let emph = {};
  const open = [];
  const push = () => {
    if (cur) out.push({ text: cur, ...emph });
    cur = "";
  };
  let i = 0;
  while (i < chars.length) {
    const c = chars[i];
    // Escapes first, so `\*` is a star and not an opener.
    if (c === "\\" && chars[i + 1] !== undefined && PUNCT.test(chars[i + 1])) {
      cur += chars[i + 1];
      i += 2;
      continue;
    }
    if (c === "`") {
      const ticks = runLen(chars, i);
      const close = findTicks(chars, i + ticks, ticks);
      if (close !== undefined) {
        const text = chars.slice(i + ticks, close).join("").trim();
        if (text) {
          push();
          out.push({ text, ...emph, code: true });
          i = close + ticks;
          continue;
        }
      }
    }
    // `<br>` is the one tag read: GFM's break inside a table cell.
    if (c === "<") {
      const auto = angleUrl(chars, i);
      if (auto) {
        push();
        out.push({ text: auto.url, ...emph, href: auto.url });
        i = auto.next;
        continue;
      }
      const n = brTag(chars, i);
      if (n) {
        cur += "\n";
        i += n;
        continue;
      }
    }
    if (c === "[" || (c === "!" && chars[i + 1] === "[")) {
      const found = link(chars, i, emph);
      if (found) {
        push();
        out.push(...found.runs);
        i = found.next;
        continue;
      }
    }
    const bare = bareUrl(chars, i);
    if (bare) {
      push();
      out.push({ text: bare.url, ...emph, href: bare.url });
      i = bare.next;
      continue;
    }
    if (c === "*" || c === "_" || c === "~") {
      const len = delimLen(chars, i);
      if (len > 0) {
        // The innermost open delimiter closes first, on a run at least its
        // length: `***both***` opens `**` then `*`.
        const last = open.at(-1);
        if (last && last.ch === c && runLen(chars, i) >= last.len && closes(chars, i, last.len)) {
          push();
          open.pop();
          emph = { ...emph, [flagOf(c, last.len)]: false };
          i += last.len;
          continue;
        }
        if (!emph[flagOf(c, len)] && opens(chars, i, len) && hasCloser(chars, i + len, c, len)) {
          push();
          open.push({ ch: c, len });
          emph = { ...emph, [flagOf(c, len)]: true };
          i += len;
          continue;
        }
      }
    }
    cur += c;
    i++;
  }
  push();
  return out;
}

// A bare web address (GFM's autolink, T-704): from `http://` or `https://`
// to the first space or `<`, less the punctuation that ends a sentence or
// closes emphasis around it, and a `)` it did not open.
const WEB = /^https?:\/\/[^\s/]/i;
function bareUrl(chars, i) {
  if ((chars[i] !== "h" && chars[i] !== "H") || word(chars[i - 1])) return undefined;
  if (!WEB.test(chars.slice(i, i + 9).join(""))) return undefined;
  let end = i;
  while (!space(chars[end]) && chars[end] !== "<") end++;
  for (;;) {
    const last = chars[end - 1];
    const url = chars.slice(i, end);
    if (/[.,;:!?'"*_~]/.test(last)) end--;
    else if (last === ")" && url.filter((ch) => ch === ")").length > url.filter((ch) => ch === "(").length) end--;
    else break;
  }
  const url = chars.slice(i, end).join("");
  return WEB.test(url) ? { url, next: end } : undefined;
}

// `<https://…>`: CommonMark's autolink, the brackets not drawn.
function angleUrl(chars, i) {
  const m = /^<(https?:\/\/[^\s<>]+)>/i.exec(chars.slice(i, i + 2048).join(""));
  return m ? { url: m[1], next: i + Array.from(m[0]).length } : undefined;
}

// Plain text with its web addresses followed (T-704): a prompt, drawn as
// written rather than as markdown.
export function linked(s) {
  const chars = Array.from(s || "");
  const out = [];
  let from = 0;
  for (let i = 0; i < chars.length; ) {
    const bare = bareUrl(chars, i);
    if (!bare) {
      i++;
      continue;
    }
    if (i > from) out.push({ text: chars.slice(from, i).join("") });
    out.push({ text: bare.url, href: bare.url });
    i = from = bare.next;
  }
  if (from < chars.length) out.push({ text: chars.slice(from).join("") });
  return out;
}

function brTag(chars, i) {
  const ahead = chars.slice(i, i + 6).join("").toLowerCase();
  return ["<br>", "<br/>", "<br />"].find((t) => ahead.startsWith(t))?.length;
}

function findTicks(chars, from, len) {
  for (let i = from; i < chars.length; i++) if (chars[i] === "`" && runLen(chars, i) === len) return i;
  return undefined;
}

// `[label](url)` / `![alt](url)`: the label, and the target after it, read
// last. A web target is a link the page can follow; a picture of the
// ticket's (`mesimon-attachment:`) is named.
function link(chars, i, emph) {
  const image = chars[i] === "!";
  const start = image ? i + 1 : i;
  const close = chars.indexOf("]", start + 1);
  if (close < 0 || chars[close + 1] !== "(") return undefined;
  const end = chars.indexOf(")", close + 2);
  if (end < 0) return undefined;
  const label = chars.slice(start + 1, close).join("");
  const url = chars.slice(close + 2, end).join("").trim();
  if (url.startsWith("mesimon-attachment:") || image)
    return { runs: [{ text: label.trim() || "Picture", ...emph, pic: true }], next: end + 1 };
  if (!label.trim() && !url) return undefined;
  const href = /^https?:\/\//i.test(url) ? url : undefined;
  const runs = inline(label).map((r) => ({ ...r, em: true, href }));
  // The target is followed as the label is (T-704): on a phone the label
  // can be the smaller press.
  if (url && url !== label.trim()) runs.push({ text: " ", ...emph }, { text: url, ...emph, url: true, href });
  return { runs, next: end + 1 };
}

// ---- drawing -----------------------------------------------------------------

function Run({ run }) {
  if (run.pic)
    return html`<span class="markdown-pic"><${Icon} name="image" size=${14} />${run.text}</span>`;
  let node = run.text.includes("\n")
    ? run.text.split("\n").flatMap((part, i) => (i ? [html`<br />`, part] : [part]))
    : run.text;
  if (run.code) node = html`<code>${node}</code>`;
  if (run.strong) node = html`<strong>${node}</strong>`;
  if (run.em) node = html`<em>${node}</em>`;
  if (run.dead) node = html`<del>${node}</del>`;
  if (run.url) node = html`<span class="md-url">${node}</span>`;
  if (run.href) node = html`<a href=${run.href} target="_blank" rel="noopener noreferrer">${node}</a>`;
  return node;
}
const runs = (rs) => rs.map((r) => html`<${Run} run=${r} />`);

export function Linked({ text }) {
  return runs(linked(text));
}

// The clipboard, where the page may write it (a secure origin, a press).
export async function copyText(text) {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

// A fenced block with its Copy (T-704): a phone cannot drag a selection
// across a block that scrolls sideways. Where the clipboard refuses, the
// block is selected instead, and the phone's own Copy is one press away.
function Code({ rows }) {
  const ref = useRef();
  const [copied, setCopied] = useState(false);
  const text = rows.join("\n");
  const copy = async () => {
    if (await copyText(text)) {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
      return;
    }
    const range = document.createRange();
    range.selectNodeContents(ref.current);
    getSelection()?.removeAllRanges();
    getSelection()?.addRange(range);
  };
  return html`<div class="markdown-code-box"><pre ref=${ref} class="markdown-code">${text}</pre>
    <button type="button" class="code-copy" aria-label=${copied ? "Copied" : "Copy"} onClick=${copy}><${Icon} name=${copied ? "check" : "copy"} size=${15} /></button></div>`;
}

function Table({ table }) {
  const cell = (rs, i, Tag) => html`<${Tag} class=${`md-${table.align[i]}`}>${runs(rs)}</${Tag}>`;
  return html`<div class="md-table" tabindex="0"><table>
    <thead><tr>${table.head.map((rs, i) => cell(rs, i, "th"))}</tr></thead>
    <tbody>${table.rows.map((row) => html`<tr>${row.map((rs, i) => cell(rs, i, "td"))}</tr>`)}</tbody>
  </table></div>`;
}

function Item({ b }) {
  const mark = b.item.task !== undefined
    ? html`<span class=${`md-task${b.item.task ? " md-done" : ""}`} aria-label=${b.item.task ? "done" : "to do"}>${b.item.task ? "☑" : "☐"}</span>`
    : html`<span class="md-mark" aria-hidden="true">${b.item.marker || "•"}</span>`;
  return html`<li class=${`md-item md-d${b.item.depth}`}>${mark}<span class="md-text">${runs(b.runs)}</span></li>`;
}

// Consecutive items are one list.
function Blocks({ list }) {
  const out = [];
  for (let i = 0; i < list.length; i++) {
    const b = list[i];
    if (b.item) {
      const items = [];
      while (list[i]?.item) items.push(list[i++]);
      i--;
      out.push(html`<ul class="md-list">${items.map((it) => html`<${Item} b=${it} />`)}</ul>`);
    } else if (b.code) out.push(html`<${Code} rows=${b.code} />`);
    else if (b.picture)
      out.push(html`<p class="markdown-picture"><${Icon} name="image" size=${16} /><span>${b.picture} · open it at your desk</span></p>`);
    else if (b.head) out.push(html`<h4 class=${`md-h${Math.min(b.head, 3)}`}>${runs(b.runs)}</h4>`);
    else if (b.para) out.push(html`<p>${runs(b.para)}</p>`);
    else if (b.quote) out.push(html`<blockquote class="md-quote"><${Blocks} list=${b.quote} /></blockquote>`);
    else if (b.table) out.push(html`<${Table} table=${b.table} />`);
    else if (b.raw !== undefined) out.push(html`<pre class="md-raw">${b.raw}</pre>`);
    else if (b.rule) out.push(html`<hr class="md-rule" />`);
  }
  return out;
}

export function Markdown({ text, breaks = false }) {
  return html`<div class="markdown" dir="auto"><${Blocks} list=${blocks(text, breaks)} /></div>`;
}
