//! Scalar staged integer inverse transforms from AV1 section 7.13.2.
//! No floating point, approximate cosine matrix, or substitute transform.
use super::transform::TxType1D;
use codec_core::{CodecError, CodecResult};

const COS: [i64; 65] = [
    4096, 4095, 4091, 4085, 4076, 4065, 4052, 4036, 4017, 3996, 3973, 3948, 3920, 3889, 3857, 3822,
    3784, 3745, 3703, 3659, 3612, 3564, 3513, 3461, 3406, 3349, 3290, 3229, 3166, 3102, 3035, 2967,
    2896, 2824, 2751, 2675, 2598, 2520, 2440, 2359, 2276, 2191, 2106, 2019, 1931, 1842, 1751, 1660,
    1567, 1474, 1380, 1285, 1189, 1092, 995, 897, 799, 700, 601, 501, 401, 301, 201, 101, 0,
];
fn cos(angle: i32) -> i64 {
    let a = (angle & 255) as usize;
    match a {
        0..=64 => COS[a],
        65..=128 => -COS[128 - a],
        129..=192 => -COS[a - 128],
        _ => COS[256 - a],
    }
}
fn fits(v: i64, r: u8) -> bool {
    let limit = 1i64 << (r - 1);
    (-limit..limit).contains(&v)
}
fn check(v: i64, r: u8) -> CodecResult<i64> {
    if fits(v, r) {
        Ok(v)
    } else {
        Err(CodecError::InvalidData)
    }
}
fn round(v: i64, bits: u8) -> i64 {
    if bits == 0 {
        v
    } else {
        (v + (1 << (bits - 1))) >> bits
    }
}
fn brev(n: u32, i: usize) -> usize {
    i.reverse_bits() >> (usize::BITS - n)
}
fn b(t: &mut [i64], a: usize, b: usize, angle: i32, flip: bool, r: u8) -> CodecResult<()> {
    let x = t[a] * cos(angle) - t[b] * cos(angle - 64);
    let y = t[a] * cos(angle - 64) + t[b] * cos(angle);
    let (x, y) = (check(round(x, 12), r)?, check(round(y, 12), r)?);
    (t[a], t[b]) = if flip { (y, x) } else { (x, y) };
    Ok(())
}
fn h(t: &mut [i64], a: usize, b: usize, flip: bool, r: u8) {
    let (a, b) = if flip { (b, a) } else { (a, b) };
    let limit = 1i64 << (r - 1);
    let (x, y) = (t[a], t[b]);
    t[a] = (x + y).clamp(-limit, limit - 1);
    t[b] = (x - y).clamp(-limit, limit - 1);
}

/// Transform one row/column, without the 2D row/column shifts or flip placement.
/// FlipADST uses the ADST kernel; the caller reverses spatial samples in the 2D
/// reconstruction step. Supported lengths: DCT 4..64, ADST 4..16, identity 4..32.
/// Range bits are the normative row/column clamp range (16..20 for 8/10/12-bit).
pub fn inverse_1d(input: &[i32], kind: TxType1D, range_bits: u8) -> CodecResult<Vec<i32>> {
    let len = input.len();
    if !matches!(range_bits, 16..=20) || !matches!(len, 4 | 8 | 16 | 32 | 64) {
        return Err(CodecError::InvalidData);
    }
    if matches!(kind, TxType1D::Adst | TxType1D::FlipAdst) && len > 16
        || kind == TxType1D::Idtx && len > 32
    {
        return Err(CodecError::Unsupported);
    }
    let mut storage = [0i64; 64];
    let t = &mut storage[..len];
    for (dst, &v) in t.iter_mut().zip(input) {
        *dst = check(i64::from(v), range_bits)?;
    }
    match kind {
        TxType1D::Dct => dct(t, range_bits)?,
        TxType1D::Adst | TxType1D::FlipAdst => adst(t, range_bits)?,
        TxType1D::Idtx => {
            for v in t.iter_mut() {
                *v = match len {
                    4 => round(*v * 5793, 12),
                    8 => *v * 2,
                    16 => round(*v * 11586, 12),
                    32 => *v * 4,
                    _ => unreachable!(),
                };
            }
        }
    }
    Ok(t.iter().map(|&v| v as i32).collect())
}

