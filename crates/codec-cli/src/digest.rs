//! RFC 1321 MD5 digest for compressed media sample bytes.
//! MD5 here is a compatibility checksum, not an authentication or security primitive.

const ROUND_CONSTANTS: [u32; 64] = [
    0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501,
    0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821,
    0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
    0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed, 0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a,
    0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70,
    0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
    0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
    0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
];

const ROTATIONS: [[u32; 4]; 4] = [
    [7, 12, 17, 22],
    [5, 9, 14, 20],
    [4, 11, 16, 23],
    [6, 10, 15, 21],
];

fn compress(state: &mut [u32; 4], block: &[u8]) {
    debug_assert_eq!(block.len(), 64);
    let mut words = [0u32; 16];
    for (i, word) in words.iter_mut().enumerate() {
        let offset = i * 4;
        *word = u32::from_le_bytes(block[offset..offset + 4].try_into().unwrap());
    }
    let [mut a, mut b, mut c, mut d] = *state;
    for i in 0..64 {
        let (f, g) = match i {
            0..=15 => ((b & c) | (!b & d), i),
            16..=31 => ((d & b) | (!d & c), (5 * i + 1) % 16),
            32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
            _ => (c ^ (b | !d), (7 * i) % 16),
        };
        let next = b.wrapping_add(
            a.wrapping_add(f)
                .wrapping_add(ROUND_CONSTANTS[i])
                .wrapping_add(words[g])
                .rotate_left(ROTATIONS[i / 16][i % 4]),
        );
        a = d;
        d = c;
        c = b;
        b = next;
    }
    for (value, increment) in state.iter_mut().zip([a, b, c, d]) {
        *value = value.wrapping_add(increment);
    }
}

/// Hex-encoded MD5 of the input bytes (not of decoded video planes).
pub fn md5_hex(data: &[u8]) -> String {
    let mut state = [0x67452301u32, 0xefcdab89, 0x98badcfe, 0x10325476];

    let (blocks, remainder) = data.as_chunks::<64>();
    for block in blocks {
        compress(&mut state, block);
    }
    let mut tail = [0u8; 128];
    tail[..remainder.len()].copy_from_slice(remainder);
    tail[remainder.len()] = 0x80;
    let tail_len = if remainder.len() < 56 { 64 } else { 128 };
    let bit_len = (data.len() as u64).wrapping_mul(8);
    tail[tail_len - 8..tail_len].copy_from_slice(&bit_len.to_le_bytes());
    for block in tail[..tail_len].as_chunks::<64>().0 {
        compress(&mut state, block);
    }

    use std::fmt::Write as _;
    let mut output = String::with_capacity(32);
    for word in state {
        for byte in word.to_le_bytes() {
            write!(&mut output, "{byte:02x}").expect("writing into String cannot fail");
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::md5_hex;

    #[test]
    fn rfc_1321_test_vectors() {
        for (bytes, expected) in [
            (&b""[..], "d41d8cd98f00b204e9800998ecf8427e"),
            (&b"a"[..], "0cc175b9c0f1b6a831c399e269772661"),
            (&b"abc"[..], "900150983cd24fb0d6963f7d28e17f72"),
            (&b"message digest"[..], "f96b697d7cb7938d525a2f31aaf161d0"),
            (&b"abcdefghijklmnopqrstuvwxyz"[..], "c3fcd3d76192e4007dfb496cca67e13b"),
            (
                &b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"[..],
                "d174ab98d277d9f5a5611c2c9f419d9f",
            ),
            (
                &b"12345678901234567890123456789012345678901234567890123456789012345678901234567890"[..],
                "57edf4a22be3c955ac49da2e2107b67a",
            ),
        ] {
            assert_eq!(md5_hex(bytes), expected);
        }
    }

    #[test]
    fn block_boundaries() {
        for (len, expected) in [
            (55, "ef1772b6dff9a122358552954ad0df65"),
            (56, "3b0c8ac703f828b04c6c197006d17218"),
            (63, "b06521f39153d618550606be297466d5"),
            (64, "014842d480b571495a4a0363793f7367"),
            (65, "c743a45e0d2e6a95cb859adae0248435"),
            (128, "e510683b3f5ffe4093d021808bc6ff70"),
            (129, "b325dc1c6f5e7a2b7cf465b9feab7948"),
        ] {
            assert_eq!(md5_hex(&vec![b'a'; len]), expected, "length {len}");
        }
    }
}
