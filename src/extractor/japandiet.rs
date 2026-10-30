//! Upstream `ShugiinItvVodIE` and `ShugiinItvLiveIE` (`yt_dlp/extractor/japandiet.py`):
//! the House of Representatives' video library page (its `vtag_src_base_vod`
//! HLS master, the title, release date and chapters) and the all-proceedings
//! index page. Running proceedings are url entries to `ShugiinItvLiveRoom`,
//! which is not ported, so an index with running rooms is an unsupported error.
use super::{Extractor, dict_info, extract_m3u8_fatal_master, search_warning};
use crate::html::clean_html;
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct ShugiinItvVod;
pub struct ShugiinItvLive;

const VOD_NAME: &str = "ShugiinItvVod";
const LIVE_NAME: &str = "ShugiinItvLive";
const PLAYLIST_TITLE: &str = "All proceedings for today";

static VOD_VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?shugiintv\.go\.jp/(?:jp|en)/index\.php\?ex=VL(?:&[^=]+=[^&]*)*&deli_id=(?P<id>\d+)",
    )
    .unwrap()
});
static LIVE_VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?shugiintv\.go\.jp/(?:jp|en)(?:/index\.php)?$").unwrap()
});
static M3U8_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"id="vtag_src_base_vod"\s*value="(http.+?\.m3u8)""#).unwrap());
static TITLE: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r#"<td\s+align="left">(.+)\s*\(\d+分\)"#,
        r#"<TD.+?<IMG\s*src=".+?/spacer\.gif".+?height="15">(.+?)<IMG"#,
    ]
    .iter()
    .map(|pattern| Regex::new(pattern).unwrap())
    .collect()
});
static RELEASE_DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"開会日</td>\s*<td.+?/td>\s*<TD>(.+?)</TD>").unwrap());
static CHAPTER_START: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)<A\s+HREF="([^"]+?)"\s*class="play_vod">"#).unwrap());
static CHAPTER_TAIL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(.+)</a>").unwrap());
static ROOM_LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<a\s+href="[^"]+\?room_id=(room\d+)"\s*class="play_live".+?class="s12_14">(.+?)</td>"#)
        .unwrap()
});
static LAST_TR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)<TR\s*class="s14_24">(.+?)</TR>"#).unwrap());
static LAST_TD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<TD.+?</TD>").unwrap());
static ERA_DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(明治|大正|昭和|平成|令和)?(\d+)年(\d+)月(\d+)日").unwrap());
static DURATION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:(\d+)日間?)?(?:(\d+)時間?)?(?:(\d+)分)?(?:(\d+)秒)?").unwrap()
});

/// `_match_id` for the video library page.
pub fn vod_suitable(url: &str) -> bool {
    VOD_VALID_URL.is_match(url)
}

/// `suitable` for the all-proceedings index page.
pub fn live_suitable(url: &str) -> bool {
    LIVE_VALID_URL.is_match(url) && !vod_suitable(url)
}

/// `_parse_japanese_date`: `YYYYMMDD` from a (possibly era-prefixed) date.
fn parse_japanese_date(text: Option<&str>) -> Option<String> {
    let text: String = text?
        .chars()
        .filter(|character| !character.is_whitespace() && *character != '\u{3000}')
        .collect();
    let found = ERA_DATE.captures(&text)?;
    let era = match found.get(1).map(|era| era.as_str()) {
        Some("明治") => 1868,
        Some("大正") => 1912,
        Some("昭和") => 1926,
        Some("平成") => 1989,
        Some("令和") => 2019,
        _ => 0,
    };
    let year: i64 = found[2].parse::<i64>().ok()? + era;
    let month: i64 = found[3].parse().ok()?;
    let day: i64 = found[4].parse().ok()?;
    Some(format!("{year:04}{month:02}{day:02}"))
}

/// `_parse_japanese_duration`: seconds from a `1日2時間3分4秒` style text.
fn parse_japanese_duration(text: &str) -> f64 {
    let text: String = text
        .chars()
        .filter(|character| !character.is_whitespace() && *character != '\u{3000}')
        .collect();
    let Some(found) = DURATION.captures(&text) else {
        return 0.0;
    };
    let group = |index: usize| {
        found
            .get(index)
            .and_then(|digits| digits.as_str().parse::<f64>().ok())
            .unwrap_or(0.0)
    };
    group(4) + group(3) * 60.0 + group(2) * 60.0 * 60.0 + group(1) * 24.0 * 60.0 * 60.0
}

