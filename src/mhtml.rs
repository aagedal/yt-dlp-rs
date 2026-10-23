//! Upstream `MhtmlFD`'s document (yt_dlp/downloader/mhtml.py): a MIME
//! `multipart/related` file whose HTML part shows each downloaded storyboard
//! image (one part per fragment) as a captioned slide.
//!
//! The pieces take the MIME boundary as a parameter so tests can pin it;
//! downloads use [`boundary`] (upstream's `uuid4().hex`). The generator meta
//! names this port (`yt-dlp-rs VERSION`) rather than upstream's version.

/// `_STYLESHEET` after upstream's whitespace squeezing.
const STYLESHEET: &str = "html,body{margin:0;padding:0;height:100vh;}html{overflow-y:scroll;scroll-snap-type:y mandatory;}body{scroll-snap-type:y mandatory;display:flex;flex-flow:column;}body>figure{max-width:100vw;max-height:100vh;scroll-snap-align:center;}body>figure>figcaption{text-align:center;height:2.5em;}body>figure>img{display:block;margin:auto;max-width:100%;max-height:calc(100vh - 5em);}";

/// The `<meta name="generator">` content.
pub(crate) const GENERATOR: &str = concat!("yt-dlp-rs ", env!("CARGO_PKG_VERSION"));

/// `str(uuid.uuid4()).replace('-', '')`: 32 lowercase hex digits with the
/// version 4 and RFC 4122 variant bits.
pub(crate) fn boundary() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut bytes = [0_u8; 16];
    for (half, chunk) in bytes.chunks_mut(8).enumerate() {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_usize(half);
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos()),
        );
        chunk.copy_from_slice(&hasher.finish().to_le_bytes());
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Upstream `escapeHTML`.
fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// `binascii.b2a_qp(data, quotetabs=False, istext=True, header=True)`
/// (what `quopri.encodestring(data, header=True)` calls).
fn b2a_qp_header(data: &[u8]) -> Vec<u8> {
    const MAXLINESIZE: usize = 76;
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let crlf = data
        .iter()
        .position(|&byte| byte == b'\n')
        .is_some_and(|at| at > 0 && data[at - 1] == b'\r');
    let mut out: Vec<u8> = Vec::new();
    let soft_break = |out: &mut Vec<u8>| {
        out.push(b'=');
        if crlf {
            out.push(b'\r');
        }
        out.push(b'\n');
    };
    let (mut at, mut linelen) = (0, 0);
    while at < data.len() {
        let byte = data[at];
        let next = data.get(at + 1).copied();
        let encode = byte > 126
            || byte == b'='
            || byte == b'_'
            || (byte == b'.' && linelen == 0 && matches!(next, None | Some(b'\n' | b'\r' | 0)))
            || (matches!(byte, b'\t' | b' ') && next.is_none())
            || (byte < 33 && !matches!(byte, b'\r' | b'\n' | b'\t' | b' '));
        if encode {
            if linelen + 3 >= MAXLINESIZE {
                soft_break(&mut out);
                linelen = 0;
            }
            out.extend([
                b'=',
                HEX[usize::from(byte >> 4)],
                HEX[usize::from(byte & 15)],
            ]);
            at += 1;
            linelen += 3;
        } else if byte == b'\n' || (byte == b'\r' && next == Some(b'\n')) {
            linelen = 0;
            if let Some(&last) = out.last()
                && matches!(last, b' ' | b'\t')
            {
                let end = out.len() - 1;
                out[end] = b'=';
                out.extend([HEX[usize::from(last >> 4)], HEX[usize::from(last & 15)]]);
            }
            if crlf {
                out.push(b'\r');
            }
            out.push(b'\n');
            at += if byte == b'\r' { 2 } else { 1 };
        } else {
            if next.is_some_and(|next| next != b'\n') && linelen + 1 >= MAXLINESIZE {
                soft_break(&mut out);
                linelen = 0;
            }
            linelen += 1;
            out.push(if byte == b' ' { b'_' } else { byte });
            at += 1;
        }
    }
    out
}

/// `MhtmlFD._escape_mime`: an RFC 2047 `Q` encoded word, with the control
/// bytes `quopri` leaves (such as its soft line breaks) escaped as well.
pub(crate) fn escape_mime(text: &str) -> String {
    let mut escaped = String::from("=?utf-8?Q?");
    for byte in b2a_qp_header(text.as_bytes()) {
        if byte >= 0x20 {
            escaped.push(char::from(byte));
        } else {
            escaped.push_str(&format!("={byte:02X}"));
        }
    }
    escaped.push_str("?=");
    escaped
}

