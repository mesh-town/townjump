# Usage

The command is `townjump`. Its help text and messages are byte-for-byte those of Holesail 2.4.4, so they say
`holesail`. Options marked "townjump only" do not exist in the original.

## Commands

```
townjump --live <port> [--host <address>] [--udp] [--public] [--key <key>]
townjump --connect <key> [--host <address>] [--port <port>] [--udp] [--public]
townjump <key>                               # same as --connect <key>
townjump --filemanager <dir> [--host <host>] [--port <port>] [--public]
            [--username <user>] [--password <pass>] [--role <admin|user>] [--safe-mode]
townjump --lookup <key>
townjump --help [filemanager]
townjump --version
```

Add `--save-nodes <file>` to `--live`, `--connect`, `--filemanager` and `--lookup` to keep the DHT routing table
between runs.

## Options

| Option | Meaning |
|---|---|
| `--live <port>` | Share the local port `<port>` (server). |
| `--connect <key>`, `<key>` | Connect to a server and expose it on a local port (client). |
| `--filemanager <dir>` | Serve a directory as a web file manager and share it. |
| `--lookup <key>` | Print what the server advertises in the DHT, without connecting. |
| `--host <address>` | Server: the local address to forward to (default `127.0.0.1`). Client: the address to listen on. |
| `--port <port>` | Client: the local port to listen on (default `8989`, or the one the server advertises). File manager: the HTTP port (default `5409`). |
| `--udp` | Forward UDP instead of TCP. Both sides must use it. |
| `--key <key>` | Server: use a fixed secret instead of a random one, so the connection string survives restarts. At least 32 characters unless `--force` is given. |
| `--force` | Drop the minimum-length check of `--key`. |
| `--public` | Use a public key (see below). |
| `--log [level]` | Enable the debug log (levels 0 to 3). |
| `--username`, `--password`, `--role` | File manager login; the role is `admin` or `user`. |
| `--safe-mode` | File manager: hardened mode (townjump only; `--no-safe-mode` turns it off). |
| `--save-nodes <file>` | Keep the DHT routing table in a file between runs (townjump only). |
| `--save-nodes-ttl <hours>` | With `--save-nodes`: age after which the file is ignored (default 168). |
| `--save-nodes-every <seconds>` | With `--save-nodes`: interval between writes (default 300). |

`townjump --help` prints the full text; `townjump --help filemanager` the file manager part.

## Keys

- **Private** (`hs://s000...`, the default): the string is the secret. Anyone who has it can connect, so treat it like
  an SSH key. The server rejects clients that do not know it.
- **Public** (`hs://0000...`, `--public`): the string is the public key; it can be published, and anyone can connect.
  Protect the service itself, for example with a password.

`--key <key>` makes the key permanent: the same string is printed after every restart. With `--log`, the original
prints the private secret in full; townjump prints its first 8 characters and replaces the rest with `*`.

## File manager

