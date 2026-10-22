# Filemanager: `--filemanager` and `livefiles` 1.1.0

Sources: `holesail/src/bin/holesail.mjs` (the `argv.filemanager` branch), `livefiles/index.js:Livefiles` (`_open`, `handleRequest`, `authenticate`, `handleGetRequest`, `handlePostRequest`, `serveFile`, `createFolder`, `createFile`, `info`). Based on reading the code; HTTP behavior was additionally recorded against a live JS run (see "The JS reference: beyond the sections above"). The Rust port is the crate `townjump-filemanager` (the CLI branch — [cli.md](cli.md)); differences from JS are in the section "Port deviations".

## What the command does

`holesail --filemanager <path> [--role admin] [--username U] [--password P] [--host H] [--port N] [--public] [--key K] [--log …]`

1. Argument checks and codes — [cli.md](cli.md), steps 7–10 (UDP, `--live`, `--connect`, path).
2. `new Livefiles({ path, role, username, password, host, port })`; `await ready()` — starts a local HTTP server on `host:port` (defaults `127.0.0.1:5409`; boolean flag values are ignored). A `listen` error does NOT cause code 1 via the callback (the `listen` callback only nominally receives the error): a busy port is an `error` event without a handler → an exception, code 1 (quirk 12).
3. `new Holesail({ server: true, port: argv.port || 5409, host: argv.host || '127.0.0.1', secure, key: argv.key, log })`, where `secure = argv.public !== undefined ? !argv.public : true`; the holesail server forwards traffic to the same `host:port`, i.e. to the filemanager's HTTP server. The DHT record is ordinary ([dht-records.md](dht-records.md)), `udp` is unset. `--host` sets both the bind address of the HTTP server and the forwarding address.
4. Banner: `Holesail Filemanager Started` and a block with `username`, `password`, `role` (the password is printed in plain text) — [cli.md](cli.md), section "Banner". Stopping is a signal via `goodbye`: `conn.close()`, then `fileServer.close()`.

## `Livefiles`: constructor

- `path` is required and must exist: otherwise `Error('INCORRECT OR NO PATH SPECIFIED')` (quirk 12, code 1).
- `role`: `'admin'` → `admin`, anything else — `user`. The login and password default to `admin`/`admin`; boolean flag values (`--username` without a value) are ignored.
- Limits: `maxRequestSize` = 10 MiB, `streamBufferSize` = 64 KiB.
- `info`: `{ type: 'filemanager', host, port, role, username, password, maxRequestSize, streamBufferSize }`.

## HTTP behavior

Every request first goes through an authorization check (Basic): the `Authorization` header, the part after the first space is taken (the word `Basic` is not checked), decoded from base64, the string is split on `:` into `name` and `password` (a password containing `:` is cut at the first colon and will not match). No header or a wrong pair → `401`, the header `WWW-Authenticate: Basic realm="Filemanager"`, the body `Authentication required.`.

`urlPath = decodeURIComponent(req.url)` (an invalid `%` sequence is an unhandled exception, the process ends with code 1), `fullPath = path.join(path, urlPath)` — there is no protection against escaping the root (`..`) (fixed in the port — see "Port deviations"). Methods other than `GET` and `POST` get no response at all (the connection hangs).

| Request | Result |
|---|---|
| `GET` directory | `200 text/html` — a page with a listing (the directory size is the sum of the sizes of only its DIRECT children that are not directories, `calculateDirectorySize`: nested directories are not entered, any read error gives 0; computed in batches of 10), a creation form and styles; the markup is in `Livefiles.listDirectory` (~370 lines of template), is not reproduced in this specification; it is taken verbatim from the source as a resource |
| `GET` file | `200`, `Content-Length`, `Content-Type` by extension (the `getContentType` table, default `application/octet-stream`), `Accept-Ranges: bytes`, `Content-Disposition: attachment; filename="<name>"`; the body is streamed in 64 KiB chunks |
| `GET` file with `Range: bytes=S-E` | `206`, `Content-Range: bytes S-E/size`, `Content-Length`; `E` defaults to `size-1`; `S >= size` or `E >= size` → `416` with `Content-Range: bytes */size`; suffix ranges (`bytes=-N`) are not supported (`S` = `NaN`) |
| `GET` nonexistent | `404 text/plain` `File Not Found` |
| `GET`, other error | `500 text/plain` `Internal Server Error`; a read error — `Error reading file.` / `Error reading file` |
| `POST` (body `application/x-www-form-urlencoded`: `item_type`, `name`, `directory`) | a body up to 10 MiB (`413` `Request entity too large` and a connection drop above that); `item_type` ∈ {`folder`,`file`} and a non-empty string `name`, otherwise `400` `Bad Request: Missing or invalid form data.`; `folder` not from `admin` → `403` `Forbidden: Only admin users can create folders.`; creating `path/<directory or .>/<name>`: a folder (`mkdir -p`) or an EMPTY file (`writeFile` with an empty string, an existing file is truncated); success — `302` with `Location: <urlPath>`; a creation error — `500` `Error creating folder.` / `Error creating file.`; anything else — `500` `Internal Server Error` |

