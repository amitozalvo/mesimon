#!/usr/bin/env python3
"""Read installed harness releases; print a digest, never file tickets.

Ordinary runs write only temporary protocol generation. --mark-read saves a reviewed
checkpoint under ~/.local/state/mesimon/release-watch/. --dry-run uses synthetic
fixtures, no binaries/network, and cannot mark them read. Only the Python standard library is needed.
"""
import argparse
from datetime import date
import difflib
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request

CI = Path(__file__).resolve().parent
FIXTURES = CI / "release-watch/fixtures"
DATA = Path.home() / ".local/state/mesimon/release-watch"
STATE = DATA / "release-watch.json"
REPOS = {
    "claude": "anthropics/claude-code",
    "codex": "openai/codex",
    "agy": "google-antigravity/antigravity-cli",
}
REQUIRED_PROVIDERS = ("claude", "codex")
CHANGELOGS = {
    "claude": "https://raw.githubusercontent.com/anthropics/claude-code/main/CHANGELOG.md",
    "agy": "https://raw.githubusercontent.com/google-antigravity/antigravity-cli/main/CHANGELOG.md",
}
CHANGELOG = CHANGELOGS["claude"]
# Moving references, not evidence that an API shipped in the installed version.
SURFACES = {
    "claude-mods.d.ts": "https://raw.githubusercontent.com/anthropics/claude-code/main/mods/types/claude-code.d.ts",
    "codex-plugins.md": "https://developers.openai.com/plugins/build/plugins.md",
    "codex-hooks.md": "https://learn.chatgpt.com/docs/hooks.md",
    "agy-hooks.md": "https://antigravity.google/docs/hooks.md",
    "agy-mcp.md": "https://antigravity.google/docs/mcp.md",
}
WATCHED = (
    "hook", "plugin", "mod", "$.prompt", "permission", "settings", "--plugin-dir",
    "mcp", "turn/steer", "app-server", "queue", "compact", "transcript", "sessionstart",
    "userpromptsubmit", "pretooluse", "posttooluse", "preinvocation", "postinvocation",
    "statusline", "--add-dir", "--conversation", "trustedworkspaces", "turn.", "classic.",
    "tool.", "session.", "agent.spawn", "paste", "stop", "interrupt", "subagent",
    "clientusermessageid", "prompt", "dialog", "plan", "tmux", "monitor", "sandbox",
)
VERSION = re.compile(r"(?:rust-v|v)?(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)(?:\+[0-9A-Za-z.-]+)?")


def version(text):
    """Parse CLI output and upstream tags without accepting arbitrary tag names."""
    match = VERSION.fullmatch(text.strip())
    if not match:
        match = re.fullmatch(r"(?:codex-cli )?(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)(?: \(Claude Code\))?", text.strip())
    if not match:
        raise ValueError(f"Unrecognized version: {text!r}")
    return match[1]


def version_key(value):
    core, _, pre = version(value).partition("-")
    # SemVer: a release follows its prereleases; numeric prerelease parts sort first.
    parts = tuple((0, int(p)) if p.isdigit() else (1, p) for p in pre.split("."))
    return (*map(int, core.split(".")), not pre, parts if pre else ())


def watched(line):
    # Prefix matching catches compaction/queued/hooks, while "mod" must not match model.
    return [term for term in WATCHED if re.search(
        r"(?<![\w])" + re.escape(term) + (r"(?:s|ules?)?\b" if term == "mod" else ""),
        line, re.IGNORECASE)]


def partition(lines):
    relevant, other = [], []
    for line in lines:
        if line.strip():
            (relevant if watched(line) else other).append(line)
    return relevant, other


def fenced(text, language="text"):
    fence = "`" * max(3, 1 + max((len(s) for s in re.findall(r"`+", text)), default=0))
    return f"{fence}{language}\n{text}\n{fence}"