`--filemanager` serves a directory as a web file manager (a port of
[livefiles](https://github.com/holesail/livefiles) 1.1.0) and shares it at the same time:

```
townjump --filemanager ./shared --username alice --password 'a long password' --safe-mode
```

The server prints a connection string; the other side connects with `townjump <key>` and opens the page in a
browser. The page lists directories, downloads files and creates new folders and empty files; there are no uploads,
deletions or renames. Only the `admin` role (`--role admin`) can create folders. Without `--username` and `--password`
the login is `admin` / `admin`.

`--safe-mode` (townjump only) refuses to start without a non-default password and hardens the server: only `GET` and
`POST` are accepted, `POST` from another origin and requests with an unexpected `Host` header are refused (which also
blocks DNS-rebinding attacks), symbolic links that point outside the shared directory are hidden and refused,
existing files are not overwritten, and listing pages are HTML-escaped. With `--safe-mode`, the page is reachable by
IP address or a single-label or `.local` name only, not through a reverse proxy with a public domain name. The full
rules are in [spec/holesail/filemanager.md](spec/holesail/filemanager.md).

Even without it, paths that escape the shared directory (`..`, `%2e%2e`) are refused with `403`, and the user name
and password are compared in constant time.

## Saved DHT nodes (townjump only)

By default every run starts with an empty DHT routing table, as in the original. With `--save-nodes <file>` the
table is read from the file at start (if it is fresh enough) and written back every 300 seconds and on exit, so
later runs find peers faster. There is no default path.

- The file is text: a `holesail-nodes 1` line, a `saved <unix seconds>` line, then one `<ipv4>:<port>` per line.
- A missing, unreadable or older-than-`--save-nodes-ttl` file is treated as empty. An empty table is never written,
  so a run without network keeps the old file.
- Ctrl+C or `SIGTERM` saves the table and exits with code 130.

## Local network discovery (townjump only)

A server answers multicast probes on the local network, and a client probes in parallel with the DHT lookup. Peers
on the same network therefore connect directly, even without internet access. It is on by default; set
`TOWNJUMP_LAN_DISCOVERY=0` to turn it off. Firewalls may block the probes, see below.

## Environment variables

| Variable | Meaning |
|---|---|
| `TOWNJUMP_BOOTSTRAP` | Comma-separated `host:port` list of DHT bootstrap nodes, for a private DHT or a test network. Unset or empty: the public HyperDHT bootstrap nodes. |
| `TOWNJUMP_LAN_DISCOVERY` | Local network discovery. On by default; `0` (also `false`, `off`, `no`) turns it off. |
| `TOWNJUMP_DHT_ADAPTIVE` | Adaptive DHT mode: a node that is reachable from outside promotes itself to a full DHT node. On by default; `0` (also `false`, `off`, `no`) turns it off. Meant for debugging. |
| `RUST_LOG` | Enables the internal `tracing` log of the DHT and transport layers (for example `RUST_LOG=info`), separate from `--log`. |

## Firewalls

### macOS

With the application firewall on (System Settings, Network, Firewall), local-network discovery packets (the probes a
server listens for and the replies a client waits for) do not reach townjump until it is allowed to accept incoming
connections: allow it in the macOS dialog if one appears, or manually under Firewall, Options. Without that,
connections go through the DHT only. The firewall tracks each build of the binary as a separate application, so a new
version needs the permission again.

### Windows

The first time you start a server (`--live` or `--filemanager`), Windows asks whether `townjump.exe` may accept
incoming connections. Allow it on the private network. If the dialog is closed or declined, Windows creates a block
rule for that executable by itself, and local-network discovery packets never reach the server: connections go
through the DHT only. To fix it, allow townjump under Windows Security, Firewall & network protection, Allow an app
through firewall, or delete the blocking townjump rules in Windows Defender Firewall with Advanced Security, Inbound
Rules. From an elevated PowerShell, the same in two commands:

```
Get-NetFirewallApplicationFilter -Program C:\path\to\townjump.exe | Get-NetFirewallRule | Where-Object Action -eq Block | Remove-NetFirewallRule
New-NetFirewallRule -DisplayName townjump -Direction Inbound -Program C:\path\to\townjump.exe -Action Allow
```

A client needs no permission. A build in a different path counts as a new application.

## Exit codes

| Code | When |
|---|---|
| 0 | help, version, `--lookup` (also when it reports an error), no mode matched |
| 1 | an unhandled error: an invalid key on the client, the local port already in use, `--filemanager` without a path; also a fatal error with `--save-nodes` |
| 2 | an invalid combination or value of options |
| 130 | the file manager, or a server or client with `--save-nodes`, stopped by Ctrl+C or `SIGTERM` |

A plain server or client stopped by Ctrl+C is terminated by the signal, so a shell reports 130 as well. The
authoritative table is [spec/holesail/cli.md](spec/holesail/cli.md), section 9.
