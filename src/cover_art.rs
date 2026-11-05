//! Native cover art for Ogg (Opus, Vorbis) and FLAC files: what upstream's
//! `EmbedThumbnailPP` does through mutagen (`Picture`, `METADATA_BLOCK_PICTURE`
//! and `FLAC.add_picture`), since FFmpeg cannot embed pictures there.
//!
//! Each embedder reads the media file's metadata and streams the rest from
//! input to output, so a large file is never held in memory; the slice
//! forms are for tests and fuzzing.
use crate::{Error, Result};
use base64::Engine;
use std::io::{Read, Seek, SeekFrom, Write};

fn invalid(message: &str) -> Error {
    Error::Postprocess(format!("Unable to embed the thumbnail: {message}"))
}

/// Fill `buffer` from `input`; fewer bytes only at the end of the input.
fn read_full(input: &mut impl Read, buffer: &mut [u8]) -> Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match input.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(count) => filled += count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(filled)
}

/// Exactly `length` bytes of `input` (allocated as they arrive), or
/// `truncated`.
fn read_exact_vec(input: &mut impl Read, length: u64, truncated: &str) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    input.take(length).read_to_end(&mut data)?;
    if data.len() as u64 != length {
        return Err(invalid(truncated));
    }
    Ok(data)
}

/// `imghdr.what` for the formats a converted thumbnail can have (`None`
/// otherwise, which upstream formats into the MIME type as is).
pub(crate) fn image_type(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if data
        .get(6..10)
        .is_some_and(|marker| marker == b"JFIF" || marker == b"Exif")
        || data.starts_with(b"\xff\xd8\xff\xdb")
    {
        Some("jpeg")
    } else {
        None
    }
}

/// The image's size (what FFmpeg reports for `_get_thumbnail_resolution`):
/// a PNG's `IHDR` or a JPEG's start-of-frame segment.
pub(crate) fn image_size(data: &[u8]) -> Option<(u32, u32)> {
    let be32 = |at: usize| Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?));
    let be16 = |at: usize| {
        Some(u32::from(u16::from_be_bytes(
            data.get(at..at + 2)?.try_into().ok()?,
        )))
    };
    if data.starts_with(b"\x89PNG\r\n\x1a\n") && data.get(12..16) == Some(b"IHDR") {
        return Some((be32(16)?, be32(20)?));
    }
    if !data.starts_with(b"\xff\xd8") {
        return None;
    }
    let mut at = 2;
    while at + 4 <= data.len() {
        if data[at] != 0xff {
            return None;
        }
        let marker = data[at + 1];
        if marker == 0xff {
            at += 1;
            continue;
        }
        let length = be16(at + 2)? as usize;
        // SOF0..SOF15, except DHT (C4), JPG (C8) and DAC (CC).
        if (0xc0..=0xcf).contains(&marker) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
            return Some((be16(at + 7)?, be16(at + 5)?));
        }
        at += 2 + length;
    }
    None
}

/// mutagen `Picture.write()`: a FLAC picture block body (front cover).
pub(crate) fn picture(data: &[u8], mime: &str, size: Option<(u32, u32)>) -> Vec<u8> {
    let (width, height) = size.unwrap_or((0, 0));
    let mut block = Vec::with_capacity(data.len() + 64);
    block.extend(3u32.to_be_bytes());
    block.extend((mime.len() as u32).to_be_bytes());
    block.extend(mime.as_bytes());
    block.extend(0u32.to_be_bytes());
    for value in [width, height, 0, 0, data.len() as u32] {
        block.extend(value.to_be_bytes());
    }
    block.extend(data);
    block
}

/// `FLAC.add_picture` and `save()`: the picture is the last metadata block
/// before any padding.
#[cfg(any(test, fuzzing))]
pub(crate) fn embed_flac(file: &[u8], picture: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(file.len() + picture.len() + 4);
    embed_flac_into(&mut std::io::Cursor::new(file), &mut out, picture)?;
    Ok(out)
}