def grouped(text, literal=False):
    relevant, other = partition(text.splitlines())
    if literal:
        relevant = [fenced("\n".join(relevant))] if relevant else []
        other = [fenced("\n".join(other))] if other else []
    return "\n".join(["#### Watched", "", *(relevant or ["None."]), "",
                      "#### Other", "", *(other or ["None."]), ""])


def fetch(url):
    request = urllib.request.Request(url, headers={"User-Agent": "mesimon-release-watch",
                                                  "Accept": "application/vnd.github+json" if "api.github.com/" in url else "text/plain"})
    with urllib.request.urlopen(request, timeout=30) as response:
        return response.read().decode("utf-8")


def release_tags(provider, value):
    if provider == "claude":
        return ("v" + value,)
    if provider == "codex":
        return ("rust-v" + value,)
    if provider == "agy":
        return (value, "v" + value)
    raise ValueError(f"Unknown provider: {provider!r}")


def release_tag(provider, value):
    return release_tags(provider, value)[0]


def releases(provider, last, installed, get=fetch, known=(), allow_unpublished_install=False):
    """Read newest-first pages through the baseline, not just the latest release.

    Read the whole boundary page before stopping; sort the result by SemVer.
    Claude's and agy's changelogs can establish an anchor absent from GitHub releases.
    agy updates itself in the background at every launch from its own CDN, so its
    installed version can lead GitHub; read-only runs surface that gap while
    --mark-read refuses to advance past an unpublished install.
    Missing anchors in both sources or exhausted pagination remain errors.
    """
    if last == installed:
        return []
    selected = {}
    found = set()
    reached_baseline = False
    anchors = {last} if allow_unpublished_install else {last, installed}
    for page in range(1, 201):
        url = f"https://api.github.com/repos/{REPOS[provider]}/releases?per_page=100&page={page}"
        rows = json.loads(get(url))
        if not isinstance(rows, list):
            raise ValueError(f"Expected a release list from {url}")
        if not rows:
            break
        for row in rows:
            if row.get("draft"):
                continue
            try:
                value = version(row["tag_name"])
            except ValueError:
                continue
            if row["tag_name"] not in release_tags(provider, value):
                continue
            found.add(value)
            reached_baseline |= version_key(value) <= version_key(last)
            if version_key(last) < version_key(value) <= version_key(installed):
                # Stable installs read stable releases; preview installs also read previews.
                if "-" not in installed and row.get("prerelease"):
                    continue
                if "-" not in installed and "-" in value:
                    continue
                selected[value] = row
        if reached_baseline and {last, installed} <= found | set(known):
            break
        if reached_baseline and allow_unpublished_install and anchors <= found | set(known):
            break
    else:
        raise ValueError(f"{provider}: release pagination exceeded 200 pages; refusing a partial digest")
    if not anchors <= found | set(known):
        raise ValueError(f"{provider}: missing baseline/installed release: {sorted({last, installed} - found - set(known))}")
    return [(v, selected[v]) for v in sorted(selected, key=version_key)]


def changelog_sections(text):
    sections = {}
    current = None
    for line in text.splitlines():
        heading = re.fullmatch(r"## (?:\[)?(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)(?:\])?(?: .*)?", line)
        if heading:
            current = heading[1]
            sections[current] = []
        elif line.startswith("## "):
            current = None
        elif current:
            sections[current].append(line)
    return {v: "\n".join(lines).strip() for v, lines in sections.items()}


def run(argv):
    return subprocess.run(argv, check=True, capture_output=True, text=True, timeout=120).stdout


def installed_versions():
    return {provider: version(run([provider, "--version"])) for provider in REPOS}


def protocol(directory):
    files = {str(p.relative_to(directory)): p.read_text() for p in sorted(directory.rglob("*.ts"))}
    if not files:
        raise ValueError("Codex generated no TypeScript files")
    return files


def changes(before, after, prefix):
    lines = []
    for name in sorted(before.keys() | after.keys()):
        lines.extend(difflib.unified_diff(before.get(name, "").splitlines(), after.get(name, "").splitlines(),
                                         fromfile=f"last-read/{prefix}/{name}",
                                         tofile=f"installed/{prefix}/{name}", lineterm=""))
    return "\n".join(lines)


