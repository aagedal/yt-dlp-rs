//! Port extension: a DASH `SegmentBase` representation (one file with an MP4
//! `sidx` or WebM `Cues` index) downloads as byte-range fragments instead of
//! one HTTP transfer. Upstream (and this port before) reads the whole file
//! with HttpFD; the fragments give per-segment retries, `.ytdl` resume,
//! `--concurrent-fragments` and fragment progress, and together cover the
//! file exactly (its head, every indexed segment, and any tail), so the
//! result is byte-identical. An index this port cannot use, a gap between
//! segments or an unknown length keeps the whole-file download.
use crate::{
    ByteRange, DownloadOptions, Fragment, Result, VideoInfo,
    download::{ByteSpan, fetch_span},
    media_index,
};
use reqwest::blocking::Client;

/// The fragments covering `info`'s file, when it is a DASH SegmentBase
/// format with a usable, contiguous index.
pub(crate) fn plan(
    client: &Client,
    info: &VideoInfo,
    options: &DownloadOptions,
) -> Result<Option<Vec<Fragment>>> {
    if !info.requested_formats.is_empty() {
        return Ok(None);
    }
    let Some(format) = info
        .formats
        .iter()
        .find(|format| info.format_id.as_deref() == Some(format.format_id.as_str()))
    else {
        return Ok(None);
    };
    plan_format(client, info, format, options)
}

/// [`plan`] for `format`, the selected format (or merged stream) of `info`.
pub(crate) fn plan_format(
    client: &Client,
    info: &VideoInfo,
    format: &crate::VideoFormat,
    options: &DownloadOptions,
) -> Result<Option<Vec<Fragment>>> {
    let http = matches!(info.url.scheme(), "http" | "https")
        && matches!(
            info.protocol.as_deref(),
            None | Some("http") | Some("https")
        );
    let segment_base = format.url == info.url
        && format.extra.get(crate::info_order::FORMAT_SHAPE)
            == Some(&serde_json::Value::from("dash-direct"));
    if !http || !segment_base || !info.fragments.is_empty() {
        return Ok(None);
    }
    let Some(ranges) = format.byte_index else {
        return Ok(None);
    };
    let referrer = (!info.is_direct()).then_some(&info.webpage_url);
    let head_start = ranges.init.0.min(ranges.index.0);
    let mut head = Vec::new();
    let total = match fetch_span(
        client,
        &ByteSpan {
            url: &info.url,
            referrer,
            start: head_start,
            end: ranges.init.1.max(ranges.index.1),
            chunk: u64::MAX,
            rate_limit: options.rate_limit,
        },
        options,
        &mut head,
        &mut |_| Ok(()),
    ) {
        Ok(total) => total,
        // A server without ranged responses: one whole-file transfer.
        Err(crate::Error::InvalidRangeResponse) => return Ok(None),
        Err(error) => return Err(error),
    };
    let Some(total) = total.or(format.filesize) else {
        return Ok(None);
    };
    let Some(media) = media_index::parse(&head, head_start, ranges, Some(total)) else {
        return Ok(None);
    };
    Ok(cover(&media.segments, total).map(|spans| {
        crate::screen::debug(&format!(
            "Downloading format {} as {} byte-range fragments from its segment index",
            format.format_id,
            spans.len()
        ));
        spans
            .into_iter()
            .map(|(start, end)| Fragment {
                encryption: None,
                url: info.url.clone(),
                byte_range: Some(ByteRange {
                    offset: start,
                    length: end - start + 1,
                }),
                path: None,
                duration: None,
                fragment_count: None,
            })
            .collect()
    }))
}

/// Inclusive byte spans covering `0..total` exactly: the bytes before the
/// first segment, each segment, and the bytes after the last one. `None`
/// unless the segments are contiguous and inside the file.
fn cover(segments: &[media_index::Segment], total: u64) -> Option<Vec<(u64, u64)>> {
    let (first, last) = (segments.first()?, segments.last()?);
    if last.end >= total
        || segments
            .windows(2)
            .any(|pair| pair[1].start != pair[0].end + 1)
    {
        return None;
    }
    let mut spans = Vec::with_capacity(segments.len() + 2);
    if first.start > 0 {
        spans.push((0, first.start - 1));
    }
    spans.extend(segments.iter().map(|segment| (segment.start, segment.end)));
    if last.end + 1 < total {
        spans.push((last.end + 1, total - 1));
    }
    // A single span is the whole-file download already.
    (spans.len() > 1).then_some(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(start: u64, end: u64) -> media_index::Segment {
        media_index::Segment {
            start,
            end,
            time: 0.0,
        }
    }

    #[test]
    fn spans_cover_the_file_exactly() {
        let segments = [segment(100, 199), segment(200, 349)];
        assert_eq!(
            cover(&segments, 400),
            Some(vec![(0, 99), (100, 199), (200, 349), (350, 399)])
        );
        assert_eq!(
            cover(&segments, 350),
            Some(vec![(0, 99), (100, 199), (200, 349)])
        );
        // Gaps, overruns and a lone span keep the whole-file download.
        assert_eq!(cover(&[segment(100, 199), segment(250, 349)], 400), None);
        assert_eq!(cover(&segments, 300), None);
        assert_eq!(cover(&[segment(0, 99)], 100), None);
        assert_eq!(cover(&[], 100), None);
    }
}