/// [`embed_flac`] from `input` to `out`: the metadata blocks are read, the
/// audio frames copied.
pub(crate) fn embed_flac_into(
    input: &mut impl Read,
    out: &mut impl Write,
    picture: &[u8],
) -> Result<()> {
    let mut magic = [0u8; 4];
    if read_full(input, &mut magic)? != 4 || &magic != b"fLaC" {
        return Err(invalid("not a FLAC file"));
    }
    let mut blocks: Vec<(u8, Vec<u8>)> = Vec::new();
    loop {
        let mut header = [0u8; 4];
        if read_full(input, &mut header)? != 4 {
            return Err(invalid("truncated FLAC metadata"));
        }
        let last = header[0] & 0x80 != 0;
        let kind = header[0] & 0x7f;
        let length = u32::from_be_bytes([0, header[1], header[2], header[3]]);
        let body = read_exact_vec(input, u64::from(length), "truncated FLAC metadata")?;
        blocks.push((kind, body));
        if last {
            break;
        }
    }
    if picture.len() >= 1 << 24 {
        return Err(invalid("the thumbnail is too large for a FLAC picture"));
    }
    let padding = blocks.iter().position(|(kind, _)| *kind == 1);
    let index = padding.unwrap_or(blocks.len());
    blocks.insert(index, (6, picture.to_vec()));
    out.write_all(b"fLaC")?;
    let count = blocks.len();
    for (position, (kind, body)) in blocks.into_iter().enumerate() {
        let last = if position + 1 == count { 0x80 } else { 0 };
        out.write_all(&[last | kind])?;
        out.write_all(&(body.len() as u32).to_be_bytes()[1..])?;
        out.write_all(&body)?;
    }
    std::io::copy(input, out)?;
    Ok(())
}

/// The Ogg page checksum (polynomial 0x04C11DB7, unreflected, zero start).
fn ogg_crc(page: &[u8]) -> u32 {
    static TABLE: std::sync::LazyLock<[u32; 256]> = std::sync::LazyLock::new(|| {
        let mut table = [0u32; 256];
        for (index, entry) in table.iter_mut().enumerate() {
            let mut value = (index as u32) << 24;
            for _ in 0..8 {
                value = if value & 0x8000_0000 != 0 {
                    (value << 1) ^ 0x04c1_1db7
                } else {
                    value << 1
                };
            }
            *entry = value;
        }
        table
    });
    page.iter().fold(0u32, |crc, byte| {
        (crc << 8) ^ TABLE[(((crc >> 24) as u8) ^ byte) as usize]
    })
}

struct Page {
    /// The page as read: its 27-byte header, lacing values and body.
    header: [u8; 27],
    header_type: u8,
    granule: [u8; 8],
    serial: u32,
    sequence: u32,
    lacing: Vec<u8>,
    body: Vec<u8>,
}

/// The next Ogg page of `input`; `None` at its end.
fn read_page(input: &mut impl Read) -> Result<Option<Page>> {
    let mut header = [0u8; 27];
    match read_full(input, &mut header)? {
        0 => return Ok(None),
        27 => {}
        _ => return Err(invalid("truncated Ogg page")),
    }
    if &header[..4] != b"OggS" || header[4] != 0 {
        return Err(invalid("not an Ogg page"));
    }
    let lacing = read_exact_vec(input, u64::from(header[26]), "truncated Ogg page")?;
    let size: u64 = lacing.iter().map(|&value| u64::from(value)).sum();
    let body = read_exact_vec(input, size, "truncated Ogg page")?;
    Ok(Some(Page {
        header,
        header_type: header[5],
        granule: header[6..14].try_into().unwrap(),
        serial: u32::from_le_bytes(header[14..18].try_into().unwrap()),
        sequence: u32::from_le_bytes(header[18..22].try_into().unwrap()),
        lacing,
        body,
    }))
}

fn write_page(
    out: &mut Vec<u8>,
    header_type: u8,
    granule: [u8; 8],
    serial: u32,
    sequence: u32,
    lacing: &[u8],
    body: &[u8],
) {
    let start = out.len();
    out.extend(b"OggS\0");
    out.push(header_type);
    out.extend(granule);
    out.extend(serial.to_le_bytes());
    out.extend(sequence.to_le_bytes());
    out.extend([0; 4]);
    out.push(lacing.len() as u8);
    out.extend(lacing);
    out.extend(body);
    let crc = ogg_crc(&out[start..]);
    out[start + 22..start + 26].copy_from_slice(&crc.to_le_bytes());
}

