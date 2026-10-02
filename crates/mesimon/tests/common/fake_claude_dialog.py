#!/usr/bin/env python3
"""A stand-in for Claude Code's AskUserQuestion dialog (T-571).

It draws the screens measured on Claude Code 2.1.287 and walks them under the
keys tmux sends, so an answer's walk is tested against the shape it was built
from (the daemon's unit tests hold the captured screens themselves):

* one question with one choice: ` ☐ Header`, the question, numbered rows with
  their descriptions, `Type something.`, a rule, `Chat about this`, and the
  footer; Enter on a row answers it at once.
* a batch, or a question that takes several choices: the tab bar
  `←  ☐ A  ☒ B  ✔ Submit  →` (☒ once a tab has an answer), the question,
  then rows. A one-choice row's Enter answers it and moves to the next tab. A
  several-choice row is `[ ] label` or `[✔] label`; Space or Enter ticks it,
  the text row is `[ ] Type something` and pasted words tick it, and the
  unnumbered row under it reads `Next`, or `Submit` on the last question.
  After the last question comes the review: `Review your answers`, a
  `● question` / `→ answer` pair per question (ticks in the order they were
  made, joined by `, `), `Ready to submit your answers?`, `1. Submit answers`
  and `2. Cancel`, and no footer.

* a plan (T-582), when the tool input is `{"plan": "..."}`: the plan's lines,
  `Would you like to proceed?` and the three rows measured on 2.1.287 (T-573,
  row 6) — `1. Yes, auto-accept edits`, `2. Yes, manually approve edits`,
  `3. Tell Claude what to change` — the cursor on the first. Enter on either
  `Yes` row accepts it: the row's label is written to `accepted-<ticket>`.
  Enter on the third row does nothing here (it opens a text box nobody types
  into in these tests).

Escape anywhere declines. The dialog is `dialog-<MESIMON_TICKET>.json` next to
this file, the tool input (`{"questions": [...]}` or `{"plan": "..."}`),
written by the test. While
it is absent the pane paints Claude's composer and appends each line typed or
pasted into it to `got.txt`, as the shell stubs do. An answer is written to
`answered-<ticket>.json` as Claude's `tool_response.answers` reads (question
to answer), a refusal to `declined-<ticket>`, and the dialog file is removed.
Every chunk of keys read is appended to `keys-<ticket>`. A `banner` file next
to this one, when there is one, is drawn above everything (the relay's tests
wait for theirs before they go on).
"""
import json
import os
import select
import sys
import termios
import tty

HERE = os.path.dirname(os.path.abspath(__file__))
TICKET = os.environ.get("MESIMON_TICKET", "none")
DIALOG = os.path.join(HERE, f"dialog-{TICKET}.json")
RULE = "─" * 60


def append(name, text):
    with open(os.path.join(HERE, name), "a") as f:
        f.write(text)


def banner():
    try:
        with open(os.path.join(HERE, "banner")) as f:
            return f.read().splitlines()
    except OSError:
        return []


def load():
    try:
        with open(DIALOG) as f:
            tool_input = json.load(f)
        if "plan" in tool_input:
            return tool_input
        return tool_input["questions"]
    except (OSError, ValueError, KeyError, TypeError):
        return None


class Plan:
    ROWS = ["Yes, auto-accept edits", "Yes, manually approve edits", "Tell Claude what to change"]

    def __init__(self, tool_input):
        self.plan = tool_input["plan"]
        self.cursor = 0
        self.accepted = None

    def draw(self):
        lines = [RULE, " Ready to code?", "", " Here is Claude's plan:", ""]
        lines += ["  " + line for line in self.plan.splitlines()]
        lines += ["", " Would you like to proceed?", ""]
        for i, row in enumerate(self.ROWS):
            lines.append((" ❯ " if self.cursor == i else "   ") + f"{i + 1}. {row}")
        return lines

    def key(self, kind, text=""):
        if kind == "esc":
            return "decline"
        if kind == "up":
            self.cursor = max(self.cursor - 1, 0)
        elif kind == "down":
            self.cursor = min(self.cursor + 1, len(self.ROWS) - 1)
        elif kind == "enter" and self.cursor < 2:
            self.accepted = self.ROWS[self.cursor]
            return "accept"
        return None


