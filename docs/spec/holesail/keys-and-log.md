# Keys, connection strings and the log: observable behavior of the keys and logger crates

This file describes, in prose and tables only, what the two small crates `townjump-keys` and `townjump-logger` must do. It is written so that the crates can be rebuilt from this description, the public signatures list and the test vectors alone. Related files: [connection-string.md](connection-string.md) (the same rules from the CLI point of view), [cli.md](cli.md) section 7 (the `--log` flag), [client.md](client.md) and [server.md](server.md) (who calls what).

Conventions: "bytes of a string" always means its UTF-8 encoding. "Hex" is lowercase, two digits per byte, no separators. "z32" is z-base-32 without padding (alphabet `ybndrfg8ejkmcpqxot1uwisza345h769`); its encoder and decoder, the `KeysError` type and the secret-masking function are supplied as ready-made files and are not described again here (only their contract where the other parts rely on it: see "Fingerprint" and "z32 decode errors").

## 1. Key pair from a seed

An Ed25519 key pair is derived from a 32-byte seed in the libsodium layout:

| Field | Content |
|---|---|
| public key | 32 bytes, the Ed25519 public key of the seed (RFC 8032 key generation) |
| secret key | 64 bytes: the 32 seed bytes followed by the 32 public key bytes |

The pair is a plain value: it can be copied, compared for equality and printed for debugging (public fields, both arrays fixed in size). Derivation cannot fail.

Example: seed `ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad` gives public key `0bdf51005e5a525cc9bfb59f18070af6a8a6013106530eb8944554a5ae4c3cfa`; the secret key is the seed followed by that public key.

## 2. The seed hash

The seed for a secret string is the SHA-256 digest (32 bytes) of the bytes of the string.

| Input | Output (hex) |
|---|---|
| `abc` | `ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad` |
| empty string | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `000102…1f` (64 hex characters) | `6c86c6aac5fb24bcf5d9939cb7d7d5645ce39418f449e03b262dd4fa14b4b92b` |
| `κλωξ-κλωξ-κλωξ-κλωξ-κλωξ-κλωξ-κλωξ` | `72abee9f2010664e…` (full values in the vector file) |

The input is not trimmed, normalized or length-checked: an empty string is hashed like any other. The function cannot fail. Non-ASCII text is hashed as its UTF-8 bytes.

## 3. Connection string parsing and formatting

Two functions: parsing returns a pair (key text, optional private flag); formatting builds a string from a key and a flag.

### Parsing

The input is any text (possibly empty). Positions below are counted in UTF-16 code units, not in characters and not in bytes (a character outside the Basic Multilingual Plane, such as an emoji, takes two positions). The prefix is the five characters `hs://` (lowercase, exact).

1. Key. If the text starts with the prefix AND is at least 9 UTF-16 units long, the key is everything from position 9 on (the four units after the prefix are skipped without any check). Otherwise the key is the whole input text unchanged (including a text that is just the prefix, or the prefix with fewer than four units after it).
2. Private flag. If the unit at position 5 (the sixth) exists and is the lowercase letter `s`, the flag is "private" (true). In every other case the flag is "not set". Parsing never returns an explicit false. This check does not depend on whether the text has the prefix: a bare key whose sixth character is `s` is flagged private, and the key stays the whole text.
3. If the cut at position 9 falls in the middle of a two-unit character, the broken half is replaced by the replacement character U+FFFD in the key.

Vectors (all 49 cases are in `url-parser.json`; the notable ones):

