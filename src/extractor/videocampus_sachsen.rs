//! Upstream `VideocampusSachsenIE` (`yt_dlp/extractor/videocampus_sachsen.py`,
//! IE_NAME `ViMP`): a single video of a ViMP instance, from its `/m/` page,
//! `/video/NAME/ID` page or `/media/embed?key=ID` player. Its HLS master
//! playlist (`hlsMedium`, an HTTP 404 or 500 means none) and the `getMedium`
//! MP4 are the formats. ViMP's playlist pages (`ViMP:Playlist`) are not ported.
use super::{Extractor, dict_format, dict_info, download_webpage, search_warning};
use crate::html::{clean_html, html_search_meta_clean};
use crate::{Error, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct VideocampusSachsen;

const NAME: &str = "ViMP";
const KEY: &str = "VideocampusSachsen";

/// The ViMP instances (upstream `_INSTANCES`), in upstream order.
const INSTANCES: &[&str] = &[
    "bergauf.tv",
    "campus.demo.vimp.com",
    "corporate.demo.vimp.com",
    "dancehalldatabase.com",
    "drehzahl.tv",
    "educhannel.hs-gesundheit.de",
    "emedia.ls.haw-hamburg.de",
    "globale-evolution.net",
    "hohu.tv",
    "htvideos.hightechhigh.org",
    "k210039.vimp.mivitec.net",
    "media.cmslegal.com",
    "media.fh-swf.de",
    "media.hs-furtwangen.de",
    "media.hwr-berlin.de",
    "mediathek.dkfz.de",
    "mediathek.htw-berlin.de",
    "mediathek.polizei-bw.de",
    "medien.hs-merseburg.de",
    "mitmedia.manukau.ac.nz",
    "mportal.europa-uni.de",
    "pacific.demo.vimp.com",
    "slctv.com",
    "streaming.prairiesouth.ca",
    "tube.isbonline.cn",
    "univideo.uni-kassel.de",
    "ursula2.genetics.emory.edu",
    "ursulablicklevideoarchiv.com",
    "v.agrarumweltpaedagogik.at",
    "video.eplay-tv.de",
    "video.fh-dortmund.de",
    "video.hs-nb.de",
    "video.hs-offenburg.de",
    "video.hs-pforzheim.de",
    "video.hspv.nrw.de",
    "video.irtshdf.fr",
    "video.pareygo.de",
    "video.tu-dortmund.de",
    "video.tu-freiberg.de",
    "videocampus.sachsen.de",
    "videoportal.uni-freiburg.de",
    "videoportal.vm.uni-freiburg.de",
    "videos.duoc.cl",
    "videos.uni-paderborn.de",
    "vimp-bemus.udk-berlin.de",
    "vimp.aekwl.de",
    "vimp.hs-mittweida.de",
    "vimp.landesfilmdienste.de",
    "vimp.oth-regensburg.de",
    "vimp.ph-heidelberg.de",
    "vimp.sma-events.com",
    "vimp.weka-fachmedien.de",
    "vimpdesk.com",
    "webtv.univ-montp3.fr",
    "www.b-tu.de/media",
    "www.bergauf.tv",
    "www.bigcitytv.de",
    "www.cad-videos.de",
    "www.drehzahl.tv",
    "www.hohu.tv",
    "www.hsbi.de/medienportal",
    "www.logistic.tv",
    "www.orvovideo.com",
    "www.printtube.co.uk",
    "www.rwe.tv",
    "www.salzi.tv",
    "www.signtube.co.uk",
    "www.twb-power.com",
    "www.wenglor-media.com",
    "www2.univ-sba.dz",
];

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    let hosts = INSTANCES
        .iter()
        .map(|host| regex::escape(host))
        .collect::<Vec<_>>()
        .join("|");
    crate::extractor::url_regex(&format!(
        r"^https?://(?P<host>{hosts})/(?:m/(?P<tmp_id>[0-9a-f]+)|(?:category/)?video/(?P<display_id>[\w-]+)/(?P<id>[0-9a-f]{{32}})|media/embed.*(?:\?|&)key=(?P<embed_id>[0-9a-f]{{32}}&?))"
    ))
    .unwrap()
});
static PIWIK_TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<video-js[^>]* data-piwik-title="([^"<]+)""#).unwrap());

/// `_match_id`-style check of the upstream `_VALID_URL`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_html_search_meta(names, webpage, fatal=False)`: the first name present,
/// else the `unable to extract` warning naming the first.
fn meta(webpage: &str, names: &[&str]) -> Option<String> {
    let found = names
        .iter()
        .find_map(|name| html_search_meta_clean(webpage, name));
    if found.is_none() {
        search_warning(NAME, names[0]);
    }
    found
}

/// `_download_webpage(url, id, fatal=False) or ''`: an HTTP error warns and
/// leaves the page empty.
fn optional_webpage(client: &Client, url: &Url, id: &str) -> Result<String> {
    match download_webpage(client, url, NAME, id) {
        Ok(text) => Ok(text),
        Err(Error::ExtractorRequest { source, .. }) => {
            crate::screen::warning(&format!(
                "[{NAME}] {id}: Unable to download webpage: {}",
                crate::upstream_error_text(&source)
            ));
            Ok(String::new())
        }
        Err(error) => Err(error),
    }
}

