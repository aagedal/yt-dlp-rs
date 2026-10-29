mod archive;
mod audio;
mod audio_download;
pub mod batch;
pub mod browser_cookies;
pub mod cache;
pub mod codecs;
pub mod config;
pub mod container;
mod control;
mod cookies;
mod cover_art;
mod crypto;
mod dash;
pub mod dates;
pub mod devalue;
mod download;
mod embed_metadata;
mod embed_subtitles;
mod error;
mod external_fd;
pub mod extractor;
mod f4m_fd;
mod ffmpeg_fd;
mod filetime;
mod fixup;
pub mod format_select;
pub mod format_sort;
mod fragment;
pub mod geo;
pub mod getpass;
mod growing;
mod hls;
mod html;
mod html_elements;
mod http;
pub mod impersonate;
mod indexed_fd;
mod info;
mod info_json;
mod info_order;
mod interrupt;
mod ism_fd;
pub mod js_to_json;
pub mod jsinterp;
mod json;
mod live_dash;
pub mod manifest_args;
mod match_filter;
mod md5;
mod media_index;
mod merge;
pub mod metadata_parser;
mod mhtml;
mod modify_chapters;
pub mod netrc;
pub mod no_formats;
pub mod output;
mod playlist;
mod postprocess;
pub mod pp_args;
mod retry;
mod rtmp_fd;
pub mod screen;
mod section_timecode;
pub mod sections;
mod sha1;
pub mod sponsorblock;
mod sqlite;
mod subtitle_convert;
mod subtitles;
mod thumbnail_process;
mod traffic;
mod video;
mod websocket_fd;
mod webvtt;
pub mod xattr;

pub use archive::{DownloadArchive, archive_id};
pub use audio::{AudioDownload, AudioOptions, audio_original_path, audio_output_path};
pub use control::{DownloadControl, Interrupt, RequestScope};
pub use cookies::CookieJar;
pub use download::{
    BUFFER_SIZE, DownloadOptions, DownloadPhase, DownloadProgress, FragmentBytes, MergeFiles,
    PartFiles, RESIZE_BUFFER,
};
pub use embed_metadata::COMPAT_EMBED_METADATA;
pub use embed_subtitles::{SubtitleEmbedding, SubtitleFile};
pub use error::{Error, Result};
pub use extractor::YoutubeOptions;
pub use format_sort::FormatSorter;
pub use http::HttpOptions;
pub use info::{
    ByteIndex, ByteRange, Chapter, DownloaderOptions, ExtractResult, Fragment, HlsAes128, Metadata,
    PlaylistInfo, PyNumber, StreamKind, Thumbnail, VideoFormat, VideoInfo,
};
pub use info::{
    COMPAT_FORMAT_SPEC, COMPAT_MANIFEST_FILESIZE_APPROX, MULTI_VIDEO, NESTED_INDEX, PendingEntries,
};
pub use info_json::{
    InfoJsonWrite, info_json_path, info_value, load_info_json, playlist_info_json,
    playlist_info_json_with, to_python_json, upstream_shape, video_info_json, video_info_json_with,
    write_info_json, write_info_json_with,
};
pub use match_filter::{EntrySelection, MatchFilters, MatchOutcome};
pub use output::{render_automatic_captions, render_subtitles};
pub use playlist::{PlaylistCounts, PlaylistSelection};
pub use postprocess::{InfoJsonEmbed, ModifyChapters, PostprocessOptions, ProcessedDownload};
pub use retry::{RetrySleep, SleepExpression, upstream_error_text};
pub use subtitle_convert::{ConvertedSubtitle, SubtitleConversion};
pub use subtitles::{
    RequestedSubtitles, SubtitleFormat, SubtitleMap, SubtitleOptions, SubtitleSelection, Subtitles,
    subtitle_paths,
};
pub use thumbnail_process::{ThumbnailConversion, fixup_webp};
pub use video::{VideoMode, VideoOptions};

use extractor::{Extractor, GenericHttp, Youtube};
use reqwest::blocking::Client;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use url::Url;

/// Upstream `build_format_selector` at startup: syntax and filter errors
/// (as `Error::Reported` with upstream's text) before any extraction.
pub fn validate_format_selector(expression: &str) -> Result<()> {
    format_select::parse(expression)
        .map(|_| ())
        .map_err(|error| Error::Reported(error.0))
}

/// Whole-extraction retry policy, independent of download retries.
/// Defaults to no retries to preserve existing library behavior.
#[derive(Debug, Clone, Default)]
pub struct ExtractionOptions {
    /// Additional attempts after transient HTTP or HTML body read failures.
    pub retries: u8,
    /// Ends the extraction with `Error::ExtractionCancelled` before its next
    /// request once stopped or cancelled.
    pub interrupt: Option<Interrupt>,
    /// Headers and cookies for requests to one origin (see [`RequestScope`]).
    pub scope: Option<RequestScope>,
}

/// The kind of a [`Manifest`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestKind {
    /// An HLS master or media playlist (M3U8).
    Hls,
    /// A DASH MPD.
    Dash,
}

/// An HLS or DASH manifest for [`YoutubeDl::extract_manifest`]: fetched
/// from `url`, or given inline as `data` with `url` as the base for its
/// relative URIs (and the origin a [`RequestScope`] usually names).
#[derive(Debug, Clone)]
pub struct Manifest {
    pub kind: ManifestKind,
    /// The manifest's URL (HTTP or HTTPS).
    pub url: Url,
    /// The manifest text; `None` fetches `url`.
    pub data: Option<String>,
    /// The result's `id` (default: the manifest file name's stem).
    pub id: Option<String>,
    /// The result's `title` (default: the `id`).
    pub title: Option<String>,
    /// The result's `webpage_url` (default: `url`).
    pub webpage_url: Option<Url>,
}

impl Manifest {
    /// A manifest fetched from `url`.
    pub fn new(kind: ManifestKind, url: Url) -> Self {
        Self {
            kind,
            url,
            data: None,
            id: None,
            title: None,
            webpage_url: None,
        }
    }

    /// An inline manifest whose relative URIs resolve against `base`.
    pub fn inline(kind: ManifestKind, base: Url, data: impl Into<String>) -> Self {
        Self {
            data: Some(data.into()),
            ..Self::new(kind, base)
        }
    }
}

pub struct YoutubeDl {
    client: Client,
    cookies: Arc<CookieJar>,
    extractors: Vec<Box<dyn Extractor>>,
    ffmpeg: Option<PathBuf>,
    ffprobe: Option<PathBuf>,
    /// `--section-timecode` (port extension), with the remux/recode mapping
    /// that decides the final container.
    section_timecode: Option<Option<VideoOptions>>,
    merge_output_format: Option<Vec<String>>,
    /// `--video-multistreams`/`--audio-multistreams`.
    multistreams: (bool, bool),
    /// `--live-from-start`.
    live_from_start: bool,
    /// `--growing-file` (port extension).
    growing_file: bool,
    /// `--keep-partial-on-stop` (port extension).
    keep_partial_on_stop: bool,
    /// The YouTube JavaScript runtime, for live-from-start refetches.
    youtube_runtime: Vec<extractor::JsRuntime>,
    format_sort: FormatSortOptions,
    prefer_best: bool,
    /// Global request headers and proxy for FFmpeg network inputs.
    headers: reqwest::header::HeaderMap,
    /// Upstream `params['http_headers']` as `_calc_headers` starts from it
    /// (title-cased names in order).
    info_headers: Vec<(String, String)>,
    proxy: Option<String>,
    ffmpeg_settings: ffmpeg_fd::Settings,
    /// `--http-chunk-size` and `--test`.
    http_chunk_size: Option<u64>,
    /// Leave extractors' pending playlist entries for [`YoutubeDl::fetch_pending`].
    defer_entries: bool,
    test: bool,
    /// `--use-extractors` (upstream extractor names in matching order) and
    /// `--force-generic-extractor`.
    allowed_extractors: Option<Vec<&'static str>>,
    force_generic: bool,
    /// `--default-search`.
    default_search: Option<String>,
    check_formats: CheckFormats,
    /// `-N`/`--concurrent-fragments`.
    concurrent_fragments: usize,
    /// `--keep-fragments`.
    keep_fragments: bool,
    /// `--enable-file-urls`.
    enable_file_urls: bool,
    /// `--downloader [PROTO:]NAME` choices by simplified protocol (`default`
    /// for the rest): `native`, FFmpeg (`ffmpeg`/`avconv` names or paths) or
    /// an external downloader name or path.
    downloaders: std::collections::HashMap<String, String>,
    /// The external downloaders' parameters.
    external_settings: external_fd::ExternalSettings,
}

pub use external_fd::ExternalSettings;

/// Upstream `get_external_downloader` names FFmpegFD/AVconvFD.
fn is_ffmpeg_name(name: &str) -> bool {
    let base = Path::new(name)
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    name != "native" && (base.contains("ffmpeg") || base.contains("avconv"))
}

/// `info_dict.get('is_live')` (from `live_status`).
pub(crate) fn is_live(info: &VideoInfo) -> bool {
    info.metadata.live_status.as_deref() == Some("is_live")
}

/// Whether every selected format comes from a fragment generator
/// (`--live-from-start`).
fn live_generated(info: &VideoInfo) -> bool {
    const GENERATOR: &str = "http_dash_segments_generator";
    if info.requested_formats.is_empty() {
        info.protocol.as_deref() == Some(GENERATOR)
    } else {
        info.requested_formats
            .iter()
            .all(|format| format.determine_protocol() == GENERATOR)
    }
}

/// A transport error as urllib words it: `[Errno N] TEXT` for an OS error.
/// HlsFD's `Total fragments` for `info` after `refresh_hls`: the whole
/// playlist's fragments even when only a discontinuity section downloads.
pub fn total_fragments(info: &VideoInfo) -> usize {
    FRAGMENT_TOTAL.with(|total| match &*total.borrow() {
        Some((url, count)) if *url == info.url => *count,
        _ => info.fragments.len(),
    })
}

thread_local! {
    /// The refetched playlist URL and its fragment count (section formats).
    static FRAGMENT_TOTAL: std::cell::RefCell<Option<(Url, usize)>> = const { std::cell::RefCell::new(None) };
    /// The URL whose download `announce_download` already reported.
    static ANNOUNCED: std::cell::RefCell<Option<Url>> = const { std::cell::RefCell::new(None) };
}

/// Upstream `dl`'s `Invoking … downloader on "URL"[, "URL"]` debug line;
/// `data:` URLs are abbreviated.
fn report_invoking(name: &str, urls: &[&Url]) {
    if !screen::verbose() {
        return;
    }
    let urls: Vec<String> = urls
        .iter()
        .map(|url| {
            let url = url.as_str();
            if url.starts_with("data:") {
                format!("{},<data>", url.split(',').next().unwrap_or(url))
            } else {
                url.to_owned()
            }
        })
        .collect();
    screen::debug(&format!(
        "Invoking {name} downloader on \"{}\"",
        urls.join("\", \"")
    ));
}

/// Upstream `shorten_protocol_name(proto, simplify=True)`.
fn simplify_protocol(protocol: &str) -> &str {
    match protocol {
        "https" => "http",
        "ftps" => "ftp",
        "m3u8" | "m3u8_native" => "m3u8",
        "http_dash_segments" | "http_dash_segments_generator" => "dash",
        "rtmp_ffmpeg" => "rtmp",
        other => other,
    }
}

/// Upstream `--fixup POLICY` and `--hls-use-mpegts`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FixupOptions {
    /// `never`, `ignore`, `warn`, `detect_or_warn` (default) or `force`.
    pub policy: Option<String>,
    /// `--hls-use-mpegts`/`--no-hls-use-mpegts` (`None` when not given:
    /// MPEG-TS for live streams).
    pub hls_use_mpegts: Option<bool>,
    /// Upstream `postprocessed_by_ffmpeg`: a remux/recode to another
    /// container follows, so only `FixupStretched` applies.
    pub converted: bool,
}

/// Upstream `check_formats`: `--check-formats` (`selected`),
/// `--check-all-formats` (`True`) and `--no-check-formats` (`False`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CheckFormats {
    /// Only formats that need testing (none in the represented extractors).
    #[default]
    Default,
    Selected,
    All,
    Never,
}

/// Upstream `dl(temp, format, test=True)`: a quiet `_TEST_FILE_SIZE` HTTP
/// download, or the first fragment of a fragmented format.
fn test_format(client: &Client, info: &VideoInfo, format: &VideoFormat) -> bool {
    let mut single = info.with_selected_format(format);
    single.fragments.truncate(1);
    let Ok(directory) = tempfile::tempdir() else {
        return false;
    };
    let target = directory.path().join("format.tmp");
    // The test parameters carry no retry counts, so nothing is retried.
    let options = DownloadOptions {
        part_files: PartFiles::NoPart { resume: false },
        skip_unavailable_fragments: Some(false),
        retries: 0,
        fragment_retries: 0,
        ..Default::default()
    };
    let mut quiet = |_| std::ops::ControlFlow::Continue(());
    let result = if single.fragments.is_empty() {
        download::download(
            client,
            &single,
            &target,
            options,
            Some(download::Chunking {
                size: download::TEST_FILE_SIZE,
                test: true,
            }),
            &mut quiet,
        )
    } else {
        fragment::download(client, &single, &target, options, 1, false, &mut quiet)
    };
    // Errors are still reported by the quiet test downloader.
    if let Err(error) = &result {
        screen::stderr(&format!("ERROR: {error}"));
    }
    result.is_ok()
}

/// Upstream `parse_duration`: whole durations are `int`, others `float`.
pub fn parse_duration(text: &str) -> Option<PyNumber> {
    match_filter::parse_duration_number(text)
}

/// A `url_transparent` playlist entry's URL and fields (see
/// [`YoutubeDl::resolve_entry`]).
pub(crate) const TRANSPARENT: &str = "__yt_dlp_rs_url_transparent";

/// A `url` result an extractor returns for the downloader to follow with
/// every registered extractor (a configured one, like YouTube's).
pub(crate) const FOLLOW_URL: &str = "__yt_dlp_rs_follow_url";

/// A playlist entry's processing error, reported with that entry.
pub(crate) const ENTRY_ERROR: &str = "__yt_dlp_rs_entry_error";
/// A fetched pending entry that was already processed (see
/// [`YoutubeDl::fetch_pending`]).
pub(crate) const PROCESSED_ENTRY: &str = "__yt_dlp_rs_processed_entry";
/// A paged playlist entry whose page failed to download: upstream's `None`
/// entry after `_handle_extraction_exceptions` reported the error (see
/// `extractor::paged`).
pub(crate) const FETCH_ERROR: &str = "__yt_dlp_rs_fetch_error";
/// The paged list's length once its failed page ended it.
pub(crate) const FETCH_ERROR_COUNT: &str = "__yt_dlp_rs_fetch_error_count";
/// An extractor's top-level `url_transparent` result: `{url, fields}`, the
/// target extracted through the registry and updated with the non-`None`
/// fields (`process_ie_result`).
pub(crate) const FOLLOW_TRANSPARENT: &str = "__yt_dlp_rs_follow_transparent";

/// Upstream `bug_reports_message()` after a lowercase-ending text.
pub(crate) const BUG_REPORTS_MESSAGE: &str = "please report this issue on  https://github.com/yt-dlp/yt-dlp/issues?q= , filling out the appropriate issue template. Confirm you are on the latest version using  yt-dlp -U";

/// Upstream extractor names this port knows, in `gen_extractor_classes`
/// order (`end` is the unsupported-URL fallback).
const EXTRACTOR_NAMES: &[&str] = &[
    "1News",
    "9gag",
    "9News",
    "acast",
    "acast:channel",
    "agalega:videos",
    "AitubeKZVideo",
    "Alibaba",
    "AliExpressLive",
    "AlJazeera",
    "AmadeusTV",
    "Angel",
    "APA",
    "apple:podcasts",
    "archive.org",
    "ArcPublishing",
    "ARDAudiothek",
    "ARDAudiothekPlaylist",
    "ARDMediathek",
    "ARDMediathekCollection",
    "ArteTVCategory",
    "ArteTVEmbed",
    "ArteTV",
    "ArteTVPlaylist",
    "AtScaleConfEvent",
    "AudiMedia",
    "AudioBoom",
    "audiomack",
    "audiomack:album",
    "AZMedien",
    "BaiduVideo",
    "Bandcamp",
    "BeaconTv",
    "Beatport",
    "Beeg",
    "BehindKink",
    "BerufeTV",
    "bfmtv",
    "bfmtv:article",
    "bfmtv:live",
    "Bigo",
    "Bild",
    "BiliBili",
    "BitChute",
    "BitChuteChannel",
    "Bitmovin",
    "blogger.com",
    "Bloomberg",
    "Bluesky",
    "BongaCams",
    "BostonGlobe",
    "BoxCastVideo",
    "BreitBart",
    "brightcove:new",
    "BTVPlus",
    "Bundesliga",
    "Bundestag",
    "BusinessInsider",
    "Caltrans",
    "CAM4",
    "CanalAlpha",
    "Cellebrite",
    "CGTN",
    "CharlieRose",
    "Chaturbate",
    "CiscoLiveSearch",
    "CiscoLiveSession",
    "CJSW",
    "Clipchamp",
    "CloudflareStream",
    "CloudyCDN",
    "Clyp",
    "CNBCVideo",
    "Coub",
    "CozyTV",
    "cpac",
    "cpac:playlist",
    "Craftsy",
    "CrooksAndLiars",
    "Crtvg",
    "CultureUnplugged",
    "DailyMail",
    "dailymotion",
    "dailymotion:playlist",
    "dailymotion:search",
    "dailymotion:user",
    "damtomo:record",
    "damtomo:video",
    "daystar:clip",
    "DBTV",
    "democracynow",
    "DeuxM",
    "DeuxMNews",
    "Digiteka",
    "DiscogsReleasePlaylist",
    "dlive:stream",
    "dlive:vod",
    "Douyin",
    "Dropbox",
    "DrTalks",
    "drtv",
    "drtv:live",
    "drtv:season",
    "drtv:series",
    "Dumpert",
    "Duoplay",
    "EbaumsWorld",
    "Ebay",
    "egghead:course",
    "egghead:lesson",
    "Elonet",
    "ElPais",
    "ElTreceTV",
    "Epoch",
    "Eporner",
    "Erocast",
    "ERRArhiiv",
    "ERRJupiter",
    "EttuTv",
    "EuropeanTour",
    "EUScreen",
    "Expressen",
    "facebook:ads",
    "facebook",
    "facebook:reel",
    "FacebookPluginsVideo",
    "FacebookRedirectURL",
    "Fathom",
    "faz.net",
    "Fczenit",
    "FilmArchiv",
    "filmon",
    "filmon:channel",
    "Filmweb",
    "FiveTV",
    "Flickr",
    "FootyRoom",
    "Formula1",
    "FrancaisFacile",
    "francetv",
    "franceinfo",
    "francetv:site",
    "Freesound",
    "freespeech.org",
    "freetv:series",
    "FreeTvMovies",
    "Funker530",
    "FuyinTV",
    "Gab",
    "GameSpot",
    "GameStar",
    "Gaskrank",
    "Genius",
    "GeniusLyrics",
    "Golem",
    "goodgame:stream",
    "HearThisAt",
    "html5",
    "Glide",
    "GodResource",
    "GodTube",
    "GoogleDrive:Folder",
    "GoPro",
    "GoogleDrive",
    "GoToStage",
    "Graspop",
    "Gronkh",
    "Groupon",
    "Harpodeon",
    "HellPorno",
    "HistoricFilms",
    "HitRecord",
    "HollywoodReporter",
    "HollywoodReporterPlaylist",
    "href.li",
    "HSEProduct",
    "HSEShow",
    "Huajiao",
    "HuffPost",
    "Hypem",
    "Hytale",
    "iheartradio",
    "iheartradio:podcast",
    "IlPost",
    "Iltalehti",
    "imdb",
    "imdb:list",
    "Imgur",
    "imgur:album",
    "imgur:gallery",
    "Ina",
    "Inc",
    "IndavideoEmbed",
    "Instagram",
    "InstagramIOS",
    "instagram:story",
    "instagram:tag",
    "instagram:user",
    "Internazionale",
    "IslamChannel",
    "IslamChannelSeries",
    "IsraelNationalNews",
    "ivideon",
    "Ivoox",
    "Jamendo",
    "JamendoAlbum",
    "Jove",
    "JStream",
    "JWPlatform",
    "Kaltura",
    "KankaNews",
    "kick:clip",
    "kick:clips",
    "kick:live",
    "kick:videos",
    "kick:vod",
    "Kicker",
    "KickStarter",
    "Kika",
    "KikaPlaylist",
    "KinoPoisk",
    "Kommunetv",
    "KompasVideo",
    "KTH",
    "Ku6",
    "KukuluLive",
    "LastFM",
    "LastFMPlaylist",
    "LastFMUser",
    "lbry",
    "lbry:channel",
    "lbry:playlist",
    "LCI",
    "LeFigaroVideoEmbed",
    "LeFigaroVideoSection",
    "LEGO",
    "Libsyn",
    "LinkedIn",
    "ListenNotes",
    "LiTV",
    "Livestreamfails",
    "Lnk",
    "loc",
    "loom",
    "LRTRadio",
    "LRTStream",
    "LRTVOD",
    "maariv.co.il",
    "MagellanTV",
    "mangomolo:live",
    "mangomolo:video",
    "MaoriTV",
    "massengeschmack.tv",
    "Masters",
    "MatchiTV",
    "MatchTV",
    "MBN",
    "MDR",
    "MedalTV",
    "media.ccc.de",
    "Mediaite",
    "MediaKlikk",
    "MediaWorksNZVOD",
    "megaphone.fm",
    "Meipai",
    "MelonVOD",
    "mir24.tv",
    "MirrorCoUK",
    "mixcloud",
    "Mixlr",
    "MixlrRecoring",
    "MochaVideo",
    "Monstercat",
    "monstersiren",
    "moviepilot",
    "MovingImage",
    "MuseScore",
    "Mux",
    "MySpass",
    "MyVidster",
    "n-tv.de",
    "NascarClassics",
    "NetAppCollection",
    "NetAppVideo",
    "Netzkino",
    "Newsy",
    "NFHSNetwork",
    "NhkVod",
    "NhkVodProgram",
    "niconico",
    "niconico:history",
    "niconico:playlist",
    "niconico:series",
    "niconico:tag",
    "NiconicoUser",
    "nicovideo:search",
    "nicovideo:search:date",
    "nicovideo:search_url",
    "NobelPrize",
    "NoicePodcast",
    "NOSNLArticle",
    "NovaPlay",
    "NowCanal",
    "Npr",
    "NRK",
    "NRKPlaylist",
    "NRKRadioPodkast",
    "NRKTV",
    "NRKTVDirekte",
    "NRKTVEpisode",
    "NRKTVSeason",
    "NRKTVSeries",
    "nts.live",
    "NZOnScreen",
    "NZZ",
    "Odnoklassniki",
    "OlympicsReplay",
    "on24",
    "OneFootball",
    "OnePlacePodcast",
    "orf:on",
    "orf:podcast",
    "orf:radio",
    "OutsideTV",
    "Parlview",
    "parti:livestream",
    "parti:video",
    "PearVideo",
    "PeerTube",
    "PeerTube:Playlist",
    "peer.tv",
    "periscope",
    "PGATour",
    "Photobucket",
    "PiaLive",
    "picarto",
    "picarto:vod",
    "Pinkbike",
    "Pinterest",
    "PlayerFm",
    "PodbayFM",
    "PodbayFMChannel",
    "Popcorntimes",
    "PopcornTV",
    "PremiershipRugby",
    "PressTV",
    "ProjectVeritas",
    "Pyvideo",
    "QingTing",
    "radiokapital",
    "radiokapital:show",
    "RadioZetPodcast",
    "Reddit",
    "ReverbNation",
    "RinseFM",
    "RinseFMArtistPlaylist",
    "RottenTomatoes",
    "RoyaLive",
    "rte",
    "rte:radio",
    "RTRFM",
    "rtve.es:alacarta",
    "rtve.es:audio",
    "rtve.es:live",
    "rtve.es:television",
    "rtvslo.si",
    "rtvslo.si:show",
    "RudoVideo",
    "Rumble",
    "RumbleEmbed",
    "rutube",
    "rutube:embed",
    "Ruv",
    "ruv.is:spila",
    "S4C",
    "S4CSeries",
    "Sapo",
    "SBS",
    "Screen9",
    "Screencastify",
    "ScreenRec",
    "Sen",
    "Servus",
    "SharePoint",
    "simplecast",
    "simplecast:episode",
    "simplecast:podcast",
    "Skeb",
    "sky:news",
    "sky:news:story",
    "sky:sports",
    "sky:sports:news",
    "SkylineWebcams",
    "SkyNewsAU",
    "SnapchatSpotlight",
    "soundcloud",
    "soundgasm",
    "soundgasm:profile",
    "Spiegel",
    "Sport5",
    "Spreaker",
    "SpreakerShow",
    "SRGSSR",
    "SRGSSRPlay",
    "startrek",
    "startv",
    "Steam",
    "SteamCommunity",
    "SteamCommunityBroadcast",
    "Streamable",
    "StreamCZ",
    "StreetVoice",
    "stv:player",
    "stvr",
    "Substack",
    "sverigesradio:episode",
    "sverigesradio:publication",
    "svt:page",
    "svt:play",
    "svt:play:series",
    "SztvHu",
    "TedEmbed",
    "TedPlaylist",
    "TedSeries",
    "TedTalk",
    "Tele13",
    "TeleBruxelles",
    "Telegraaf",
    "telegram:embed",
    "TeleTask",
    "Telewebion",
    "TF1",
    "TheGuardianPodcast",
    "TheHighWire",
    "TheIntercept",
    "TheStar",
    "TheSun",
    "ThisAmericanLife",
    "ThreeSpeak",
    "ThreeSpeakUser",
    "TikTok",
    "tiktok:collection",
    "tiktok:live",
    "tiktok:user",
    "ToonGoggles",
    "Toypics",
    "TrtCocukVideo",
    "TrtWorld",
    "TruNews",
    "Tumblr",
    "tv.dfb.de",
    "TV2",
    "TV2Article",
    "TV2DK",
    "TV2DKBornholmPlay",
    "tv2play.hu",
    "tv2playseries.hu",
    "TV4",
    "TV5MONDE",
    "TVANouvelles",
    "TVANouvellesArticle",
    "tvaplus",
    "tvigle",
    "TVIPlayer",
    "tvnoe",
    "Tweakers",
    "twitch:clips",
    "twitch:collection",
    "twitch:stream",
    "twitch:videos:clips",
    "twitch:videos:collections",
    "twitch:videos",
    "twitch:vod",
    "twitter",
    "twitter:amplify",
    "twitter:broadcast",
    "twitter:card",
    "twitter:shortener",
    "twitter:spaces",
    "UDNEmbed",
    "23video",
    "24tv.ua",
    "umg:de",
    "Unistra",
    "UnitedNationsWebTv",
    "uol.com.br",
    "URPlay",
    "USAToday",
    "ustudio",
    "ustudio:embed",
    "Vbox7",
    "Veo",
    "video.arnes.si",
    "VideoPress",
    "VidLii",
    "Vidyard",
    "vimeo",
    "Viously",
    "Visir",
    "vk",
    "vk:uservideos",
    "vk:wallpost",
    "VKPlay",
    "VKPlayLive",
    "vm.tiktok",
    "Vocaroo",
    "VODPlatform",
    "VTV",
    "VTVGo",
    "washingtonpost",
    "washingtonpost:article",
    "wat.tv",
    "Webcamerapl",
    "Weibo",
    "WeiboVideo",
    "Whyp",
    "wikimedia.org",
    "Wimbledon",
    "Wistia",
    "WistiaChannel",
    "WistiaPlaylist",
    "WorldStarHipHop",
    "WSJ",
    "WSJArticle",
    "XboxClips",
    "XiaoHongShu",
    "Xinpianchang",
    "XNXX",
    "XVideos",
    "xvideos:quickies",
    "Yappy",
    "YappyProfile",
    "yfanefa",
    "YleAreena",
    "YouJizz",
    "youtube:clip",
    "youtube:consent",
    "youtube:favorites",
    "youtube:history",
    "youtube",
    "YoutubeLivestreamEmbed",
    "youtube:music:search_url",
    "youtube:notif",
    "youtube:playlist",
    "youtube:recommended",
    "youtube:search",
    "youtube:search_url",
    "youtube:shorts:pivot:audio",
    "youtube:subscriptions",
    "youtube:tab",
    "youtube:truncated_id",
    "youtube:truncated_url",
    "youtube:watchlater",
    "YoutubeYtBe",
    "youtube:user",
    "zdf",
    "zdf:channel",
    "ZetlandDKArticle",
    "Zhihu",
    "Zype",
    "generic",
    "end",
];

