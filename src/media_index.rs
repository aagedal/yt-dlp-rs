//! Segment indexes of single-file adaptive streams (YouTube's `initRange`
//! and `indexRange`): an MP4 `sidx` box or WebM `Cues`. A partial download
//! fetches the initialization bytes and only the segments covering a time
//! range, and the result stays a valid, seekable file for FFmpeg to cut
//! locally. This is a port extension; upstream reads the whole URL with FFmpeg.
use crate::info::ByteIndex;

/// One independently decodable segment: inclusive file byte offsets and its
/// start time in seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Segment {
    pub start: u64,
    pub end: u64,
    pub time: f64,
}

/// One WebM cue point: its time in timecode units and each track's
/// original cluster position (relative to the Segment data).
#[derive(Debug, Clone, PartialEq)]
struct CuePoint {
    time: u64,
    positions: Vec<(u64, u64)>,
}

/// What precedes the selected segments in the shortened file.
#[derive(Debug, Clone, PartialEq)]
enum Header {
    /// MP4 `ftyp`+`moov`, unchanged: fragments carry their own decode times.
    Mp4(Vec<u8>),
    /// WebM: the EBML header and the Segment's top-level elements before the
    /// clusters (without SeekHead, Cues and Void), and the cue points.
    WebM {
        ebml: Vec<u8>,
        elements: Vec<u8>,
        cue_points: Vec<CuePoint>,
        /// The original Segment data offset.
        data: u64,
    },
}

/// A parsed index: every segment in file order and what the shortened file
/// starts with.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct IndexedMedia {
    pub segments: Vec<Segment>,
    header: Header,
}

impl IndexedMedia {
    /// The original file's start time (FFmpeg's `start_time`), which a
    /// relative `-ss` is measured from.
    pub fn start_time(&self) -> f64 {
        self.segments.first().map_or(0.0, |segment| segment.time)
    }

    /// The inclusive segment indices covering `start..end` seconds of the
    /// original file's timeline: from the last segment starting at or before
    /// `start`, to one segment past `end` as a margin for reordered frames.
    pub fn select(&self, start: f64, end: Option<f64>) -> (usize, usize) {
        let last = self.segments.len().saturating_sub(1);
        let first = self
            .segments
            .iter()
            .rposition(|segment| segment.time <= start)
            .unwrap_or(0);
        let through = end
            .and_then(|end| self.segments.iter().position(|segment| segment.time >= end))
            .unwrap_or(last)
            .max(first);
        (first, through)
    }

    /// The bytes to write before segments `first..=last`. A WebM file gets
    /// a SeekHead and Cues for the kept clusters at their new positions, so
    /// FFmpeg seeks to the same cue point as in the original file.
    pub fn header(&self, first: usize, last: usize) -> Vec<u8> {
        let (ebml, elements, cue_points, data) = match &self.header {
            Header::Mp4(init) => return init.clone(),
            Header::WebM {
                ebml,
                elements,
                cue_points,
                data,
            } => (ebml, elements, cue_points, *data),
        };
        let (start, end) = (
            self.segments[first].start - data,
            self.segments[last].end - data,
        );
        let kept: Vec<&CuePoint> = cue_points
            .iter()
            .filter(|point| {
                point
                    .positions
                    .iter()
                    .all(|(_, position)| (start..=end).contains(position))
            })
            .collect();
        // Fixed-width numbers: lengths do not depend on the positions.
        let cues = |shift: u64| {
            let points: Vec<u8> = kept
                .iter()
                .flat_map(|point| {
                    let mut body = ebml_element(&[0xb3], &point.time.to_be_bytes());
                    for (track, position) in &point.positions {
                        let positions = [
                            ebml_element(&[0xf7], &track.to_be_bytes()),
                            ebml_element(&[0xf1], &(position - start + shift).to_be_bytes()),
                        ]
                        .concat();
                        body.extend(ebml_element(&[0xb7], &positions));
                    }
                    ebml_element(&[0xbb], &body)
                })
                .collect();
            ebml_element(&CUES.to_be_bytes(), &points)
        };
        let seek_head = |cues_at: u64| {
            let seek = [
                ebml_element(&[0x53, 0xab], &CUES.to_be_bytes()),
                ebml_element(&[0x53, 0xac], &cues_at.to_be_bytes()),
            ]
            .concat();
            ebml_element(
                &SEEK_HEAD.to_be_bytes(),
                &ebml_element(&[0x4d, 0xbb], &seek),
            )
        };
        let cues_at = (seek_head(0).len() + elements.len()) as u64;
        let clusters_at = cues_at + cues(0).len() as u64;
        let body = [seek_head(cues_at), elements.clone(), cues(clusters_at)].concat();
        let size = body.len() as u64 + end + 1 - start;
        let mut header = ebml.clone();
        header.extend(SEGMENT.to_be_bytes());
        header.push(0x01);
        header.extend(&size.to_be_bytes()[1..]);
        header.extend(body);
        header
    }
}

