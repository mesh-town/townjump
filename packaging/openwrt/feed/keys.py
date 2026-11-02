"""Signing and verification: usign for opkg indexes, a P-256 key for apk indexes. Secrets are never printed."""
import shutil
import subprocess
import tempfile
from pathlib import Path

USIGN_PUB = "townjump-usign.pub"
APK_PEM = "townjump-apk.pem"
KEY_COMMENT = "Mesh Town OpenWrt feed"


def _ok(cmd) -> bool:
    return subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0


def sign_usign(file, sec, usign_bin) -> Path:
    subprocess.run([str(usign_bin), "-S", "-m", str(file), "-s", str(sec)], check=True)
    return Path(str(file) + ".sig")


def verify_usign(file, pub, usign_bin) -> bool:
    return _ok([str(usign_bin), "-V", "-m", str(file), "-p", str(pub)])


def verify_apk_index(adb, pubdir, apk_bin) -> bool:
    """True only if the index is signed by the key in pubdir (apk wants an absolute keys dir)."""
    with tempfile.TemporaryDirectory() as tmp:
        shutil.copy(Path(pubdir) / APK_PEM, Path(tmp) / APK_PEM)
        return _ok([str(apk_bin), "--keys-dir", str(Path(tmp).resolve()), "verify", str(adb)])


def usign_key_matches(sec, pub, usign_bin) -> bool:
    with tempfile.TemporaryDirectory() as tmp:
        probe = Path(tmp) / "probe"
        probe.write_text("townjump key check\n")
        sign_usign(probe, sec, usign_bin)
        return verify_usign(probe, pub, usign_bin)


def _public_pem(args) -> bytes:
    done = subprocess.run(["openssl", "pkey", *args, "-pubout"], capture_output=True, check=True)
    return done.stdout


def apk_key_matches(sec, pem) -> bool:
    return _public_pem(["-in", str(sec)]) == _public_pem(["-pubin", "-in", str(pem)])


def ephemeral_apk_key(tmp, pubdir):
    """A one-off P-256 key: the secret goes to tmp, the public half to pubdir under its published name."""
    sec = Path(tmp) / "apk-sec.pem"
    subprocess.run(["openssl", "ecparam", "-name", "prime256v1", "-genkey", "-noout", "-out", str(sec)],
                   check=True)
    (Path(pubdir) / APK_PEM).write_bytes(_public_pem(["-in", str(sec)]))
    return sec


def ephemeral_keys(tmp, usign_bin):
    """One-off keys for dry runs: (usign secret, apk secret, directory with both public keys)."""
    tmp = Path(tmp)
    pubdir = tmp / "pub"
    pubdir.mkdir(parents=True, exist_ok=True)
    usign_sec = tmp / "usign.sec"
    subprocess.run([str(usign_bin), "-G", "-c", KEY_COMMENT, "-p", str(pubdir / USIGN_PUB),
                    "-s", str(usign_sec)], check=True, stderr=subprocess.DEVNULL)
    return usign_sec, ephemeral_apk_key(tmp, pubdir), pubdir
