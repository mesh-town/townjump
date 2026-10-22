// JS: qrcode-terminal 0.12.0, vendor/QRCode/index.js (make, createData, createBytes), QR8bitByte, QRPolynomial, QRMath.
// See also: matrix.rs lays the resulting bytes out over modules; block tables — tables.rs.
use super::tables::RS_BLOCKS_L;
use std::sync::OnceLock;

/// Error-correction block: total bytes and data bytes.
struct Block {
    total: usize,
    data: usize,
}

/// Byte mode `QR8bitByte`: the length is the number of UTF-16 units, each written with its low 8 bits.
fn payload(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .map(|unit| (unit & 0xff) as u8)
        .collect()
}

fn blocks(version: usize) -> Vec<Block> {
    RS_BLOCKS_L[version - 1]
        .chunks(3)
        .flat_map(|g| {
            (0..g[0]).map(|_| Block {
                total: g[1],
                data: g[2],
            })
        })
        .collect()
}

/// The smallest version (the JS loop goes up to 39, then takes 40) that fits the string.
pub fn pick_version(text: &str) -> usize {
    let len_bits = text.encode_utf16().count() * 8;
    let fits = |v: usize| {
        let capacity: usize = blocks(v).iter().map(|b| b.data).sum();
        4 + length_field_bits(v) + len_bits <= capacity * 8
    };
    (1..40).find(|&v| fits(v)).unwrap_or(40)
}

fn length_field_bits(version: usize) -> usize {
    if version < 10 { 8 } else { 16 }
}

/// Version codewords: data + Reed-Solomon bytes, blocks interleaved.
pub fn codewords(text: &str, version: usize) -> Vec<u8> {
    let blocks = blocks(version);
    let capacity: usize = blocks.iter().map(|b| b.data).sum();
    let data = pad_data(text, version, capacity);
    let mut offset = 0;
    let mut parts = Vec::new();
    for block in &blocks {
        let chunk = data[offset..offset + block.data].to_vec();
        offset += block.data;
        let ec = remainder(&chunk, block.total - block.data);
        parts.push((chunk, ec));
    }
    interleave(&parts)
}

/// Data with the mode header, length, terminator and `0xEC 0x11` padding up to capacity.
fn pad_data(text: &str, version: usize, capacity: usize) -> Vec<u8> {
    let bytes = payload(text);
    let mut bits: Vec<bool> = Vec::new();
    put(&mut bits, 4, 4);
    put(&mut bits, bytes.len(), length_field_bits(version));
    bytes.iter().for_each(|&b| put(&mut bits, b.into(), 8));
    let limit = capacity * 8;
    assert!(bits.len() <= limit, "code length overflow");
    if bits.len() + 4 <= limit {
        put(&mut bits, 0, 4);
    }
    while !bits.len().is_multiple_of(8) {
        bits.push(false);
    }
    let mut out: Vec<u8> = bits
        .chunks(8)
        .map(|c| c.iter().fold(0, |a, &b| a << 1 | u8::from(b)))
        .collect();
    let pads = [0xec, 0x11].into_iter().cycle();
    out.extend(pads.take(capacity - out.len()));
    out
}

fn put(bits: &mut Vec<bool>, value: usize, len: usize) {
    bits.extend((0..len).map(|i| (value >> (len - 1 - i)) & 1 == 1));
}

fn interleave(parts: &[(Vec<u8>, Vec<u8>)]) -> Vec<u8> {
    let longest =
        |pick: fn(&(Vec<u8>, Vec<u8>)) -> usize| parts.iter().map(pick).max().unwrap_or(0);
    let mut out = Vec::new();
    for i in 0..longest(|p| p.0.len()) {
        out.extend(parts.iter().filter_map(|p| p.0.get(i)));
    }
    for i in 0..longest(|p| p.1.len()) {
        out.extend(parts.iter().filter_map(|p| p.1.get(i)));
    }
    out
}

/// GF(256) field tables from `QRMath`: (exp, log).
fn gf_tables() -> &'static ([u8; 256], [usize; 256]) {
    static TABLES: OnceLock<([u8; 256], [usize; 256])> = OnceLock::new();
    TABLES.get_or_init(build_tables)
}

fn build_tables() -> ([u8; 256], [usize; 256]) {
    let mut exp = [0u8; 256];
    for i in 0..256 {
        exp[i] = if i < 8 {
            1 << i
        } else {
            exp[i - 4] ^ exp[i - 5] ^ exp[i - 6] ^ exp[i - 8]
        };
    }
    let mut log = [0usize; 256];
    for (i, &e) in exp.iter().enumerate().take(255) {
        log[e as usize] = i;
    }
    (exp, log)
}

fn gmul(a: u8, b: u8) -> u8 {
    let (exp, log) = gf_tables();
    if a == 0 || b == 0 {
        return 0;
    }
    exp[(log[a as usize] + log[b as usize]) % 255]
}

/// Generator polynomial `∏ (x + α^i)`, highest coefficient first.
fn generator(ec: usize) -> Vec<u8> {
    let (exp, _) = gf_tables();
    let mut poly = vec![1u8];
    for &alpha in exp.iter().take(ec) {
        let mut next = vec![0u8; poly.len() + 1];
        for (j, &c) in poly.iter().enumerate() {
            next[j] ^= c;
            next[j + 1] ^= gmul(c, alpha);
        }
        poly = next;
    }
    poly
}

/// Remainder of dividing `data · x^ec` by the generator polynomial: `ec` correction bytes.
fn remainder(data: &[u8], ec: usize) -> Vec<u8> {
    let gen_poly = generator(ec);
    let mut rest: Vec<u8> = data
        .iter()
        .copied()
        .chain(std::iter::repeat_n(0, ec))
        .collect();
    for i in 0..data.len() {
        let lead = rest[i];
        for (j, &g) in gen_poly.iter().enumerate() {
            rest[i + j] ^= gmul(g, lead);
        }
    }
    rest[data.len()..].to_vec()
}
