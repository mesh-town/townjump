// JS: qrcode-terminal 0.12.0, vendor/QRCode/index.js (makeImpl, setup*, mapData, getBestMaskPattern) and QRUtil.js (masks, BCH).
// See also: lost.rs computes the mask penalties; encode.rs provides the codewords.
use super::lost::lost_point;
use super::tables::PATTERN_POSITIONS;

/// Square of modules: `None` — not yet occupied (room for data).
pub struct Matrix {
    pub size: usize,
    pub cells: Vec<Vec<Option<bool>>>,
}

/// Level L in the format field: `errorCorrectLevel = 1`.
const LEVEL_L: usize = 1;

/// Final matrix: the mask with the lowest penalty (the first on a tie), like `getBestMaskPattern`.
pub fn build(version: usize, data: &[u8]) -> Matrix {
    let mut best = (0, f64::MAX);
    for mask in 0..8 {
        let lost = lost_point(&make(version, data, true, mask));
        if lost < best.1 {
            best = (mask, lost);
        }
    }
    make(version, data, false, best.0)
}

fn make(version: usize, data: &[u8], test: bool, mask: usize) -> Matrix {
    let size = version * 4 + 17;
    let mut m = Matrix {
        size,
        cells: vec![vec![None; size]; size],
    };
    m.probe(0, 0);
    m.probe(size - 7, 0);
    m.probe(0, size - 7);
    m.adjust(version);
    m.timing();
    m.type_info(test, mask);
    if version >= 7 {
        m.version_info(test, version);
    }
    m.map_data(data, mask);
    m
}

impl Matrix {
    fn set(&mut self, row: usize, col: usize, dark: bool) {
        self.cells[row][col] = Some(dark);
    }

    pub fn dark(&self, row: usize, col: usize) -> bool {
        self.cells[row][col] == Some(true)
    }

    fn probe(&mut self, row: usize, col: usize) {
        for r in -1..=7i32 {
            for c in -1..=7i32 {
                let (y, x) = (row as i32 + r, col as i32 + c);
                if y < 0 || x < 0 || y >= self.size as i32 || x >= self.size as i32 {
                    continue;
                }
                let ring = (0..=6).contains(&r) && (c == 0 || c == 6);
                let ring = ring || ((0..=6).contains(&c) && (r == 0 || r == 6));
                let core = (2..=4).contains(&r) && (2..=4).contains(&c);
                self.set(y as usize, x as usize, ring || core);
            }
        }
    }

    fn adjust(&mut self, version: usize) {
        let pos = PATTERN_POSITIONS[version - 1];
        for &row in pos {
            for &col in pos {
                if self.cells[row][col].is_none() {
                    self.align_at(row, col);
                }
            }
        }
    }

    fn align_at(&mut self, row: usize, col: usize) {
        for r in -2..=2i32 {
            for c in -2..=2i32 {
                let dark = r.abs() == 2 || c.abs() == 2 || (r == 0 && c == 0);
                self.set((row as i32 + r) as usize, (col as i32 + c) as usize, dark);
            }
        }
    }

    fn timing(&mut self) {
        for i in 8..self.size - 8 {
            if self.cells[i][6].is_none() {
                self.set(i, 6, i.is_multiple_of(2));
            }
            if self.cells[6][i].is_none() {
                self.set(6, i, i.is_multiple_of(2));
            }
        }
    }

    fn version_info(&mut self, test: bool, version: usize) {
        let bits = bch_version(version);
        for i in 0..18 {
            let dark = !test && (bits >> i) & 1 == 1;
            self.set(i / 3, i % 3 + self.size - 11, dark);
            self.set(i % 3 + self.size - 11, i / 3, dark);
        }
    }

    fn type_info(&mut self, test: bool, mask: usize) {
        let bits = bch_type((LEVEL_L << 3) | mask);
        let n = self.size;
        for i in 0..15 {
            let dark = !test && (bits >> i) & 1 == 1;
            let row = if i < 6 {
                i
            } else if i < 8 {
                i + 1
            } else {
                n - 15 + i
            };
            self.set(row, 8, dark);
            let col = if i < 8 {
                n - i - 1
            } else if i < 9 {
                15 - i
            } else {
                14 - i
            };
            self.set(8, col, dark);
        }
        self.set(n - 8, 8, !test);
    }

    fn map_data(&mut self, data: &[u8], mask: usize) {
        let (mut up, mut row, mut bit) = (true, self.size - 1, 0usize);
        let mut col = self.size - 1;
        while col > 0 {
            if col == 6 {
                col -= 1;
            }
            loop {
                for c in [col, col - 1] {
                    if self.cells[row][c].is_none() {
                        let raw = data
                            .get(bit / 8)
                            .is_some_and(|b| (b >> (7 - bit % 8)) & 1 == 1);
                        self.set(row, c, raw != mask_at(mask, row, c));
                        bit += 1;
                    }
                }
                match step(row, up, self.size) {
                    Some(next) => row = next,
                    None => break,
                }
            }
            up = !up;
            col = col.saturating_sub(2);
        }
    }
}

/// Next row of the walk; `None` — hit the edge, the direction changes.
fn step(row: usize, up: bool, size: usize) -> Option<usize> {
    if up {
        return row.checked_sub(1);
    }
    Some(row + 1).filter(|&r| r < size)
}

fn mask_at(mask: usize, i: usize, j: usize) -> bool {
    match mask {
        0 => (i + j).is_multiple_of(2),
        1 => i.is_multiple_of(2),
        2 => j.is_multiple_of(3),
        3 => (i + j).is_multiple_of(3),
        4 => (i / 2 + j / 3).is_multiple_of(2),
        5 => (i * j) % 2 + (i * j) % 3 == 0,
        6 => ((i * j) % 2 + (i * j) % 3).is_multiple_of(2),
        _ => ((i * j) % 3 + (i + j) % 2).is_multiple_of(2),
    }
}

fn digit(mut n: usize) -> usize {
    let mut d = 0;
    while n != 0 {
        d += 1;
        n >>= 1;
    }
    d
}

fn bch(data: usize, shift: usize, poly: usize) -> usize {
    let mut d = data << shift;
    while digit(d) >= digit(poly) {
        d ^= poly << (digit(d) - digit(poly));
    }
    (data << shift) | d
}

fn bch_type(data: usize) -> usize {
    bch(data, 10, 0b101_0011_0111) ^ 0b101_0100_0001_0010
}

fn bch_version(version: usize) -> usize {
    bch(version, 12, 0b1_1111_0010_0101)
}
