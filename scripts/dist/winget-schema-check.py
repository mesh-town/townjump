#!/usr/bin/env python3
"""winget-schema-check.py <schema-dir> <manifest-dir>

Checks every *.yaml in <manifest-dir> (searched recursively) against the winget
manifest schema of its ManifestType (winget-manifest.<type>.<ManifestVersion>.schema.json
in <schema-dir>) and against the schema header comment.
Exit 0: all valid; 1: a violation (path and error printed); 2: usage or no input.
"""
import json
import sys
from pathlib import Path

import jsonschema
import yaml

TYPES = {"installer", "version", "defaultLocale"}


def check_file(path, schema_dir):
    """Return a list of error strings for one manifest file."""
    text = path.read_text(encoding="utf-8")
    # BaseLoader keeps every scalar a string: versions like 1.10.0 or 1.2.3 stay text.
    doc = yaml.load(text, Loader=yaml.BaseLoader)
    if not isinstance(doc, dict):
        return [f"{path}: not a mapping"]
    mtype = doc.get("ManifestType", "")
    mver = doc.get("ManifestVersion", "")
    if mtype not in TYPES:
        return [f"{path}: unsupported ManifestType {mtype!r}"]
    header = (
        "# yaml-language-server: $schema=https://aka.ms/winget-manifest."
        f"{mtype}.{mver}.schema.json"
    )
    errors = []
    if header not in text.splitlines():
        errors.append(f"{path}: missing schema header: {header}")
    schema_file = schema_dir / f"winget-manifest.{mtype}.{mver}.schema.json"
    if not schema_file.is_file():
        return errors + [f"{path}: no schema file {schema_file}"]
    schema = json.loads(schema_file.read_text(encoding="utf-8"))
    cls = jsonschema.validators.validator_for(schema)
    for err in sorted(cls(schema).iter_errors(doc), key=lambda e: list(e.absolute_path)):
        where = "/".join(str(p) for p in err.absolute_path) or "(root)"
        errors.append(f"{path}: {where}: {err.message[:200]}")
    return errors


def main(argv):
    if len(argv) != 3:
        print(__doc__.strip().splitlines()[0], file=sys.stderr)
        return 2
    schema_dir, mdir = Path(argv[1]), Path(argv[2])
    files = sorted(mdir.rglob("*.yaml")) if mdir.is_dir() else []
    if not schema_dir.is_dir() or not files:
        print("winget-schema-check: no schema dir or no manifests", file=sys.stderr)
        return 2
    errors = []
    for f in files:
        errors += check_file(f, schema_dir)
    for e in errors:
        print(e, file=sys.stderr)
    if errors:
        return 1
    print(f"winget-schema-check: {len(files)} manifests valid")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
