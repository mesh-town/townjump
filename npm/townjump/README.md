# townjump

Share a local TCP or UDP service through the HyperDHT network, with a Holesail-compatible command line and `hs://` keys.
This package installs the prebuilt `townjump` binary for your platform; it has no install scripts and no downloads at
install time.

```
npm install -g townjump
townjump --live 3000
```

or without installing: `npx townjump --live 3000`.

Supported platforms: macOS (arm64, x64), Linux (x64, arm64, armv7), Windows (x64), Android/Termux (arm64).
Needs Node.js 18 or newer. Other systems and install channels:
<https://github.com/mesh-town/townjump#install>

License: AGPL-3.0-only. Source and documentation: <https://github.com/mesh-town/townjump>
