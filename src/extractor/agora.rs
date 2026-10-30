//! Upstream `WyborczaVideoIE`, `WyborczaPodcastIE`, `TokFMPodcastIE` and
//! `TokFMAuditionIE` (`yt_dlp/extractor/agora.py`): Wyborcza's video API
//! (MP4 renditions and the DASH manifest), the Wyborcza and TOK FM podcast
//! APIs as direct audio, and a TOK FM audition's paged episode list as
//! `url_transparent` entries (paged eagerly, one page of 30 at a time).
use super::{
    Extractor, dict_format, dict_info, download_json, download_json_headers, download_json_note,
    streamable::int_or_none, update_url_query,
};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct WyborczaVideo;
pub struct WyborczaPodcast;
pub struct TokFmPodcast;
pub struct TokFmAudition;

/// Each extractor's `IE_NAME` (the `extractor` field) and its key.
const VIDEO_NAME: &str = "wyborcza:video";
const VIDEO_KEY: &str = "WyborczaVideo";
const PODCAST_NAME: &str = "WyborczaPodcast";
const TOKFM_PODCAST_NAME: &str = "tokfm:podcast";
const TOKFM_PODCAST_KEY: &str = "TokFMPodcast";
const TOKFM_AUDITION_NAME: &str = "tokfm:audition";
const TOKFM_AUDITION_KEY: &str = "TokFMAudition";
const PAGE_SIZE: usize = 30;
const TOKFM_HEADERS: [(&str, &str); 1] = [(
    "User-Agent",
    "Mozilla/5.0 (Linux; Android 9; Redmi 3S Build/PQ3A.190801.002; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/87.0.4280.101 Mobile Safari/537.36",
)];

static VIDEO_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:wyborcza:video:|https?://wyborcza\.pl/(?:api-)?video/)(?P<id>\d+)").unwrap()
});
static PODCAST_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"https?://(?:www\.)?(?:wyborcza\.pl/podcast(?:/0,172673\.html)?|wysokieobcasy\.pl/wysokie-obcasy/0,176631\.html)(?:\?(?:[^&#]+?&)*podcast=(?P<id>\d+))?",
    )
    .unwrap()
});
static TOKFM_PODCAST_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:https?://audycje\.tokfm\.pl/podcast/|tokfm:podcast:)(?P<id>\d+),?").unwrap()
});
static TOKFM_AUDITION_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:https?://audycje\.tokfm\.pl/audycja/|tokfm:audition:)(?P<id>\d+),?").unwrap()
});
static MP4_HEIGHT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"p(\d+)[a-z]+\.mp4$").unwrap());
static PUBLISHED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d\d?) (\w+) (\d{4})$").unwrap());

/// `_match_valid_url` of [`WyborczaVideo`].
pub fn video_suitable(url: &str) -> bool {
    VIDEO_URL.is_match(url)
}

/// `_match_valid_url` of [`WyborczaPodcast`].
pub fn podcast_suitable(url: &str) -> bool {
    PODCAST_URL.is_match(url)
}

/// `_match_valid_url` of [`TokFmPodcast`].
pub fn tokfm_podcast_suitable(url: &str) -> bool {
    TOKFM_PODCAST_URL.is_match(url)
}

/// `_match_valid_url` of [`TokFmAudition`].
pub fn tokfm_audition_suitable(url: &str) -> bool {
    TOKFM_AUDITION_URL.is_match(url)
}

