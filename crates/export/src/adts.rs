//! ADTS framing for raw AAC-LC packets, so ffmpeg can mux them without re-encoding.

const SAMPLE_RATES: [u32; 13] = [
    96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
];

pub fn sample_rate_index(rate: u32) -> Option<u8> {
    SAMPLE_RATES
        .iter()
        .position(|&r| r == rate)
        .map(|i| i as u8)
}

/// The 7-byte ADTS header (no CRC) for an AAC-LC packet of `payload_len` bytes.
pub fn header(rate: u32, channels: u16, payload_len: usize) -> Option<[u8; 7]> {
    let sf = sample_rate_index(rate)?;
    if !(1..=7).contains(&channels) {
        return None;
    }
    let len = payload_len + 7;
    if len >= 1 << 13 {
        return None;
    }
    let profile = 1u8; // AAC-LC (audio object type 2, stored minus one)
    Some([
        0xFF,
        0xF1, // MPEG-4, layer 0, no CRC
        (profile << 6) | (sf << 2) | ((channels >> 2) as u8 & 1),
        (((channels & 3) as u8) << 6) | ((len >> 11) as u8 & 3),
        (len >> 3) as u8,
        (((len & 7) as u8) << 5) | 0x1F,
        0xFC,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_header_for_44k_stereo() {
        assert_eq!(
            header(44100, 2, 93).unwrap(),
            [0xFF, 0xF1, 0x50, 0x80, 0x0C, 0x9F, 0xFC]
        );
    }

    #[test]
    fn frame_length_covers_header_plus_payload() {
        let h = header(48000, 2, 1000).unwrap();
        let len = (usize::from(h[3] & 3) << 11) | (usize::from(h[4]) << 3) | usize::from(h[5] >> 5);
        assert_eq!(len, 1007);
        assert_eq!((h[2] >> 2) & 0xF, 3); // 48 kHz
    }

    #[test]
    fn rejects_what_adts_cannot_express() {
        assert!(header(44101, 2, 10).is_none());
        assert!(header(44100, 0, 10).is_none());
        assert!(header(44100, 2, 9000).is_none());
    }
}
