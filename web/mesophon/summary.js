// A note's Summary, read as the desk reads it (`core/src/summary.rs`,
// T-696): under a heading named `Summary`, a task item (`- [ ] …`,
// `- [x] …`) is a box the card counts, and every other line is the note's
// own. `extract` lists the boxes in document order; `toggle` flips one in
// the body's text and gives the body to write back. The two files agree on
// what a box is and share nothing else: a rule changed there changes here.

// A markdown heading: its level and its words. Up to three leading blanks.
function heading(line) {
  const s = line.replace(/^ {1,3}/, "");
  const level = /^#*/.exec(s)[0].length;
  if (level === 0 || level > 6) return undefined;
  const rest = s.slice(level);
  if (rest && rest[0] !== " " && rest[0] !== "\t") return undefined;
  return { level, text: rest.trim().replace(/#+$/, "").trimEnd() };
}

const fence = (line) => /^\s*(```|~~~)/.test(line);

// A list item's words, past its marker: `-`, `*`, `+`, `1.` or `1)`.
function item(line) {
  const t = line.trimStart();
  if ("-*+".includes(t[0] || " ")) return t[1] === " " ? t.slice(2) : undefined;
  const digits = /^\d*/.exec(t)[0].length;
  if (digits === 0 || digits > 9) return undefined;
  const rest = t.slice(digits);
  return (rest[0] === "." || rest[0] === ")") && rest[1] === " " ? rest.slice(2) : undefined;
}

// `[ ] rest` or `[x] rest` at an item's head; a box with nothing after it
// is still a box.
function taskBox(text) {
  const mark = text.slice(0, 3);
  if (!["[ ]", "[x]", "[X]"].includes(mark)) return undefined;
  if (text.length > 3 && text[3] !== " ") return undefined;
  return { done: mark !== "[ ]", text: text.slice(4) };
}

// The lines inside every Summary section: from its heading to the next of
// the same or a higher level, a fenced block skipped whole.
function sectionLines(body) {
  const out = [];
  let open;
  let fenced = false;
  body.split("\n").forEach((raw, n) => {
    const line = raw.endsWith("\r") ? raw.slice(0, -1) : raw;
    if (fence(line)) {
      fenced = !fenced;
      return;
    }
    if (fenced) return;
    const h = heading(line);
    if (h) {
      if (open !== undefined) {
        if (h.level <= open) open = undefined;
        else return;
      }
      if (open === undefined && h.text.toLowerCase() === "summary") open = h.level;
      return;
    }
    if (open !== undefined) out.push([n, line]);
  });
  return out;
}

export function extract(body) {
  const rows = [];
  for (const [line, text] of sectionLines(typeof body === "string" ? body : "")) {
    const words = item(text);
    const box = words === undefined ? undefined : taskBox(words);
    if (box) rows.push({ line, text: box.text.trim(), done: box.done });
  }
  return rows;
}

// `body` with the box on `line` flipped, every other byte kept; nothing
// when that line holds no box (the row is older than the body).
export function toggle(body, line) {
  const pieces = body.split(/(?<=\n)/);
  const piece = pieces[line];
  if (piece === undefined) return undefined;
  const text = piece.replace(/[\r\n]+$/, "");
  const words = item(text);
  const box = words === undefined ? undefined : taskBox(words);
  if (!box) return undefined;
  const head = text.length - words.length;
  pieces[line] = `${text.slice(0, head)}${box.done ? "[ ]" : "[x]"}${words.slice(3)}${piece.slice(text.length)}`;
  return pieces.join("");
}
