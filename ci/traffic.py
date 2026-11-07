#!/usr/bin/env python3
"""Keep GitHub's 14-day traffic for mesimon's repositories past the 14 days.

GitHub holds a repository's traffic (views, unique visitors, clones, referrers, popular
paths) for 14 days only, and mesimon.dev keeps no analytics and no logs. Run this by hand,
weekly or more often; nothing is lost while runs are less than 12 days apart. It reads
through `gh api` (the author's own login, which needs push access for traffic) and appends
JSON lines to a file outside the repository:

  kind "day"       one per repository per settled UTC day: views, unique visitors,
                   clones and unique cloners that day. GitHub's counts arrive hours late,
                   so today and yesterday are shown but recorded only once two days old.
  kind "snapshot"  one per repository per run day: stars, forks, watchers, the 14-day
                   referrers and popular paths, and every release asset's download count.

A day or a snapshot already in the file is never written again, so a second run on the
same day adds nothing. The file is the author's record; it holds GitHub's aggregate counts
only, no visitor identity (GitHub gives none). Nothing is sent anywhere.

Search queries are not here: Google Search Console is their measure and has no `gh` road.

  python3 -B ci/traffic.py                 # record and print the last two weeks
  python3 -B ci/traffic.py --file PATH     # another record file

Only the Python standard library and `gh` are needed.
"""
import argparse
from datetime import datetime, timedelta, timezone
import json
from pathlib import Path
import subprocess
import sys

REPOS = ("amitozalvo/mesimon", "amitozalvo/mesimon-releases")
RECORD = Path.home() / ".local/state/mesimon/traffic/traffic.jsonl"
WINDOW = 14


def gh(path):
    try:
        run = subprocess.run(["gh", "api", path], capture_output=True, text=True, timeout=60)
    except FileNotFoundError:
        sys.exit("traffic: gh is not installed")
    if run.returncode != 0:
        sys.exit(f"traffic: gh api {path} failed: {run.stderr.strip()}")
    return json.loads(run.stdout)


def fetch(repo):
    """Everything GitHub says about one repository now, as plain data."""
    releases, page = [], 1
    while True:
        batch = gh(f"repos/{repo}/releases?per_page=100&page={page}")
        releases += batch
        if len(batch) < 100:
            break
        page += 1
    return {
        "repo": gh(f"repos/{repo}"),
        "views": gh(f"repos/{repo}/traffic/views"),
        "clones": gh(f"repos/{repo}/traffic/clones"),
        "referrers": gh(f"repos/{repo}/traffic/popular/referrers"),
        "paths": gh(f"repos/{repo}/traffic/popular/paths"),
        "releases": releases,
    }


def per_day(raw):
    """{date: {views, view_uniques, clones, clone_uniques}} over GitHub's window."""
    days = {}
    for key, field in (("views", "views"), ("clones", "clones")):
        for row in raw[key].get(field, []):
            day = days.setdefault(row["timestamp"][:10], {
                "views": 0, "view_uniques": 0, "clones": 0, "clone_uniques": 0})
            day[field] = row["count"]
            day[field.rstrip("s") + "_uniques"] = row["uniques"]
    return days


def snapshot(repo, raw, today, taken):
    meta = raw["repo"]
    return {
        "kind": "snapshot", "repo": repo, "date": today, "taken": taken,
        "stars": meta.get("stargazers_count", 0),
        "forks": meta.get("forks_count", 0),
        "watchers": meta.get("subscribers_count", 0),
        "referrers": [{k: r[k] for k in ("referrer", "count", "uniques")} for r in raw["referrers"]],
        "paths": [{k: p[k] for k in ("path", "count", "uniques")} for p in raw["paths"]],
        "downloads": {rel["tag_name"]: {a["name"]: a["download_count"] for a in rel.get("assets", [])}
                      for rel in raw["releases"]},
    }


def settled(today):
    """The first day still counting: yesterday, as GitHub's counts arrive hours late."""
    return (datetime.fromisoformat(today) - timedelta(days=1)).date().isoformat()


