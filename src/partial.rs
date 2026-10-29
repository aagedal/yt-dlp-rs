//! Stopped (`--keep-partial-on-stop`) files: an ISO-BMFF top-level box walk
//! that trims a fragmented MP4 cut inside its last fragment and tells whether
//! a file can be read.
use std::{
    fs,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
};

/// The top-level boxes of an ISO-BMFF file.
#[derive(Debug)]
struct Walk {
    /// Complete boxes: type, start and end offsets.
    boxes: Vec<([u8; 4], u64, u64)>,
    /// The boxes end exactly at the end of the file.
    complete: bool,
}

impl Walk {
    fn has(&self, kind: &[u8; 4]) -> bool {
        self.boxes.iter().any(|(found, ..)| found == kind)
    }
}

/// Box types a top-level walk may start with (as `container::sniff`).
const FIRST: [&[u8; 4]; 7] = [
    b"ftyp", b"moov", b"moof", b"styp", b"sidx", b"free", b"mdat",
];

/// Walk `path`'s top-level boxes; `None` when it is not ISO-BMFF.
fn walk(path: &Path) -> io::Result<Option<Walk>> {
    let mut file = fs::File::open(path)?;
    let len = file.metadata()?.len();
    let mut boxes = Vec::new();
    let mut offset = 0u64;
    while offset < len {
        file.seek(SeekFrom::Start(offset))?;
        let mut header = [0u8; 16];
        let read = (&mut file).take(16).read(&mut header)?;
        if read < 8 {
            break;
        }
        let kind: [u8; 4] = header[4..8].try_into().unwrap();
        if boxes.is_empty() && !FIRST.contains(&&kind) {
            return Ok(None);
        }
        let size = u64::from(u32::from_be_bytes(header[..4].try_into().unwrap()));
        let (size, header_len) = match size {
            // `largesize` follows the type.
            1 if read < 16 => break,
            1 => (u64::from_be_bytes(header[8..16].try_into().unwrap()), 16),
            // The box extends to the end of the file.
            0 => (len - offset, 8),
            size => (size, 8),
        };
        let Some(end) = offset.checked_add(size).filter(|_| size >= header_len) else {
            break;
        };
        if end > len {
            break;
        }
        boxes.push((kind, offset, end));
        offset = end;
    }
    if boxes.is_empty() {
        return Ok(None);
    }
    Ok(Some(Walk {
        boxes,
        complete: offset == len,
    }))
}

/// Truncate a fragmented MP4 (`moov` then `moof`+`mdat` pairs) that ends in
/// an incomplete box, or in a fragment header without its data, after its
/// last complete `mdat`. FFmpeg otherwise reads past the cut and writes an
/// unreadable file (while exiting 0). Returns the bytes removed.
pub(crate) fn trim_fragmented(path: &Path) -> io::Result<Option<u64>> {
    let Some(walk) = walk(path)? else {
        return Ok(None);
    };
    if !walk.has(b"moov") || !walk.has(b"moof") {
        return Ok(None);
    }
    let ends_in_header = walk
        .boxes
        .last()
        .is_some_and(|(kind, ..)| kind != b"mdat" && kind != b"mfra");
    if walk.complete && !ends_in_header {
        return Ok(None);
    }
    let Some(&(_, _, cut)) = walk.boxes.iter().rev().find(|(kind, ..)| kind == b"mdat") else {
        return Ok(None);
    };
    let len = fs::metadata(path)?.len();
    if cut >= len {
        return Ok(None);
    }
    let file = fs::OpenOptions::new().write(true).open(path)?;
    file.set_len(cut)?;
    file.sync_all()?;
    Ok(Some(len - cut))
}

/// How a stopped MP4 can be read.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Mp4 {
    /// Not ISO-BMFF (MPEG-TS, WebM, ...).
    Other,
    /// Every top-level box is whole and there is a `moov`.
    Readable,
    /// A progressive MP4 whose up-front `moov` describes more data than the
    /// file holds: players see the full duration, then fail. A stream copy
    /// keeps the samples that are present.
    Truncated,
    /// No reachable `moov`.
    Unreadable,
}

