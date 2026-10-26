//! Minimal ID3v2.3 reader for the `ID3 ` chunk Serato appends on a clean close.

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Id3Info {
    pub title: Option<String>,
    /// Timestamp from the "Recorded ..." comment, with the prefix removed.
    pub recorded: Option<String>,
}

/// Parse the chunk body (an ID3v2 tag). Returns `None` if it is not one.
pub fn parse(tag: &[u8]) -> Option<Id3Info> {
    if tag.len() < 10 || &tag[0..3] != b"ID3" || tag[3] != 3 {
        return None;
    }
    let size = tag[6..10]
        .iter()
        .fold(0usize, |acc, b| (acc << 7) | usize::from(b & 0x7f));
    let end = (10 + size).min(tag.len());
    let mut info = Id3Info::default();
    let mut pos = 10;
    while pos + 10 <= end {
        let id = &tag[pos..pos + 4];
        if id[0] == 0 {
            break; // padding
        }
        let len = u32::from_be_bytes(tag[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let (start, stop) = (pos + 10, pos + 10 + len);
        if stop > end {
            break;
        }
        let body = &tag[start..stop];
        match id {
            b"TIT2" => info.title = decode_text(body.first().copied()?, &body[1..]),
            b"COMM" if body.len() >= 4 => info.recorded = comment_text(body),
            _ => {}
        }
        pos = stop;
    }
    Some(info)
}

fn decode_text(encoding: u8, bytes: &[u8]) -> Option<String> {
    let s = match encoding {
        0 => bytes.iter().map(|&b| b as char).collect::<String>(),
        1 | 2 => {
            let (big_endian, rest) = match bytes {
                [0xFE, 0xFF, rest @ ..] => (true, rest),
                [0xFF, 0xFE, rest @ ..] => (false, rest),
                _ => (encoding == 2, bytes),
            };
            let units: Vec<u16> = rest
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&c| {
                    if big_endian {
                        u16::from_be_bytes(c)
                    } else {
                        u16::from_le_bytes(c)
                    }
                })
                .collect();
            String::from_utf16_lossy(&units)
        }
        _ => String::from_utf8_lossy(bytes).into_owned(),
    };
    let s = s.trim_end_matches('\0').to_string();
    (!s.is_empty()).then_some(s)
}

/// COMM frame: encoding, 3-byte language, NUL-terminated description, then the text.
fn comment_text(body: &[u8]) -> Option<String> {
    let encoding = body[0];
    let rest = &body[4..];
    let wide = matches!(encoding, 1 | 2);
    let text_start = if wide {
        rest.as_chunks::<2>()
            .0
            .iter()
            .position(|&c| c == [0, 0])
            .map(|i| (i + 1) * 2)?
    } else {
        rest.iter().position(|&b| b == 0).map(|i| i + 1)?
    };
    let text = decode_text(encoding, &rest[text_start..])?;
    Some(
        text.strip_prefix("Recorded ")
            .map(str::to_string)
            .unwrap_or(text),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::id3_chunk;

    #[test]
    fn reads_title_and_recorded_time() {
        let chunk = id3_chunk("Recorded 2025-10-04 10:31:51");
        let info = parse(&chunk[8..]).unwrap();
        assert_eq!(info.title.as_deref(), Some("Serato Recording"));
        assert_eq!(info.recorded.as_deref(), Some("2025-10-04 10:31:51"));
    }

    #[test]
    fn comment_without_the_prefix_is_kept_whole() {
        let chunk = id3_chunk("hello");
        assert_eq!(
            parse(&chunk[8..]).unwrap().recorded.as_deref(),
            Some("hello")
        );
    }

    #[test]
    fn garbage_is_none() {
        assert!(parse(b"not an id3 tag at all").is_none());
        assert!(parse(b"").is_none());
    }

    #[test]
    fn truncated_frame_does_not_panic() {
        let chunk = id3_chunk("Recorded 2025-10-04 10:31:51");
        for cut in 0..chunk.len() - 8 {
            let _ = parse(&chunk[8..8 + cut]);
        }
    }
}
