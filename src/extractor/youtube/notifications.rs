//! Upstream `YoutubeNotificationsIE` (`yt_dlp/extractor/youtube/_notifications.py`):
//! the signed-in account's notification menu (`:ytnotif`) as a playlist of
//! `url` entries, through the `notification/get_notification_menu` API.
use super::tab::{at, text_at, thumbnails, ucid_or_none};
use super::watch;
use crate::{ExtractResult, Result};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub(crate) const NAME: &str = "youtube:notif";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| crate::extractor::url_regex(r"^:ytnotif(?:ication)?s?").unwrap());

/// `YoutubeNotificationsIE.suitable`.
pub(crate) fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_extract_notification_renderer`: a video or community post entry.
fn notification_entry(notification: Option<&Value>) -> Option<Map<String, Value>> {
    let notification = notification.unwrap_or(&Value::Null);
    let string = |path: &[&str]| {
        at(notification, path)
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let video_id = string(&["navigationEndpoint", "watchEndpoint", "videoId"]);
    let mut channel_id = None;
    let url = match &video_id {
        Some(video_id) if !video_id.is_empty() => {
            format!("https://www.youtube.com/watch?v={video_id}")
        }
        _ => {
            channel_id = string(&["navigationEndpoint", "browseEndpoint", "browseId"])
                .and_then(|id| ucid_or_none(&id));
            static POST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/post/(.+)").unwrap());
            let post_id = string(&["navigationEndpoint", "browseEndpoint", "canonicalBaseUrl"])
                .and_then(|url| POST.captures(&url).map(|found| found[1].to_owned()));
            let (Some(channel), Some(post_id)) = (&channel_id, post_id) else {
                return None;
            };
            // The direct /post URL redirects here in the browser.
            format!("https://www.youtube.com/channel/{channel}/community?lb={post_id}")
        }
    };
    let channel = string(&[
        "contextualMenu",
        "menuRenderer",
        "items",
        "1",
        "menuServiceItemRenderer",
        "text",
        "runs",
        "1",
        "text",
    ]);
    // Soft hyphens are removed.
    let notification_title =
        text_at(notification, &["shortMessage"]).map(|title| title.replace('\u{ad}', ""));
    let title = notification_title.and_then(|text| {
        let pattern = format!(
            "{}[^:]+: (.+)",
            regex::escape(channel.as_deref().unwrap_or(""))
        );
        Regex::new(&pattern)
            .ok()?
            .captures(&text)
            .map(|found| found[1].to_owned())
    });
    let video = video_id.as_deref().is_some_and(|id| !id.is_empty());
    let mut entry = Map::new();
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), url.into());
    entry.insert(
        "ie_key".into(),
        if video { "Youtube" } else { super::tab::KEY }.into(),
    );
    entry.insert("video_id".into(), video_id.into());
    entry.insert("title".into(), title.into());
    entry.insert("channel_id".into(), channel_id.into());
    entry.insert("channel".into(), channel.clone().into());
    entry.insert("uploader".into(), channel.into());
    entry.insert(
        "thumbnails".into(),
        thumbnails(notification.get("videoThumbnail"), "thumbnails").into(),
    );
    // Only with the `youtubetab:approximate_date` extractor argument.
    let sent = text_at(notification, &["sentTimeText"]);
    entry.insert(
        "timestamp".into(),
        super::tab::approximate_timestamp(sent.as_deref(), true).into(),
    );
    Some(entry)
}

/// `_notification_menu_entries` as the state of upstream's generator:
/// the menu pages are requested as the playlist's entries are read.
fn menu_state(ytcfg: Map<String, Value>) -> Value {
    json!({
        "kind": "notifications",
        "ytcfg": ytcfg,
        "continuation": null,
        "previous_visitor": null,
        "page": 1,
        "done": false,
    })
}

/// The notification menu pages of a [`menu_state`] until `limit` entries
/// are read (or its end): the remaining state unless read past its end.
pub(crate) fn fetch_pages(
    client: &Client,
    mut state: Value,
    limit: Option<usize>,
    out: &mut Vec<crate::VideoInfo>,
) -> Result<Option<Value>> {
    let ytcfg = state["ytcfg"].as_object().cloned().unwrap_or_default();
    loop {
        if limit.is_some_and(|limit| out.len() >= limit) {
            return Ok(Some(state));
        }
        if state["done"] == true {
            return Ok(None);
        }
        let page = state["page"].as_u64().unwrap_or(1);
        let mut query = Map::new();
        if let Some(ctoken) = at(
            &state["continuation"],
            &[
                "continuationEndpoint",
                "getNotificationMenuEndpoint",
                "ctoken",
            ],
        )
        .and_then(Value::as_str)
        {
            query.insert("ctoken".into(), ctoken.into());
        }
        let visitor = state["previous_visitor"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| watch::visitor_from(&state["ytcfg"]));
        let current = watch::innertube_api(
            client,
            "notification/get_notification_menu",
            NAME,
            &format!("page {page}"),
            &ytcfg,
            visitor.as_deref(),
            query,
            "Downloading API JSON",
        )?;
        // `_extract_notification_menu`.
        let items = [
            &[
                "actions",
                "0",
                "openPopupAction",
                "popup",
                "multiPageMenuRenderer",
                "sections",
                "0",
                "multiPageMenuNotificationSectionRenderer",
                "items",
            ][..],
            &[
                "actions",
                "0",
                "appendContinuationItemsAction",
                "continuationItems",
            ][..],
        ]
        .iter()
        .find_map(|path| at(&current, path).and_then(Value::as_array))
        .cloned()
        .unwrap_or_default();
        let mut continuation = Value::Null;
        for item in &items {
            if let Some(entry) = notification_entry(item.get("notificationRenderer")) {
                out.push(super::tab::entry_info(entry)?);
            }
            if let Some(next) = item
                .get("continuationItemRenderer")
                .filter(|value| super::tab::truthy(Some(value)))
            {
                continuation = next.clone();
            }
        }
        state["previous_visitor"] = watch::visitor_from(&current).into();
        state["page"] = (page + 1).into();
        if continuation.is_null() {
            state["done"] = true.into();
        }
        state["continuation"] = continuation;
    }
}

pub struct YoutubeNotifications;

impl super::super::Extractor for YoutubeNotifications {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<crate::VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let text = super::super::input_text(url);
        if !suitable(&text) {
            return Ok(None);
        }
        crate::screen::set_extractor(NAME);
        super::login(NAME, "")?;
        super::initialize();
        super::redirect::check_login_required(NAME)?;
        super::search::announce(NAME, &text);
        let display_id = "notifications";
        // `_download_ytcfg('web', display_id) if not self.skip_webpage else {}`.
        let home = Url::parse("https://www.youtube.com")?;
        let ytcfg = if super::tab::skip_webpage() {
            Map::new()
        } else {
            match super::super::download_webpage_note(
                client,
                &home,
                NAME,
                display_id,
                "Downloading web client config",
            ) {
                Ok(webpage) => watch::extract_ytcfg(&webpage),
                Err(error) => {
                    crate::screen::warning(&error.to_string());
                    Map::new()
                }
            }
        };
        // `_report_playlist_authcheck(ytcfg)` (`:ytnotif` has no temp ID).
        let skip_authcheck = super::tab::tab_arg("skip")
            .iter()
            .any(|value| value == "authcheck");
        if ytcfg.is_empty() && skip_authcheck {
            crate::screen::warning_once(&format!(
                "[{NAME}] Playlists that require authentication may not extract correctly without a successful webpage download"
            ));
        } else if ytcfg.is_empty() {
            return Err(crate::Error::Reported(format!(
                "[{NAME}] Playlists that require authentication may not extract correctly without a successful webpage download. If you are not downloading private content, or your cookies are only for the first account and channel, pass \"--extractor-args youtubetab:skip=authcheck\" to skip this check"
            )));
        }
        let mut result = super::search::search_playlist(
            Vec::new(),
            display_id,
            url.clone(),
            NAME,
            "YoutubeNotifications",
        )?;
        if let ExtractResult::Playlist(playlist) = &mut result {
            playlist.pending = Some(crate::PendingEntries {
                state: menu_state(ytcfg),
            });
        }
        Ok(Some(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn notification_entries_follow_upstream() {
        assert!(suitable(":ytnotifications") && suitable(":ytnotif") && !suitable("ytnotif"));
        let entry = notification_entry(Some(&json!({
            "navigationEndpoint": {"watchEndpoint": {"videoId": "BaW_jenozKc"}},
            "shortMessage": {"simpleText": "Chan up\u{ad}loaded: A video"},
            "contextualMenu": {"menuRenderer": {"items": [{}, {"menuServiceItemRenderer": {
                "text": {"runs": [{"text": "Turn off for "}, {"text": "Chan"}]}}}]}},
        })))
        .unwrap();
        assert_eq!(entry["title"], "A video");
        assert_eq!(entry["channel"], "Chan");
        assert_eq!(entry["ie_key"], "Youtube");
        let post = notification_entry(Some(&json!({"navigationEndpoint": {"browseEndpoint": {
            "browseId": "UC0123456789abcdefghijkl", "canonicalBaseUrl": "/post/Ugkx1"}}})))
        .unwrap();
        assert_eq!(
            post["url"],
            "https://www.youtube.com/channel/UC0123456789abcdefghijkl/community?lb=Ugkx1"
        );
        assert!(notification_entry(None).is_none());
    }
}
