//! Upstream's small YouTube extractors that only redirect or explain:
//! `_redirect.py` (`YoutubeYtBeIE`, `YoutubeLivestreamEmbedIE`,
//! `YoutubeYtUserIE`, the feed keywords, `YoutubeShortsAudioPivotIE`,
//! `YoutubeConsentRedirectIE`), `_mistakes.py` (`YoutubeTruncatedURLIE`,
//! `YoutubeTruncatedIDIE`) and `_tab.py`'s `YoutubePlaylistIE`. Each returns
//! upstream's `url_result`, which the downloader follows through every
//! extractor.
use super::urls;
use crate::{Error, ExtractResult, Result};
use base64::Engine;
use regex::Regex;
use reqwest::blocking::Client;
use std::sync::LazyLock;
use url::Url;

/// One of the redirecting extractors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Consent,
    Favourites,
    History,
    LivestreamEmbed,
    Playlist,
    Recommended,
    ShortsAudioPivot,
    Subscriptions,
    TruncatedId,
    TruncatedUrl,
    WatchLater,
    YtBe,
    YtUser,
}

/// `_youtube_login_hint` after `Login details are needed…`.
pub(super) const LOGIN_HINT: &str = "Use --cookies-from-browser or --cookies for the authentication. See  https://github.com/yt-dlp/yt-dlp/wiki/FAQ#how-do-i-pass-cookies-to-yt-dlp  for how to manually pass cookies. Also see  https://github.com/yt-dlp/yt-dlp/wiki/Extractors#exporting-youtube-cookies  for tips on effectively exporting YouTube cookies";

/// `_check_login_required` for a `_LOGIN_REQUIRED` extractor.
pub(super) fn check_login_required(name: &str) -> Result<()> {
    if super::has_auth_cookies() {
        return Ok(());
    }
    Err(Error::Reported(format!(
        "[{name}] Login details are needed to download this content. {LOGIN_HINT}"
    )))
}

fn pattern(text: &str) -> Regex {
    Regex::new(text).expect("static pattern")
}

static CONSENT: LazyLock<Regex> = LazyLock::new(|| pattern(r"^https?://consent\.youtube\.com/m\?"));
static FAVOURITES: LazyLock<Regex> = LazyLock::new(|| pattern(r"^:ytfav(?:ou?rite)?s?"));
static HISTORY: LazyLock<Regex> = LazyLock::new(|| pattern(r"^:ythis(?:tory)?"));
static LIVESTREAM_EMBED: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r"^https?://(?:\w+\.)?youtube\.com/embed/live_stream/?\?(?:[^#]+&)?channel=(?P<id>[^&#]+)",
    )
});
static PLAYLIST: LazyLock<Regex> = LazyLock::new(|| {
    pattern(&format!(
        r"^(?:https?://)?(?:\w+\.)?(?:(?:youtube(?:kids)?\.com|{})/.*?\?.*?\blist=)?(?P<id>{})",
        urls::invidious(),
        urls::PLAYLIST_ID_RE
    ))
});
static RECOMMENDED: LazyLock<Regex> = LazyLock::new(|| {
    pattern(r"^(?:https?://(?:www\.)?youtube\.com/?(?:[?#]|$)|:ytrec(?:ommended)?)")
});
static SHORTS_AUDIO_PIVOT: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"^https?://(?:www\.)?youtube\.com/source/(?P<id>[\w-]{11})/shorts"));
static SUBSCRIPTIONS: LazyLock<Regex> = LazyLock::new(|| pattern(r"^:ytsub(?:scription)?s?"));
static TRUNCATED_ID: LazyLock<Regex> = LazyLock::new(|| {
    pattern(r"^https?://(?:www\.)?youtube\.com/watch\?v=(?P<id>[0-9A-Za-z_-]{1,10})\n?$")
});
static TRUNCATED_URL: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r"^(?:https?://)?(?:\w+\.)?[yY][oO][uU][tT][uU][bB][eE](?:-nocookie)?\.com/(?:watch\?(?:feature=[a-z_]+|annotation_id=annotation_[^&]+|x-yt-cl=[0-9]+|hl=[^&]*|t=[0-9]+)?|attribution_link\?a=[^&]+)\n?$",
    )
});
static WATCH_LATER: LazyLock<Regex> = LazyLock::new(|| pattern(r"^:ytwatchlater"));
static YT_BE: LazyLock<Regex> = LazyLock::new(|| {
    pattern(&format!(
        r"^https?://youtu\.be/(?P<id>[0-9A-Za-z_-]{{11}})/*?.*?\blist=(?P<playlist_id>{})",
        urls::PLAYLIST_ID_RE
    ))
});
static YT_USER: LazyLock<Regex> = LazyLock::new(|| pattern(r"^ytuser:(?P<id>.+)"));