fn key_error(id: &str, key: &str) -> Error {
    super::clyp::key_error(id, key)
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// `month_by_name(name, lang='pl')`: the month's number from its genitive
/// name.
fn polish_month(name: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = [
        "stycznia",
        "lutego",
        "marca",
        "kwietnia",
        "maja",
        "czerwca",
        "lipca",
        "sierpnia",
        "września",
        "października",
        "listopada",
        "grudnia",
    ];
    MONTHS
        .iter()
        .position(|month| *month == name)
        .map(|index| index as u32 + 1)
}

/// `try_call(lambda: f'{year}{month:0>2}{day:0>2}')` of a `publishedDate`
/// such as `29 października 2026`.
fn upload_date(published: Option<&Value>) -> Value {
    let Some(text) = published.and_then(Value::as_str) else {
        return Value::Null;
    };
    let Some(found) = PUBLISHED.captures(text) else {
        return Value::Null;
    };
    match polish_month(&found[2]) {
        Some(month) => format!("{}{month:02}{:0>2}", &found[3], &found[1]).into(),
        None => Value::Null,
    }
}

/// A direct-audio info (`url` as its one format) whose dict keys follow
/// `fields`: the dict's keys with their `None` values kept.
fn direct_info(
    page: &Url,
    media: &Url,
    ext: &str,
    (extractor, key): (&str, &str),
    fields: Vec<(&str, Value)>,
) -> VideoInfo {
    let mut info = super::direct::with_format(
        super::direct::build_at(media.clone(), media, ext.to_owned()),
        "",
    );
    // The dict's own `vcodec` (a `none` audio track) also marks the format.
    let audio_only = fields
        .iter()
        .any(|(key, value)| *key == "vcodec" && value.as_str() == Some("none"));
    for format in &mut info.formats {
        format.ext = ext.to_owned();
        format.format_id = String::new();
        if audio_only {
            format.vcodec = Some("none".into());
        }
    }
    info.format_id = None;
    info.webpage_url = page.clone();
    info.direct = None;
    info.extractor = extractor.to_owned();
    info.extractor_key = key.to_owned();
    let mut keys = Vec::new();
    for (name, value) in fields {
        keys.push(name.to_owned());
        match name {
            "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
            "title" => info.title = value.as_str().unwrap_or_default().to_owned(),
            "description" => info.metadata.description = value.as_str().map(str::to_owned),
            "thumbnail" => info.metadata.thumbnail = value.as_str().map(str::to_owned),
            _ => crate::metadata_parser::set_field(&mut info, name, value),
        }
    }
    crate::info_order::mark_info(info, &format!("untyped-dict;{}", keys.join(",")))
}

/// `url_result(target, ie=..)` as a playlist entry, with the
/// `url_transparent` dict's `fields` (in order) set on the target's result.
fn url_transparent_entry(
    target: &str,
    (ie_key, extractor): (&'static str, &'static str),
    fields: Vec<(&str, Value)>,
) -> Result<VideoInfo> {
    let url = super::target_url(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    entry.insert("_type".into(), "url_transparent".into());
    entry.insert("url".into(), target.into());
    entry.insert("ie_key".into(), ie_key.into());
    for (key, value) in fields {
        entry.insert(key.into(), value);
    }
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": ie_key,
            "extractor": extractor,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

impl Extractor for WyborczaVideo {
    fn key(&self) -> &'static str {
        VIDEO_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VIDEO_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        announce(VIDEO_NAME, url);
        let meta = download_json(
            client,
            &Url::parse(&format!("https://wyborcza.pl/api-video/{video_id}"))?,
            VIDEO_NAME,
            &video_id,
        )?;
        let base = meta
            .get("redirector")
            .and_then(Value::as_str)
            .ok_or_else(|| key_error(&video_id, "redirector"))?
            .replacen("http://", "https://", 1)
            + meta
                .get("basePath")
                .and_then(Value::as_str)
                .ok_or_else(|| key_error(&video_id, "basePath"))?;
        let files = meta
            .get("files")
            .ok_or_else(|| key_error(&video_id, "files"))?;

        let mut formats = Vec::new();
        for quality in ["standard", "high"] {
            let Some(file) = files.get(quality).and_then(Value::as_str) else {
                continue;
            };
            let height = MP4_HEIGHT
                .captures(file)
                .and_then(|found| found[1].parse::<i64>().ok());
            formats.push(dict_format(
                vec![
                    ("url", format!("{base}{file}").into()),
                    ("height", height.map_or(Value::Null, Value::from)),
                    ("format_id", quality.into()),
                ],
                VIDEO_NAME,
                &video_id,
            )?);
        }
        if let Some(dash) = files.get("dash").and_then(Value::as_str) {
            let manifest = Url::parse(&format!("{base}{dash}"))?;
            formats.extend(
                super::extract_mpd_fatal(client, &manifest, VIDEO_NAME, &video_id, false)?.formats,
            );
        }

        Ok(Some(dict_info(
            url,
            (VIDEO_NAME, VIDEO_KEY),
            vec![
                ("id", video_id.clone().into()),
                ("formats", Value::Null),
                ("title", meta.get("title").cloned().unwrap_or(Value::Null)),
                (
                    "description",
                    meta.get("lead").cloned().unwrap_or(Value::Null),
                ),
                (
                    "uploader",
                    meta.get("signature").cloned().unwrap_or(Value::Null),
                ),
                (
                    "thumbnail",
                    meta.get("imageUrl").cloned().unwrap_or(Value::Null),
                ),
                (
                    "duration",
                    meta.get("duration").cloned().unwrap_or(Value::Null),
                ),
            ],
            formats,
        )))
    }
}

impl Extractor for WyborczaPodcast {
    fn key(&self) -> &'static str {
        PODCAST_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PODCAST_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        announce(PODCAST_NAME, url);
        let Some(podcast_id) = found.name("id").map(|id| id.as_str().to_owned()) else {
            // The playlist of the audition the page embeds (upstream's
            // `url_result` to `TokFMAudition`).
            let audition = if url.as_str().contains("wysokieobcasy.pl/") {
                "395"
            } else {
                "334"
            };
            let target = format!("https://audycje.tokfm.pl/audycja/{audition}");
            return Ok(Some(super::follow_url(url, target)));
        };
        let mut api = Url::parse("https://wyborcza.pl/api/podcast")?;
        let wo = url.as_str().contains("wysokieobcasy.pl/");
        // `query={'guid': ..., 'type': 'wo' if ... else None}`: a `None` is sent as "None".
        api = update_url_query(
            &api,
            &[
                ("guid", &podcast_id),
                ("type", if wo { "wo" } else { "None" }),
            ],
        );
        let meta = download_json(client, &api, PODCAST_NAME, &podcast_id)?;
        let media = meta
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| key_error(&podcast_id, "url"))?;
        let media = Url::parse(media)?;
        let duration = meta
            .get("duration")
            .and_then(Value::as_str)
            .and_then(crate::parse_duration)
            .and_then(|duration| serde_json::to_value(duration).ok())
            .unwrap_or(Value::Null);
        let info = direct_info(
            url,
            &media,
            "mp3",
            (PODCAST_NAME, PODCAST_NAME),
            vec![
                ("id", podcast_id.clone().into()),
                ("url", media.as_str().into()),
                ("title", meta.get("title").cloned().unwrap_or(Value::Null)),
                (
                    "description",
                    meta.get("description").cloned().unwrap_or(Value::Null),
                ),
                (
                    "thumbnail",
                    meta.get("imageUrl").cloned().unwrap_or(Value::Null),
                ),
                ("duration", duration),
                (
                    "uploader",
                    meta.get("author").cloned().unwrap_or(Value::Null),
                ),
                ("upload_date", upload_date(meta.get("publishedDate"))),
            ],
        );
        Ok(Some(ExtractResult::Video(info)))
    }
}

