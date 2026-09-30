#!/usr/bin/env bash
# Is the live relay current with this checkout? ci/release.sh asks before it
# publishes anything, and the relay's deploy/ship.sh asks before it tags.
#
# The relay's `Request` match is exhaustive, so a relay built before a new
# wire request answers it InvalidRequest, and the phone's page (Mesophon) is
# inside the relay's image. So a release must never go out ahead of the relay
# its clients talk to. The relay's browser origin answers GET /version with
# the core commit it was built against; the relay is current when that commit
# has every commit up to HEAD that touched what the relay is built from. No
# minimum version is kept by hand: git already knows what changed.
#
# Usage:  ci/check-relay.sh
#         MESIMON_RELAY_ORIGIN=http://localhost:8444 ci/check-relay.sh
#         MESIMON_RELAY_ORIGIN=none ci/check-relay.sh    skip, and say so

set -euo pipefail
cd "$(dirname "$0")/.."

origin="${MESIMON_RELAY_ORIGIN:-https://remote.mesimon.dev}"
# What the relay's image is built from, in this repository: the one core crate
# its binary links (mesimon-team depends on no other), the phone's page and
# its Wasm crate, and the two scripts that build and stage them.
inputs="crates/mesimon-team crates/mesimon-web web/mesophon ci/build-mesophon.sh ci/stage-mesophon.sh"

die() { echo "error: $1" >&2; [ $# -gt 1 ] && echo "  fix: $2" >&2; exit 1; }
ship="ship the relay first: deploy/ship.sh all in mesimon-relay (or deploy/ship.sh <host> for the relay alone)"

if [ "$origin" = "none" ]; then
  echo "SKIPPED: the live relay was not asked (MESIMON_RELAY_ORIGIN=none)"
  exit 0
fi

body=$(curl -fsS --max-time 10 "$origin/version" 2>&1) || \
  die "$origin/version did not answer: ${body:-no reply}" \
      "$ship; a relay from before T-526 has no /version. MESIMON_RELAY_ORIGIN=none skips this"

fields=$(python3 -c '
import json, sys
try:
    core = json.loads(sys.argv[1])["core"]
except (ValueError, KeyError, TypeError, AttributeError):
    sys.exit(1)
print(core.get("sha") or "-", core.get("version") or "?", "dirty" if core.get("dirty") else "clean")
' "$body") || die "$origin/version answered something other than the relay's stamp: $body" "$ship"
read -r sha version state <<<"$fields"

[ "$sha" != "-" ] || \
  die "the relay at $origin does not say which core it was built from" \
      "$ship, which stamps the build"
[ "$state" = "clean" ] || \
  die "the relay at $origin was built from ${sha:0:7} with uncommitted changes, so that commit does not describe it" \
      "$ship, from a committed tree"
git cat-file -e "$sha^{commit}" 2>/dev/null || \
  die "the relay at $origin was built from $sha, which this repository does not have" \
      "git fetch origin; if it still is not here, $ship"

# shellcheck disable=SC2086 # the inputs are a list of paths
missing=$(git log --format='  %h %s' "$sha..HEAD" -- $inputs)
[ -z "$missing" ] || \
  die "the relay at $origin was built from ${sha:0:7} (v$version), without these changes to what it serves:
$missing" "$ship"

echo "the relay at $origin is current: built from ${sha:0:7} (v$version), and nothing it serves has changed since"
