//! Bounded, pure HLS VOD parsing. Transport and playlist traversal live elsewhere.
use crate::{ByteRange, Error, Fragment, Result, StreamKind};
use std::collections::BTreeMap;
use url::Url;

const MAX_FRAGMENTS: usize = 10_000;
const MAX_VARIANTS: usize = 256;

#[derive(Debug)]
pub(crate) enum Playlist {
    Master(Vec<Variant>),
    Media {
        fragments: Vec<Fragment>,
        ext: String,
    },
}

#[derive(Debug)]
pub(crate) struct Variant {
    pub url: Url,
    /// Kilobits per second, with valid average bandwidth preferred.
    pub bandwidth: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub stream_kind: StreamKind,
}

fn invalid(message: impl Into<String>) -> Error {
    Error::UnsupportedManifest(message.into())
}

/// Parse an attribute list without splitting commas inside quoted values.
fn attributes(input: &str) -> Result<BTreeMap<&str, &str>> {
    let mut result = BTreeMap::new();
    let mut rest = input;
    while !rest.is_empty() {
        let (name, value) = rest
            .split_once('=')
            .ok_or_else(|| invalid("malformed HLS attribute"))?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(invalid("invalid HLS attribute name"));
        }
        let (value, tail) = if let Some(quoted) = value.strip_prefix('"') {
            let end = quoted
                .find('"')
                .ok_or_else(|| invalid("unterminated HLS attribute quote"))?;
            (&quoted[..end], &quoted[end + 1..])
        } else {
            let end = value.find(',').unwrap_or(value.len());
            let plain = &value[..end];
            if plain.is_empty() || plain.contains(['"', ' ', '\t']) {
                return Err(invalid("invalid unquoted HLS attribute"));
            }
            (plain, &value[end..])
        };
        if result.insert(name, value).is_some() {
            return Err(invalid("duplicate HLS attribute"));
        }
        if tail.is_empty() {
            break;
        }
        rest = tail
            .strip_prefix(',')
            .filter(|v| !v.is_empty())
            .ok_or_else(|| invalid("malformed HLS attribute separator"))?;
    }
    Ok(result)
}

fn absolute(base: &Url, value: &str) -> Result<Url> {
    if value.is_empty() || value.contains(['\r', '\n']) || value.contains("{$") {
        return Err(invalid("empty or variable HLS URI"));
    }
    let url = base.join(value)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || url.fragment().is_some()
    {
        return Err(invalid("HLS URI must be an HTTP(S) URL without a fragment"));
    }
    Ok(url)
}

fn positive_integer(value: &str) -> Result<u64> {
    let number = unsigned(value)?;
    if number == 0 {
        return Err(invalid("HLS number must be positive"));
    }
    Ok(number)
}

fn unsigned(value: &str) -> Result<u64> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid("invalid HLS integer"));
    }
    value.parse().map_err(|_| invalid("HLS integer overflow"))
}

fn byte_range(
    value: &str,
    previous: Option<(&Url, &ByteRange)>,
    url: &Url,
    explicit: bool,
) -> Result<ByteRange> {
    let (length, offset) = match value.split_once('@') {
        Some((length, offset)) => (positive_integer(length)?, unsigned(offset)?),
        None if !explicit => {
            let (_, previous) = previous
                .filter(|(previous_url, _)| *previous_url == url)
                .ok_or_else(|| {
                    invalid("implicit HLS range requires preceding range on the same URI")
                })?;
            (
                positive_integer(value)?,
                previous
                    .offset
                    .checked_add(previous.length)
                    .ok_or_else(|| invalid("HLS range overflow"))?,
            )
        }
        None => {
            return Err(invalid(
                "HLS initialization range requires an explicit offset",
            ));
        }
    };
    offset
        .checked_add(length)
        .ok_or_else(|| invalid("HLS range overflow"))?;
    Ok(ByteRange { offset, length })
}

