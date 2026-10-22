# deltas-js — capturing wire changes from JS

The scripts that captured the vectors of `docs/spec/deltas.md` (the table "change → vector → how to capture
from JS" at the end): `plugin.js` — `NS_PLUGIN` and the PLUGIN request encodings; `udxcap.js` — empty MESSAGE and
HEARTBEAT over UDX; `udxzwp.js` — zero-window probes (also the model of a raw UDP peer); `udxrto.js` —
retransmissions and the drop with a silent peer. They run on `udx-native` 1.21.3 and 1.19.2 — paths in `deltas.md`.
`plugin.js` takes sources from `vendor/js/`, and their dependencies from JS holesail 2.4.4:
`NODE_PATH=.tmp/npm-holesail/node_modules node testvectors/deltas-js/plugin.js` (without `NODE_PATH` —
`MODULE_NOT_FOUND`: `vendor/js/` is an excerpt of sources, not `node_modules`). The vectors in git are captured by
`testvectors/gen/deltas-plugin.mjs`, this script is a manual probe.