def load_checkpoint(state_path, data):
    state = json.loads(state_path.read_text())
    present = [p for p in REPOS if p in state]
    for provider in REQUIRED_PROVIDERS:
        if provider not in state:
            raise KeyError(provider)
    for provider in present:
        version(state[provider]["version"])
        date.fromisoformat(state[provider]["read_at"])
    name = state["snapshot"]
    if not re.fullmatch(r"[0-9a-f]{64}\.json", name):
        raise ValueError("Invalid snapshot name")
    raw = (data / name).read_bytes()
    if hashlib.sha256(raw).hexdigest() + ".json" != name:
        raise ValueError("Checkpoint hash mismatch")
    snapshot = json.loads(raw)
    if snapshot["versions"] != {p: state[p]["version"] for p in present}:
        raise ValueError("Checkpoint versions do not match last-read versions")
    return state, snapshot


def mark_read(state_path, data, versions, files, surfaces, today=None):
    """Immutable capture first; atomic pointer last. Failure never advances the cursor."""
    snapshot = {"versions": versions, "protocol": files, "surfaces": surfaces}
    raw = (json.dumps(snapshot, indent=2, sort_keys=True) + "\n").encode()
    name = hashlib.sha256(raw).hexdigest() + ".json"
    data.mkdir(parents=True, exist_ok=True)
    atomic_write(data / name, raw)
    state = {p: {"version": v, "read_at": (today or date.today()).isoformat()} for p, v in versions.items()}
    state["snapshot"] = name
    atomic_write(state_path, (json.dumps(state, indent=2) + "\n").encode())


def atomic_write(path, raw):
    with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as stream:
        temp = Path(stream.name)
        try:
            stream.write(raw)
            stream.close()
            temp.replace(path)
        finally:
            temp.unlink(missing_ok=True)


def collect(state, get, versions, files):
    for provider, installed in versions.items():
        if provider in state and version_key(installed) < version_key(state[provider]["version"]):
            raise ValueError(f"{provider}: installed {installed} is older than last-read; refusing to rewind")
    changelogs = {}
    for provider, url in CHANGELOGS.items():
        if provider in versions and state.get(provider, {}).get("version") != versions[provider]:
            changelogs[provider] = changelog_sections(get(url))
        else:
            changelogs[provider] = {}
    notes = {}
    unpublished = {}
    for p in REPOS:
        if p not in versions or p not in state:
            notes[p] = None
            continue
        last, installed = state[p]["version"], versions[p]
        sections = changelogs.get(p, {})
        found_releases = releases(
            p,
            last,
            installed,
            get,
            known=sections,
            allow_unpublished_install=(p == "agy"),
        )
        rows = dict(found_releases)
        if p in CHANGELOGS:
            # Either source may publish a version without the other. Print their union.
            wanted = {
                v
                for v in sections
                if version_key(last) < version_key(v) <= version_key(installed)
                and ("-" in installed or "-" not in v)
            }
            notes[p] = [(v, rows.get(v)) for v in sorted(wanted | rows.keys(), key=version_key)]
        else:
            notes[p] = found_releases
        if p == "agy" and last != installed and installed not in (rows.keys() | sections.keys()):
            unpublished[p] = installed
    surfaces = {name: get(url) for name, url in SURFACES.items()}
    return {
        "versions": versions,
        "protocol": files,
        "surfaces": surfaces,
        "notes": notes,
        "changelogs": changelogs,
        "changelog": changelogs["claude"],
        "unpublished": unpublished,
    }