impl Kind {
    /// `IE_NAME`.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Kind::Consent => "youtube:consent",
            Kind::Favourites => "youtube:favorites",
            Kind::History => "youtube:history",
            Kind::LivestreamEmbed => "YoutubeLivestreamEmbed",
            Kind::Playlist => "youtube:playlist",
            Kind::Recommended => "youtube:recommended",
            Kind::ShortsAudioPivot => "youtube:shorts:pivot:audio",
            Kind::Subscriptions => "youtube:subscriptions",
            Kind::TruncatedId => "youtube:truncated_id",
            Kind::TruncatedUrl => "youtube:truncated_url",
            Kind::WatchLater => "youtube:watchlater",
            Kind::YtBe => "YoutubeYtBe",
            Kind::YtUser => "youtube:user",
        }
    }

    fn regex(self) -> &'static Regex {
        match self {
            Kind::Consent => &CONSENT,
            Kind::Favourites => &FAVOURITES,
            Kind::History => &HISTORY,
            Kind::LivestreamEmbed => &LIVESTREAM_EMBED,
            Kind::Playlist => &PLAYLIST,
            Kind::Recommended => &RECOMMENDED,
            Kind::ShortsAudioPivot => &SHORTS_AUDIO_PIVOT,
            Kind::Subscriptions => &SUBSCRIPTIONS,
            Kind::TruncatedId => &TRUNCATED_ID,
            Kind::TruncatedUrl => &TRUNCATED_URL,
            Kind::WatchLater => &WATCH_LATER,
            Kind::YtBe => &YT_BE,
            Kind::YtUser => &YT_USER,
        }
    }

    /// `_LOGIN_REQUIRED`.
    fn login_required(self) -> bool {
        matches!(self, Kind::Favourites | Kind::History | Kind::Subscriptions)
    }

    /// The extractor's `suitable`.
    pub(crate) fn suitable(self, url: &str) -> bool {
        if self == Kind::Playlist
            && (super::tab::suitable(url) || urls::first_qs(url, "v").is_some())
        {
            return false;
        }
        self.regex().is_match(url)
    }

    /// `_real_extract`: the URL to follow.
    fn target(self, url: &str) -> Result<String> {
        let found = self.regex().captures(url).expect("a suitable URL");
        let id = found.name("id").map_or("", |id| id.as_str());
        Ok(match self {
            Kind::Consent => {
                let redirect = urls::parse_qs(url)
                    .into_iter()
                    .find(|(key, _)| key == "continue")
                    .and_then(|(_, values)| values.into_iter().last())
                    .and_then(|value| super::super::json_ld::url_or_none(&value));
                redirect.ok_or_else(|| {
                    Error::Reported(format!(
                        "[{}] Invalid cookie consent redirect URL",
                        self.name()
                    ))
                })?
            }
            Kind::Favourites => "https://www.youtube.com/playlist?list=LL".into(),
            Kind::WatchLater => "https://www.youtube.com/playlist?list=WL".into(),
            Kind::History => "https://www.youtube.com/feed/history".into(),
            Kind::Recommended => "https://www.youtube.com/feed/recommended".into(),
            Kind::Subscriptions => "https://www.youtube.com/feed/subscriptions".into(),
            Kind::LivestreamEmbed => format!("https://www.youtube.com/channel/{id}/live"),
            Kind::YtUser => format!("https://www.youtube.com/user/{id}"),
            Kind::ShortsAudioPivot => format!(
                "https://www.youtube.com/feed/sfv_audio_pivot?bp={}",
                audio_pivot_params(id)
            ),
            Kind::YtBe => format!(
                "https://www.youtube.com/watch?{}",
                urls::urlencode(&[
                    ("v".into(), vec![id.to_owned()]),
                    ("list".into(), vec![found["playlist_id"].to_owned()]),
                    ("feature".into(), vec!["youtu.be".into()]),
                ])
            ),
            Kind::Playlist => {
                let mut query = urls::parse_qs(url);
                if query.is_empty() {
                    query.push(("list".into(), vec![id.to_owned()]));
                }
                // Upstream smuggles `is_music_url` into the www URL; the
                // music host keeps it for `youtube:tab`.
                let host = if urls::is_music_url(url) {
                    "music.youtube.com"
                } else {
                    "www.youtube.com"
                };
                format!("https://{host}/playlist?{}", urls::urlencode(&query))
            }
            Kind::TruncatedId => {
                return Err(Error::Reported(format!(
                    "[{}] {id}: Incomplete YouTube ID {id}. URL {url} looks truncated.",
                    self.name()
                )));
            }
            Kind::TruncatedUrl => {
                return Err(Error::Reported(format!(
                    "[{}] Did you forget to quote the URL? Remember that & is a meta character in most shells, so you want to put the URL in quotes, like  yt-dlp \"https://www.youtube.com/watch?feature=foo&v=YE7VzlLtp-4\"  or simply  yt-dlp YE7VzlLtp-4  .",
                    self.name()
                )));
            }
        })
    }
}

/// `_generate_audio_pivot_params`: the `sfv_audio_pivot` browse params.
fn audio_pivot_params(video_id: &str) -> String {
    let mut params = b"\xf2\x05+\n)\x12'\n\x0b".to_vec();
    params.extend_from_slice(video_id.as_bytes());
    params.extend_from_slice(b"\x12\x0b");
    params.extend_from_slice(video_id.as_bytes());
    params.extend_from_slice(b"\x1a\x0b");
    params.extend_from_slice(video_id.as_bytes());
    // `urllib.parse.quote` (`/` is safe).
    base64::engine::general_purpose::STANDARD
        .encode(params)
        .replace('+', "%2B")
        .replace('=', "%3D")
}