/// Upstream `orderedSet_from_options(names, {'all', 'default'}, use_regex=True)`.
fn ordered_extractors(
    options: &[String],
    start: Vec<&'static str>,
) -> std::result::Result<Vec<&'static str>, String> {
    let mut requested = start;
    for option in options {
        let (discard, value) = match option.strip_prefix('-') {
            Some(value) => (true, value),
            None => (false, option.as_str()),
        };
        let alias: Option<&[&str]> = match value {
            "all" => Some(EXTRACTOR_NAMES),
            "default" => Some(&EXTRACTOR_NAMES[..EXTRACTOR_NAMES.len() - 1]),
            _ => None,
        };
        if let Some(names) = alias {
            let flipped: Vec<String> = names
                .iter()
                .map(|name| {
                    if discard {
                        format!("-{name}")
                    } else {
                        (*name).to_owned()
                    }
                })
                .collect();
            requested = ordered_extractors(&flipped, requested)?;
            continue;
        }
        let regex = regex::RegexBuilder::new(&format!("^(?:{value})$"))
            .case_insensitive(true)
            .build()
            .map_err(|_| format!("Wrong regex for allowed_extractors: {value}"))?;
        for name in EXTRACTOR_NAMES.iter().filter(|name| regex.is_match(name)) {
            if discard {
                requested.retain(|item| item != name);
            } else {
                requested.push(name);
            }
        }
    }
    let mut seen = Vec::new();
    requested.retain(|name| {
        let new = !seen.contains(name);
        seen.push(*name);
        new
    });
    Ok(requested)
}

/// Python `urlparse(url).scheme` is nonempty.
pub(crate) fn has_scheme(input: &str) -> bool {
    input.split_once(':').is_some_and(|(scheme, _)| {
        scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    })
}

/// Upstream `-S`/`--format-sort`, `--format-sort-force` and
/// `--prefer-free-formats`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FormatSortOptions {
    pub fields: Vec<String>,
    pub force: bool,
    pub prefer_free: bool,
}

impl YoutubeDl {
    pub fn new() -> Result<Self> {
        Self::with_youtube_options(YoutubeOptions::default())
    }

    /// Configure YouTube extraction. The runtime path is inspected only when a
    /// YouTube player challenge needs solving; construction does not launch it.
    pub fn with_youtube_options(options: YoutubeOptions) -> Result<Self> {
        Self::with_options(options, HttpOptions::default())
    }

    /// Configure global HTTP defaults together with YouTube extraction.
    /// Per-request extractor and transport headers override these defaults.
    pub fn with_options(youtube: YoutubeOptions, http: HttpOptions) -> Result<Self> {
        let headers = http.header_map()?;
        let proxy_url = http.proxy.clone();
        let proxy = http.transport_config()?;
        let cookies = http.cookies.clone().unwrap_or_default();
        cookies::register(&cookies);
        extractor::NO_PLAYLIST.store(youtube.no_playlist, std::sync::atomic::Ordering::Relaxed);
        extractor::AGE_LIMIT.store(
            youtube.age_limit.unwrap_or(i64::MIN),
            std::sync::atomic::Ordering::Relaxed,
        );
        extractor::GET_COMMENTS.store(youtube.get_comments, std::sync::atomic::Ordering::Relaxed);
        extractor::youtube::live_chat::SKIP_DOWNLOAD
            .store(youtube.skip_download, std::sync::atomic::Ordering::Relaxed);
        if let Ok(mut agent) = extractor::USER_AGENT.write() {
            *agent = headers
                .get(reqwest::header::USER_AGENT)
                .map(|value| String::from_utf8_lossy(value.as_bytes()).into_owned())
                .unwrap_or_default();
        }
        let builder = Client::builder()
            .user_agent(concat!("yt-dlp-rs/", env!("CARGO_PKG_VERSION")))
            .default_headers(headers.clone())
            .cookie_provider(cookies.clone())
            .redirect(reqwest::redirect::Policy::limited(10))
            // Preserve an HTML5 page's explicit referrer across media redirects.
            .referer(false)
            // Python's http.client sends `User-Agent`, not `user-agent`;
            // Cloudflare rejects lowercase HTTP/1.1 header names on some
            // sites (kick.com answers 403).
            .http1_title_case_headers()
            .connection_verbose(http.print_traffic);
        if http.print_traffic {
            traffic::enable();
        }
        let client = proxy.apply(builder)?.build()?;
        impersonate::configure(&http, &cookies)?;
        let youtube_runtime = youtube.runtimes();
        Ok(Self {
            client,
            ffmpeg: None,
            merge_output_format: None,
            multistreams: (false, false),
            live_from_start: false,
            growing_file: false,
            keep_partial_on_stop: false,
            youtube_runtime,
            format_sort: FormatSortOptions::default(),
            prefer_best: false,
            headers: headers.clone(),
            info_headers: headers
                .iter()
                .map(|(name, value)| {
                    (
                        ffmpeg_fd::py_title(name.as_str()),
                        String::from_utf8_lossy(value.as_bytes()).into_owned(),
                    )
                })
                .collect(),
            proxy: proxy_url,
            ffmpeg_settings: ffmpeg_fd::Settings::default(),
            http_chunk_size: None,
            defer_entries: false,
            test: false,
            allowed_extractors: None,
            force_generic: false,
            default_search: None,
            check_formats: CheckFormats::Default,
            concurrent_fragments: 1,
            keep_fragments: false,
            enable_file_urls: false,
            downloaders: Default::default(),
            external_settings: Default::default(),
            ffprobe: None,
            section_timecode: None,
            cookies: cookies.clone(),
            extractors: vec![
                Box::new(extractor::YoutubeTab {
                    no_playlist: youtube.no_playlist,
                }),
                Box::new(extractor::AGalega),
                Box::new(extractor::AitubeKzVideo),
                Box::new(extractor::AlJazeera),
                Box::new(extractor::AliExpressLive),
                Box::new(extractor::Alibaba),
                Box::new(extractor::Apa),
                Box::new(extractor::ArcPublishing),
                Box::new(extractor::AzMedien),
                Box::new(extractor::BaiduVideo),
                Box::new(extractor::BerufeTv),
                Box::new(extractor::Bfmtv),
                Box::new(extractor::BfmtvArticle),
                Box::new(extractor::BfmtvLive),
                Box::new(extractor::BoxCastVideo),
                Box::new(extractor::Bundestag),
                Box::new(extractor::BtvPlus),
                Box::new(extractor::BusinessInsider),
                Box::new(extractor::CanalAlpha),
                Box::new(extractor::Cellebrite),
                Box::new(extractor::Chaturbate),
                Box::new(extractor::CiscoLiveSession),
                Box::new(extractor::CiscoLiveSearch),
                Box::new(extractor::Cpac),
                Box::new(extractor::CpacPlaylist),
                Box::new(extractor::Craftsy),
                Box::new(extractor::DailyMail),
                Box::new(extractor::Dbtv),
                Box::new(extractor::DeuxM),
                Box::new(extractor::DeuxMNews),
                Box::new(extractor::Digiteka),
                Box::new(extractor::DiscogsReleasePlaylist),
                Box::new(extractor::Duoplay),
                Box::new(extractor::EggheadCourse),
                Box::new(extractor::EggheadLesson),
                Box::new(extractor::ElPais),
                Box::new(extractor::ElTreceTv),
                Box::new(extractor::Expressen),
                Box::new(extractor::Faz),
                Box::new(extractor::FilmOn),
                Box::new(extractor::FilmOnChannel),
                Box::new(extractor::FiveTv),
                Box::new(extractor::FootyRoom),
                Box::new(extractor::FrancaisFacile),
                Box::new(extractor::FreeTv),
                Box::new(extractor::FreeTvMovies),
                Box::new(extractor::Freespeech),
                Box::new(extractor::Gaskrank),
                Box::new(extractor::Genius),
                Box::new(extractor::GeniusLyrics),
                Box::new(extractor::GoPro),
                Box::new(extractor::Groupon),
                Box::new(extractor::HuffPost),
                Box::new(extractor::Hytale),
                Box::new(extractor::IndavideoEmbed),
                Box::new(extractor::IslamChannel),
                Box::new(extractor::IslamChannelSeries),
                Box::new(extractor::Ivideon),
                Box::new(extractor::Kika),
                Box::new(extractor::KikaPlaylist),
                Box::new(extractor::KinoPoisk),
                Box::new(extractor::KompasVideo),
                Box::new(extractor::KukuluLive),
                Box::new(extractor::Lci),
                Box::new(extractor::LeFigaroVideoEmbed),
                Box::new(extractor::LeFigaroVideoSection),
                Box::new(extractor::Lego),
                Box::new(extractor::LibraryOfCongress),
                Box::new(extractor::LastFm),
                Box::new(extractor::LastFmPlaylist),
                Box::new(extractor::LastFmUser),
                Box::new(extractor::ListenNotes),
                Box::new(extractor::MagellanTv),
                Box::new(extractor::MassengeschmackTv),
                Box::new(extractor::Mbn),
                Box::new(extractor::Mdr),
                Box::new(extractor::MediaKlikk),
                Box::new(extractor::MediaWorksNzVod),
                Box::new(extractor::MelonVod),
                Box::new(extractor::MonsterSirenHypergryphMusic),
                Box::new(extractor::Monstercat),
                Box::new(extractor::Moviepilot),
                Box::new(extractor::NascarClassics),
                Box::new(extractor::NetAppCollection),
                Box::new(extractor::NetAppVideo),
                Box::new(extractor::NfhsNetwork),
                Box::new(extractor::NoicePodcast),
                Box::new(extractor::NovaPlay),
                Box::new(extractor::NowCanal),
                Box::new(extractor::NtvDe),
                Box::new(extractor::OlympicsReplay),
                Box::new(extractor::On24),
                Box::new(extractor::OutsideTv),
                Box::new(extractor::PgaTour),
                Box::new(extractor::Picarto),
                Box::new(extractor::PicartoVod),
                Box::new(extractor::Popcorntimes),
                Box::new(extractor::Rtvs),
                Box::new(extractor::Rtvslo),
                Box::new(extractor::RtvsloShow),
                Box::new(extractor::RudoVideo),
                Box::new(extractor::Screencastify),
                Box::new(extractor::Servus),
                Box::new(extractor::SharePoint),
                Box::new(extractor::Skeb),
                Box::new(extractor::SkyNews),
                Box::new(extractor::SkyNewsStory),
                Box::new(extractor::SkySports),
                Box::new(extractor::SkySportsNews),
                Box::new(extractor::Sport5),
                Box::new(extractor::StarTrek),
                Box::new(extractor::StarTv),
                Box::new(extractor::StreamCz),
                Box::new(extractor::MuseScore),
                Box::new(extractor::StvPlayer),
                Box::new(extractor::Tele13),
                Box::new(extractor::Telewebion),
                Box::new(extractor::TeleBruxelles),
                Box::new(extractor::PiaLive),
                Box::new(extractor::LiTv),
                Box::new(extractor::DamtomoRecord),
                Box::new(extractor::DamtomoVideo),
                Box::new(extractor::Tf1),
                Box::new(extractor::ToonGoggles),
                Box::new(extractor::TruNews),
                Box::new(extractor::Tv24UaVideo),
                Box::new(extractor::Tv2Hu),
                Box::new(extractor::Tv2HuSeries),
                Box::new(extractor::Tva),
                Box::new(extractor::Tvigle),
                Box::new(extractor::UdnEmbed),
                Box::new(extractor::UrPlay),
                Box::new(extractor::Vbox7),
                Box::new(extractor::VidLii),
                Box::new(extractor::Visir),
                Box::new(extractor::VodPlatform),
                Box::new(extractor::Vtv),
                Box::new(extractor::VtvGo),
                Box::new(extractor::UsaToday),
                Box::new(extractor::WashingtonPost),
                Box::new(extractor::WashingtonPostArticle),
                Box::new(extractor::Wat),
                Box::new(extractor::Wsj),
                Box::new(extractor::WsjArticle),
                Box::new(extractor::Yappy),
                Box::new(extractor::YappyProfile),
                Box::new(extractor::YoutubeSearchUrl),
                Box::new(extractor::YoutubeClip),
                Box::new(extractor::Reddit),
                Box::new(extractor::Twitter),
                Box::new(extractor::TwitterCard),
                Box::new(extractor::TwitterAmplify),
                Box::new(extractor::TwitterBroadcast),
                Box::new(extractor::TwitterSpaces),
                Box::new(extractor::TwitterShortener),
                Box::new(extractor::Periscope),
                Box::new(extractor::TikTok),
                Box::new(extractor::TikTokVm),
                Box::new(extractor::TikTokUser),
                Box::new(extractor::TikTokCollection),
                Box::new(extractor::TikTokLive),
                Box::new(extractor::Douyin),
                Box::new(extractor::Instagram),
                Box::new(extractor::InstagramIos),
                Box::new(extractor::InstagramStory),
                Box::new(extractor::InstagramTag),
                Box::new(extractor::InstagramUser),
                Box::new(extractor::Facebook),
                Box::new(extractor::FacebookReel),
                Box::new(extractor::FacebookPluginsVideo),
                Box::new(extractor::FacebookRedirectUrl),
                Box::new(extractor::FacebookAds),
                Box::new(extractor::Vimeo),
                Box::new(extractor::Dailymotion),
                Box::new(extractor::DailymotionPlaylist),
                Box::new(extractor::DailymotionSearch),
                Box::new(extractor::DailymotionUser),
                Box::new(extractor::Soundcloud),
                Box::new(extractor::Soundgasm),
                Box::new(extractor::SoundgasmProfile),
                Box::new(extractor::TwitchVod),
                Box::new(extractor::TwitchClips),
                Box::new(extractor::TwitchCollection),
                Box::new(extractor::TwitchVideosClips),
                Box::new(extractor::TwitchVideosCollections),
                Box::new(extractor::TwitchVideos),
                Box::new(extractor::TwitchStream),
                Box::new(extractor::Bandcamp),
                Box::new(extractor::RumbleEmbed),
                Box::new(extractor::Rumble),
                Box::new(extractor::KickVod),
                Box::new(extractor::KickClip),
                Box::new(extractor::KickVideos),
                Box::new(extractor::KickClips),
                Box::new(extractor::KickLive),
                Box::new(extractor::Bluesky),
                Box::new(extractor::Mixcloud),
                Box::new(extractor::Mixlr),
                Box::new(extractor::MixlrRecording),
                Box::new(extractor::Pinterest),
                Box::new(extractor::Lbry),
                Box::new(extractor::LbryChannel),
                Box::new(extractor::LbryPlaylist),
                Box::new(extractor::TrtCocukVideo),
                Box::new(extractor::Freesound),
                Box::new(extractor::HearThisAt),
                Box::new(extractor::Libsyn),
                Box::new(extractor::LrtRadio),
                Box::new(extractor::LrtStream),
                Box::new(extractor::LrtVod),
                Box::new(extractor::ACast),
                Box::new(extractor::ACastChannel),
                Box::new(extractor::Spreaker),
                Box::new(extractor::SpreakerShow),
                Box::new(extractor::IHeartRadio),
                Box::new(extractor::IHeartRadioPodcast),
                Box::new(extractor::Jamendo),
                Box::new(extractor::JamendoAlbum),
                Box::new(extractor::Audiomack),
                Box::new(extractor::AudiomackAlbum),
                Box::new(extractor::Gab),
                Box::new(extractor::Internazionale),
                Box::new(extractor::MatchiTv),
                Box::new(extractor::Pinkbike),
                Box::new(extractor::Nrk),
                Box::new(extractor::NrkTv),
                Box::new(extractor::NrkPlaylist),
                Box::new(extractor::NrkRadioPodkast),
                Box::new(extractor::NrkTvDirekte),
                Box::new(extractor::NrkTvEpisode),
                Box::new(extractor::NrkTvSeason),
                Box::new(extractor::NrkTvSeries),
                Box::new(extractor::NtsLive),
                Box::new(extractor::Netzkino),
                Box::new(extractor::PearVideo),
                Box::new(extractor::PodbayFmChannel),
                Box::new(extractor::PodbayFm),
                Box::new(extractor::RinseFmArtistPlaylist),
                Box::new(extractor::RinseFm),
                Box::new(extractor::BongaCams),
                Box::new(extractor::Democracynow),
                Box::new(extractor::DLiveStream),
                Box::new(extractor::DLiveVod),
                Box::new(extractor::Mux),
                Box::new(extractor::NineGag),
                Box::new(extractor::NineNews),
                Box::new(extractor::RottenTomatoes),
                Box::new(extractor::Mediaite),
                Box::new(extractor::OneNewsNz),
                Box::new(extractor::NzOnScreen),
                Box::new(extractor::Nzz),
                Box::new(extractor::CloudflareStream),
                Box::new(extractor::CloudyCdn),
                Box::new(extractor::Dumpert),
                Box::new(extractor::Steam),
                Box::new(extractor::SteamCommunity),
                Box::new(extractor::SteamCommunityBroadcast),
                Box::new(extractor::Tv2),
                Box::new(extractor::Tv2Article),
                Box::new(extractor::Tv4),
                Box::new(extractor::Tv2Dk),
                Box::new(extractor::Tv2DkBornholmPlay),
                Box::new(extractor::Tv5MondePlus),
                Box::new(extractor::Ruv),
                Box::new(extractor::RuvSpila),
                Box::new(extractor::Rte),
                Box::new(extractor::RteRadio),
                Box::new(extractor::XVideosQuickies),
                Box::new(extractor::XVideos),
                Box::new(extractor::Xnxx),
                Box::new(extractor::Imdb),
                Box::new(extractor::ImdbList),
                Box::new(extractor::Flickr),
                Box::new(extractor::Telegraaf),
                Box::new(extractor::TvNoe),
                Box::new(extractor::TviPlayer),
                Box::new(extractor::ThreeSpeak),
                Box::new(extractor::ThreeSpeakUser),
                Box::new(extractor::Ustudio),
                Box::new(extractor::UstudioEmbed),
                Box::new(extractor::Arnes),
                Box::new(extractor::VideoPress),
                Box::new(extractor::Vidyard),
                Box::new(extractor::S4c),
                Box::new(extractor::S4cSeries),
                Box::new(extractor::Sapo),
                Box::new(extractor::Sbs),
                Box::new(extractor::Clyp),
                Box::new(extractor::Elonet),
                Box::new(extractor::Dropbox),
                Box::new(extractor::BostonGlobe),
                Box::new(extractor::BeaconTv),
                Box::new(extractor::Beeg),
                Box::new(extractor::Clipchamp),
                Box::new(extractor::Ccc),
                Box::new(extractor::DaystarClip),
                Box::new(extractor::Newsy),
                Box::new(extractor::NhkVod),
                Box::new(extractor::NhkVodProgram),
                Box::new(extractor::TrtWorld),
                Box::new(extractor::Tumblr),
                Box::new(extractor::Iltalehti),
                Box::new(extractor::IlPost),
                Box::new(extractor::ImgurAlbum),
                Box::new(extractor::ImgurGallery),
                Box::new(extractor::Imgur),
                Box::new(extractor::BiliBili),
                Box::new(extractor::Vk),
                Box::new(extractor::VkPlay),
                Box::new(extractor::VkPlayLive),
                Box::new(extractor::VkUserVideos),
                Box::new(extractor::VkWallPost),
                Box::new(extractor::NiconicoHistory),
                Box::new(extractor::Niconico),
                Box::new(extractor::NiconicoPlaylist),
                Box::new(extractor::NiconicoSeries),
                Box::new(extractor::NiconicoUser),
                Box::new(extractor::NicovideoSearchDate),
                Box::new(extractor::NicovideoSearch),
                Box::new(extractor::NicovideoSearchUrl),
                Box::new(extractor::NicovideoTagUrl),
                Box::new(extractor::Weibo),
                Box::new(extractor::WeiboVideo),
                Box::new(extractor::TedEmbed),
                Box::new(extractor::TedPlaylist),
                Box::new(extractor::TedSeries),
                Box::new(extractor::TedTalk),
                Box::new(extractor::RutubeEmbed),
                Box::new(extractor::Rutube),
                Box::new(extractor::LinkedIn),
                Box::new(extractor::ArchiveOrg),
                Box::new(extractor::ArdAudiothek),
                Box::new(extractor::ArdAudiothekPlaylist),
                Box::new(extractor::ArdMediathek),
                Box::new(extractor::ArdMediathekCollection),
                Box::new(extractor::ArteTvCategory),
                Box::new(extractor::ArteTvEmbed),
                Box::new(extractor::ArteTv),
                Box::new(extractor::ArteTvPlaylist),
                Box::new(extractor::BitChute),
                Box::new(extractor::BitChuteChannel),
                Box::new(extractor::CnbcVideo),
                Box::new(extractor::Coub),
                Box::new(extractor::Loom),
                Box::new(extractor::Odnoklassniki),
                Box::new(extractor::YoutubeSearch),
                Box::new(Youtube::with_options(youtube)),
                Box::new(extractor::YoutubeRedirect(
                    extractor::YoutubeRedirectKind::Consent,
                )),
                Box::new(extractor::YoutubeRedirect(
                    extractor::YoutubeRedirectKind::Favourites,
                )),
                Box::new(extractor::YoutubeRedirect(
                    extractor::YoutubeRedirectKind::History,
                )),
                Box::new(extractor::YoutubeRedirect(
                    extractor::YoutubeRedirectKind::LivestreamEmbed,
                )),
                Box::new(extractor::YoutubeMusicSearchUrl),
                Box::new(extractor::YoutubeNotifications),
                Box::new(extractor::YoutubeRedirect(
                    extractor::YoutubeRedirectKind::Playlist,
                )),
                Box::new(extractor::YoutubeRedirect(
                    extractor::YoutubeRedirectKind::Recommended,
                )),
                Box::new(extractor::YoutubeRedirect(
                    extractor::YoutubeRedirectKind::ShortsAudioPivot,
                )),
                Box::new(extractor::YoutubeRedirect(
                    extractor::YoutubeRedirectKind::Subscriptions,
                )),
                Box::new(extractor::YoutubeRedirect(
                    extractor::YoutubeRedirectKind::TruncatedId,
                )),
                Box::new(extractor::YoutubeRedirect(
                    extractor::YoutubeRedirectKind::TruncatedUrl,
                )),
                Box::new(extractor::YoutubeRedirect(
                    extractor::YoutubeRedirectKind::WatchLater,
                )),
                Box::new(extractor::YoutubeRedirect(
                    extractor::YoutubeRedirectKind::YtBe,
                )),
                Box::new(extractor::YoutubeRedirect(
                    extractor::YoutubeRedirectKind::YtUser,
                )),
                // After every YouTube extractor.
                Box::new(extractor::YoutubeUnsupported),
                Box::new(extractor::AmadeusTv),
                Box::new(extractor::Angel),
                Box::new(extractor::ApplePodcasts),
                Box::new(extractor::AudiMedia),
                Box::new(extractor::AtScaleConfEvent),
                Box::new(extractor::AudioBoom),
                Box::new(extractor::Beatport),
                Box::new(extractor::BehindKink),
                Box::new(extractor::Bild),
                Box::new(extractor::Bitmovin),
                Box::new(extractor::Blogger),
                Box::new(extractor::Bloomberg),
                Box::new(extractor::BreitBart),
                Box::new(extractor::BrightcoveNew),
                Box::new(extractor::Bundesliga),
                Box::new(extractor::EttuTv),
                Box::new(extractor::Erocast),
                Box::new(extractor::ErrArhiiv),
                Box::new(extractor::ErrJupiter),
                Box::new(extractor::Caltrans),
                Box::new(extractor::Bigo),
                Box::new(extractor::Cam4),
                Box::new(extractor::Cgtn),
                Box::new(extractor::Cjsw),
                Box::new(extractor::CozyTv),
                Box::new(extractor::CrooksAndLiars),
                Box::new(extractor::Crtvg),
                Box::new(extractor::CultureUnplugged),
                Box::new(extractor::EbaumsWorld),
                Box::new(extractor::Ebay),
                Box::new(extractor::Epoch),
                Box::new(extractor::Eporner),
                Box::new(extractor::Fathom),
                Box::new(extractor::Fczenit),
                Box::new(extractor::FilmArchiv),
                Box::new(extractor::Filmweb),
                Box::new(extractor::Funker530),
                Box::new(extractor::FuyinTv),
                Box::new(extractor::GameSpot),
                Box::new(extractor::GameStar),
                Box::new(extractor::Glide),
                Box::new(extractor::GodResource),
                Box::new(extractor::GodTube),
                Box::new(extractor::Golem),
                Box::new(extractor::GoodGame),
                Box::new(extractor::GoogleDriveFolder),
                Box::new(extractor::GoogleDrive),
                Box::new(extractor::GoToStage),
                Box::new(extractor::Graspop),
                Box::new(extractor::Gronkh),
                Box::new(extractor::Harpodeon),
                Box::new(extractor::HellPorno),
                Box::new(extractor::HistoricFilms),
                Box::new(extractor::HitRecord),
                Box::new(extractor::HollywoodReporter),
                Box::new(extractor::HollywoodReporterPlaylist),
                Box::new(extractor::HrefLiRedirect),
                Box::new(extractor::HseProduct),
                Box::new(extractor::HseShow),
                Box::new(extractor::Huajiao),
                Box::new(extractor::Hypem),
                Box::new(extractor::Ina),
                Box::new(extractor::Inc),
                Box::new(extractor::IsraelNationalNews),
                Box::new(extractor::Ivoox),
                Box::new(extractor::Jove),
                Box::new(extractor::JStream),
                Box::new(extractor::JwPlatform),
                Box::new(extractor::Kaltura),
                Box::new(extractor::KankaNews),
                Box::new(extractor::Kicker),
                Box::new(extractor::KickStarter),
                Box::new(extractor::Kommunetv),
                Box::new(extractor::Kth),
                Box::new(extractor::Ku6),
                Box::new(extractor::Livestreamfails),
                Box::new(extractor::Lnk),
                Box::new(extractor::Maariv),
                Box::new(extractor::Masters),
                Box::new(extractor::MedalTv),
                Box::new(extractor::MatchTv),
                Box::new(extractor::Megaphone),
                Box::new(extractor::Meipai),
                Box::new(extractor::Mir24Tv),
                Box::new(extractor::MirrorCoUk),
                Box::new(extractor::MochaVideo),
                Box::new(extractor::MovingImage),
                Box::new(extractor::MySpass),
                Box::new(extractor::MyVidster),
                Box::new(extractor::NobelPrize),
                Box::new(extractor::NosNlArticle),
                Box::new(extractor::Npr),
                Box::new(extractor::OneFootball),
                Box::new(extractor::OnePlacePodcast),
                Box::new(extractor::OrfOn),
                Box::new(extractor::OrfPodcast),
                Box::new(extractor::OrfRadio),
                Box::new(extractor::Parlview),
                Box::new(extractor::PartiLivestream),
                Box::new(extractor::PartiVideo),
                Box::new(extractor::PeerTube),
                Box::new(extractor::PeerTubePlaylist),
                Box::new(extractor::PeerTv),
                Box::new(extractor::Photobucket),
                Box::new(extractor::PlayerFm),
                Box::new(extractor::PopcornTv),
                Box::new(extractor::PremiershipRugby),
                Box::new(extractor::PressTv),
                Box::new(extractor::ProjectVeritas),
                Box::new(extractor::Pyvideo),
                Box::new(extractor::QingTing),
                Box::new(extractor::RadioKapital),
                Box::new(extractor::RadioKapitalShow),
                Box::new(extractor::RadioZetPodcast),
                Box::new(extractor::ReverbNation),
                Box::new(extractor::RoyaLive),
                Box::new(extractor::Rtrfm),
                Box::new(extractor::RtveALaCarta),
                Box::new(extractor::RtveAudio),
                Box::new(extractor::RtveLive),
                Box::new(extractor::RtveTelevision),
                Box::new(extractor::ScreenRec),
                Box::new(extractor::Sen),
                Box::new(extractor::Simplecast),
                Box::new(extractor::SimplecastEpisode),
                Box::new(extractor::SimplecastPodcast),
                Box::new(extractor::SkylineWebcams),
                Box::new(extractor::SnapchatSpotlight),
                Box::new(extractor::Spiegel),
                Box::new(extractor::SrgSsr),
                Box::new(extractor::SrgSsrPlay),
                Box::new(extractor::Streamable),
                Box::new(extractor::StreetVoice),
                Box::new(extractor::Substack),
                Box::new(extractor::SverigesRadioEpisode),
                Box::new(extractor::SverigesRadioPublication),
                Box::new(extractor::SvtPage),
                Box::new(extractor::SvtPlay),
                Box::new(extractor::SvtSeries),
                Box::new(extractor::SztvHu),
                Box::new(extractor::TelegramEmbed),
                Box::new(extractor::TeleTask),
                Box::new(extractor::TheIntercept),
                Box::new(extractor::TheStar),
                Box::new(extractor::Formula1),
                Box::new(extractor::FranceTv),
                Box::new(extractor::FranceTvInfo),
                Box::new(extractor::FranceTvSite),
                Box::new(extractor::MangomoloVideo),
                Box::new(extractor::MangomoloLive),
                Box::new(extractor::MaoriTV),
                Box::new(extractor::EuropeanTour),
                Box::new(extractor::EuScreen),
                Box::new(extractor::CharlieRose),
                Box::new(extractor::SkyNewsAU),
                Box::new(extractor::TvaNouvelles),
                Box::new(extractor::TvaNouvellesArticle),
                Box::new(extractor::Wimbledon),
                Box::new(extractor::Screen9),
                Box::new(extractor::TheHighWire),
                Box::new(extractor::TheGuardianPodcast),
                Box::new(extractor::TheSun),
                Box::new(extractor::ThisAmericanLife),
                Box::new(extractor::Toypics),
                Box::new(extractor::Dfb),
                Box::new(extractor::DrTalks),
                Box::new(extractor::Drtv),
                Box::new(extractor::DrtvLive),
                Box::new(extractor::DrtvSeason),
                Box::new(extractor::DrtvSeries),
                Box::new(extractor::Tweakers),
                Box::new(extractor::TwentyThreeVideo),
                Box::new(extractor::UmgDe),
                Box::new(extractor::Unistra),
                Box::new(extractor::Veo),
                Box::new(extractor::UnitedNationsWebTv),
                Box::new(extractor::Uol),
                Box::new(extractor::Vocaroo),
                Box::new(extractor::Webcamerapl),
                Box::new(extractor::Whyp),
                Box::new(extractor::Wikimedia),
                Box::new(extractor::WistiaChannel),
                Box::new(extractor::Wistia),
                Box::new(extractor::WistiaPlaylist),
                Box::new(extractor::WorldStarHipHop),
                Box::new(extractor::XboxClips),
                Box::new(extractor::XiaoHongShu),
                Box::new(extractor::Xinpianchang),
                Box::new(extractor::Yfanefa),
                Box::new(extractor::YleAreena),
                Box::new(extractor::YouJizz),
                Box::new(extractor::Zdf),
                Box::new(extractor::ZdfChannel),
                Box::new(extractor::ZetlandDkArticle),
                Box::new(extractor::Zhihu),
                Box::new(extractor::Zype),
                Box::new(GenericHttp),
            ],
        })
    }