def digest(state, before, current, dry=False):
    out = ["# Release watch" + (" — synthetic offline fixtures" if dry else ""), "",
           "Keyword matches are review leads, not compatibility verdicts. Nothing is filed.", ""]
    changelogs = current.get("changelogs") or {"claude": current.get("changelog", {})}
    unpublished = current.get("unpublished", {})
    for provider, installed in current["versions"].items():
        if provider not in state:
            out += [
                f"## {provider}: first capture → {installed}",
                "",
                "No last-read checkpoint for this provider yet (agy updates itself in the background at every launch); `--mark-read` records this baseline.",
                "",
            ]
            body = changelogs.get(provider, {}).get(installed)
            if body:
                out += [f"### {installed} ([CHANGELOG.md]({CHANGELOGS[provider]}))", "", grouped(body)]
            continue
        out += [f"## {provider}: {state[provider]['version']} → {installed}", "",
                f"Last read: {state[provider]['read_at']}", ""]
        if not current["notes"][provider] and provider not in unpublished:
            out += ["No intervening releases.", ""]
        changelog_url = CHANGELOGS.get(provider)
        provider_changelog = changelogs.get(provider, {})
        for value, row in current["notes"][provider]:
            if row is None:
                out += [f"### {value} (changelog only; no GitHub release)", "",
                        f"[CHANGELOG.md]({changelog_url})", "", grouped(provider_changelog[value])]
                continue
            out += [f"### {value} ({row['published_at']})", "",
                    f"[Release notes]({row['html_url']})", "", grouped(row.get("body") or "No release body published.")]
            if changelog_url:
                if value not in provider_changelog:
                    out += ["Release only; no CHANGELOG.md entry for this version.", ""]
                    continue
                body = provider_changelog[value]
                # Common identical bodies need not be printed twice.
                release_lines = [s for s in (row.get("body") or "").splitlines() if s.strip() and not s.startswith("#")]
                if release_lines == [s for s in body.splitlines() if s.strip()]:
                    out += [f"[CHANGELOG.md]({changelog_url}) contains the same entries.", ""]
                else:
                    out += [f"[CHANGELOG.md]({changelog_url}) (independent source):", "", grouped(body)]
        if provider in unpublished:
            out += [
                f"### {unpublished[provider]} (installed locally; not yet published on GitHub)",
                "",
                "agy updates itself in the background at every launch; `--mark-read` waits until upstream publishes this version.",
                "",
            ]
    diff = changes(before["protocol"], current["protocol"], "codex")
    out += ["## Codex app-server protocol", "", "`codex app-server generate-ts --out <temporary directory>` (default surface, as in T-934).", "",
            f"Compared {len(before['protocol'])} → {len(current['protocol'])} TypeScript files.", "",
            fenced(diff, "diff") if diff else "No protocol changes.", ""]
    out += ["## Published plugin / hooks API surfaces", "",
            "Moving upstream references; changes here are not necessarily shipped in the installed versions.", ""]
    for name, body in current["surfaces"].items():
        old = before["surfaces"].get(name)
        out += [f"### [{name}]({SURFACES[name]})", ""]
        if old is None:
            out += ["First capture; no historical comparison. Current reference follows.", "", grouped(body, literal=True)]
        else:
            diff = changes({name: old}, {name: body}, "published")
            out += [fenced(diff, "diff") if diff else "No published surface changes.", ""]
    return "\n".join(out)


def claude_declarations(binary):
    """Read the native binary's embedded declarations; never execute a session."""
    try:
        from compression import zstd
    except ImportError as error:
        raise ValueError("--claude-decls requires Python 3.14's compression.zstd") from error
    image = binary.read_bytes()
    for match in re.finditer(re.escape(b"\x28\xb5\x2f\xfd"), image):
        decoder = zstd.ZstdDecompressor()
        try:
            raw = decoder.decompress(image[match.start():], max_length=2_000_000)
        except zstd.ZstdError:
            continue
        if b"// Claude Code function hooks:" not in raw[:1024]:
            continue
        if not decoder.eof:
            raise ValueError("Claude declarations are truncated or exceed the 2 MB extraction bound")
        return raw.decode("utf-8")
    raise ValueError("No embedded Claude declaration frame found; baseline unchanged")


