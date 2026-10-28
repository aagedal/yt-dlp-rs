//! Upstream `InfoExtractor._extract_akamai_formats_and_subtitles` without
//! `hosts`: an Akamai stream's HDS formats (with `hdcore`) and HLS formats.
use super::{extract_f4m_optional, extract_m3u8_master_optional};
use crate::{Result, Subtitles, VideoFormat};
use regex::Regex;
use reqwest::blocking::Client;
use std::sync::LazyLock;
use url::Url;

/// Akamai's stream packaging parameters, removed from unsigned URLs.
static PARAMS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:b=[\d,-]+|(?:__a__|attributes)=off|__b__=\d+)&?").unwrap());
static HLS_PATH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(https?://[^/]+)/i/").unwrap());
static HDS_PATH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(https?://[^/]+)/z/").unwrap());

const HDCORE_SIGN: &str = "hdcore=3.7.0";

/// `_extract_akamai_formats_and_subtitles(manifest_url, video_id)` without
/// `hosts`: the non-fatal HDS formats (`hds`, with `hdcore`) and the
/// non-fatal HLS formats (`hls`, master playlist only) with their subtitles.
pub(crate) fn formats_and_subtitles(
    client: &Client,
    manifest_url: &str,
    extractor: &'static str,
    id: &str,
) -> Result<(Vec<VideoFormat>, Subtitles)> {
    let manifest_url = if manifest_url.contains("hdnea=") {
        manifest_url.to_owned()
    } else {
        PARAMS
            .replace_all(manifest_url, "")
            .trim_matches('?')
            .to_owned()
    };
    let mut formats = Vec::new();
    let mut subtitles = Subtitles::default();
    let mut f4m_url = HLS_PATH
        .replace(&manifest_url, "${1}/z/")
        .replace("/master.m3u8", "/manifest.f4m");
    if !f4m_url.contains("hdcore=") {
        f4m_url.push(if f4m_url.contains('?') { '&' } else { '?' });
        f4m_url.push_str(HDCORE_SIGN);
    }
    let f4m_url = Url::parse(&f4m_url)?;
    for mut format in extract_f4m_optional(client, extractor, &f4m_url, id, Some("hds"))? {
        format
            .extra
            .insert("extra_param_to_segment_url".into(), HDCORE_SIGN.into());
        crate::info_order::mark_format_update(&mut format, "extra_param_to_segment_url");
        formats.push(format);
    }
    let m3u8_url = HDS_PATH
        .replace(&manifest_url, "${1}/i/")
        .replace("/manifest.f4m", "/master.m3u8");
    let m3u8_url = Url::parse(&m3u8_url)?;
    if let Some(stream) =
        extract_m3u8_master_optional(client, &m3u8_url, extractor, id, Some("hls"), true, false)
    {
        formats.extend(stream.formats);
        super::svt::merge_subtitles(&mut subtitles, stream.subtitles);
    }
    Ok((formats, subtitles))
}

/// `_extract_akamai_formats(manifest_url, video_id)`: the subtitles dropped
/// with `_report_ignoring_subs('akamai')`.
pub(crate) fn formats(
    client: &Client,
    manifest_url: &str,
    extractor: &'static str,
    id: &str,
) -> Result<Vec<VideoFormat>> {
    let (formats, subtitles) = formats_and_subtitles(client, manifest_url, extractor, id)?;
    if !subtitles.is_empty() {
        crate::screen::warning_once(&format!(
            "[{extractor}] Ignoring subtitle tracks found in the akamai manifest; if any subtitle tracks are missing, {}",
            crate::BUG_REPORTS_MESSAGE
        ));
    }
    Ok(formats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameters_are_removed_from_unsigned_urls() {
        let url =
            "https://x.akamaihd.net/i/a/,b,c,.mp4.csmil/master.m3u8?__b__=800&b=500-1000&__a__=off";
        let stripped = PARAMS.replace_all(url, "");
        assert_eq!(
            stripped.trim_matches('?'),
            "https://x.akamaihd.net/i/a/,b,c,.mp4.csmil/master.m3u8"
        );
    }
}
