#[cfg(feature = "zlib")]
pub fn adler32(sum: u32, bytes: &[u8]) -> u32 {
    use std::os::raw::c_uint;

    unsafe { libz_sys::adler32(sum, &bytes[0], bytes.len() as c_uint) as u32 }
}

#[cfg(feature = "zlib")]
pub fn adler32_initial() -> u32 {
    use std::ptr;

    unsafe { libz_sys::adler32(0, ptr::null(), 0) as u32 }
}

#[cfg(feature = "zlib")]
pub fn adler32_combine(sum_a: u32, sum_b: u32, len_b: usize) -> u32 {
    use std::os::raw::c_long;

    unsafe { libz_sys::adler32_combine(sum_a, sum_b, len_b as c_long) as u32 }
}

#[cfg(all(feature = "miniz", not(feature = "zlib")))]
pub fn adler32(sum: u32, bytes: &[u8]) -> u32 {
    let mut adler = simd_adler32::Adler32::from_checksum(sum);
    adler.write(bytes);
    adler.finish()
}

#[cfg(all(feature = "miniz", not(feature = "zlib")))]
pub fn adler32_initial() -> u32 {
    1
}

#[cfg(all(feature = "miniz", not(feature = "zlib")))]
pub fn adler32_combine(sum_a: u32, sum_b: u32, len_b: usize) -> u32 {
    const BASE: u32 = 65521;

    /* the derivation of this formula is left as an exercise for the reader */
    let rem = (len_b as u32) % BASE;

    let mut sum1 = sum_a & 0xffff;
    let mut sum2 = rem.wrapping_mul(sum1);
    sum2 %= BASE;

    sum1 += (sum_b & 0xffff).wrapping_add(BASE - 1);
    sum2 += ((sum_a >> 16) & 0xffff)
        .wrapping_add((sum_b >> 16) & 0xffff)
        .wrapping_add(BASE)
        .wrapping_sub(rem);

    if sum1 >= BASE {
        sum1 = sum1.wrapping_sub(BASE);
    }
    if sum1 >= BASE {
        sum1 = sum1.wrapping_sub(BASE);
    }

    if sum2 >= (BASE << 1) {
        sum2 = sum2.wrapping_sub(BASE << 1);
    }
    if sum2 >= BASE {
        sum2 = sum2.wrapping_sub(BASE);
    }

    sum1 | (sum2 << 16)
}

#[cfg(test)]
mod tests {
    use super::adler32_combine;

    #[test]
    fn adler_combine_test() {
        const LEN_B: usize = 307320;
        let parts = [
            [0x732CBF4D_u32, 0xADC515B1_u32, 0x9F7ED4FD_u32],
            [0x9F7ED4FD_u32, 0x99AD44FE_u32, 0xD80F1A09_u32],
            [0xD80F1A09_u32, 0x67BD47A0_u32, 0x1B1261A8_u32],
        ];
        for part in parts.iter() {
            let r = adler32_combine(part[0], part[1], LEN_B);
            assert_eq!(r, part[2]);
        }
    }
}
