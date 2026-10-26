//! Sample conversion and channel routing for the audio path.

/// Append big-endian signed PCM of `bits` (8, 16, 24 or 32) to `out` as floats in [-1, 1).
pub fn decode_be(bytes: &[u8], bits: u16, out: &mut Vec<f32>) {
    match bits {
        8 => out.extend(bytes.iter().map(|&b| f32::from(b as i8) / 128.0)),
        16 => out.extend(
            bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&c| f32::from(i16::from_be_bytes(c)) / 32768.0),
        ),
        24 => out.extend(bytes.as_chunks::<3>().0.iter().map(|c| {
            let v = (i32::from(c[0] as i8) << 16) | (i32::from(c[1]) << 8) | i32::from(c[2]);
            v as f32 / 8_388_608.0
        })),
        32 => out.extend(
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|&c| i32::from_be_bytes(c) as f32 / 2_147_483_648.0),
        ),
        _ => {}
    }
}

/// Route interleaved source frames (`src_ch` channels) onto a device with `dev_ch` channels,
/// putting left/right on the channel pair starting at `pair * 2` and silence elsewhere.
///
/// Mono sources go to both sides; sources with more than two channels use their first two.
/// On a mono device the two sides are averaged. A pair that does not fit falls back to the first.
pub fn route(src: &[f32], src_ch: usize, dev_ch: usize, pair: usize, out: &mut Vec<f32>) {
    if src_ch == 0 || dev_ch == 0 {
        return;
    }
    let frames = src.len() / src_ch;
    let base = if (pair * 2 + 1) < dev_ch { pair * 2 } else { 0 };
    out.reserve(frames * dev_ch);
    for frame in src.chunks_exact(src_ch) {
        let l = frame[0];
        let r = if src_ch >= 2 { frame[1] } else { l };
        if dev_ch == 1 {
            out.push((l + r) * 0.5);
            continue;
        }
        let start = out.len();
        out.resize(start + dev_ch, 0.0);
        out[start + base] = l;
        out[start + base + 1] = r;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decoded(bytes: &[u8], bits: u16) -> Vec<f32> {
        let mut v = Vec::new();
        decode_be(bytes, bits, &mut v);
        v
    }

    #[test]
    fn sixteen_bit_endpoints_and_zero() {
        let v = decoded(&[0x00, 0x00, 0x7F, 0xFF, 0x80, 0x00, 0xFF, 0xFF], 16);
        assert_eq!(v[0], 0.0);
        assert!((v[1] - 32767.0 / 32768.0).abs() < 1e-9);
        assert_eq!(v[2], -1.0);
        assert!((v[3] + 1.0 / 32768.0).abs() < 1e-9);
    }

    #[test]
    fn twenty_four_bit_is_sign_extended() {
        let v = decoded(
            &[
                0x80, 0x00, 0x00, 0x7F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x01,
            ],
            24,
        );
        assert_eq!(v[0], -1.0);
        assert!((v[1] - 8_388_607.0 / 8_388_608.0).abs() < 1e-9);
        assert!((v[2] + 1.0 / 8_388_608.0).abs() < 1e-9);
        assert!((v[3] - 1.0 / 8_388_608.0).abs() < 1e-9);
    }

    #[test]
    fn eight_and_thirty_two_bit() {
        assert_eq!(decoded(&[0x80, 0x00, 0x40], 8), vec![-1.0, 0.0, 0.5]);
        let v = decoded(&[0x80, 0, 0, 0, 0x40, 0, 0, 0], 32);
        assert_eq!(v, vec![-1.0, 0.5]);
    }

    #[test]
    fn a_trailing_partial_sample_is_ignored_and_odd_depths_decode_nothing() {
        assert_eq!(decoded(&[0, 0, 0x40], 16).len(), 1);
        assert!(decoded(&[1, 2, 3, 4], 12).is_empty());
    }

    fn routed(src: &[f32], src_ch: usize, dev_ch: usize, pair: usize) -> Vec<f32> {
        let mut v = Vec::new();
        route(src, src_ch, dev_ch, pair, &mut v);
        v
    }

    #[test]
    fn stereo_to_stereo_is_a_copy() {
        assert_eq!(
            routed(&[0.1, 0.2, 0.3, 0.4], 2, 2, 0),
            vec![0.1, 0.2, 0.3, 0.4]
        );
    }

    #[test]
    fn mono_is_duplicated_to_both_sides() {
        assert_eq!(routed(&[0.5, -0.5], 1, 2, 0), vec![0.5, 0.5, -0.5, -0.5]);
    }

    #[test]
    fn extra_source_channels_are_dropped() {
        assert_eq!(
            routed(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 3, 2, 0),
            vec![1.0, 2.0, 4.0, 5.0]
        );
    }

    #[test]
    fn a_multichannel_device_gets_the_chosen_pair_and_silence_elsewhere() {
        assert_eq!(routed(&[0.1, 0.2], 2, 4, 0), vec![0.1, 0.2, 0.0, 0.0]);
        assert_eq!(routed(&[0.1, 0.2], 2, 4, 1), vec![0.0, 0.0, 0.1, 0.2]);
        assert_eq!(
            routed(&[0.1, 0.2], 2, 6, 2),
            vec![0.0, 0.0, 0.0, 0.0, 0.1, 0.2]
        );
    }

    #[test]
    fn a_pair_that_does_not_fit_falls_back_to_the_first() {
        assert_eq!(routed(&[0.1, 0.2], 2, 4, 5), vec![0.1, 0.2, 0.0, 0.0]);
        assert_eq!(routed(&[0.1, 0.2], 2, 3, 1), vec![0.1, 0.2, 0.0]);
    }

    #[test]
    fn a_mono_device_gets_the_average() {
        assert_eq!(routed(&[0.2, 0.4], 2, 1, 0), vec![0.3]);
    }

    #[test]
    fn degenerate_channel_counts_do_nothing() {
        assert!(routed(&[1.0], 0, 2, 0).is_empty());
        assert!(routed(&[1.0], 1, 0, 0).is_empty());
    }
}
