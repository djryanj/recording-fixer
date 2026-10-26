//! What the user can choose when exporting: codec settings and metadata.

/// Pseudo-encoder name for Apple's CoreAudio AAC encoder driven directly (Windows).
pub const APPLE_ENCODER: &str = "apple_coreaudio";

#[derive(Debug, Clone, PartialEq)]
pub enum Mp3Mode {
    /// Constant bitrate in kbps.
    Cbr(u32),
    /// LAME VBR quality, 0 (best) to 9.
    Vbr(u8),
    /// Average bitrate in kbps.
    Abr(u32),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mp3Options {
    pub mode: Mp3Mode,
    /// ID3v2 version to write: 3 (widest compatibility) or 4.
    pub id3_version: u8,
    /// Also append an ID3v1 tag for very old players.
    pub id3v1: bool,
}

impl Mp3Options {
    pub fn new(mode: Mp3Mode) -> Self {
        Mp3Options {
            mode,
            id3_version: 3,
            id3v1: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AacMode {
    Cbr(u32),
    /// Quality level 1 (smallest) to 5 (best). Each encoder maps this to its own scale.
    Vbr(u8),
}

impl AacMode {
    /// Approximate CBR bitrate for encoders without a VBR mode.
    pub fn fallback_kbps(&self) -> u32 {
        match self {
            AacMode::Cbr(k) => *k,
            AacMode::Vbr(l) => [96, 128, 192, 256, 320][usize::from((*l).clamp(1, 5)) - 1],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Codec {
    Mp3(Mp3Options),
    /// `encoder` is an ffmpeg encoder name, or [`APPLE_ENCODER`].
    Aac {
        encoder: String,
        mode: AacMode,
    },
}

impl Codec {
    pub fn extension(&self) -> &'static str {
        match self {
            Codec::Mp3(_) => "mp3",
            Codec::Aac { .. } => "m4a",
        }
    }
}

/// Tags written to the output file. Empty fields are omitted.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Metadata {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    /// Year or full date, e.g. "2025" or "2025-10-04".
    pub date: String,
    pub comment: String,
    pub track: String,
}

impl Metadata {
    /// `(ffmpeg key, value)` for each non-empty field.
    pub fn pairs(&self) -> Vec<(&'static str, &str)> {
        [
            ("title", &self.title),
            ("artist", &self.artist),
            ("album", &self.album),
            ("album_artist", &self.album_artist),
            ("genre", &self.genre),
            ("date", &self.date),
            ("comment", &self.comment),
            ("track", &self.track),
        ]
        .into_iter()
        .filter(|(_, v)| !v.trim().is_empty())
        .map(|(k, v)| (k, v.as_str()))
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_skips_blank_fields() {
        let m = Metadata {
            title: "Set".into(),
            artist: "  ".into(),
            date: "2025".into(),
            ..Default::default()
        };
        assert_eq!(m.pairs(), vec![("title", "Set"), ("date", "2025")]);
    }

    #[test]
    fn vbr_levels_fall_back_to_sensible_cbr_rates() {
        assert_eq!(AacMode::Vbr(1).fallback_kbps(), 96);
        assert_eq!(AacMode::Vbr(5).fallback_kbps(), 320);
        assert_eq!(AacMode::Vbr(99).fallback_kbps(), 320);
        assert_eq!(AacMode::Cbr(192).fallback_kbps(), 192);
    }
}
