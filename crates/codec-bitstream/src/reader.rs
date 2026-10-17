use codec_core::{CodecError, CodecResult};

#[derive(Clone)]
pub struct BitReader<'a> {
    data: &'a [u8],
    byte_pos: usize,
    bit_pos: u8,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte_pos: 0,
            bit_pos: 0,
        }
    }

    pub fn bits_left(&self) -> usize {
        if self.byte_pos >= self.data.len() {
            return 0;
        }
        (self.data.len() - self.byte_pos).saturating_mul(8) - self.bit_pos as usize
    }

    /// 64-bit register lookahead without advancing the bit cursor.
    /// Packs up to `n` upcoming bits (1..=56) in a single register load when byte-aligned
    /// or across an unaligned 64-bit window.
    pub fn peek_bits64(&self, n: u8) -> CodecResult<u64> {
        if n == 0 {
            return Ok(0);
        }
        if n > 56 {
            return Err(CodecError::InvalidData);
        }
        if self.bits_left() < n as usize {
            return Err(CodecError::EndOfStream);
        }
        let rem = &self.data[self.byte_pos..];
        let mut reg = 0u64;
        let load_len = rem.len().min(8);
        for (i, &b) in rem[..load_len].iter().enumerate() {
            reg |= u64::from(b) << (56 - i * 8);
        }
        let shifted = reg << self.bit_pos;
        Ok(shifted >> (64 - n))
    }

    /// Fast SIMD-friendly 8-byte aligned zero/marker lookahead for high-bitrate tile group parsing.
    pub fn lookahead_aligned_bytes(&self, len: usize) -> CodecResult<&'a [u8]> {
        let aligned_pos = if self.bit_pos == 0 {
            self.byte_pos
        } else {
            self.byte_pos.saturating_add(1)
        };
        let end = aligned_pos
            .checked_add(len)
            .ok_or(CodecError::InvalidData)?;
        self.data
            .get(aligned_pos..end)
            .ok_or(CodecError::EndOfStream)
    }

    pub fn read_bit(&mut self) -> CodecResult<bool> {
        if self.byte_pos >= self.data.len() {
            return Err(CodecError::EndOfStream);
        }
        let bit = (self.data[self.byte_pos] >> (7 - self.bit_pos)) & 1;
        self.bit_pos += 1;
        if self.bit_pos == 8 {
            self.bit_pos = 0;
            self.byte_pos += 1;
        }
        Ok(bit == 1)
    }

    pub fn read_bits(&mut self, n: u8) -> CodecResult<u32> {
        if n == 0 {
            return Ok(0);
        }
        if n > 32 {
            return Err(CodecError::InvalidData);
        }
        if self.bits_left() < n as usize {
            return Err(CodecError::EndOfStream);
        }

        let mut result = 0;
        for _ in 0..n {
            result = (result << 1) | (self.read_bit()? as u32);
        }
        Ok(result)
    }

    pub fn read_bits64(&mut self, n: u8) -> CodecResult<u64> {
        if n == 0 {
            return Ok(0);
        }
        if n > 64 {
            return Err(CodecError::InvalidData);
        }
        if self.bits_left() < n as usize {
            return Err(CodecError::EndOfStream);
        }

        let mut result = 0;
        for _ in 0..n {
            result = (result << 1) | (self.read_bit()? as u64);
        }
        Ok(result)
    }

    pub fn read_bytes(&mut self, n: usize) -> CodecResult<&'a [u8]> {
        if self.bit_pos != 0 {
            return Err(CodecError::InvalidData); // Must be byte aligned
        }
        let end = self
            .byte_pos
            .checked_add(n)
            .ok_or(CodecError::InvalidData)?;
        if end > self.data.len() {
            return Err(CodecError::EndOfStream);
        }
        let slice = &self.data[self.byte_pos..end];
        self.byte_pos = end;
        Ok(slice)
    }

    pub fn read_leb128(&mut self) -> CodecResult<u64> {
        // AV1 leb128(): at most eight bytes, value at most u32::MAX.
        // Non-minimal representations are explicitly permitted by AV1.
        let mut value = 0u64;
        for i in 0..8 {
            let byte = self.read_bits(8)?;
            value |= u64::from(byte & 0x7f) << (7 * i);
            if value > u64::from(u32::MAX) {
                return Err(CodecError::InvalidData);
            }
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(CodecError::InvalidData)
    }

    /// AV1 uvlc differs from Exp-Golomb at 32 leading zero bits.
    pub fn read_uvlc(&mut self) -> CodecResult<u32> {
        let mut zeros = 0;
        while !self.read_bit()? {
            zeros += 1;
            if zeros == 32 {
                return Ok(u32::MAX);
            }
        }
        Ok(((1u32 << zeros) - 1) + self.read_bits(zeros)?)
    }

    /// AV1 su(n): n-bit two's-complement signed integer.
    pub fn read_signed(&mut self, n: u8) -> CodecResult<i32> {
        if n == 0 || n > 32 {
            return Err(CodecError::InvalidData);
        }
        let value = self.read_bits(n)?;
        Ok(((value << (32 - n)) as i32) >> (32 - n))
    }

    /// AV1 ns(n), including the zero-bit n=1 case.
    pub fn read_ns(&mut self, n: u32) -> CodecResult<u32> {
        if n == 0 {
            return Err(CodecError::InvalidData);
        }
        let w = (32 - n.leading_zeros()) as u8;
        let m = ((1u64 << w) - u64::from(n)) as u32;
        let v = self.read_bits(w - 1)?;
        if v < m {
            Ok(v)
        } else {
            Ok((v << 1) - m + u32::from(self.read_bit()?))
        }
    }

    /// AV1 decode_subexp(numSyms), section 5.9.27.
    pub fn read_subexp(&mut self, num_syms: u32) -> CodecResult<u32> {
        if num_syms == 0 {
            return Err(CodecError::InvalidData);
        }
        let mut i = 0u8;
        let mut mk = 0u64;
        loop {
            let b = if i == 0 { 3 } else { i + 2 };
            let a = 1u64 << b;
            if u64::from(num_syms) <= mk + 3 * a {
                return Ok(mk as u32 + self.read_ns(num_syms - mk as u32)?);
            }
            if !self.read_bit()? {
                return Ok(mk as u32 + self.read_bits(b)?);
            }
            i += 1;
            mk += a;
        }
    }

    /// Strict AV1 byte_alignment(), never silently discard nonzero bits.
    pub fn read_byte_alignment(&mut self) -> CodecResult<()> {
        while !self.is_aligned() {
            if self.read_bit()? {
                return Err(CodecError::InvalidData);
            }
        }
        Ok(())
    }

    pub fn read_trailing_bits(&mut self) -> CodecResult<()> {
        if !self.read_bit()? {
            return Err(CodecError::InvalidData);
        }
        while self.bits_left() > 0 {
            if self.read_bit()? {
                return Err(CodecError::InvalidData);
            }
        }
        Ok(())
    }

    pub fn read_ue(&mut self) -> CodecResult<u32> {
        let mut leading_zero_bits = 0;
        while !self.read_bit()? {
            leading_zero_bits += 1;
            if leading_zero_bits >= 32 {
                return Err(CodecError::InvalidData);
            }
        }
        let read_val = self.read_bits(leading_zero_bits)?;
        Ok((1 << leading_zero_bits) - 1 + read_val)
    }

    pub fn byte_align(&mut self) {
        if self.bit_pos != 0 {
            self.bit_pos = 0;
            self.byte_pos += 1;
        }
    }

    pub fn is_aligned(&self) -> bool {
        self.bit_pos == 0
    }
}