/// Parse the index from `head`, the file bytes starting at offset `offset`
/// that contain both ranges. `total` is the file length, needed for the
/// end of the last WebM cluster. `None` means the index is unusable.
pub(crate) fn parse(
    head: &[u8],
    offset: u64,
    ranges: ByteIndex,
    total: Option<u64>,
) -> Option<IndexedMedia> {
    let slice = |(start, end): (u64, u64)| {
        let from = usize::try_from(start.checked_sub(offset)?).ok()?;
        let to = usize::try_from(end.checked_sub(offset)?).ok()?;
        head.get(from..=to)
    };
    let init = slice(ranges.init)?;
    let index = slice(ranges.index)?;
    if ranges.init.0 != 0 {
        return None;
    }
    let media = if init.get(4..8).is_some_and(|kind| kind == b"ftyp") {
        IndexedMedia {
            segments: sidx(index, ranges.index.0)?,
            header: Header::Mp4(init.to_vec()),
        }
    } else if init.starts_with(&EBML_HEADER.to_be_bytes()) {
        webm(init, index, ranges.index.0, total)?
    } else {
        return None;
    };
    let ordered = media
        .segments
        .windows(2)
        .all(|pair| pair[0].end < pair[1].start && pair[0].time <= pair[1].time);
    (ordered && !media.segments.is_empty() && media.segments[0].start > ranges.init.1)
        .then_some(media)
}

fn be(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .fold(0, |value, byte| (value << 8) | u64::from(*byte))
}

/// The `sidx` box in `index` (at file offset `offset`): its references,
/// anchored at the first byte after the box.
fn sidx(index: &[u8], offset: u64) -> Option<Vec<Segment>> {
    let mut position = 0usize;
    let (body, end) = loop {
        let header = index.get(position..position + 8)?;
        let (size, header_len) = match be(&header[..4]) {
            1 => (be(index.get(position + 8..position + 16)?), 16),
            size => (size, 8),
        };
        let size = usize::try_from(size)
            .ok()
            .filter(|size| *size >= header_len)?;
        let end = position.checked_add(size)?;
        if &header[4..8] == b"sidx" {
            break (index.get(position + header_len..end)?, end);
        }
        position = end;
    };
    let version = *body.first()?;
    let timescale = be(body.get(8..12)?);
    let (earliest, first_offset, rest) = match version {
        0 => (be(body.get(12..16)?), be(body.get(16..20)?), 20),
        1 => (be(body.get(12..20)?), be(body.get(20..28)?), 28),
        _ => return None,
    };
    if timescale == 0 {
        return None;
    }
    let count = be(body.get(rest + 2..rest + 4)?) as usize;
    let mut start = offset + end as u64 + first_offset;
    let mut time = earliest;
    let mut segments = Vec::with_capacity(count);
    for reference in body.get(rest + 4..)?.as_chunks::<12>().0.iter().take(count) {
        let size = be(&reference[..4]);
        // A reference to another `sidx` (hierarchical index) is unsupported.
        if size & 0x8000_0000 != 0 || size == 0 {
            return None;
        }
        segments.push(Segment {
            start,
            end: start + size - 1,
            time: time as f64 / timescale as f64,
        });
        start += size;
        time += be(&reference[4..8]);
    }
    (segments.len() == count).then_some(segments)
}