/// Python's float `divmod` (for the timecode helpers).
fn divmod(x: f64, y: f64) -> (f64, f64) {
    let mut modulo = x % y;
    let mut div = (x - modulo) / y;
    if modulo != 0.0 {
        if (y < 0.0) != (modulo < 0.0) {
            modulo += y;
            div -= 1.0;
        }
    } else {
        modulo = 0.0_f64.copysign(y);
    }
    let floordiv = if div != 0.0 {
        let floor = div.floor();
        if div - floor > 0.5 {
            floor + 1.0
        } else {
            floor
        }
    } else {
        0.0_f64.copysign(x / y)
    };
    (floordiv, modulo)
}

/// `timetuple_from_msec(seconds * 1000)`, each part truncated like `%d`.
fn timetuple(seconds: f64) -> [i64; 4] {
    let (secs, msec) = divmod(seconds * 1000.0, 1000.0);
    let (mins, secs) = divmod(secs, 60.0);
    let (hours, mins) = divmod(mins, 60.0);
    [hours, mins, secs, msec].map(|part| part.trunc() as i64)
}

/// Upstream `srt_subtitles_timecode`.
fn srt_timecode(seconds: f64) -> String {
    let [hours, mins, secs, msec] = timetuple(seconds);
    format!("{hours:02}:{mins:02}:{secs:02},{msec:03}")
}

/// Upstream `formatSeconds(seconds, msec=True)`.
fn format_seconds_msec(seconds: f64) -> String {
    let [hours, mins, secs, msec] = timetuple(seconds);
    let whole = if hours != 0 {
        format!("{hours}:{mins:02}:{secs:02}")
    } else if mins != 0 {
        format!("{mins}:{secs:02}")
    } else {
        secs.to_string()
    };
    format!("{whole}.{msec:03}")
}

/// `MhtmlFD._gen_cid`.
fn cid(index: usize, boundary: &str) -> String {
    format!("{index}.{boundary}@yt-dlp.github.io.invalid")
}

/// `MhtmlFD._gen_stub`: the HTML part, one captioned `<figure>` per
/// fragment (`durations` in fragment order).
pub(crate) fn stub(durations: &[Option<f64>], boundary: &str, title: &str) -> String {
    let mut output = format!(
        "<!DOCTYPE html><html><head><meta name=\"generator\" content=\"{}\"><title>{}</title><style>{STYLESHEET}</style><body>",
        escape_html(GENERATOR),
        escape_html(title)
    );
    let mut t0 = Some(0.0_f64);
    for (index, duration) in durations.iter().enumerate() {
        output.push_str("<figure>");
        let t1 = t0.zip(*duration).map(|(t0, duration)| t0 + duration);
        match (t0, t1, duration) {
            (Some(t0), Some(t1), Some(duration)) => output.push_str(&format!(
                "<figcaption>Slide #{}: {} \u{2013} {} (duration: {})</figcaption>",
                index + 1,
                srt_timecode(t0),
                srt_timecode(t1),
                format_seconds_msec(*duration)
            )),
            _ => output.push_str(&format!("<figcaption>Slide #{}</figcaption>", index + 1)),
        }
        output.push_str(&format!("<img src=\"cid:{}\">", cid(index, boundary)));
        output.push_str("</figure>");
        t0 = t1;
    }
    output
}

/// The MIME headers and the HTML part, written before any fragment.
pub(crate) fn header(stub: &str, boundary: &str, title: &str, origin: &str) -> Vec<u8> {
    let mut header = format!(
        "MIME-Version: 1.0\r\n\
         From: <nowhere@yt-dlp.github.io.invalid>\r\n\
         To: <nowhere@yt-dlp.github.io.invalid>\r\n\
         Subject: {}\r\n\
         Content-type: multipart/related; boundary=\"{boundary}\"; type=\"text/html\"\r\n\
         X.yt-dlp.Origin: {origin}\r\n\
         \r\n\
         --{boundary}\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         \r\n",
        escape_mime(title),
        stub.len()
    )
    .into_bytes();
    header.extend_from_slice(stub.as_bytes());
    header.extend_from_slice(b"\r\n");
    header
}

/// `compat.imghdr.what(h=...)`, defaulting to `jpeg`.
fn image_type(content: &[u8]) -> &'static str {
    if content.starts_with(b"RIFF") && content.get(8..12) == Some(b"WEBP") {
        "webp"
    } else if content.starts_with(b"\x89PNG") {
        "png"
    } else if content.starts_with(b"GIF") {
        "gif"
    } else {
        "jpeg"
    }
}

