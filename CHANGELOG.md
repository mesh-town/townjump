# Changelog

All notable changes are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project uses [Semantic Versioning](https://semver.org/).

## 0.1.0 — Unreleased

First public version of the port of Holesail 2.4.4.

- CLI `townjump` (help text and messages as in the original): `--live`, `--connect` (and a bare key), `--filemanager`, `--lookup`, `--help`, `--version`;
  options `--host`, `--port`, `--udp`, `--public`, `--key`, `--log`.
- Wire- and key-compatible with JavaScript Holesail 2.4.4 (TCP and UDP, private and public keys).
- DHT layer: a patched fork of peeroxide (`fork/peeroxide`).
- Web file manager, a port of livefiles 1.1.0.
- Reference vectors and an interop test harness against JavaScript Holesail.
