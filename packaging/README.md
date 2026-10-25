# Packaging

Drafts of the Homebrew formula and the OpenWrt package. Both build from the
source tarball of a release tag (`v<version>`, equal to `version` in the root
`Cargo.toml`), using the committed `Cargo.lock` and the vendored `fork/`.
Both contain the repository URL (`mesh-town/townjump`) and a placeholder hash:
replace the hash after the release is published.

Unofficial Rust port of Holesail, not affiliated with Holesail / LIVEPORT.
The installed command is called `townjump` (the npm package `holesail`
installs `holesail`, so the names do not collide). The help text still says
`holesail`, as in the original (the formula mentions it in `caveats`).

## Release

1. Set the same `version` in `Cargo.toml` and `CHANGELOG.md`, commit.
2. Tag and push: `git tag v0.1.0 && git push origin v0.1.0`. The `release`
   workflow checks that the tag equals the crate version and uploads
   archives plus `SHA256SUMS` to the GitHub release.
3. Compute the hash of the **source** tarball (not of the binaries):
   `curl -sL https://github.com/mesh-town/townjump/archive/refs/tags/v0.1.0.tar.gz | shasum -a 256`

## Homebrew (own tap)

1. Once: `brew tap-new mesh-town/homebrew-tap`, push it as `mesh-town/homebrew-tap`
   on GitHub (formulas live in `Formula/`).
2. Copy `homebrew/townjump.rb` to `Formula/townjump.rb` in the tap,
   set `url` (new tag) and `sha256` (step 3 above).
3. Check locally:
   ```
   brew install --build-from-source --verbose mesh-town/tap/townjump
   brew test mesh-town/tap/townjump
   brew audit --strict --formula mesh-town/tap/townjump
   ```
   `--online`/`--new` audits fail on notability until the repository is
   known: expected for a tap.
4. Users: `brew install mesh-town/tap/townjump`.

`std_cargo_args` already passes `--locked`, `--root` and `--path`; do not
add `--locked` again. homebrew-core needs a repository older than 30 days
with enough stars/forks; not before that.

## OpenWrt (own feed or SDK)

`openwrt/townjump/Makefile` follows `utils/ripgrep` from `openwrt/packages`
(`MAKE_PATH:=crates/townjump`, `rust-package.mk`). 32-bit MIPS targets are
excluded: they have no `AtomicU64`.

1. Set `PKG_VERSION` and replace `PKG_HASH:=skip` with the sha256 of the
   source tarball (`make package/townjump/download` prints the expected
   value).
2. Own feed: put `townjump/Makefile` into a repository `openwrt-feed`;
   in the firmware tree add `src-git townjump https://github.com/mesh-town/openwrt-feed.git`
   to `feeds.conf`, then `./scripts/feeds update townjump &&
   ./scripts/feeds install townjump`, then
   `make package/townjump/{download,check,compile} V=s`.
3. The Rust compiler in the feed must satisfy `rust-version` of the
   workspace (OpenWrt master and 25.12 ship rustc 1.96, 24.10 ships 1.94).
   If it does not, `cargo` refuses to build the package.
4. No SDK at hand: the release has static musl binaries (aarch64, armv7,
   x86_64); copy one to `/usr/bin` of the router.

The official `openwrt/packages` feed requires a real name and a non-private
e-mail in `Author` and `Signed-off-by`, so the package stays in an own feed.
