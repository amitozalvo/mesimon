// The agent's conversation, read from its transcript file a page at a time
// (T-626). The file is only appended to, so a row's `at` (its record's byte
// offset) is its place for good: a page wholly before the tail never
// changes, and only what was written since the last ask is asked again.
import { html, useLayoutEffect, useRef, useState } from "./html.js";
import { Icon, Tick } from "./icons.js";
import { ghostOf, receiptTick } from "./sessions.js";
import { Linked, Markdown, tapCopy } from "./markdown.js";
import { keepAtEnd } from "./viewport.js";
import { startLine } from "./lists.js";

const clock = (at) => new Date(at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

// What the page holds of one conversation: its rows oldest first, where the
// held part ends (`end`) and where the page before it does (`floor`, null at
// the file's start).
// The crown's acts on the ticket (T-1264) come whole with every page from
// a host that reads them, and replace what was held; an older host's page
// has none, and `acts` stays undefined.
export function mergePage(chat, asked, reply) {
  const rows = Array.isArray(reply.rows) ? reply.rows : [];
  const floor = Number.isInteger(reply.next_before) ? reply.next_before : null;
  const same = chat && chat.conversation === reply.conversation;
  const acts = Array.isArray(reply.acts) ? reply.acts : same ? chat.acts : undefined;
  if (asked.before != null) {
    // An earlier page joins only where the held part begins.
    if (!same || reply.end !== chat.floor) return chat;
    return { ...chat, rows: [...rows, ...chat.rows], floor, acts };
  }
  // What was written since: the rows past `from` are replaced, the rest kept.
  if (same && asked.after != null && reply.from <= chat.end)
    return { ...chat, rows: [...chat.rows.filter((r) => r.at < reply.from), ...rows], end: reply.end, acts };
  // A new conversation, or more written than one page holds: start over at
  // the tail, and earlier pages are asked again as the reader scrolls.
  return { conversation: reply.conversation, rows, end: reply.end, floor, acts };
}

// The ask for the tail: only what was written since what is held.
export const tailAsk = (chat) => (chat ? { after: chat.end, conversation: chat.conversation } : {});

// A row's key: its record's offset and its place among that record's rows,
// so a page put on top re-keys nothing below it.
// Its kind and its conversation are in it too: a key that came back as
// another kind of row (another conversation's offset, a hand whose call
// landed with an earlier page) was redrawn in place and could land below
// the rows after it.
const rowKey = (rows, i, conversation) => {
  let n = 0;
  while (i - n - 1 >= 0 && rows[i - n - 1].at === rows[i].at) n++;
  return `${conversation}|${rows[i].kind}:${rows[i].at}.${n}`;
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

// Two steps or more in a row fold to one line (T-701, from three; T-1208,
// from two) that says what they did in words: files read, edited and
// written counted once each, searches and commands by how many, any other
// tool by its name. A press unfolds them, and folds them again.
export const FOLD_FROM = 2;
const KIND = {
  Read: "read", Edit: "edit", MultiEdit: "edit", NotebookEdit: "edit", apply_patch: "edit", Write: "write",
  Grep: "search", Glob: "search", WebSearch: "search", Bash: "run", shell: "run", exec_command: "run", local_shell: "run",
};
const files = (n) => `${n} ${n === 1 ? "file" : "files"}`;
const SAID = {
  read: (n) => `read ${files(n)}`,
  edit: (n) => `edited ${files(n)}`,
  write: (n) => `wrote ${files(n)}`,
  search: (n) => (n === 1 ? "searched once" : `searched ${n} times`),
  run: (n) => `ran ${n} ${n === 1 ? "command" : "commands"}`,
};
export function foldWords(rows) {
  const seen = new Map();
  for (const row of rows) {
    const { name, arg } = stepParts(row.text);
    const kind = KIND[name] || `=${name || "step"}`;
    if (!seen.has(kind)) seen.set(kind, { n: 0, args: new Set() });
    const of = seen.get(kind);
    of.n++;
    of.args.add(arg);
  }
  const parts = [...seen].map(([kind, of]) =>
    kind[0] === "="
      ? `${of.n} ${kind.slice(1)}`
      : SAID[kind](["read", "edit", "write"].includes(kind) ? of.args.size : of.n));
  const said = parts.join(", ");
  return said ? said[0].toUpperCase() + said.slice(1) : `${rows.length} steps`;
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

// A ticket's news among the rows (T-1245): each line after the rows the
// page held when it was said, one from another conversation (or before
// any was written) at the top. A line due above what is held waits until
// the start of the conversation is.
// `rows` stand in for the conversation's own (its rows with the hands
// placed).
export function withLines(chat, lines, rows = chat?.rows || []) {
  if (!chat || !lines?.length) return rows;
  const due = lines
    .map((line) => ({ kind: "said", line, after: line.conversation === chat.conversation ? line.after ?? Infinity : -1 }))
    .sort((a, b) => a.after - b.after || a.line.id - b.line.id);
  const out = [];
  let k = 0;
  const flush = (upTo) => {
    for (; k < due.length && due[k].after <= upTo; k++) if (out.length || chat.floor == null) out.push(due[k]);
  };
  for (const row of rows) {
    flush(row.at);
    out.push(row);
  }
  flush(Infinity);
  return out;
}

// Whether a step names mesimon's `raise_hand`, as Claude Code
// (`mcp__mesimon__raise_hand`) or Codex spells it (core's `is_raise_hand`).
const isRaiseHand = (name) => name === "raise_hand" || name.endsWith("__raise_hand") || name.endsWith(".raise_hand");
const words = (s) => (s || "").split(/\s+/).filter(Boolean).join(" ");

// The raised hands among the rows (T-1304), as the ticket page draws them
// (T-1155): a `raise_hand` call's reason (the agent's words right after the
// call, in the same record) is a hand, lowered unless the board's `hand` is
// up with its words, where the newest such is up. A hand up whose call is not
// among the rows (a Codex call from a code cell, an earlier page still to
// come) is placed by its time: after the last row dated in or before its
// second, and that row's record; last in a conversation with no clock, and
// nowhere when every row is later.
export function withHand(rows, hand) {
  let newest = -1;
  const out = rows.map((row, i) => {
    const step = rows[i - 1];
    if (row.kind !== "reply" || step?.kind !== "tool" || step.at !== row.at || !isRaiseHand(stepParts(step.text).name))
      return row;
    newest = i;
    return { ...row, kind: "hand", up: false };
  });
  if (!hand?.reason) return newest < 0 ? rows : out;
  if (newest >= 0 && words(out[newest].text) === words(hand.reason)) {
    out[newest] = { ...out[newest], up: true };
    return out;
  }
  const upTo = hand.ms + 999;
  let before = -1;
  for (let i = out.length - 1; i >= 0; i--) if (out[i].ms != null && out[i].ms <= upTo) { before = i; break; }
  if (before < 0) {
    if (!out.length || out.some((r) => r.ms != null)) return out;
    before = out.length - 1;
  }
  let at = before;
  while (at + 1 < out.length && out[at + 1].at === out[before].at) at++;
  out.splice(at + 1, 0, { at: out[before].at, kind: "hand", up: true, board: true, ms: hand.ms, text: hand.reason });
  return out;
}

// How long before an act a prompt it carried may be dated: the act's time
// is the host's clock, the row's the transcript's (T-1162's slack).
export const ACT_SLACK_MS = 5000;

// Where words separate, as Rust's `split_whitespace` reads them (Unicode
// White_Space): JS's `\s` also takes U+FEFF and leaves out U+0085.
const SPACE = /[\t\n\v\f\r \u0085\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000]+/;
const MASK = 0xffffffffffffffffn;
const utf8 = new TextEncoder();

// Words' key, as the host keys the words an act carried (core's
// `crownact::words_key`, T-1309): FNV-1a over the words joined by one
// space, sixteen hex digits.
// A prompt row's key is read once for its life: a row the page keeps is
// the same object, and its words never change.
const KEYS = new WeakMap();
const keyOf = (row) => {
  if (!KEYS.has(row)) KEYS.set(row, wordsKey(row.text));
  return KEYS.get(row);
};
export function wordsKey(text) {
  let h = 0xcbf29ce484222325n;
  for (const b of utf8.encode((text || "").split(SPACE).filter(Boolean).join(" ")))
    h = ((h ^ BigInt(b)) * 0x100000001b3n) & MASK;
  return h.toString(16).padStart(16, "0");
}

// The crown's acts among the rows (T-1264), placed as the ticket page places
// them (T-1162): each before the first row dated after it, one that carried
// words before the first prompt dated at or after it, one dated after every
// row at the end. One dated before what is held waits until the start of
// the conversation is.
// Words whose key the host sent (T-1309) mark the prompt that holds them,
// the first not yet marked and not dated before the act less the slack:
// that prompt is the crown's, or another board's crown's, not the person's.
export function withActs(chat, rows) {
  const acts = chat?.acts;
  if (!acts?.length) return rows;
  // Each row's time: its own, else the one before it's. A news line keeps
  // its place among the rows and dates nothing.
  let last = -Infinity;
  const times = rows.map((r) => (r.kind === "said" ? last : (last = r.ms ?? last)));
  const first = times.find((t) => t > -Infinity);
  const marks = new Map();
  const at = [];
  acts.forEach((act, n) => {
    if (act.words && !(chat.floor != null && !(first <= act.ms + ACT_SLACK_MS))) {
      const j = rows.findIndex((r, i) => r.kind === "prompt" && !marks.has(i)
        && (r.ms == null || r.ms + ACT_SLACK_MS >= act.ms) && keyOf(r) === act.words);
      if (j >= 0) return marks.set(j, act);
    }
    if (chat.floor != null && !(first <= act.ms)) return;
    const after = (i) => rows[i].kind !== "said" && times[i] > act.ms;
    const prompt = (i) => rows[i].kind === "prompt" && times[i] + ACT_SLACK_MS >= act.ms;
    let i = act.prompt ? rows.findIndex((_, i) => prompt(i)) : -1;
    if (i < 0) i = rows.findIndex((_, i) => after(i));
    at.push({ i: i < 0 ? rows.length : i, item: { kind: "act", act, n } });
  });
  if (!at.length && !marks.size) return rows;
  at.sort((a, b) => a.i - b.i || a.item.n - b.item.n);
  const out = [];
  let k = 0;
  rows.forEach((row, i) => {
    for (; k < at.length && at[k].i === i; k++) out.push(at[k].item);
    out.push(marks.has(i) ? { ...row, crown: marks.get(i) } : row);
  });
  for (; k < at.length; k++) out.push(at[k].item);
  return out;
}

// What the crown did, in the crown's ink (T-1264): the ticket page's row,
// its mark, its sentence and its time, then the crown's own words where it
// answered, and what started its turn on a quiet line.
const capital = (s) => (s ? s[0].toUpperCase() + s.slice(1) : s);
const Stamp = ({ ms }) => html`<time datetime=${new Date(ms).toISOString()}>${clock(ms)}</time>`;
// An act's head, as its row and the words it carried both draw it.
const ActHead = ({ act, ms, cls }) =>
  html`<p class=${cls} dir="auto"><${Icon} name=${act.road ? "zap" : "crown"} size=${13} width=${2.2} /><span>${capital(act.text)}</span><${Stamp} ms=${ms} /></p>`;
function Act({ act }) {
  return html`<div class=${`chat-row chat-act${act.road ? " chat-act-road" : ""}`}>
    <${ActHead} act=${act} ms=${act.ms} />
    ${act.body && html`<p class="chat-act-body" dir="auto">${act.body}</p>`}
    ${act.cause && html`<p class="chat-act-cause" dir="auto">${capital(act.cause)}</p>`}
  </div>`;
}

// Words mesimon carried in (T-1309), headed as the ticket page heads them
// (T-1162): who said them over the words, in the crown's ink, and off the
// person's bubble, since they are not the person's.
function Crowned({ row }) {
  const act = row.crown;
  const ms = row.ms ?? act.ms;
  return html`<div class=${`chat-row chat-crowned${act.road ? " chat-crowned-road" : ""}`}>
    <${ActHead} act=${act} ms=${ms} cls="chat-crowned-head" />
    <p class="chat-crowned-body" dir="auto"><${Linked} text=${row.text} /></p>
    ${act.cause && html`<p class="chat-crowned-cause" dir="auto">${capital(act.cause)}</p>`}
  </div>`;
}

// One line of news: its words, its tick, its time, and a button while it
// is offered. A start's line reads off the start as it goes.
function Said({ store, line }) {
  const now = line.start ? { ...line, ...startLine(line.start) } : line;
  const offered = !!line.action && Date.now() < line.until;
  return html`<p class=${`chat-row chat-said${now.err ? " chat-said-err" : ""}`} dir="auto"><span>${now.text}</span>${now.tick &&
    html`<${Tick} state=${now.tick} />`}<${Stamp} ms=${now.at} />${offered &&
    html`<button type="button" class="chat-said-act" onClick=${() => store.lineAction(line)}>${line.action.label}</button>`}</p>`;
}

// A raised hand (T-1304), as the ticket page draws it (T-1155): who raised
// it over the words, in the needs-you colour while it is up and quiet once
// it is down, so that colour never outlives the need.
function Hand({ row, provider }) {
  return html`<div class=${`chat-row chat-hand${row.up ? " chat-hand-up" : ""}`} aria-label=${row.up ? "Raised hand, waiting on you" : "Raised hand"}>
    <p class="chat-hand-head" dir="auto"><${Icon} name="hand" size=${13} width=${2.2} /><span>${provider || "claude"} raised a hand</span>${row.ms != null &&
      html`<${Stamp} ms=${row.ms} />`}</p>
    <p class="chat-hand-body" dir="auto"><${Linked} text=${row.text} /></p>
  </div>`;
}

function Step({ text }) {
  const { name, arg } = stepParts(text);
  // The space between the spans keeps the line's text one line.
  return html`<${Icon} name="terminal" size=${12} width=${2.2} />${name && html`<span class="step-name">${name}</span>${" "}`}<span class="step-arg" onClick=${tapCopy}>${arg}</span>`;
}

function Row({ row }) {
  if (row.kind === "prompt" && row.crown) return html`<${Crowned} row=${row} />`;
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
export function Chat({ store, entry, agent, tail }) {
  const { doing, hand, provider } = agent || {};
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
  useLayoutEffect(() => keepAtEnd(ref.current, () => entry?.chatFollowing !== false));
  const held = chat?.rows || [];
  // Away, the hand the board last said is not known to be up.
  const rows = withActs(chat, withLines(chat, store.linesOf(store.board?.current), withHand(held, store.live ? hand : null)));
  // Away (T-698), the conversation is what this page last held or the
  // shelf's newest page: earlier pages wait for the terminal.
  const away = !store.live;
  const empty = !chat
    ? entry?.chatError || "Reading the conversation…"
    : held.length || chat.floor != null
      ? ""
      : "Nothing said yet.";
  const toggle = (key) => setOpen((was) => {
    const now = new Set(was);
    if (!now.delete(key)) now.add(key);
    return now;
  });
  return html`<div id="chat" ref=${ref} class="chat" aria-label="Conversation" tabindex="0"
      onScroll=${(e) => store.chatScrolled(e.currentTarget)}>
    ${chat?.floor != null && html`<p class="chat-edge" aria-live="polite">${away ? "Earlier parts need your terminal" : entry.chatOlder ? "Reading earlier…" : "Scroll up for earlier"}</p>`}
    ${chat && chat.floor == null && held.length > 0 && html`<p class="chat-edge">Start of the conversation</p>`}
    ${empty && html`<p class="chat-empty">${empty}</p>`}
    ${groupRows(rows).map((item) => {
      if (item.row?.kind === "said") return html`<${Said} key=${`said:${item.row.line.id}`} store=${store} line=${item.row.line} />`;
      if (item.row?.kind === "act") return html`<${Act} key=${`act:${item.row.act.ms}.${item.row.n}`} act=${item.row.act} />`;
      if (item.row?.board) return html`<${Hand} key=${`${chat?.conversation}|hand:board`} row=${item.row} provider=${provider} />`;
      const key = rowKey(rows, item.i, chat?.conversation);
      if (item.row?.kind === "hand") return html`<${Hand} key=${key} row=${item.row} provider=${provider} />`;
      if (!item.run) return html`<${Row} key=${key} row=${item.row} />`;
      const shown = open.has(key);
      return html`<button key=${`run:${key}`} type="button" class="chat-row chat-run" aria-expanded=${String(shown)}
        onClick=${() => toggle(key)}><${Icon} name=${shown ? "chevronDown" : "chevronRight"} size=${12} width=${2.4} /><span>${foldWords(item.run)}</span></button>
        ${shown && item.run.map((row, n) => html`<${Row} key=${rowKey(rows, item.i + n, chat?.conversation)} row=${row} />`)}`;
    })}
    ${ghost && html`<div class="chat-row chat-prompt chat-ghost" aria-label="Sent, not yet in the conversation">
      <p dir="auto">${ghost.text}</p><${Tick} state=${receiptTick(ghost.status) || "clock"} /></div>`}
    ${tail}
    ${doing && !away && html`<p class="chat-row chat-doing" dir="auto"><span class="dot" aria-hidden="true"></span><${Step} text=${doing} /></p>`}
    ${away && chat && entry.chatAt && html`<p id="chat-as-of" class="chat-edge">As of ${clock(entry.chatAt)}</p>`}
  </div>`;
}
