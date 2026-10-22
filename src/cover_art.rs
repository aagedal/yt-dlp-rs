//! Native cover art for Ogg (Opus, Vorbis) and FLAC files: what upstream's
//! `EmbedThumbnailPP` does through mutagen (`Picture`, `METADATA_BLOCK_PICTURE`
//! and `FLAC.add_picture`), since FFmpeg cannot embed pictures there.
use crate::{Error, Result};
use base64::Engine;

fn invalid(message: &str) -> Error {
    Error::Postprocess(format!("Unable to embed the thumbnail: {message}"))
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
pub(crate) fn embed_flac(file: &[u8], picture: &[u8]) -> Result<Vec<u8>> {
    if !file.starts_with(b"fLaC") {
        return Err(invalid("not a FLAC file"));
    }
    let mut blocks: Vec<(u8, &[u8])> = Vec::new();
    let mut at = 4;
    loop {
        let header = file
            .get(at..at + 4)
            .ok_or_else(|| invalid("truncated FLAC metadata"))?;
        let last = header[0] & 0x80 != 0;
        let kind = header[0] & 0x7f;
        let length = u32::from_be_bytes([0, header[1], header[2], header[3]]) as usize;
        let body = file
            .get(at + 4..at + 4 + length)
            .ok_or_else(|| invalid("truncated FLAC metadata"))?;
        blocks.push((kind, body));
        at += 4 + length;
        if last {
            break;
        }
    }
    if picture.len() >= 1 << 24 {
        return Err(invalid("the thumbnail is too large for a FLAC picture"));
    }
    let padding = blocks.iter().position(|(kind, _)| *kind == 1);
    let index = padding.unwrap_or(blocks.len());
    blocks.insert(index, (6, picture));
    let mut out = Vec::with_capacity(file.len() + picture.len() + 4);
    out.extend(b"fLaC");
    let count = blocks.len();
    for (position, (kind, body)) in blocks.into_iter().enumerate() {
        let last = if position + 1 == count { 0x80 } else { 0 };
        out.push(last | kind);
        out.extend(&(body.len() as u32).to_be_bytes()[1..]);
        out.extend(body);
    }
    out.extend(&file[at..]);
    Ok(out)
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

struct Page<'a> {
    header_type: u8,
    granule: [u8; 8],
    serial: u32,
    sequence: u32,
    lacing: &'a [u8],
    body: &'a [u8],
}

fn pages(file: &[u8]) -> Result<Vec<Page<'_>>> {
    let mut pages = Vec::new();
    let mut at = 0;
    while at < file.len() {
        let header = file
            .get(at..at + 27)
            .ok_or_else(|| invalid("truncated Ogg page"))?;
        if &header[..4] != b"OggS" || header[4] != 0 {
            return Err(invalid("not an Ogg page"));
        }
        let segments = usize::from(header[26]);
        let lacing = file
            .get(at + 27..at + 27 + segments)
            .ok_or_else(|| invalid("truncated Ogg page"))?;
        let size: usize = lacing.iter().map(|&value| usize::from(value)).sum();
        let start = at + 27 + segments;
        let body = file
            .get(start..start + size)
            .ok_or_else(|| invalid("truncated Ogg page"))?;
        pages.push(Page {
            header_type: header[5],
            granule: header[6..14].try_into().unwrap(),
            serial: u32::from_le_bytes(header[14..18].try_into().unwrap()),
            sequence: u32::from_le_bytes(header[18..22].try_into().unwrap()),
            lacing,
            body,
        });
        at = start + size;
    }
    Ok(pages)
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
pub(crate) fn embed_ogg(file: &[u8], picture: &[u8]) -> Result<Vec<u8>> {
    let pages = pages(file)?;
    let first = pages.first().ok_or_else(|| invalid("empty Ogg file"))?;
    let serial = first.serial;
    if pages.iter().any(|page| page.serial != serial) {
        return Err(invalid("multiplexed Ogg streams are not supported"));
    }
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
    let mut last_header_page = None;
    for (index, page) in pages.iter().enumerate().skip(1) {
        let mut offset = 0;
        for &value in page.lacing {
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
            last_header_page = Some(index);
            break;
        }
    }
    let last_header_page = last_header_page.ok_or_else(|| invalid("missing Ogg header packets"))?;
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
    let mut out = Vec::with_capacity(file.len() + picture.len() * 4 / 3 + 1024);
    out.extend(&file[..27 + first.lacing.len() + first.body.len()]);
    let mut sequence = first.sequence + 1;
    let mut lacing: Vec<u8> = Vec::new();
    let mut body: Vec<u8> = Vec::new();
    // Whether the page being filled begins with a continued packet.
    let mut continued = false;
    let mut flush =
        |out: &mut Vec<u8>, lacing: &mut Vec<u8>, body: &mut Vec<u8>, continued: bool| {
            write_page(
                out,
                u8::from(continued),
                [0; 8],
                serial,
                sequence,
                lacing,
                body,
            );
            sequence += 1;
            lacing.clear();
            body.clear();
        };
    for packet in &packets {
        let mut remaining: &[u8] = packet;
        let mut started = false;
        loop {
            if lacing.len() == 255 {
                flush(&mut out, &mut lacing, &mut body, continued);
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
        flush(&mut out, &mut lacing, &mut body, continued);
    }
    drop(flush);
    // Renumber the audio pages.
    for page in &pages[last_header_page + 1..] {
        write_page(
            &mut out,
            page.header_type,
            page.granule,
            serial,
            sequence,
            page.lacing,
            page.body,
        );
        sequence += 1;
    }
    Ok(out)
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
    start: usize,
    end: usize,
    payload: &'a [u8],
}

fn mp4_boxes(data: &[u8]) -> Result<Vec<RawBox<'_>>> {
    let mut boxes = Vec::new();
    let mut at = 0;
    while at + 8 <= data.len() {
        let size = u32::from_be_bytes(data[at..at + 4].try_into().unwrap()) as u64;
        let kind: [u8; 4] = data[at + 4..at + 8].try_into().unwrap();
        let (header, size) = match size {
            0 => (8, (data.len() - at) as u64),
            1 => {
                let large = data
                    .get(at + 8..at + 16)
                    .ok_or_else(|| invalid("truncated MP4 box"))?;
                (16, u64::from_be_bytes(large.try_into().unwrap()))
            }
            size => (8, size),
        };
        let end = at
            .checked_add(usize::try_from(size).map_err(|_| invalid("MP4 box too large"))?)
            .filter(|end| *end <= data.len() && size >= header as u64)
            .ok_or_else(|| invalid("truncated MP4 box"))?;
        boxes.push(RawBox {
            kind,
            start: at,
            end,
            payload: &data[at + header..end],
        });
        at = end;
    }
    if at != data.len() {
        return Err(invalid("truncated MP4 box"));
    }
    Ok(boxes)
}

/// The tree under `moov`: containers down to `ilst` and `stco`/`co64`.
fn mp4_tree(kind: [u8; 4], payload: &[u8]) -> Result<Mp4Box> {
    let container = matches!(
        &kind,
        b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"udta" | b"meta" | b"ilst"
    );
    if !container {
        return Ok(Mp4Box::leaf(&kind, payload.to_vec()));
    }
    // `meta` is a full box (version and flags first).
    let skip = if &kind == b"meta" { 4 } else { 0 };
    let prefix = payload
        .get(..skip)
        .ok_or_else(|| invalid("truncated MP4 box"))?
        .to_vec();
    let children = mp4_boxes(&payload[skip..])?
        .into_iter()
        .map(|raw| mp4_tree(raw.kind, raw.payload))
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
pub(crate) fn embed_mp4(file: &[u8], image: &[u8], format: u32) -> Result<Vec<u8>> {
    let boxes = mp4_boxes(file).map_err(|_| invalid("not a MP4 file"))?;
    let (moov_start, moov_end, moov_payload) = boxes
        .iter()
        .find(|raw| &raw.kind == b"moov")
        .map(|raw| (raw.start, raw.end, raw.payload))
        .ok_or_else(|| invalid("not a MP4 file"))?;
    let mut moov = mp4_tree(*b"moov", moov_payload)?;
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
        shift_offsets(&mut moov, moov_end as u64, delta)?;
        rendered.clear();
        moov.render(&mut rendered);
    }
    let mut out = Vec::with_capacity(file.len() + image.len() + 64);
    out.extend(&file[..moov_start]);
    out.extend(rendered);
    out.extend(&file[moov_end..]);
    Ok(out)
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
