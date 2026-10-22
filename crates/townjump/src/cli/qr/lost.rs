// JS: qrcode-terminal 0.12.0, vendor/QRCode/QRUtil.js:getLostPoint (own penalties, not per the ISO standard).
// See also: matrix.rs picks the mask with the lowest value.
use super::matrix::Matrix;

/// Mask penalty: same-color neighbors, 2x2 blocks, the 1:1:3:1:1 pattern along rows and columns, dark balance.
pub fn lost_point(m: &Matrix) -> f64 {
    let dark = darkness(m);
    same_neighbours(m) + blocks(m) + finders(m) + dark_penalty(m, dark)
}

fn darkness(m: &Matrix) -> usize {
    m.cells
        .iter()
        .flatten()
        .filter(|c| **c == Some(true))
        .count()
}

fn dark_penalty(m: &Matrix, dark: usize) -> f64 {
    let n = m.size as f64;
    (100.0 * dark as f64 / n / n - 50.0).abs() / 5.0 * 10.0
}

fn same_neighbours(m: &Matrix) -> f64 {
    let mut lost = 0;
    for row in 0..m.size {
        for col in 0..m.size {
            let same = neighbours(m, row, col);
            if same > 5 {
                lost += 3 + same - 5;
            }
        }
    }
    lost as f64
}

fn neighbours(m: &Matrix, row: usize, col: usize) -> usize {
    let mut same = 0;
    for r in -1..=1i32 {
        for c in -1..=1i32 {
            let (y, x) = (row as i32 + r, col as i32 + c);
            let inside = y >= 0 && x >= 0 && y < m.size as i32 && x < m.size as i32;
            if inside && (r, c) != (0, 0) && m.dark(y as usize, x as usize) == m.dark(row, col) {
                same += 1;
            }
        }
    }
    same
}

fn blocks(m: &Matrix) -> f64 {
    let mut lost = 0;
    for row in 0..m.size - 1 {
        for col in 0..m.size - 1 {
            let count = [(0, 0), (1, 0), (0, 1), (1, 1)]
                .iter()
                .filter(|(r, c)| m.dark(row + r, col + c))
                .count();
            if count == 0 || count == 4 {
                lost += 3;
            }
        }
    }
    lost as f64
}

fn finders(m: &Matrix) -> f64 {
    let mut lost = 0;
    for a in 0..m.size {
        for b in 0..m.size - 6 {
            lost += 40 * usize::from(pattern(|i| m.dark(a, b + i)));
            lost += 40 * usize::from(pattern(|i| m.dark(b + i, a)));
        }
    }
    lost as f64
}

fn pattern(at: impl Fn(usize) -> bool) -> bool {
    [true, false, true, true, true, false, true]
        .iter()
        .enumerate()
        .all(|(i, &want)| at(i) == want)
}
