//! `--embed-thumbnail` for Opus, Vorbis and FLAC files: upstream embeds
//! through mutagen (`METADATA_BLOCK_PICTURE`, `FLAC.add_picture`); this port
//! writes the same picture natively. FFmpeg is never run for PNG thumbnails.
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::Command,
    thread,
};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/cover_art")
            .join(name),
    )
    .unwrap()
}

/// A PNG of `size` bytes: its signature, a 640x480 `IHDR` and filler (the
/// picture's MIME type and size are read from those).
fn large_png(size: usize) -> Vec<u8> {
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    png.extend(640u32.to_be_bytes());
    png.extend(480u32.to_be_bytes());
    png.extend((png.len()..size).map(|index| (index * 7 % 251) as u8));
    png
}

fn serve(files: Vec<(&'static str, Vec<u8>)>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                request.push(byte[0]);
            }
            let request = String::from_utf8_lossy(&request);
            let path = request.split(' ').nth(1).unwrap_or_default();
            let body = files
                .iter()
                .find(|(name, _)| path == format!("/{name}"))
                .map(|(_, body)| body.as_slice());
            match body {
                Some(body) => {
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(body);
                }
                None => {
                    let _ = stream.write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                }
            }
        }
    });
    origin
}

/// mutagen `Picture.write()` for a front cover.
fn picture(data: &[u8], mime: &str, width: u32, height: u32) -> Vec<u8> {
    let mut block = Vec::new();
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

fn embed(media: &str, thumbnail: &str, files: Vec<(&'static str, Vec<u8>)>) -> Vec<u8> {
    let origin = serve(files);
    let directory = tempfile::tempdir().unwrap();
    let ext = media.rsplit('.').next().unwrap();
    std::fs::write(
        directory.path().join("in.info.json"),
        serde_json::json!({
            "id": "cover", "title": "Cover", "ext": ext, "protocol": "http",
            "url": format!("{origin}/{media}"), "webpage_url": "https://example.test/cover",
            "extractor": "fixture", "extractor_key": "Fixture",
            "thumbnails": [{"url": format!("{origin}/{thumbnail}"), "id": "0"}],
        })
        .to_string(),
    )
    .unwrap();
    // The native embedding needs no FFmpeg run; any explicit path enables it.
    let ffmpeg = directory.path().join("ffmpeg-unused");
    std::fs::write(&ffmpeg, b"").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "--ignore-config",
            "--no-progress",
            "--embed-thumbnail",
            "-o",
            "o.%(ext)s",
        ])
        .arg("--ffmpeg-location")
        .arg(&ffmpeg)
        .args(["--load-info-json", "in.info.json"])
        .current_dir(directory.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains(&format!(
            "[EmbedThumbnail] mutagen: Adding thumbnail to \"o.{ext}\"\n"
        )),
        "{stdout}"
    );
    // The thumbnail is deleted after embedding, as upstream does.
    assert!(!directory.path().join("o.png").exists());
    std::fs::read(directory.path().join(format!("o.{ext}"))).unwrap()
}

