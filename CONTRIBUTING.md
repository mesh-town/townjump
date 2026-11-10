# Contributing to townjump

Bug reports, ideas and pull requests are welcome.

## Licensing of contributions

townjump is distributed under AGPL-3.0-only (see [LICENSE](LICENSE)). Contributions are accepted under a
different, more permissive license:

**By submitting a contribution (a pull request, patch, or any other code, documentation or artwork) to this
repository, you license it to the project and to everyone else under the
[Apache License, Version 2.0](https://www.apache.org/licenses/LICENSE-2.0).**

This is known as "inbound = Apache-2.0, outbound = AGPL-3.0". It keeps the project free for everyone under
the AGPL, and it also lets the project offer the code under other terms where the AGPL does not fit — for
example, builds for app stores or a commercial license for device makers. You keep the copyright to your
work, and the Apache-2.0 grant includes an explicit patent license.

By submitting, you also certify that:

1. the contribution is your own original work, or you otherwise have the right to submit it under
   Apache-2.0; and
2. if your employer or anyone else has rights to work you create, they have permitted you to submit it
   on these terms.

If you cannot agree to this, please open an issue describing the change instead of sending code.

Code copied from other projects must carry a license compatible with both Apache-2.0 and AGPL-3.0
(MIT, BSD, Apache-2.0, ISC, Zlib and similar). Name its source in the pull request so it can be listed in
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). Code under GPL, LGPL or AGPL cannot be accepted.

The `townjump-keys` crate (except the z-base-32 module, which is based on the MIT-licensed `z32` package) and the
`townjump-logger` crate are independent implementations written from the behavioral specification in
[docs/spec/](docs/spec/), without access to any other implementation. Contributions to them must keep that
property: write from the specification, not from other code.

Two parts of the tree keep their own licenses and follow them rather than the rule above:

- `fork/peeroxide/` is MIT OR Apache-2.0; changes there are contributed under the same terms.
- The logo and the file-manager page resources in `crates/townjump-filemanager/` come from livefiles
  (GPL-3.0); changes to those resources stay under GPL-3.0.

## Before you open a pull request

- Building: [docs/building.md](docs/building.md); testing: [docs/testing.md](docs/testing.md).
- Behaviour must stay compatible with the JavaScript Holesail: see
  [docs/compatibility.md](docs/compatibility.md) and the specification in [docs/spec/](docs/spec/).
- Keep a pull request to one change, with tests.
- Security issues: do not open a public issue, follow [SECURITY.md](SECURITY.md).