impl Extractor for TokFmPodcast {
    fn key(&self) -> &'static str {
        TOKFM_PODCAST_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = TOKFM_PODCAST_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let media_id = found["id"].to_owned();
        announce(TOKFM_PODCAST_NAME, url);
        let metadata = download_json_note(
            client,
            &Url::parse(&format!("https://audycje.tokfm.pl/getp/3{media_id}"))?,
            TOKFM_PODCAST_NAME,
            &media_id,
            "Downloading podcast metadata",
        )?;
        let Some(metadata) = metadata.as_array().and_then(|list| list.first()) else {
            return Err(Error::Reported(format!(
                "[{TOKFM_PODCAST_NAME}] {media_id}: No such podcast"
            )));
        };
        // A random device ID per download (`str(uuid.uuid4())`).
        let device_id = super::drtv::uuid4();
        let mut api = Url::parse("https://api.podcast.radioagora.pl/api4/getSongUrl")?;
        api = update_url_query(
            &api,
            &[
                ("podcast_id", &media_id),
                ("device_id", &device_id),
                ("ppre", "false"),
                ("audio", "mp3"),
            ],
        );
        let song = download_json_note(
            client,
            &api,
            TOKFM_PODCAST_NAME,
            &media_id,
            "Downloading podcast mp3 URL",
        )?;
        let media = song
            .get("link_ssl")
            .and_then(Value::as_str)
            .ok_or_else(|| key_error(&media_id, "link_ssl"))?;
        let media = Url::parse(media)?;
        let info = direct_info(
            url,
            &media,
            "mp3",
            (TOKFM_PODCAST_NAME, TOKFM_PODCAST_KEY),
            vec![
                ("id", media_id.clone().into()),
                ("url", media.as_str().into()),
                ("vcodec", "none".into()),
                ("ext", "mp3".into()),
                (
                    "title",
                    metadata.get("podcast_name").cloned().unwrap_or(Value::Null),
                ),
                (
                    "series",
                    metadata.get("series_name").cloned().unwrap_or(Value::Null),
                ),
                (
                    "episode",
                    metadata.get("podcast_name").cloned().unwrap_or(Value::Null),
                ),
            ],
        );
        Ok(Some(info))
    }
}

