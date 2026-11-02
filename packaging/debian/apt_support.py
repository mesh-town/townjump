"""Pool and index helpers of apt_repo.py: package names, control files, pool upkeep, Packages and Release text."""
import email.utils
import gzip
import hashlib
import io
import re
import shutil
import sys
import tarfile
from pathlib import Path

POOL = Path("pool/main/t/townjump")
ARCHES = ("amd64", "arm64", "armhf")
NAME = re.compile(r"^townjump_(\d+)\.(\d+)\.(\d+)-([1-9]\d*)_(amd64|arm64|armhf)\.deb$")
HEAD = "Origin: Mesh Town\nLabel: Mesh Town\nSuite: stable\nCodename: stable\n"
TAIL = ("Architectures: amd64 arm64 armhf\nComponents: main\nDescription: townjump packages\n"
        "Acquire-By-Hash: yes\n")


class RepoError(Exception):
    pass


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def parse_name(name: str):
    """((X, Y, Z, N), arch) for a pool file name, None when it does not fit the pattern."""
    m = NAME.match(name)
    if not m:
        return None
    return tuple(int(x) for x in m.groups()[:4]), m.group(5)


def read_control(path: Path) -> str:
    """Text of the control file of a .deb; RepoError when the file is not a readable deb."""
    try:
        raw = Path(path).read_bytes()
        if raw[:8] != b"!<arch>\n":
            raise ValueError("not an ar archive")
        pos = 8
        while pos < len(raw):
            head = raw[pos:pos + 60]
            size = int(head[48:58])
            body = raw[pos + 60:pos + 60 + size]
            if head[:16].decode().strip() == "control.tar.gz":
                tar = tarfile.open(fileobj=io.BytesIO(gzip.decompress(body)))
                return tar.extractfile("./control").read().decode()
            pos += 60 + size + size % 2
        raise ValueError("no control.tar.gz")
    except (OSError, ValueError, KeyError, EOFError, tarfile.TarError) as e:
        raise RepoError(f"not a deb package: {path} ({e})") from e


def _check_one(path: Path) -> None:
    parsed = parse_name(path.name)
    if parsed is None:
        raise RepoError(f"unexpected package name: {path.name}")
    fields = dict(re.findall(r"^([\w-]+):\s*(.*)$", read_control(path), re.M))
    version = "{}.{}.{}-{}".format(*parsed[0])
    if fields.get("Package") != "townjump" or fields.get("Version") != version \
            or fields.get("Architecture") != parsed[1]:
        raise RepoError(f"control of {path.name} does not match its name")


def check_incoming(incoming: Path) -> list:
    debs = sorted(Path(incoming).glob("*.deb"))
    if not debs:
        raise RepoError(f"no .deb files in {incoming}")
    for path in debs:
        _check_one(path)
    return debs


def add_to_pool(repo: Path, debs: list) -> None:
    pool = Path(repo) / POOL
    pool.mkdir(parents=True, exist_ok=True)
    for src in debs:
        dst = pool / src.name
        if not dst.exists():
            shutil.copyfile(src, dst)
        elif dst.read_bytes() != src.read_bytes():
            print(f"apt_repo: notice: pool keeps the published {src.name} (incoming differs)", file=sys.stderr)


def pool_by_arch(repo: Path) -> dict:
    out = {arch: [] for arch in ARCHES}
    pool = Path(repo) / POOL
    for path in (pool.iterdir() if pool.is_dir() else []):
        parsed = parse_name(path.name)
        if parsed is None:
            raise RepoError(f"unexpected file in pool: {path.name}")
        out[parsed[1]].append((parsed[0], path))
    return {arch: [p for _, p in sorted(items)] for arch, items in out.items()}


def prune_pool(repo: Path, keep: int) -> dict:
    if keep < 1:
        raise RepoError("--keep must be at least 1")
    pool = pool_by_arch(repo)
    for arch, paths in pool.items():
        for old in paths[:-keep]:
            old.unlink()
        pool[arch] = paths[-keep:]
    return pool


def write_direct_links(repo: Path, pool: dict) -> None:
    for arch, paths in pool.items():
        link = Path(repo) / f"townjump_{arch}.deb"
        if paths:
            shutil.copyfile(paths[-1], link)
        else:
            link.unlink(missing_ok=True)


def packages_index(debs: list) -> bytes:
    paras = []
    for path in debs:
        control = read_control(path).rstrip("\n")
        if "\n\n" in control:
            raise RepoError(f"blank line inside control of {path.name}")
        raw = path.read_bytes()
        extra = f"Filename: {POOL.as_posix()}/{path.name}\nSize: {len(raw)}\nSHA256: {sha(raw)}\n"
        at = control.index("Description:")
        paras.append(control[:at] + extra + control[at:])
    return ("\n\n".join(paras) + "\n").encode() if paras else b""


def gzip_bytes(data: bytes) -> bytes:
    buf = io.BytesIO()
    with gzip.GzipFile(fileobj=buf, mode="wb", mtime=0, compresslevel=9) as g:
        g.write(data)
    return buf.getvalue()


def release_text(files: dict, ts: float) -> str:
    date = email.utils.formatdate(ts, usegmt=True)
    out = f"{HEAD}Date: {date}\n{TAIL}"
    for label, algo in (("SHA256", hashlib.sha256), ("SHA512", hashlib.sha512)):
        out += f"{label}:\n" + "".join(f" {algo(files[p]).hexdigest()} {len(files[p])} {p}\n" for p in files)
    return out
