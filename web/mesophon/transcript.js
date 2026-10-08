// The agent's conversation, read from its transcript file a page at a time
// (T-626). The file is only appended to, so a row's `at` (its record's byte
// offset) is its place for good: a page wholly before the tail never
// changes, and only what was written since the last ask is asked again.
import { html, useLayoutEffect, useRef, useState } from "./html.js";
import { Icon, Tick } from "./icons.js";
import { ghostOf, receiptTick } from "./sessions.js";
import { Linked, Markdown } from "./markdown.js";

const clock = (at) => new Date(at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

// What the page holds of one conversation: its rows oldest first, where the
// held part ends (`end`) and where the page before it does (`floor`, null at
// the file's start).
export function mergePage(chat, asked, reply) {
  const rows = Array.isArray(reply.rows) ? reply.rows : [];
  const floor = Number.isInteger(reply.next_before) ? reply.next_before : null;
  const same = chat && chat.conversation === reply.conversation;
  if (asked.before != null) {
    // An earlier page joins only where the held part begins.
    if (!same || reply.end !== chat.floor) return chat;
    return { ...chat, rows: [...rows, ...chat.rows], floor };
  }
  // What was written since: the rows past `from` are replaced, the rest kept.
  if (same && asked.after != null && reply.from <= chat.end)
    return { ...chat, rows: [...chat.rows.filter((r) => r.at < reply.from), ...rows], end: reply.end };
  // A new conversation, or more written than one page holds: start over at
  // the tail, and earlier pages are asked again as the reader scrolls.
  return { conversation: reply.conversation, rows, end: reply.end, floor };
}

// The ask for the tail: only what was written since what is held.
export const tailAsk = (chat) => (chat ? { after: chat.end, conversation: chat.conversation } : {});

// A row's key: its record's offset and its place among that record's rows,
// so a page put on top re-keys nothing below it.
const rowKey = (rows, i) => {
  let n = 0;
  while (i - n - 1 >= 0 && rows[i - n - 1].at === rows[i].at) n++;
  return `${rows[i].at}.${n}`;
};

// A step's line, as the host spells it (`Read src/main.rs`, `Bash(cargo
// test)`), split into the tool's name and what it was given (T-701): the
// name reads in the body face, the argument in mono.
export function stepParts(text) {
  const m = /^([A-Za-z_][\w.-]*)\s*(.*)$/s.exec(text || "");
  if (!m) return { name: "", arg: text || "" };
  let arg = m[2];
  if (arg.startsWith("(") && arg.endsWith(")")) arg = arg.slice(1, -1);
  return { name: m[1], arg };
}

// Three steps or more in a row fold to one line (T-701): how many, and the
// tools' names in order, once each. A press unfolds them.
export const FOLD_FROM = 3;
export function foldWords(rows) {
  const names = [];
  for (const row of rows) {
    const { name } = stepParts(row.text);
    if (name && !names.includes(name)) names.push(name);
  }
  return `${rows.length} steps${names.length ? ` · ${names.join(", ")}` : ""}`;
}

// The rows grouped for drawing: a run of FOLD_FROM or more tool rows is
// one item, the rest are themselves.
export function groupRows(rows) {
  const items = [];
  let i = 0;
  while (i < rows.length) {
    if (rows[i].kind !== "tool") {
      items.push({ row: rows[i], i });
      i++;
      continue;
    }
    let j = i;
    while (j < rows.length && rows[j].kind === "tool") j++;
    if (j - i >= FOLD_FROM) items.push({ run: rows.slice(i, j), i });
    else for (let k = i; k < j; k++) items.push({ row: rows[k], i: k });
    i = j;
  }
  return items;
}

function Step({ text }) {
  const { name, arg } = stepParts(text);
  // The space between the spans keeps the line's text one line.
  return html`<${Icon} name="terminal" size=${12} width=${2.2} />${name && html`<span class="step-name">${name}</span>${" "}`}<span class="step-arg">${arg}</span>`;
}

function Row({ row }) {
  if (row.kind === "prompt")
    return html`<div class="chat-row chat-prompt"><p dir="auto"><${Linked} text=${row.text} /></p></div>`;
  if (row.kind === "reply")
    return html`<div class="chat-row chat-reply"><${Markdown} text=${row.text} /></div>`;
  if (row.kind === "tool")
    return html`<p class="chat-row chat-tool" dir="auto"><${Step} text=${row.text} /></p>`;
  return html`<p class="chat-row chat-notice" dir="auto"><span>${row.text}</span></p>`;
}

// Newest at the bottom. Scrolling near the top asks for the page before; a
// page put on top keeps the rows under the reader's eye where they were.
// `tail` is drawn after the rows: the dialog waiting on you (T-701).
export function Chat({ store, entry, doing, tail }) {
  const ref = useRef();
  const seen = useRef({});
  // The runs unfolded by a press, by their first row's key.
  const [open, setOpen] = useState(() => new Set());
  const chat = entry?.chat;
  const ghost = ghostOf(entry);
  useLayoutEffect(() => {
    const node = ref.current;
    if (!node || !node.getClientRects().length) return;
    const last = seen.current;
    const first = chat?.rows[0]?.at;
    const key = `${entry?.key}|${chat?.conversation}`;
    if (last.key !== key) node.scrollTop = entry?.chatFollowing === false ? entry.chatScroll || 0 : node.scrollHeight;
    else if (last.first !== undefined && first !== undefined && first < last.first)
      node.scrollTop += node.scrollHeight - last.height;
    else if (entry?.chatFollowing !== false) node.scrollTop = node.scrollHeight;
    seen.current = { key, first, height: node.scrollHeight };
    // A page too short to scroll asks for the one before by itself.
    if (chat?.floor != null && node.scrollHeight <= node.clientHeight) store.olderSoon();
  });
  const rows = chat?.rows || [];
  // Away (T-698), the conversation is what this page last held or the
  // shelf's newest page: earlier pages wait for the terminal.
  const away = !store.live;
  const empty = !chat
    ? entry?.chatError || "Reading the conversation…"
    : rows.length || chat.floor != null
      ? ""
      : "Nothing said yet.";
  const unfold = (key) => setOpen((was) => new Set([...was, key]));
  return html`<div id="chat" ref=${ref} class="chat" aria-label="Conversation" tabindex="0"
      onScroll=${(e) => store.chatScrolled(e.currentTarget)}>
    ${chat?.floor != null && html`<p class="chat-edge" aria-live="polite">${away ? "Earlier parts need your terminal" : entry.chatOlder ? "Reading earlier…" : "Scroll up for earlier"}</p>`}
    ${chat && chat.floor == null && rows.length > 0 && html`<p class="chat-edge">Start of the conversation</p>`}
    ${empty && html`<p class="chat-empty">${empty}</p>`}
    ${groupRows(rows).map((item) => {
      const key = rowKey(rows, item.i);
      if (!item.run || open.has(key)) {
        if (!item.run) return html`<${Row} key=${key} row=${item.row} />`;
        return item.run.map((row, n) => html`<${Row} key=${rowKey(rows, item.i + n)} row=${row} />`);
      }
      return html`<button key=${key} type="button" class="chat-row chat-run" aria-expanded="false"
        onClick=${() => unfold(key)}><${Icon} name="chevronRight" size=${12} width=${2.4} /><span>${foldWords(item.run)}</span></button>`;
    })}
    ${ghost && html`<div class="chat-row chat-prompt chat-ghost" aria-label="Sent, not yet in the conversation">
      <p dir="auto">${ghost.text}</p><${Tick} state=${receiptTick(ghost.status) || "clock"} /></div>`}
    ${tail}
    ${doing && !away && html`<p class="chat-row chat-doing" dir="auto"><span class="dot" aria-hidden="true"></span><${Step} text=${doing} /></p>`}
    ${away && chat && entry.chatAt && html`<p id="chat-as-of" class="chat-edge">As of ${clock(entry.chatAt)}</p>`}
  </div>`;
}