fn variant(
    values: &BTreeMap<&str, &str>,
    url: Url,
    groups: &BTreeMap<(&str, &str), bool>,
) -> Result<Variant> {
    for group_type in ["AUDIO", "SUBTITLES", "VIDEO", "CLOSED-CAPTIONS"] {
        if let Some(group) = values.get(group_type).filter(|group| **group != "NONE")
            && groups.get(&(group_type, *group)) != Some(&false)
        {
            return Err(invalid(
                "external or unresolved HLS rendition group is unsupported",
            ));
        }
    }
    let bandwidth = |name| {
        values
            .get(name)
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite() && *value > 0.0)
            .map(|value| value / 1000.0)
    };
    let (width, height) = if let Some(resolution) = values.get("RESOLUTION") {
        let (width, height) = resolution
            .split_once('x')
            .ok_or_else(|| invalid("invalid HLS resolution"))?;
        let dimension = |value| {
            u32::try_from(positive_integer(value)?).map_err(|_| invalid("HLS dimension overflow"))
        };
        (Some(dimension(width)?), Some(dimension(height)?))
    } else {
        (None, None)
    };
    let stream_kind = if let Some(codecs) = values.get("CODECS") {
        let mut video = false;
        let mut audio = false;
        for codec in codecs.split(',').map(str::trim) {
            let family = codec.split('.').next().unwrap_or("");
            match family {
                "avc1" | "avc3" | "av01" | "hvc1" | "hev1" | "vp8" | "vp9" | "vp09" => video = true,
                "mp4a" | "aac" | "ac-3" | "ec-3" | "opus" | "vorbis" => audio = true,
                _ => return Err(invalid("unsupported HLS codec")),
            }
        }
        match (video, audio) {
            (true, false) => StreamKind::VideoOnly,
            (false, true) => StreamKind::AudioOnly,
            (true, true) => StreamKind::Standalone,
            _ => return Err(invalid("empty HLS codecs")),
        }
    } else {
        StreamKind::Standalone
    };
    Ok(Variant {
        url,
        bandwidth: bandwidth("AVERAGE-BANDWIDTH").or_else(|| bandwidth("BANDWIDTH")),
        width,
        height,
        stream_kind,
    })
}

