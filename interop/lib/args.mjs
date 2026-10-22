// Arguments of run.mjs. A flag value is `--flag value` or `--flag=value`.
export const HELP = `Holesail compatibility harness: scenario → ok/fail on a local testnet 127.0.0.1.
Run: node interop/run.mjs [flags]
  --client js,rust    which clients (default js,rust)
  --server js,rust    which servers (default js,rust)
  --only substring   only scenarios whose id contains the substring (comma-separated — any); "only via
                      --only" scenarios (see --list) are not in the default matrix and are enabled by the exact id
  --rust-bin path    Rust binary townjump (or TOWNJUMP_BIN; otherwise target/{release,debug}/townjump)
  --strict            count "skip" (no Rust binary) as a failure
  --nodes N           testnet nodes (default 4)
  --neg-timeout sec  how long to wait for the close on a rejection (default 45)
  --settle ms         pause after the client start before the first connection (default 1500)
  --mutate open-firewall  harness self-check: the JS server admits any client, fw3-public-client must fail
  --list              show the scenarios and exit
Exit code: 0 — no fail (with --strict no skip either), 1 — some fail, 2 — startup error.`

const IMPLS = ['js', 'rust']
const list = (v) => v.split(',').map((s) => s.trim()).filter(Boolean)

export function parseArgs(argv, env = process.env) {
  const o = { client: IMPLS, server: IMPLS, only: [], rustBin: env.TOWNJUMP_BIN || null, strict: false, list: false, help: false, nodes: 4, negTimeoutMs: 45_000, settleMs: 1500, mutate: null }
  const flags = { '--strict': () => (o.strict = true), '--list': () => (o.list = true), '--help': () => (o.help = true), '-h': () => (o.help = true) }
  const values = {
    '--client': (v) => (o.client = list(v)),
    '--server': (v) => (o.server = list(v)),
    '--only': (v) => (o.only = list(v)),
    '--rust-bin': (v) => (o.rustBin = v),
    '--nodes': (v) => (o.nodes = Number(v)),
    '--neg-timeout': (v) => (o.negTimeoutMs = Number(v) * 1000),
    '--settle': (v) => (o.settleMs = Number(v)),
    '--mutate': (v) => (o.mutate = v)
  }
  for (let i = 0; i < argv.length; i++) {
    const [name, inline] = argv[i].split(/=(.*)/s)
    if (flags[name]) flags[name]()
    else if (values[name]) values[name](inline ?? argv[++i])
    else throw new Error(`unknown flag ${argv[i]} (see --help)`)
  }
  for (const side of [...o.client, ...o.server]) if (!IMPLS.includes(side)) throw new Error(`unknown side «${side}»: js or rust`)
  return o
}