    /// Return shared scoped cookies for explicit library persistence or reuse.
    pub fn cookie_jar(&self) -> Arc<CookieJar> {
        self.cookies.clone()
    }

    /// Configure an explicit ffmpeg executable for two-stream copy merging.
    /// The path is inspected only for a merged download, never a single stream.
    /// Default selection prefers a video/audio pair when this option is present.
    pub fn with_ffmpeg(mut self, executable: impl Into<PathBuf>) -> Self {
        self.ffmpeg = Some(executable.into());
        self
    }

    /// Upstream `FFmpegFixupM3u8PP`: when `path` holds MPEG-TS, print
    /// `[FixupM3u8] Fixing MPEG-TS in MP4 container of "PATH"` and remux it to
    /// MP4 in place (`aac_adtstoasc` when ffprobe reports AAC audio).
    /// Returns whether a fixup ran; requires a configured ffmpeg.
    pub fn fixup_m3u8(&self, path: &Path) -> Result<bool> {
        self.fixup_m3u8_as(path, path)
    }

    /// [`Self::fixup_m3u8`] naming `shown` (the destination) in its message.
    fn fixup_m3u8_as(&self, path: &Path, shown: &Path) -> Result<bool> {
        if crate::container::sniff(path) != Some("ts") {
            return Ok(false);
        }
        let ffmpeg = self
            .ffmpeg
            .as_ref()
            .ok_or_else(|| Error::Postprocess("fixup requires ffmpeg".into()))?;
        screen::screen(&format!(
            "[FixupM3u8] Fixing MPEG-TS in MP4 container of \"{}\"",
            shown.display()
        ));
        let probe = self.ffprobe.clone().or_else(|| {
            ffmpeg
                .parent()
                .map(|dir| dir.join(format!("ffprobe{}", std::env::consts::EXE_SUFFIX)))
        });
        let aac = probe.is_some_and(|probe| {
            audio::ffmpeg::probe(&probe, path, "ts", &mut || {
                std::ops::ControlFlow::Continue(())
            })
            .is_ok_and(|codec| codec == "aac")
        });
        fixup::remux_to_mp4("FixupM3u8", ffmpeg, path, "ts", aac, &mut || {
            std::ops::ControlFlow::Continue(())
        })?;
        Ok(true)
    }

    /// Upstream `FFmpegFixupM4aPP`: print `[FixupM4a] Correcting container of
    /// "PATH"` and remux a DASH m4a download to a plain MP4 container.
    pub fn fixup_m4a(&self, path: &Path) -> Result<()> {
        self.fixup_m4a_as(path, path)
    }

    fn fixup_m4a_as(&self, path: &Path, shown: &Path) -> Result<()> {
        let ffmpeg = self
            .ffmpeg
            .as_ref()
            .ok_or_else(|| Error::Postprocess("fixup requires ffmpeg".into()))?;
        screen::screen(&format!(
            "[FixupM4a] Correcting container of \"{}\"",
            shown.display()
        ));
        let container = crate::container::sniff(path).unwrap_or("m4a");
        fixup::remux_to_mp4("FixupM4a", ffmpeg, path, container, false, &mut || {
            std::ops::ControlFlow::Continue(())
        })
    }

    /// Upstream `FFmpegFixupStretchedPP` (`_restrict_to(images=False,
    /// audio=False)`): `[FixupStretched] Fixing aspect ratio of "PATH"`, a
    /// stream copy with `-aspect RATIO`.
    fn fixup_stretched_as(
        &self,
        info: &VideoInfo,
        ratio: f64,
        path: &Path,
        shown: &Path,
    ) -> Result<()> {
        let ffmpeg = self
            .ffmpeg
            .as_ref()
            .ok_or_else(|| Error::Postprocess("fixup requires ffmpeg".into()))?;
        let none = |codec: &Option<String>| codec.as_deref() == Some("none");
        if none(&info.vcodec) {
            let kind = if none(&info.acodec) {
                "images"
            } else {
                "audio"
            };
            screen::screen(&format!("[FixupStretched] Skipping {kind}"));
            return Ok(());
        }
        screen::screen(&format!(
            "[FixupStretched] Fixing aspect ratio of \"{}\"",
            shown.display()
        ));
        let container = crate::container::sniff(path).unwrap_or(info.ext.as_str());
        fixup::set_aspect(
            "FixupStretched",
            ffmpeg,
            path,
            container,
            &info.ext,
            ratio,
            &mut || std::ops::ControlFlow::Continue(()),
        )
    }

    /// Upstream `FFmpegFixupDuplicateMoovPP`: `[FixupDuplicateMoov] Fixing
    /// duplicate MOOV atoms of "PATH"`, a stream copy in place.
    fn fixup_duplicate_moov_as(&self, info: &VideoInfo, path: &Path, shown: &Path) -> Result<()> {
        let ffmpeg = self
            .ffmpeg
            .as_ref()
            .ok_or_else(|| Error::Postprocess("fixup requires ffmpeg".into()))?;
        screen::screen(&format!(
            "[FixupDuplicateMoov] Fixing duplicate MOOV atoms of \"{}\"",
            shown.display()
        ));
        let container = crate::container::sniff(path).unwrap_or(info.ext.as_str());
        fixup::copy_stream(
            "FixupDuplicateMoov",
            ffmpeg,
            path,
            container,
            &info.ext.to_lowercase(),
            &mut || std::ops::ControlFlow::Continue(()),
        )
    }

    /// Upstream `HlsFD.real_download`: `[hlsnative] Downloading m3u8
    /// manifest` refetches the media playlist, whose fragments are then
    /// downloaded. `None` for other downloads.
    pub fn refresh_hls(&self, info: &VideoInfo) -> Result<Option<VideoInfo>> {
        // Formats loaded from info JSON have no fragments yet, like
        // upstream's, which HlsFD always reads from the playlist.
        if !info.protocol.as_deref().unwrap_or("").starts_with("m3u8") || self.uses_ffmpeg_fd(info)
        {
            return Ok(None);
        }
        screen::screen("[hlsnative] Downloading m3u8 manifest");
        let referer = (!info.is_direct()).then_some(&info.webpage_url);
        let response = crate::extractor::direct::probe(&self.client, &info.url, referer)?;
        let (_, playlist) = crate::extractor::hls::read_playlist(response)?;
        let hls::Playlist::Media {
            fragments,
            discontinuity,
            ..
        } = playlist
        else {
            return Err(Error::UnsupportedManifest(
                "the refetched HLS manifest is not a media playlist".into(),
            ));
        };
        // HlsFD keeps only the format's discontinuity section.
        let index = info
            .extra
            .get("format_index")
            .and_then(serde_json::Value::as_u64)
            .and_then(|index| u32::try_from(index).ok());
        let mut refreshed = info.clone();
        // HlsFD's `Total fragments` counts every media URI of the playlist.
        FRAGMENT_TOTAL
            .with(|total| total.replace(index.map(|_| (info.url.clone(), fragments.len()))));
        refreshed.fragments = hls::section(fragments, &discontinuity, index);
        Ok(Some(refreshed))
    }

    /// Upstream `YoutubeDL.fixup()` for an unmerged download at `path`
    /// (shown as `shown`): `FixupM4a` for `m4a_dash` containers and
    /// `FixupM3u8` for native HLS MP4 downloads, under `--fixup` policies.
    pub fn fixup(
        &self,
        info: &VideoInfo,
        path: &Path,
        shown: &Path,
        options: &FixupOptions,
    ) -> Result<()> {
        let policy = options.policy.as_deref().unwrap_or("detect_or_warn");
        // FFmpeg wrote a growing file, which may be in use.
        if matches!(policy, "never" | "ignore") || self.grows(info) {
            return Ok(());
        }
        // `FFmpegFixupStretchedPP` comes first and also follows merges and
        // FFmpegFD downloads.
        let stretched = info
            .extra
            .get("stretched_ratio")
            .filter(|ratio| ratio.as_f64().is_some_and(|ratio| ratio != 1.0));
        if let Some(ratio) = stretched {
            let message = format!("Non-uniform pixel ratio {ratio}");
            if policy == "warn" {
                screen::warning(&format!("{}: {message}", info.id));
            } else if self.ffmpeg.is_none() {
                screen::warning(&format!(
                    "{}: {message}. Install ffmpeg to fix this automatically",
                    info.id
                ));
            } else {
                screen::postprocess("FixupStretched", || {
                    self.fixup_stretched_as(info, ratio.as_f64().unwrap_or(1.0), path, shown)
                })?;
            }
        }
        let protocol = info.protocol.as_deref().unwrap_or("");
        // `(is_live and hls_use_mpegts is None)` also follows FFmpegFD.
        let live_m3u8 = is_live(info) && options.hls_use_mpegts.is_none();
        // Other fixups skip FFmpegFD downloads and merges.
        let ffmpeg_fd = self.uses_ffmpeg_fd(info);
        if !info.requested_formats.is_empty() || options.converted || (ffmpeg_fd && !live_m3u8) {
            return self.fixup_websocket(info, path, shown, policy);
        }
        let m3u8 = live_m3u8
            || (!ffmpeg_fd
                && protocol.starts_with("m3u8")
                && !options.hls_use_mpegts.unwrap_or(false));
        let m4a = info.ext == "m4a" && info.container.as_deref() == Some("m4a_dash");
        // Every MPD format is `is_dash_periods` upstream.
        // (YouTube's post-live formats are not `is_dash_periods`.)
        let dash = (protocol == "http_dash_segments"
            && !info.fragments.is_empty()
            && !info.extra.contains_key("target_duration"))
            || (protocol == "http_dash_segments_generator" && is_live(info));
        for (needed, message, fixup) in [
            (
                m4a,
                "writing DASH m4a. Only some players support this container",
                "m4a",
            ),
            (
                m3u8,
                "Possible MPEG-TS in MP4 container or malformed AAC timestamps",
                "m3u8",
            ),
            (dash, "Possible duplicate MOOV atoms", "moov"),
        ] {
            if !needed {
                continue;
            }
            if policy == "warn" {
                screen::warning(&format!("{}: {message}", info.id));
            } else if self.ffmpeg.is_none() {
                screen::warning(&format!(
                    "{}: {message}. Install ffmpeg to fix this automatically",
                    info.id
                ));
            } else if fixup == "m3u8" {
                screen::postprocess("FixupM3u8", || {
                    if matches!(info.ext.as_str(), "mp4" | "m4a") {
                        self.fixup_m3u8_as(path, shown)?;
                    }
                    Ok(())
                })?;
            } else if fixup == "moov" {
                screen::postprocess("FixupDuplicateMoov", || {
                    self.fixup_duplicate_moov_as(info, path, shown)
                })?;
            } else {
                screen::postprocess("FixupM4a", || self.fixup_m4a_as(path, shown))?;
            }
        }
        self.fixup_websocket(info, path, shown, policy)
    }