/// `YoutubeYtUserIE.suitable` (a `ytuser:` pseudo-URL).
pub(crate) fn user_suitable(url: &str) -> bool {
    Kind::YtUser.suitable(url)
}

pub struct YoutubeRedirect(pub(crate) Kind);

impl super::super::Extractor for YoutubeRedirect {
    fn key(&self) -> &'static str {
        self.0.name()
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<crate::VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let text = if url.scheme() == "ytuser" {
            super::search::input_text(url)
        } else {
            super::super::input_text(url)
        };
        let kind = self.0;
        if !kind.suitable(&text) {
            return Ok(None);
        }
        let name = kind.name();
        crate::screen::set_extractor(name);
        super::login(name, "")?;
        super::initialize();
        if kind.login_required() {
            check_login_required(name)?;
        }
        crate::screen::screen(&format!(
            "[{name}] Extracting URL: {}",
            crate::screen::extracting_url(&text)
        ));
        let target = super::super::target_url(&kind.target(&text)?)?;
        Ok(Some(super::tab::follow(&target)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(kind: Kind, url: &str) -> String {
        assert!(kind.suitable(url), "{url}");
        kind.target(url).unwrap()
    }

    #[test]
    fn redirects_follow_upstream() {
        assert_eq!(
            target(
                Kind::YtBe,
                "https://youtu.be/yeWKywCrFtk?list=PL2qgrgXsNUG5ig9cat4ohreBjYLAPC0J5"
            ),
            "https://www.youtube.com/watch?v=yeWKywCrFtk&list=PL2qgrgXsNUG5ig9cat4ohreBjYLAPC0J5&feature=youtu.be"
        );
        assert_eq!(
            target(
                Kind::LivestreamEmbed,
                "https://www.youtube.com/embed/live_stream?channel=UC2_KI6RB__jGdlnK6dvFEZA"
            ),
            "https://www.youtube.com/channel/UC2_KI6RB__jGdlnK6dvFEZA/live"
        );
        assert_eq!(
            target(
                Kind::ShortsAudioPivot,
                "https://www.youtube.com/source/Lyj-MZSAA9o/shorts"
            ),
            // Pinned upstream's `_generate_audio_pivot_params('Lyj-MZSAA9o')`.
            "https://www.youtube.com/feed/sfv_audio_pivot?bp=8gUrCikSJwoLTHlqLU1aU0FBOW8SC0x5ai1NWlNBQTlvGgtMeWotTVpTQUE5bw%3D%3D"
        );
        assert_eq!(
            target(Kind::Playlist, "PLBB231211A4F62143"),
            "https://www.youtube.com/playlist?list=PLBB231211A4F62143"
        );
        assert_eq!(
            target(
                Kind::Playlist,
                "https://www.youtube.com/embed/videoseries?list=PL6IaIsEjSbf96XFRuNccS_RuEXwNdsoEu&x=a+b"
            ),
            "https://www.youtube.com/playlist?list=PL6IaIsEjSbf96XFRuNccS_RuEXwNdsoEu&x=a+b"
        );
        assert!(
            !Kind::Playlist.suitable("https://www.youtube.com/playlist?list=PLBB231211A4F62143")
        );
        assert!(
            !Kind::Playlist.suitable("https://www.youtube.com/embed/x?v=1&list=PLBB231211A4F62143")
        );
        assert_eq!(
            target(
                Kind::Consent,
                "https://consent.youtube.com/m?continue=https%3A%2F%2Fwww.youtube.com%2Flive%2FqVv6vCqciTM%3Fcbrd%3D1&gl=NL"
            ),
            "https://www.youtube.com/live/qVv6vCqciTM?cbrd=1"
        );
        for (kind, url) in [
            (Kind::Favourites, ":ytfavorites"),
            (Kind::Favourites, ":ytfavourites"),
            (Kind::Recommended, "https://youtube.com"),
            (Kind::Recommended, "https://www.youtube.com/?app=desktop"),
            (Kind::Recommended, ":ytrecommended"),
            (Kind::Subscriptions, ":ytsubscriptions"),
            (Kind::History, ":ythis"),
            (Kind::WatchLater, ":ytwatchlater"),
            (Kind::YtUser, "ytuser:phihag"),
            (
                Kind::TruncatedId,
                "https://www.youtube.com/watch?v=N_708QY7Ob",
            ),
            (Kind::TruncatedUrl, "https://www.youtube.com/watch?"),
            (
                Kind::TruncatedUrl,
                "youtube.com/watch?annotation_id=annotation_3951667041",
            ),
            (Kind::TruncatedUrl, "https://www.youtube.com/watch?hl=en-GB"),
        ] {
            assert!(kind.suitable(url), "{url}");
        }
        assert!(!Kind::TruncatedId.suitable("https://www.youtube.com/watch?v=N_708QY7Ob&t=1"));
        assert!(!Kind::Recommended.suitable("https://www.youtube.com/feed"));
    }
}