const EBML_HEADER: u32 = 0x1a45_dfa3;
const SEGMENT: u32 = 0x1853_8067;
const SEEK_HEAD: u32 = 0x114d_9b74;
const INFO: u32 = 0x1549_a966;
const TIMECODE_SCALE: u32 = 0x2a_d7b1;
const CUES: u32 = 0x1c53_bb6b;
const CUE_POINT: u32 = 0xbb;
const CUE_TIME: u32 = 0xb3;
const CUE_TRACK_POSITIONS: u32 = 0xb7;
const CUE_TRACK: u32 = 0xf7;
const CUE_CLUSTER_POSITION: u32 = 0xf1;
const VOID: u32 = 0xec;
const CLUSTER: u32 = 0x1f43_b675;

/// An element with an 8-byte size field.
fn ebml_element(id: &[u8], body: &[u8]) -> Vec<u8> {
    let mut bytes = id.to_vec();
    bytes.push(0x01);
    bytes.extend_from_slice(&(body.len() as u64).to_be_bytes()[1..]);
    bytes.extend_from_slice(body);
    bytes
}

/// An EBML variable-length integer at `position`: its value (with the
/// length marker for IDs) and length.
fn vint(bytes: &[u8], position: usize, keep_marker: bool) -> Option<(u64, usize)> {
    let first = *bytes.get(position)?;
    let length = first.leading_zeros() as usize + 1;
    if length > 8 {
        return None;
    }
    let marker = if keep_marker {
        first
    } else {
        first & 0xffu8.checked_shr(length as u32).unwrap_or(0)
    };
    let rest = bytes.get(position + 1..position + length)?;
    Some((
        rest.iter().fold(u64::from(marker), |value, byte| {
            (value << 8) | u64::from(*byte)
        }),
        length,
    ))
}

/// One element: its ID, start, data offset and end (sizes must be known).
struct Element {
    id: u32,
    start: usize,
    data: usize,
    end: usize,
}

fn element(bytes: &[u8], position: usize) -> Option<Element> {
    let (id, id_len) = vint(bytes, position, true)?;
    if id_len > 4 {
        return None;
    }
    let (size, size_len) = vint(bytes, position + id_len, false)?;
    // An unknown size (every value bit set) is unusable here.
    if size == (1 << (7 * size_len)) - 1 {
        return None;
    }
    let data = position + id_len + size_len;
    Some(Element {
        id: id as u32,
        start: position,
        data,
        end: data.checked_add(usize::try_from(size).ok()?)?,
    })
}

/// The child elements of `bytes[start..end]`.
fn children(bytes: &[u8], start: usize, end: usize) -> impl Iterator<Item = Element> + '_ {
    let mut position = start;
    std::iter::from_fn(move || {
        if position >= end {
            return None;
        }
        let element = element(bytes, position).filter(|element| element.end <= end)?;
        position = element.end;
        Some(element)
    })
}

