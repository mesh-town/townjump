# Compatibility with Holesail

townjump follows JavaScript Holesail **2.4.4**. Holesail 3.x is not supported (see below).

## Compatible

- **Wire and keys.** A townjump server and client interoperate with JavaScript `holesail` 2.4.4 in all four
  combinations, over TCP and UDP, with private and public keys. The test suite checks this against a local DHT test
  network and against reference vectors captured from the JavaScript implementation.
- **Command line.** The same commands and options, the same help text and messages, the same exit codes. Even the
  quirks of the original are kept, so that scripts written for `holesail` keep working: for example
  `--public=false` still means public, and `--live P --port Q` ignores `Q`. The list is in
  [spec/holesail/README.md](spec/holesail/README.md).
- **`--version`** prints `2.4.4 (townjump <version>)`: the Holesail version this port follows, then the port's own
  version.

## Different

- **Command name.** The binary is `townjump`, so it can be installed next to the npm `holesail`.
- **Secrets in logs.** With `--log`, the original prints the private secret in full. townjump prints its first
  8 characters and replaces the rest with `*`.
- **File manager.** `--safe-mode` is new. Paths that escape the shared directory (`..`, `%2e%2e`) are refused with
  `403`; form fields with multi-byte characters are decoded correctly; the user name and password are compared in
  constant time.
- **UDP on Windows.** Forwarding with `--udp` does not stop when the remote side is briefly unreachable (ICMP "port
  unreachable"), and datagrams up to 64 KiB are accepted.
- **Faster shutdown.** Ctrl+C on a server withdraws its DHT announcement in parallel with a 1.5 s limit; the server
  exits in under a second instead of several seconds. A record that could not be withdrawn expires on its own.
- **Network changes.** The node's public address follows the last 16 observations, so after switching networks
  (for example Wi-Fi to mobile data) it moves to the new address without a restart, after a few minutes of DHT
  traffic.
- **Same public IP.** A client behind the same public IP as a townjump server connects without hole punching.
- **Extras.** `--save-nodes*`, `--safe-mode` and local network discovery exist only in townjump; see
  [usage.md](usage.md).

## Not supported

- **Holesail 3.x.** Version 3.0 changed the connection string and the wire header; townjump speaks 2.4.4 only.
- **Automatic reconnection** of the client is not implemented, as in the original.
- **Network interface events.** After a network change the node learns its new address from DHT traffic only, not
  from the operating system, so this takes minutes.
- **Persistent DHT node.** A long-running node does not become a full DHT node (as hyperdht does after about
  20 minutes behind a friendly NAT).

## Performance

Measured on one long-distance path with a 220-250 ms round-trip time, plain UDX streams; your numbers will differ:

| Direction | townjump | holesail (JS) |
|---|---|---|
| Upload | 78-86 Mbit/s | 72-75 Mbit/s |
| Download | 30-37 Mbit/s | 30-37 Mbit/s (the limit of the path) |

On a single machine (loopback, 64 MiB transfers) townjump moves 28-32 MiB/s against 10-17 MiB/s for JavaScript.
Opening a new connection takes about the same time as JavaScript on a repeat connection (about 1.5 s on such a
path); the first, cold connection is slower (2.6-3.1 s against 1.9-2.6 s). Loss recovery was reworked recently;
there are no published numbers for heavy packet loss yet.