/// A Vorbis comment block (after its codec signature).
struct Comments<'a> {
    vendor: &'a [u8],
    /// `KEY=value` entries.
    entries: Vec<&'a [u8]>,
    /// Whatever follows the entries.
    rest: &'a [u8],
}

fn parse_comments(data: &[u8]) -> Result<Comments<'_>> {
    let le32 = |at: usize| -> Result<usize> {
        Ok(u32::from_le_bytes(
            data.get(at..at + 4)
                .ok_or_else(|| invalid("truncated comment header"))?
                .try_into()
                .unwrap(),
        ) as usize)
    };
    let vendor_length = le32(0)?;
    let vendor = data
        .get(4..4 + vendor_length)
        .ok_or_else(|| invalid("truncated comment header"))?;
    let count = le32(4 + vendor_length)?;
    let mut at = 8 + vendor_length;
    let mut comments = Vec::new();
    for _ in 0..count {
        let length = le32(at)?;
        comments.push(
            data.get(at + 4..at + 4 + length)
                .ok_or_else(|| invalid("truncated comment header"))?,
        );
        at += 4 + length;
    }
    Ok(Comments {
        vendor,
        entries: comments,
        rest: &data[at..],
    })
}

/// `f['METADATA_BLOCK_PICTURE'] = value; f.save()` for an Ogg Opus or
/// Vorbis file: the comment header gets the base64 picture (replacing any
/// earlier one), the header pages are laced again and later pages renumbered.
#[cfg(any(test, fuzzing))]
pub(crate) fn embed_ogg(file: &[u8], picture: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(file.len() + picture.len() * 4 / 3 + 1024);
    embed_ogg_into(&mut std::io::Cursor::new(file), &mut out, picture)?;
    Ok(out)
}

