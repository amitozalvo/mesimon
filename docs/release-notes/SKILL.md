---
name: release-notes
description: Write or rewrite release notes and changelog entries in plain language, with concrete user-visible changes, accurate release scope, and actionable upgrade details.
---

# Release notes

Write for someone deciding whether to update and what to do afterward. They
should understand each change on the first read, without knowing the codebase
or the conversation that produced it.

## Establish the facts

- Identify the release range and intended readers. Use the requested scope;
  when rewriting a changelog, preserve its versions, dates, and release order.
- Read the existing notes and release workflow. For new notes, inspect the
  relevant commits, diffs, tests, and product documentation. Commit subjects
  are leads to investigate, not finished release notes.
- For each candidate item, establish what changed, who encounters it, the
  observable result, and any action or limitation. Keep evidence references
  while drafting; do not invent benefits, measurements, defaults, or guarantees.
- Historical notes describe behavior in that release. Check ambiguous claims
  against the code at its tag, not just today's code. Flag an unresolved claim
  in the handoff instead of silently making it sound certain.

## Choose what belongs

- Include features, changed behavior, fixes, removals, compatibility changes,
  and known limitations that affect users of this release.
- Group related commits into one user-visible change. Split an item when it
  contains unrelated changes; do not hide them under "Smaller" or "Other."
- Omit refactoring, test plumbing, debugging chronology, and development
  anecdotes unless they change something the intended reader needs to know.
- Keep required migration steps, changed defaults, opt-in status, affected
  platforms, activation steps, and material exceptions. Brevity must not erase
  information needed to use the feature safely or correctly.

## Write the point first

- Start each bullet with the actual change, preferably as a short bold sentence.
  A reader scanning only those sentences should still learn what shipped.
- Follow with how to use it or when the fix matters. Aim for one to three short
  sentences per bullet. Add detail when it changes the reader's next action;
  move long procedures to documentation when a suitable link exists.
- For a feature: state the capability, then its control and relevant scope.
  For a fix: name the affected action or symptom and the corrected behavior.
  For a breaking change: state what stopped working and what to use instead.
- Use concrete subjects and verbs: "Press `!` to open a terminal" or "Reloading
  after a Linux update no longer fails with a missing-file error."
- Name actual keys, menu rows, commands, and affected platforms. Explain a
  technical term when the audience needs it; retain terms that identify the
  feature accurately.
- Remove jokes, metaphors, personification, rhetorical questions, suspense,
  slogans, self-congratulation, and invented terminology. Do not write a story
  about discovering or fixing the problem.
- Replace vague claims such as "better performance," "more robust," "seamless,"
  and "various improvements" with the specific observable change. If the
  evidence does not support a specific claim, leave it out.
- Do not manufacture a benefit sentence for every item. "Titles can contain up
  to 2 KB" is clearer than an invented claim about productivity.

## Organize for scanning

Put required upgrade actions first, then the most consequential changes. Use
plain category headings such as `Added`, `Changed`, and `Fixed` when they help
scan a longer release. Omit empty sections; a one-item release needs one bullet.
Skip introductions and closing summaries that repeat the bullets.

For Mesimon, edit `CHANGELOG.md`. Preserve `## <tag> — <date>` headings: both
`crates/mesimon-core/src/relnotes.rs` and `ci/release.sh` consume them. Use `###`
for categories and simple Markdown that works in the terminal viewer. Spell
shortcuts clearly, for example `Ctrl+Shift+S`, while retaining their exact key
meaning. A local rewrite affects future builds; existing binaries and published
GitHub release bodies need separate updates if those are requested.

## Review before handing off

Read only the first sentence of each bullet. Does each state a concrete change?
Then read the rest: does it add necessary use, scope, or limitation information?
Delete sentences that only explain the author's feelings or implementation
journey. Compare the result with the source notes or release diff to catch lost
changes, altered defaults, unsupported claims, and accidental version mixing.

Run the repository's relevant document/parser checks and inspect the diff.
Report the edited scope, checks, and any facts that still need confirmation.
Drafting notes does not itself request a new release or artifact upload.

## Examples

Before: "The archive reclaims a landed worktree."

After: "**Archiving a merged ticket removes its worktree.** Unmerged work and
worktrees with session panes are kept. Snoozing does not remove the worktree."

Before: "A reload mid-tool no longer reads as a dead turn."

After: "**Agents remain marked as working after a daemon restart during a tool
call.** Previously, a long-running tool could leave the card incorrectly marked
as idle."

## Editorial references

- [GitHub release-note guidance](https://docs.github.com/en/contributing/style-guide-and-content-model/style-guide#release-notes): answer the reader's questions about the affected behavior and use.
- [Google voice and tone](https://developers.google.com/style/tone): use direct language and avoid figurative or overly playful writing.
- [Keep a Changelog](https://keepachangelog.com/en/1.1.0/): curate notable changes for readers and group them by dated release.