fn dct(t: &mut [i64], r: u8) -> CodecResult<()> {
    let n = t.len().ilog2();
    let copy = t.to_vec();
    for (i, v) in t.iter_mut().enumerate() {
        *v = copy[brev(n, i)];
    }
    if n == 6 {
        for i in 0..16 {
            b(t, 32 + i, 63 - i, 63 - 4 * brev(4, i) as i32, false, r)?;
        }
    }
    if n >= 5 {
        for i in 0..8 {
            b(
                t,
                16 + i,
                31 - i,
                6 + ((brev(3, 7 - i) as i32) << 3),
                false,
                r,
            )?;
        }
    }
    if n == 6 {
        for i in 0..16 {
            h(t, 32 + 2 * i, 33 + 2 * i, i & 1 != 0, r);
        }
    }
    if n >= 4 {
        for i in 0..4 {
            b(
                t,
                8 + i,
                15 - i,
                12 + ((brev(2, 3 - i) as i32) << 4),
                false,
                r,
            )?;
        }
    }
    if n >= 5 {
        for i in 0..8 {
            h(t, 16 + 2 * i, 17 + 2 * i, i & 1 != 0, r);
        }
    }
    if n == 6 {
        for i in 0..4 {
            for j in 0..2 {
                b(
                    t,
                    62 - i * 4 - j,
                    33 + i * 4 + j,
                    60 - 16 * brev(2, i) as i32 + 64 * j as i32,
                    true,
                    r,
                )?;
            }
        }
    }
    if n >= 3 {
        for i in 0..2 {
            b(t, 4 + i, 7 - i, 56 - 32 * i as i32, false, r)?;
        }
    }
    if n >= 4 {
        for i in 0..4 {
            h(t, 8 + 2 * i, 9 + 2 * i, i & 1 != 0, r);
        }
    }
    if n >= 5 {
        for i in 0..2 {
            for j in 0..2 {
                b(
                    t,
                    30 - 4 * i - j,
                    17 + 4 * i + j,
                    24 + (j as i32 * 64) + (1 - i as i32) * 32,
                    true,
                    r,
                )?;
            }
        }
    }
    if n == 6 {
        for i in 0..8 {
            for j in 0..2 {
                h(t, 32 + i * 4 + j, 35 + i * 4 - j, i & 1 != 0, r);
            }
        }
    }
    for i in 0..2 {
        b(t, 2 * i, 2 * i + 1, 32 + 16 * i as i32, i == 0, r)?;
    }
    if n >= 3 {
        for i in 0..2 {
            h(t, 4 + 2 * i, 5 + 2 * i, i != 0, r);
        }
    }
    if n >= 4 {
        for i in 0..2 {
            b(t, 14 - i, 9 + i, 48 + 64 * i as i32, true, r)?;
        }
    }
    if n >= 5 {
        for i in 0..4 {
            for j in 0..2 {
                h(t, 16 + 4 * i + j, 19 + 4 * i - j, i & 1 != 0, r);
            }
        }
    }
    if n == 6 {
        for i in 0..2 {
            for j in 0..4 {
                b(
                    t,
                    61 - i * 8 - j,
                    34 + i * 8 + j,
                    56 - i as i32 * 32 + (j as i32 >> 1) * 64,
                    true,
                    r,
                )?;
            }
        }
    }
    for i in 0..2 {
        h(t, i, 3 - i, false, r);
    }
    if n >= 3 {
        b(t, 6, 5, 32, true, r)?;
    }
    if n >= 4 {
        for i in 0..2 {
            for j in 0..2 {
                h(t, 8 + 4 * i + j, 11 + 4 * i - j, i != 0, r);
            }
        }
    }
    if n >= 5 {
        for i in 0..4 {
            b(t, 29 - i, 18 + i, 48 + (i as i32 >> 1) * 64, true, r)?;
        }
    }
    if n == 6 {
        for i in 0..4 {
            for j in 0..4 {
                h(t, 32 + 8 * i + j, 39 + 8 * i - j, i & 1 != 0, r);
            }
        }
    }
    if n >= 3 {
        for i in 0..4 {
            h(t, i, 7 - i, false, r);
        }
    }
    if n >= 4 {
        for i in 0..2 {
            b(t, 13 - i, 10 + i, 32, true, r)?;
        }
    }
    if n >= 5 {
        for i in 0..2 {
            for j in 0..4 {
                h(t, 16 + 8 * i + j, 23 + 8 * i - j, i != 0, r);
            }
        }
    }
    if n == 6 {
        for i in 0..8 {
            b(t, 59 - i, 36 + i, if i < 4 { 48 } else { 112 }, true, r)?;
        }
    }
    if n >= 4 {
        for i in 0..8 {
            h(t, i, 15 - i, false, r);
        }
    }
    if n >= 5 {
        for i in 0..4 {
            b(t, 27 - i, 20 + i, 32, true, r)?;
        }
    }
    if n == 6 {
        for i in 0..8 {
            h(t, 32 + i, 47 - i, false, r);
            h(t, 48 + i, 63 - i, true, r);
        }
    }
    if n >= 5 {
        for i in 0..16 {
            h(t, i, 31 - i, false, r);
        }
    }
    if n == 6 {
        for i in 0..8 {
            b(t, 55 - i, 40 + i, 32, true, r)?;
        }
        for i in 0..32 {
            h(t, i, 63 - i, false, r);
        }
    }
    Ok(())
}