Uploads, deletions, renames and writing file contents are not supported. The `user` role cannot create folders but can create empty files (per the source).

## Notes for porting

- A byte-for-byte match of the directory page HTML is needed only if tests compare it; for parity, codes, headers and forms are enough. The port takes the template from livefiles 1.1.0 as the resource `page/*` and checks the round trip with the script `filemanager-extract-page.mjs --check`.
- In this branch the filemanager and the holesail server get the same `--host`/`--port` (the default port 5409 is the same for both), so the DHT record points to the filemanager's HTTP server.
- The JS reference was captured on Node 24.4.0 + livefiles 1.1.0 (holesail 2.4.4), ~190 requests per role (user and admin); the raw recording is not kept in the repository; the port matches it apart from a few allowed differences. The recording adds the following beyond the sections above.
- Not verified by a live run: a request over HTTP/1.0 (including HTTP/1.0 + keep-alive), `Expect: 100-continue`, a request without `Host`, raw bytes >= 0x80 in the target, an empty file with a non-Latin-1 character in the name (the crash is inferred from the Node source). The port follows Node/llhttp behavior for these branches, not the recording.

## The JS reference: beyond the sections above

- The common header tail of every response: `Date`, `Connection: keep-alive`, `Keep-Alive: timeout=5`, then `Transfer-Encoding: chunked` if there is no `Content-Length`. The listing differs in size between roles (admin has the creation form).
- The methods `HEAD`/`OPTIONS`/`PUT`/`DELETE`/`PATCH`/`TRACE` hang with the right password; without a password they get `401`, `HEAD` — 4 headers, no body and no `Transfer-Encoding`. Absolute-form `GET http://x/...` does not hang: the target goes into `decodeURIComponent` as is, the reply is `404 File Not Found` (without a password `401`).
- `Authorization`: `Bearer ...` is accepted as Basic (200); lowercase `basic ` too; with duplicate headers the first wins (right, wrong -> 200; wrong, right -> 401); base64 with garbage (`!!!!`) -> `401`, not a crash.
- `Range`: a `RangeError ERR_OUT_OF_RANGE` crash on `bytes=-5`, `5-2`, `abc`, `items=0-5`, `bytes=`, `BYTES=0-1`, `bytes=-`; duplicate `Range` headers are joined with `, `; `bytes=0-1,3-4` and `bytes= 0-5` are tolerated (`parseInt`); `bytes=0--1` returns the whole file; `bytes=0-0` on an empty file -> `416` with `Content-Range: bytes */0`. All `416`s have an empty body, with the headers `Content-Range`, `Date`, keep-alive, `Transfer-Encoding: chunked`.
- A file without permissions (chmod 0) -> the connection hangs (0 bytes).
- `413`: the reply reaches the client (keep-alive and chunked headers, the body `Request entity too large`, 24 bytes), then `req.destroy()`.
- A URL 20 KiB long -> `431` with `Connection: close`, without `Date`, the server closes. `#` and `?` in the target remain part of the path (`decodeURIComponent(req.url)` without parsing) -> `404 File Not Found`. `/%00` -> `500 Internal Server Error`; NUL in a POST `name` -> `500 Error creating file.`. `%C0%AE` -> `URIError`, a crash.
- With plain keep-alive, the server closes the connection 6004 ms after the reply (not exactly 5 s).
- JS has no protection against escaping the root (a listing of the parent and files outside the root, 200); the port replies `403`. Paths that do not exist and are outside the root give 404 in both.
- The recording marks a process crash after a reply as `process-crash` on the request during which the process died.

