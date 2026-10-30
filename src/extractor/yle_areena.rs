//! Upstream `YleAreenaIE` (`yt_dlp/extractor/yle_areena.py`): the page's
//! JSON-LD title and thumbnails, then the player preview API's ongoing live
//! event, on-demand HLS manifest, podcast media URL, Kaltura entry (a
//! `url_transparent` result of `kaltura:1955031:ID`) or `gone` notice, with
//! the on-demand SRT subtitles and the programme metadata.
use super::{Extractor, download_json_headers, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct YleAreena;

const NAME: &str = "YleAreena";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://areena\.yle\.fi/(?P<podcast>podcastit/)?(?P<id>[\d-]+)")
        .unwrap()
});
/// `K(?P<season_no>\d+),\s*J(?P<episode_no>\d+):?\s*\b(?P<episode>[^|]+)\s*|\s*(?P<series>.+)`:
/// the top-level alternation makes either the episode groups or the series
/// group match.
static EPISODE_METADATA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"K(?P<season_no>\d+),\s*J(?P<episode_no>\d+):?\s*\b(?P<episode>[^|]+)\s*|\s*(?P<series>.+)",
    )
    .unwrap()
});
static SEASON: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Kausi (\d+)").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("YleAreena {what} are not supported natively"))
}

/// `int_or_none`: integers, truncated floats and integer strings.
fn int_or_none(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Bool(flag) => Some(i64::from(*flag)),
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
        Value::String(text) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
}

/// `traverse_obj(obj, (KEY, ..., {str}))` along object keys.
fn path<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .try_fold(value, |value, key| value.as_object()?.get(*key))
}

fn path_str<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    path(value, keys).and_then(Value::as_str)
}

/// `traverse_obj(obj, (..., {url_or_none}))`: the raw string when valid.
fn url_value<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    path_str(value, keys).filter(|text| super::json_ld::url_or_none(text).is_some())
}

/// `_merge_subtitles`: tracks append per language, duplicate URLs dropped.
fn merge_subtitles(target: &mut crate::Subtitles, source: crate::Subtitles) {
    for (language, tracks) in source.entries {
        match target.entries.iter_mut().find(|(key, _)| *key == language) {
            Some((_, existing)) => {
                for track in tracks {
                    if !existing.iter().any(|known| known.url == track.url) {
                        existing.push(track);
                    }
                }
            }
            None => target.entries.push((language, tracks)),
        }
    }
}

/// Which `video_data` item supplies the media.
enum Source {
    Podcast(String),
    Live,
    OnDemand,
    Kaltura(String),
    Gone,
}

