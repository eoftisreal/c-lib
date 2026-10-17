use crate::SequenceHeader;
use codec_bitstream::BitReader;
use codec_core::{CodecError, CodecResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quantization {
    pub base_q_idx: u8,
    /// Y DC, U DC, U AC, V DC, V AC.
    pub deltas: [i32; 5],
    pub qm: Option<[u8; 3]>,
}
fn delta(r: &mut BitReader) -> CodecResult<i32> {
    if r.read_bit()? {
        r.read_signed(7)
    } else {
        Ok(0)
    }
}
impl Quantization {
    pub(crate) fn parse(r: &mut BitReader, s: &SequenceHeader) -> CodecResult<Self> {
        let base_q_idx = r.read_bits(8)? as u8;
        let mut deltas = [0; 5];
        deltas[0] = delta(r)?;
        if !s.color_config.mono_chrome {
            let diff = s.color_config.separate_uv_delta_q && r.read_bit()?;
            deltas[1] = delta(r)?;
            deltas[2] = delta(r)?;
            deltas[3] = if diff { delta(r)? } else { deltas[1] };
            deltas[4] = if diff { delta(r)? } else { deltas[2] };
        }
        let qm = if r.read_bit()? {
            let y = r.read_bits(4)? as u8;
            let u = r.read_bits(4)? as u8;
            let v = if s.color_config.separate_uv_delta_q {
                r.read_bits(4)? as u8
            } else {
                u
            };
            Some([y, u, v])
        } else {
            None
        };
        Ok(Self {
            base_q_idx,
            deltas,
            qm,
        })
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Segmentation {
    pub enabled: bool,
    pub features: [[Option<i32>; 8]; 8],
}
impl Segmentation {
    // All supported headers have PRIMARY_REF_NONE: update_map/data are inferred.
    pub(crate) fn parse(r: &mut BitReader) -> CodecResult<Self> {
        let mut out = Self {
            enabled: r.read_bit()?,
            ..Self::default()
        };
        if out.enabled {
            let bits = [8, 6, 6, 6, 6, 3, 0, 0];
            let limits = [255, 63, 63, 63, 63, 7, 0, 0];
            for segment in &mut out.features {
                for j in 0..8 {
                    if r.read_bit()? {
                        let value = if j < 5 {
                            r.read_signed(bits[j] + 1)?
                        } else {
                            r.read_bits(bits[j])? as i32
                        };
                        segment[j] = Some(value.clamp(-limits[j], limits[j]));
                    }
                }
            }
        }
        Ok(out)
    }
    pub fn coded_lossless(&self, q: &Quantization) -> bool {
        q.deltas == [0; 5]
            && if self.enabled {
                self.features.iter().all(|s| {
                    (i64::from(q.base_q_idx) + i64::from(s[0].unwrap_or(0))).clamp(0, 255) == 0
                })
            } else {
                q.base_q_idx == 0
            }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaParameters {
    pub q_res: Option<u8>,
    pub lf_res: Option<u8>,
    pub lf_multi: bool,
}
impl DeltaParameters {
    pub(crate) fn parse(r: &mut BitReader, base: u8, intrabc: bool) -> CodecResult<Self> {
        let q_res = if base > 0 && r.read_bit()? {
            Some(r.read_bits(2)? as u8)
        } else {
            None
        };
        let lf_res = if q_res.is_some() && !intrabc && r.read_bit()? {
            Some(r.read_bits(2)? as u8)
        } else {
            None
        };
        let lf_multi = lf_res.is_some() && r.read_bit()?;
        Ok(Self {
            q_res,
            lf_res,
            lf_multi,
        })
    }
}

/// Normative DC and AC step sizes (not dequantized coefficient values).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuantizerSteps {
    pub dc: u16,
    pub ac: u16,
}

impl Quantization {
    /// Derive segment- and plane-specific steps from the current decoded Q index.
    /// `current_q_index` is Some only when delta-Q is present; this function does
    /// not parse the delta-Q symbol or apply the resolution shift itself.
    pub fn steps(
        &self,
        segmentation: &Segmentation,
        segment_id: u8,
        plane: u8,
        bit_depth: u8,
        current_q_index: Option<u8>,
    ) -> CodecResult<QuantizerSteps> {
        if segment_id >= 8 || plane >= 3 || !matches!(bit_depth, 8 | 10 | 12) {
            return Err(CodecError::InvalidData);
        }
        let alt_q = if segmentation.enabled {
            segmentation.features[usize::from(segment_id)][0].unwrap_or(0)
        } else {
            0
        };
        if !(-255..=255).contains(&alt_q) || self.deltas.iter().any(|&d| !(-64..=63).contains(&d)) {
            return Err(CodecError::InvalidData);
        }
        let index = (i32::from(current_q_index.unwrap_or(self.base_q_idx)) + alt_q).clamp(0, 255);
        let (dc_delta, ac_delta) = match plane {
            0 => (self.deltas[0], 0),
            1 => (self.deltas[1], self.deltas[2]),
            _ => (self.deltas[3], self.deltas[4]),
        };
        quantizer_steps(index + dc_delta, index + ac_delta, bit_depth)
    }
}

/// Lookup with normative Q-index clipping. All six 256-entry tables are exact;
/// scaling the 8-bit table is not equivalent for 10- or 12-bit content.
pub fn quantizer_steps(dc_index: i32, ac_index: i32, bit_depth: u8) -> CodecResult<QuantizerSteps> {
    use super::quant_tables::{AC_QLOOKUP, DC_QLOOKUP};
    if !matches!(bit_depth, 8 | 10 | 12) {
        return Err(CodecError::InvalidData);
    }
    let row = usize::from((bit_depth - 8) / 2);
    Ok(QuantizerSteps {
        dc: DC_QLOOKUP[row][dc_index.clamp(0, 255) as usize],
        ac: AC_QLOOKUP[row][ac_index.clamp(0, 255) as usize],
    })
}
