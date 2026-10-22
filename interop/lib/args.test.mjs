// Tests of run.mjs argument parsing: `node --test interop/lib/`.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { parseArgs } from './args.mjs'

test('defaults: both sides, both clients, Rust not set', () => {
  const o = parseArgs([], {})
  assert.deepEqual(o.client, ['js', 'rust'])
  assert.deepEqual(o.server, ['js', 'rust'])
  assert.equal(o.rustBin, null)
  assert.equal(o.settleMs, 1500)
})

test('flag with a value: separated by space and by =', () => {
  const o = parseArgs(['--client', 'js', '--server=rust', '--only', 'udp,fw', '--neg-timeout=10'], {})
  assert.deepEqual(o.client, ['js'])
  assert.deepEqual(o.server, ['rust'])
  assert.deepEqual(o.only, ['udp', 'fw'])
  assert.equal(o.negTimeoutMs, 10_000)
})

test('TOWNJUMP_BIN is picked up, --rust-bin overrides it', () => {
  assert.equal(parseArgs([], { TOWNJUMP_BIN: '/a' }).rustBin, '/a')
  assert.equal(parseArgs(['--rust-bin', '/b'], { TOWNJUMP_BIN: '/a' }).rustBin, '/b')
})

test('unknown flag and unknown side — error', () => {
  assert.throws(() => parseArgs(['--nope'], {}), /unknown flag/)
  assert.throws(() => parseArgs(['--client', 'go'], {}), /unknown side/)
})