/// The HTTP status of a request error (through the page download's wrapper).
fn http_status(error: &Error) -> Option<u16> {
    match error {
        Error::ExtractorRequest { source, .. } => http_status(source),
        Error::Http(source) | Error::HttpRetryAfter { source, .. } => {
            source.status().map(|status| status.as_u16())
        }
        _ => None,
    }
}

/// `_extract_m3u8_formats_and_subtitles(..., m3u8_id='hls', fatal=True)`
/// that ignores an HTTP 404 or 500 from the manifest (the stream is absent).
fn hls_stream(client: &Client, manifest: &Url, id: &str) -> Result<Option<VideoInfo>> {
    let note = format!("[{NAME}] {id}: Downloading m3u8 information");
    crate::screen::expect_page(id, Some(note.clone()), None);
    let response = super::direct::request(client, manifest, None);
    if response.is_err() {
        crate::screen::screen(&note);
    }
    let parsed = match response {
        Ok(response) => super::hls::from_response_with(
            client,
            response,
            manifest,
            None,
            true,
            Some("hls"),
            true,
        ),
        Err(error) if matches!(http_status(&error), Some(404 | 500)) => {
            crate::screen::clear_page();
            return Ok(None);
        }
        Err(error) => Err(Error::Reported(
            super::request_error_text(NAME, id, "m3u8 information", &error).replacen(
                ": Unable to download ",
                ": Failed to download ",
                1,
            ),
        )),
    };
    crate::screen::clear_page();
    parsed?
        .map(Some)
        .ok_or_else(|| Error::UnsupportedHtml5(format!("the {NAME} stream is not an HLS playlist")))
}

impl Extractor for VideocampusSachsen {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let host = found["host"].to_owned();
        let tmp_id = found.name("tmp_id").map(|value| value.as_str().to_owned());
        let display_id = found
            .name("display_id")
            .map(|value| value.as_str().to_owned());
        let video_id = found.name("id").map(|value| value.as_str().to_owned());
        let embed_id = found
            .name("embed_id")
            .map(|value| value.as_str().to_owned());
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let page_id = video_id
            .clone()
            .or_else(|| tmp_id.clone())
            .unwrap_or_default();
        let webpage = optional_webpage(client, url, &page_id)?;

        let video_id = match video_id.or(embed_id) {
            Some(id) => id,
            None => {
                let pattern = Regex::new(&format!(
                    r#"src="https?://{}/media/embed.*(?:\?|&)key=([0-9a-f]+)&?"#,
                    regex::escape(&host)
                ))
                .map_err(|_| Error::UnsupportedHtml5("ViMP hosts that do not parse".into()))?;
                match pattern.captures(&webpage).and_then(|found| found.get(1)) {
                    Some(id) => id.as_str().to_owned(),
                    None => {
                        return Err(Error::Reported(format!(
                            "[{NAME}] Unable to extract video_id; {}",
                            crate::BUG_REPORTS_MESSAGE
                        )));
                    }
                }
            }
        };

        let (title, description, thumbnail) = if display_id.is_none() && tmp_id.is_none() {
            // The embedded page's meta would not be the video's.
            let title = PIWIK_TITLE
                .captures(&webpage)
                .and_then(|found| found.get(1))
                .map(|value| clean_html(value.as_str()));
            if title.is_none() {
                search_warning(NAME, "title");
            }
            (title, None, None)
        } else {
            let title = meta(&webpage, &["og:title", "twitter:title", "title"]);
            let description = meta(
                &webpage,
                &["og:description", "twitter:description", "description"],
            );
            let thumbnail = meta(&webpage, &["og:image", "twitter:image"]);
            (title, description, thumbnail)
        };

        let manifest = Url::parse(&format!(
            "https://{host}/media/hlsMedium/key/{video_id}/format/auto/ext/mp4/learning/0/path/m3u8"
        ))?;
        let (mut formats, subtitles) = match hls_stream(client, &manifest, &video_id)? {
            Some(stream) => (stream.formats, stream.subtitles),
            None => (Vec::<VideoFormat>::new(), Default::default()),
        };
        let medium = format!("https://{host}/getMedium/{video_id}.mp4");
        formats.push(dict_format(
            vec![("url", Value::from(medium))],
            NAME,
            &video_id,
        )?);

        let mut info = dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", video_id.clone().into()),
                ("title", title.map_or(Value::Null, Value::from)),
                ("description", description.map_or(Value::Null, Value::from)),
                ("thumbnail", thumbnail.map_or(Value::Null, Value::from)),
                ("display_id", display_id.unwrap_or(video_id).into()),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
            ],
            formats,
        );
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://videocampus.sachsen.de/m/e0d6c8ce6e394c188f1342f1ab7c50ed6fc4490b808699801def5cb2e46d76ca7367f622a9f516c542ffb805b24d6b643bd7c81f385acaac4c59081b87a2767b"
        ));
        assert!(suitable(
            "https://videocampus.sachsen.de/video/Was-ist-selbstgesteuertes-Lernen/fc99c527e4205b121cb7c74433469262"
        ));
        assert!(suitable(
            "https://www2.univ-sba.dz/video/Presentation/0183356e41af7bfb83d7667b20d9b6a3"
        ));
        assert!(suitable(
            "https://videocampus.sachsen.de/media/embed?key=fc99c527e4205b121cb7c74433469262"
        ));
        assert!(!suitable(
            "https://videocampus.sachsen.de/category/online-tutorials-onyx/91"
        ));
        assert!(!suitable(
            "https://example.com/video/x/0123456789abcdef0123456789abcdef"
        ));
    }
}
