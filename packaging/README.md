# Packaging

Packaging of townjump: prebuilt binaries on GitHub Releases, a Homebrew
formula (own tap), a Windows installer with a winget manifest, and
package feeds for Debian, OpenWrt and Keenetic. A release tag (`v<version>`, equal to `version` in the root
`Cargo.toml`) starts everything; the Homebrew formula and the Windows
installer install the binaries of that release, they do not build from source.
The Debian, OpenWrt and Keenetic feeds repack the static musl binaries of the
release; they do not build from source.

Unofficial Rust port of Holesail, not affiliated with Holesail / LIVEPORT.
The installed command is called `townjump` (the npm package `holesail`
installs `holesail`, so the names do not collide). The help text still says
`holesail`, as in the original (the formula mentions it in `caveats`).

## Release

Flow: tag, then the jobs of `.github/workflows/release.yml`, then the assets
of the GitHub release, then the tap and winget jobs.

1. Set the same `version` in `Cargo.toml` and `CHANGELOG.md`, commit.
2. Tag and push: `git tag v0.1.0 && git push origin v0.1.0`.
3. `check-tag` compares the tag with the crate version. `build` makes one
   archive per target (`.tar.gz`; `.zip` for Windows). `windows-installer`
   builds `townjump-setup-<version>.exe` from the Windows zip.
4. `publish` adds the source archive and `SHA256SUMS`, checks the full list
   of assets and creates the release (`scripts/dist/publish.sh`).
5. `tap` and `winget` (reusable workflows `tap-bump.yml`, `winget.yml`) run
   after `publish`; both can also be started by hand with a version.

Settings (repository variable and secrets; an empty secret skips the step
with a `::notice::`, it does not fail the release):

| Name | Kind | Meaning |
|---|---|---|
| `DIST_REPO` | variable | Repository that holds the release assets. Empty: the repository that runs the workflow. |
| `DIST_TOKEN` | secret | Token with write access to `DIST_REPO`. Required only when `DIST_REPO` is another repository (an error without it). |
| `TAP_TOKEN` | secret | Token with write access to the Homebrew tap. Empty: the tap is not updated. |
| `WINGET_TOKEN` | secret | Token for `komac` (a fork of `winget-pkgs` and pull requests). Empty: nothing is submitted. |

`check-dist.yml` runs the script tests, the manifest schema check and the
Homebrew checks on every change under `packaging/`, `scripts/dist/` and the
release workflows. New package jobs go after the marker comment at the end of
`release.yml`; the list of release assets is one function in `publish.sh`.

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

homebrew-core needs a repository older than 30 days with enough stars/forks;
not before that.

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

The official `openwrt/packages` feed requires a real name and a non-private
e-mail in `Author` and `Signed-off-by`, so the package stays in an own feed.