class Dialog:
    def __init__(self, questions):
        self.qs = questions
        self.batched = len(questions) > 1 or any(q.get("multiSelect") for q in questions)
        self.tab = 0
        self.cursor = 0
        self.picked = [None] * len(questions)
        self.ticks = [[] for _ in questions]
        self.texts = [""] * len(questions)
        self.text_ticked = [False] * len(questions)

    def multi(self):
        return self.tab < len(self.qs) and self.qs[self.tab].get("multiSelect")

    def rows(self):
        """How many cursor positions the tab has."""
        if self.tab == len(self.qs):
            return 2
        n = len(self.qs[self.tab]["options"])
        return n + 3 if self.multi() else n + 2

    def answered(self, i):
        return self.picked[i] is not None or bool(self.ticks[i]) or self.text_ticked[i]

    def answer(self, i):
        q = self.qs[i]
        if q.get("multiSelect"):
            words = [q["options"][o]["label"] if o >= 0 else self.texts[i] for o in self.ticks[i]]
            return ", ".join(words)
        if self.picked[i] == "text":
            return self.texts[i]
        return q["options"][self.picked[i]]["label"]

    def draw(self):
        lines = []
        mark = lambda at: "❯ " if self.cursor == at else "  "
        if self.batched:
            tabs = "  ".join(("☒ " if self.answered(i) else "☐ ") + q["header"]
                             for i, q in enumerate(self.qs))
            lines += [RULE, f"←  {tabs}  ✔ Submit  →", ""]
        else:
            lines += [RULE, " ☐ " + self.qs[0]["header"], ""]
        if self.tab == len(self.qs):
            lines += ["Review your answers", ""]
            for i, q in enumerate(self.qs):
                lines += [" ● " + q["question"], "   → " + self.answer(i)]
            lines += ["", "Ready to submit your answers?", "",
                      mark(0) + "1. Submit answers", mark(1) + "2. Cancel"]
            return lines
        q = self.qs[self.tab]
        n = len(q["options"])
        lines += [q["question"], ""]
        if self.multi():
            for i, o in enumerate(q["options"]):
                tick = "✔" if i in self.ticks[self.tab] else " "
                lines.append(f"{mark(i)}{i + 1}. [{tick}] {o['label']}")
                if o.get("description"):
                    lines.append("         " + o["description"])
            tick = "✔" if self.text_ticked[self.tab] else " "
            lines.append(f"{mark(n)}{n + 1}. [{tick}] {self.texts[self.tab] or 'Type something'}")
            button = "Submit" if self.tab == len(self.qs) - 1 else "Next"
            lines.append(("❯    " if self.cursor == n + 1 else "     ") + button)
            chat = n + 2
        else:
            for i, o in enumerate(q["options"]):
                lines.append(f"{mark(i)}{i + 1}. {o['label']}")
                if o.get("description"):
                    lines.append("     " + o["description"])
            lines.append(f"{mark(n)}{n + 1}. {self.texts[self.tab] or 'Type something.'}")
            chat = n + 1
        # Numbered after the text row either way: the button has no number.
        lines += [RULE, f"{mark(chat)}{n + 2}. Chat about this", ""]
        nav = "Tab/Arrow keys to navigate" if self.batched else "↑/↓ to navigate"
        lines.append(f"Enter to select · {nav} · Esc to cancel")
        return lines

    def advance(self):
        self.tab += 1
        self.cursor = 0
        if not self.batched or self.tab > len(self.qs):
            return "submit"
        return None

    def key(self, kind, text=""):
        """Apply one key; `submit` or `decline` when the dialog ends."""
        if kind == "esc":
            return "decline"
        if kind == "up":
            self.cursor = max(self.cursor - 1, 0)
        elif kind == "down":
            self.cursor = min(self.cursor + 1, self.rows() - 1)
        elif self.tab == len(self.qs):
            if kind == "enter" and self.cursor == 0:
                return "submit"
        elif kind == "paste" or kind == "char":
            n = len(self.qs[self.tab]["options"])
            if self.cursor == n:
                self.texts[self.tab] += text
                if self.multi() and self.texts[self.tab] and not self.text_ticked[self.tab]:
                    self.text_ticked[self.tab] = True
                    self.ticks[self.tab].append(-1)
        elif self.multi():
            n = len(self.qs[self.tab]["options"])
            ticks = self.ticks[self.tab]
            if kind in ("space", "enter") and self.cursor < n:
                if self.cursor in ticks:
                    ticks.remove(self.cursor)
                else:
                    ticks.append(self.cursor)
            elif kind == "enter" and self.cursor == n:
                self.text_ticked[self.tab] = not self.text_ticked[self.tab]
                if -1 in ticks:
                    ticks.remove(-1)
                if self.text_ticked[self.tab]:
                    ticks.append(-1)
            elif kind == "space" and self.cursor == n:
                self.texts[self.tab] += " "
            elif kind == "enter" and self.cursor == n + 1 and self.answered(self.tab):
                return self.advance()
        elif kind == "enter":
            n = len(self.qs[self.tab]["options"])
            if self.cursor < n:
                self.picked[self.tab] = self.cursor
            elif self.cursor == n and self.texts[self.tab]:
                self.picked[self.tab] = "text"
            else:
                return None
            if not self.batched:
                return "submit"
            return self.advance()
        return None