/// [`embed_ogg`] from `input` to `out`: the header pages are read and laced
/// again, the audio pages renumbered one at a time.
pub(crate) fn embed_ogg_into(
    input: &mut impl Read,
    out: &mut impl Write,
    picture: &[u8],
) -> Result<()> {
    let first = read_page(input)?.ok_or_else(|| invalid("empty Ogg file"))?;
    let serial = first.serial;
    let multiplexed = || invalid("multiplexed Ogg streams are not supported");
    let (opus, headers) = if first.body.starts_with(b"OpusHead") {
        (true, 1)
    } else if first.body.starts_with(b"\x01vorbis") {
        (false, 2)
    } else {
        return Err(invalid("not an Ogg Opus or Vorbis file"));
    };
    // The packets after the identification page, up to the page completing
    // the last header packet (headers end their pages).
    let mut packets: Vec<Vec<u8>> = Vec::new();
    let mut current = Vec::new();
    loop {
        let page = read_page(input)?.ok_or_else(|| invalid("missing Ogg header packets"))?;
        if page.serial != serial {
            return Err(multiplexed());
        }
        let mut offset = 0;
        for &value in &page.lacing {
            current.extend(&page.body[offset..offset + usize::from(value)]);
            offset += usize::from(value);
            if value < 255 {
                packets.push(std::mem::take(&mut current));
            }
        }
        if packets.len() >= headers {
            if packets.len() > headers || !current.is_empty() {
                return Err(invalid("an Ogg header packet does not end its page"));
            }
            break;
        }
    }
    let signature: &[u8] = if opus { b"OpusTags" } else { b"\x03vorbis" };
    let comment = packets[0]
        .strip_prefix(signature)
        .ok_or_else(|| invalid("missing comment header"))?;
    let Comments {
        vendor,
        entries: comments,
        rest,
    } = parse_comments(comment)?;
    let value = base64::engine::general_purpose::STANDARD.encode(picture);
    let entry = format!("METADATA_BLOCK_PICTURE={value}");
    let kept: Vec<&[u8]> = comments
        .into_iter()
        .filter(|comment| {
            !comment
                .split(|&byte| byte == b'=')
                .next()
                .unwrap_or_default()
                .eq_ignore_ascii_case(b"METADATA_BLOCK_PICTURE")
        })
        .collect();
    let mut packet = signature.to_vec();
    packet.extend((vendor.len() as u32).to_le_bytes());
    packet.extend(vendor);
    packet.extend(((kept.len() + 1) as u32).to_le_bytes());
    for comment in kept
        .iter()
        .copied()
        .chain(std::iter::once(entry.as_bytes()))
    {
        packet.extend((comment.len() as u32).to_le_bytes());
        packet.extend(comment);
    }
    // Opus keeps binary data after the comments (a set first bit) and drops
    // padding; Vorbis ends with its framing bit.
    if opus {
        if rest.first().is_some_and(|byte| byte & 1 == 1) {
            packet.extend(rest);
        }
    } else {
        packet.push(1);
    }
    packets[0] = packet;
    // Lace the header packets onto pages of at most 255 segments.
    out.write_all(&first.header)?;
    out.write_all(&first.lacing)?;
    out.write_all(&first.body)?;
    let mut page = Vec::new();
    let mut sequence = first.sequence.wrapping_add(1);
    let mut lacing: Vec<u8> = Vec::new();
    let mut body: Vec<u8> = Vec::new();
    // Whether the page being filled begins with a continued packet.
    let mut continued = false;
    let mut flush =
        |page: &mut Vec<u8>, lacing: &mut Vec<u8>, body: &mut Vec<u8>, continued: bool| {
            write_page(
                page,
                u8::from(continued),
                [0; 8],
                serial,
                sequence,
                lacing,
                body,
            );
            sequence = sequence.wrapping_add(1);
            lacing.clear();
            body.clear();
        };
    for packet in &packets {
        let mut remaining: &[u8] = packet;
        let mut started = false;
        loop {
            if lacing.len() == 255 {
                flush(&mut page, &mut lacing, &mut body, continued);
                continued = started;
            }
            let take = remaining.len().min(255);
            lacing.push(take as u8);
            body.extend(&remaining[..take]);
            remaining = &remaining[take..];
            started = true;
            if take < 255 {
                break;
            }
        }
    }
    if !lacing.is_empty() {
        flush(&mut page, &mut lacing, &mut body, continued);
    }
    drop(flush);
    out.write_all(&page)?;
    // Renumber the audio pages.
    while let Some(audio) = read_page(input)? {
        if audio.serial != serial {
            return Err(multiplexed());
        }
        page.clear();
        write_page(
            &mut page,
            audio.header_type,
            audio.granule,
            serial,
            sequence,
            &audio.lacing,
            &audio.body,
        );
        out.write_all(&page)?;
        sequence = sequence.wrapping_add(1);
    }
    Ok(())
}

/// An MP4 box: its type and either children (containers on the path to
/// the `ilst` and the chunk offset tables) or its raw payload.
struct Mp4Box {
    kind: [u8; 4],
    /// A full box's version and flags (`meta`), kept before its children.
    prefix: Vec<u8>,
    children: Option<Vec<Mp4Box>>,
    payload: Vec<u8>,
}

impl Mp4Box {
    fn new(kind: &[u8; 4], prefix: Vec<u8>, children: Vec<Mp4Box>) -> Self {
        Self {
            kind: *kind,
            prefix,
            children: Some(children),
            payload: Vec::new(),
        }
    }

    fn leaf(kind: &[u8; 4], payload: Vec<u8>) -> Self {
        Self {
            kind: *kind,
            prefix: Vec::new(),
            children: None,
            payload,
        }
    }

    fn child(&mut self, kind: &[u8; 4]) -> Option<&mut Mp4Box> {
        self.children
            .as_mut()?
            .iter_mut()
            .find(|child| &child.kind == kind)
    }

    fn render(&self, out: &mut Vec<u8>) {
        let start = out.len();
        out.extend([0; 4]);
        out.extend(self.kind);
        out.extend(&self.prefix);
        match &self.children {
            Some(children) => children.iter().for_each(|child| child.render(out)),
            None => out.extend(&self.payload),
        }
        let size = (out.len() - start) as u32;
        out[start..start + 4].copy_from_slice(&size.to_be_bytes());
    }
}

/// A box of a byte range: its type, offsets and payload.
struct RawBox<'a> {
    kind: [u8; 4],
    payload: &'a [u8],
}