## JS crashes that the port repeats

The JS process crashes with an unhandled exception on these inputs (code 1, text in stderr); under `--safe-mode`, instead of crashing the port replies `400 Bad Request`. Without the flag the port does the same: all cases go through one function `conn::on_crash` (it calls the `on_fatal` callback, the socket is closed without a byte of reply), so that the crash can later be replaced by a 400 reply in one place.

- An invalid `%` sequence in the request target: `URIError: URI malformed`. It fires BEFORE the password check, i.e. even without credentials.
- An `Authorization` header of one word (`Basic` or `Basic ` without credentials): there is no second word, `Buffer.from(undefined)` gives `TypeError [ERR_INVALID_ARG_TYPE]`.
- An empty file with a character outside Latin-1 in the name (`φ.txt`, 0 bytes): `Content-Disposition` fails validation — `TypeError [ERR_INVALID_CHAR]`. A non-empty file with such a name does NOT crash JS: with a nonzero `Content-Length` Node writes `Content-Disposition` via `Buffer.from(v, 'latin1')`, each character is truncated to its low byte (`ŀŃŁŁĺĸĹ ńİĹĻ.txt` -> `@CAA:89 D09;.txt`); the crash remains only if after truncation the bytes do not form a valid string (a control character, non-utf8 gives U+FFFD).
- `Location` with non-Latin-1 characters in the POST reply: `writeHead(302)` throws `ERR_INVALID_CHAR` but manages to write `statusMessage = 'Found'`; the `catch` writes `writeHead(500)` with that reason — the reply `HTTP/1.1 500 Found` with the body `Error creating …`, although the folder or file has already been created.
- `Range` with a non-numeric start/end or `start > end`: `RangeError [ERR_OUT_OF_RANGE]` from `createReadStream`.
- NUL in the path: `stat` fails with an error other than "no such file" → `500 Internal Server Error` (not a crash, but different from 404).

A method other than `GET`/`POST` (`HEAD`, `OPTIONS`, `PUT` …): no reply, the connection hangs until the client closes it. The exception is a request without the right password: the reply is `401` (for `HEAD` — without a body and without `Transfer-Encoding`).

## Port deviations

