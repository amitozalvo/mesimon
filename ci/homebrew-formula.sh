#!/usr/bin/env bash
# Print the Homebrew formula for one release: ci/homebrew/mesimon.rb with the
# version and each tarball's sha256 filled in.
#
# Usage:  ci/homebrew-formula.sh <tag> <dir>
#
# <dir> holds the release's `mesimon-<tag>-<target>.tar.gz.sha256` files.
# ci/release.sh passes dist/. For a release that is already published:
#
#   gh release download <tag> --repo amitozalvo/mesimon-releases \
#     --pattern '*.sha256' --dir <dir>
#
# Every target the template names must have its checksum in <dir>. A formula
# with a placeholder left in it is never printed.

set -euo pipefail

die() { echo "error: $1" >&2; exit 1; }

[ $# -eq 2 ] || die "usage: ci/homebrew-formula.sh <tag> <dir>"
tag="$1"
dir="$2"
case "$tag" in
  v*) ;;
  *) die "the tag is v<version>, got '$tag'" ;;
esac

formula=$(cat "$(dirname "$0")/homebrew/mesimon.rb")
formula=${formula//@VERSION@/${tag#v}}

for target in $(grep -o '@SHA256:[a-z0-9_-]*@' <<<"$formula" | sort -u | sed 's/@SHA256:\(.*\)@/\1/'); do
  sums="$dir/mesimon-$tag-$target.tar.gz.sha256"
  [ -r "$sums" ] || die "no checksum for $target: $sums"
  # `shasum -a 256` writes `<hex>  <name>`; the hash is the first field.
  hash=$(awk '{print $1; exit}' "$sums")
  [[ "$hash" =~ ^[0-9a-f]{64}$ ]] || die "$sums does not start with a sha256"
  formula=${formula//@SHA256:$target@/$hash}
done

if grep -q '@VERSION@\|@SHA256:' <<<"$formula"; then
  die "a placeholder is left in the formula: $(grep -o '@VERSION@\|@SHA256:[^@]*@' <<<"$formula" | head -1)"
fi

printf '%s\n' "$formula"