/// The boxes of `data`, which they must fill exactly.
fn mp4_boxes(data: &[u8]) -> Result<Vec<RawBox<'_>>> {
    let mut input = std::io::Cursor::new(data);
    let boxes = mp4_box_ranges(&mut input, data.len() as u64)?;
    Ok(boxes
        .into_iter()
        .map(|range| RawBox {
            kind: range.kind,
            payload: &data[range.payload as usize..range.end as usize],
        })
        .collect())
}

/// A box found from its header: type, start, payload start and end.
struct BoxRange {
    kind: [u8; 4],
    start: u64,
    payload: u64,
    end: u64,
}

/// The boxes of `input`'s first `length` bytes, which they must fill
/// exactly, read from their headers alone.
fn mp4_box_ranges(input: &mut (impl Read + Seek), length: u64) -> Result<Vec<BoxRange>> {
    let truncated = || invalid("truncated MP4 box");
    let mut boxes = Vec::new();
    let mut at = 0u64;
    while at + 8 <= length {
        input.seek(SeekFrom::Start(at))?;
        let mut header = [0u8; 16];
        let read = read_full(input, &mut header[..(length - at).min(16) as usize])?;
        let size = u64::from(u32::from_be_bytes(header[..4].try_into().unwrap()));
        let kind: [u8; 4] = header[4..8].try_into().unwrap();
        let (header_size, size) = match size {
            0 => (8, length - at),
            1 if read >= 16 => (16, u64::from_be_bytes(header[8..16].try_into().unwrap())),
            1 => return Err(truncated()),
            size => (8, size),
        };
        let end = at
            .checked_add(size)
            .filter(|end| *end <= length && size >= header_size)
            .ok_or_else(truncated)?;
        boxes.push(BoxRange {
            kind,
            start: at,
            payload: at + header_size,
            end,
        });
        at = end;
    }
    if at != length {
        return Err(truncated());
    }
    Ok(boxes)
}

/// How deep `moov`'s containers may nest (a real file needs about six
/// levels); deeper input would exhaust the stack.
const MP4_DEPTH: usize = 32;

/// The tree under `moov`: containers down to `ilst` and `stco`/`co64`.
fn mp4_tree(kind: [u8; 4], payload: &[u8], depth: usize) -> Result<Mp4Box> {
    let container = matches!(
        &kind,
        b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"udta" | b"meta" | b"ilst"
    );
    if !container {
        return Ok(Mp4Box::leaf(&kind, payload.to_vec()));
    }
    if depth >= MP4_DEPTH {
        return Err(invalid("MP4 boxes nested too deeply"));
    }
    // `meta` is a full box (version and flags first).
    let skip = if &kind == b"meta" { 4 } else { 0 };
    let prefix = payload
        .get(..skip)
        .ok_or_else(|| invalid("truncated MP4 box"))?
        .to_vec();
    let children = mp4_boxes(&payload[skip..])?
        .into_iter()
        .map(|raw| mp4_tree(raw.kind, raw.payload, depth + 1))
        .collect::<Result<Vec<_>>>()?;
    Ok(Mp4Box::new(&kind, prefix, children))
}

/// Shift the chunk offsets (`stco`, `co64`) past `after` by `delta`.
fn shift_offsets(node: &mut Mp4Box, after: u64, delta: i64) -> Result<()> {
    if let Some(children) = &mut node.children {
        for child in children {
            shift_offsets(child, after, delta)?;
        }
        return Ok(());
    }
    let wide = match &node.kind {
        b"stco" => false,
        b"co64" => true,
        _ => return Ok(()),
    };
    let width = if wide { 8 } else { 4 };
    let payload = &mut node.payload;
    let count = payload
        .get(4..8)
        .map(|count| u32::from_be_bytes(count.try_into().unwrap()) as usize)
        .ok_or_else(|| invalid("truncated chunk offset table"))?;
    if payload.len() < 8 + count * width {
        return Err(invalid("truncated chunk offset table"));
    }
    for index in 0..count {
        let at = 8 + index * width;
        let value = if wide {
            u64::from_be_bytes(payload[at..at + 8].try_into().unwrap())
        } else {
            u64::from(u32::from_be_bytes(payload[at..at + 4].try_into().unwrap()))
        };
        if value < after {
            continue;
        }
        let shifted = value
            .checked_add_signed(delta)
            .ok_or_else(|| invalid("chunk offset overflow"))?;
        if wide {
            payload[at..at + 8].copy_from_slice(&shifted.to_be_bytes());
        } else {
            let shifted = u32::try_from(shifted).map_err(|_| invalid("chunk offset overflow"))?;
            payload[at..at + 4].copy_from_slice(&shifted.to_be_bytes());
        }
    }
    Ok(())
}

