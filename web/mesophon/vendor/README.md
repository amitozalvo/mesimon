# Vendored browser libraries

Mesophon has no build step: the relay serves these files as they are, and its
CSP (`script-src 'self'`) rules out a CDN and an inline import map. So the
libraries live here, unchanged except for one import specifier.

| File | Package | Licence |
| --- | --- | --- |
| `preact.module.js` | `preact@10.29.8`, `dist/preact.module.js` | MIT, `LICENSE-preact` |
| `hooks.module.js` | `preact@10.29.8`, `hooks/dist/hooks.module.js` | MIT, `LICENSE-preact` |
| `htm.module.js` | `htm@3.1.1`, `dist/htm.module.js` | Apache-2.0, `LICENSE-htm` |

The one change: `hooks.module.js` imports `"preact"`, a bare specifier a browser
cannot resolve without an import map, so it is rewritten to
`"./preact.module.js"`. Each file's trailing `sourceMappingURL` comment is
removed so a browser never asks the relay for a map that is not there.

To update, from a scratch directory:

```sh
npm pack preact@10 htm@3 && for f in *.tgz; do mkdir -p "${f%.tgz}" && tar -xzf "$f" -C "${f%.tgz}"; done
cp preact-*/package/dist/preact.module.js "$MESOPHON/vendor/"
sed 's/from"preact"/from".\/preact.module.js"/' preact-*/package/hooks/dist/hooks.module.js > "$MESOPHON/vendor/hooks.module.js"
cp htm-*/package/dist/htm.module.js "$MESOPHON/vendor/"
sed -i '' -e 's#//\# sourceMappingURL=.*$##' "$MESOPHON"/vendor/*.module.js
```

Then run `npm test` in `web/mesophon`. Only `html.js` imports from this
directory.

The fonts in `../fonts` are IBM Plex Sans, Sans Hebrew and Mono under the SIL
Open Font License (`../fonts/OFL.txt`): the Latin, Latin Extended and Hebrew
subsets Google Fonts serves, self-hosted for the same CSP reason.