| Input | Key | Flag |
|---|---|---|
| empty text | empty | not set |
| `hs://s000abc` | `abc` | private |
| `hs://0000abc` | `abc` | not set |
| `hs://x000abc` | `abc` | not set |
| `hs://sXYZabc` | `abc` | private |
| `hs://s000` | empty | private |
| `hs://0000` | empty | not set |
| `hs://s0000` | `0` | private |
| `hs://` | `hs://` | not set |
| `hs://s00` (8 units) | `hs://s00` | private (the sixth character is `s`) |
| `hs://S000abc` (capital S) | `abc` | not set |
| `HS://s000abc` (capital prefix) | `HS://s000abc` | private (position 5 is `s`) |
| ` hs://s000abc` (leading space) | ` hs://s000abc` | not set |
| `hs:/s000abc`, `hs//s000abc`, `https://s000abc` | the input itself | not set |
| `hs://s000hs://s000x` | `hs://s000x` | private |
| `abc`, `abcde` | the input | not set |
| `abcdes` (six characters, the sixth is `s`) | `abcdes` | private |
| `abcdefs` | `abcdefs` | not set |
| `hs://κλωξ-κλωξ` | `-κλωξ` | not set |
| `hs://s000κλωξ` | `κλωξ` | private |
| `κλωξ-κλωξ-s` | the input | not set |
| `😀😀😀abc` | the input | not set (position 5 is a half of the third emoji) |
| `hs://😀000abc` | `0abc` | not set (the emoji takes positions 5 and 6, so the key starts one unit earlier than it looks) |
| `hs://s😀abc` | `bc` | private |
| `ab😀s` | the input | not set |
| a 52-character public z32 key whose sixth character is `s` | the input | private |
| a 52-character public z32 key with another sixth character | the input | not set |

### Formatting

Formatting returns the prefix `hs://`, then the four characters `s000` if the mode is private or `0000` if it is public, then the key text unchanged. No validation, no escaping. Examples: key `abc`, private gives `hs://s000abc`; key `abc`, public gives `hs://0000abc`; empty key, private gives `hs://s000`.

Parsing the result of formatting returns the same key; the flag comes back "private" for the private form and "not set" for the public form.

## 4. Server keys

Inputs: an optional secret key text, the final mode (private or public), and a source of randomness (a callback that fills a byte buffer it is given; the buffer here is always 32 bytes). The key text is already after URL parsing. The callback is the only source of randomness, so the function is deterministic when the callback is.

An empty key text is the same as no key. The text is never trimmed.

The result has seven fields:

| Field | Meaning |
|---|---|
| key | The secret key text used by the server. Empty string only in the public mode without a key. |
| seed hex | Hex of the SHA-256 seed of the key text (64 characters). Absent ("none") only in the public mode without a key. |
| generated seed hex | Hex of the 32-byte seed the key pair was actually built from. |
| key pair | The pair derived from that seed (section 1). |
| server key | z32 of the seed in the private mode; z32 of the public key in the public mode. |
| info key | The key shown to the user: the secret key text in the private mode; z32 of the public key in the public mode. |
| url | The connection string: formatting (section 3) of the info key and the mode. |

The three cases for obtaining the seed:

| Key given | Mode | Key field | Seed hex | Seed used for the pair | Randomness |
|---|---|---|---|---|---|
| yes | private or public | the given text | hex of its SHA-256 seed | its SHA-256 seed | not called |
| no | private | 32 random bytes written as hex (64 characters) | hex of the SHA-256 seed of that hex text | that SHA-256 seed | called once, 32 bytes |
| no | public | empty string | none | the 32 random bytes themselves, WITHOUT hashing | called once, 32 bytes |

In the first row the key is used in either mode: in the public mode the key does not appear in the url, but its hash still sets the identity. Randomness is never requested in the first row and is requested exactly once in the other two.

Worked examples (randomness, where called, returns the bytes `00 01 02 … 1f`, hex `000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f`):

