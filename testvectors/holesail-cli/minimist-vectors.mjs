// Reference argv parsing: minimist 1.2.8 without options, as in holesail (minimist(process.argv.slice(2))). Writes minimist-vectors.json
import { createRequire } from 'node:module'
import { writeFileSync } from 'node:fs'
const require = createRequire(new URL('../../.tmp/npm-holesail/node_modules/holesail/', import.meta.url))
const minimist = require('minimist')
const list = [
  [], ['x'], ['--live', '3000'], ['--live=3000'], ['--live', '03000'], ['--live', '0x10'], ['--live', '1e3'], ['--live', '-5'], ['--live', '3000.5'],
  ['--live', '+5'], ['--live', ' 5'], ['--live', '5 '], ['--live', ''], ['--live='], ['--live', 'Infinity'], ['--live', 'NaN'], ['--live', '0b11'], ['--live', '1_000'],
  ['--live', '0'], ['--live', '00'], ['--live', '-0'], ['--live', '.5'], ['--live', '5.'], ['--live', '1e400'],
  ['--live', '99999999999999999999'],
  ['--live'], ['--live', '--udp'], ['--live', '-x'], ['--live', '-'], ['--live', '--'], ['--live', '1', '--live', '2'], ['--live', '1', '--live=abc'],
  ['--public'], ['--public=false'], ['--public', 'false'], ['--public', 'true'], ['--no-public'], ['--no-public=1'], ['--public=true'],
  ['--udp', 'x'], ['--udp=1'], ['--udp=0'],
  ['-h'], ['-h', 'live'], ['-hv'], ['-v'], ['-vh'], ['-p', '80'], ['-p80'], ['-abc'], ['-abc', 'x'], ['-a=b'], ['-a', '-b'], ['-1'], ['-12'], ['-1a'],
  ['--a=b=c'], ['--a='], ['--=x'], ['--', 'x', '--y'], ['x', '--', '--y', 'z'], ['-'], ['--', '--live', '3000'], ['a', 'b', 'c'],
  ['--key', 'hs://s000abc'], ['--key=hs://0000abc'], ['--key', '123456'], ['--key', '0123456789012345678901234567890123456789'],
  ['--key', '12345678901234567890123456789012'], ['--key', '1234567890123456789012345678901234567890123456789012345678901234'],
  ['--connect', '1234'], ['--connect', 'a', 'b'], ['a', '--connect', 'b'], ['1234'], ['00443214'],
  ['--x.y=1'], ['--x.y.z', '1'], ['--__proto__.polluted=1'], ['--constructor.prototype.p=1'], ['--__proto__', 'x'],
  ['--port', '8989', '--port', '9000'], ['--port=8989', '--host', '0.0.0.0'], ['--port', '-1'], ['--port', '--connect', 'k'],
  ['--log'], ['--log', '2'], ['--log=2'], ['--log', 'abc'], ['--log', '-1'], ['--log', '0'], ['--log', '3.7'], ['--log', '9'],
  ['--filemanager', '.'], ['--filemanager', '/tmp/a b'], ['--filemanager=.'], ['--filemanager'], ['--role', 'admin', '--username', 'bob', '--password', '123'],
  ['--password', '0123'], ['--password', '1e3'], ['--username', 'true'], ['--username', '007'],
  ['--force'], ['--lookup', 'abc'], ['--lookup'], ['--help', 'live'], ['--help'], ['--help', '3000'], ['--version'],
  ['--Live', '3000'], ['--LIVE', '3000'], ['--live-x', '1'], ['--live_x', '1'], ['—live', '3000'], ['--live', '３０００'],
  // All 12 Object.prototype names in three flag forms (`--name x`, `--no-name`, `--name=1`), except `--__proto__ x` (already above): 35 vectors at the end of the list.
  ...['constructor', '__defineGetter__', '__defineSetter__', 'hasOwnProperty', '__lookupGetter__', '__lookupSetter__', 'isPrototypeOf', 'propertyIsEnumerable', 'toString', 'valueOf', '__proto__', 'toLocaleString']
    .flatMap((n) => [[`--${n}`, 'x'], [`--no-${n}`], [`--${n}=1`]])
    .filter(([a, b]) => !(a === '--__proto__' && b === 'x'))
]
const out = list.map((argv) => { try { return { argv, parsed: minimist(argv) } } catch (e) { return { argv, throws: e.message } } })
writeFileSync(new URL('./minimist-vectors.json', import.meta.url), JSON.stringify(out, (k, v) => (typeof v === 'number' && !Number.isFinite(v) ? `__${String(v)}__` : v), 0).replace(/\},\{"argv"/g, '},\n{"argv"'))
for (const { argv, parsed, throws } of out) console.log(JSON.stringify(argv), "=>", throws ? "THROWS " + throws : JSON.stringify(parsed, (k, v) => (typeof v === 'number' && !Number.isFinite(v) ? `__${String(v)}__` : v)))
