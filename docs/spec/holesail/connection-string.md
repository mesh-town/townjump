# Connection string and key derivation

Sources: `holesail/src/index.js:Holesail.urlParser`, `holesail/src/index.js:Holesail` (constructor, `#initialise`, `info`), `holesail-server/index.js:HolesailServer.generateKeyPair`, `holesail-server/index.js:HolesailServer.key`, `holesail-client/index.js:HolesailClient` (constructor), `hyper-cmd-lib-keys/index.js:randomBytes`.

## Two forms

| Mode | String | What is in the tail |
|---|---|---|
| private | `hs://s000<K>` | `K` is the secret string: 64 hex characters (32 random bytes, if the server generated it) or any text given by `--key` |
| public | `hs://0000<Z>` | `Z` is the z-base-32 (alphabet `ybndrfg8ejkmcpqxot1uwisza345h769`, unpadded, 52 characters) of the server's public key |

A bare key without `hs://…` is also accepted by the client and `--lookup`; the mode is then determined by the rules below (quirk 1, quirk 2).

## `urlParser(url)` — the exact algorithm

```
url = String(url || '')                       // a number is stringified too (quirk 5)
if url is non-empty AND url[0..5) == 'hs://' AND length of url[5..9) == 4:
    key = url[9..]                            // the four characters after hs:// are NOT checked, any are discarded
else:
    key = url                                 // a bare key
if url is non-empty AND url[5..6) == 's':
    secure = true                             // (quirk 2) the check is NOT tied to the prefix
else:
    secure = unset (undefined)                // this parsing never returns false
return { key, secure }
```

Indexes are in UTF-16 code units (`String.substring`). Consequences:

- `hs://s000K` and `hs://sXYZK` are private; `hs://0000K`, `hs://x000K` — the flag is unset. `hs://0000…` does NOT force public mode: the `--public` flag decides (quirk 10 on the server, quirk 1 on the client).
- A string `hs://` shorter than 9 characters (for example `hs://s00`) is not considered a prefix: the whole string becomes the key.
- A bare key whose 6th character (index 5) is `s` is marked private, and the key remains the WHOLE string (quirk 2). For a random public z32 key the chance is about 1/32.

## The resulting mode

`Holesail.constructor`: `secure = data.secure ?? opts.secure` (that is, `true` from the URL wins, otherwise the flag value).

| Who | `opts.secure` |
|---|---|
| server (`--live`, `--filemanager`) | `argv.public !== undefined ? !argv.public : true`. The string `'false'` (`--public=false`) is truthy, so `!'false'` = `false` — public (quirk 6); `--no-public` gives `false` → private |
| client (`--connect`, positional) | `argv.public` as is, i.e. inverted (quirk 1): without the flag `undefined` → public; `--public` → `true` → private |

Then `HolesailClient.secure = opts.secure || false`. For the banner the `secure` value is taken as is (the string `'false'` is truthy → "Private").

## Deriving the seed and key pair

`sha256` is SHA-256 over the UTF-8 bytes of the string `K`; for non-strings `.toString()` is applied (a number from minimist turns into its JS decimal notation, quirk 5).

### Server (`holesail/src/index.js:Holesail.#initialise`, `holesail-server/index.js:HolesailServer.generateKeyPair`)

1. There is a `key` (from `--key` after `urlParser`): `seed = hex(sha256(key))` — 64 hex characters, in any mode.
2. Otherwise `secure`: `key` = 32 random bytes in hex (64 characters), `seed = hex(sha256(key))`.
3. Otherwise (public without `--key`): there is no seed; `generateKeyPair` takes 32 random bytes in hex and uses them AS IS (without hashing) as the seed.
4. `keyPair = HyperDHT.keyPair(Buffer.from(seedHex, 'hex'))` — an Ed25519 pair from a 32-byte seed (the function belongs to the hyperdht layer: `crypto_sign_seed_keypair`).

The key for the client and for printing:

| Mode | Displayed key | URL |
|---|---|---|
| private | `Holesail.info` takes `this.key` — `K` ITSELF (not a hash) | `hs://s000` + `K` |
| public | `z32(keyPair.publicKey)` (`HolesailServer.key`) | `hs://0000` + `z32(publicKey)` |

Public mode with `--key` also gives `seed = sha256(key)`, but the displayed key is `z32(publicKey)`, and the `--key` itself disappears from the URL.

### Client (`holesail/src/index.js:Holesail.#initialise`, `holesail-client/index.js:HolesailClient`)

- Private: `seedZ32 = z32(sha256(K))`; `HolesailClient`: `keyPair = HyperDHT.keyPair(z32.decode(seedZ32))` (that is, the same pair as the server's), `publicKey = keyPair.publicKey`. The client's DHT node is created as `new HyperDHT({ keyPair })`, so the client presents the same public identity to the server, and the server's firewall predicate (key equality) lets it through.
- Public: `publicKey = z32.decode(key)`; `keyPair` is unset, the node `new HyperDHT({ keyPair: undefined })` creates a random one-time pair (`hyperdht/index.js:HyperDHT.defaultKeyPair`, substituted in `hyperdht/lib/connect.js` as `opts.keyPair || dht.defaultKeyPair`). A bare key that is not valid z32 is an exception from `z32.decode` (quirk 1, quirk 12).
- A client key outside the z32 alphabet: `new Holesail(...)` does not throw (only `Key is empty` for an empty key); `z32.decode` is called by `new HolesailClient` in `_open`, so `ready()` rejects with `Invalid character in base32 input: "!" at position <index>` before the DHT node is created.
- In both modes the client reads the record by `publicKey` (see [dht-records.md](dht-records.md)) and connects to `publicKey`.

## Vectors (the full set is `testvectors/holesail-cli/keyderivation-vectors.json`)

| `K` (private) | `sha256` = seed (hex) | public key (hex) | `z32(seed)` (what `HolesailClient` receives) |
|---|---|---|---|
| `abc` | `ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad` | `0bdf51005e5a525cc9bfb59f18070af6a8a6013106530eb8944554a5ae4c3cfa` | `zjhbpxhxy886…` |
| `abcdefghijklmnopqrstuvwxyz012345` | `653bb1245e828fcd…` | `467fc0ce9e366f91…` | `cw75njn6ok8h…` |
| `000102…1f` (64 hex) | `6c86c6aac5fb24bcf5d9939cb7d7d5645ce39418f449e03b262dd4fa14b4b92b` | `cd38df7e4940e276ebf8d3d2ceedb583f46a2beb2c66f131aba5ad35406ff4b5` | `p1dcpksf9c1m…` |
| `κλωξ-κλωξ-κλωξ-κλωξ-κλωξ-κλωξ-κλωξ` | `72abee9f2010664e…` | `9f37004c52380ead…` | `qki6783ynbur…` |

Public without `--key`, random bytes `000102…1f`: the seed = these bytes as is, public key `03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8`, key `yqooxx9u3aemh8mo5wcqq16yufu6jitouq1o4za751dger1igghy`, URL `hs://0000yqooxx9u3aemh8mo5wcqq16yufu6jitouq1o4za751dger1igghy`.

Number-like values (quirk 5): `--key 123456` → key `"123456"`; `--key 0123456789012345678901234567890123456789` → key `"1.2345678901234568e+38"` (the "≥32" length check is bypassed: a number has no `length`). The value `--connect 00443214` becomes `443214`.

## Key length check (`validateInput`)

`--key` shorter than 32 characters without `--force` is rejected with code 2 (see [cli.md](cli.md)). The length is `args.key.length` of the string as parsed by minimist, before `urlParser`, so the prefix `hs://s000` counts (`--key hs://s000abc` requires `--force`, while `hs://s000` plus 23 characters of key — 32 in total — passes without it, although the key itself is shorter than 32).
