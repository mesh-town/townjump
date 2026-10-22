// Parsing of the CLI banner: the line `Connect with key: hs://s000<K>` (private) or `hs://0000<z32>` (public).
// The key is taken from the output of the real CLI, so the harness checks that too.
const URL_RE = /hs:\/\/(s000|0000)([^\s│|]+)/

export function parseBannerUrl(text) {
  const m = text.match(URL_RE)
  if (!m) return null
  return { secure: m[1] === 's000', key: m[2], url: `hs://${m[1]}${m[2]}` }
}

export const BANNER_WAIT = URL_RE