def declarations_step(args):
    checkpoint = DATA / "claude-decls.json"
    if args.dry_run:
        fixture = json.loads((FIXTURES / "input.json").read_text())["claude_decls"]
        before, installed, text = fixture["before"], fixture["version"], fixture["text"]
    else:
        before = json.loads(checkpoint.read_text()) if checkpoint.exists() else None
        binary = shutil.which("claude")
        if binary is None:
            raise ValueError("claude is not installed")
        installed = version(run(["claude", "--version"]))
        text = claude_declarations(Path(binary).resolve())
        if version(run(["claude", "--version"])) != installed:
            raise ValueError("Installed Claude changed during declaration capture; rerun")
    checksum = hashlib.sha256(text.encode()).hexdigest()
    out = ["# Installed Claude declarations" + (" — synthetic offline fixtures" if args.dry_run else ""), "",
           f"Version: {installed}; SHA-256: `{checksum}`; {len(text.encode())} bytes.", ""]
    if before is None:
        out += ["First capture; no last-read installed declarations to compare."]
    else:
        date.fromisoformat(before["read_at"])
        if hashlib.sha256(before["text"].encode()).hexdigest() != before["sha256"]:
            raise ValueError("Claude declaration checkpoint hash mismatch")
        if version_key(installed) < version_key(before["version"]):
            raise ValueError("Installed Claude is older than last-read declarations; refusing to rewind")
        diff = changes({"claude.d.ts": before["text"]}, {"claude.d.ts": text}, "claude-decls")
        out += [f"Last read: {before['version']} ({before['read_at']}); SHA-256: `{before['sha256']}`.", "",
                fenced(diff, "diff") if diff else "No installed declaration changes."]
    print("\n".join(out), flush=True)
    if args.mark_read:
        DATA.mkdir(parents=True, exist_ok=True)
        record = dict(version=installed, read_at=date.today().isoformat(), sha256=checksum, text=text)
        atomic_write(checkpoint, (json.dumps(record, indent=2) + "\n").encode())
        print("Marked installed Claude declarations read.", file=sys.stderr)
    return 0


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--dry-run", action="store_true", help="offline synthetic fixtures; never updates last-read")
    mode.add_argument("--mark-read", action="store_true", help="print and checkpoint installed versions and API captures after reading")
    parser.add_argument("--claude-decls", action="store_true", help="separate offline installed-Claude declaration comparison (Python 3.14)")
    parser.add_argument("--capture", action="store_true", help="capture the installed CLIs' screens and check the ticket page reads them (ci/pane-capture.py; two tiny Haiku turns)")
    args = parser.parse_args(argv)
    try:
        if args.claude_decls:
            return declarations_step(args)
        if args.capture:
            return subprocess.run([sys.executable, "-B", str(CI / "pane-capture.py")]).returncode
        if args.dry_run:
            fixture = json.loads((FIXTURES / "input.json").read_text())
            state, before = fixture["state"], fixture["before"]
            current = collect(state, fixture["responses"].__getitem__, fixture["versions"], fixture["protocol"])
        else:
            state, before = load_checkpoint(STATE, DATA)
            versions = installed_versions()
            with tempfile.TemporaryDirectory(prefix="mesimon-release-watch-") as directory:
                run(["codex", "app-server", "generate-ts", "--out", directory])
                current = collect(state, fetch, versions, protocol(Path(directory)))
            if installed_versions() != versions:
                raise ValueError("Installed versions changed during capture; rerun before marking read")
        print(digest(state, before, current, args.dry_run), flush=True)
        if args.mark_read:
            if current.get("unpublished"):
                ahead = ", ".join(f"{p} {v}" for p, v in sorted(current["unpublished"].items()))
                raise ValueError(f"unpublished installed release ({ahead}); refusing to mark read")
            mark_read(STATE, DATA, current["versions"], current["protocol"], current["surfaces"])
            print("Marked installed versions read.", file=sys.stderr)
        return 0
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError, urllib.error.URLError) as error:
        print(f"release-watch: incomplete; last-read unchanged: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