- `..` and `%2e%2e` beyond the root (the GET path, and also `directory`/`name` in POST after `path.join`) → `403`, `Content-Type: text/plain`, the body `Forbidden`. JS has no check; the port refuses the escape even for a root made only of `..` (`../..`). Covered by `crates/townjump-filemanager/tests/http_server.rs` (the tests are split across `http_server.rs`, `http_auth.rs`, `http_files.rs`, `http_post.rs`). This case cannot be recorded in `cli-matrix.json`: it holds only CLI output, without HTTP.
- On Windows `\` is a path separator, and a component with `:` (a drive, an ADS stream) after the root is an escape from the root: `403`, as for `..`. Covered by `escapes_root_as` in `codec/jspath.rs`; on posix `\` is an ordinary character.
- Windows device names and trailing dots or spaces, without `--safe-mode` (the pure rule is in the Safe mode section): the port does not hang or crash on any of them, so it is left unchanged. JS gives `404` (`fs.stat` returns `ENOENT`) for `secret.txt.`, `secret.txt `, `CON`, `NUL`, `nul.txt`, `COM1`, `LPT9.log`, `aux.`, `PRN`, `CON.txt`, `CONIN$`, `sub\NUL`; the port walks ordinary Win32 paths and differs on four of them: `/secret.txt.` and `/secret.txt%20` serve `secret.txt` (`200`, Win32 trims the trailing dot or space), `/NUL` and `/sub/NUL` give `500`. On current Windows versions `root\nul.txt`, `root\CON`, `root\COM1` are plain names (`404`, or a real file after a POST), so the rule is protective for Windows versions that resolve them to devices; `--safe-mode` refuses all of them with `403`. The Windows variants of the symlink tests are skipped when the process lacks the symlink privilege.
- Windows path forms without `--safe-mode`: names are case-insensitive (`/HELLO.TXT`) and `\` is a separator (`/sub%5cinner.txt`), both served with `200`, as JS does (`fs.stat` in Node agrees). A trailing slash on a file (`/hello.txt/`): Node `fs.stat` reports a file (the read that follows was not checked), the port gives `500`. Not a hang or a crash, so it is left unchanged; `--safe-mode` answers `403`. Over HTTP the Windows path forms are covered by `tests/http_safe_win.rs` (safe mode: `..%5c`, `C:`, `hello.txt:stream`, and POST `directory` outside the root give `403`; backslash and case give `200`).
- Without the flag, as in JS: symbolic links are dereferenced (a link inside the root leading outside is served and accepts writes); fixed under `--safe-mode`.
- Without the flag, as in JS: POST `item_type=file` truncates an existing file (the `user` role too); fixed under `--safe-mode`.
- Without the flag, as in JS: names in the listing (`href`, `value`, `action`) are not escaped in HTML; fixed under `--safe-mode`.
- `--port abc`: Node listens on a unix socket `abc` in the current directory and prints the banner `127.0.0.1:abc`; the port returns a port error before the banner (`Err` from `ready()`).
- A second signal during shutdown also gives code 130 (in JS the signal itself kills the process). SIGINT and SIGTERM give 130, like `graceful-goodbye`.
- The POST body is decoded as a whole; JS (`body += chunk.toString()`) corrupts a multibyte character at a TCP chunk boundary — not reproduced.
- The `413 Request entity too large` reply: in the reference recording it arrives (the headers `Connection: keep-alive`, `Keep-Alive: timeout=5`, `Transfer-Encoding: chunked`), then `req.destroy()` tears down the socket; the port writes the same headers and closes the connection.
- The Node 24 timeouts (headers 60 s, request 300 s, keep-alive idle 5 s) are reproduced without emulating the coarse precision of Node's "every 30 s" check.
- `localhost` in `--host`: Node takes the first `dns.lookup` address, the port takes the first `lookup_host` address, like Node.
- The order of directory entries is `strcmp` over the name bytes (like libuv); on Windows the `readdir` order is not verified.
- The busy-port error: `ready()` returns `Ok`, the error text goes to `on_fatal` — the CLI prints the banner, then stderr and code 1, like JS.
- `--password 007` (a numeric value after argv parsing) never matches, as in JS (`===` with a string).
- The name and password are always both compared, byte by byte in constant time with respect to the position of the first difference (JS: `===` with `&&`).
- `RLIMIT_NOFILE`: like Node at startup, the port raises the soft limit to the hard one (with an infinite hard limit — by binary search up to 1 << 20); `crates/townjump/src/cli/fd_limit.rs`, errors are silent, and it is a no-op on non-Unix platforms.

## Safe mode (`--safe-mode`)

A port extension, not in JS. Enabled by the flag `--safe-mode` (`FilemanagerOpts.safe_mode`, `Config.safe_root`); `--no-safe-mode` turns it off, `--safe-mode=false` turns it on (the string is truthy). Without the flag the replies are as in JS. The mode's code is `route/safe.rs`.

- Startup: the password is unset or equals `admin` (with any name) → stderr `Error: --safe-mode refuses the default password; set --password (and --username)`, code 1. `--safe-mode=false|0|no` (any case) turns the mode off. A root that cannot be canonicalized → `Error: --safe-mode cannot resolve the root directory: <error>`, code 1. `confined` is lifted to the ancestor only on `NotFound`, any other resolution failure → 403. `Location` in the POST reply under the flag encodes `#` and `?` too.
- The order of request checks after the password: Host, method, (POST) role, (POST) Origin. Without the right password the client sees only `401` (and `400` on a broken `%` in the URL and a one-word `Authorization`).
- `Host` not in the allowed set → `403`, `Forbidden: Host not allowed.` The allowed set (case-insensitive, `:<digits>` optional): an IPv4 literal, IPv6 in brackets, a hostname made of `[A-Za-z0-9-]` labels, allowed if it has a single label (`nas`, `localhost`) or its last label is `localhost`/`local`. No header (HTTP/1.0) — pass. The trade-off: access through a public DNS name (for example behind a reverse proxy) is not possible. The code is `403`, not `421`: in browsers `421` means "retry on a different connection".
- Origin (POST only): present and its authority does not match `Host` (case-insensitive), or the scheme is not `http`/`https`, or `null` → `403`, `Forbidden: Cross-origin request.`; no Origin — pass.
- A method other than GET/POST (including HEAD) → `405`, `Allow: GET, POST`, `Content-Type: text/plain`, the body `Method Not Allowed`.
- The `user` role: any POST → `403`, `Forbidden: Only admin users can create files.` (before the body is read).
- Any JS crash → `400`, `text/plain`, `Bad Request`, the connection is closed; `on_fatal` is not called.
- Symbolic links: a path is allowed if the canonical path of its nearest existing ancestor lies inside the canonical root; otherwise GET and POST → `403 Forbidden`. Links pointing outside are not shown in the listing, `dir_size` does not count them.
- POST `item_type=file` creates via `O_CREAT|O_EXCL`: already exists (a file, a directory, a dangling link) → `409`, `Conflict: File already exists.`.
- Windows path names (the rule is active only in Windows builds; `Config.win_rules`, the pure function `codec/winname.rs::unsafe_component`, `jspath::win_names_as`): any path component after the root is refused with `403`, `text/plain`, `Forbidden` (like a root escape; GET, and POST for both `directory` and `name`) if it (1) ends with `.` or a space, (2) has a base (the part before the first `.`, trailing spaces removed) equal, case-insensitively, to `CON`, `PRN`, `AUX`, `NUL`, `COM0`-`COM9`, `LPT0`-`LPT9`, `COM¹ COM² COM³ LPT¹ LPT² LPT³`, with any extension (`nul.txt`), or (3) equals `CONIN$` or `CONOUT$`. The root itself is not checked (the operator chose it). False positives such as `com0.txt` or `lpt0` are accepted. Without the flag the behavior is as in JS. Entries with such names are left out of the listing.
- Listing HTML: `href`, `action`, `value` and the text of `<option>` are escaped with `escape_html`.
- `Content-Disposition`: `attachment; filename="<ascii>"; filename*=UTF-8''<pct>` (RFC 5987), no crash on non-ASCII names.
- `Location` is encoded like `encodeURI`; `500 Found` is unreachable in the mode.
- Help text: `--help filemanager` contains the line `--safe-mode` (`townjump::cli::help::SAFE_MODE_LINE`); `help_text/filemanager.txt` remains a copy of JS.

