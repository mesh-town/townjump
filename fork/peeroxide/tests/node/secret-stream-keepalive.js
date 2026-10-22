'use strict';
// Probe: what @hyperswarm/secret-stream puts on the raw stream when idle (keepAlive) and when
// the receive timeout fires. Prints one JSON document; timings are real (about 15 s).
// Run: node secret-stream-keepalive.js   (needs `npm install` in this directory)
const { Duplex } = require('streamx');
const SecretStream = require('@hyperswarm/secret-stream');

const K1 = Buffer.alloc(32, 0x11);
const K2 = Buffer.alloc(32, 0x22);
const HASH = Buffer.alloc(32, 0x33);
const PK_A = Buffer.alloc(32, 0xaa);
const PK_B = Buffer.alloc(32, 0xbb);

function rawPair() {
  let a, b;
  a = new Duplex({ write(d, cb) { b.push(d); cb(null); } , final(cb) { b.push(null); cb(); } });
  b = new Duplex({ write(d, cb) { a.push(d); cb(null); } , final(cb) { a.push(null); cb(); } });
  return [a, b];
}

function pair(optsA, optsB) {
  const [ra, rb] = rawPair();
  const sent = { a: [], b: [] };
  const t0 = Date.now();
  for (const [name, r] of [['a', ra], ['b', rb]]) {
    const w = r.write.bind(r);
    r.write = (d) => { sent[name].push({ t: Date.now() - t0, hex: Buffer.from(d).toString('hex') }); return w(d); };
  }
  const a = new SecretStream(true, ra, { ...optsA, handshake: { tx: K1, rx: K2, hash: HASH, publicKey: PK_A, remotePublicKey: PK_B } });
  const b = new SecretStream(false, rb, { ...optsB, handshake: { tx: K2, rx: K1, hash: HASH, publicKey: PK_B, remotePublicKey: PK_A } });
  a.on('error', () => {}); b.on('error', () => {});
  return { a, b, sent, t0 };
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function keepaliveScenario() {
  // keepAlive 5000 on both; a writes 'x' at 3 s; observe until 14 s
  const p = pair({ keepAlive: 5000 }, { keepAlive: 5000 });
  const got = [];
  p.b.on('data', (d) => got.push({ t: Date.now() - p.t0, text: d.toString() }));
  await sleep(3000);
  p.a.write(Buffer.from('x'));
  await sleep(11000);
  const out = { sentA: p.sent.a, sentB: p.sent.b, receivedByB: got, keepAlive: p.a.keepAlive };
  p.a.destroy(); p.b.destroy();
  return out;
}

async function timeoutScenario() {
  // a: setTimeout(2000), b: silent then writes at 1 s -> a must die at ~3 s
  const p = pair({}, {});
  let diedAt = null; let err = null;
  p.a.on('close', () => { diedAt = Date.now() - p.t0; });
  p.a.on('error', (e) => { err = e.message; });
  p.a.on('data', () => {});
  p.a.setTimeout(2000);
  await sleep(1000);
  p.b.write(Buffer.from('y'));
  await sleep(3000);
  const out = { diedAt, err, timeout: p.a.timeout };
  p.a.destroy(); p.b.destroy();
  return out;
}

(async () => {
  const keepalive = await keepaliveScenario();
  const timeout = await timeoutScenario();
  console.log(JSON.stringify({ keys: { k1: K1.toString('hex'), k2: K2.toString('hex'), hash: HASH.toString('hex') }, keepalive, timeout }, null, 1));
  process.exit(0);
})();
