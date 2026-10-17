//! Normative scalar Wiener and SGRPROJ kernels with explicit pre-/post-CDEF
//! sources. Unit syntax and frame-wide scheduling are not implemented here.
use codec_core::{CodecError, CodecResult, Plane};
pub struct RestorationSource<'a> {
    pre: &'a Plane,
    post: &'a Plane,
    start: usize,
    end: usize,
    depth: u8,
}
fn round(v: i64, n: u8) -> i64 {
    if n == 0 {
        v
    } else {
        (v + (1i64 << (n - 1))) >> n
    }
}
impl<'a> RestorationSource<'a> {
    /// Stripe end is exclusive. Outside-stripe taps use preserved pre-CDEF rows,
    /// with the third row repeating the second, as required by AV1 7.17.6.
    pub fn new(
        pre: &'a Plane,
        post: &'a Plane,
        stripe: (usize, usize),
        depth: u8,
    ) -> CodecResult<Self> {
        if !matches!(depth, 8 | 10 | 12)
            || pre.width() != post.width()
            || pre.height() != post.height()
            || stripe.0 >= stripe.1
            || stripe.1 > pre.height()
        {
            return Err(CodecError::InvalidData);
        }
        Ok(Self {
            pre,
            post,
            start: stripe.0,
            end: stripe.1,
            depth,
        })
    }
    pub fn sample(&self, x: isize, y: isize) -> CodecResult<i64> {
        let x = x.clamp(0, self.post.width() as isize - 1) as usize;
        let y = y.clamp(0, self.post.height() as isize - 1) as usize;
        let (plane, y) = if y < self.start {
            (self.pre, y.max(self.start.saturating_sub(2)))
        } else if y >= self.end {
            (self.pre, y.min(self.end + 1))
        } else {
            (self.post, y)
        };
        let v = plane.row(y).ok_or(CodecError::InvalidData)?[x];
        if v >= (1u16 << self.depth) {
            return Err(CodecError::InvalidData);
        }
        Ok(i64::from(v))
    }
    fn region(&self, origin: (usize, usize), size: (usize, usize)) -> CodecResult<()> {
        let (x, y) = origin;
        let (w, h) = size;
        if w == 0
            || h == 0
            || w > 256
            || h > 256
            || y < self.start
            || x.checked_add(w).is_none_or(|v| v > self.post.width())
            || y.checked_add(h).is_none_or(|v| v > self.end)
        {
            return Err(CodecError::InvalidData);
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy)]
pub struct WienerCoefficients {
    pub horizontal: [i16; 3],
    pub vertical: [i16; 3],
}
impl WienerCoefficients {
    pub fn apply(
        self,
        source: &RestorationSource<'_>,
        origin: (usize, usize),
        size: (usize, usize),
    ) -> CodecResult<Vec<u16>> {
        source.region(origin, size)?;
        fn kernel(c: [i16; 3]) -> CodecResult<[i64; 7]> {
            if !(-5..=10).contains(&c[0])
                || !(-23..=8).contains(&c[1])
                || !(-17..=46).contains(&c[2])
            {
                return Err(CodecError::InvalidData);
            }
            let (a, b, c) = (i64::from(c[0]), i64::from(c[1]), i64::from(c[2]));
            Ok([a, b, c, 128 - 2 * (a + b + c), c, b, a])
        }
        let hk = kernel(self.horizontal)?;
        let vk = kernel(self.vertical)?;
        let (w, h) = size;
        let (x, y) = (origin.0 as isize, origin.1 as isize);
        let r0 = if source.depth == 12 { 5 } else { 3 };
        let r1 = 14 - r0;
        let offset = 1i64 << (source.depth + 7 - r0 - 1);
        let limit = (1i64 << (source.depth + 1 + 7 - r0)) - 1;
        let mut temp = vec![0i64; w * (h + 6)];
        for row in 0..h + 6 {
            for col in 0..w {
                let mut sum = 0;
                for (k, &tap) in hk.iter().enumerate() {
                    sum += tap
                        * source.sample(x + col as isize + k as isize - 3, y + row as isize - 3)?;
                }
                temp[row * w + col] = round(sum, r0).clamp(-offset, limit - offset);
            }
        }
        let max = (1i64 << source.depth) - 1;
        let mut out = vec![0; w * h];
        for row in 0..h {
            for col in 0..w {
                let mut sum = 0;
                for (k, &tap) in vk.iter().enumerate() {
                    sum += tap * temp[(row + k) * w + col];
                }
                out[row * w + col] = round(sum, r1).clamp(0, max) as u16;
            }
        }
        Ok(out)
    }
}
const SGR: [[i64; 4]; 16] = [
    [2, 12, 1, 4],
    [2, 15, 1, 6],
    [2, 18, 1, 8],
    [2, 21, 1, 9],
    [2, 24, 1, 10],
    [2, 29, 1, 11],
    [2, 36, 1, 12],
    [2, 45, 1, 13],
    [2, 56, 1, 14],
    [2, 68, 1, 15],
    [0, 0, 1, 5],
    [0, 0, 1, 8],
    [0, 0, 1, 11],
    [0, 0, 1, 14],
    [2, 30, 0, 0],
    [2, 75, 0, 0],
];
#[derive(Debug, Clone, Copy)]
pub struct SgrprojParameters {
    pub set: u8,
    pub xqd: [i16; 2],
}
impl SgrprojParameters {
    pub fn apply(
        self,
        source: &RestorationSource<'_>,
        origin: (usize, usize),
        size: (usize, usize),
    ) -> CodecResult<Vec<u16>> {
        source.region(origin, size)?;
        let params = *SGR
            .get(usize::from(self.set))
            .ok_or(CodecError::InvalidData)?;
        if !(-96..=31).contains(&self.xqd[0])
            || !(-32..=95).contains(&self.xqd[1])
            || (params[0] == 0 && self.xqd[0] != 0)
        {
            return Err(CodecError::InvalidData);
        }
        let f0 = box_filter(source, origin, size, params, 0)?;
        let f1 = box_filter(source, origin, size, params, 1)?;
        let (w, h) = size;
        let w0 = i64::from(self.xqd[0]);
        let w1 = i64::from(self.xqd[1]);
        let w2 = 128 - w0 - w1;
        let max = (1i64 << source.depth) - 1;
        let mut out = vec![0; w * h];
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                let u = source.sample((origin.0 + x) as isize, (origin.1 + y) as isize)? << 4;
                let v = w1 * u
                    + w0 * f0.as_ref().map_or(u, |f| f[i])
                    + w2 * f1.as_ref().map_or(u, |f| f[i]);
                out[i] = round(v, 11).clamp(0, max) as u16;
            }
        }
        Ok(out)
    }
}
fn box_filter(
    source: &RestorationSource<'_>,
    origin: (usize, usize),
    size: (usize, usize),
    params: [i64; 4],
    pass: usize,
) -> CodecResult<Option<Vec<i64>>> {
    let radius = params[pass * 2];
    if radius == 0 {
        return Ok(None);
    }
    let eps = params[pass * 2 + 1];
    let n = (2 * radius + 1) * (2 * radius + 1);
    let n2e = n * n * eps;
    let scale = ((1 << 20) + n2e / 2) / n2e;
    let recip = ((1 << 12) + n / 2) / n;
    let (w, h) = size;
    let pitch = w + 2;
    let mut a = vec![0i64; (w + 2) * (h + 2)];
    let mut b = vec![0i64; a.len()];
    for i in -1..h as isize + 1 {
        for j in -1..w as isize + 1 {
            let mut sumsq = 0;
            let mut sum = 0;
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    let c = source.sample(
                        origin.0 as isize + j + dx as isize,
                        origin.1 as isize + i + dy as isize,
                    )?;
                    sumsq += c * c;
                    sum += c;
                }
            }
            sumsq = round(sumsq, 2 * (source.depth - 8));
            let d = round(sum, source.depth - 8);
            let p = (sumsq * n - d * d).max(0);
            let z = round(p * scale, 20);
            let a2 = if z >= 255 {
                256
            } else if z == 0 {
                1
            } else {
                ((z << 8) + z / 2) / (z + 1)
            };
            let index = (i + 1) as usize * pitch + (j + 1) as usize;
            a[index] = a2;
            b[index] = round((256 - a2) * sum * recip, 12);
        }
    }
    let mut out = vec![0i64; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut aa = 0;
            let mut bb = 0;
            for dy in -1isize..=1 {
                for dx in -1isize..=1 {
                    let weight = if pass == 0 {
                        if (y as isize + dy) & 1 != 0 {
                            if dx == 0 {
                                6
                            } else {
                                5
                            }
                        } else {
                            0
                        }
                    } else if dx == 0 || dy == 0 {
                        4
                    } else {
                        3
                    };
                    let idx =
                        (y as isize + dy + 1) as usize * pitch + (x as isize + dx + 1) as usize;
                    aa += weight * a[idx];
                    bb += weight * b[idx];
                }
            }
            let shift = if pass == 0 && y & 1 != 0 { 4 } else { 5 };
            let v = aa * source.sample((origin.0 + x) as isize, (origin.1 + y) as isize)? + bb;
            out[y * w + x] = round(v, 8 + shift - 4);
        }
    }
    Ok(Some(out))
}
