//! Tile-bounded AV1 symbol decoder (specification sections 8.2.1–8.2.6).
//! CDFs are ascending cumulative probabilities, followed by the adaptation count.
//! This is the arithmetic primitive, not block syntax or a bank of codec CDFs.
use codec_bitstream::BitReader;
use codec_core::{CodecError, CodecResult};

#[derive(Clone)]
pub struct SymbolDecoder<'a> {
    data: &'a [u8],
    reader: BitReader<'a>,
    range: u32,
    value: u32,
    max_bits: i64,
    disable_cdf_update: bool,
}
impl<'a> SymbolDecoder<'a> {
    /// The slice must contain exactly one tile's entropy-coded bytes.
    pub fn new(data: &'a [u8], disable_cdf_update: bool) -> CodecResult<Self> {
        if data.is_empty() {
            return Err(CodecError::EndOfStream);
        }
        let total = i64::try_from(data.len())
            .ok()
            .and_then(|n| n.checked_mul(8))
            .ok_or(CodecError::InvalidData)?;
        let mut reader = BitReader::new(data);
        let bits = total.min(15) as u8;
        let value = 32767 ^ (reader.read_bits(bits)? << (15 - bits));
        Ok(Self {
            data,
            reader,
            range: 32768,
            value,
            max_bits: total - 15,
            disable_cdf_update,
        })
    }
    /// Decode and adapt atomically: errors change neither decoder nor caller CDF.
    pub fn read_symbol(&mut self, cdf: &mut [u16]) -> CodecResult<usize> {
        let n = cdf.len().checked_sub(1).ok_or(CodecError::InvalidData)?;
        if !(2..=16).contains(&n)
            || cdf[n - 1] != 32768
            || cdf[n] > 32
            || cdf[..n].windows(2).any(|w| w[0] > w[1])
        {
            return Err(CodecError::InvalidData);
        }
        let mut next = self.clone();
        let mut cur = next.range;
        let mut symbol = 0;
        let prev = loop {
            let prev = cur;
            let f = 32768 - u32::from(cdf[symbol]);
            cur = ((next.range >> 8) * (f >> 6)) >> 1;
            cur += 4 * (n - symbol - 1) as u32;
            if cur >= prev {
                return Err(CodecError::InvalidData);
            }
            if next.value >= cur {
                break prev;
            }
            symbol += 1;
        };
        next.range = prev - cur;
        next.value -= cur;
        let bits = (next.range.leading_zeros() - 16) as u8;
        next.range <<= bits;
        let num_bits = i64::from(bits).min(next.max_bits.max(0)) as u8;
        let data = next.reader.read_bits(num_bits)?;
        next.value = (data << (bits - num_bits)) ^ (((next.value + 1) << bits) - 1);
        next.max_bits -= i64::from(bits);
        if next.max_bits < -14 {
            return Err(CodecError::EndOfStream);
        }
        if !self.disable_cdf_update {
            let rate = 3
                + u32::from(cdf[n] > 15)
                + u32::from(cdf[n] > 31)
                + (usize::BITS - 1 - n.leading_zeros()).min(2);
            for (i, value) in cdf[..n - 1].iter_mut().enumerate() {
                if i < symbol {
                    *value -= *value >> rate;
                } else {
                    *value += (32768 - *value) >> rate;
                }
            }
            cdf[n] += u16::from(cdf[n] < 32);
        }
        *self = next;
        Ok(symbol)
    }
    pub fn read_bool(&mut self) -> CodecResult<bool> {
        Ok(self.read_symbol(&mut [16384, 32768, 0])? != 0)
    }
    pub fn read_literal(&mut self, bits: u8) -> CodecResult<u32> {
        if bits > 32 {
            return Err(CodecError::InvalidData);
        }
        let mut next = self.clone();
        let mut value = 0;
        for _ in 0..bits {
            value = (value << 1) | u32::from(next.read_bool()?);
        }
        *self = next;
        Ok(value)
    }
    pub fn max_bits(&self) -> i64 {
        self.max_bits
    }
    /// Verify the normative tile termination bit and all remaining zero padding.
    /// Call after decoding the last expected syntax symbol; symbols alone cannot
    /// determine where a tile's block syntax ends.
    pub fn finish(self) -> CodecResult<()> {
        if self.max_bits < -14 {
            return Err(CodecError::EndOfStream);
        }
        let position = self.data.len() * 8 - self.reader.bits_left();
        let rewind = (self.max_bits + 15).min(15) as usize;
        let trailing = position
            .checked_sub(rewind)
            .ok_or(CodecError::InvalidData)?;
        for bit in trailing..self.data.len() * 8 {
            let set = (self.data[bit / 8] >> (7 - bit % 8)) & 1 != 0;
            if set != (bit == trailing) {
                return Err(CodecError::InvalidData);
            }
        }
        Ok(())
    }
}
