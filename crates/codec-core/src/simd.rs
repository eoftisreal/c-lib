//! Runtime CPU feature detection (`CpuFlags`) and SIMD function-pointer dispatch (`DspContext`).

use bitflags::bitflags;

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct CpuFlags: u32 {
        const SCALAR   = 0;
        const SSE41    = 1 << 0;
        const AVX2     = 1 << 1;
        const AVX512   = 1 << 2;
        const NEON     = 1 << 3;
    }
}

impl CpuFlags {
    pub fn detect() -> Self {
        if std::env::var("CODEC_SIMD_OFF")
            .map(|v| v.eq_ignore_ascii_case("all") || v == "1")
            .unwrap_or(false)
        {
            return Self::SCALAR;
        }
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        {
            let mut flags = Self::SCALAR;
            if std::arch::is_x86_feature_detected!("sse4.1") {
                flags |= Self::SSE41;
            }
            if std::arch::is_x86_feature_detected!("avx2") {
                flags |= Self::AVX2;
            }
            flags
        }
        #[cfg(target_arch = "aarch64")]
        {
            Self::NEON
        }
        #[cfg(not(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")))]
        {
            Self::SCALAR
        }
    }
}

pub type CdefFindDirFn = fn(block: &[u16], stride: usize, bit_depth: u8) -> (u8, u32);

#[derive(Clone, Copy)]
pub struct DspContext {
    pub flags: CpuFlags,
    pub cdef_find_dir: CdefFindDirFn,
}

impl std::fmt::Debug for DspContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DspContext")
            .field("flags", &self.flags)
            .finish()
    }
}

impl Default for DspContext {
    fn default() -> Self {
        Self::new(CpuFlags::detect())
    }
}

impl DspContext {
    pub fn new(flags: CpuFlags) -> Self {
        Self {
            flags,
            cdef_find_dir: cdef_find_dir_scalar,
        }
    }
}

/// Normative AV1 8-direction CDEF variance search over an 8x8 block:
/// `s[d] = sum_{L} (partial[d][L])^2 * (840 / count[d][L])`, `d* = argmax_d s[d]`.
pub fn cdef_find_dir_scalar(block: &[u16], stride: usize, bit_depth: u8) -> (u8, u32) {
    const DIV_TABLE: [i32; 9] = [0, 840, 420, 280, 210, 168, 140, 120, 105];
    let shift = bit_depth.saturating_sub(8);
    let mut partial = [[0i32; 15]; 8];

    for r in 0..8usize {
        for c in 0..8usize {
            let px = (i32::from(block[r * stride + c]) >> shift) - 128;
            partial[0][r + c] += px;
            partial[1][r + (c >> 1)] += px;
            partial[2][r] += px;
            partial[3][3 + r - (c >> 1)] += px;
            partial[4][7 + r - c] += px;
            partial[5][3 - (r >> 1) + c] += px;
            partial[6][c] += px;
            partial[7][(r >> 1) + c] += px;
        }
    }

    let mut cost = [0i64; 8];
    for k in 0..8usize {
        let count = k + 1;
        let div = i64::from(DIV_TABLE[count]);
        cost[2] += i64::from(partial[2][k]) * i64::from(partial[2][k]) * i64::from(DIV_TABLE[8]);
        cost[6] += i64::from(partial[6][k]) * i64::from(partial[6][k]) * i64::from(DIV_TABLE[8]);
        cost[0] += (i64::from(partial[0][k]) * i64::from(partial[0][k])
            + i64::from(partial[0][14 - k]) * i64::from(partial[0][14 - k]))
            * div;
        cost[4] += (i64::from(partial[4][k]) * i64::from(partial[4][k])
            + i64::from(partial[4][14 - k]) * i64::from(partial[4][14 - k]))
            * div;
    }
    cost[0] -= i64::from(partial[0][7]) * i64::from(partial[0][7]) * i64::from(DIV_TABLE[8]);
    cost[4] -= i64::from(partial[4][7]) * i64::from(partial[4][7]) * i64::from(DIV_TABLE[8]);

    for d in (1..8usize).step_by(2) {
        for k in 0..5usize {
            cost[d] += i64::from(partial[d][3 + k])
                * i64::from(partial[d][3 + k])
                * i64::from(DIV_TABLE[8]);
        }
        for k in 0..3usize {
            let c_len = k * 2 + 2;
            let div = i64::from(DIV_TABLE[c_len]);
            cost[d] += (i64::from(partial[d][k]) * i64::from(partial[d][k])
                + i64::from(partial[d][10 - k]) * i64::from(partial[d][10 - k]))
                * div;
        }
    }

    let mut best_dir = 0u8;
    let mut best_cost = cost[0];
    for (d, &c) in cost.iter().enumerate().skip(1) {
        if c > best_cost {
            best_cost = c;
            best_dir = d as u8;
        }
    }
    let var = ((best_cost - cost[(best_dir as usize + 4) & 7]) >> 10).max(0) as u32;
    (best_dir, var)
}