| Call | key | seed hex = generated seed hex | public key (hex) | server key | info key | url |
|---|---|---|---|---|---|---|
| key `abc`, private | `abc` | `ba7816bf…0015ad` | `0bdf5100…4c3cfa` | `zjhbpxhxy886wokbedxf5mtnrqaygapd1amzi8fwnd9sdhoynsso` | `abc` | `hs://s000abc` |
| key `abc`, public | `abc` | `ba7816bf…0015ad` | `0bdf5100…4c3cfa` | `bxxinyn6mjjf31p9ssxtobak64wkcyjty3jo7qrweikkmm1c8u7y` | same as server key | `hs://0000bxxinyn6mjjf31p9ssxtobak64wkcyjty3jo7qrweikkmm1c8u7y` |
| no key, private | `000102…1f` (64 hex) | `6c86c6aa…b4b92b` | `cd38df7e…6ff4b5` | `p1dcpksf9c1m37q31qqmxi6ictqq8fya6tr6yq3gfzkxwffwzrio` | `000102…1f` | `hs://s000000102…1f` |
| no key, public | empty | none; generated seed hex is `000102…1f` | `03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8` | `yqooxx9u3aemh8mo5wcqq16yufu6jitouq1o4za751dger1igghy` | same as server key | `hs://0000yqooxx9u3aemh8mo5wcqq16yufu6jitouq1o4za751dger1igghy` |

Empty key text with the private mode behaves like no key (a random key is generated). The vectors `key-derivation.json` (server records) cover keys given as text, as numbers rendered as text, empty, Greek text and absent, in both modes.

The mode is a strict boolean. Some records of `key-derivation.json` use the text `false` as the mode (truthy for the original front end, but not strictly true); such a record is a hybrid that a boolean cannot express, so for those records the server key is not compared (the key pair, seed and url are). The command line never produces such a mode.

The function cannot fail. The server never prints the generated seed hex except in debug logs of the CLI (outside these crates).

## 5. Client identity

Inputs: a key text and the mode flag (true means private).

| Mode | Result |
|---|---|
| private | "private identity": the key pair derived (section 1) from the SHA-256 seed (section 2) of the key text, plus the z32 text of that seed (z32 of the 32 bytes, 52 characters). The pair equals the pair a server with the same secret builds, so the server recognizes the client by its public key. An empty key is hashed like any other (seed z32 `hqaceowa9oqbjgz56urj1573rou4hoxrc1p3gufr1scts6n1zbko`, public key `be907b4bac84fee5ce8811db2defc9bf0b2a2a2bbc3d54d8a2257ecd70441962`). |
| public | "public identity": only a public key, the z32 decoding of the key text. The length of the decoded key is NOT checked (`abc` gives two bytes `c0 58`; the empty text gives zero bytes). No key pair is made. |

Errors: only the public mode can fail, and only through z32 decoding. A character outside the alphabet gives the error "invalid character" with the offending character and its position (index in UTF-16 units from 0). The printed message is exactly `Invalid character in base32 input: "<ch>" at position <n>`; for example key `abcdefghijklmnopqrstuvwxyz012345` gives `"l" at position 11`, a 64-character hex key gives `"0" at position 0`, `κλωξ-…` gives `"κ" at position 0`, `123456` gives `"2" at position 1`. The private mode never fails.

In the public mode a bare 64-hex key fails (the digit 0 is outside the alphabet), whereas the same text in the private mode succeeds. This is the reason the client's flag meaning is inverted in the CLI (see [connection-string.md](connection-string.md)).

## 6. z32 decode errors (contract only)

The supplied z32 decoder reports a bad character as the error above. The other error kind, "seed has the wrong length", prints exactly `seed must be 'crypto_sign_SEEDBYTES' bytes`; it is produced by callers outside these crates. Error values can be copied and compared and print as the messages above.

## 7. Fingerprint (contract only)

A secret-masking function is supplied as a ready-made file: it keeps the first 8 characters and replaces every later character with `*`, preserving the length. Callers use it in logs. Nothing to reimplement.

## 8. The logger

A logger object writes lines to two output streams. The streams are supplied by the owner as shared, lock-protected writers (so that tests can substitute memory buffers for the console). The logger can be copied cheaply: all copies share the same streams and the same level.

### Level

The level is optional and may be fractional.

| Level value | Effect |
|---|---|
| absent (off) | nothing is ever printed |
| number | a message of severity s (DEBUG 0, INFO 1, WARN 2, ERROR 3) is printed unless s is strictly less than the level |

