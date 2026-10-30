//! Upstream `AsobiChannelIE` and `AsobiChannelTagURLIE`
//! (`yt_dlp/extractor/asobichannel.py`): a microCMS media record and its
//! survapi VOD stream, and a tag page's media list as URL entries.
//! Live events are not ported: they raise an unsupported error.
use super::{
    Extractor, dict_info, download_json, download_json_headers, download_webpage,
    extract_m3u8_fatal_master,
};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct AsobiChannel;
pub struct AsobiChannelTagUrl;

const NAME: &str = "asobichannel";
const KEY: &str = "AsobiChannel";
const TAG_NAME: &str = "asobichannel:tag";
const TAG_KEY: &str = "AsobiChannelTagURL";
const MICROCMS_HEADER: [(&str, &str); 1] =
    [("X-MICROCMS-API-KEY", "qRaKehul9AHU8KtL0dnq1OCLKnFec6yrbcz3")];

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://asobichannel\.asobistore\.jp/watch/(?P<id>[\w-]+)").unwrap()
});
static TAG_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://asobichannel\.asobistore\.jp/tag/(?P<id>[a-z0-9-_]+)").unwrap()
});

fn text(value: Option<&Value>) -> Option<Value> {
    value
        .and_then(Value::as_str)
        .map(|text| Value::from(text.to_owned()))
}

/// `_extract_info(metadata)`: the traversed fields in upstream's order.
fn metadata_fields(metadata: &Value) -> Vec<(&'static str, Value)> {
    let mut fields = Vec::new();
    let mut push = |name: &'static str, value: Option<Value>| {
        if let Some(value) = value.filter(|value| !value.is_null()) {
            fields.push((name, value));
        }
    };
    push("id", text(metadata.get("id")));
    push("title", text(metadata.get("title")));
    push(
        "description",
        text(metadata.get("body"))
            .map(|body| Value::from(crate::html::clean_html(body.as_str().unwrap_or_default()))),
    );
    push(
        "thumbnail",
        text(
            metadata
                .get("contents")
                .and_then(|contents| contents.get("video_thumb"))
                .and_then(|thumb| thumb.get("url")),
        )
        .and_then(|url| url.as_str().and_then(super::json_ld::url_or_none))
        .map(Value::from),
    );
    push(
        "timestamp",
        text(metadata.get("publishedAt"))
            .and_then(|date| crate::dates::parse_iso8601(date.as_str().unwrap_or_default()))
            .map(Value::from),
    );
    push(
        "modified_timestamp",
        text(metadata.get("updatedAt"))
            .and_then(|date| crate::dates::parse_iso8601(date.as_str().unwrap_or_default()))
            .map(Value::from),
    );
    push(
        "channel",
        text(
            metadata
                .get("channel")
                .and_then(|channel| channel.get("name")),
        ),
    );
    push(
        "channel_id",
        text(
            metadata
                .get("channel")
                .and_then(|channel| channel.get("id")),
        ),
    );
    fields
}

fn video_type(metadata: &Value) -> Option<String> {
    metadata
        .get("contents")
        .and_then(|contents| contents.get("video_type"))
        .and_then(Value::as_array)
        .and_then(|types| types.first())
        .and_then(Value::as_str)
        .map(str::to_owned)
}