def new_rows(existing, repo, raw, today, live):
    """The rows this run adds for one repository: settled days and today's snapshot not yet kept."""
    have = {(r["kind"], r["repo"], r["date"]) for r in existing}
    rows = [{"kind": "day", "repo": repo, "date": date, **counts}
            for date, counts in sorted(per_day(raw).items())
            if date < settled(today) and ("day", repo, date) not in have]
    if ("snapshot", repo, today) not in have:
        rows.append(live)
    return rows


def read(path):
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]


def total(downloads):
    return sum(sum(assets.values()) for assets in downloads.values())


def delta(old, new):
    change = new - old
    return f"{new}" if change == 0 else f"{old} → {new} ({change:+d})"


def report(repo, rows, raw, today, live):
    """The last two weeks of recorded days, the days still counting, and the change since the previous run."""
    days = sorted((r for r in rows if r["kind"] == "day" and r["repo"] == repo), key=lambda r: r["date"])
    first = (datetime.fromisoformat(today) - timedelta(days=WINDOW)).date().isoformat()
    lines = [f"{repo}", f"  {'day (UTC)':<12}{'views':>7}{'uniq':>6}{'clones':>8}{'uniq':>6}"]
    for d in (d for d in days if d["date"] >= first):
        lines.append(f"  {d['date']:<12}{d['views']:>7}{d['view_uniques']:>6}{d['clones']:>8}{d['clone_uniques']:>6}")
    counting = [(date, d) for date, d in sorted(per_day(raw).items()) if date >= settled(today)]
    for date, d in counting:
        lines.append(f"  {date + '*':<12}{d['views']:>7}{d['view_uniques']:>6}{d['clones']:>8}{d['clone_uniques']:>6}")
    lines.append(f"  14 days: {raw['views'].get('count', 0)} views, {raw['views'].get('uniques', 0)} unique visitors,"
                 f" {raw['clones'].get('count', 0)} clones" + ("  (* still counting)" if counting else ""))
    refs = ", ".join(f"{r['referrer']} {r['count']}/{r['uniques']}" for r in live["referrers"])
    lines.append(f"  referrers (views/uniques): {refs or 'none'}")
    earlier = [r for r in rows if r["kind"] == "snapshot" and r["repo"] == repo]
    if earlier:
        prev = max(earlier, key=lambda r: r["taken"])
        lines.append(f"  since {prev['taken']}: stars {delta(prev['stars'], live['stars'])},"
                     f" forks {delta(prev['forks'], live['forks'])},"
                     f" downloads {delta(total(prev['downloads']), total(live['downloads']))}")
        moved = [(tag, name, prev["downloads"].get(tag, {}).get(name, 0), count)
                 for tag, assets in live["downloads"].items() for name, count in assets.items()
                 if count != prev["downloads"].get(tag, {}).get(name, 0) and not name.endswith(".sha256")]
        for tag, name, old, new in moved[:8]:
            lines.append(f"    {name}: {delta(old, new)}")
        if len(moved) > 8:
            lines.append(f"    and {len(moved) - 8} more assets")
    else:
        lines.append(f"  stars {live['stars']}, forks {live['forks']}, downloads {total(live['downloads'])} (first run)")
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--file", type=Path, default=RECORD, help=f"record file (default {RECORD})")
    args = parser.parse_args()
    now = datetime.now(timezone.utc)
    today, taken = now.date().isoformat(), now.isoformat(timespec="seconds")
    existing = read(args.file)
    added, sections = [], []
    for repo in REPOS:
        raw = fetch(repo)
        live = snapshot(repo, raw, today, taken)
        rows = new_rows(existing, repo, raw, today, live)
        added += rows
        sections.append(report(repo, existing + [r for r in rows if r["kind"] == "day"], raw, today, live))
    if added:
        args.file.parent.mkdir(parents=True, exist_ok=True)
        with args.file.open("a") as out:
            out.writelines(json.dumps(row, sort_keys=True) + "\n" for row in added)
    print("\n\n".join(sections))
    print(f"\n{len(added)} rows added to {args.file}" if added else f"\nno new rows; {args.file} unchanged")


if __name__ == "__main__":
    main()