/// WebM: the clusters from `Cues`, and the top-level elements to keep.
fn webm(init: &[u8], index: &[u8], index_offset: u64, total: Option<u64>) -> Option<IndexedMedia> {
    let header = element(init, 0)?;
    let ebml = init.get(..header.end)?.to_vec();
    // The Segment's size may be unknown; its children are read up to the
    // initialization's end.
    let (id, id_len) = vint(init, header.end, true)?;
    let (_, size_len) = vint(init, header.end + id_len, false)?;
    if id != u64::from(SEGMENT) {
        return None;
    }
    let data = header.end + id_len + size_len;
    let mut elements = Vec::new();
    let mut scale = 1_000_000u64;
    for child in children(init, data, init.len()) {
        match child.id {
            SEEK_HEAD | VOID | CUES => continue,
            CLUSTER => return None,
            INFO => {
                if let Some(value) = children(init, child.data, child.end)
                    .find(|element| element.id == TIMECODE_SCALE)
                {
                    scale = be(&init[value.data..value.end]);
                }
            }
            _ => {}
        }
        elements.extend_from_slice(&init[child.start..child.end]);
    }
    if scale == 0 {
        return None;
    }
    let cues = element(index, 0).filter(|cues| cues.id == CUES && cues.end <= index.len())?;
    let mut cue_points = Vec::new();
    for point in children(index, cues.data, cues.end) {
        if point.id != CUE_POINT {
            continue;
        }
        let mut time = None;
        let mut positions = Vec::new();
        for child in children(index, point.data, point.end) {
            match child.id {
                CUE_TIME => time = Some(be(&index[child.data..child.end])),
                CUE_TRACK_POSITIONS => {
                    let mut track = None;
                    let mut position = None;
                    for value in children(index, child.data, child.end) {
                        match value.id {
                            CUE_TRACK => track = Some(be(&index[value.data..value.end])),
                            CUE_CLUSTER_POSITION => {
                                position = Some(be(&index[value.data..value.end]));
                            }
                            _ => {}
                        }
                    }
                    positions.push((track?, position?));
                }
                _ => {}
            }
        }
        if positions.is_empty() {
            return None;
        }
        cue_points.push(CuePoint {
            time: time?,
            positions,
        });
    }
    // Segments start at each first track's cluster; cue points of several
    // tracks may share one.
    let base = data as u64;
    let mut starts: Vec<(u64, f64)> = cue_points
        .iter()
        .map(|point| {
            (
                base + point.positions[0].1,
                point.time as f64 * scale as f64 / 1e9,
            )
        })
        .collect();
    starts.dedup_by_key(|(start, _)| *start);
    let total = total?;
    let segments = starts
        .iter()
        .enumerate()
        .map(|(index, (start, time))| {
            let end = starts.get(index + 1).map_or(total, |(next, _)| *next);
            Some(Segment {
                start: *start,
                end: end.checked_sub(1)?,
                time: *time,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    // The clusters follow the index.
    let index_end = index_offset + cues.end as u64;
    segments
        .first()
        .is_some_and(|first| first.start >= index_end)
        .then_some(IndexedMedia {
            segments,
            header: Header::WebM {
                ebml,
                elements,
                cue_points,
                data: base,
            },
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mp4_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut bytes = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(kind);
        bytes.extend_from_slice(body);
        bytes
    }

    /// `ftyp`+`moov`, then a version 0 `sidx` (timescale 1000, first
    /// presentation at 80 ms) of references of 100, 200 and 300 bytes.
    fn mp4() -> (Vec<u8>, ByteIndex) {
        let mut file = mp4_box(b"ftyp", b"iso5\0\0\0\0");
        file.extend(mp4_box(b"moov", &[0; 16]));
        let init_end = file.len() as u64 - 1;
        let mut sidx = vec![0, 0, 0, 0, 0, 0, 0, 1];
        sidx.extend(1000u32.to_be_bytes());
        sidx.extend(80u32.to_be_bytes());
        sidx.extend(0u32.to_be_bytes());
        sidx.extend([0, 0, 0, 3]);
        for size in [100u32, 200, 300] {
            sidx.extend(size.to_be_bytes());
            sidx.extend(2000u32.to_be_bytes());
            sidx.extend(0x9000_0000u32.to_be_bytes());
        }
        file.extend(mp4_box(b"sidx", &sidx));
        let index = (init_end + 1, file.len() as u64 - 1);
        (
            file,
            ByteIndex {
                init: (0, init_end),
                index,
            },
        )
    }

    #[test]
    fn sidx_references_become_segments() {
        let (file, ranges) = mp4();
        let media = parse(&file, 0, ranges, None).unwrap();
        let first = ranges.index.1 + 1;
        let segment = |start, end, time| Segment { start, end, time };
        assert_eq!(
            media.segments,
            [
                segment(first, first + 99, 0.08),
                segment(first + 100, first + 299, 2.08),
                segment(first + 300, first + 599, 4.08),
            ]
        );
        assert_eq!(media.start_time(), 0.08);
        // The MP4 initialization is kept as is.
        assert_eq!(media.header(1, 2), file[..=ranges.init.1 as usize]);
        // The head must contain the initialization.
        assert_eq!(parse(&file[4..], 4, ranges, None), None);
        // A hierarchical reference is unusable.
        let mut nested = file.clone();
        nested[ranges.index.0 as usize + 8 + 24] |= 0x80;
        assert_eq!(parse(&nested, 0, ranges, None), None);
    }

    #[test]
    fn selection_covers_the_range_with_a_margin() {
        let segments = (0..10)
            .map(|index| Segment {
                start: index * 10,
                end: index * 10 + 9,
                time: index as f64 * 2.0,
            })
            .collect();
        let media = IndexedMedia {
            segments,
            header: Header::Mp4(Vec::new()),
        };
        assert_eq!(media.select(5.0, Some(9.0)), (2, 5));
        assert_eq!(media.select(4.0, Some(8.0)), (2, 4));
        assert_eq!(media.select(0.0, Some(1.0)), (0, 1));
        assert_eq!(media.select(17.0, None), (8, 9));
        assert_eq!(media.select(30.0, Some(40.0)), (9, 9));
    }

    fn short(id: &[u8], body: &[u8]) -> Vec<u8> {
        let mut bytes = id.to_vec();
        bytes.push(0x80 | body.len() as u8);
        bytes.extend_from_slice(body);
        bytes
    }

    /// A WebM file with a SeekHead, Void, Info (1 ms timecodes), Tracks and
    /// Cues (two tracks per point) before four clusters of 50, 70, 90 and
    /// 110 bytes at 0, 5, 10 and 15 s. Returns the file, its ranges and the
    /// clusters' offsets.
    fn webm() -> (Vec<u8>, ByteIndex, Vec<usize>) {
        let mut file = ebml_element(&EBML_HEADER.to_be_bytes(), &short(&[0x42, 0x82], b"webm"));
        let segment_at = file.len();
        let info = short(
            &[0x15, 0x49, 0xa9, 0x66],
            &short(&[0x2a, 0xd7, 0xb1], &[0x0f, 0x42, 0x40]),
        );
        let tracks = short(&[0x16, 0x54, 0xae, 0x6b], &[0xae, 0x80]);
        let before = [
            ebml_element(&SEEK_HEAD.to_be_bytes(), &[0xaa; 6]),
            short(&[0xec], &[0; 3]),
            info,
            tracks,
        ]
        .concat();
        let sizes = [50usize, 70, 90, 110];
        let cues = |first: u64| {
            let mut position = first;
            let points: Vec<u8> = sizes
                .iter()
                .enumerate()
                .flat_map(|(index, size)| {
                    let track = |number: u8| {
                        ebml_element(
                            &[0xb7],
                            &[
                                short(&[0xf7], &[number]),
                                ebml_element(&[0xf1], &position.to_be_bytes()),
                            ]
                            .concat(),
                        )
                    };
                    let point = ebml_element(
                        &[0xbb],
                        &[
                            short(&[0xb3], &(index as u16 * 5000).to_be_bytes()),
                            track(1),
                            track(2),
                        ]
                        .concat(),
                    );
                    position += *size as u64;
                    point
                })
                .collect();
            ebml_element(&CUES.to_be_bytes(), &points)
        };
        let cues_len = cues(0).len();
        let first = (before.len() + cues_len) as u64;
        let clusters: Vec<u8> = sizes
            .iter()
            .enumerate()
            .flat_map(|(index, size)| {
                ebml_element(&CLUSTER.to_be_bytes(), &vec![index as u8 + 1; size - 12])
            })
            .collect();
        let body = [before.clone(), cues(first), clusters].concat();
        // A YouTube-like unknown Segment size.
        file.extend(SEGMENT.to_be_bytes());
        file.extend([0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
        file.extend(body);
        let data = segment_at + 12;
        let init_end = data + before.len() - 1;
        let ranges = ByteIndex {
            init: (0, init_end as u64),
            index: (init_end as u64 + 1, (init_end + cues_len) as u64),
        };
        let mut offsets = vec![data + first as usize];
        for size in sizes {
            offsets.push(offsets.last().unwrap() + size);
        }
        (file, ranges, offsets)
    }

    #[test]
    fn webm_cues_become_segments() {
        let (file, ranges, offsets) = webm();
        assert_eq!(parse(&file, 0, ranges, None), None, "needs the length");
        let media = parse(&file, 0, ranges, Some(file.len() as u64)).unwrap();
        let expected: Vec<_> = (0..4)
            .map(|index| Segment {
                start: offsets[index] as u64,
                end: offsets[index + 1] as u64 - 1,
                time: index as f64 * 5.0,
            })
            .collect();
        assert_eq!(media.segments, expected);
        assert_eq!(media.select(7.0, Some(9.0)), (1, 2));
    }

    #[test]
    fn shortened_webm_files_index_their_kept_clusters() {
        let (file, ranges, offsets) = webm();
        let media = parse(&file, 0, ranges, Some(file.len() as u64)).unwrap();
        let header = media.header(1, 2);
        let shortened = [&header[..], &file[offsets[1]..offsets[3]]].concat();
        // The EBML header, then a Segment sized to the end of the file.
        let ebml = element(&file, 0).unwrap().end;
        assert_eq!(shortened[..ebml], file[..ebml]);
        let segment = element(&shortened, ebml).unwrap();
        assert_eq!((segment.id, segment.end), (SEGMENT, shortened.len()));
        // A SeekHead pointing at the Cues, Info and Tracks, without the Void.
        let top: Vec<_> = children(&shortened, segment.data, segment.end)
            .map(|element| element.id)
            .collect();
        assert_eq!(top, [SEEK_HEAD, INFO, 0x1654_ae6b, CUES, CLUSTER, CLUSTER]);
        let cues_at = children(&shortened, segment.data, segment.end)
            .find(|element| element.id == CUES)
            .unwrap();
        let seek_position = shortened[segment.data..]
            .windows(2)
            .position(|window| window == [0x53, 0xac])
            .unwrap()
            + segment.data;
        let position = element(&shortened, seek_position).unwrap();
        assert_eq!(
            be(&shortened[position.data..position.end]) as usize + segment.data,
            cues_at.start
        );
        // Parsing the shortened file finds the kept clusters at their new
        // offsets with their original times.
        let reparsed = parse(
            &shortened,
            0,
            ByteIndex {
                init: (0, cues_at.start as u64 - 1),
                index: (cues_at.start as u64, cues_at.end as u64 - 1),
            },
            Some(shortened.len() as u64),
        )
        .unwrap();
        let kept = cues_at.end as u64;
        assert_eq!(
            reparsed.segments,
            [
                Segment {
                    start: kept,
                    end: kept + 69,
                    time: 5.0
                },
                Segment {
                    start: kept + 70,
                    end: kept + 159,
                    time: 10.0
                },
            ]
        );
        assert_eq!(
            shortened[kept as usize..kept as usize + 4],
            CLUSTER.to_be_bytes()
        );
    }
}