pub(crate) fn mp4(path: &Path) -> io::Result<Mp4> {
    let Some(walk) = walk(path)? else {
        return Ok(Mp4::Other);
    };
    Ok(if !walk.has(b"moov") {
        Mp4::Unreadable
    } else if walk.complete {
        Mp4::Readable
    } else if walk.has(b"moof") {
        // Trimmed by `trim_fragmented` first; whole fragments read fine.
        Mp4::Unreadable
    } else {
        Mp4::Truncated
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atom(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }

    fn write(bytes: &[u8]) -> tempfile::NamedTempFile {
        let file = tempfile::NamedTempFile::new().unwrap();
        fs::write(file.path(), bytes).unwrap();
        file
    }

    fn fragmented() -> (Vec<u8>, usize) {
        let mut bytes = [
            atom(b"ftyp", b"isom\0\0\x02\0"),
            atom(b"moov", &[0; 16]),
            atom(b"sidx", &[0; 8]),
            atom(b"moof", &[1; 12]),
            atom(b"mdat", &[2; 40]),
        ]
        .concat();
        let whole = bytes.len();
        bytes.extend(atom(b"moof", &[3; 12]));
        bytes.extend(atom(b"mdat", &[4; 40]));
        (bytes, whole)
    }

    #[test]
    fn a_fragmented_mp4_cut_mid_fragment_is_trimmed_to_its_last_whole_mdat() {
        let (bytes, whole) = fragmented();
        let file = write(&bytes[..bytes.len() - 7]);
        assert_eq!(mp4(file.path()).unwrap(), Mp4::Unreadable);
        assert_eq!(
            trim_fragmented(file.path()).unwrap(),
            Some((bytes.len() - 7 - whole) as u64)
        );
        assert_eq!(fs::read(file.path()).unwrap(), bytes[..whole]);
        assert_eq!(mp4(file.path()).unwrap(), Mp4::Readable);
        // Already whole: unchanged.
        assert_eq!(trim_fragmented(file.path()).unwrap(), None);
    }

    #[test]
    fn a_trailing_fragment_header_without_data_is_trimmed() {
        let (bytes, whole) = fragmented();
        let file = write(&bytes[..whole + 20]);
        trim_fragmented(file.path()).unwrap();
        assert_eq!(fs::read(file.path()).unwrap(), bytes[..whole]);
        // A header cut inside its 8 bytes, too.
        let file = write(&bytes[..whole + 3]);
        trim_fragmented(file.path()).unwrap();
        assert_eq!(fs::read(file.path()).unwrap(), bytes[..whole]);
    }

    #[test]
    fn progressive_and_other_files_are_not_trimmed() {
        let progressive = [
            atom(b"ftyp", b"isom\0\0\x02\0"),
            atom(b"moov", &[0; 16]),
            atom(b"mdat", &[2; 40]),
        ]
        .concat();
        let file = write(&progressive[..progressive.len() - 10]);
        assert_eq!(trim_fragmented(file.path()).unwrap(), None);
        assert_eq!(mp4(file.path()).unwrap(), Mp4::Truncated);
        let file = write(&progressive);
        assert_eq!(mp4(file.path()).unwrap(), Mp4::Readable);
        let file = write(b"\x1a\x45\xdf\xa3webm");
        assert_eq!(trim_fragmented(file.path()).unwrap(), None);
        assert_eq!(mp4(file.path()).unwrap(), Mp4::Other);
    }

    #[test]
    fn a_merge_written_from_a_cut_fragment_is_unreadable() {
        // FFmpeg's output in the report: `ftyp`, then an `mdat` whose 64-bit
        // size points past the end, and no `moov`.
        let mut bytes = atom(b"ftyp", b"isom\0\0\x02\0");
        bytes.extend(1u32.to_be_bytes());
        bytes.extend(b"mdat");
        bytes.extend(0xFFFF_FFFF_CCF9_6DBAu64.to_be_bytes());
        bytes.extend([0; 64]);
        let file = write(&bytes);
        assert_eq!(mp4(file.path()).unwrap(), Mp4::Unreadable);
    }
}