impl Extractor for AsobiChannel {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(KEY);
        crate::screen::screen(&format!(
            "[{KEY}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        // `_real_initialize`: the survapi bearer token.
        let token = download_json(
            client,
            &Url::parse("https://asobichannel-api.asobistore.jp/api/v1/vspf/token")?,
            NAME,
            &video_id,
        )?;
        let token = token.as_str().unwrap_or_default().to_owned();
        let survapi_header = format!("Bearer {token}");

        let api = Url::parse(&format!(
            "https://channel.microcms.io/api/v1/media/{video_id}"
        ))?;
        let metadata = download_json_headers(client, &api, NAME, &video_id, &MICROCMS_HEADER)?;
        let fields = metadata_fields(&metadata);

        match video_type(&metadata).as_deref() {
            Some("VOD") => {
                let content_id = metadata
                    .get("contents")
                    .and_then(|contents| contents.get("video_id"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let vod_api = Url::parse(&format!(
                    "https://survapi.channel.or.jp/proxy/v1/contents/{content_id}/get_by_cuid"
                ))?;
                let vod = download_json_headers(
                    client,
                    &vod_api,
                    NAME,
                    &video_id,
                    &[("Authorization", survapi_header.as_str())],
                )?;
                let streaming = vod
                    .get("ex_content")
                    .and_then(|content| content.get("streaming_url"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        Error::Reported(format!(
                            "[{NAME}] {video_id}: An extractor error has occurred. (caused by KeyError('streaming_url')); {}",
                            crate::BUG_REPORTS_MESSAGE
                        ))
                    })?;
                let manifest = Url::parse(streaming)?;
                let formats =
                    extract_m3u8_fatal_master(client, &manifest, NAME, &video_id, None, false)?;
                let mut keys: Vec<(&str, Value)> = fields;
                keys.push(("formats", Value::Null));
                let mut info = dict_info(url, (NAME, KEY), keys, formats.formats);
                info.subtitles = formats.subtitles;
                Ok(Some(info))
            }
            Some("LIVE") => Err(Error::UnsupportedHtml5(
                "AsobiChannel live events are not supported natively".into(),
            )),
            other => Err(Error::Reported(format!(
                "[{NAME}] {video_id}: Unexpected video type {}",
                match other {
                    Some(kind) => format!("'{kind}'"),
                    None => "None".to_owned(),
                }
            ))),
        }
    }
}

/// A `url_result` entry for a tag's media: `_type`, `url`, `ie_key`, then
/// the metadata's fields.
fn watch_entry(metadata: &Value) -> Result<VideoInfo> {
    let id = metadata
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let target = format!("https://asobichannel.asobistore.jp/watch/{id}");
    let url = Url::parse(&target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.id = id;
    let mut entry = Map::new();
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.clone().into());
    entry.insert("ie_key".into(), KEY.into());
    for (key, value) in metadata_fields(metadata) {
        entry.insert(key.into(), value);
    }
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": KEY,
            "extractor": NAME,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

impl Extractor for AsobiChannelTagUrl {
    fn key(&self) -> &'static str {
        TAG_KEY
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = TAG_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let tag_id = found["id"].to_owned();
        crate::screen::set_extractor(TAG_NAME);
        crate::screen::screen(&format!(
            "[{TAG_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, TAG_NAME, &tag_id)?;
        // `_search_nextjs_data(webpage, tag_id, fatal=False)`.
        let title = match super::search_nextjs_data(&webpage, TAG_NAME, &tag_id) {
            Ok(data) => data
                .get("props")
                .and_then(|props| props.get("pageProps"))
                .and_then(|page| page.get("data"))
                .and_then(|data| data.get("name"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            Err(_) => {
                crate::screen::warning(&format!(
                    "[{TAG_NAME}] unable to extract next.js data; {}",
                    crate::BUG_REPORTS_MESSAGE
                ));
                None
            }
        };

        let api = Url::parse(&format!(
            "https://channel.microcms.io/api/v1/media?limit=999&filters=(tag[contains]{tag_id})"
        ))?;
        let media = download_json_headers(client, &api, TAG_NAME, &tag_id, &MICROCMS_HEADER)?;
        let mut entries = Vec::new();
        if let Some(Value::Array(contents)) = media.get("contents") {
            for metadata in contents
                .iter()
                .filter(|item| item.get("id").is_some_and(super::nuxt::truthy))
            {
                entries.push(watch_entry(metadata)?);
            }
        }
        // `playlist_result(entries, tag_id, title)`.
        let shape = if title.is_some() {
            "playlist-dict;id,title"
        } else {
            "playlist-dict;id"
        };
        let mut extra = Map::new();
        extra.insert(crate::info_order::INFO_SHAPE.into(), shape.into());
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: tag_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: TAG_NAME.into(),
            extractor_key: TAG_KEY.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        match self.extract_result(client, url)? {
            Some(ExtractResult::Video(info)) => Ok(Some(info)),
            _ => Ok(None),
        }
    }
}

/// URL matching for `--list-extractors`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// URL matching for `--list-extractors`.
pub fn tag_suitable(url: &str) -> bool {
    TAG_URL.is_match(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suitable(url: &str) -> bool {
        VALID_URL.is_match(url)
    }

    fn tag_suitable(url: &str) -> bool {
        TAG_URL.is_match(url)
    }

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://asobichannel.asobistore.jp/watch/1ypp48qd32p"
        ));
        assert!(!suitable(
            "https://asobichannel.asobistore.jp/tag/bjhh-nbcja"
        ));
        assert!(tag_suitable(
            "https://asobichannel.asobistore.jp/tag/bjhh-nbcja"
        ));
    }
}