    /// The last fixups, after any conversion too: `FFmpegFixupTimestampPP`
    /// and `FFmpegFixupDurationPP` for WebSocketFragmentFD downloads.
    fn fixup_websocket(
        &self,
        info: &VideoInfo,
        path: &Path,
        shown: &Path,
        policy: &str,
    ) -> Result<()> {
        if !info.requested_formats.is_empty() || info.protocol.as_deref() != Some("websocket_frag")
        {
            return Ok(());
        }
        let setts = std::cell::OnceCell::new();
        for (message, pp) in [
            ("Malformed timestamps detected", "FixupTimestamp"),
            ("Malformed duration detected", "FixupDuration"),
        ] {
            let Some(ffmpeg) = self.ffmpeg.as_deref().filter(|_| policy != "warn") else {
                let hint = if policy == "warn" {
                    ""
                } else {
                    ". Install ffmpeg to fix this automatically"
                };
                screen::warning(&format!("{}: {message}{hint}", info.id));
                continue;
            };
            screen::postprocess(pp, || {
                // `_restrict_to(images=False)`.
                if info.vcodec.as_deref() == Some("none") && info.acodec.as_deref() == Some("none")
                {
                    screen::screen(&format!("[{pp}] Skipping images"));
                    return Ok(());
                }
                let container = crate::container::sniff(path).unwrap_or(info.ext.as_str());
                let ext = info.ext.to_lowercase();
                let mut proceed = || std::ops::ControlFlow::Continue(());
                if pp == "FixupTimestamp" {
                    let setts = *setts.get_or_init(|| fixup::has_setts(ffmpeg));
                    if !setts {
                        screen::warning(
                            "A re-encode is needed to fix timestamps in older versions of ffmpeg. Please install ffmpeg 4.4 or later to fixup without re-encoding",
                        );
                    }
                    screen::screen(&format!(
                        "[{pp}] Fixing frame timestamp of \"{}\"",
                        shown.display()
                    ));
                    fixup::fix_timestamps(ffmpeg, path, container, &ext, setts, &mut proceed)
                } else {
                    screen::screen(&format!(
                        "[{pp}] Fixing video duration of \"{}\"",
                        shown.display()
                    ));
                    fixup::copy_stream(pp, ffmpeg, path, container, &ext, &mut proceed)
                }
            })?;
        }
        Ok(())
    }

