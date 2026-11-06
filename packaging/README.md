# Packaging

Packaging of townjump: prebuilt binaries on GitHub Releases, a Homebrew
formula (own tap), a Windows installer with a winget manifest, and
package feeds for Debian, OpenWrt and Keenetic. A release tag (`v<version>`, equal to `version` in the root
`Cargo.toml`) starts everything; the Homebrew formula and the Windows
installer install the binaries of that release, they do not build from source.
The Debian, OpenWrt and Keenetic feeds repack the static musl binaries of the
release; they do not build from source.

## Release

Flow: tag, then the jobs of `.github/workflows/release.yml`, then the assets
of the GitHub release, then, in this order of declaration, `tap`, `winget`,
`deb`, `openwrt-feed`, `keenetic-feed`, `dist` and `container-image`.

1. Set the same `version` in `Cargo.toml` and `CHANGELOG.md`, commit.
2. Tag and push: `git tag v0.1.0 && git push origin v0.1.0`.
3. `check-tag` compares the tag with the crate version. `build` makes one
   archive per target (`.tar.gz`; `.zip` for Windows). `windows-installer`
   builds `townjump-setup-<version>.exe` from the Windows zip.
4. `publish` adds the source archive and `SHA256SUMS`, checks the full list
   of assets and creates the release (`scripts/dist/publish.sh`).
5. `tap` and `winget` (reusable workflows `tap-bump.yml`, `winget.yml`) run
   after `publish`; both can also be started by hand with a version.
6. `deb`, `openwrt-feed`, `keenetic-feed`, `dist` (direct downloads) and
   `container-image` run after `publish` as well; they do not wait for each
   other (concurrency groups and the lease on the target branch keep pushes in order).

Targets: macOS (arm64, x86_64), Linux musl (x86_64, aarch64, armv7 hard and
soft float, mipsel, mips), Android (`aarch64-linux-android`, bionic build with
a pinned NDK, for Termux) and Windows (`x86_64-pc-windows-msvc`).

Dry run: start `release` by hand (`workflow_dispatch`, input `publish` is
`false` by default). It reads the version from `Cargo.toml`, builds every
target (including MIPS, armv7 and the MSVC build) and the Windows installer.
It does not create a release and runs none of the publishing jobs (`publish`,
`tap`, `winget`, `deb`, `openwrt-feed`, `keenetic-feed`, `dist`,
`container-image`): they run only for a `v*` tag, and for a manual run only
with `publish` enabled. The Android binary is not executed there (foreign
architecture); only its ELF header is checked.

Settings (repository variable and secrets; an empty secret skips the step
with a `::notice::`, it does not fail the release):

| Name | Kind | Meaning |
|---|---|---|
| `DIST_REPO` | variable | Repository that holds the release assets. Empty: the repository that runs the workflow (the releases are read from the same repository the feeds are built in). |
| `DIST_TOKEN` | secret | Token with write access to `DIST_REPO`. Required only when `DIST_REPO` is another repository (an error without it). |
| `DIST_DEPLOY_KEY` | secret | SSH deploy key with write access to `mesh-town/townjump-dist`: the apt repository is published to `linux/debian/` there. Empty: the apt repository is not published. |
| `OPENWRT_DEPLOY_KEY` | secret | SSH deploy key with write access to `mesh-town/townjump-openwrt`: the whole repository is replaced by the OpenWrt feed. Empty: not published. |
| `KEENETIC_DEPLOY_KEY` | secret | SSH deploy key with write access to `mesh-town/townjump-keenetic`: the whole repository is replaced by the Keenetic feed. Empty: not published. |
| `TAP_TOKEN` | secret | Token with write access to the Homebrew tap. Empty: the tap is not updated. |
| `WINGET_TOKEN` | secret | Token for `komac` (a fork of `winget-pkgs` and pull requests). Empty: nothing is submitted. |

`check-dist.yml` runs the script tests, the manifest schema check and the
Homebrew checks on every change under `packaging/`, `scripts/dist/` and the
release workflows. New package jobs go after the marker comment at the end of
`release.yml`; the list of release assets is one function in `publish.sh`.

