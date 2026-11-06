# Publication keys

Everything that is signed or pushed during a release uses keys generated once by `scripts/dist/gen-keys.sh <dir>`.
The directory must be outside any git repository; the script refuses otherwise. Secret keys are kept only there and in
GitHub Actions secrets, never in this tree.

| Purpose | Format | Secret (file in the key directory) | Public half in this tree |
|---|---|---|---|
| apt repository | OpenPGP ed25519, no expiry, no passphrase | `APT_GPG_KEY` (`townjump-apt.sec.asc`) | `packaging/debian/keys/townjump.gpg` (binary keyring) |
| opkg index (OpenWrt 24.10 and older) | usign ed25519 | `OPENWRT_USIGN_KEY` (`townjump-usign.sec`) | `packaging/openwrt/keys/townjump-usign.pub` |
| apk index (OpenWrt 25.12 and newer) | P-256 EC key, PEM | `OPENWRT_APK_KEY` (`townjump-apk.sec`) | `packaging/openwrt/keys/townjump-apk.pem` |
| push to the distribution repositories | SSH ed25519 | `DIST_DEPLOY_KEY`, `OPENWRT_DEPLOY_KEY`, `KEENETIC_DEPLOY_KEY` | public halves are registered as write deploy keys in `townjump-dist`, `townjump-openwrt`, `townjump-keenetic` |

All secrets are repository secrets of `mesh-town/townjump`. The OpenPGP identity is
`Mesh Town <340663027+mesh-town@users.noreply.github.com>`. The key directory also holds `townjump.asc` (the public key,
armored), `townjump-apt.rev` (a revocation certificate) and `fingerprints.txt`.

## Install the public keys

```
scripts/dist/install-public-keys.sh <key-dir>
```

The script first signs test data with every secret key and verifies it with the matching public key; one mismatch aborts
it before anything is copied. Then it places the three public files above. Commit them; the builders read them from there.

## Check a fingerprint

```
gpg --show-keys --with-fingerprint packaging/debian/keys/townjump.gpg
```

The fingerprint must equal the one in `fingerprints.txt` and in the apt instructions of the release. For the other
public keys compare the SHA-256 lines of `fingerprints.txt` (`sha256sum <file>`). A user's machine holds the public key
only (`/usr/share/keyrings/townjump.gpg`, `/etc/apk/keys/townjump.pem`, the opkg key store).

## Rotate

1. Run `gen-keys.sh` into a new empty directory outside the repository.
2. Update the repository secrets and the deploy keys (the script prints the table).
3. Run `install-public-keys.sh`, commit the new public keys and publish a release; clients must install the new public
   key (`apt`: new keyring, `opkg-key add`, new `.pem`), so announce the change.
4. Remove the old deploy keys from the distribution repositories.

If a secret key leaks, import the revocation certificate (`townjump-apt.rev`, remove its leading `:` guard line first),
publish the revoked key and rotate as above.
