Real apk-tools v3 samples used by test_adb_check.py (the check itself never runs apk).

Made inside a container from the alpine:edge image, apk-tools 3.0.7-r0 (aarch64), with these commands:

  openssl ecparam -name prime256v1 -genkey -noout -out sec.pem        (the secret key is NOT stored here)
  apk mkpkg --info name:townjump --info version:0.1.7-r1 --info arch:x86_64 ... --files root --output sample.apk
  apk mkndx --allow-untrusted --output packages.adb sample.apk
  apk --allow-untrusted adbsign --sign-key sec.pem sample.apk
  apk --allow-untrusted adbsign --sign-key sec.pem packages.adb

root holds one file, usr/bin/townjump, 8 bytes. sample.apk is the signed package, sample-packages.adb is the
signed index (copied as packages.adb). Both start with the ADBd (deflate) wrapper, as apk writes them by default.