/// `_search_regex` over several patterns (the first match wins), fatal
/// `[EXTRACTOR] ID: Unable to extract WHAT` when none matches.
fn search_html(patterns: &[&Regex], webpage: &str) -> Option<String> {
    patterns.iter().find_map(|pattern| {
        let found = pattern.captures(webpage)?;
        found.get(1).map(|text| clean_html(text.as_str()))
    })
}

/// The page as `_download_webpage(url, id, encoding='euc-jp')` reads it.
fn download_euc_jp(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
    note: &str,
) -> Result<String> {
    use std::io::Read;
    let prefix = if id.is_empty() {
        format!("[{extractor}]")
    } else {
        format!("[{extractor}] {id}:")
    };
    crate::screen::screen(&format!("{prefix} {note}"));
    let mut response = super::direct::request(client, url, None).map_err(|error| match error {
        Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
            extractor,
            id: id.to_owned(),
            what: "webpage",
            source: Box::new(error),
        },
        error => error,
    })?;
    let page_url = response.url().clone();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    crate::screen::expect_page(id, None, None);
    crate::screen::page_read(&page_url, &body)?;
    let (text, _, _) = encoding_rs::EUC_JP.decode(&body);
    Ok(text.into_owned())
}

/// The shared `-J` shape of the title-less index playlist: `playlist-dict`.
fn playlist_entries(url: &Url, entries: Vec<VideoInfo>) -> ExtractResult {
    // `playlist_result(entries, playlist_title=...)` has no `id`: an `id: None`
    // extra marks it for the shape.
    let mut extra = Map::new();
    extra.insert("id".into(), Value::Null);
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        "playlist-dict;title".into(),
    );
    ExtractResult::Playlist(PlaylistInfo {
        id: String::new(),
        title: PLAYLIST_TITLE.into(),
        webpage_url: url.clone(),
        extractor: LIVE_NAME.into(),
        extractor_key: LIVE_NAME.into(),
        nested: Vec::new(),
        pending: None,
        entries,
        extra,
    })
}

