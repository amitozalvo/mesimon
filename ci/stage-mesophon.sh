#!/bin/sh
# Assemble the complete browser runtime without Node tests or dependencies.
# Usage: ci/stage-mesophon.sh NEW_DESTINATION [SOURCE_DIRECTORY]
set -eu
mesophon_destination=${1:?pass a new destination directory}
mesophon_source=${2:-"$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)/web/mesophon"}
if [ ! -f "$mesophon_source/pkg/mesimon_web_bg.wasm" ]; then
    echo 'Missing Mesophon Wasm assets; run ci/build-mesophon.sh first.' >&2
    exit 1
fi
# Requiring a fresh directory prevents stale modules surviving a new package.
mkdir "$mesophon_destination"
cp "$mesophon_source/index.html" "$mesophon_source/style.css" "$mesophon_destination/"
for mesophon_module in "$mesophon_source"/*.js; do
    case "$mesophon_module" in *.test.js) continue ;; esac
    cp "$mesophon_module" "$mesophon_destination/"
done
cp -R "$mesophon_source/pkg" "$mesophon_destination/pkg"
