# Third-party notices

This project is licensed under AGPL-3.0-only (see `LICENSE`). It includes or derives from the following third-party material.

## peeroxide (vendored, modified)

- Location: `fork/peeroxide/`
- Upstream: https://github.com/Rightbracket/peeroxide, tag `peeroxide-v1.7.3`
- License: `MIT OR Apache-2.0` (texts in `fork/peeroxide/LICENSE-MIT` and `fork/peeroxide/LICENSE-APACHE`)
- The fork carries our changes; they are listed in `fork/peeroxide/FORK.md`. Upstream's own notices for the
  projects it studied are kept in `fork/peeroxide/THIRD_PARTY_NOTICES`.

## livefiles 1.1.0

- Upstream: https://github.com/holesail/livefiles
- License: GPL-3.0 (compatible with AGPL-3.0 under section 13 of both licenses)
- Used for: `crates/townjump-filemanager/src/page/logo.txt` and the resources of the file manager web page, which the
  `--filemanager` mode serves in the same form as the JavaScript implementation.

## Reference implementation (not included)

The JavaScript Holesail 2.4.4 and its dependencies are the behavioural reference. They are not part of this
repository; `interop/setup-js.sh` downloads them from npm into the ignored `.tmp/` directory for the interop tests only.
