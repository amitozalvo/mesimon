"""Offline contracts for the Claude rig's ordered case steps."""
from pathlib import PurePath
import re
from string import Formatter


# check_ticket evaluates these on the ticket, board or crown even without a seat.
AGENTLESS_CHECKS = set("""
    woke woke_once no_wake_after crown_hand crown_reply crown_line crown_obstacle
    crown crown_board peer_fed board_exchange filed crown_wrote_nothing title_words
    title_never title_is title_match described create_start hand_said hand_unread
    refusal_named wake_said wake_count
""".split())

# Steps that operate on the test ticket's worker. The forge setup/teardown
# steps (forge_lab and forge_off) can run before a worker exists.
WORKER_STEPS = set("""
    prompt prompt_at_start press_enter shell picture_ask file_ask resend still_waiting ping
    pane_keys draft_at_launch queue_in_compaction await_compaction crash limit fail
    person_idles person_leaves train_wait forge_pr forge_merged_elsewhere forge_merged_running forge_closed
    forge_empty forge_bound_passed forge_fail forge_pass forge_merged_pass
    forge_merge_ready forge_merge forge_later forge_conflict forge_clear forge_restart
    forge_later_news forge_later_open prs_board forge_reviewed forge_prs forge_merge_all
    forge_archived forge_again merge_with_notice
""".split())


CROWN_STEPS = set("""
    person_in_crown person_leaves person_idles peer_ask peer_reply crown_road
    crown_away crown_rest crown_compact queue_in_compaction merge_with_notice
""".split())
BOARD_STEPS = set("""
    person_start rebase shared_shell person_note restart_drop_mod phone_allow phone_deny
    peer_board peer_stop hold_branch free_branch train_on train_off binary_moved
    binary_back restart restart_old_claude restart_mods_off restart_dead_launch
    async_review restart_plain gate_pipeline gate_off doctor_mcp doctor forge_lab forge_off
""".split())


def summary_rows(body):
    """The rows of a note's `Summary` sections as `core/src/summary.rs`
    reads them: (ticked | None for a plain row, text), plus the headings
    the note used, for the verdict of a note that put its list elsewhere."""
    rows, headings, open_level, fenced = [], [], None, False
    for line in body.splitlines():
        if line.lstrip().startswith(("```", "~~~")):
            fenced = not fenced
            continue
        if fenced:
            continue
        m = re.match(r" {0,3}(#{1,6})(?:[ \t]+(.*?))?[ \t]*#*[ \t]*$", line)
        if m:
            level, text = len(m.group(1)), (m.group(2) or "").strip()
            headings.append(text)
            if open_level is not None and level <= open_level:
                open_level = None
            if open_level is None and text.lower() == "summary":
                open_level = level
            continue
        if open_level is None or not line.strip():
            continue
        item = re.match(r"\s*(?:[-*+]|\d{1,9}[.)]) (.*)$", line)
        if item:
            box = re.match(r"\[( |x|X)\](?: (.*))?$", item.group(1))
            if box:
                rows.append((box.group(1) != " ", (box.group(2) or "").strip()))
                continue
            rows.append((None, item.group(1).strip()))
        else:
            rows.append((None, line.strip()))
    return rows, headings


