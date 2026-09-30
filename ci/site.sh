#!/usr/bin/env bash
# Build mesimon.dev into dist/site, and with --publish put it on the web.
#
# GitHub Pages serves the site from the root of amitozalvo/mesimon-releases'
# main branch: the same repo and branch that already carry the released
# install.sh. So https://mesimon.dev/install.sh is the released installer byte
# for byte, and this script never publishes install.sh — ci/release.sh does,
# after each release, so the installer never runs ahead of the binaries it
# names. The page's source is site/; the fonts, the demo GIF and the icon are
# copied in from where the repo already keeps them.
#
# Usage:  ci/site.sh                     build dist/site and check it
#         ci/site.sh --publish [label]   build, check, publish every changed file
#
# Preview the build with `open dist/site/index.html`. ci/release.sh runs the
# build in a dry run and publishes after the release.

set -euo pipefail
cd "$(dirname "$0")/.."

# The same public repo as DIST_REPO in ci/release.sh.
DIST_REPO="amitozalvo/mesimon-releases"
# Every page that tells a person how to install says exactly this.
ONE_LINER="curl -fsSL https://mesimon.dev/install.sh | sh"

die() { echo "error: $1" >&2; [ $# -gt 1 ] && echo "  fix: $2" >&2; exit 1; }

publish=0
label="site"
if [ "${1:-}" = "--publish" ]; then
  publish=1
  label="${2:-site}"
elif [ $# -gt 0 ]; then
  die "usage: ci/site.sh [--publish [label]]"
fi

# Every page, by its path under site/; each is served at its directory.
pages="index.html relay/index.html relay/thanks/index.html"

out=dist/site
rm -rf "$out"
mkdir -p "$out/fonts"
for p in $pages; do
  mkdir -p "$out/$(dirname "$p")"
  cp "site/$p" "$out/$p"
done
cp site/style.css site/site.js site/CNAME site/.nojekyll "$out/"
cp assets/demo.gif "$out/demo.gif"
cp assets/mascot/resting.png "$out/favicon.png"
for f in plex-sans-latin.woff2 plex-sans-hebrew-500.woff2 plex-mono-400-latin.woff2 OFL.txt; do
  cp "web/mesophon/fonts/$f" "$out/fonts/"
done

# Every relative reference resolves inside the build, from the file that makes
# it; a reference to a directory resolves to its index.html.
local_refs() { grep -vE '^(https?:|#|mailto:)' || true; }
for p in $pages; do
  dir="$out/$(dirname "$p")"
  for r in $(grep -oE '(href|src)="[^"]+"' "site/$p" | sed -E 's/^[a-z]+="//; s/"$//' | local_refs); do
    r="${r%%#*}"
    [ -f "$dir/$r" ] || [ -f "$dir/$r/index.html" ] || die "site/$p refers to $r, which the build does not have"
  done
done
for r in $(grep -oE 'url\([^)]+\)' site/style.css | sed -E 's/^url\(//; s/\)$//' | local_refs); do
  [ -f "$out/$r" ] || die "site/style.css refers to $r, which the build does not have"
done

for f in README.md docs/USING.md ci/releases-readme.md install.sh site/index.html; do
  grep -qF "$ONE_LINER" "$f" || die "$f does not give the install line" "use: $ONE_LINER"
done
[ "$(cat "$out/CNAME")" = "mesimon.dev" ] || die "site/CNAME must name mesimon.dev"

echo "built $out ($(find "$out" -type f | wc -l | tr -d ' ') files, $(du -sh "$out" | cut -f1))"
[ "$publish" = "1" ] || exit 0

command -v gh >/dev/null || die "the GitHub CLI is required to publish" "brew install gh"
gh auth status >/dev/null 2>&1 || die "gh is not logged in" "gh auth login"
(cd "$out" && find . -type f | sed 's|^\./||' | sort) | while read -r rel; do
  ci/publish-file.sh "$DIST_REPO" "$out/$rel" "$rel" "sync $rel ($label)"
done
echo "published the site to $DIST_REPO; Pages serves it at https://mesimon.dev within a minute or two"