/// Zero-branch, 64-bit register-buffered bit reader designed for high-bitrate
/// tile group and entropy stream parsing.
#[derive(Clone, Debug)]
pub struct RegisterBitReader<'a> {
    data: &'a [u8],
    cursor: usize,
    bit_buf: u64,
    bits_in_buf: u8,
}

impl<'a> RegisterBitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        let mut r = Self {
            data,
            cursor: 0,
            bit_buf: 0,
            bits_in_buf: 0,
        };
        r.refill();
        r
    }

    #[inline]
    pub fn refill(&mut self) {
        while self.bits_in_buf <= 56 && self.cursor < self.data.len() {
            let b = u64::from(self.data[self.cursor]);
            self.bit_buf |= b << (56 - self.bits_in_buf);
            self.bits_in_buf += 8;
            self.cursor += 1;
        }
    }

    #[inline]
    pub fn bits_left(&self) -> usize {
        (self.data.len() - self.cursor) * 8 + usize::from(self.bits_in_buf)
    }

    #[inline]
    pub fn read_bits_fast(&mut self, n: u8) -> CodecResult<u32> {
        if n == 0 {
            return Ok(0);
        }
        if n > 32 {
            return Err(CodecError::InvalidData);
        }
        if self.bits_in_buf < n {
            self.refill();
            if self.bits_in_buf < n {
                return Err(CodecError::EndOfStream);
            }
        }
        let val = (self.bit_buf >> (64 - n)) as u32;
        self.bit_buf <<= n;
        self.bits_in_buf -= n;
        self.refill();
        Ok(val)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_bits() {
        let data = [0b10101010, 0b11001100];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.read_bits(4).unwrap(), 0b1010);
        assert_eq!(reader.read_bits(4).unwrap(), 0b1010);
        assert_eq!(reader.read_bits(8).unwrap(), 0b11001100);
    }

    #[test]
    fn test_register_bit_reader_and_peek() {
        let data = [0xDE, 0xAD, 0xBE, 0xEF, 0x12, 0x34];
        let mut r1 = BitReader::new(&data);
        let mut r2 = RegisterBitReader::new(&data);
        assert_eq!(r1.peek_bits64(16).unwrap(), 0xDEAD);
        for &width in &[5, 11, 8, 13, 11] {
            assert_eq!(
                r1.read_bits(width).unwrap(),
                r2.read_bits_fast(width).unwrap()
            );
        }
    }

    #[test]
    fn test_leb128() {
        let data = [0xE5, 0x8E, 0x26];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.read_leb128().unwrap(), 624485);
    }

    #[test]
    fn test_exp_golomb() {
        let data = [0b00100000];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.read_ue().unwrap(), 3);
    }
}