## Where the feeds live

Three separate repositories, each with its own GitHub Pages site (branch `main`, root) and its own deploy key.
The addresses are in `openwrt/common/site.env`.

| Feed | Repository | Pages address | How it is replaced |
|---|---|---|---|
| apt | `mesh-town/townjump-dist` | `https://mesh-town.github.io/townjump-dist/linux/debian` | only `linux/debian/` (the rest of the repository stays) |
| OpenWrt | `mesh-town/townjump-openwrt` | `https://mesh-town.github.io/townjump-openwrt` | the whole repository |
| Keenetic | `mesh-town/townjump-keenetic` | `https://mesh-town.github.io/townjump-keenetic` | the whole repository |

`openwrt/common/publish_pages.sh` does the push: one orphan commit per publication, a force push guarded by a lease,
`.nojekyll`, an empty remote repository is fine. A feed repository is generated; nobody edits it by hand.

## Homebrew (own tap)

The formula `homebrew/townjump.rb` installs the prebuilt archive of the
macOS architecture (arm64 or x86_64) and verifies it against
`SHA256SUMS` of the release.

1. Once: create the tap repository `mesh-town/homebrew-macos` on GitHub
   (formulas live in `Formula/`) and a token with write access to it, stored as
   the secret `TAP_TOKEN`.
2. Every release: the `tap` job runs `scripts/dist/tap-bump.sh <version>`: it
   copies the formula, writes the version and hashes from the release
   `SHA256SUMS` and pushes one commit. No change: no commit.
3. Check locally (needs `brew`): `scripts/dist/fake-release.sh 1.2.3 <dir>`,
   `scripts/dist/brew-check.sh` (style and audit) and
   `scripts/dist/brew-check.sh --install <dir> 1.2.3` (install, test, uninstall).
4. Users: `brew install mesh-town/macos/townjump`.

homebrew-core is not used: the formula lives in the own tap only.

## Windows installer

`scripts/dist/windows-installer.sh <zip> [--out <dir>]` builds
`townjump-setup-<version>.exe` with NSIS (`packaging/windows/installer.nsi`)
from the Windows zip of the release; the `windows-installer` job does this in
CI. The installer:

- copies `townjump.exe` and the license to `%ProgramFiles%\Townjump`
  (x64 and ARM64 Windows; the x64 binary runs on ARM64 under emulation);
- adds the folder to the system `PATH` (`packaging/windows/path-entry.ps1`,
  tested by `path-entry.tests.ps1`) and tells running programs about it; a
  new terminal window sees `townjump`;
- registers an uninstaller that removes the files and the `PATH` entry;
- installs silently with `/S` (`townjump-setup-<version>.exe /S`).

The installer is not code-signed, so Windows SmartScreen shows a warning on
the first run ("More info", then "Run anyway"). The same applies to the winget
package, which points at this installer.

## winget

Manifests are rendered from `packaging/windows/winget/*.yaml.in` for package
`MeshTown.Townjump`:

```
scripts/dist/winget.sh render <version> townjump-setup-<version>.exe <out>
```

1. First version, by hand: run `render`, check the result against the schema
   (`scripts/dist/winget-schema-check.py <schema-dir> <out>`), copy
   `manifests/m/MeshTown/Townjump/<version>/` into a fork of
   `microsoft/winget-pkgs` and open a pull request from the fork.
2. Later versions: the `winget` job runs `scripts/dist/winget.sh submit
   <version>`, which calls `komac update MeshTown.Townjump` with the URL of the installer in the release. It needs
   `WINGET_TOKEN`. While the package is not yet in `winget-pkgs` it only
   prints a warning and exits 0 (step 1 is still to be done).
3. `komac` is installed by the job with a pinned version and a checked
   SHA-256.
4. Users: `winget install MeshTown.Townjump`.

## OpenWrt

OpenWrt gets a signed package feed built without the SDK (`openwrt/feed/build_feed.py`, `.ipk` and `.apk`); see
`openwrt/README.md`. Keenetic (Entware) has its own feed, see `keenetic/README.md`.

The package is served from its own signed feed; it is not submitted to `openwrt/packages`.