    /// Upstream `FFmpegSplitChaptersPP`: cut `input` into one file per
    /// chapter (`destinations` pairs each chapter with its prepared path).
    /// With `force_keyframes` (`--force-keyframes-at-cuts`), a re-encoded copy
    /// with keyframes at the chapter starts is split instead.
    pub fn split_chapters(
        &self,
        input: &Path,
        chapters: &[(Chapter, PathBuf)],
        force_keyframes: bool,
    ) -> Result<()> {
        let ffmpeg = self.ffmpeg.as_ref().ok_or_else(|| {
            Error::Postprocess(
                "ffmpeg not found. Please install or provide the path using --ffmpeg-location"
                    .into(),
            )
        })?;
        // Upstream deletes the keyframes copy afterwards, without a message.
        let mut workspace = None;
        let mut input = input.to_path_buf();
        if force_keyframes && chapters.len() > 1 {
            screen::screen(&format!(
                "[SplitChapters] Re-encoding \"{}\" with appropriate keyframes",
                input.display()
            ));
            let parent = input
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let directory = tempfile::Builder::new()
                .prefix(".yt-dlp-rs-keyframes-")
                .tempdir_in(parent)?;
            let ext = input
                .extension()
                .map(|ext| ext.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            let container = crate::container::sniff(&input).unwrap_or(&ext).to_owned();
            let keyframes = directory.path().join(format!("keyframes.{ext}"));
            let starts: Vec<PyNumber> = chapters
                .iter()
                .map(|(chapter, _)| chapter.start_time.unwrap_or(PyNumber::Int(0)))
                .collect();
            modify_chapters::force_keyframes(
                ffmpeg,
                "SplitChapters",
                &input,
                &container,
                &ext,
                &keyframes,
                &starts,
            )?;
            input = keyframes;
            workspace = Some(directory);
        }
        let input = input.as_path();
        screen::screen(&format!(
            "[SplitChapters] Splitting video by chapters; {} chapters found",
            chapters.len()
        ));
        let py_str = |value: PyNumber| match value {
            PyNumber::Int(value) => value.to_string(),
            PyNumber::Float(value) => screen::py_float(value),
        };
        for (index, (chapter, destination)) in chapters.iter().enumerate() {
            let (Some(start), Some(end)) = (chapter.start_time, chapter.end_time) else {
                return Err(Error::Postprocess(
                    "chapter end times need the video duration".into(),
                ));
            };
            let duration = match (end, start) {
                (PyNumber::Int(end), PyNumber::Int(start)) => PyNumber::Int(end - start),
                (end, start) => PyNumber::Float(end.as_f64() - start.as_f64()),
            };
            screen::screen(&format!(
                "[SplitChapters] Chapter {:03}; Destination: {}",
                index + 1,
                destination.display()
            ));
            let ext = destination
                .extension()
                .map(|ext| ext.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            fixup::cut(
                ffmpeg,
                input,
                destination,
                &py_str(start),
                &py_str(duration),
                embed_metadata::muxer(&ext),
                &mut || std::ops::ControlFlow::Continue(()),
            )?;
        }
        drop(workspace);
        Ok(())
    }

    /// Upstream `SponsorBlockPP.run` (an `after_filter` processor): fetch the
    /// video's segments in `categories` from `api` and store them as
    /// `sponsorblock_chapters`. `retries` follows `--extractor-retries`.
    pub fn fetch_sponsorblock(
        &self,
        info: &mut VideoInfo,
        categories: &[String],
        api: &str,
        retries: u32,
    ) -> Result<()> {
        let say = |text: &str| screen::screen(&format!("[SponsorBlock] {text}"));
        if !sponsorblock::supported(info) {
            say(&format!(
                "SponsorBlock is not supported for {}",
                info.extractor_key
            ));
            return Ok(());
        }
        say("Fetching SponsorBlock segments");
        let url = sponsorblock::query_url(api, &info.id, categories);
        screen::debug(&format!("SponsorBlock query: {url}"));
        // `_download_json` with `RetryManager`: 404 means no segments.
        let mut attempt = 0;
        let body: Option<serde_json::Value> = loop {
            attempt += 1;
            let result = crate::control::scoped(self.client.get(&url))
                .and_then(|request| request.send())
                .inspect(|_| crate::screen::after_request())
                .map_err(|error| retry::urllib_error(&error))
                .and_then(|response| {
                    let status = response.status();
                    if status == reqwest::StatusCode::NOT_FOUND {
                        return Ok(None);
                    }
                    if !status.is_success() {
                        return Err(format!(
                            "HTTP Error {}: {}",
                            status.as_u16(),
                            status.canonical_reason().unwrap_or("")
                        ));
                    }
                    let bytes = response.bytes().map_err(|error| error.to_string())?;
                    serde_json::from_slice::<serde_json::Value>(&bytes)
                        .map(Some)
                        .map_err(|error| error.to_string())
                });
            match result {
                Ok(body) => break body,
                Err(error) => {
                    let error = format!("Unable to communicate with SponsorBlock API: {error}");
                    if attempt > retries {
                        return Err(Error::Postprocess(error));
                    }
                    screen::warning(&format!("{error}. Retrying ({attempt}/{retries})..."));
                }
            }
        };
        let segments: Vec<serde_json::Value> = body
            .as_ref()
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .find(|entry| entry["videoID"].as_str() == Some(info.id.as_str()))
            .and_then(|entry| entry["segments"].as_array().cloned())
            .unwrap_or_default();
        let (chapters, mismatched) = sponsorblock::chapters(&segments, info.metadata.duration)?;
        if mismatched {
            screen::warning(
                "Some SponsorBlock segments are from a video of different duration, maybe from an old version of this video",
            );
        }
        if chapters.is_empty() {
            say("No matching segments were found in the SponsorBlock database");
        } else {
            say(&format!(
                "Found {} segments in the SponsorBlock database",
                chapters.len()
            ));
        }
        info.extra.insert(
            "sponsorblock_chapters".into(),
            chapters
                .iter()
                .map(sponsorblock::SponsorChapter::to_json)
                .collect(),
        );
        Ok(())
    }

    /// Upstream `FFmpegConcatPP.concat_files` for `--concat-playlist`:
    /// several files with identical streams are concatenated into `output`.
    pub fn concat_files(&self, inputs: &[PathBuf], output: &Path) -> Result<()> {
        let ffmpeg = self.ffmpeg.as_ref().ok_or_else(|| {
            Error::Postprocess(
                "ffmpeg not found. Please install or provide the path using --ffmpeg-location"
                    .into(),
            )
        })?;
        let ffprobe = self
            .ffprobe
            .clone()
            .unwrap_or_else(|| ffmpeg.with_file_name("ffprobe"));
        modify_chapters::concat_files(ffmpeg, &ffprobe, inputs, output)
    }

    /// Whether an ffmpeg executable is configured (upstream `merger.available`).
    /// Upstream `dl`'s `--verbose` line naming the downloader, for callers
    /// that announce a download before starting it; the next
    /// `download_with_progress` of the same info does not repeat it.
    pub fn announce_download(&self, info: &VideoInfo) {
        self.report_invoking(info);
        ANNOUNCED.with(|announced| announced.replace(Some(info.url.clone())));
    }

    fn report_invoking(&self, info: &VideoInfo) {
        let ffmpeg_fd = self.uses_ffmpeg_fd(info);
        // DashSegmentsFD downloads generated formats together.
        let generated = |protocol: Option<&str>| protocol == Some("http_dash_segments_generator");
        let live = if info.requested_formats.is_empty() {
            generated(info.protocol.as_deref())
        } else {
            info.requested_formats
                .iter()
                .all(|format| generated(Some(&format.determine_protocol())))
        };
        if !(ffmpeg_fd || live || info.requested_formats.is_empty()) {
            return;
        }
        let external = self.external_downloader(info, Path::new(""));
        let name = if ffmpeg_fd {
            "ffmpeg"
        } else if live {
            "dashsegments"
        } else if let Some(external) = &external {
            external.basename()
        } else if matches!(info.protocol.as_deref(), Some("rtmp" | "rtmpe")) {
            "rtmp"
        } else if info.protocol.as_deref() == Some("websocket_frag") {
            "web_socket_fragment"
        } else if info.protocol.as_deref() == Some("f4m") {
            "f4m"
        } else if info.protocol.as_deref() == Some("ism") {
            "ism"
        } else if info.protocol.as_deref() == Some("mhtml") {
            "mhtml"
        } else if info
            .protocol
            .as_deref()
            .is_some_and(|p| p.starts_with("m3u8"))
        {
            "hlsnative"
        } else if info.fragments.is_empty() {
            "http"
        } else if info.protocol.as_deref() == Some("http_dash_segments") {
            "dashsegments"
        } else {
            "hlsnative"
        };
        let urls: Vec<&Url> = if info.requested_formats.is_empty() {
            vec![&info.url]
        } else {
            info.requested_formats
                .iter()
                .map(|format| &format.url)
                .collect()
        };
        report_invoking(name, &urls);
    }

    /// The configured ffmpeg executable.
    pub fn ffmpeg_path(&self) -> Option<&Path> {
        self.ffmpeg.as_deref()
    }

    pub fn has_ffmpeg(&self) -> bool {
        self.ffmpeg.is_some()
    }

    /// Configure an explicit ffprobe executable for audio codec inspection.
    /// Without this override, audio processing uses ffprobe beside explicit ffmpeg.
    pub fn with_ffprobe(mut self, executable: impl Into<PathBuf>) -> Self {
        self.ffprobe = Some(executable.into());
        self
    }

    /// `--section-timecode` (port extension): give partial downloads the
    /// timecode their first frame has in the whole video. `video` is the
    /// `--remux-video`/`--recode-video` mapping applied afterwards; only
    /// MP4/MOV results carry the timecode, others are warned about.
    pub fn with_section_timecode(mut self, video: Option<VideoOptions>) -> Self {
        self.section_timecode = Some(video);
        self
    }

    /// The timecode settings for a download of `info`.
    fn timecode_request(&self, info: &VideoInfo) -> Option<section_timecode::Request> {
        let video = self.section_timecode.as_ref()?;
        let final_ext = video
            .as_ref()
            .and_then(|video| video.plan(&info.ext).ok())
            .filter(|plan| !plan.skip)
            .map_or_else(|| info.ext.clone(), |plan| plan.extension);
        let ffprobe = self.ffprobe.clone().or_else(|| {
            self.ffmpeg
                .as_ref()?
                .parent()
                .map(|dir| dir.join(format!("ffprobe{}", std::env::consts::EXE_SUFFIX)))
        });
        Some(section_timecode::Request { ffprobe, final_ext })
    }

    /// Upstream's `prefer_best` default (`best/bestvideo+bestaudio`, no
    /// ffmpeg warning), used for `-o -` output.
    /// Upstream `quiet` for FFmpeg downloads (`-loglevel quiet`).
    pub fn with_external_quiet(mut self, quiet: bool) -> Self {
        self.ffmpeg_settings.quiet = quiet;
        self
    }

    /// Return playlists with their extractor's pending entries (upstream's
    /// lazily extracted entries) for [`YoutubeDl::fetch_pending`]; by default
    /// extraction fetches every entry.
    pub fn with_deferred_entries(mut self, defer: bool) -> Self {
        self.defer_entries = defer;
        self
    }

    /// Upstream `post_extract`: what the extractor left for after the match
    /// filters (YouTube's comments with `getcomments`).
    pub fn post_extract(&self, info: &mut VideoInfo) -> Result<()> {
        extractor::youtube_comments(&self.client, info)?;
        extractor::pialive_comments(&self.client, info)?;
        extractor::instagram_comments(&self.client, info)
    }

    /// Upstream `DashSegmentsFD` over `http_dash_segments_generator`
    /// formats (`[dashsegments] Total fragments: unknown (live)` for each),
    /// downloaded in parallel; each pair is a format's result and file.
    fn download_live_streams(
        &self,
        info: &VideoInfo,
        streams: &[(VideoInfo, PathBuf)],
        options: DownloadOptions,
        merged: bool,
        callback: &mut dyn FnMut(DownloadProgress) -> std::ops::ControlFlow<()>,
    ) -> Result<Vec<u64>> {
        let jobs = self.live_jobs(info, streams, merged, false)?;
        live_dash::download(&self.client, jobs, options, merged, callback)
    }

    /// The DashSegmentsFD jobs for `streams`; `pipe` jobs feed named pipes.
    fn live_jobs(
        &self,
        info: &VideoInfo,
        streams: &[(VideoInfo, PathBuf)],
        merged: bool,
        pipe: bool,
    ) -> Result<Vec<live_dash::Job>> {
        let feed = extractor::LiveFeed::new(&self.client, info, &self.youtube_runtime).ok_or_else(
            || {
                Error::Reported(
                    "the stream needs to be re-extracted to download it from the start".into(),
                )
            },
        )?;
        let mut jobs = Vec::new();
        for (stream, path) in streams {
            screen::screen("[dashsegments] Total fragments: unknown (live)");
            if merged {
                screen::screen(&format!("[download] Destination: {}", path.display()));
            }
            jobs.push(live_dash::Job {
                fragments: extractor::LiveFragments::new(feed.clone(), &stream.extra, &stream.url),
                info: stream.clone(),
                destination: path.clone(),
                pipe,
            });
        }
        Ok(jobs)
    }

    /// `--keep-partial-on-stop` (port extension): SIGINT or SIGTERM during a
    /// download stops it like [`Interrupt::stop`]: the data so far is
    /// published at the destination (merged streams merged) and the download
    /// returns `Error::DownloadStopped`. Growing-file recordings keep their
    /// own stop handling.
    pub fn with_keep_partial_on_stop(mut self, keep: bool) -> Self {
        self.keep_partial_on_stop = keep;
        self
    }

    /// `--growing-file` (port extension): live results are written as one
    /// growing QuickTime file (see `growing.rs`).
    pub fn with_growing_file(mut self, growing: bool) -> Self {
        self.growing_file = growing;
        self
    }

    /// Whether `--growing-file` applies to a selected result: a live stream
    /// that FFmpeg reads itself, or `--live-from-start` streams that the
    /// native downloader feeds it (named pipes, Unix only).
    pub fn grows(&self, info: &VideoInfo) -> bool {
        self.growing_file
            && is_live(info)
            && self.ffmpeg.is_some()
            && ((cfg!(unix) && live_generated(info)) || self.uses_ffmpeg_fd(info))
    }

    /// `--growing-file`'s download: FFmpeg writes the file at `destination`
    /// from the live URLs, or from named pipes the `--live-from-start`
    /// fragment downloads feed.
    fn download_growing(
        &self,
        info: &VideoInfo,
        destination: &Path,
        options: DownloadOptions,
        callback: &mut dyn FnMut(DownloadProgress) -> std::ops::ControlFlow<()>,
    ) -> Result<u64> {
        let ffmpeg = self
            .ffmpeg
            .as_deref()
            .ok_or_else(|| Error::Reported("--growing-file needs ffmpeg".into()))?;
        let network = ffmpeg_fd::Network {
            headers: &self.headers,
            cookies: &self.cookies,
            proxy: self.proxy.as_deref(),
        };
        if !live_generated(info) {
            let growing = ffmpeg_fd::Growing {
                timecode: growing::timecode(growing::now()),
                pipes: false,
            };
            return ffmpeg_fd::download_growing(
                ffmpeg,
                &network,
                &self.ffmpeg_settings,
                info,
                None,
                &growing,
                destination,
                &options,
                callback,
            );
        }
        #[cfg(not(unix))]
        return Err(Error::Reported(
            "--growing-file with --live-from-start needs named pipes".into(),
        ));
        #[cfg(unix)]
        {
            use std::sync::atomic::{AtomicBool, Ordering};
            download::check_destination(destination, &options)?;
            let parent = destination
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            std::fs::create_dir_all(parent)?;
            let workspace = tempfile::Builder::new()
                .prefix(".yt-dlp-rs-growing-")
                .tempdir_in(parent)?;
            let streams: Vec<(VideoInfo, PathBuf)> = if info.requested_formats.is_empty() {
                vec![(info.clone(), workspace.path().join("input-0"))]
            } else {
                info.requested_formats
                    .iter()
                    .enumerate()
                    .map(|(index, format)| {
                        (
                            info.with_selected_format(format),
                            workspace.path().join(format!("input-{index}")),
                        )
                    })
                    .collect()
            };
            for (_, pipe) in &streams {
                growing::fifo::create(pipe)?;
            }
            let jobs = self.live_jobs(info, &streams, false, true)?;
            if !info.requested_formats.is_empty() {
                screen::screen(&format!(
                    "[download] Destination: {}",
                    destination.display()
                ));
            }
            let local: Vec<ffmpeg_fd::LocalInput> = streams
                .iter()
                .map(|(_, pipe)| ffmpeg_fd::LocalInput {
                    path: pipe.clone(),
                    start_time: 0.0,
                })
                .collect();
            // The stream's start, when the extractor knows it.
            let start = info
                .extra
                .get("release_timestamp")
                .and_then(serde_json::Value::as_i64);
            let growing = ffmpeg_fd::Growing {
                timecode: start.map_or_else(|| "00:00:00:00".into(), growing::timecode),
                pipes: true,
            };
            // One handler for both threads' interrupt checks.
            let _interrupt = interrupt::Guard::install_with_terminate();
            let ended = &AtomicBool::new(false);
            let client = &self.client;
            // The feeders' messages are written by this thread, which may
            // hold the output streams.
            let (sender, messages) = std::sync::mpsc::channel::<screen::OwnedMessage>();
            let relay = || messages.try_iter().for_each(|message| message.emit());
            std::thread::scope(|scope| {
                let context = control::current();
                let feeder = scope.spawn(move || {
                    let _context = control::resume(context);
                    screen::forward(Some(Box::new(move |message| {
                        let _ = sender.send(message);
                    })));
                    let fed = live_dash::download(client, jobs, options, false, &mut |_| {
                        if ended.load(Ordering::SeqCst) {
                            std::ops::ControlFlow::Break(())
                        } else {
                            std::ops::ControlFlow::Continue(())
                        }
                    });
                    screen::forward(None);
                    fed
                });
                let result = ffmpeg_fd::download_growing(
                    ffmpeg,
                    &network,
                    &self.ffmpeg_settings,
                    info,
                    Some(&local),
                    &growing,
                    destination,
                    &options,
                    &mut |progress| {
                        relay();
                        callback(progress)
                    },
                );
                // Stop the feeders; one may still wait for FFmpeg to open its pipe.
                ended.store(true, Ordering::SeqCst);
                while !feeder.is_finished() {
                    relay();
                    for (_, pipe) in &streams {
                        growing::fifo::release(pipe);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                let fed = feeder
                    .join()
                    .unwrap_or_else(|_| Err(Error::Reported("a fragment worker panicked".into())));
                relay();
                let size = result?;
                match fed {
                    // FFmpeg may end first (an interrupt reaching it too).
                    Ok(_) | Err(Error::DownloadCancelled) => Ok(size),
                    // What was recorded stays in the file.
                    Err(error) => Err(error),
                }
            })
        }
    }

    /// Evaluate YouTube post-live formats' fragment lists (upstream's
    /// `LazyList`, read by JSON output and the downloaders): one HEAD
    /// request's `X-Head-Seqnum` bounds them.
    pub fn prepare_live_fragments(&self, info: &mut VideoInfo) -> Result<()> {
        extractor::youtube_live_fragments(&self.client, info)
    }

    /// Fetch a playlist's pending entries (and its nested playlists') until
    /// it has `limit` entries, or all of them; entries still pending remain.
    pub fn fetch_pending(&self, playlist: &mut PlaylistInfo, limit: Option<usize>) -> Result<()> {
        if let Some(pending) = playlist.pending.take() {
            let fetch = if pending.state["kind"] == "twitch" {
                extractor::fetch_twitch_pages
            } else if extractor::tiktok_pending_state(&pending.state) {
                extractor::fetch_tiktok_pages
            } else if extractor::zdf_pending_state(&pending.state) {
                extractor::fetch_zdf_pages
            } else if extractor::steam_pending_state(&pending.state) {
                extractor::fetch_steam_trailers
            } else if extractor::orf_pending_state(&pending.state) {
                extractor::fetch_orf_segments
            } else if extractor::nhk_pending_state(&pending.state) {
                extractor::fetch_nhk_episodes
            } else if extractor::monstercat_pending_state(&pending.state) {
                extractor::fetch_monstercat_tracks
            } else if extractor::freetv_pending_state(&pending.state) {
                extractor::fetch_freetv_episodes
            } else if extractor::spreaker_pending_state(&pending.state) {
                extractor::fetch_spreaker_episodes
            } else if extractor::simplecast_pending_state(&pending.state) {
                extractor::fetch_simplecast_episodes
            } else if extractor::ard_pending_state(&pending.state) {
                extractor::fetch_ard_pages
            } else if extractor::vk_user_videos_pending_state(&pending.state) {
                extractor::fetch_vk_user_videos_pages
            } else if extractor::kick_pending_state(&pending.state) {
                extractor::fetch_kick_entries
            } else if extractor::peertube_pending_state(&pending.state) {
                extractor::fetch_peertube_pages
            } else if extractor::podbayfm_pending_state(&pending.state) {
                extractor::fetch_podbayfm_pages
            } else if extractor::yappy_pending_state(&pending.state) {
                extractor::fetch_yappy_pages
            } else if extractor::lefigaro_pending_state(&pending.state) {
                extractor::fetch_lefigaro_pages
            } else if extractor::hollywoodreporter_pending_state(&pending.state) {
                extractor::fetch_hollywoodreporter_pages
            } else if extractor::lbry_pending_state(&pending.state) {
                extractor::fetch_lbry_pages
            } else if extractor::bitchute_pending_state(&pending.state) {
                extractor::fetch_bitchute_pages
            } else if extractor::niconico_pending_state(&pending.state) {
                extractor::fetch_niconico_pages
            } else if extractor::dailymotion_pending_state(&pending.state) {
                extractor::fetch_dailymotion_pages
            } else if extractor::kika_pending_state(&pending.state) {
                extractor::fetch_kika_pages
            } else if extractor::lastfm_pending_state(&pending.state) {
                extractor::fetch_lastfm_pages
            } else if extractor::ciscolive_pending_state(&pending.state) {
                extractor::fetch_ciscolive_pages
            } else if extractor::radiokapital_pending_state(&pending.state) {
                extractor::fetch_radiokapital_pages
            } else if extractor::nzz_pending_state(&pending.state) {
                extractor::fetch_nzz_entries
            } else {
                extractor::fetch_youtube_tab_pages
            };
            // Steam's generator yields extracted videos (not `url` entries),
            // processed as they are fetched.
            // Some generators and paged lists yield extracted videos (not
            // `url` entries), processed once they are fetched (random-access
            // pages may fill earlier placeholders).
            let resolved = extractor::steam_pending_state(&pending.state)
                || extractor::orf_pending_state(&pending.state)
                || extractor::nhk_pending_state(&pending.state)
                || extractor::monstercat_pending_state(&pending.state)
                || extractor::freetv_pending_state(&pending.state)
                || extractor::spreaker_pending_state(&pending.state)
                || extractor::simplecast_pending_state(&pending.state)
                || extractor::podbayfm_pending_state(&pending.state)
                || extractor::nzz_pending_state(&pending.state);
            playlist.pending = fetch(&self.client, pending.state, limit, &mut playlist.entries)?
                .map(|state| PendingEntries { state });
            if resolved {
                let unprocessed: Vec<usize> = (0..playlist.entries.len())
                    .filter(|index| !playlist.entries[*index].extra.contains_key(PROCESSED_ENTRY))
                    .collect();
                if !unprocessed.is_empty() {
                    let mut fetched = ExtractResult::Playlist(PlaylistInfo {
                        entries: unprocessed
                            .iter()
                            .map(|index| playlist.entries[*index].clone())
                            .collect(),
                        nested: Vec::new(),
                        pending: None,
                        extra: serde_json::Map::new(),
                        ..playlist.clone()
                    });
                    self.process_result(&mut fetched)?;
                    if let ExtractResult::Playlist(fetched) = fetched {
                        for (index, mut entry) in unprocessed.into_iter().zip(fetched.entries) {
                            entry
                                .extra
                                .insert(PROCESSED_ENTRY.into(), serde_json::Value::Bool(true));
                            playlist.entries[index] = entry;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn fetch_all_pending(&self, result: &mut ExtractResult) -> Result<()> {
        if let ExtractResult::Playlist(playlist) = result {
            self.fetch_pending(playlist, None)?;
            for nested in &mut playlist.nested {
                self.fetch_pending(nested, None)?;
            }
        }
        Ok(())
    }

    /// `--http-chunk-size`: chunked HTTP downloads (overrides a format's
    /// `downloader_options`).
    pub fn with_http_chunk_size(mut self, size: Option<u64>) -> Self {
        self.http_chunk_size = size;
        self
    }

    /// `--use-extractors NAMES`: upstream names (`html5`, `youtube`,
    /// `generic`, `end`, regexes, `all`, `default`, `-` exclusions) select and
    /// order the extractors.
    pub fn with_allowed_extractors(
        mut self,
        names: &[String],
    ) -> std::result::Result<Self, String> {
        self.allowed_extractors = if names.is_empty() {
            None
        } else {
            Some(ordered_extractors(names, Vec::new())?)
        };
        Ok(self)
    }

    /// Upstream `writesubtitles`/`listsubtitles`: extractors'
    /// `extract_subtitles` (lazy subtitle requests) run only with them.
    pub fn with_subtitles_requested(self, requested: bool) -> Self {
        extractor::SUBTITLES_REQUESTED.store(requested, std::sync::atomic::Ordering::Relaxed);
        self
    }

    /// `--force-generic-extractor`: only the generic extractor is used.
    pub fn with_force_generic_extractor(mut self, force: bool) -> Self {
        self.force_generic = force;
        extractor::html5::FORCE_GENERIC.store(force, std::sync::atomic::Ordering::Relaxed);
        self
    }

    /// `-N`/`--concurrent-fragments`: fragments downloaded in parallel.
    pub fn with_concurrent_fragments(mut self, count: usize) -> Self {
        self.concurrent_fragments = count.max(1);
        self
    }

    /// `--enable-file-urls`: extract and download local `file:` media.
    pub fn with_file_urls(mut self, enable: bool) -> Self {
        self.enable_file_urls = enable;
        self
    }

    /// `--keep-fragments`: keep `TMPFILENAME-FragN` fragment files.
    pub fn with_keep_fragments(mut self, keep: bool) -> Self {
        self.keep_fragments = keep;
        self
    }

    /// `--downloader`: `ffmpeg` (FFmpegFD) or `native` per simplified
    /// protocol key (`http`, `ftp`, `m3u8`, `dash`, `rtmp`, `default`).
    pub fn with_downloader(self, protocol: &str, ffmpeg: bool) -> Self {
        self.with_downloader_name(protocol, if ffmpeg { "ffmpeg" } else { "native" })
    }

    /// `--downloader [PROTO:]NAME` with the given name or path (`native`,
    /// `ffmpeg`, `curl`, `wget`, `aria2c`, `axel`, `httpie`, ...).
    pub fn with_downloader_name(mut self, protocol: &str, name: &str) -> Self {
        self.downloaders
            .insert(protocol.to_owned(), name.to_owned());
        self
    }

    /// Parameters for the external downloaders (`--downloader-args` are
    /// added by [`Self::with_downloader_args`]).
    pub fn with_external_settings(mut self, settings: ExternalSettings) -> Self {
        let args = std::mem::take(&mut self.external_settings.args);
        self.external_settings = ExternalSettings { args, ..settings };
        self
    }

    /// The `--downloader` name for a protocol.
    fn downloader_name(&self, protocol: &str) -> Option<&str> {
        self.downloaders
            .get(simplify_protocol(protocol))
            .or_else(|| self.downloaders.get("default"))
            .map(String::as_str)
    }

    /// Upstream `_get_suitable_downloader`'s external downloader for a
    /// single-format download (`ed.can_download(info_dict, name)`).
    fn external_downloader(
        &self,
        info: &VideoInfo,
        destination: &Path,
    ) -> Option<external_fd::Available> {
        if !info.requested_formats.is_empty() || self.uses_ffmpeg_fd(info) {
            return None;
        }
        let protocol = info
            .protocol
            .clone()
            .unwrap_or_else(|| info.url.scheme().to_owned());
        let name = self.downloader_name(&protocol)?;
        if name.eq_ignore_ascii_case("native") || is_ffmpeg_name(name) {
            return None;
        }
        let to_stdout = destination == Path::new("-");
        external_fd::supports(info, &protocol, to_stdout)
            .then(|| external_fd::available(name))
            .flatten()
    }

    /// `--downloader-args NAME:ARGS` for FFmpegFD (`ffmpeg`, `ffmpeg_o`,
    /// `ffmpeg_oN`, `ffmpeg_i`, `ffmpeg_iN`, `default`).
    pub fn with_downloader_args(mut self, key: &str, args: Vec<String>) -> Self {
        self.ffmpeg_settings
            .args
            .insert(key.to_lowercase(), args.clone());
        self.external_settings.args.insert(key.to_lowercase(), args);
        self
    }

    /// Whether a selected format has the `m3u8` protocol, which upstream
    /// downloads with FFmpegFD unless `--downloader native` is given, or
    /// `rtmp_ffmpeg` (`PROTOCOL_MAP`'s FFmpegFD for any downloader).
    fn forced_ffmpeg_hls(&self, info: &VideoInfo) -> bool {
        let forced = |protocol: &str| {
            protocol == "rtmp_ffmpeg"
                || (protocol == "m3u8"
                    && !self
                        .downloader_name(protocol)
                        .is_some_and(|name| name.eq_ignore_ascii_case("native")))
        };
        if info.requested_formats.is_empty() {
            info.protocol.as_deref().is_some_and(forced)
        } else {
            info.requested_formats
                .iter()
                .any(|format| forced(&format.determine_protocol()))
        }
    }

    /// Upstream `get_suitable_downloader(...) == FFmpegFD`: partial downloads,
    /// or every selected protocol assigned to `ffmpeg` by `--downloader`.
    pub fn uses_ffmpeg_fd(&self, info: &VideoInfo) -> bool {
        // `FFmpegFD.can_download` needs ffmpeg; otherwise the native
        // downloaders are used (a partial download without ffmpeg is an
        // error unless `--compat-options no-direct-merge`).
        if self.ffmpeg.is_none() {
            return false;
        }
        if sections::is_partial(info) {
            return true;
        }
        // `PROTOCOL_MAP['m3u8']`: the non-native HLS protocol is FFmpeg's.
        if self.forced_ffmpeg_hls(info) {
            return true;
        }
        // `_get_suitable_downloader`: live HLS uses FFmpegFD, as does live
        // DASH unless the downloader is `native`.
        let live = |protocol: &str| {
            protocol.starts_with("m3u8")
                || (protocol == "http_dash_segments"
                    && !self
                        .downloader_name(protocol)
                        .is_some_and(|name| name.eq_ignore_ascii_case("native")))
        };
        if is_live(info)
            && (info.protocol.as_deref().is_some_and(live)
                || info
                    .requested_formats
                    .iter()
                    .any(|format| live(&format.determine_protocol())))
        {
            return true;
        }
        let protocols: Vec<String> = if info.requested_formats.is_empty() {
            vec![
                info.protocol
                    .clone()
                    .unwrap_or_else(|| info.url.scheme().to_owned()),
            ]
        } else {
            info.requested_formats
                .iter()
                .map(VideoFormat::determine_protocol)
                .collect()
        };
        // `FFmpegFD.supports`: its `SUPPORTED_PROTOCOLS` only.
        protocols.iter().all(|protocol| {
            ffmpeg_fd::SUPPORTED_PROTOCOLS.contains(&protocol.as_str())
                && self.downloader_name(protocol).is_some_and(is_ffmpeg_name)
        })
    }

    /// Upstream `check_formats`.
    pub fn with_check_formats(mut self, check: CheckFormats) -> Self {
        extractor::CHECK_FORMATS_NEVER.store(
            check == CheckFormats::Never,
            std::sync::atomic::Ordering::Relaxed,
        );
        self.check_formats = check;
        self
    }

    /// `--check-all-formats`: test every format, best first, before listing
    /// and selection, dropping those that fail (`process_video_result`).
    pub fn check_all_formats(&self, info: &mut VideoInfo) {
        if self.check_formats != CheckFormats::All {
            return;
        }
        let mut working = Vec::new();
        for format in info.formats.iter().rev() {
            screen::screen(&format!("[info] Testing format {}", format.format_id));
            if test_format(&self.client, info, format) {
                working.push(format.clone());
            } else {
                screen::screen(&format!(
                    "[info] Unable to download format {}. Skipping...",
                    format.format_id
                ));
            }
        }
        working.reverse();
        info.formats = working;
    }

    /// `--default-search PREFIX` for inputs without a URL scheme.
    pub fn with_default_search(mut self, prefix: Option<String>) -> Self {
        self.default_search = prefix;
        self
    }

    /// `--test`: download only the first 10241 bytes of HTTP media.
    pub fn with_test(mut self, test: bool) -> Self {
        self.test = test;
        self.ffmpeg_settings.test = test;
        self
    }

    /// `--hls-use-mpegts` for FFmpeg HLS downloads.
    pub fn with_hls_use_mpegts(mut self, mpegts: Option<bool>) -> Self {
        self.ffmpeg_settings.hls_use_mpegts = mpegts;
        self
    }

    pub fn with_prefer_best_format(mut self, prefer_best: bool) -> Self {
        self.prefer_best = prefer_best;
        self
    }

    /// Sort formats like upstream with user fields and flags.
    pub fn with_format_sort(mut self, options: FormatSortOptions) -> Self {
        self.format_sort = options;
        self
    }

    /// Upstream `process_video_result` format processing for every video of
    /// an extracted or loaded result: common fields, thumbnails, per-format
    /// fills, `FormatSorter` order (worst first) and format ID cleanup.
    pub fn process_result(&self, result: &mut ExtractResult) -> Result<()> {
        // Nested playlists are processed like extracted ones.
        if let ExtractResult::Playlist(playlist) = result
            && !playlist.nested.is_empty()
        {
            let mut nested = Vec::new();
            for inner in std::mem::take(&mut playlist.nested) {
                let mut inner = ExtractResult::Playlist(inner);
                self.process_result(&mut inner)?;
                if let ExtractResult::Playlist(inner) = inner {
                    nested.push(inner);
                }
            }
            playlist.nested = nested;
        }
        // A playlist's entries report their sort order as each is processed.
        let playlist = matches!(result, ExtractResult::Playlist(_));
        // `_fill_common_fields(final=False)`: a playlist's duration string,
        // dates from timestamps and release year.
        if let ExtractResult::Playlist(playlist) = result {
            let extra = &mut playlist.extra;
            if let Some(duration) = extra.get("duration").and_then(serde_json::Value::as_f64) {
                extra.insert(
                    "duration_string".into(),
                    info::format_seconds(duration.max(0.0) as u64).into(),
                );
            }
            for (timestamp, date) in [
                ("timestamp", "upload_date"),
                ("release_timestamp", "release_date"),
                ("modified_timestamp", "modified_date"),
            ] {
                if extra.get(date).is_none_or(serde_json::Value::is_null)
                    && let Some(day) = extra
                        .get(timestamp)
                        .and_then(serde_json::Value::as_f64)
                        .and_then(|value| dates::strftime_date(value.floor() as i64))
                {
                    extra.insert(date.into(), day.into());
                }
            }
            let year_set = extra.get("release_year").is_some_and(|year| match year {
                serde_json::Value::Null | serde_json::Value::Bool(false) => false,
                serde_json::Value::Number(number) => number.as_f64() != Some(0.0),
                serde_json::Value::String(text) => !text.is_empty(),
                _ => true,
            });
            if !year_set
                && let Some(year) = extra
                    .get("release_date")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|date| dates::py_int(&date.chars().take(4).collect::<String>()))
            {
                extra.insert("release_year".into(), year.into());
            }
        }
        // `_fill_common_fields(final=False)`: the deprecated multivalue fields.
        if let ExtractResult::Playlist(playlist) = result {
            for (old, new) in info::MULTIVALUE_FIELDS {
                if playlist.extra.contains_key(new) && playlist.extra.contains_key(old) {
                    continue;
                }
                match (playlist.extra.get(old), playlist.extra.get(new)) {
                    (Some(serde_json::Value::String(text)), _) if !text.is_empty() => {
                        let values: Vec<serde_json::Value> =
                            text.split(", ").map(serde_json::Value::from).collect();
                        playlist.extra.insert(new.into(), values.into());
                    }
                    (_, Some(serde_json::Value::Array(values))) if !values.is_empty() => {
                        let joined = values
                            .iter()
                            .map(|value| {
                                value.as_str().unwrap_or_default().replace(',', "\u{ff0c}")
                            })
                            .collect::<Vec<_>>()
                            .join(", ");
                        playlist.extra.insert(old.into(), joined.into());
                    }
                    _ => {}
                }
            }
        }
        // `_sanitize_thumbnails`, after `_fill_common_fields`: a lone
        // truthy `thumbnail` becomes the `thumbnails` list.
        if let ExtractResult::Playlist(playlist) = result {
            if !playlist.extra.contains_key("thumbnails")
                && let Some(serde_json::Value::String(thumbnail)) = playlist.extra.get("thumbnail")
                && !thumbnail.is_empty()
            {
                let thumbnails = serde_json::json!([{"url": thumbnail}]);
                playlist.extra.insert("thumbnails".into(), thumbnails);
            }
            if let Some(serde_json::Value::Array(thumbnails)) = playlist.extra.get_mut("thumbnails")
            {
                info::sanitize_thumbnail_values(thumbnails);
            }
        }
        let entries = match result {
            ExtractResult::Video(video) => std::slice::from_mut(video),
            ExtractResult::Playlist(playlist) => playlist.entries.as_mut_slice(),
        };
        for entry in entries {
            // `__x_forwarded_for_ip`, copied to the formats' `http_headers`.
            let fake_ip = entry.extra.shift_remove(geo::FAKE_IP_FIELD);
            // `_fill_common_fields`: a generic title for a missing or empty one.
            let missing = entry.extra.shift_remove(info::TITLE_MISSING).is_some();
            if missing {
                entry.fulltitle = None;
                screen::warning(
                    "Extractor failed to obtain \"title\". Creating a generic title instead",
                );
                entry.title = format!("{} video #{}", entry.extractor.replace(':', "-"), entry.id);
            } else if entry.title.is_empty()
                && entry.fulltitle.is_none()
                && !entry.extractor.is_empty()
            {
                // `title == ''`.
                entry.fulltitle = Some(String::new());
                screen::debug("Extractor gave empty title. Creating a generic title");
                entry.title = format!("{} video #{}", entry.extractor.replace(':', "-"), entry.id);
            } else {
                entry.fulltitle = Some(entry.title.clone());
            }
            // `process_video_result`: a section's own duration.
            if let (Some(start), Some(end)) = (entry.section_start, entry.section_end)
                && end.as_f64() != 0.0
            {
                let seconds = ((end.as_f64() - start.as_f64()) * 1000.0).round() / 1000.0;
                entry.metadata.duration = Some(crate::PyNumber::Float(seconds));
            }
            // `process_video_result`: a zero or negative duration is dropped,
            // with a warning when it was negative.
            if let Some(duration) = entry.metadata.duration
                && duration.as_f64() <= 0.0
            {
                entry.metadata.duration = None;
                if duration.as_f64() != 0.0 {
                    screen::warning(
                        "\"duration\" field is negative, there is an error in extractor",
                    );
                }
            }
            // `process_video_result`: `_has_drm`, and DRM formats dropped
            // without `--allow-unplayable-formats`.
            let drm = |format: &VideoFormat| {
                format.extra.get("has_drm") == Some(&serde_json::Value::Bool(true))
            };
            let has_drm = entry.formats.iter().any(drm);
            if has_drm {
                entry
                    .extra
                    .insert("_has_drm".into(), serde_json::Value::Bool(true));
                if !no_formats::allow_unplayable() {
                    entry.formats.retain(|format| !drm(format));
                    if entry.formats.is_empty() {
                        entry
                            .extra
                            .insert(info::NO_FORMATS.into(), serde_json::Value::Bool(true));
                    }
                }
            }
            // `process_video_result`: `raise_no_formats` for a result
            // without formats.
            if entry.extra.shift_remove(info::NO_FORMATS).is_some() {
                // `msg = has_drm and 'This video is DRM protected'`: expected.
                let drm_message = has_drm.then_some("This video is DRM protected");
                if no_formats::ignore() {
                    // `--ignore-no-formats-error`: processing continues.
                    screen::warning(drm_message.unwrap_or("No video formats found!"));
                    entry
                        .extra
                        .insert(info::FORMATS_IGNORED.into(), serde_json::Value::Bool(true));
                } else {
                    let message = match drm_message {
                        Some(message) => format!("[{}] {}: {message}", entry.extractor, entry.id),
                        None => format!(
                            "[{}] {}: No video formats found!; {}",
                            entry.extractor, entry.id, BUG_REPORTS_MESSAGE
                        ),
                    };
                    // A playlist entry's error is reported when the entry is
                    // processed; the other entries continue.
                    if playlist {
                        entry.extra.insert(ENTRY_ERROR.into(), message.into());
                        continue;
                    }
                    return Err(Error::Reported(message));
                }
            }
            // `process_video_result`: `display_id` defaults to the ID.
            // (a `None` value too, in its place).
            if entry
                .extra
                .get("display_id")
                .is_none_or(serde_json::Value::is_null)
            {
                entry
                    .extra
                    .insert("display_id".into(), entry.id.clone().into());
            }
            entry.metadata.fill_common_fields();
            // `_fill_common_fields`: release and modification dates from
            // their timestamps, then `release_year` from `release_date`.
            for (timestamp, date) in [
                ("release_timestamp", "release_date"),
                ("modified_timestamp", "modified_date"),
            ] {
                if entry.extra.get(date).is_none_or(serde_json::Value::is_null)
                    && let Some(day) = entry
                        .extra
                        .get(timestamp)
                        .and_then(serde_json::Value::as_f64)
                        .and_then(|value| dates::strftime_date(value.floor() as i64))
                {
                    entry.extra.insert(date.into(), day.into());
                }
            }
            let year_set = entry
                .extra
                .get("release_year")
                .is_some_and(|year| match year {
                    serde_json::Value::Null | serde_json::Value::Bool(false) => false,
                    serde_json::Value::Number(number) => number.as_f64() != Some(0.0),
                    serde_json::Value::String(text) => !text.is_empty(),
                    _ => true,
                });
            if !year_set
                && let Some(year) = entry
                    .extra
                    .get("release_date")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|date| dates::py_int(&date.chars().take(4).collect::<String>()))
            {
                entry.extra.insert("release_year".into(), year.into());
            }
            // `_fill_common_fields`: without a `live_status`, the first of
            // `is_live`/`was_live` that is not `False` decides it if truthy
            // (both `False`: `not_live`).
            if entry.metadata.live_status.is_none() {
                let flag = |key: &str| entry.extra.get(key).cloned();
                let mut status = None;
                for key in ["is_live", "was_live"] {
                    let value = flag(key);
                    if value == Some(serde_json::Value::Bool(false)) {
                        continue;
                    }
                    if value.as_ref().is_some_and(|value| match value {
                        serde_json::Value::Null => false,
                        serde_json::Value::Bool(flag) => *flag,
                        serde_json::Value::Number(number) => number.as_f64() != Some(0.0),
                        serde_json::Value::String(text) => !text.is_empty(),
                        serde_json::Value::Array(items) => !items.is_empty(),
                        serde_json::Value::Object(items) => !items.is_empty(),
                    }) {
                        status = Some(key.to_owned());
                    }
                    break;
                }
                if ["is_live", "was_live"]
                    .iter()
                    .all(|key| flag(key) == Some(serde_json::Value::Bool(false)))
                {
                    status = Some("not_live".into());
                }
                entry.metadata.live_status = status;
            }
            // `_fill_common_fields`: `is_live`/`was_live` from `live_status`.
            if let Some(status) = entry.metadata.live_status.clone() {
                for key in ["is_live", "was_live"] {
                    crate::info::set_default(
                        &mut entry.extra,
                        key,
                        serde_json::Value::Bool(status == key),
                    );
                }
                if status == "post_live" {
                    entry.extra.insert("was_live".into(), true.into());
                }
            }
            // `_fill_common_fields`: clean titles for numbered chapters,
            // seasons and episodes.
            for field in ["chapter", "season", "episode"] {
                let number = entry
                    .extra
                    .get(&format!("{field}_number"))
                    .and_then(serde_json::Value::as_i64);
                let present = entry.extra.get(field).is_some_and(|value| match value {
                    serde_json::Value::Null => false,
                    serde_json::Value::String(text) => !text.is_empty(),
                    _ => true,
                });
                if let Some(number) = number
                    && !present
                {
                    let mut name = field.to_owned();
                    name[..1].make_ascii_uppercase();
                    entry
                        .extra
                        .insert(field.into(), format!("{name} {number}").into());
                }
            }
            // `_fill_common_fields`: the deprecated single-value fields and
            // their lists, each from the other (the derived keys are noted
            // for `info_order`).
            let mut derived = Vec::new();
            for (old, new) in info::MULTIVALUE_FIELDS {
                if entry.extra.contains_key(new) && entry.extra.contains_key(old) {
                    continue;
                }
                match (entry.extra.get(old), entry.extra.get(new)) {
                    (Some(serde_json::Value::String(text)), _) if !text.is_empty() => {
                        let values: Vec<serde_json::Value> =
                            text.split(", ").map(serde_json::Value::from).collect();
                        entry.extra.insert(new.into(), values.into());
                        derived.push(new);
                    }
                    (_, Some(serde_json::Value::Array(values))) if !values.is_empty() => {
                        let joined = values
                            .iter()
                            .map(|value| {
                                value.as_str().unwrap_or_default().replace(',', "\u{ff0c}")
                            })
                            .collect::<Vec<_>>()
                            .join(", ");
                        entry.extra.insert(old.into(), joined.into());
                        derived.push(old);
                    }
                    _ => {}
                }
            }
            if !derived.is_empty()
                && let Some(serde_json::Value::String(shape)) =
                    entry.extra.get_mut(info_order::INFO_SHAPE)
            {
                shape.push_str(&format!("|multivalue:{}", derived.join(",")));
            }
            // `process_video_result`: a live stream not downloaded from the
            // start is titled with the local time.
            if self.live_from_now(entry) {
                entry.title = format!("{} {}", entry.title, dates::local_now_minutes());
            }
            // Only formats downloadable from the start with
            // `--live-from-start`, only the others without it.
            // (`_get_formats`: a result without formats is its own format.)
            if is_live(entry) {
                let from_start = self.live_from_start;
                let starts = |extra: &serde_json::Map<String, serde_json::Value>| {
                    (extra.get("is_from_start") == Some(&serde_json::Value::Bool(true)))
                        == from_start
                };
                let none_left = if entry.formats.is_empty() {
                    !starts(&entry.extra)
                } else {
                    entry.formats.retain(|format| starts(&format.extra));
                    entry.formats.is_empty()
                };
                if from_start && none_left {
                    return Err(Error::Reported(format!(
                        "[{}] {}: --live-from-start is passed, but there are no formats that can be downloaded from the start. If you want to download from the current time, use --no-live-from-start",
                        entry.extractor, entry.id
                    )));
                }
            }
            entry.metadata.fill_chapters();
            entry.metadata.sanitize_thumbnails();
            // Thumbnails this port keeps as upstream's dicts (`None` values).
            if let Some(serde_json::Value::Array(thumbnails)) = entry.extra.get_mut("thumbnails") {
                info::sanitize_thumbnail_values(thumbnails);
                if entry.metadata.thumbnail.is_none()
                    && let Some(best) = thumbnails
                        .last()
                        .and_then(|thumbnail| thumbnail.get("url"))
                        .and_then(serde_json::Value::as_str)
                {
                    entry.metadata.thumbnail = Some(best.to_owned());
                }
            }
            info::fill_format_fields(&mut entry.formats);
            // `format['http_headers'] = _calc_headers(ChainMap(format, info))`;
            // HTML5 media carries the page as its Referer.
            let html5 = entry.extractor_key == "HTML5MediaEmbed";
            let info_own: Option<Vec<(String, serde_json::Value)>> = entry
                .extra
                .get("http_headers")
                .and_then(serde_json::Value::as_object)
                .map(|headers| {
                    headers
                        .iter()
                        .map(|(name, value)| (name.clone(), value.clone()))
                        .collect()
                });
            for format in &mut entry.formats {
                let mut own: Vec<(String, serde_json::Value)> = format
                    .http_headers
                    .take()
                    .map(|headers| headers.into_iter().collect())
                    .or_else(|| info_own.clone())
                    .unwrap_or_else(|| {
                        if html5 {
                            vec![("Referer".into(), entry.webpage_url.to_string().into())]
                        } else {
                            Vec::new()
                        }
                    });
                let mut merged: Vec<(String, serde_json::Value)> = self
                    .info_headers
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone().into()))
                    .collect();
                for (name, value) in own.drain(..) {
                    let name = ffmpeg_fd::py_title(&name);
                    match merged.iter_mut().find(|(existing, _)| *existing == name) {
                        Some(entry) => entry.1 = value,
                        None => merged.push((name, value)),
                    }
                }
                if let Some(ip) = &fake_ip
                    && !merged.iter().any(|(name, _)| name == "X-Forwarded-For")
                {
                    merged.push(("X-Forwarded-For".into(), ip.clone()));
                }
                format.http_headers = (!merged.is_empty()).then(|| merged.into_iter().collect());
                // Cookies for the format URL become its `cookies` field.
                match self.cookies.info_field(&format.url) {
                    Some(cookies) => {
                        format.extra.insert("cookies".into(), cookies.into());
                    }
                    None => {
                        format.extra.shift_remove("cookies");
                    }
                }
            }
            let sorter = self.sorter(entry)?;
            // Upstream sorts (and reports the order) only with formats.
            if !playlist && !entry.formats.is_empty() && screen::verbose() {
                for line in sorter.verbose_info() {
                    screen::debug(&line);
                }
            }
            info::fill_filesize_approx(&mut entry.formats, entry.metadata.duration);
            info::sort_formats(&mut entry.formats, &sorter);
            info::sanitize_format_ids(&mut entry.formats);
            info::fill_format_descriptions(&mut entry.formats, entry.metadata.duration);
        }
        Ok(())
    }

    /// `FormatSorter` for an entry: the user's and the extractor's fields.
    fn sorter(&self, entry: &VideoInfo) -> Result<format_sort::FormatSorter> {
        let extractor: Vec<&str> = entry
            .format_sort_fields
            .iter()
            .map(String::as_str)
            .collect();
        // `--growing-file` prefers codecs QuickTime and editors take, after
        // the user's own sort fields.
        let mut user = std::borrow::Cow::Borrowed(&self.format_sort.fields);
        if self.growing_file && is_live(entry) {
            user.to_mut()
                .extend(["vcodec:h264".to_owned(), "acodec:aac".to_owned()]);
        }
        format_sort::FormatSorter::new(
            &user,
            &extractor,
            self.format_sort.force,
            self.format_sort.prefer_free,
        )
        .map_err(|error| {
            Error::FormatSelection(format!(
                "Invalid format sort string \"{}\" given by extractor",
                error.0
            ))
        })
    }

    /// Upstream `--merge-output-format`: ordered containers that a merge may
    /// use, passed to `get_compatible_ext` as preferences.
    pub fn with_merge_output_format(mut self, formats: Vec<String>) -> Self {
        self.merge_output_format = Some(formats);
        self
    }

    /// `--live-from-start`: live streams keep their title and default format.
    pub fn with_live_from_start(mut self, from_start: bool) -> Self {
        self.live_from_start = from_start;
        extractor::TWITCH_LIVE_FROM_START.store(from_start, std::sync::atomic::Ordering::Relaxed);
        self
    }

    /// Whether `info` is live and downloaded from now (upstream's
    /// `not get_from_start`).
    pub fn live_from_now(&self, info: &VideoInfo) -> bool {
        is_live(info) && !self.live_from_start
    }

    /// Upstream `allow_multiple_video_streams`/`allow_multiple_audio_streams`:
    /// merges may combine several video or audio streams.
    pub fn with_multistreams(mut self, video: bool, audio: bool) -> Self {
        self.multistreams = (video, audio);
        self
    }

    /// Download and extract audio, publishing the result without overwriting.
    pub fn download_audio_with_options(
        &self,
        info: &VideoInfo,
        destination: impl AsRef<Path>,
        options: DownloadOptions,
        audio: &AudioOptions,
    ) -> Result<AudioDownload> {
        self.download_audio_with_progress(info, destination, options, audio, |_| {
            std::ops::ControlFlow::Continue(())
        })
    }

    /// Observe/cancel transfer and audio processing. Only final publication emits
    /// Publishing; a processing failure can retain the completed source download.
    pub fn download_audio_with_progress(
        &self,
        info: &VideoInfo,
        destination: impl AsRef<Path>,
        options: DownloadOptions,
        audio: &AudioOptions,
        mut callback: impl FnMut(DownloadProgress) -> std::ops::ControlFlow<()>,
    ) -> Result<AudioDownload> {
        audio_download::download(
            self,
            info,
            destination.as_ref(),
            options,
            audio,
            &mut callback,
        )
    }

    /// Download, then extract audio and/or remux/recode local media in order.
    pub fn download_processed_with_options(
        &self,
        info: &VideoInfo,
        destination: impl AsRef<Path>,
        options: DownloadOptions,
        processors: &PostprocessOptions,
    ) -> Result<ProcessedDownload> {
        self.download_processed_with_progress(info, destination, options, processors, |_| {
            std::ops::ControlFlow::Continue(())
        })
    }

    /// Observe/cancel downloads and local processors before final publication.
    /// A processing failure preserves its latest completed input; keep-video also
    /// retains earlier stages. Multiple retained/final publications can partially succeed.
    pub fn download_processed_with_progress(
        &self,
        info: &VideoInfo,
        destination: impl AsRef<Path>,
        options: DownloadOptions,
        processors: &PostprocessOptions,
        mut callback: impl FnMut(DownloadProgress) -> std::ops::ControlFlow<()>,
    ) -> Result<ProcessedDownload> {
        postprocess::download(
            self,
            info,
            destination.as_ref(),
            options,
            processors,
            &mut callback,
        )
    }

    /// Convert a local subtitle sidecar, preserving its source on failure.
    pub fn convert_subtitle_with_options(
        &self,
        source: impl AsRef<Path>,
        source_ext: &str,
        options: &SubtitleConversion,
    ) -> Result<ConvertedSubtitle> {
        self.convert_subtitle_with_progress(source, source_ext, options, |_| {
            std::ops::ControlFlow::Continue(())
        })
    }

    /// Observe/cancel subtitle processing before no-clobber final publication.
    pub fn convert_subtitle_with_progress(
        &self,
        source: impl AsRef<Path>,
        source_ext: &str,
        options: &SubtitleConversion,
        mut callback: impl FnMut(DownloadProgress) -> std::ops::ControlFlow<()>,
    ) -> Result<ConvertedSubtitle> {
        subtitle_convert::convert(self, source.as_ref(), source_ext, options, &mut callback)
    }

    /// Insert a site extractor before the generic fallback.
    pub fn register_extractor(&mut self, extractor: Box<dyn Extractor>) {
        self.extractors.insert(0, extractor);
    }

    pub fn extract_info(&self, input: &str) -> Result<VideoInfo> {
        self.extract_info_with_format(input, None)
    }

    /// Extract and select a supported single source, two-stream pair, or fallback.
    /// Full upstream selector grammar and ranking are unsupported.
    pub fn extract_info_with_format(&self, input: &str, format: Option<&str>) -> Result<VideoInfo> {
        self.extract_result_with_format(input, format)?.into_video()
    }

    /// Extract a video or an HTML5 playlist, resolving formats for every entry.
    pub fn extract_result(&self, input: &str) -> Result<ExtractResult> {
        self.extract_result_with_format(input, None)
    }

    /// The supported selector applies independently to each entry.
    /// Extraction or selection failure in any entry fails the whole result.
    pub fn extract_result_with_format(
        &self,
        input: &str,
        format: Option<&str>,
    ) -> Result<ExtractResult> {
        self.extract_result_with_options(input, format, ExtractionOptions::default())
    }

    /// Retry the entire extractor registry after transient network failures.
    /// Earlier page reads and source probes repeat; no partial result is returned.
    /// Format selection and unsupported behavior are never retried.
    pub fn extract_result_with_options(
        &self,
        input: &str,
        format: Option<&str>,
        options: ExtractionOptions,
    ) -> Result<ExtractResult> {
        let mut info = self.extract_result_unselected_with_options(input, options)?;
        self.select_result(&mut info, format)?;
        Ok(info)
    }

    /// Apply a requested or configured default selector atomically to an extracted result.
    pub fn select_result(&self, result: &mut ExtractResult, format: Option<&str>) -> Result<()> {
        // Upstream `_default_format_spec` `prefer_best` (stdout output).
        let format = format.or(self.prefer_best.then_some("best/bestvideo+bestaudio"));
        result.select_format_with_merge(
            format,
            self.ffmpeg.is_some(),
            self.merge_output_format.as_deref(),
        )
    }

    /// Select formats for one video or playlist entry (upstream
    /// `process_video_result`), leaving it unchanged on failure; with
    /// several selected formats (`,`, `all`), the first is applied.
    pub fn select_video(&self, info: &mut VideoInfo, format: Option<&str>) -> Result<()> {
        let mut selected = self.select_videos(info, format)?;
        *info = selected.remove(0);
        Ok(())
    }

    /// `--growing-file`: selected live results become QuickTime files (or
    /// fragmented MP4 for codecs MOV cannot hold, such as AV1).
    fn growing_ext(&self, mut selected: Vec<VideoInfo>) -> Vec<VideoInfo> {
        for entry in &mut selected {
            if self.grows(entry) {
                entry.ext = growing::ext(entry).into();
            } else if self.growing_file && is_live(entry) {
                screen::warning(if self.ffmpeg.is_none() {
                    "--growing-file needs ffmpeg; downloading the live stream normally"
                } else {
                    "--growing-file does not support this live stream's protocol; downloading it normally"
                });
            }
        }
        selected
    }

    /// Upstream `formats_to_download`: one result per selected format, each
    /// downloaded on its own by `process_info`.
    pub fn select_videos(&self, info: &VideoInfo, format: Option<&str>) -> Result<Vec<VideoInfo>> {
        // `--ignore-no-formats-error`: "Process what we can, even without
        // any available formats" (`formats_to_download = [{}]`).
        if info.has_no_formats() {
            screen::warning("Requested format is not available");
            return Ok(vec![info.clone()]);
        }
        // A playlist entry's `sort_formats` report (see `process_result`).
        if info.playlist_index.is_some() && !info.formats.is_empty() && screen::verbose() {
            for line in self.sorter(info)?.verbose_info() {
                screen::debug(&line);
            }
        }
        let prefer_best = self.prefer_best || self.live_from_now(info);
        if format.is_none() && prefer_best {
            screen::debug("Default format spec: best/bestvideo+bestaudio");
        }
        let format = format.or(prefer_best.then_some("best/bestvideo+bestaudio"));
        let options = format_select::Options {
            merge_output_format: self.merge_output_format.clone(),
            prefer_free: self.format_sort.prefer_free,
            multiple_video: self.multistreams.0,
            multiple_audio: self.multistreams.1,
        };
        // By default, only formats with DRM or `__needs_testing` are tested.
        let needs_testing = |format: &VideoFormat| {
            format.extra.get("has_drm") == Some(&serde_json::Value::Bool(true))
                || format
                    .extra
                    .get("__needs_testing")
                    .is_some_and(|value| value.as_bool() != Some(false) && !value.is_null())
        };
        // `--allow-unplayable-formats` tests none of them.
        let default = self.check_formats == CheckFormats::Default
            && !no_formats::allow_unplayable()
            && info.formats.iter().any(needs_testing);
        if self.check_formats != CheckFormats::Selected && !default {
            let selected = info.select_formats(format, self.ffmpeg.is_some(), &options)?;
            return Ok(self.growing_ext(selected));
        }
        // Upstream `_check_formats` for `selected`: each candidate is tested
        // once, in preference order, as selection reaches it.
        let client = self.client.clone();
        let snapshot = info.clone();
        let results: std::rc::Rc<std::cell::RefCell<std::collections::HashMap<String, bool>>> =
            Default::default();
        let tested = results.clone();
        let check = Box::new(move |format: &VideoFormat| {
            if default && !needs_testing(format) {
                return true;
            }
            *tested
                .borrow_mut()
                .entry(format.format_id.clone())
                .or_insert_with(|| {
                    screen::screen(&format!("[info] Testing format {}", format.format_id));
                    let works = test_format(&client, &snapshot, format);
                    if !works {
                        screen::warning(&format!(
                            "Unable to download format {}. Skipping...",
                            format.format_id
                        ));
                    }
                    works
                })
        });
        let mut selected = info::with_format_check(check, || {
            info.select_formats(format, self.ffmpeg.is_some(), &options)
        })?;
        // `f['__working'] = success` (and a working format's
        // `__needs_testing` popped) on the shared format dicts.
        let results = results.borrow();
        let mark = |format: &mut VideoFormat| {
            if let Some(works) = results.get(&format.format_id) {
                if *works {
                    format.extra.shift_remove("__needs_testing");
                }
                format.extra.insert("__working".into(), (*works).into());
            }
        };
        for entry in &mut selected {
            entry.formats.iter_mut().for_each(mark);
            entry.requested_formats.iter_mut().for_each(mark);
        }
        Ok(self.growing_ext(selected))
    }

    /// Extract all supported alternatives without applying library format selection.
    /// Useful for inspection, including separate-only presentations without a merger.
    /// Metadata retains the extractor's representative fields; select before downloading.
    pub fn extract_result_unselected(&self, input: &str) -> Result<ExtractResult> {
        self.extract_result_unselected_with_options(input, ExtractionOptions::default())
    }

    /// Retry whole extraction independently of format selection and output rendering.
    /// All playlist entries must extract successfully before a result is returned.
    pub fn extract_result_unselected_with_options(
        &self,
        input: &str,
        options: ExtractionOptions,
    ) -> Result<ExtractResult> {
        let _context = control::enter(
            options.interrupt.as_ref(),
            options.scope.as_ref(),
            &self.cookies,
        )?;
        control::check()?;
        let url = if has_scheme(input) {
            Url::parse(input)?
        } else {
            // Extractors whose `_VALID_URL` takes text without a scheme (a
            // naked YouTube ID, `:ytfav`) come before GenericIE.
            match extractor::bare_input(input) {
                Some((url, name)) if self.extractor_allowed(name) => url,
                _ => return self.extract_unqualified(input, options),
            }
        };
        if url.scheme() == "file" {
            return self.extract_file(&url);
        }
        // Extractor pseudo-URLs (`jwplatform:ID`) reach their extractor.
        if has_scheme(input)
            && !matches!(url.scheme(), "http" | "https")
            && !extractor::pseudo_url(input)
        {
            return Err(Error::UnsupportedScheme(url.scheme().into()));
        }
        for attempt in 0..=options.retries {
            match self.extract_once(&url) {
                Ok(mut info) => {
                    info.add_default_extra_info(input);
                    if !self.defer_entries {
                        self.fetch_all_pending(&mut info)?;
                    }
                    return Ok(info);
                }
                Err(error) => {
                    let Some(delay) = retry::failure_delay(&error) else {
                        return Err(error);
                    };
                    if attempt == options.retries {
                        return Err(error);
                    }
                    retry::backoff(u32::from(attempt), delay);
                    control::check()?;
                }
            }
        }
        unreachable!("the final attempt returns success or an error")
    }

    /// Formats from an HLS or DASH [`Manifest`] (inline or fetched), as a
    /// processed but unselected video result: pass it to
    /// [`select_result`](Self::select_result) and download it like an
    /// extracted one. A master playlist's variant playlists are fetched.
    /// `options.scope` applies to the manifest and variant requests; pass the
    /// same scope in [`DownloadControl`] for the media requests.
    pub fn extract_manifest(
        &self,
        manifest: &Manifest,
        options: ExtractionOptions,
    ) -> Result<ExtractResult> {
        let _context = control::enter(
            options.interrupt.as_ref(),
            options.scope.as_ref(),
            &self.cookies,
        )?;
        control::check()?;
        if !matches!(manifest.url.scheme(), "http" | "https") {
            return Err(Error::UnsupportedScheme(manifest.url.scheme().into()));
        }
        let webpage = manifest.webpage_url.as_ref().unwrap_or(&manifest.url);
        let mut info = match (&manifest.data, manifest.kind) {
            (Some(text), ManifestKind::Hls) => {
                extractor::hls::from_text(&self.client, text, &manifest.url, webpage, false)?
            }
            (Some(text), ManifestKind::Dash) => {
                extractor::dash::from_text(text, &manifest.url, webpage)?
            }
            (None, kind) => {
                let response = extractor::direct::probe(&self.client, &manifest.url, None)?;
                match kind {
                    ManifestKind::Hls => extractor::hls::from_response(
                        &self.client,
                        response,
                        webpage,
                        None,
                        true,
                        None,
                    )?,
                    ManifestKind::Dash => extractor::dash::from_response(response, webpage, true)?,
                }
                .ok_or_else(|| Error::UnsupportedManifest("the manifest has no media".into()))?
            }
        };
        if let Some(id) = &manifest.id {
            info.id = id.clone();
            if manifest.title.is_none() {
                info.title = id.clone();
            }
        }
        if let Some(title) = &manifest.title {
            info.title = title.clone();
        }
        let mut result = ExtractResult::Video(info);
        self.process_result(&mut result)?;
        Ok(result)
    }

    /// The extractor names in matching order (`None`: the registry order).
    fn extractor_order(&self) -> Option<Vec<&'static str>> {
        if self.force_generic {
            return Some(vec!["generic"]);
        }
        self.allowed_extractors.clone()
    }

    /// Whether the named extractor is among those matched.
    fn extractor_allowed(&self, name: &str) -> bool {
        self.extractor_order().is_none_or(|order| {
            order
                .iter()
                .take_while(|item| **item != "end")
                .any(|item| item.eq_ignore_ascii_case(name))
        })
    }

    fn generic_allowed(&self) -> bool {
        self.extractor_order()
            .is_none_or(|order| order.contains(&"generic"))
    }

    fn no_suitable(&self, url: &str) -> Error {
        let key = if self.force_generic { " (Generic)" } else { "" };
        Error::Reported(format!("No suitable extractor{key} found for URL {url}"))
    }

    /// Upstream generic extraction of a `file:` URL, refused unless
    /// `--enable-file-urls` (urllib's file handler then reads it).
    fn extract_file(&self, url: &Url) -> Result<ExtractResult> {
        if !self.generic_allowed() {
            return Err(self.no_suitable(url.as_str()));
        }
        screen::set_extractor("generic");
        screen::screen(&format!(
            "[generic] Extracting URL: {}",
            screen::extracting_url(url.as_str())
        ));
        screen::screen(&format!(
            "[generic] {}: Downloading webpage",
            screen::generic_id(url.as_str())
        ));
        if !self.enable_file_urls {
            return Err(Error::Reported(
                "file:// URLs are disabled by default in yt-dlp for security reasons. Use --enable-file-urls to enable at your own risk."
                    .into(),
            ));
        }
        let mut result = ExtractResult::Video(extractor::direct::from_file(url)?);
        self.process_result(&mut result)?;
        Ok(result)
    }

    /// Upstream `GenericIE._real_extract` for an input without a scheme.
    fn extract_unqualified(
        &self,
        input: &str,
        options: ExtractionOptions,
    ) -> Result<ExtractResult> {
        if !self.generic_allowed() {
            return Err(self.no_suitable(input));
        }
        screen::screen(&format!(
            "[generic] Extracting URL: {}",
            screen::extracting_url(input)
        ));
        let search = self.default_search.as_deref().unwrap_or("fixup_error");
        static DOMAIN: std::sync::LazyLock<regex::Regex> =
            std::sync::LazyLock::new(|| regex::Regex::new(r"^[^\s/]+\.[^\s/]+/").unwrap());
        if matches!(search, "auto" | "auto_warning" | "fixup_error") && DOMAIN.is_match(input) {
            screen::warning("[generic] The url doesn't specify the protocol, trying with https");
            return self
                .extract_result_unselected_with_options(&format!("https://{input}"), options);
        }
        let quoted = output::traverse::py_repr_str(input);
        if matches!(search, "error" | "fixup_error") {
            return Err(Error::Reported(format!(
                "[generic] {quoted} is not a valid URL"
            )));
        }
        Err(Error::Reported(format!(
            "[generic] {quoted}: search extractors ({search}) are not supported natively"
        )))
    }

    /// The extraction error of a playlist entry whose processing failed
    /// (upstream reports it for that entry and continues with the others).
    pub fn entry_error(entry: &VideoInfo) -> Option<&str> {
        entry
            .extra
            .get(ENTRY_ERROR)
            .and_then(serde_json::Value::as_str)
    }

    /// The error of a paged playlist entry whose page failed to download
    /// (already reported upstream; the entry is `None`).
    pub fn fetch_error(entry: &VideoInfo) -> Option<&str> {
        extractor::paged::fetch_error(entry)
    }

    /// Whether `entry` is an unresolved `url_transparent` entry.
    pub fn is_transparent(entry: &VideoInfo) -> bool {
        entry.extra.contains_key(TRANSPARENT)
    }

    /// Upstream `process_ie_result` for a `url_transparent` playlist entry
    /// (RSS items): its URL is extracted (smuggling the `guid` as
    /// `force_videoid`), then updated with the entry's non-`None` fields,
    /// keeping the playlist fields. Other entries are returned unchanged.
    pub fn resolve_entry(&self, entry: VideoInfo) -> Result<VideoInfo> {
        let Some(marker) = entry.extra.get(TRANSPARENT).cloned() else {
            return Ok(entry);
        };
        let target = marker["url"].as_str().unwrap_or_default().to_owned();
        let Ok(url) = Url::parse(&target) else {
            // GenericIE reports the (smuggled) input as not a URL.
            let shown = match marker["force_videoid"].as_str() {
                Some(id) => extractor::html5::smuggled_text(&target, id),
                None => target,
            };
            screen::set_extractor("generic");
            screen::screen(&format!(
                "[generic] Extracting URL: {}",
                screen::extracting_url(&shown)
            ));
            return Err(Error::Reported(format!(
                "[generic] '{shown}' is not a valid URL"
            )));
        };
        let shown = match marker["force_videoid"].as_str() {
            Some(id) => extractor::html5::smuggle_video_id(&url, id),
            None => target.clone(),
        };
        let fields: Vec<(String, serde_json::Value)> = marker["fields"]
            .as_object()
            .map(|fields| {
                fields
                    .iter()
                    .filter(|(_, value)| !value.is_null())
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let shown_url = Url::parse(&shown).unwrap_or_else(|_| url.clone());
        // A playlist extractor's `url` entry (`_type: url`) keeps the
        // extracted video's own `webpage_url`.
        // (as does a URL whose extractor names its own: YouTube's feeds).
        let plain_url = marker["entry"]["_type"] == "url" || marker["keep_webpage_url"] == true;
        let merge = |result: &mut ExtractResult| -> Result<()> {
            let ExtractResult::Video(info) = result else {
                return Err(Error::UnsupportedHtml5(
                    "url_transparent playlists are not supported natively".into(),
                ));
            };
            let mut inner_shape = match info.extra.get(info_order::INFO_SHAPE) {
                Some(serde_json::Value::String(shape)) => shape.clone(),
                _ => String::new(),
            };
            // An overlaid `title` precedes `_fill_common_fields`: no generic
            // title for an untitled inner extraction.
            if fields.iter().any(|(key, _)| key == "title")
                && info.extra.shift_remove(info::TITLE_MISSING).is_some()
            {
                inner_shape = inner_shape.replacen("-untitled;", ";", 1);
            }
            for (key, value) in &fields {
                match key.as_str() {
                    "title" => info.title = value.as_str().unwrap_or_default().to_owned(),
                    _ => metadata_parser::set_field(info, key, value.clone()),
                }
            }
            // `process_video_result`: `live_status` from an overlaid `is_live`.
            if info.metadata.live_status.is_none()
                && fields
                    .iter()
                    .any(|(key, value)| key == "is_live" && *value == serde_json::Value::Bool(true))
            {
                info.metadata.live_status = Some("is_live".into());
            }
            // Upstream extracts (and names) the smuggled URL.
            if !plain_url {
                info.webpage_url = shown_url.clone();
            }
            let keys: Vec<&str> = fields.iter().map(|(key, _)| key.as_str()).collect();
            info.extra.insert(
                info_order::INFO_SHAPE.into(),
                format!("{inner_shape}|transparent-entry:{}", keys.join(",")).into(),
            );
            Ok(())
        };
        let result = self.extract_once_with(&url, &merge);
        extractor::html5::clear_smuggled();
        let mut result = result?;
        result.add_default_extra_info(&shown);
        let mut info = result.into_video()?;
        // The playlist's fields stay.
        info.playlist_index = entry.playlist_index;
        info.playlist_autonumber = entry.playlist_autonumber;
        info.playlist_count = entry.playlist_count;
        info.n_entries = entry.n_entries;
        info.last_playlist_index = entry.last_playlist_index;
        for key in [
            "playlist",
            "playlist_id",
            "playlist_title",
            "playlist_uploader",
            "playlist_uploader_id",
            "playlist_channel",
            "playlist_channel_id",
            "playlist_webpage_url",
        ] {
            if let Some(value) = entry.extra.get(key) {
                info.extra.insert(key.into(), value.clone());
            }
        }
        Ok(info)
    }

    /// A `--flat-playlist` `url_transparent` entry, unextracted: upstream's
    /// raw entry dict (`-J`) and, with `epoch`, the dict its forced
    /// printings see (`-j`, `--print`).
    pub fn flat_entry(
        entry: &VideoInfo,
        epoch: u64,
    ) -> Option<(
        serde_json::Map<String, serde_json::Value>,
        serde_json::Map<String, serde_json::Value>,
    )> {
        use serde_json::Value;
        let marker = entry.extra.get(TRANSPARENT)?;
        let target = marker["url"].as_str().unwrap_or_default();
        let shown = match marker["force_videoid"].as_str() {
            Some(id) => extractor::html5::smuggled_text(target, id),
            None => target.to_owned(),
        };
        let mut raw = serde_json::Map::new();
        // An embed's `url_result(url, ie)` (`ie_key` set) and its
        // `add_default_extra_info(info, ie, None)`.
        let ie_key = marker["ie_key"].as_str();
        if let Some(entry) = marker["entry"].as_object() {
            // A playlist extractor's own entry dict (`YoutubeTabIE`).
            raw.extend(entry.clone());
        } else if let Some(ie_key) = ie_key {
            raw.insert("ie_key".into(), ie_key.into());
            raw.insert("_type".into(), "url".into());
            raw.insert("url".into(), shown.clone().into());
            raw.insert(
                "extractor".into(),
                marker["extractor"].as_str().unwrap_or(ie_key).into(),
            );
            raw.insert("extractor_key".into(), ie_key.into());
        } else {
            raw.insert("_type".into(), "url_transparent".into());
            raw.insert("url".into(), shown.clone().into());
        }
        if let Some(fields) = marker["fields"].as_object() {
            raw.extend(fields.clone());
        }
        raw.insert("__x_forwarded_for_ip".into(), Value::Null);
        let mut full = raw.clone();
        if ie_key.is_some() {
            // `ie.get_temp_id(url)` (`None` when the URL does not match: an
            // empty ID).
            let id = if entry.id.is_empty() {
                Value::Null
            } else {
                entry.id.clone().into()
            };
            full.insert("id".into(), id);
        }
        full.insert("webpage_url".into(), shown.clone().into());
        full.insert("original_url".into(), shown.clone().into());
        if let Ok(url) = Url::parse(&shown) {
            full.insert(
                "webpage_url_basename".into(),
                info::url_basename(&url).into(),
            );
            full.insert("webpage_url_domain".into(), info::get_domain(&url).into());
        }
        // `add_default_extra_info(info, ie, url)` for an entry dict's `ie_key`.
        let entry_dict = marker["entry"].is_object();
        if entry_dict && let Some(ie_key) = ie_key {
            for (key, value) in [
                ("extractor", marker["extractor"].as_str().unwrap_or(ie_key)),
                ("extractor_key", ie_key),
            ] {
                if !full.contains_key(key) {
                    full.insert(key.into(), value.into());
                }
            }
        }
        let number = |value: Option<usize>| value.map_or(Value::Null, Value::from);
        // `add_extra_info` keeps an entry's own keys.
        let own: Vec<String> = raw.keys().cloned().collect();
        full.insert("playlist_count".into(), number(entry.playlist_count));
        for key in [
            "playlist",
            "playlist_id",
            "playlist_title",
            "playlist_uploader",
            "playlist_uploader_id",
            "playlist_channel",
            "playlist_channel_id",
            "playlist_webpage_url",
        ] {
            full.insert(
                key.into(),
                entry.extra.get(key).cloned().unwrap_or(Value::Null),
            );
        }
        full.insert("n_entries".into(), number(entry.n_entries));
        full.insert("playlist_index".into(), number(entry.playlist_index));
        full.insert(
            "__last_playlist_index".into(),
            number(entry.last_playlist_index),
        );
        if ie_key.is_none() {
            // The playlist's extractor (`_playlist_infodict`).
            let playlist = |index: usize, default: &'static str| {
                marker["playlist_extractor"][index]
                    .as_str()
                    .unwrap_or(default)
                    .to_owned()
            };
            full.insert("extractor".into(), playlist(0, "generic").into());
            full.insert("extractor_key".into(), playlist(1, "Generic").into());
        }
        full.insert(
            "playlist_autonumber".into(),
            number(entry.playlist_autonumber),
        );
        for key in &own {
            // `if ie and not ie_result.get('id')`: a falsy own ID gives way to
            // the temporary one.
            if key == "id" && ie_key.is_some() && !extractor::nuxt::truthy(&raw[key]) {
                continue;
            }
            full.insert(key.clone(), raw[key].clone());
        }
        full.insert("epoch".into(), epoch.into());
        // `_fill_common_fields(final=False)`.
        if let Some(duration) = raw.get("duration").and_then(Value::as_f64) {
            full.insert(
                "duration_string".into(),
                info::format_seconds(duration.max(0.0) as u64).into(),
            );
        }
        if let Some(date) = raw
            .get("timestamp")
            .and_then(Value::as_f64)
            .and_then(|timestamp| dates::strftime_date(timestamp.floor() as i64))
        {
            full.insert("upload_date".into(), date.into());
        }
        full.insert("release_year".into(), Value::Null);
        Some((raw, full))
    }

    fn extract_once(&self, url: &Url) -> Result<ExtractResult> {
        self.extract_once_with(url, &|_| Ok(()))
    }

    /// [`Self::extract_once`] with a change to the extracted result before
    /// it is processed.
    fn extract_once_with(
        &self,
        url: &Url,
        before_processing: &dyn Fn(&mut ExtractResult) -> Result<()>,
    ) -> Result<ExtractResult> {
        let order = self.extractor_order();
        let extractors: Vec<&Box<dyn Extractor>> = match &order {
            None => self.extractors.iter().collect(),
            Some(order) => {
                let mut selected = Vec::new();
                for name in order {
                    if *name == "end" {
                        break;
                    }
                    selected.extend(
                        self.extractors
                            .iter()
                            .filter(|extractor| extractor.key().eq_ignore_ascii_case(name)),
                    );
                }
                selected
            }
        };
        for extractor in extractors {
            geo::begin(extractor.key(), extractor.geo());
            let mut extracted = extractor.extract_result(&self.client, url);
            // `extract`: a geo restriction may retry with a fake IP.
            if extracted.is_err() && geo::retry_with_fake_ip() {
                geo::restart();
                extracted = extractor.extract_result(&self.client, url);
            }
            let extracted = extracted?;
            if extracted.is_none() {
                geo::abandon();
            }
            if let Some(mut info) = extracted {
                geo::attach(&mut info);
                if let ExtractResult::Video(video) = &info
                    && let Some(marker) = video.extra.get(FOLLOW_TRANSPARENT).cloned()
                {
                    let target = Url::parse(marker["url"].as_str().unwrap_or_default())?;
                    // `exempted_fields`: without sections, the inner `id`,
                    // `extractor` and `extractor_key` are kept.
                    let sectioned = marker["fields"]
                        .get("section_start")
                        .is_some_and(|value| !value.is_null())
                        || marker["fields"]
                            .get("section_end")
                            .is_some_and(extractor::nuxt::truthy);
                    let fields: Vec<(String, serde_json::Value)> = marker["fields"]
                        .as_object()
                        .map(|fields| {
                            fields
                                .iter()
                                .filter(|(_, value)| !value.is_null())
                                .filter(|(key, _)| {
                                    sectioned
                                        || !matches!(
                                            key.as_str(),
                                            "id" | "extractor" | "extractor_key"
                                        )
                                })
                                .map(|(key, value)| (key.clone(), value.clone()))
                                .collect()
                        })
                        .unwrap_or_default();
                    let outer = url.clone();
                    // The outer result's fake IP is among its forced fields.
                    let outer_ip = geo::fake_ip();
                    let merge = |result: &mut ExtractResult| -> Result<()> {
                        let ExtractResult::Video(inner) = result else {
                            return Err(Error::UnsupportedHtml5(
                                "url_transparent playlists are not supported natively".into(),
                            ));
                        };
                        let mut inner_shape = match inner.extra.get(info_order::INFO_SHAPE) {
                            Some(serde_json::Value::String(shape)) => shape.clone(),
                            _ => String::new(),
                        };
                        // Overriding `subtitles` tracks keep the outer
                        // extractor's key order (`subtitle_keys`), not the
                        // inner one's.
                        if let Some(keys) = marker["subtitle_keys"].as_str()
                            && fields.iter().any(|(key, _)| key == "subtitles")
                        {
                            inner_shape = inner_shape
                                .split(';')
                                .filter(|part| !part.starts_with("subtitle="))
                                .collect::<Vec<_>>()
                                .join(";");
                            inner_shape.push_str(&format!(";subtitle={keys}"));
                        }
                        for (key, value) in &fields {
                            match key.as_str() {
                                "title" => {
                                    inner.title = value.as_str().unwrap_or_default().to_owned()
                                }
                                _ => metadata_parser::set_field(inner, key, value.clone()),
                            }
                        }
                        // `_sanitize_thumbnails` builds a `None` `thumbnails`
                        // from the forced `thumbnail`.
                        if fields.iter().any(|(key, _)| key == "thumbnail")
                            && inner
                                .extra
                                .get("thumbnails")
                                .is_some_and(serde_json::Value::is_null)
                        {
                            inner.extra.shift_remove("thumbnails");
                        }
                        inner.webpage_url = outer.clone();
                        if let Some(ip) = &outer_ip {
                            inner
                                .extra
                                .insert(geo::FAKE_IP_FIELD.into(), ip.clone().into());
                        }
                        let keys: Vec<&str> = fields.iter().map(|(key, _)| key.as_str()).collect();
                        inner.extra.insert(
                            info_order::INFO_SHAPE.into(),
                            format!("{inner_shape}|transparent:{}", keys.join(",")).into(),
                        );
                        before_processing(result)
                    };
                    return self.extract_once_with(&target, &merge);
                }
                if let ExtractResult::Video(video) = &info
                    && let Some(target) = video
                        .extra
                        .get(FOLLOW_URL)
                        .and_then(serde_json::Value::as_str)
                {
                    let target = extractor::target_url(target)?;
                    // A followed `url` result passes `original_url` on as
                    // extra info (see `info_order::FOLLOWED`).
                    let followed = |result: &mut ExtractResult| -> Result<()> {
                        let extra = match result {
                            ExtractResult::Video(video) => &mut video.extra,
                            ExtractResult::Playlist(playlist) => &mut playlist.extra,
                        };
                        extra.insert(info_order::FOLLOWED.into(), serde_json::Value::Bool(true));
                        before_processing(result)
                    };
                    return self.extract_once_with(&target, &followed);
                }
                before_processing(&mut info)?;
                self.process_result(&mut info)?;
                return Ok(info);
            }
        }
        // Upstream `UnsupportedURLIE` (`end`) or no matching extractor.
        let input = extractor::input_text(url);
        match &order {
            None => return Err(Error::UnsupportedUrl(input)),
            Some(order) if order.contains(&"end") => {
                screen::screen(&format!(
                    "[UnsupportedURL] Extracting URL: {}",
                    screen::extracting_url(&input)
                ));
                return Err(Error::Reported(format!("Unsupported URL: {input}")));
            }
            Some(_) => {}
        }
        Err(self.no_suitable(&input))
    }

    pub fn download(&self, info: &VideoInfo, destination: impl AsRef<Path>) -> Result<u64> {
        self.download_with_options(info, destination, DownloadOptions::default())
    }

    /// Download with bounded retries. Interrupted transfers may resume with a strong ETag and known length.
    pub fn download_with_options(
        &self,
        info: &VideoInfo,
        destination: impl AsRef<Path>,
        options: DownloadOptions,
    ) -> Result<u64> {
        self.download_with_progress(info, destination, options, |_| {
            std::ops::ControlFlow::Continue(())
        })
    }

    /// [`download_with_progress`](Self::download_with_progress) with a
    /// [`DownloadControl`]: its [`Interrupt`] can stop the download from
    /// another thread (keeping and publishing what was downloaded, as
    /// `Error::DownloadStopped { path }`) or cancel it (`Error::DownloadCancelled`,
    /// private temporary files removed), and its [`RequestScope`] adds
    /// headers and cookies to requests for its origin only.
    ///
    /// A stop publishes the data downloaded so far at `destination`: a
    /// whole-file transfer's bytes, whole fragments of a fragmented stream,
    /// the streams of a merged download so far merged with FFmpeg (the last
    /// one partial), or FFmpeg's output after it is asked to finish (`q`).
    /// A stop before any data arrived is a cancel.
    pub fn download_with_control(
        &self,
        info: &VideoInfo,
        destination: impl AsRef<Path>,
        options: DownloadOptions,
        control: &DownloadControl,
        mut callback: impl FnMut(DownloadProgress) -> std::ops::ControlFlow<()>,
    ) -> Result<u64> {
        let _context = control::enter(
            control.interrupt.as_ref(),
            control.scope.as_ref(),
            &self.cookies,
        )?;
        let interrupt = control.interrupt.clone();
        self.download_with_progress(info, destination, options, move |progress| {
            if interrupt.as_ref().is_some_and(Interrupt::is_requested) {
                std::ops::ControlFlow::Break(())
            } else {
                callback(progress)
            }
        })
    }

    /// Observe download checkpoints; return `ControlFlow::Break(())` to cancel.
    /// Cancellation removes the private temporary file and never retries or publishes.
    /// Callbacks run synchronously before requests, after validated headers and each
    /// written chunk, during retry waits (at intervals of at most 100 ms of sleep),
    /// and immediately before publication. `Publishing` is not a success event;
    /// only an `Ok` return confirms publication. Blocking requests, reads, and disk
    /// operations cannot be interrupted; existing request timeouts still apply.
    /// Byte counts may reset on whole-file/fragment retries or selected stream changes.
    /// No callback follows publication.
    pub fn download_with_progress(
        &self,
        info: &VideoInfo,
        destination: impl AsRef<Path>,
        options: DownloadOptions,
        mut callback: impl FnMut(DownloadProgress) -> std::ops::ControlFlow<()>,
    ) -> Result<u64> {
        // Upstream `get_suitable_downloader`: partial downloads and
        // `--downloader ffmpeg` use FFmpegFD, which also merges selected
        // formats directly.
        let ffmpeg_fd = self.uses_ffmpeg_fd(info);
        // FFmpegFD's `m3u8` protocol without ffmpeg (`_call_downloader`).
        if self.ffmpeg.is_none() && self.forced_ffmpeg_hls(info) {
            return Err(Error::Reported(
                "m3u8 download detected but ffmpeg could not be found. Please install".into(),
            ));
        }
        // Merged F4M formats are not supported (upstream merges F4mFD files).
        if info
            .requested_formats
            .iter()
            .any(|format| format.protocol.as_deref() == Some("f4m"))
        {
            return Err(Error::UnsupportedHtml5(
                "merging F4M (HDS) formats is not supported natively".into(),
            ));
        }
        let announced = ANNOUNCED.with(|announced| announced.take());
        if announced.as_ref() != Some(&info.url) {
            self.report_invoking(info);
        }
        if self.grows(info) {
            return self.download_growing(info, destination.as_ref(), options, &mut callback);
        }
        // `--keep-partial-on-stop`: SIGINT/SIGTERM stop the download like
        // `Interrupt::stop` (the outermost call holds the signal guard).
        let _signals = self.keep_partial_on_stop.then(control::SignalStop::enter);
        let mut callback = move |progress| {
            if control::requested() {
                std::ops::ControlFlow::Break(())
            } else {
                callback(progress)
            }
        };
        if ffmpeg_fd {
            let ffmpeg = self.ffmpeg.as_deref().ok_or_else(|| {
                Error::Reported(
                    "You have requested downloading the video partially, but ffmpeg is not installed. Aborting"
                        .into(),
                )
            })?;
            let network = ffmpeg_fd::Network {
                headers: &self.headers,
                cookies: &self.cookies,
                proxy: self.proxy.as_deref(),
            };
            // Indexed single-file streams fetch only the section's segments,
            // unless `--downloader ffmpeg` names FFmpeg for them.
            let ffmpeg_named = |protocol: &str| {
                self.downloader_name(protocol)
                    .is_some_and(|name| name.eq_ignore_ascii_case("ffmpeg"))
            };
            let named = if info.requested_formats.is_empty() {
                ffmpeg_named(
                    &info
                        .protocol
                        .clone()
                        .unwrap_or_else(|| info.url.scheme().to_owned()),
                )
            } else {
                info.requested_formats
                    .iter()
                    .any(|format| ffmpeg_named(&format.determine_protocol()))
            };
            let timecode = self.timecode_request(info);
            if !named && indexed_fd::supported(info) {
                let params = indexed_fd::Params {
                    client: &self.client,
                    ffmpeg,
                    network: &network,
                    settings: &self.ffmpeg_settings,
                    chunk: self.http_chunk_size,
                    timecode: timecode.as_ref(),
                };
                if let Some(size) = indexed_fd::download(
                    &params,
                    info,
                    destination.as_ref(),
                    &options,
                    &mut callback,
                )? {
                    return Ok(size);
                }
            }
            return ffmpeg_fd::download(
                ffmpeg,
                &network,
                &self.ffmpeg_settings,
                info,
                None,
                timecode.as_ref(),
                destination.as_ref(),
                &options,
                &mut callback,
            );
        }
        if let Some(external) = self.external_downloader(info, destination.as_ref()) {
            let params = external_fd::Params {
                settings: &self.external_settings,
                cookies: &self.cookies,
                proxy: self.proxy.as_deref(),
                options: &options,
            };
            return external_fd::download(
                &external,
                &params,
                info,
                destination.as_ref(),
                &mut callback,
            );
        }
        // Upstream `PROTOCOL_MAP`'s RtmpFD and WebSocketFragmentFD.
        if info.requested_formats.is_empty()
            && matches!(info.protocol.as_deref(), Some("rtmp" | "rtmpe"))
        {
            let params = rtmp_fd::Params {
                options: &options,
                continue_download: self.external_settings.continue_download,
                test: self.test,
            };
            return rtmp_fd::download(&params, info, destination.as_ref(), &mut callback);
        }
        if info.requested_formats.is_empty() && info.protocol.as_deref() == Some("websocket_frag") {
            let network = ffmpeg_fd::Network {
                headers: &self.headers,
                cookies: &self.cookies,
                proxy: self.proxy.as_deref(),
            };
            // `_calc_headers`: the defaults updated by the format's own.
            let mut headers = self.info_headers.clone();
            for (name, value) in external_fd::http_headers(info) {
                let name = ffmpeg_fd::py_title(&name);
                match headers.iter_mut().find(|(existing, _)| *existing == name) {
                    Some(entry) => entry.1 = value,
                    None => headers.push((name, value)),
                }
            }
            return websocket_fd::download(
                self.ffmpeg.as_deref(),
                headers,
                &network,
                &self.ffmpeg_settings,
                info,
                destination.as_ref(),
                &options,
                &mut callback,
            );
        }
        // Upstream `DashSegmentsFD` with a fragment generator.
        if info.requested_formats.is_empty()
            && info.protocol.as_deref() == Some("http_dash_segments_generator")
        {
            let bytes = self.download_live_streams(
                info,
                &[(info.clone(), destination.as_ref().to_path_buf())],
                options,
                false,
                &mut callback,
            )?;
            return Ok(bytes.iter().sum());
        }
        if !info.requested_formats.is_empty() {
            return self.download_merged(info, destination.as_ref(), options, &mut callback);
        }
        // Upstream `IsmFD` (`--test` takes the first fragment).
        if info.protocol.as_deref() == Some("ism") {
            let params = info
                .extra
                .get("_download_params")
                .cloned()
                .ok_or_else(|| Error::Reported("KeyError: '_download_params'".into()))?;
            let mut plan = info.clone();
            if self.test {
                plan.fragments.truncate(1);
            }
            return fragment::download_ism(
                &self.client,
                &plan,
                destination.as_ref(),
                options,
                self.keep_fragments,
                &params,
                &mut callback,
            );
        }
        // Upstream `F4mFD`.
        if info.protocol.as_deref() == Some("f4m") {
            return f4m_fd::download(
                &self.client,
                info,
                destination.as_ref(),
                options,
                self.keep_fragments,
                self.test,
                &mut callback,
            );
        }
        // Upstream `MhtmlFD` (storyboards; `--test` takes the first fragment).
        if info.protocol.as_deref() == Some("mhtml") {
            let mut plan = info.clone();
            if self.test {
                plan.fragments.truncate(1);
            }
            return fragment::download_mhtml(
                &self.client,
                &plan,
                destination.as_ref(),
                options,
                self.keep_fragments,
                &mut callback,
            );
        }
        // A loaded HLS format reads its fragments from the playlist now.
        let refreshed = if info.fragments.is_empty() {
            self.refresh_hls(info)?
        } else {
            None
        };
        let info = refreshed.as_ref().unwrap_or(info);
        if !info.fragments.is_empty() {
            return fragment::download(
                &self.client,
                info,
                destination.as_ref(),
                options,
                self.concurrent_fragments,
                self.keep_fragments,
                &mut callback,
            );
        }
        // Upstream `HttpFD`: `_TEST_FILE_SIZE` for `--test`, else the
        // `http_chunk_size` parameter, else the format's downloader options.
        let chunk = if self.test {
            Some(download::Chunking {
                size: download::TEST_FILE_SIZE,
                test: true,
            })
        } else {
            self.http_chunk_size
                .or_else(|| info.downloader_options.as_ref()?.http_chunk_size)
                .filter(|size| *size > 0)
                .map(|size| download::Chunking { size, test: false })
        };
        if info.url.scheme() == "file" {
            return download::download_file(info, destination.as_ref(), options, &mut callback);
        }
        download::download(
            &self.client,
            info,
            destination.as_ref(),
            options,
            chunk,
            &mut callback,
        )
    }
    /// Copy a selected HTTP subtitle resource using the native whole-file transport.
    /// Caption bytes are not parsed or converted; the page referrer is preserved.
    pub fn download_subtitle_with_options(
        &self,
        info: &VideoInfo,
        subtitle: &SubtitleFormat,
        destination: impl AsRef<Path>,
        options: DownloadOptions,
    ) -> Result<u64> {
        self.download_subtitle_with_progress(info, subtitle, destination, options, |_| {
            std::ops::ControlFlow::Continue(())
        })
    }

    /// Observe/cancel a subtitle transfer at the same checkpoints as whole-file media.
    pub fn download_subtitle_with_progress(
        &self,
        info: &VideoInfo,
        subtitle: &SubtitleFormat,
        destination: impl AsRef<Path>,
        options: DownloadOptions,
        mut callback: impl FnMut(DownloadProgress) -> std::ops::ControlFlow<()>,
    ) -> Result<u64> {
        if subtitle.protocol.as_deref() == Some("m3u8_native") {
            report_invoking("hlsnative", &[&subtitle.url]);
            return download::download_hls_subtitle(
                &self.client,
                info,
                subtitle,
                destination.as_ref(),
                options,
                &mut callback,
            );
        }
        if let Some(protocol @ ("youtube_live_chat" | "youtube_live_chat_replay")) =
            subtitle.protocol.as_deref()
        {
            report_invoking("youtube_live_chat", &[&subtitle.url]);
            return extractor::youtube::live_chat::download(
                &self.client,
                protocol == "youtube_live_chat",
                subtitle,
                destination.as_ref(),
                options,
                &mut callback,
            );
        }
        if subtitle.protocol.as_deref() == Some("http_dash_segments") {
            report_invoking("dashsegments", &[&subtitle.url]);
            return download::download_dash_subtitle(
                &self.client,
                info,
                subtitle,
                destination.as_ref(),
                options,
                &mut callback,
            );
        }
        report_invoking("http", &[&subtitle.url]);
        download::download_subtitle(
            &self.client,
            info,
            subtitle,
            destination.as_ref(),
            options,
            &mut callback,
        )
    }

    /// Convert a local thumbnail image with the explicit FFmpeg (upstream
    /// `FFmpegThumbnailsConvertorPP.convert_thumbnail`).
    pub fn convert_thumbnail(&self, input: &Path, output: &Path, target: &str) -> Result<()> {
        let ffmpeg = self.ffmpeg.as_ref().ok_or_else(|| {
            Error::Postprocess(
                "thumbnail conversion requires an explicit --ffmpeg-location PATH".into(),
            )
        })?;
        thumbnail_process::convert(ffmpeg, input, output, target, &mut || {
            std::ops::ControlFlow::Continue(())
        })
    }

    /// Copy a thumbnail URL to `destination` without overwriting it.
    pub fn download_thumbnail(
        &self,
        url: &str,
        destination: impl AsRef<Path>,
        options: DownloadOptions,
    ) -> Result<u64> {
        download::download_thumbnail(
            &self.client,
            &Url::parse(url)?,
            destination.as_ref(),
            options,
        )
    }

    fn download_merged(
        &self,
        info: &VideoInfo,
        destination: &Path,
        options: DownloadOptions,
        callback: &mut dyn FnMut(DownloadProgress) -> std::ops::ControlFlow<()>,
    ) -> Result<u64> {
        use std::{fs, ops::ControlFlow};
        download::check_destination(destination, &options)?;
        let streams = &info.requested_formats;
        if streams.len() < 2 || streams.len() > usize::from(u8::MAX) {
            return Err(Error::Postprocess(
                "merging needs two or more selected streams".into(),
            ));
        }
        let count = streams.len() as u8;
        // A video-only plus an audio-only stream is the usual pair; more
        // streams come from `--video-multistreams`/`--audio-multistreams`.
        let video = streams
            .iter()
            .position(|stream| stream.stream_kind == StreamKind::VideoOnly);
        let audio = streams
            .iter()
            .position(|stream| stream.stream_kind == StreamKind::AudioOnly);
        if let (2, Some(video), Some(audio)) = (streams.len(), video, audio)
            && !info::mergeable(&streams[video], &streams[audio])
        {
            return Err(Error::Postprocess("unsupported stream containers".into()));
        }
        // The selected container (upstream `get_compatible_ext`).
        let output_ext = info.ext.as_str();
        let upstream_files = matches!(options.merge_files, download::MergeFiles::Upstream { .. });
        let executable = match &self.ffmpeg {
            Some(executable) => Some(executable),
            // Upstream downloads the formats without merging (after warning).
            None if upstream_files => None,
            None => {
                return Err(Error::Postprocess(
                    "separate streams require --ffmpeg-location PATH or YoutubeDl::with_ffmpeg"
                        .into(),
                ));
            }
        };
        let executable = executable
            .map(|executable| executable.canonicalize())
            .transpose()
            .map_err(|error| {
                Error::Postprocess(format!("invalid ffmpeg executable path: {error}"))
            })?;
        if executable
            .as_ref()
            .is_some_and(|executable| !executable.is_file())
        {
            return Err(Error::Postprocess(
                "ffmpeg executable path is not a file".into(),
            ));
        }
        for stream in streams {
            if !matches!(stream.url.scheme(), "http" | "https") {
                return Err(Error::UnsupportedScheme(stream.url.scheme().into()));
            }
        }
        let parent = destination
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent)?;
        let workspace = tempfile::Builder::new()
            .prefix(".yt-dlp-rs-merge-")
            .tempdir_in(parent)?;
        let upstream = match options.merge_files {
            download::MergeFiles::Private => None,
            download::MergeFiles::Upstream { keep } => Some(keep),
        };
        let mut paths = Vec::new();
        let mut stored = 0u64;
        // A library `Interrupt::stop` during a stream: the streams so far
        // (the last one partial) are merged and published.
        let mut stopped = false;
        // `get_suitable_downloader`: generated (`--live-from-start`) formats
        // download together through DashSegmentsFD, into files named with
        // the merged extension.
        let live = streams
            .iter()
            .all(|stream| stream.determine_protocol() == "http_dash_segments_generator");
        if live {
            let jobs: Vec<(VideoInfo, PathBuf)> = streams
                .iter()
                .enumerate()
                .map(|(index, stream)| {
                    let path = match upstream {
                        None => workspace
                            .path()
                            .join(format!("input-{index}.{}", stream.ext)),
                        Some(_) => {
                            let mut named = stream.clone();
                            named.ext = info.ext.clone();
                            stream_path(destination, &info.ext, &named)
                        }
                    };
                    (info.with_selected_format(stream), path)
                })
                .collect();
            let sizes = self.download_live_streams(
                info,
                &jobs,
                DownloadOptions {
                    part_files: if upstream.is_some() {
                        options.part_files
                    } else {
                        PartFiles::Private
                    },
                    ..options
                },
                true,
                callback,
            )?;
            stored = sizes.iter().sum();
            paths = jobs.into_iter().map(|(_, path)| path).collect();
        }
        // `--keep-partial-on-stop` fetches audio-only streams first, so a stop
        // during the (larger) video stream still has the whole audio.
        let mut order: Vec<usize> = (0..streams.len()).collect();
        if self.keep_partial_on_stop {
            let audio_only = |stream: &VideoFormat| {
                stream.stream_kind == StreamKind::AudioOnly
                    || (stream.vcodec.as_deref() == Some("none")
                        && stream
                            .acodec
                            .as_deref()
                            .is_some_and(|codec| codec != "none"))
            };
            order.sort_by_key(|&index| !audio_only(&streams[index]));
        }
        // The streams behind `paths`, in download order until sorted below.
        let mut downloaded: Vec<usize> = if live {
            (0..streams.len()).collect()
        } else {
            Vec::new()
        };
        for (index, stream) in order
            .iter()
            .map(|&index| (index, &streams[index]))
            .filter(|_| !live)
        {
            let path = match upstream {
                None => workspace
                    .path()
                    .join(format!("input-{index}.{}", stream.ext)),
                Some(_) => stream_path(destination, &info.ext, stream),
            };
            if upstream.is_some() && path.exists() {
                // Upstream FileDownloader.download reuses a complete file.
                screen::screen(&format!(
                    "[download] {} has already been downloaded",
                    path.display()
                ));
                let bytes = fs::metadata(&path)?.len();
                stored = stored.saturating_add(bytes);
                paths.push(path);
                downloaded.push(index);
                if callback(DownloadProgress {
                    attempt: 1,
                    downloaded_bytes: bytes,
                    total_bytes: Some(bytes),
                    total_bytes_estimate: None,
                    phase: DownloadPhase::Publishing,
                    stream_index: Some(index as u8 + 1),
                    stream_count: Some(count),
                    fragment_index: None,
                    fragment_count: None,
                    fragment: None,
                    recorded_ms: None,
                })
                .is_break()
                {
                    return Err(Error::DownloadCancelled);
                }
                continue;
            }
            let mut part = info.clone();
            part.fragments = stream.fragments.clone();
            part.requested_formats.clear();
            part.formats.clear();
            part.url = stream.url.clone();
            part.ext = stream.ext.clone();
            part.protocol = Some(stream.determine_protocol());
            part.stream_kind = stream.stream_kind;
            part.format_id = Some(stream.format_id.clone());
            // IsmFD reads the stream's `_download_params`.
            match stream.extra.get("_download_params") {
                Some(params) => {
                    part.extra.insert("_download_params".into(), params.clone());
                }
                None => {
                    part.extra.shift_remove("_download_params");
                }
            }
            match stream.format_index {
                Some(index) => {
                    part.extra.insert("format_index".into(), index.into());
                }
                None => {
                    part.extra.shift_remove("format_index");
                }
            }
            // `new_info.update(fmt)`: the format's request headers.
            match &stream.http_headers {
                Some(headers) => {
                    part.extra.insert(
                        "http_headers".into(),
                        serde_json::Value::Object(headers.clone()),
                    );
                }
                None => {
                    part.extra.shift_remove("http_headers");
                }
            }
            // `HttpFD` chunking (YouTube's anti-throttling `http_chunk_size`).
            part.downloader_options = stream.downloader_options.clone();
            part.filesize = stream.filesize;
            part.width = stream.width;
            part.height = stream.height;
            if upstream.is_some() {
                self.announce_download(&part);
                // Upstream `HlsFD` refetches each stream's media playlist.
                if let Some(refreshed) = self.refresh_hls(&part)? {
                    part = refreshed;
                }
                if !part.fragments.is_empty() {
                    let downloader = match part.protocol.as_deref() {
                        Some("http_dash_segments") => "dashsegments",
                        Some("ism") => "ism",
                        Some("mhtml") => "mhtml",
                        _ => "hlsnative",
                    };
                    screen::screen(&format!(
                        "[{downloader}] Total fragments: {}",
                        total_fragments(&part)
                    ));
                }
            }
            // `HttpFD` names the destination once the connection is
            // established; fragment downloaders before their first fragment.
            let announce = std::cell::Cell::new(upstream.is_some());
            if !part.fragments.is_empty() && announce.replace(false) {
                screen::screen(&format!("[download] Destination: {}", path.display()));
            }
            let bytes = self.download_with_progress(
                &part,
                &path,
                DownloadOptions {
                    part_files: if upstream.is_some() {
                        options.part_files
                    } else {
                        PartFiles::Private
                    },
                    ..options
                },
                |mut progress: DownloadProgress| {
                    if !matches!(
                        progress.phase,
                        DownloadPhase::Requesting | DownloadPhase::WaitingToRetry
                    ) && announce.replace(false)
                    {
                        screen::screen(&format!("[download] Destination: {}", path.display()));
                    }
                    // Private inputs are not published; upstream-style ones are.
                    if progress.phase == DownloadPhase::Publishing && upstream.is_none() {
                        return ControlFlow::Continue(());
                    }
                    progress.stream_index = Some(index as u8 + 1);
                    progress.stream_count = Some(count);
                    callback(progress)
                },
            );
            let bytes = match bytes {
                Err(Error::DownloadStopped { .. }) => {
                    stored = stored.saturating_add(fs::metadata(&path)?.len());
                    paths.push(path);
                    downloaded.push(index);
                    stopped = true;
                    break;
                }
                // Stopped before this stream had data: the earlier ones.
                Err(Error::DownloadCancelled) if control::stopping() && !paths.is_empty() => {
                    stopped = true;
                    break;
                }
                bytes => bytes?,
            };
            if announce.replace(false) {
                screen::screen(&format!("[download] Destination: {}", path.display()));
            }
            stored = stored
                .checked_add(bytes)
                .ok_or_else(|| Error::Postprocess("combined stream byte count overflow".into()))?;
            paths.push(path);
            downloaded.push(index);
        }
        // Merge inputs follow the selection order.
        let mut pairs: Vec<(usize, PathBuf)> = downloaded.into_iter().zip(paths).collect();
        pairs.sort_by_key(|(index, _)| *index);
        let (downloaded, paths): (Vec<usize>, Vec<PathBuf>) = pairs.into_iter().unzip();
        let Some(executable) = executable else {
            // No merger: the format files are the result.
            return Ok(stored);
        };
        if upstream.is_some() {
            screen::postprocessor("Merger", "started");
            screen::screen(&format!(
                "[Merger] Merging formats into \"{}\"",
                destination.display()
            ));
        }
        let output =
            download::media_tempfile("merged-", ".part", workspace.path())?.into_temp_path();
        let oldest = filetime::oldest(paths.iter().map(PathBuf::as_path));
        // Each input's streams: by its kind, else upstream's codec test.
        let inputs: Vec<merge::Input<'_>> = downloaded
            .iter()
            .map(|&index| &streams[index])
            .zip(&paths)
            .map(|(stream, path)| {
                let (audio, video) = match stream.stream_kind {
                    StreamKind::AudioOnly => (true, false),
                    StreamKind::VideoOnly => (false, true),
                    StreamKind::Standalone => (
                        stream.acodec.as_deref() != Some("none"),
                        stream.vcodec.as_deref() != Some("none"),
                    ),
                };
                merge::Input {
                    path,
                    ext: &stream.ext,
                    audio,
                    video,
                }
            })
            .collect();
        merge::merge_inputs(&executable, &inputs, &output, output_ext, &mut || {
            if stopped {
                return ControlFlow::Continue(());
            }
            callback(DownloadProgress {
                attempt: 1,
                downloaded_bytes: stored,
                total_bytes: Some(stored),
                total_bytes_estimate: None,
                phase: DownloadPhase::Merging,
                stream_index: None,
                stream_count: Some(count),
                fragment_index: None,
                fragment_count: None,
                fragment: None,
                recorded_ms: None,
            })
        })?;
        filetime::inherit(&output, oldest);
        let bytes = fs::metadata(&output)?.len();
        fs::OpenOptions::new()
            .write(true)
            .open(&output)?
            .sync_all()?;
        if !stopped
            && callback(DownloadProgress {
                attempt: 1,
                downloaded_bytes: bytes,
                total_bytes: Some(bytes),
                total_bytes_estimate: None,
                phase: DownloadPhase::Publishing,
                stream_index: None,
                stream_count: Some(count),
                fragment_index: None,
                fragment_count: None,
                fragment: None,
                recorded_ms: None,
            })
            .is_break()
        {
            return Err(Error::DownloadCancelled);
        }
        download::persist_path(output, destination, options.overwrite)?;
        // `run_pp` deletes the inputs after the merger returns.
        if upstream.is_some() {
            screen::postprocessor("Merger", "finished");
        }
        if upstream == Some(false) {
            for path in &paths {
                screen::screen(&format!(
                    "Deleting original file {} (pass -k to keep)",
                    path.display()
                ));
                fs::remove_file(path)?;
            }
        }
        if stopped {
            return Err(Error::DownloadStopped {
                path: destination.to_owned(),
            });
        }
        Ok(bytes)
    }
}

/// Upstream's per-format merge input name: `correct_ext` of the destination
/// for the stream's extension, then `prepend_extension` with `fFORMAT_ID`.
pub fn stream_path(destination: &Path, merged_ext: &str, stream: &VideoFormat) -> PathBuf {
    let name = destination
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && (ext == merged_ext || ext == stream.ext) => {
            stem.to_owned()
        }
        _ => name,
    };
    destination.with_file_name(format!("{stem}.f{}.{}", stream.format_id, stream.ext))
}