#[cfg(test)]
mod safety_tests {
    use super::*;
    #[test]
    fn bytes_overflow_and_alignment() {
        let mut r = BitReader::new(&[0, 0]);
        r.read_bytes(1).unwrap();
        assert!(r.read_bytes(usize::MAX).is_err());
        assert_eq!(r.read_bytes(1).unwrap(), &[0]);
        let mut r = BitReader::new(&[0]);
        r.read_bit().unwrap();
        assert!(r.read_bytes(0).is_err());
    }
    #[test]
    fn leb128_limits() {
        assert_eq!(
            BitReader::new(&[0xff, 0xff, 0xff, 0xff, 0x0f])
                .read_leb128()
                .unwrap(),
            u64::from(u32::MAX)
        );
        assert_eq!(
            BitReader::new(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0])
                .read_leb128()
                .unwrap(),
            0
        );
        for bytes in [
            &[0x80][..],
            &[0xff, 0xff, 0xff, 0xff, 0x10],
            &[0x80; 8],
            &[0xff; 10],
        ] {
            assert!(BitReader::new(bytes).read_leb128().is_err());
        }
    }
}

#[cfg(test)]
mod uvlc_tests {
    use super::*;
    #[test]
    fn av1_uvlc_maximum() {
        assert_eq!(BitReader::new(&[0; 4]).read_uvlc().unwrap(), u32::MAX);
        assert!(BitReader::new(&[0; 3]).read_uvlc().is_err());
        assert_eq!(BitReader::new(&[0x80]).read_uvlc().unwrap(), 0);
    }
}

#[cfg(test)]
mod av1_syntax_tests {
    use super::*;
    #[test]
    fn signed_extremes_and_ns_boundaries() {
        assert_eq!(BitReader::new(&[0x80]).read_signed(7).unwrap(), -64);
        assert_eq!(BitReader::new(&[0x7e]).read_signed(7).unwrap(), 63);
        assert_eq!(BitReader::new(&[0xff; 4]).read_signed(32).unwrap(), -1);
        assert!(BitReader::new(&[]).read_signed(0).is_err());
        assert_eq!(BitReader::new(&[]).read_ns(1).unwrap(), 0);
        // ns(3): codewords 0,10,11.
        for (byte, value, left) in [(0, 0, 7), (0x80, 1, 6), (0xc0, 2, 6)] {
            let bytes = [byte];
            let mut r = BitReader::new(&bytes);
            assert_eq!(r.read_ns(3).unwrap(), value);
            assert_eq!(r.bits_left(), left);
        }
        assert_eq!(
            BitReader::new(&[0xff; 4]).read_ns(u32::MAX).unwrap(),
            u32::MAX - 1
        );
        assert!(BitReader::new(&[]).read_ns(0).is_err());
    }
    #[test]
    fn subexponential_branches_and_alignment() {
        // numSyms=40: first bucket [0,7] uses 0+3 bits; second [8,15]
        // uses 10+3 bits; final [16,39] uses 11+ns(24).
        assert_eq!(BitReader::new(&[0x70]).read_subexp(40).unwrap(), 7);
        assert_eq!(BitReader::new(&[0xb8]).read_subexp(40).unwrap(), 15);
        assert_eq!(BitReader::new(&[0xc0]).read_subexp(40).unwrap(), 16);
        assert_eq!(BitReader::new(&[0xfe]).read_subexp(40).unwrap(), 39);
        assert_eq!(BitReader::new(&[]).read_subexp(1).unwrap(), 0);
        assert!(BitReader::new(&[]).read_subexp(0).is_err());
        let mut r = BitReader::new(&[0x80]);
        r.read_bit().unwrap();
        r.read_byte_alignment().unwrap();
        let mut r = BitReader::new(&[0x81]);
        r.read_bit().unwrap();
        assert!(r.read_byte_alignment().is_err());
    }
}
