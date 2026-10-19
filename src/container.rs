//! Media container detection from file contents. HLS formats are labeled
//! `mp4` like upstream even when their assembled segments are MPEG-TS or
//! raw AAC, so FFmpeg inputs choose demuxers from the bytes.
use std::{io::Read, path::Path};

/// The container of `bytes` as one of this port's extension names.
pub fn sniff_bytes(bytes: &[u8]) -> Option<&'static str> {
    if bytes.first() == Some(&0x47) && bytes.get(188).is_none_or(|byte| *byte == 0x47) {
        return Some("ts");
    }
    if bytes.len() >= 8
        && matches!(
            &bytes[4..8],
            b"ftyp" | b"moov" | b"moof" | b"styp" | b"sidx" | b"free" | b"mdat"
        )
    {
        return Some("mp4");
    }
    if bytes.starts_with(b"\x1a\x45\xdf\xa3") {
        return Some("webm");
    }
    // Packed audio: ID3-prefixed or bare ADTS.
    if bytes.starts_with(b"ID3")
        || (bytes.len() >= 2 && bytes[0] == 0xff && bytes[1] & 0xf6 == 0xf0)
    {
        return Some("aac");
    }
    None
}

/// The container of the file at `path`, read from its first bytes.
pub fn sniff(path: &Path) -> Option<&'static str> {
    let mut head = Vec::with_capacity(256);
    std::fs::File::open(path)
        .ok()?
        .take(256)
        .read_to_end(&mut head)
        .ok()?;
    sniff_bytes(&head)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_containers_are_recognized() {
        let mut ts = vec![0x47; 1];
        ts.resize(188, 0);
        ts.push(0x47);
        assert_eq!(sniff_bytes(&ts), Some("ts"));
        assert_eq!(sniff_bytes(b"\0\0\0\x18ftypisom"), Some("mp4"));
        assert_eq!(sniff_bytes(b"\0\0\0\x18moof...."), Some("mp4"));
        assert_eq!(sniff_bytes(b"\x1a\x45\xdf\xa3...."), Some("webm"));
        assert_eq!(sniff_bytes(b"ID3\x04..."), Some("aac"));
        assert_eq!(sniff_bytes(&[0xff, 0xf1, 0x50]), Some("aac"));
        assert_eq!(sniff_bytes(b"media bytes"), None);
    }
}