Resulting table (printed severities):

| Level | DEBUG | INFO | WARN | ERROR |
|---|---|---|---|---|
| off | no | no | no | no |
| 0 | yes | yes | yes | yes |
| 1 | no | yes | yes | yes |
| 2 | no | no | yes | yes |
| 3 | no | no | no | yes |
| 4 | no | no | no | no |
| 0.5 | no | yes | yes | yes |
| 2.5 | no | no | no | yes |
| 3.5 | no | no | no | no |
| -1 | yes | yes | yes | yes |
| negative infinity | yes | yes | yes | yes |
| positive infinity | no | no | no | no |
| NaN (not a number) | yes | yes | yes | yes |

NaN is printed everywhere because the comparison "less than" is false for it: the rule is "skip if s < level", not "print if s >= level". Implement it that way.

### Line format

Each message becomes one line (text, then a single line feed), written in one piece:

`ESC[90m<time>ESC[0m ESC[34m[Holesail]ESC[0m ESC[<c>m[<NAME>]ESC[0m <message>LF`

where ESC is the byte 0x1b, a single space separates the four parts, and:

| Severity | NAME | c (color code) | Stream |
|---|---|---|---|
| DEBUG | DEBUG | 90 (grey) | standard output |
| INFO | INFO | 32 (green) | standard output |
| WARN | WARN | 33 (yellow) | standard error |
| ERROR | ERROR | 31 (red) | standard error |

Time is the current UTC time as an ISO 8601 text with milliseconds and the letter Z, exactly 24 characters: `2026-01-02T03:04:05.678Z` (four-digit year, two-digit fields, three-digit milliseconds, truncated and not rounded). If the system clock is before 1970 the time is treated as 1970-01-01T00:00:00.000Z. It is read once per message.

The message is written verbatim: no trimming, no escaping, no placeholders are expanded (`%s` and `{}` stay), embedded line feeds and non-ASCII text are kept, an empty message still gives the separating space before the final line feed. Example line (shown without escapes, ESC bytes omitted): `2026-01-02T03:04:05.678Z [Holesail] [INFO] hello`.

### Streams and failures

- Each message is written with a single write to its stream while holding that stream's lock, so lines from different threads never interleave inside a line; the order inside one stream equals the call order of each thread.
- A write error is ignored (nothing is returned, nothing panics).
- If a stream's lock is poisoned by a panic in another thread, the logger takes the lock anyway and writes.
- When the level suppresses a message nothing is written to either stream, and the clock is not needed.

### Who passes what

The CLI gives the console streams (wrapped for sharing) and the level; tests give memory buffers. The level that arrives from the command line is described in [cli.md](cli.md) section 7 (a flag without value gives 1; text is parsed as a base-10 integer, `NaN` becomes 1, the result is clamped to 0..3; a number is clamped to 0..3 and may stay fractional; `--no-log` and an unset flag give "off").

## 9. The fatal-error channel

Server and client tasks that run in the background cannot stop the process themselves; they report the failure through a channel, and the top-level run function reads it, prints the text to standard error and ends the process with exit code 1 (see [cli.md](cli.md) section 9).

- Creating the channel returns a pair: a sender object and a receiver. The channel is unbounded: reporting never blocks and never fails visibly.
- The sender reports a text (anything convertible to an owned text, so both borrowed and owned strings work). The receiver gets exactly that text, byte for byte: empty text, multi-line text, non-ASCII text and text with escape sequences are delivered unchanged; nothing is appended or trimmed. By convention the reporter passes the first line of the error description.
- Several reports arrive in the order of sending (per sender, and in the global order of the calls for one thread).
- The sender can be copied; all copies feed the same receiver, from any number of threads, with no loss.
- Reporting after the receiver is gone is not an error and not a panic: the text is dropped silently.
- When every sender (original and copies) is dropped, the receiver sees "closed" after it has read the remaining texts; while at least one sender is alive it sees "empty", not "closed".
- Nothing is printed by this crate.