impl Extractor for YleAreena {
    fn key(&self) -> &'static str {
        NAME
    }

    /// `_GEO_COUNTRIES = ['FI']`.
    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::countries(&["FI"])
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        let is_podcast = found.name("podcast").is_some();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        let json_ld = super::json_ld::search(&webpage);
        let api = Url::parse(&format!(
            "https://player.api.yle.fi/v1/preview/{video_id}.json?app_id=player_static_prod&app_key=8930d72170e48303cf5f3867780d549b"
        ))?;
        let response = download_json_headers(
            client,
            &api,
            NAME,
            &video_id,
            &[
                ("origin", "https://areena.yle.fi"),
                ("referer", "https://areena.yle.fi/"),
                ("content-type", "application/json"),
            ],
        )?;
        let Some(video_data) = response.as_object().and_then(|map| map.get("data")) else {
            if !response.is_object() {
                return Err(unsupported("preview responses that are not JSON objects"));
            }
            return Err(Error::Reported(format!(
                "{video_id}: An extractor error has occurred. (caused by KeyError('data')); {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };

        // Example title: 'K1, J2: Pouchit | Modernit miehet'
        let ld_title = json_ld.get("title").and_then(Value::as_str);
        let (mut season_number, mut episode_number, mut episode, mut series) =
            (None, None, None, None);
        if let Some(found) = EPISODE_METADATA.captures(ld_title.unwrap_or_default()) {
            let group = |name: &str| found.name(name).map(|text| text.as_str().to_owned());
            season_number = group("season_no");
            episode_number = group("episode_no");
            episode = group("episode");
            series = group("series");
        }
        let description = path_str(video_data, &["ongoing_ondemand", "description", "fin"]);

        let mut subtitles: Map<String, Value> = Map::new();
        if let Some(Value::Array(items)) = path(video_data, &["ongoing_ondemand", "subtitles"]) {
            for sub in items {
                let Some(sub_url) = url_value(sub, &["uri"]) else {
                    continue;
                };
                let language = match sub.get("language") {
                    None | Some(Value::Null) => "und".to_owned(),
                    Some(Value::String(text)) if text.is_empty() => "und".to_owned(),
                    Some(Value::String(text)) => text.clone(),
                    Some(_) => return Err(unsupported("non-string subtitle languages")),
                };
                let mut track = Map::new();
                track.insert("url".into(), sub_url.into());
                track.insert("ext".into(), "srt".into());
                match sub.get("kind") {
                    None | Some(Value::Null) => {}
                    Some(Value::String(kind)) => {
                        track.insert("name".into(), kind.clone().into());
                    }
                    Some(_) => return Err(unsupported("non-string subtitle kinds")),
                }
                subtitles
                    .entry(language)
                    .or_insert_with(|| Value::Array(Vec::new()))
                    .as_array_mut()
                    .expect("subtitle lists")
                    .push(Value::Object(track));
            }
        } else if path(video_data, &["ongoing_ondemand", "subtitles"]).is_some_and(Value::is_object)
        {
            return Err(unsupported("subtitle objects that are not lists"));
        }
        let mut subtitles: crate::Subtitles = serde_json::from_value(Value::Object(subtitles))
            .map_err(|error| Error::Reported(format!("[{NAME}] {video_id}: {error}")))?;
        // Each track dict sets `name` (even `None`).
        for (_, tracks) in &mut subtitles.entries {
            for track in tracks.iter_mut().filter(|track| track.name.is_none()) {
                track.extra.insert("name".into(), Value::Null);
            }
        }

        let source = if let Some(media) =
            url_value(video_data, &["ongoing_ondemand", "media_url"]).filter(|_| is_podcast)
        {
            Source::Podcast(media.to_owned())
        } else if url_value(video_data, &["ongoing_event", "manifest_url"]).is_some() {
            Source::Live
        } else if url_value(video_data, &["ongoing_ondemand", "manifest_url"]).is_some() {
            Source::OnDemand
        } else if let Some(kaltura) = path_str(video_data, &["ongoing_ondemand", "kaltura", "id"]) {
            Source::Kaltura(kaltura.to_owned())
        } else if path(video_data, &["gone"]).is_some_and(Value::is_object) {
            crate::no_formats::raise_expected(
                NAME,
                Some(&video_id),
                None,
                "The content is no longer available",
            )?;
            Source::Gone
        } else {
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: Unable to extract content; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        let metadata_key = match source {
            Source::Live => "ongoing_event",
            Source::Gone => "gone",
            _ => "ongoing_ondemand",
        };
        let metadata = &video_data[metadata_key];
        let live = matches!(source, Source::Live);

        let mut formats = Vec::new();
        let has_url = matches!(source, Source::Podcast(_) | Source::Kaltura(_));
        let manifest = match metadata.get("manifest_url") {
            _ if has_url => None,
            Some(Value::String(text)) if !text.is_empty() => Some(text.as_str()),
            None | Some(Value::Null) | Some(Value::Bool(false)) | Some(Value::String(_)) => None,
            Some(_) => return Err(unsupported("non-string manifest URLs")),
        };
        if let Some(manifest) = manifest {
            let manifest = Url::parse(manifest)
                .map_err(|_| unsupported("manifest URLs that are not absolute"))?;
            let stream = super::extract_m3u8_fatal_master(
                client,
                &manifest,
                NAME,
                &video_id,
                Some("hls"),
                true,
            )?;
            formats = stream.formats;
            merge_subtitles(&mut subtitles, stream.subtitles);
        }

        let season_from_description = description
            .and_then(|text| SEASON.captures(text))
            .and_then(|found| int_or_none(Some(&Value::from(&found[1]))))
            .filter(|number| *number != 0);
        let season_number = season_from_description.or_else(|| {
            season_number
                .as_deref()
                .and_then(|text| int_or_none(Some(&Value::from(text))))
        });
        let episode_number = episode_number
            .as_deref()
            .and_then(|text| int_or_none(Some(&Value::from(text))));

        // The returned dict's keys in insertion order, final values.
        let meta_str = |keys: &[&str]| path_str(metadata, keys).map(str::to_owned);
        let title = meta_str(&["title", "fin"]).or(episode);
        let description = meta_str(&["description", "fin"]).or(description.map(str::to_owned));
        let series = meta_str(&["series", "title", "fin"]).or(series);
        let episode_number = int_or_none(metadata.get("episode_number")).or(episode_number);
        let age_limit = int_or_none(path(metadata, &["content_rating", "age_restriction"]));
        let release_timestamp = metadata
            .get("start_time")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601);
        // `metadata.pop('duration')` for livestreams.
        let duration = (!live)
            .then(|| int_or_none(path(metadata, &["duration", "duration_in_seconds"])))
            .flatten();
        let thumbnails: Vec<Value> = match json_ld.get("thumbnails") {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|item| item.get("url").filter(|url| !url.is_null()))
                .map(|url| json!({ "url": url }))
                .collect(),
            _ => Vec::new(),
        };

        let opt = |value: Option<String>| value.map(Value::from);
        let opt_int = |value: Option<i64>| value.map(Value::from);
        let mut fields: Vec<(&str, Option<Value>)> = Vec::new();
        let ld_titled = json_ld.contains_key("title");
        if ld_titled {
            fields.push((
                "title",
                Some(title.clone().map_or(Value::Null, Value::from)),
            ));
        }
        if !thumbnails.is_empty() {
            fields.push(("thumbnails", Some(Value::Array(thumbnails))));
        }
        fields.push(("id", Some(Value::from(video_id.clone()))));
        if !ld_titled {
            fields.push((
                "title",
                Some(title.clone().map_or(Value::Null, Value::from)),
            ));
        }
        fields.push(("description", opt(description)));
        fields.push(("series", opt(series)));
        fields.push(("season_number", opt_int(season_number)));
        fields.push(("episode_number", opt_int(episode_number)));
        let has_subtitles = !subtitles.entries.is_empty();
        fields.push(("subtitles", Some(Value::Null)));
        // `traverse_obj(metadata, {...})` keeps only found values.
        for (key, value) in [
            ("age_limit", age_limit),
            ("release_timestamp", release_timestamp),
            ("duration", duration),
        ] {
            if value.is_some() {
                fields.push((key, opt_int(value)));
            }
        }
        if live {
            fields.push(("live_status", Some("is_live".into())));
        }
        let fields: Vec<(&str, Value)> = fields
            .into_iter()
            .map(|(key, value)| (key, value.unwrap_or(Value::Null)))
            .collect();

        match source {
            Source::Kaltura(kaltura_id) => {
                // `url_transparent`: the non-`None` fields other than
                // `_type`, `url`, `ie_key` and `id`.
                let mut forced = Map::new();
                for (key, value) in &fields {
                    if *key == "id" || value.is_null() {
                        continue;
                    }
                    forced.insert((*key).to_owned(), value.clone());
                }
                if has_subtitles {
                    forced.insert(
                        "subtitles".into(),
                        serde_json::to_value(&subtitles).map_err(|error| {
                            Error::Reported(format!("[{NAME}] {video_id}: {error}"))
                        })?,
                    );
                }
                let target = super::brightcove::smuggle(
                    &format!("kaltura:1955031:{kaltura_id}"),
                    &json!({ "source_url": url.as_str() }),
                );
                let mut follow = super::direct::build_at(url.clone(), url, String::new());
                follow.formats.clear();
                follow.direct = None;
                follow.extra.insert(
                    crate::FOLLOW_TRANSPARENT.into(),
                    json!({ "url": target, "fields": Value::Object(forced) }),
                );
                Ok(Some(ExtractResult::Video(follow)))
            }
            Source::Podcast(media) => {
                let media = Url::parse(&media)
                    .map_err(|_| unsupported("podcast media URLs that are not absolute"))?;
                let ext = super::direct::determine_ext(media.as_str())
                    .unwrap_or_else(|| "unknown_video".into());
                let mut info = super::direct::with_format(
                    super::direct::build_at(media.clone(), &media, ext.clone()),
                    "",
                );
                for format in &mut info.formats {
                    format.ext = ext.clone();
                    format.format_id = String::new();
                }
                info.format_id = None;
                info.direct = None;
                info.webpage_url = url.clone();
                info.extractor = NAME.into();
                info.extractor_key = NAME.into();
                let mut keys = Vec::new();
                let mut titled = false;
                // `None` values, set after the typed fields.
                let mut nulls = Vec::new();
                for (key, value) in fields {
                    match key {
                        "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
                        "title" if value.is_null() => continue,
                        "title" => {
                            titled = true;
                            info.title = value.as_str().unwrap_or_default().to_owned();
                        }
                        "subtitles" if !has_subtitles => nulls.push(key),
                        "subtitles" => {}
                        _ if value.is_null() => nulls.push(key),
                        _ => crate::metadata_parser::set_field(&mut info, key, value),
                    }
                    keys.push(key);
                }
                if !titled {
                    info.title = String::new();
                    info.extra
                        .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
                }
                info.subtitles = subtitles;
                for key in nulls {
                    info.extra.insert(key.into(), Value::Null);
                }
                keys.push("url");
                Ok(Some(ExtractResult::Video(crate::info_order::mark_info(
                    info,
                    &format!(
                        "untyped-dict;{};subtitle=url,ext,name{}",
                        keys.join(","),
                        if titled { "" } else { "-untitled" }
                    ),
                ))))
            }
            Source::Live | Source::OnDemand | Source::Gone => {
                let mut fields = fields;
                if !formats.is_empty() || manifest.is_some() {
                    fields.push(("formats", Value::Null));
                }
                let mut info = super::dict_info(url, (NAME, NAME), fields, formats);
                if has_subtitles {
                    info.subtitles = subtitles;
                } else {
                    // `subtitles or None`.
                    info.extra.insert("subtitles".into(), Value::Null);
                }
                if let Some(Value::String(shape)) =
                    info.extra.get_mut(crate::info_order::INFO_SHAPE)
                {
                    shape.push_str(";subtitle=url,ext,name");
                }
                Ok(Some(ExtractResult::Video(info)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://areena.yle.fi/1-4371942"));
        assert!(suitable("https://areena.yle.fi/podcastit/1-71022852"));
        assert!(!suitable("https://areena.yle.fi/tv"));
        assert!(!suitable("https://yle.fi/a/1-4371942"));
    }

    #[test]
    fn episode_metadata_follows_upstream_alternation() {
        let found = EPISODE_METADATA
            .captures("K1, J2: Pouchit | Modernit miehet")
            .unwrap();
        assert_eq!(&found["season_no"], "1");
        assert_eq!(&found["episode_no"], "2");
        assert_eq!(&found["episode"], "Pouchit ");
        assert!(found.name("series").is_none());
        let found = EPISODE_METADATA.captures("Albi Lumiukko").unwrap();
        assert_eq!(&found["series"], "Albi Lumiukko");
        assert!(found.name("episode").is_none());
        assert!(EPISODE_METADATA.captures("").is_none());
    }
}
