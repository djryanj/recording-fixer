//! Which audio encoders an ffmpeg build offers, and which AAC encoder to prefer.

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Encoders {
    pub names: Vec<String>,
}

/// AAC encoders in order of preference: Apple's, then Fraunhofer's, then ffmpeg's own.
/// `aac_mf` (Windows Media Foundation) is last because quality varies by system.
pub const AAC_PREFERENCE: [&str; 4] = ["aac_at", "libfdk_aac", "aac", "aac_mf"];

impl Encoders {
    pub fn has(&self, name: &str) -> bool {
        self.names.iter().any(|n| n == name)
    }

    pub fn best_aac(&self) -> Option<&'static str> {
        AAC_PREFERENCE.into_iter().find(|n| self.has(n))
    }

    pub fn aac_available(&self) -> Vec<&'static str> {
        AAC_PREFERENCE.into_iter().filter(|n| self.has(n)).collect()
    }

    pub fn has_mp3(&self) -> bool {
        self.has("libmp3lame")
    }
}

/// Parse the output of `ffmpeg -hide_banner -encoders`; only audio encoders are kept.
pub fn parse_encoders(output: &str) -> Encoders {
    let names = output
        .lines()
        .skip_while(|l| !l.trim_start().starts_with("---"))
        .skip(1)
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let (flags, name) = (it.next()?, it.next()?);
            (flags.len() == 6 && flags.starts_with('A')).then(|| name.to_string())
        })
        .collect();
    Encoders { names }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Encoders:
 V..... = Video
 A..... = Audio
 ------
 V....D libx264              libx264 H.264
 A....D aac                  AAC (Advanced Audio Coding)
 A....D aac_mf               AAC via MediaFoundation (codec aac)
 A....D libmp3lame           libmp3lame MP3 (MPEG audio layer 3) (codec mp3)
 S..... srt                  SubRip subtitle
";

    #[test]
    fn keeps_only_audio_encoders() {
        let e = parse_encoders(SAMPLE);
        assert_eq!(e.names, ["aac", "aac_mf", "libmp3lame"]);
    }

    #[test]
    fn legend_lines_are_not_mistaken_for_encoders() {
        assert!(!parse_encoders(SAMPLE).has("="));
    }

    #[test]
    fn prefers_apple_then_fdk_then_native() {
        let all = parse_encoders(
            "------\n A....D aac_at x\n A....D libfdk_aac x\n A....D aac x\n A....D aac_mf x\n",
        );
        assert_eq!(all.best_aac(), Some("aac_at"));
        let no_apple = Encoders {
            names: vec!["aac".into(), "libfdk_aac".into()],
        };
        assert_eq!(no_apple.best_aac(), Some("libfdk_aac"));
        let native = parse_encoders(SAMPLE);
        assert_eq!(native.best_aac(), Some("aac"));
    }

    #[test]
    fn reports_missing_encoders() {
        let none = parse_encoders("------\n V....D libx264 x\n");
        assert_eq!(none.best_aac(), None);
        assert!(!none.has_mp3());
        assert!(parse_encoders(SAMPLE).has_mp3());
    }

    #[test]
    fn empty_or_garbage_output_yields_nothing() {
        assert!(parse_encoders("").names.is_empty());
        assert!(parse_encoders("ffmpeg: command not found").names.is_empty());
    }
}