def validate_case(case):
    keys = ["key"] + [f"key{i + 2}" for i in range(len(case.get("siblings", [])))]
    owners = {}
    start_keys = set()
    seen_steps = set()
    uncrowned = case.get("uncrowned", False)
    guard = case.get("merge_guard")
    at = "setup"

    def fail(reason):
        raise ValueError(f"{case['id']} {at}: {reason}")

    if case["workspace"] not in ("worktree", "shared_checkout"):
        fail(f"unknown workspace {case['workspace']!r}")
    if not isinstance(uncrowned, bool):
        fail("uncrowned must be a boolean")
    if guard not in (None, "fixture", "train_off"):
        fail(f"unknown merge_guard {guard!r}")
    if (guard or uncrowned) and case["workspace"] != "worktree":
        fail("a commit on a shared checkout bypasses the board's merge target")
    if case.get("wake_counts") not in (None, "delivered", "owed"):
        fail("wake_counts must name delivered or owed evidence")

    def check_worker_base():
        # A worker in a worktree can deliver even when its brief never says
        # "commit". Only rebase and the uncrowned runner supply their own
        # isolated histories. Other worktree starts declare their guard.
        if case["workspace"] == "worktree" and not (guard or uncrowned):
            fail("a worker commit could reach the real base; select a fixture or train_off guard")

    forge = False
    train = False

    def rig_step(step):
        nonlocal forge, train
        action = step["rig"]
        if action not in WORKER_STEPS | CROWN_STEPS | BOARD_STEPS:
            fail(f"unknown rig step {action!r}")
        if action in CROWN_STEPS and uncrowned:
            fail(f"{action} needs a crown")
        if action in ("peer_reply", "peer_ask", "peer_stop") and "peer_board" not in seen_steps:
            fail(f"{action} needs peer_board first")
        if action in ("person_idles", "person_leaves") and "person_in_crown" not in seen_steps:
            fail(f"{action} needs person_in_crown first")
        seen_steps.add(action)
        if "starts" in step:
            fail("a rig action determines its own producer")
        if action in ("person_start", "rebase"):
            if action == "person_start":
                check_worker_base()
            if "key" in owners:
                fail("the test ticket already has a worker")
            owners["key"] = "person"
            start_keys.add("key")
        if action in WORKER_STEPS:
            targets = keys if action in ("forge_prs", "forge_merge_all", "forge_archived") else ["key"]
            for key in targets:
                if key not in owners:
                    fail(f"{action} needs a worker on {key} but none has started")
        if action == "merge_with_notice" and guard != "fixture":
            fail("the notice merge needs a fixture branch")
        if action == "train_on":
            if guard != "fixture" or uncrowned:
                fail("train_on could land on the real base; use a fixture branch")
            train = True
        if action in ("train_on", "train_off") and "block" in step:
            block = step["block"]
            if guard != "fixture" or PurePath(block).name != block or block in ("", "..", "."):
                fail("a train blocker must be a filename inside the fixture checkout")
        if action == "train_off":
            train = False
        if action == "train_wait" and not train:
            fail("train_wait needs an armed train")
        if action in ("forge_pr", "forge_prs", "forge_lab", "prs_board", "forge_empty",
                      "forge_merged_elsewhere", "forge_merged_running", "forge_closed"):
            forge = True
        if action == "forge_off":
            forge = False

    def crown_step(step):
        if uncrowned:
            fail("a crown prompt needs a crowned case")
        words = step["crown"]
        try:
            fields = {key for _, key, _, _ in Formatter().parse(words) if key is not None}
        except ValueError as error:
            fail(str(error))
        unknown = fields.difference(keys, {"crown", "stub"})
        if unknown:
            fail(f"unknown ticket or substitution {sorted(unknown)}")
        until = step.get("until", {})
        if not isinstance(until, dict):
            fail("a crown wait needs an until table")
        unknown = until.keys() - {"crown_only", "crown_fed", "woke", "stops", "events",
                                  "state", "filed", "all", "on", "fed"}
        if unknown:
            fail(f"unknown until selector {sorted(unknown)}")
        scripted = re.findall(r"\bcall start_agent for \{(key\d*)\}", words, re.I)
        starts = step.get("starts", scripted)
        if not isinstance(starts, list):
            fail("starts must list ticket keys")
        workspace = re.search(r"\bwith workspace (worktree|shared_checkout)\b", words, re.I)
        if starts and workspace and workspace[1] != case["workspace"]:
            fail(f"the start asks for {workspace[1]}, the case declares {case['workspace']}")
        for key in starts:
            if key not in [*keys, "filed"]:
                fail(f"unknown start target {key!r}")
            if key != "filed" and key not in fields:
                fail(f"the crown's start does not name {{{key}}}")
            start_keys.add(key)
            if until.get("woke") != "unstarted":
                check_worker_base()
                owners.setdefault(key, "crown")
        if re.search(r"\bmerge_ticket\b", words) and not (forge or guard == "fixture"):
            fail("a local merge needs a fixture branch; a train hold cannot stop the crown")
        # Claude denies a standalone foreground sleep in the crown's Bash.
        # A sleep in words sent to the worker is the worker's command.
        for sentence in re.split(r"(?<=[.!?])\s+", words):
            sleep = re.search(r"\brun (?:the (?:shell )?command )?[`']?sleep (\d+)\b", sentence, re.I)
            if (sleep and int(sleep[1]) >= 90
                    and not re.search(r"ask_agent|\bagent\b|\bbackground\b", sentence, re.I)):
                fail("Claude denies the crown's standalone foreground sleep")

        on = until.get("on", "key")
        if on not in keys or ("on" in until and until.get("all")):
            fail(f"invalid worker selector {until}")
        # Where the prompt gives the ordering literally, no annotation is
        # needed. A mixed foreground/background brief can declare its edge.
        worker_words = (case["brief"] if starts else words).lower()
        background = (step.get("background_at_stop")
                      or ("in the background, without waiting for it" in worker_words
                          and "foreground" not in worker_words))
        if background and until.get("state", "idle") != "working":
            fail("a worker with a background task at Stop reads working; an idle wait drains the task")
        if until.get("crown_only") or until.get("crown_fed"):
            if any(k in until for k in ("stops", "events", "fed", "on", "all", "woke", "filed")):
                fail("a crown-only wait cannot also wait on a worker")
            if not step.get("wake"):
                return
        targets = keys if until.get("all") else [on]
        if until.get("filed"):
            if "filed" not in starts or step.get("wake") or "on" in until or until.get("all"):
                fail("filed waits on a child filed and started in this step")
            targets = ["filed"]
        for target in targets:
            producers = starts if until.get("woke") == "unstarted" else owners
            if target not in producers:
                fail(f"wait needs a worker on {target} but none has started")

    def verdict(check):
        name, _, arg = check.partition(":")
        if name == "summary" and any(box is not None for box, _ in summary_rows(case["brief"])[0]):
            fail("the description adds Summary boxes to the checklist the worker is asked to write")
        if name not in AGENTLESS_CHECKS:
            for key in keys:
                if key not in owners:
                    fail(f"needs an agent record on {key}")
                if name.startswith("started_by_") and name != "started_by_" + owners[key]:
                    fail(f"{key} was started by the {owners[key]}")
        if uncrowned and name.startswith(("crown", "woke", "wake_", "no_wake", "started_by_crown")):
            fail("this verdict needs a crown")
        if name in ("woke_once", "wake_count"):
            evidence = "owed" if name == "woke_once" else "delivered"
            if case.get("wake_counts") != evidence:
                fail(f"{name} counts {evidence} wakes; declare the intended wake_counts and use its counter")
            if name == "wake_count" and not re.fullmatch(r".+=\d+", arg):
                fail("wake_count needs words=count")

    for i, step in enumerate(case["steps"], 1):
        at = f"step {i}"
        if ("crown" in step) == ("rig" in step):
            fail("expected one crown or rig action")
        if "rig" in step:
            rig_step(step)
        else:
            crown_step(step)
    if train:
        fail("the case leaves the train armed")
    for i, check in enumerate(case["checks"], 1):
        at = f"check {i} ({check})"
        verdict(check)
    return start_keys