fn crc(data: &[u8]) -> u32 {
    let mut crc = 0u32;
    for &byte in data {
        crc ^= u32::from(byte) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ 0x04c1_1db7
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// Each page's flags, sequence number and packets' bytes, checking every CRC.
fn ogg_pages(file: &[u8]) -> Vec<(u8, u32, Vec<u8>, Vec<u8>)> {
    let mut pages = Vec::new();
    let mut at = 0;
    while at < file.len() {
        assert_eq!(&file[at..at + 4], b"OggS");
        let segments = usize::from(file[at + 26]);
        let lacing = file[at + 27..at + 27 + segments].to_vec();
        let size: usize = lacing.iter().map(|&value| usize::from(value)).sum();
        let end = at + 27 + segments + size;
        let mut page = file[at..end].to_vec();
        let stored = u32::from_le_bytes(page[22..26].try_into().unwrap());
        page[22..26].fill(0);
        assert_eq!(crc(&page), stored, "page {}", pages.len());
        let sequence = u32::from_le_bytes(file[at + 18..at + 22].try_into().unwrap());
        pages.push((
            file[at + 5],
            sequence,
            lacing,
            file[at + 27 + segments..end].to_vec(),
        ));
        at = end;
    }
    pages
}

fn ogg_packets(pages: &[(u8, u32, Vec<u8>, Vec<u8>)]) -> Vec<Vec<u8>> {
    let mut packets = Vec::new();
    let mut current = Vec::new();
    for (flags, _, lacing, body) in pages {
        // A continued page exactly when a packet is unfinished.
        assert_eq!(flags & 1 == 1, !current.is_empty());
        let mut offset = 0;
        for &value in lacing {
            current.extend(&body[offset..offset + usize::from(value)]);
            offset += usize::from(value);
            if value < 255 {
                packets.push(std::mem::take(&mut current));
            }
        }
    }
    packets
}

fn check_ogg(original: &[u8], embedded: &[u8], comment_signature: &[u8], expected: &[u8]) {
    let before = ogg_pages(original);
    let after = ogg_pages(embedded);
    let sequences: Vec<u32> = after.iter().map(|page| page.1).collect();
    assert_eq!(sequences, (0..after.len() as u32).collect::<Vec<_>>());
    let (old, new) = (ogg_packets(&before), ogg_packets(&after));
    assert_eq!(old.len(), new.len());
    // Every packet but the comment header is unchanged (audio pages too).
    for (index, (old, new)) in old.iter().zip(&new).enumerate() {
        if index != 1 {
            assert_eq!(old, new, "packet {index}");
        }
    }
    assert_eq!(before.last().unwrap().3, after.last().unwrap().3);
    let comment = new[1].strip_prefix(comment_signature).unwrap();
    use base64::Engine;
    let entry = format!(
        "METADATA_BLOCK_PICTURE={}",
        base64::engine::general_purpose::STANDARD.encode(expected)
    );
    let found = comment
        .windows(entry.len())
        .any(|window| window == entry.as_bytes());
    assert!(found, "no METADATA_BLOCK_PICTURE entry");
}

#[test]
fn opus_and_vorbis_files_get_a_metadata_block_picture() {
    let png = fixture("cover.png");
    for (media, signature) in [("tiny.opus", &b"OpusTags"[..]), ("tiny.ogg", b"\x03vorbis")] {
        let original = fixture(media);
        let embedded = embed(
            media,
            "cover.png",
            vec![(media, original.clone()), ("cover.png", png.clone())],
        );
        check_ogg(
            &original,
            &embedded,
            signature,
            &picture(&png, "image/png", 2, 2),
        );
        // A picture spanning many pages, including a full last segment.
        let large = large_png(255 * 600);
        let embedded = embed(
            media,
            "cover.png",
            vec![(media, original.clone()), ("cover.png", large.clone())],
        );
        check_ogg(
            &original,
            &embedded,
            signature,
            &picture(&large, "image/png", 640, 480),
        );
    }
}

#[test]
fn flac_files_get_a_picture_block_before_their_padding() {
    let png = fixture("cover.png");
    let original = fixture("tiny.flac");
    let embedded = embed(
        "tiny.flac",
        "cover.png",
        vec![("tiny.flac", original.clone()), ("cover.png", png.clone())],
    );
    let blocks = |file: &[u8]| {
        let mut blocks = Vec::new();
        let mut at = 4;
        loop {
            let last = file[at] & 0x80 != 0;
            let length = u32::from_be_bytes([0, file[at + 1], file[at + 2], file[at + 3]]) as usize;
            blocks.push((file[at] & 0x7f, file[at + 4..at + 4 + length].to_vec()));
            at += 4 + length;
            if last {
                return (blocks, at);
            }
        }
    };
    let (old, old_end) = blocks(&original);
    let (new, new_end) = blocks(&embedded);
    let mut expected = old.clone();
    let padding = expected.iter().position(|(kind, _)| *kind == 1).unwrap();
    expected.insert(padding, (6, picture(&png, "image/png", 2, 2)));
    assert_eq!(new, expected);
    assert_eq!(original[old_end..], embedded[new_end..]);
}

/// The payload of the first box of `kind` on the path below `data`'s boxes.
fn mp4_path<'a>(data: &'a [u8], path: &[&[u8; 4]]) -> Option<&'a [u8]> {
    let (first, rest) = path.split_first()?;
    let mut at = 0;
    while at + 8 <= data.len() {
        let size = u32::from_be_bytes(data[at..at + 4].try_into().unwrap()) as usize;
        let size = if size == 0 { data.len() - at } else { size };
        if &data[at + 4..at + 8] == *first {
            let payload = &data[at + 8..at + size];
            // `meta` is a full box.
            let payload = if *first == b"meta" {
                &payload[4..]
            } else {
                payload
            };
            return if rest.is_empty() {
                Some(payload)
            } else {
                mp4_path(payload, rest)
            };
        }
        at += size;
    }
    None
}

/// The first `stco` chunk offsets and the 16 bytes at each.
fn chunks(file: &[u8]) -> Vec<(u32, Vec<u8>)> {
    let stco = mp4_path(
        file,
        &[b"moov", b"trak", b"mdia", b"minf", b"stbl", b"stco"],
    )
    .unwrap();
    let count = u32::from_be_bytes(stco[4..8].try_into().unwrap()) as usize;
    (0..count)
        .map(|index| {
            let offset =
                u32::from_be_bytes(stco[8 + index * 4..12 + index * 4].try_into().unwrap());
            (offset, file[offset as usize..offset as usize + 16].to_vec())
        })
        .collect()
}

#[test]
fn m4a_files_get_an_itunes_cover_with_chunk_offsets_kept_valid() {
    let png = fixture("cover.png");
    for media in ["tiny-faststart.m4a", "tiny.m4a"] {
        let original = fixture(media);
        let embedded = embed(
            media,
            "cover.png",
            vec![(media, original.clone()), ("cover.png", png.clone())],
        );
        // mutagen's `hdlr` and the cover: `data` of type 14 (PNG), locale 0.
        let hdlr = mp4_path(&embedded, &[b"moov", b"udta", b"meta", b"hdlr"]).unwrap();
        assert_eq!(&hdlr[8..16], b"mdirappl");
        let data = mp4_path(
            &embedded,
            &[b"moov", b"udta", b"meta", b"ilst", b"covr", b"data"],
        )
        .unwrap();
        let mut expected = 14u32.to_be_bytes().to_vec();
        expected.extend(0u32.to_be_bytes());
        expected.extend(&png);
        assert_eq!(data, expected, "{media}");
        // Every chunk still starts with the same bytes; with `moov` first
        // the offsets moved by the added boxes.
        let (before, after) = (chunks(&original), chunks(&embedded));
        assert_eq!(
            before.iter().map(|(_, bytes)| bytes).collect::<Vec<_>>(),
            after.iter().map(|(_, bytes)| bytes).collect::<Vec<_>>()
        );
        let moved = before[0].0 != after[0].0;
        assert_eq!(moved, media == "tiny-faststart.m4a");
    }
}