impl Extractor for TokFmAudition {
    fn key(&self) -> &'static str {
        TOKFM_AUDITION_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = TOKFM_AUDITION_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let audition_id = found["id"].to_owned();
        announce(TOKFM_AUDITION_NAME, url);
        let data = download_json_headers(
            client,
            &Url::parse(&format!(
                "https://api.podcast.radioagora.pl/api4/getSeries?series_id={audition_id}"
            ))?,
            TOKFM_AUDITION_NAME,
            &audition_id,
            &TOKFM_HEADERS,
        )?;
        let Some(series) = data.as_array().and_then(|list| list.first()) else {
            return Err(Error::Reported(format!(
                "[{TOKFM_AUDITION_NAME}] {audition_id}: No such audition"
            )));
        };
        let series_name = series.get("series_name").cloned().unwrap_or(Value::Null);

        let mut entries = Vec::new();
        for page in 0.. {
            let api = Url::parse(&format!(
                "https://api.podcast.radioagora.pl/api4/getPodcasts?series_id={audition_id}&limit={PAGE_SIZE}&offset={page}&with_guests=true&with_leaders_for_mobile=true"
            ))?;
            let podcasts = download_json_headers(
                client,
                &api,
                TOKFM_AUDITION_NAME,
                &audition_id,
                &TOKFM_HEADERS,
            )?;
            let podcasts = podcasts.as_array().cloned().unwrap_or_default();
            if podcasts.is_empty() {
                return Err(Error::Reported("Agora returned empty page".into()));
            }
            for podcast in &podcasts {
                let Some(target) = podcast.get("podcast_sharing_url").and_then(Value::as_str)
                else {
                    return Err(key_error(&audition_id, "podcast_sharing_url"));
                };
                let name = podcast.get("podcast_name").cloned().unwrap_or(Value::Null);
                let timestamp = int_or_none(podcast.get("podcast_timestamp"));
                entries.push(url_transparent_entry(
                    target,
                    (TOKFM_PODCAST_KEY, TOKFM_PODCAST_NAME),
                    vec![
                        ("title", name.clone()),
                        ("episode", name),
                        (
                            "description",
                            podcast
                                .get("podcast_description")
                                .cloned()
                                .unwrap_or(Value::Null),
                        ),
                        ("timestamp", timestamp),
                        ("series", series_name.clone()),
                    ],
                )?);
            }
            if podcasts.len() < PAGE_SIZE {
                break;
            }
        }

        let mut extra = Map::new();
        extra.insert("series".into(), series_name.clone());
        // `{'_type': 'playlist', 'id', 'title', 'series', 'entries'}`.
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "youtube-tab-plain;_type,id,title,series,entries".into(),
        );
        let title = series_name.as_str().unwrap_or_default().to_owned();
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: audition_id,
            title,
            webpage_url: url.clone(),
            extractor: TOKFM_AUDITION_NAME.into(),
            extractor_key: TOKFM_AUDITION_KEY.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(video_suitable("https://wyborcza.pl/video/26207634"));
        assert!(video_suitable("https://wyborcza.pl/api-video/26207634"));
        assert!(podcast_suitable(
            "https://wyborcza.pl/podcast/0,172673.html?podcast=100720#S.main_topic"
        ));
        assert!(podcast_suitable("https://wyborcza.pl/podcast"));
        assert!(!podcast_suitable("https://wyborcza.pl/video/26207634"));
        assert!(tokfm_podcast_suitable(
            "https://audycje.tokfm.pl/podcast/91275,-Systemowy-rasizm"
        ));
        assert!(tokfm_audition_suitable(
            "https://audycje.tokfm.pl/audycja/218,Analizy"
        ));
    }

    #[test]
    fn polish_upload_dates() {
        assert_eq!(
            upload_date(Some(&Value::from("29 października 2026"))),
            Value::from("20261029")
        );
        assert_eq!(upload_date(Some(&Value::from("nonsense"))), Value::Null);
    }
}
