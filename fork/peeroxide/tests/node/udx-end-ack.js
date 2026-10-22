'use strict';
// Probe for delta D8 (libudx send_ack after our END is acknowledged). A JS udx-native stream talks to a
// raw UDP peer; prints every datagram the JS stream sends, per scenario, as JSON.
//  E1: JS ends first; peer acks the END (no END from peer)      -> JS answers with a final ACK
//  E3: E1 plus a second identical ACK of the END             -> no further datagram
//  E2: peer ENDs first (JS acks); JS ends; peer acks JS's END    -> stream closes, no more datagrams
// Run: node udx-end-ack.js   (needs `npm install` in this directory)
const dgram = require('dgram');
const UDX = require('udx-native');

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function header(type, remoteId, seq, ack, win = 0x400000) {
  const h = Buffer.alloc(20);
  h[0] = 0xff; h[1] = 1; h[2] = type; h[3] = 0;
  h.writeUInt32LE(remoteId, 4); h.writeUInt32LE(win, 8); h.writeUInt32LE(seq, 12); h.writeUInt32LE(ack, 16);
  return h;
}

async function scenario(name, body) {
  const raw = dgram.createSocket('udp4');
  const got = [];
  let t0 = Date.now();
  raw.on('message', (m) => got.push({ t: Date.now() - t0, hex: m.toString('hex') }));
  await new Promise((r) => raw.bind(0, '127.0.0.1', r));
  const u = new UDX();
  const sock = u.createSocket(); sock.bind(0, '127.0.0.1');
  const s = u.createStream(5);
  s.on('error', () => {}); s.on('data', () => {});
  let closedAt = null;
  s.on('close', () => { closedAt = Date.now() - t0; });
  s.connect(sock, 7, raw.address().port, '127.0.0.1');
  const send = (h) => raw.send(h, sock.address().port, '127.0.0.1');
  t0 = Date.now();
  await body({ s, send, got, mark: () => got.length });
  await sleep(600);
  const out = { name, datagrams: got, closedAt };
  s.destroy(); sock.close(); raw.close(); await sleep(50);
  return out;
}

(async () => {
  const e1 = await scenario('E1_js_ends_first', async ({ s, send }) => {
    s.end();
    await sleep(200);
    send(header(0x00, 5, 0, 1)); // pure ACK covering JS's END (seq 0 -> ack 1)
  });
  const e2 = await scenario('E2_peer_ends_first', async ({ s, send }) => {
    send(header(0x02, 5, 0, 0)); // peer END, seq 0
    await sleep(200);
    s.end();
    await sleep(200);
    send(header(0x00, 5, 1, 1)); // peer ack of JS END (peer seq now 1, ack 1)
  });
  const e3 = await scenario('E3_duplicate_ack_of_end', async ({ s, send }) => {
    s.end();
    await sleep(200);
    send(header(0x00, 5, 0, 1));
    await sleep(200);
    send(header(0x00, 5, 0, 1)); // duplicate
  });
  console.log(JSON.stringify({ e1, e2, e3 }, null, 1));
  process.exit(0);
})();
