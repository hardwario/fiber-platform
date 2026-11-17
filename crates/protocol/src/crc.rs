//! CRC-32/IEEE (reflected, polynomial `0xEDB88320`, as in zlib): the check of every frame, and
//! of a firmware image (`Hello.image_crc`). Bitwise, with no table: it is small on the south
//! bridge, whose flash is 32 KB.

/// One pass over `data`, continuing from `crc` (no initial value, no final xor), so that a
/// long image can be fed in pieces: `!crc32_update(crc32_update(!0, a), b)`.
pub const fn crc32_update(mut crc: u32, data: &[u8]) -> u32 {
    let mut i = 0;
    while i < data.len() {
        crc ^= data[i] as u32;
        let mut bit = 0;
        while bit < 8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            bit += 1;
        }
        i += 1;
    }
    crc
}

/// The CRC-32/IEEE of `data`.
pub const fn crc32(data: &[u8]) -> u32 {
    !crc32_update(0xFFFF_FFFF, data)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The standard check value of CRC-32/ISO-HDLC: a round trip alone would pass for a wrong CRC.
    #[test]
    fn check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
        assert_eq!(!crc32_update(crc32_update(!0, b"1234"), b"56789"), 0xCBF4_3926);
    }
}