/// mutagen `MP4.tags['covr'] = [MP4Cover(data, imageformat)]; save()`: the
/// `moov.udta.meta.ilst` cover (JPEG 13, PNG 14), creating the boxes it
/// needs, with chunk offsets moved when `moov` precedes the media data.
#[cfg(any(test, fuzzing))]
pub(crate) fn embed_mp4(file: &[u8], image: &[u8], format: u32) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(file.len() + image.len() + 64);
    embed_mp4_into(&mut std::io::Cursor::new(file), &mut out, image, format)?;
    Ok(out)
}

/// [`embed_mp4`] from `input` to `out`: only `moov` is read; the boxes
/// around it are copied.
pub(crate) fn embed_mp4_into(
    input: &mut (impl Read + Seek),
    out: &mut impl Write,
    image: &[u8],
    format: u32,
) -> Result<()> {
    let length = input.seek(SeekFrom::End(0))?;
    let boxes = mp4_box_ranges(input, length).map_err(|_| invalid("not a MP4 file"))?;
    let range = boxes
        .iter()
        .find(|range| &range.kind == b"moov")
        .ok_or_else(|| invalid("not a MP4 file"))?;
    let (moov_start, moov_end) = (range.start, range.end);
    input.seek(SeekFrom::Start(range.payload))?;
    let moov_payload = read_exact_vec(input, moov_end - range.payload, "truncated MP4 box")?;
    let mut moov = mp4_tree(*b"moov", &moov_payload, 0)?;
    let mut data = format.to_be_bytes().to_vec();
    data.extend(0u32.to_be_bytes());
    data.extend(image);
    let covr = Mp4Box::new(b"covr", Vec::new(), vec![Mp4Box::leaf(b"data", data)]);
    if moov.child(b"udta").is_none() {
        moov.children
            .as_mut()
            .unwrap()
            .push(Mp4Box::new(b"udta", Vec::new(), Vec::new()));
    }
    let udta = moov.child(b"udta").unwrap();
    if udta.child(b"meta").is_none() {
        // mutagen's `hdlr` for iTunes metadata.
        let mut hdlr = vec![0; 8];
        hdlr.extend(b"mdirappl");
        hdlr.extend([0; 9]);
        udta.children.as_mut().unwrap().insert(
            0,
            Mp4Box::new(b"meta", vec![0; 4], vec![Mp4Box::leaf(b"hdlr", hdlr)]),
        );
    }
    let meta = udta.child(b"meta").unwrap();
    if meta.child(b"ilst").is_none() {
        meta.children
            .as_mut()
            .unwrap()
            .push(Mp4Box::new(b"ilst", Vec::new(), Vec::new()));
    }
    let ilst = meta.child(b"ilst").unwrap();
    let items = ilst.children.as_mut().unwrap();
    match items.iter().position(|item| &item.kind == b"covr") {
        Some(index) => items[index] = covr,
        None => items.push(covr),
    }
    let mut rendered = Vec::new();
    moov.render(&mut rendered);
    let delta = rendered.len() as i64 - (moov_end - moov_start) as i64;
    if delta != 0 {
        shift_offsets(&mut moov, moov_end, delta)?;
        rendered.clear();
        moov.render(&mut rendered);
    }
    input.seek(SeekFrom::Start(0))?;
    copy_exact(input, out, moov_start)?;
    out.write_all(&rendered)?;
    input.seek(SeekFrom::Start(moov_end))?;
    copy_exact(input, out, length - moov_end)?;
    Ok(())
}

