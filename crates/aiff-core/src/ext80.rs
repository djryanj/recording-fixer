//! 80-bit IEEE extended floats, the sample-rate encoding used by the AIFF COMM chunk.

pub fn decode(b: &[u8; 10]) -> f64 {
    let negative = b[0] & 0x80 != 0;
    let exp = ((u16::from(b[0] & 0x7f) << 8) | u16::from(b[1])) as i32;
    let mant = u64::from_be_bytes(b[2..10].try_into().unwrap());
    if exp == 0 && mant == 0 {
        return 0.0;
    }
    let v = mant as f64 * 2f64.powi(exp - 16383 - 63);
    if negative {
        -v
    } else {
        v
    }
}

pub fn encode(value: f64) -> [u8; 10] {
    let mut out = [0u8; 10];
    if value == 0.0 || !value.is_finite() {
        return out;
    }
    let abs = value.abs();
    let exp = abs.log2().floor() as i32;
    let mant = (abs / 2f64.powi(exp) * 2f64.powi(63)) as u64;
    let biased = (exp + 16383) as u16;
    out[0] = (biased >> 8) as u8 | if value < 0.0 { 0x80 } else { 0 };
    out[1] = biased as u8;
    out[2..10].copy_from_slice(&mant.to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_the_bytes_serato_writes() {
        assert_eq!(decode(&[0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]), 44100.0);
        assert_eq!(decode(&[0x40, 0x0E, 0xBB, 0x80, 0, 0, 0, 0, 0, 0]), 48000.0);
    }

    #[test]
    fn round_trips_common_rates() {
        for rate in [
            8000.0, 22050.0, 44100.0, 48000.0, 88200.0, 96000.0, 192000.0,
        ] {
            assert_eq!(decode(&encode(rate)), rate);
        }
    }

    #[test]
    fn zero_is_zero() {
        assert_eq!(decode(&[0; 10]), 0.0);
        assert_eq!(encode(0.0), [0; 10]);
    }
}