def tokens(data):
    """Split what tmux sent into keys: pastes, arrows, Escape, Enter, Space."""
    i = 0
    while i < len(data):
        if data.startswith(b"\x1b[200~", i):
            end = data.find(b"\x1b[201~", i)
            end = len(data) if end < 0 else end
            yield "paste", data[i + 6:end].decode("utf-8", "replace")
            i = end + 6
        elif data.startswith((b"\x1b[A", b"\x1bOA"), i):
            yield "up", ""
            i += 3
        elif data.startswith((b"\x1b[B", b"\x1bOB"), i):
            yield "down", ""
            i += 3
        elif data.startswith(b"\x1b[", i) or data.startswith(b"\x1bO", i):
            i += 3
        elif data[i:i + 1] == b"\x1b":
            yield "esc", ""
            i += 1
        elif data[i:i + 1] in (b"\r", b"\n"):
            yield "enter", ""
            i += 1
        elif data[i:i + 1] == b" ":
            yield "space", ""
            i += 1
        else:
            j = i + 1
            while j < len(data) and data[j] >= 0x80 and data[j] < 0xC0:
                j += 1
            yield "char", data[i:j].decode("utf-8", "replace")
            i = j


def main():
    fd = sys.stdin.fileno()
    tty.setcbreak(fd)
    attrs = termios.tcgetattr(fd)
    attrs[0] &= ~termios.ICRNL
    termios.tcsetattr(fd, termios.TCSANOW, attrs)
    # Claude asks for bracketed paste, so tmux brackets what it pastes.
    sys.stdout.write("\x1b[?2004h")
    dialog, source, typed, drawn = None, None, "", None
    while True:
        questions = load()
        if questions is None:
            dialog, source = None, None
        elif json.dumps(questions) != source:
            shape = Plan if isinstance(questions, dict) else Dialog
            dialog, source = shape(questions), json.dumps(questions)
        top = "\x1b[2J\x1b[H" + "".join(line + "\r\n" for line in banner())
        if dialog:
            screen = top + "\r\n".join(dialog.draw())
        else:
            screen = top + "\x1b[999;1H\x1b[3A" + "\r\n".join(
                [RULE, "❯ " + typed, RULE, "  ? for shortcuts"])
        if screen != drawn:
            sys.stdout.write(screen)
            sys.stdout.flush()
            drawn = screen
        if not select.select([fd], [], [], 0.1)[0]:
            continue
        data = os.read(fd, 65536)
        append(f"keys-{TICKET}", repr(data) + "\n")
        for kind, text in tokens(data):
            if not dialog:
                if kind == "enter":
                    append("got.txt", typed + "\n")
                    typed = ""
                elif kind == "paste":
                    *whole, typed = (typed + text).replace("\r", "\n").split("\n")
                    for line in whole:
                        append("got.txt", line + "\n")
                elif kind in ("char", "space"):
                    typed += text or " "
                continue
            ended = dialog.key(kind, text)
            if ended == "accept":
                with open(os.path.join(HERE, f"accepted-{TICKET}"), "w") as f:
                    f.write(dialog.accepted)
            elif ended == "submit":
                answers = {q["question"]: dialog.answer(i) for i, q in enumerate(dialog.qs)}
                with open(os.path.join(HERE, f"answered-{TICKET}.json"), "w") as f:
                    json.dump(answers, f)
            elif ended == "decline":
                append(f"declined-{TICKET}", "declined\n")
            if ended:
                os.remove(DIALOG)
                dialog, source = None, None
                break


main()