### TODO (open)

A quadratic `drain` in `http/head.rs`/`body.rs`; strict chunked per llhttp; listing traversal limits; a connection ceiling and `Hang` → `500` on an `open`/FIFO failure, write deadlines; `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`; `access(R_OK)` instead of `open` in `readable`; an absolute path in `<option>` with an abnormal root; TOCTOU between `canonicalize` and `open`; a pause after a failed password attempt.

## Map: JS function → port file

All files are in `crates/townjump-filemanager/src/`.

| JS (livefiles/index.js or Node) | Rust file |
|---|---|
| the `Livefiles` constructor, defaults | `opts.rs` |
| `_open`, `ready`, `close`, the `listen` error | `server.rs` |
| `http.Server`, timeouts, keep-alive, crash | `conn.rs` |
| Node request and response parsing (llhttp) | `http/head.rs`, `http/body.rs`, `http/response.rs`, `http/date.rs` |
| `decodeURIComponent` | `codec/uri.rs` |
| `Buffer.from(…, 'base64')` | `codec/base64.rs` |
| `querystring.parse` | `codec/query.rs` |
| `path.join`/`normalize`/`extname`/`basename` | `codec/jspath.rs` |
| `formatBytes`, `escapeHtml`, `parseInt`, `String.replace` | `codec/text.rs` |
| `getContentType` | `codec/mime.rs` |
| `Range` parsing and `createReadStream` checks | `codec/range.rs` |
| `handleRequest`, `authenticate` | `route/mod.rs` |
| `handleGetRequest`, `serveFile` | `route/get.rs` |
| `handlePostRequest`, `createFolder`, `createFile` | `route/post.rs` |
| `listDirectory`, `calculateDirectorySize`, `getDirectoryOptions` | `route/listing.rs` |
| page HTML, form, icons, logo | `page/mod.rs` and the resources `page/*` (livefiles 1.1.0 under GPLv3, compatible with AGPL-3.0-only via GPLv3 §13) |
| the `--filemanager` branch in `holesail.mjs` | `crates/townjump/src/cli/run/filemanager.rs` |
