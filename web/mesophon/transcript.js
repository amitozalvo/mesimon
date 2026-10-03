// The agent's conversation, read from its transcript file a page at a time
// (T-626). The file is only appended to, so a row's `at` (its record's byte
// offset) is its place for good: a page wholly before the tail never
// changes, and only what was written since the last ask is asked again.
import { html, useLayoutEffect, useRef } from "./html.js";
import { Icon } from "./icons.js";
import { Markdown } from "./markdown.js";

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

function Row({ row }) {
  if (row.kind === "prompt")
    return html`<div class="chat-row chat-prompt"><p dir="auto">${row.text}</p></div>`;
  if (row.kind === "reply")
    return html`<div class="chat-row chat-reply"><${Markdown} text=${row.text} /></div>`;
  if (row.kind === "tool")
    return html`<p class="chat-row chat-tool" dir="auto"><${Icon} name="terminal" size=${13} /><span>${row.text}</span></p>`;
  return html`<p class="chat-row chat-notice" dir="auto"><span>${row.text}</span></p>`;
}

// Newest at the bottom. Scrolling near the top asks for the page before; a
// page put on top keeps the rows under the reader's eye where they were.
export function Chat({ store, entry, doing }) {
  const ref = useRef();
  const seen = useRef({});
  const chat = entry?.chat;
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
  const empty = !chat
    ? entry?.chatError || "Reading the conversation…"
    : rows.length || chat.floor != null
      ? ""
      : "Nothing said yet.";
  return html`<div id="chat" ref=${ref} class="chat" aria-label="Conversation" tabindex="0"
      onScroll=${(e) => store.chatScrolled(e.currentTarget)}>
    ${chat?.floor != null && html`<p class="chat-edge" aria-live="polite">${entry.chatOlder ? "Reading earlier…" : "Scroll up for earlier"}</p>`}
    ${chat && chat.floor == null && rows.length > 0 && html`<p class="chat-edge">Start of the conversation</p>`}
    ${empty && html`<p class="chat-empty">${empty}</p>`}
    ${rows.map((row, i) => html`<${Row} key=${rowKey(rows, i)} row=${row} />`)}
    ${doing && html`<p class="chat-row chat-doing" dir="auto"><span class="dot" aria-hidden="true"></span><span>${doing}</span></p>`}
  </div>`;
}