impl Extractor for ShugiinItvVod {
    fn key(&self) -> &'static str {
        VOD_NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VOD_VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(VOD_NAME);
        crate::screen::screen(&format!(
            "[{VOD_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let page_url = format!(
            "https://www.shugiintv.go.jp/jp/index.php?ex=VL&media_type=&deli_id={video_id}"
        );
        let webpage = download_euc_jp(
            client,
            &Url::parse(&page_url)?,
            VOD_NAME,
            &video_id,
            "Downloading webpage",
        )?;

        let Some(m3u8) = M3U8_URL.captures(&webpage).and_then(|found| found.get(1)) else {
            return Err(Error::Reported(format!(
                "[{VOD_NAME}] {video_id}: Unable to extract m3u8 url; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        let m3u8 = m3u8.as_str().replacen("http://", "https://", 1);
        let manifest = Url::parse(&m3u8)?;
        let mut stream =
            extract_m3u8_fatal_master(client, &manifest, VOD_NAME, &video_id, None, true)?;
        for format in &mut stream.formats {
            format.ext = "mp4".into();
        }

        let title = search_html(&TITLE.iter().collect::<Vec<_>>(), &webpage);
        if title.is_none() {
            search_warning(VOD_NAME, "title");
        }
        let release_date = RELEASE_DATE
            .captures(&webpage)
            .and_then(|found| found.get(1))
            .map(|text| clean_html(text.as_str()));
        if release_date.is_none() {
            search_warning(VOD_NAME, "title");
        }
        let release_date = parse_japanese_date(release_date.as_deref());
        let chapters = chapters(&webpage)?;

        let mut info = dict_info(
            url,
            (VOD_NAME, VOD_NAME),
            vec![
                ("id", video_id.clone().into()),
                ("title", title.map_or(Value::Null, Value::from)),
                (
                    "release_date",
                    release_date.map_or(Value::Null, Value::from),
                ),
                ("chapters", Value::Array(chapters)),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
            ],
            stream.formats,
        );
        info.subtitles = stream.subtitles;
        // Each chapter is `{title, start_time}` before processing's `end_time`.
        let shape = info
            .extra
            .get(crate::info_order::INFO_SHAPE)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        Ok(Some(crate::info_order::mark_info(
            info,
            &format!("{shape};chapter=title,start_time"),
        )))
    }
}

/// The `chapters` of the video library page's rows (`play_vod` links with a
/// `time=` query), the last one ending after its row's duration cell.
fn chapters(webpage: &str) -> Result<Vec<Value>> {
    let mut chapters: Vec<Value> = Vec::new();
    // `finditer` over `<A HREF=... class="play_vod">(?!<img)(.+)</a>`: a match
    // consumes through its line's last `</a>`, and a failed one moves on by
    // one character.
    let mut position = 0;
    while let Some(start) = CHAPTER_START.find_at(webpage, position) {
        let rest = &webpage[start.end()..];
        let is_image = rest
            .as_bytes()
            .get(..4)
            .is_some_and(|tag| tag.eq_ignore_ascii_case(b"<img"));
        let tail = (!is_image).then(|| CHAPTER_TAIL.captures(rest)).flatten();
        let Some(tail) = tail else {
            position = start.start() + 1;
            continue;
        };
        let href = CHAPTER_START
            .captures(&webpage[start.start()..start.end()])
            .and_then(|found| found.get(1))
            .map_or("", |href| href.as_str());
        let start_time = Url::parse("https://www.shugiintv.go.jp/")?
            .join(href)
            .ok()
            .and_then(|url| {
                url.query_pairs()
                    .find(|(key, _)| key == "time")
                    .map(|(_, value)| value.into_owned())
            })
            .and_then(|value| value.trim().parse::<f64>().ok());
        chapters.push(json!({
            "title": clean_html(&tail[1]).trim(),
            "start_time": start_time,
        }));
        position = start.end() + tail.get(0).map_or(0, |whole| whole.end());
    }
    // `re.findall(...)[-1]` raises IndexError when no row matches.
    let last_tr = LAST_TR
        .captures_iter(webpage)
        .last()
        .and_then(|found| found.get(1).map(|row| row.as_str().to_owned()))
        .ok_or_else(|| Error::Reported("list index out of range".into()))?;
    if let Some(last) = chapters.last_mut() {
        let last_td = LAST_TD
            .find_iter(&last_tr)
            .last()
            .map(|cell| cell.as_str().to_owned())
            .ok_or_else(|| Error::Reported("list index out of range".into()))?;
        let Some(last) = last.as_object_mut() else {
            return Ok(chapters);
        };
        let Some(start) = last.get("start_time").and_then(Value::as_f64) else {
            return Err(Error::UnsupportedHtml5(
                "ShugiinItvVod chapters without a start time are not supported natively".into(),
            ));
        };
        let end = start + parse_japanese_duration(&clean_html(&last_td));
        last.insert("end_time".into(), json!(end));
    }
    Ok(chapters)
}

impl Extractor for ShugiinItvLive {
    fn key(&self) -> &'static str {
        LIVE_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if !LIVE_VALID_URL.is_match(url.as_str()) || vod_suitable(url.as_str()) {
            return Ok(None);
        }
        crate::screen::set_extractor(LIVE_NAME);
        let webpage = download_euc_jp(
            client,
            &Url::parse("https://www.shugiintv.go.jp/jp/index.php")?,
            LIVE_NAME,
            "",
            "Downloading proceedings info",
        )?;
        let rooms = ROOM_LINK.captures_iter(&webpage).count();
        if rooms > 0 {
            return Err(Error::UnsupportedHtml5(format!(
                "{LIVE_NAME} running proceedings ({rooms} listed) are room url entries, which are not supported natively"
            )));
        }
        Ok(Some(playlist_entries(url, Vec::new())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(vod_suitable(
            "https://www.shugiintv.go.jp/jp/index.php?ex=VL&media_type=&deli_id=53846"
        ));
        assert!(vod_suitable(
            "https://www.shugiintv.go.jp/en/index.php?ex=VL&media_type=&deli_id=53846"
        ));
        assert!(!vod_suitable(
            "https://www.shugiintv.go.jp/jp/index.php?room_id=room01"
        ));
        assert!(live_suitable("https://www.shugiintv.go.jp/jp/index.php"));
        assert!(live_suitable("https://shugiintv.go.jp/en"));
        assert!(!live_suitable(
            "https://www.shugiintv.go.jp/jp/index.php?room_id=room01"
        ));
    }

    #[test]
    fn japanese_dates_and_durations() {
        assert_eq!(
            parse_japanese_date(Some("令和5年3月34日")).as_deref(),
            Some("20240334")
        );
        assert_eq!(
            parse_japanese_date(Some("2022年3月23日")).as_deref(),
            Some("20220323")
        );
        assert_eq!(parse_japanese_date(Some("no date")), None);
        assert_eq!(parse_japanese_duration("1時間2分3秒"), 3723.0);
        assert_eq!(parse_japanese_duration("45分"), 2700.0);
    }
}