fn adst(t: &mut [i64], r: u8) -> CodecResult<()> {
    if t.len() == 4 {
        let (a, b, c, d) = (t[0], t[1], t[2], t[3]);
        let a7 = check(a - c, r + 1)?;
        let b7 = check(a7 + d, r)?;
        let mut s = [
            1321 * a,
            2482 * a,
            3344 * b,
            3803 * c,
            1321 * c,
            2482 * d,
            3803 * d,
        ];
        for &v in &s {
            check(v, r + 12)?;
        }
        s[0] = check(s[0] + s[3], r + 12)?;
        s[1] = check(s[1] - s[4], r + 12)?;
        s[3] = s[2];
        s[2] = check(3344 * b7, r + 12)?;
        s[0] = check(s[0] + s[5], r + 12)?;
        s[1] = check(s[1] - s[6], r + 12)?;
        let mut x = [
            check(s[0] + s[3], r + 12)?,
            check(s[1] + s[3], r + 12)?,
            s[2],
            check(s[0] + s[1], r + 12)?,
        ];
        x[3] = check(x[3] - s[3], r + 12)?;
        for (v, x) in t.iter_mut().zip(x) {
            *v = round(x, 12);
        }
        return Ok(());
    }
    let len = t.len();
    let n = len.ilog2();
    let copy = t.to_vec();
    for (i, v) in t.iter_mut().enumerate() {
        *v = copy[if i & 1 != 0 { i - 1 } else { len - i - 1 }];
    }
    if len == 8 {
        for i in 0..4 {
            b(t, 2 * i, 2 * i + 1, 60 - 16 * i as i32, true, r)?;
        }
        for i in 0..4 {
            h(t, i, 4 + i, false, r);
        }
        for i in 0..2 {
            b(t, 4 + 3 * i, 5 + i, 48 - 32 * i as i32, true, r)?;
        }
    } else {
        for i in 0..8 {
            b(t, 2 * i, 2 * i + 1, 62 - 8 * i as i32, true, r)?;
        }
        for i in 0..8 {
            h(t, i, 8 + i, false, r);
        }
        for i in 0..2 {
            b(t, 8 + 2 * i, 9 + 2 * i, 56 - 32 * i as i32, true, r)?;
            b(t, 13 + 2 * i, 12 + 2 * i, 8 + 32 * i as i32, true, r)?;
        }
        for i in 0..4 {
            for j in 0..2 {
                h(t, 8 * j + i, 4 + 8 * j + i, false, r);
            }
        }
        for i in 0..2 {
            for j in 0..2 {
                b(
                    t,
                    4 + 8 * j + 3 * i,
                    5 + 8 * j + i,
                    48 - 32 * i as i32,
                    true,
                    r,
                )?;
            }
        }
    }
    for i in 0..2 {
        for j in 0..len / 4 {
            h(t, 4 * j + i, 2 + 4 * j + i, false, r);
        }
    }
    for i in 0..len / 4 {
        b(t, 2 + 4 * i, 3 + 4 * i, 32, true, r)?;
    }
    let copy = t.to_vec();
    for (i, v) in t.iter_mut().enumerate() {
        let a = (i >> 3) & 1;
        let b = ((i >> 2) & 1) ^ ((i >> 3) & 1);
        let c = ((i >> 1) & 1) ^ ((i >> 2) & 1);
        let d = (i & 1) ^ ((i >> 1) & 1);
        let idx = ((d << 3) | (c << 2) | (b << 1) | a) >> (4 - n);
        *v = if i & 1 != 0 { -copy[idx] } else { copy[idx] };
    }
    Ok(())
}