pub(crate) fn parse(text: &str, base: &Url) -> Result<Playlist> {
    let lines: Vec<_> = text
        .trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if lines.first() != Some(&"#EXTM3U") {
        return Err(invalid("missing HLS EXTM3U header"));
    }
    let mut groups = BTreeMap::new();
    for line in &lines {
        if let Some(value) = line.strip_prefix("#EXT-X-MEDIA:") {
            let attrs = attributes(value)?;
            let kind = attrs
                .get("TYPE")
                .ok_or_else(|| invalid("HLS rendition has no TYPE"))?;
            let group = attrs
                .get("GROUP-ID")
                .filter(|v| !v.is_empty())
                .ok_or_else(|| invalid("HLS rendition has no GROUP-ID"))?;
            if !matches!(*kind, "AUDIO" | "VIDEO" | "SUBTITLES" | "CLOSED-CAPTIONS") {
                return Err(invalid("unsupported HLS rendition type"));
            }
            let external = attrs.contains_key("URI");
            groups
                .entry((*kind, *group))
                .and_modify(|value| *value |= external)
                .or_insert(external);
        }
    }
    let mut variants = Vec::new();
    let mut fragments: Vec<Fragment> = Vec::new();
    let mut pending_variant = None;
    let mut pending_duration = false;
    let mut pending_range = None;
    let mut endlist = false;
    let mut media_seen = false;
    let mut has_map = false;
    let mut ext: Option<&str> = None;
    for line in lines.into_iter().skip(1) {
        if endlist && (line.starts_with("#EXT") || !line.starts_with('#')) {
            return Err(invalid("HLS ENDLIST must terminate the media playlist"));
        }
        if line == "#EXT-X-ENDLIST" {
            if endlist || pending_duration || pending_range.is_some() || pending_variant.is_some() {
                return Err(invalid("misplaced HLS ENDLIST"));
            }
            endlist = true;
            media_seen = true;
        } else if let Some(value) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            if media_seen || pending_variant.is_some() {
                return Err(invalid("mixed or incomplete HLS playlist"));
            }
            pending_variant = Some(attributes(value)?);
        } else if let Some(value) = line.strip_prefix("#EXTINF:") {
            if endlist || !variants.is_empty() || pending_variant.is_some() || pending_duration {
                return Err(invalid("misplaced HLS segment duration"));
            }
            let duration = value
                .split(',')
                .next()
                .unwrap_or("")
                .parse::<f64>()
                .map_err(|_| invalid("invalid HLS duration"))?;
            if !duration.is_finite() || duration <= 0.0 {
                return Err(invalid("invalid HLS duration"));
            }
            pending_duration = true;
            media_seen = true;
        } else if let Some(value) = line.strip_prefix("#EXT-X-BYTERANGE:") {
            if endlist
                || pending_range.is_some()
                || !variants.is_empty()
                || pending_variant.is_some()
            {
                return Err(invalid("misplaced HLS byte range"));
            }
            pending_range = Some(value);
            media_seen = true;
        } else if let Some(value) = line.strip_prefix("#EXT-X-MAP:") {
            if endlist
                || has_map
                || !fragments.is_empty()
                || pending_duration
                || pending_range.is_some()
                || pending_variant.is_some()
                || !variants.is_empty()
            {
                return Err(invalid("only one initial HLS MAP is supported"));
            }
            let attrs = attributes(value)?;
            let url = absolute(
                base,
                attrs
                    .get("URI")
                    .ok_or_else(|| invalid("HLS MAP has no URI"))?,
            )?;
            let range = attrs
                .get("BYTERANGE")
                .map(|value| byte_range(value, None, &url, true))
                .transpose()?;
            fragments.push(Fragment {
                url,
                byte_range: range,
            });
            has_map = true;
            media_seen = true;
            ext = Some("mp4");
        } else if let Some(value) = line.strip_prefix("#EXT-X-KEY:") {
            if !variants.is_empty() || pending_variant.is_some() {
                return Err(invalid("HLS media key appears in a master playlist"));
            }
            media_seen = true;
            let attrs = attributes(value)?;
            if attrs.get("METHOD") != Some(&"NONE") || attrs.len() != 1 {
                return Err(invalid("encrypted HLS is unsupported"));
            }
        } else if let Some(value) = line.strip_prefix("#EXT-X-VERSION:") {
            positive_integer(value)?;
        } else if let Some(value) = line.strip_prefix("#EXT-X-TARGETDURATION:") {
            positive_integer(value)?;
            media_seen = true;
        } else if let Some(value) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            unsigned(value)?;
            media_seen = true;
        } else if let Some(value) = line.strip_prefix("#EXT-X-PLAYLIST-TYPE:") {
            if !matches!(value, "VOD" | "EVENT") {
                return Err(invalid("invalid HLS playlist type"));
            }
            media_seen = true;
        } else if let Some(value) = line.strip_prefix("#EXT-X-ALLOW-CACHE:") {
            if !matches!(value, "YES" | "NO") {
                return Err(invalid("invalid HLS cache policy"));
            }
        } else if line == "#EXT-X-INDEPENDENT-SEGMENTS"
            || line.starts_with("#EXT-X-PROGRAM-DATE-TIME:")
            || line.starts_with("#EXT-X-MEDIA:")
        {
            // Informational metadata; media groups are validated against references above.
        } else if line.starts_with("#EXT-X-") || line.starts_with("#EXT") {
            return Err(invalid(format!(
                "unsupported HLS tag: {}",
                line.split(':').next().unwrap_or(line)
            )));
        } else if line.starts_with('#') {
            continue;
        } else {
            let url = absolute(base, line)?;
            if let Some(attrs) = pending_variant.take() {
                if variants.len() == MAX_VARIANTS {
                    return Err(invalid("HLS variant limit exceeded"));
                }
                variants.push(variant(&attrs, url, &groups)?);
                continue;
            }
            if endlist || !pending_duration || !variants.is_empty() {
                return Err(invalid("HLS media URI has no preceding EXTINF"));
            }
            if fragments.len() == MAX_FRAGMENTS {
                return Err(invalid("HLS fragment limit exceeded"));
            }
            if !has_map {
                let suffix = url
                    .path()
                    .rsplit('/')
                    .next()
                    .unwrap_or("")
                    .rsplit('.')
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let segment_ext = match suffix.as_str() {
                    "aac" => "aac",
                    "m3u8" | "mpd" | "html" | "htm" | "mp4" | "m4s" => {
                        return Err(invalid(
                            "unsupported HLS segment without initialization MAP",
                        ));
                    }
                    _ => "ts",
                };
                if ext.is_some_and(|ext| ext != segment_ext) {
                    return Err(invalid("mixed HLS segment containers"));
                }
                ext = Some(segment_ext);
            }
            let previous = fragments
                .last()
                .filter(|_| fragments.len() > usize::from(has_map))
                .and_then(|fragment| {
                    fragment
                        .byte_range
                        .as_ref()
                        .map(|range| (&fragment.url, range))
                });
            let range = pending_range
                .take()
                .map(|value| byte_range(value, previous, &url, false))
                .transpose()?;
            fragments.push(Fragment {
                url,
                byte_range: range,
            });
            pending_duration = false;
        }
    }
    if pending_variant.is_some() || pending_duration || pending_range.is_some() {
        return Err(invalid("incomplete HLS playlist"));
    }
    if !variants.is_empty() {
        if media_seen {
            return Err(invalid("mixed HLS master/media playlist"));
        }
        return Ok(Playlist::Master(variants));
    }
    if !endlist {
        return Err(invalid("live HLS without ENDLIST is unsupported"));
    }
    if fragments.len() <= usize::from(has_map) {
        return Err(invalid("HLS playlist has no media segments"));
    }
    if !groups.is_empty() {
        return Err(invalid("HLS media playlist contains rendition groups"));
    }
    Ok(Playlist::Media {
        fragments,
        ext: ext.unwrap_or("ts").into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn base() -> Url {
        Url::parse("https://example.com/path/list.m3u8?token=one").unwrap()
    }
    fn media(text: &str) -> (Vec<Fragment>, String) {
        match parse(text, &base()).unwrap() {
            Playlist::Media { fragments, ext } => (fragments, ext),
            _ => panic!("expected media playlist"),
        }
    }
    #[test]
    fn vod_segments_preserve_order_and_resolve_relative_urls() {
        let (fragments, ext) = media(
            "\u{feff}#EXTM3U\r\n#EXT-X-VERSION:3\r\n#EXT-X-TARGETDURATION:6\r\n#EXT-X-MEDIA-SEQUENCE:12\r\n#EXTINF:5.25,title\r\na.ts?sig=x\r\n# comment\r\n#EXTINF:4,\r\n../b.ts\r\n#EXT-X-ENDLIST\r\n",
        );
        assert_eq!(ext, "ts");
        assert_eq!(fragments.len(), 2);
        assert_eq!(
            fragments[0].url.as_str(),
            "https://example.com/path/a.ts?sig=x"
        );
        assert_eq!(fragments[1].url.as_str(), "https://example.com/b.ts");
        assert!(fragments.iter().all(|f| f.byte_range.is_none()));
        assert_eq!(
            media("#EXTM3U\n#EXTINF:1,\nchunk?id=1\n#EXT-X-ENDLIST").1,
            "ts"
        );
        assert_eq!(
            media("#EXTM3U\n#EXT-X-KEY:METHOD=NONE\n#EXTINF:1,\na.aac\n#EXT-X-ENDLIST").1,
            "aac"
        );
    }
    #[test]
    fn master_quoted_codecs_dimensions_bandwidth_and_audio_kind() {
        let Playlist::Master(variants) = parse("#EXTM3U\n#EXT-X-INDEPENDENT-SEGMENTS\n#EXT-X-STREAM-INF:BANDWIDTH=900000,AVERAGE-BANDWIDTH=800000,RESOLUTION=1280x720,CODECS=\"avc1.64001F,mp4a.40.2\"\nvideo/playlist.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=128000,CODECS=\"mp4a.40.2\"\naudio.m3u8", &base()).unwrap() else { panic!() };
        assert_eq!(variants.len(), 2);
        assert_eq!(
            variants[0].url.as_str(),
            "https://example.com/path/video/playlist.m3u8"
        );
        assert_eq!(variants[0].bandwidth, Some(800.0));
        assert_eq!(
            (variants[0].width, variants[0].height),
            (Some(1280), Some(720))
        );
        assert_eq!(variants[0].stream_kind, StreamKind::Standalone);
        assert_eq!(variants[1].stream_kind, StreamKind::AudioOnly);
        assert_eq!(variants[1].bandwidth, Some(128.0));
    }
    #[test]
    fn master_inband_groups_are_resolved_even_when_declared_later() {
        let Playlist::Master(variants) = parse("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000,AUDIO=\"a\",CLOSED-CAPTIONS=NONE,CODECS=\"avc1.4,mp4a.40.2\"\nv.m3u8\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"Audio, original\"", &base()).unwrap() else { panic!() };
        assert_eq!(variants[0].stream_kind, StreamKind::Standalone);
    }
    #[test]
    fn fmp4_map_precedes_explicit_and_implicit_ranges() {
        let (fragments, ext) = media(
            "#EXTM3U\n#EXT-X-MAP:URI=\"all.mp4\",BYTERANGE=\"20@0\"\n#EXTINF:1,\n#EXT-X-BYTERANGE:10@20\nall.mp4\n#EXTINF:1,\n#EXT-X-BYTERANGE:15\nall.mp4\n#EXT-X-ENDLIST",
        );
        assert_eq!(ext, "mp4");
        assert_eq!(fragments.len(), 3);
        let ranges: Vec<_> = fragments
            .iter()
            .map(|f| {
                let r = f.byte_range.as_ref().unwrap();
                (r.offset, r.length)
            })
            .collect();
        assert_eq!(ranges, [(0, 20), (20, 10), (30, 15)]);
        assert_eq!(
            media("#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:1,\na.m4s\n#EXT-X-ENDLIST").1,
            "mp4"
        );
    }
    #[test]
    fn rejects_unsafe_or_unsupported_state_and_incomplete_playlists() {
        let valid = "#EXTM3U\n#EXTINF:1,\na.ts\n#EXT-X-ENDLIST";
        for tag in [
            "#EXT-X-KEY:METHOD=AES-128,URI=\"key\"",
            "#EXT-X-KEY:METHOD=SAMPLE-AES",
            "#EXT-X-KEY:METHOD=NONE,URI=\"key\"",
            "#EXT-X-DISCONTINUITY",
            "#EXT-X-DISCONTINUITY-SEQUENCE:0",
            "#EXT-X-GAP",
            "#EXT-X-PART:DURATION=1,URI=\"a.ts\"",
            "#EXT-X-PRELOAD-HINT:TYPE=PART,URI=\"a\"",
            "#EXT-X-SKIP:SKIPPED-SEGMENTS=1",
            "#EXT-X-UNKNOWN:1",
            "#EXT-X-START:TIME-OFFSET=2",
        ] {
            let text = valid.replace("#EXTINF", &format!("{tag}\n#EXTINF"));
            assert!(parse(&text, &base()).is_err(), "{tag}");
        }
        for text in [
            "not a manifest",
            "#EXTM3U",
            "#EXTM3U\n#EXTINF:1,\na.ts",
            "#EXTM3U\n#EXT-X-ENDLIST",
            "#EXTM3U\n#EXTINF:1,\n#EXT-X-ENDLIST",
            "#EXTM3U\na.ts\n#EXT-X-ENDLIST",
            "#EXTM3U\n#EXTINF:NaN,\na.ts\n#EXT-X-ENDLIST",
            "#EXTM3U\n#EXTINF:0,\na.ts\n#EXT-X-ENDLIST",
            "#EXTM3U\n#EXTINF:1,\nfile:///a.ts\n#EXT-X-ENDLIST",
            "#EXTM3U\n#EXTINF:1,\na.m3u8\n#EXT-X-ENDLIST",
            "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000",
            "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000\na.m3u8\n#EXTINF:1,\na.ts\n#EXT-X-ENDLIST",
        ] {
            assert!(parse(text, &base()).is_err(), "{text}");
        }
    }
    #[test]
    fn endlist_is_terminal_and_media_keys_cannot_appear_in_master() {
        let text = "#EXTM3U\n#EXTINF:1,\na.ts\n#EXT-X-ENDLIST";
        assert!(parse(&format!("{text}\n# trailing comment"), &base()).is_ok());
        for trailing in [
            "#EXT-X-KEY:METHOD=NONE",
            "#EXT-X-VERSION:3",
            "#EXT-X-ENDLIST",
            "b.ts",
        ] {
            assert!(parse(&format!("{text}\n{trailing}"), &base()).is_err());
        }
        assert!(
            parse(
                "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000\na.m3u8\n#EXT-X-KEY:METHOD=NONE",
                &base()
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_external_and_unresolved_rendition_groups() {
        for group in ["AUDIO", "SUBTITLES", "VIDEO", "CLOSED-CAPTIONS"] {
            let text = format!(
                "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000,{group}=\"a\"\na.m3u8\n#EXT-X-MEDIA:TYPE={group},GROUP-ID=\"a\",NAME=\"A\",URI=\"extra.m3u8\""
            );
            assert!(parse(&text, &base()).is_err());
            let text =
                format!("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000,{group}=\"missing\"\na.m3u8");
            assert!(parse(&text, &base()).is_err());
        }
    }
    #[test]
    fn rejects_invalid_ranges_maps_and_container_changes() {
        for body in [
            "#EXT-X-BYTERANGE:10\n#EXTINF:1,\na.ts",
            "#EXT-X-BYTERANGE:0@0\n#EXTINF:1,\na.ts",
            "#EXT-X-BYTERANGE:2@18446744073709551615\n#EXTINF:1,\na.ts",
            "#EXT-X-BYTERANGE:1@0@1\n#EXTINF:1,\na.ts",
            "#EXTINF:1,\n#EXT-X-BYTERANGE:10@0\na.ts\n#EXTINF:1,\n#EXT-X-BYTERANGE:10\nb.ts",
            "#EXT-X-MAP:URI=\"a.mp4\",BYTERANGE=\"10\"\n#EXTINF:1,\na.m4s",
            "#EXT-X-MAP:URI=\"a.mp4\",BYTERANGE=\"10@0\"\n#EXTINF:1,\n#EXT-X-BYTERANGE:10\na.mp4",
            "#EXTINF:1,\na.ts\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:1,\nb.m4s",
            "#EXT-X-MAP:URI=\"init.mp4\"\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:1,\na.m4s",
            "#EXTINF:1,\na.aac\n#EXTINF:1,\nb.ts",
        ] {
            let text = format!("#EXTM3U\n{body}\n#EXT-X-ENDLIST");
            assert!(parse(&text, &base()).is_err(), "{body}");
        }
    }
    #[test]
    fn malformed_attributes_and_unsupported_codecs_fail_closed() {
        for attributes in [
            "CODECS=\"avc1,mp4a",
            "BANDWIDTH=1000,BANDWIDTH=2000",
            "BANDWIDTH=1000,",
            "CODECS=\"unknown\"",
            "RESOLUTION=0x720",
            "RESOLUTION=1280",
            "BANDWIDTH=1000 junk",
            "CODECS=\"avc1\"junk",
            "BANDWIDTH=1,,CODECS=\"avc1\"",
        ] {
            assert!(
                parse(
                    &format!("#EXTM3U\n#EXT-X-STREAM-INF:{attributes}\na.m3u8"),
                    &base()
                )
                .is_err(),
                "{attributes}"
            );
        }
    }
    #[test]
    fn enforces_fragment_and_variant_resource_limits() {
        let media = format!(
            "#EXTM3U\n{}#EXT-X-ENDLIST",
            "#EXTINF:1,\na.ts\n".repeat(MAX_FRAGMENTS)
        );
        assert!(parse(&media, &base()).is_ok());
        let media = media.replace("#EXT-X-ENDLIST", "#EXTINF:1,\nb.ts\n#EXT-X-ENDLIST");
        assert!(parse(&media, &base()).is_err());
        let master = format!(
            "#EXTM3U\n{}",
            "#EXT-X-STREAM-INF:BANDWIDTH=1000\na.m3u8\n".repeat(MAX_VARIANTS)
        );
        assert!(parse(&master, &base()).is_ok());
        assert!(
            parse(
                &format!("{master}#EXT-X-STREAM-INF:BANDWIDTH=1000\nb.m3u8"),
                &base()
            )
            .is_err()
        );
    }
}