/// One image part: its headers, the fragment's bytes and a line break.
pub(crate) fn part(
    index: usize,
    boundary: &str,
    url: &str,
    duration: f64,
    content: &[u8],
) -> Vec<u8> {
    let mut part = format!(
        "--{boundary}\r\n\
         Content-ID: <{}>\r\n\
         Content-type: image/{}\r\n\
         Content-length: {}\r\n\
         Content-location: {url}\r\n\
         X.yt-dlp.Duration: {duration:.6}\r\n\
         \r\n",
        cid(index, boundary),
        image_type(content),
        content.len()
    )
    .into_bytes();
    part.extend_from_slice(content);
    part.extend_from_slice(b"\r\n");
    part
}

/// The closing boundary.
pub(crate) fn trailer(boundary: &str) -> Vec<u8> {
    format!("--{boundary}--\r\n\r\n").into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subjects_are_quoted_printable_like_quopri() {
        // Expected values from CPython's `MhtmlFD._escape_mime`.
        assert_eq!(escape_mime("Plain title"), "=?utf-8?Q?Plain_title?=");
        assert_eq!(
            escape_mime(
                "Storyboard \"slides\" & <tiles> \u{2013} \u{fc}n\u{ef}code_title with a fairly long name to wrap = yes"
            ),
            "=?utf-8?Q?Storyboard_\"slides\"_&_<tiles>_=E2=80=93_=C3=BCn=C3=AFcode=5Ftitle_with_a_fa==0Airly_long_name_to_wrap_=3D_yes?="
        );
        assert_eq!(
            escape_mime("tab\there end "),
            "=?utf-8?Q?tab=09here_end=20?="
        );
        assert_eq!(escape_mime("a\nb"), "=?utf-8?Q?a=0Ab?=");
        assert_eq!(escape_mime(".\n"), "=?utf-8?Q?=2E=0A?=");
        assert_eq!(
            escape_mime(&"x".repeat(80)),
            format!("=?utf-8?Q?{}==0Axxxxx?=", "x".repeat(75))
        );
        assert_eq!(
            escape_mime("a\r\nb c \r\nd"),
            "=?utf-8?Q?a=0D=0Ab_c_=0D=0Ad?="
        );
    }

    #[test]
    fn captions_use_upstream_timecodes() {
        let third = 125.0 / 7.5;
        assert_eq!(srt_timecode(third * 3.0), "00:00:50,000");
        assert_eq!(srt_timecode(3725.5), "01:02:05,500");
        assert_eq!(format_seconds_msec(third), "16.666");
        assert_eq!(format_seconds_msec(3725.5), "1:02:05.500");
        assert_eq!(format_seconds_msec(65.0), "1:05.000");
        let html = stub(&[Some(1.5), None, Some(2.0)], "b", "t'");
        assert!(html.contains("<title>t&#39;</title>"));
        assert!(html.contains(
            "<figcaption>Slide #1: 00:00:00,000 \u{2013} 00:00:01,500 (duration: 1.500)</figcaption><img src=\"cid:0.b@yt-dlp.github.io.invalid\">"
        ));
        // Without a duration, later slides have no times either.
        assert!(html.contains("<figcaption>Slide #2</figcaption>"));
        assert!(html.contains("<figcaption>Slide #3</figcaption>"));
    }

    #[test]
    fn parts_detect_image_types_like_imghdr() {
        assert_eq!(image_type(b"RIFF\0\0\0\0WEBPVP8 "), "webp");
        assert_eq!(image_type(b"\x89PNG\r\n"), "png");
        assert_eq!(image_type(b"GIF89a"), "gif");
        assert_eq!(image_type(b"\xff\xd8\xff\xe0"), "jpeg");
        assert_eq!(image_type(b"??"), "jpeg");
        assert_eq!(
            part(2, "b", "https://x.test/2.jpg", 8.0 + 1.0 / 3.0, b"GIF"),
            b"--b\r\nContent-ID: <2.b@yt-dlp.github.io.invalid>\r\nContent-type: image/gif\r\nContent-length: 3\r\nContent-location: https://x.test/2.jpg\r\nX.yt-dlp.Duration: 8.333333\r\n\r\nGIF\r\n"
        );
    }

    #[test]
    fn boundaries_look_like_uuid4_hex() {
        let first = boundary();
        assert_eq!(first.len(), 32);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(&first[12..13], "4");
        assert_ne!(first, boundary());
    }
}