/// Copy `length` bytes of `input`, which must have them.
fn copy_exact(input: &mut impl Read, out: &mut impl Write, length: u64) -> Result<()> {
    if std::io::copy(&mut input.take(length), out)? != length {
        return Err(invalid("the media file changed while it was read"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ogg_crc_matches_the_reference() {
        // The checksum of a page whose CRC field is zero, from libogg.
        assert_eq!(ogg_crc(b""), 0);
        assert_eq!(ogg_crc(b"123456789"), 0x89a1_897f);
    }

    #[test]
    fn png_and_jpeg_types_and_sizes_are_read_like_imghdr_and_ffmpeg() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend(480u32.to_be_bytes());
        png.extend(360u32.to_be_bytes());
        assert_eq!(image_type(&png), Some("png"));
        assert_eq!(image_size(&png), Some((480, 360)));
        let jpeg = b"\xff\xd8\xff\xe0\0\x10JFIF\0\x01\x01\0\0\x01\0\x01\0\0\xff\xc0\0\x11\x08\x01\x68\x01\xe0\x03";
        assert_eq!(image_type(jpeg), Some("jpeg"));
        assert_eq!(image_size(jpeg), Some((480, 360)));
        assert_eq!(image_type(b"GIF89a"), None);
    }

    fn mp4_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend(kind);
        out.extend(payload);
        out
    }

    #[test]
    fn deeply_nested_mp4_boxes_fail_instead_of_overflowing_the_stack() {
        let mut moov = Vec::new();
        for _ in 0..100_000 {
            moov = mp4_box(b"moov", &moov);
        }
        let error = embed_mp4(&moov, b"PNG", 14).unwrap_err().to_string();
        assert!(error.ends_with("MP4 boxes nested too deeply"), "{error}");
    }

    #[test]
    fn mp4_media_around_moov_is_copied_and_offsets_shifted() {
        // `moov` with one chunk offset into the `mdat` after it.
        let mut stco = vec![0; 4];
        stco.extend(1u32.to_be_bytes());
        let ftyp = mp4_box(b"ftyp", b"isom");
        let moov_len = 8 + 8 + 8 + 8 + 8 + 8 + 12;
        stco.extend(((ftyp.len() + moov_len + 8) as u32).to_be_bytes());
        let stbl = mp4_box(b"stbl", &mp4_box(b"stco", &stco));
        let minf = mp4_box(b"minf", &stbl);
        let mdia = mp4_box(b"mdia", &minf);
        let trak = mp4_box(b"trak", &mdia);
        let moov = mp4_box(b"moov", &trak);
        assert_eq!(moov.len(), moov_len);
        let mut file = ftyp.clone();
        file.extend(&moov);
        file.extend(mp4_box(b"mdat", b"frames"));
        let out = embed_mp4(&file, b"PNG", 14).unwrap();
        assert!(out.starts_with(&ftyp));
        assert!(out.ends_with(&mp4_box(b"mdat", b"frames")));
        // The offset still points at the `mdat` payload.
        let mdat = out.len() - 6;
        let at = out.windows(4).position(|w| w == b"stco").unwrap() + 12;
        assert_eq!(
            u32::from_be_bytes(out[at..at + 4].try_into().unwrap()) as usize,
            mdat
        );
        // A 64-bit size field cut short is not a MP4 file.
        let mut cut = file.clone();
        cut.extend([0, 0, 0, 1, b'f', b'r', b'e', b'e', 0]);
        assert!(embed_mp4(&cut, b"PNG", 14).is_err());
    }

    #[test]
    fn a_flac_picture_precedes_the_padding() {
        let mut file = b"fLaC".to_vec();
        file.extend([0x00, 0, 0, 2, 0xaa, 0xbb]); // STREAMINFO (shortened)
        file.extend([0x81, 0, 0, 1, 0]); // last: PADDING
        file.extend(b"frames");
        let out = embed_flac(&file, b"PIC").unwrap();
        let mut expected = b"fLaC".to_vec();
        expected.extend([0x00, 0, 0, 2, 0xaa, 0xbb]);
        expected.extend([0x06, 0, 0, 3]);
        expected.extend(b"PIC");
        expected.extend([0x81, 0, 0, 1, 0]);
        expected.extend(b"frames");
        assert_eq!(out, expected);
    }
}
