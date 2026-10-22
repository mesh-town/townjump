// Tests of CLI banner parsing: `node --test interop/lib/`.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { parseBannerUrl } from './banner.mjs'
import { stripAnsi } from './proc.mjs'

const box = (key) => `|      Connect with key: ${key}      │`

test('private link: secure and key K', () => {
  assert.deepEqual(parseBannerUrl(box('hs://s000interop-abc123')), { secure: true, key: 'interop-abc123', url: 'hs://s000interop-abc123' })
})

test('public link: secure=false, z32 key', () => {
  const key = 'rmat7socnfxmp4xbudrxhipr9keq6pic4x91j5t3fi9tjh14xoao'
  assert.deepEqual(parseBannerUrl(box(`hs://0000${key}`)), { secure: false, key, url: `hs://0000${key}` })
})

test('ANSI colors around the link do not interfere after stripAnsi', () => {
  const colored = `Connect with key: \x1b[2mhs://s000abc\x1b[22m\x1b[0m   \x1b[0m│`
  assert.equal(parseBannerUrl(stripAnsi(colored)).key, 'abc')
})

test('no link — null', () => {
  assert.equal(parseBannerUrl('Server listening on key: pu4j'), null)
})
