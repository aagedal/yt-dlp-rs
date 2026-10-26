pub mod adobepass;
mod amadeustv;
mod angel;
mod archiveorg;
mod arte;
mod audioboom;
mod bandcamp;
mod beacon;
mod behindkink;
mod bigo;
mod bild;
mod bilibili;
pub(crate) mod bitchute;
mod bitmovin;
mod blogger;
mod bluesky;
mod bongacams;
mod bostonglobe;
mod breitbart;
pub(crate) mod brightcove;
mod bundesliga;
mod caltrans;
mod cam4;
mod ccc;
mod cgtn;
mod cjsw;
mod clipchamp;
mod cloudflarestream;
mod cloudycdn;
mod clyp;
mod coub;
mod cozytv;
mod crooksandliars;
mod crtvg;
mod cultureunplugged;
mod dailymotion;
mod dash;
mod daystar;
mod democracynow;
mod dfb;
pub(crate) mod direct;
mod dlive;
mod dropbox;
mod drtalks;
mod drtv;
mod dumpert;
mod ebaumsworld;
mod ebay;
mod elonet;
mod epoch;
mod erocast;
mod ettutv;
mod europeantour;
mod f4m;
mod facebook;
mod fathom;
mod fczenit;
mod filmarchiv;
mod filmweb;
mod formula1;
mod freesound;
mod fuyintv;
mod gab;
mod gamestar;
mod generic_urls;
mod glide;
mod godresource;
mod godtube;
mod golem;
mod goodgame;
mod graspop;
mod gronkh;
mod harpodeon;
mod hearthisat;
mod hellporno;
mod historicfilms;
mod hitrecord;
pub(crate) mod hls;
mod hrefli;
mod hse;
pub(crate) mod html5;
mod huajiao;
mod hypem;
mod iheart;
mod ilpost;
mod iltalehti;
mod imgur;
mod ina;
mod inc;
mod instagram;
mod internazionale;
pub(crate) mod ism;
mod israelnationalnews;
mod ivoox;
mod jove;
mod json_ld;
mod jwplatform;
mod jwplayer;
mod kaltura;
mod kankanews;
mod kick;
mod kicker;
mod kickstarter;
mod kommunetv;
mod kth;
mod ku6;
mod kvs;
mod lbry;
mod libsyn;
mod linkedin;
mod livestreamfails;
mod lnk;
pub mod login;
mod loom;
mod maariv;
mod maoritv;
mod masters;
mod matchitv;
mod matchtv;
mod mediaite;
mod megaphone;
mod mir24tv;
mod mixcloud;
mod mocha;
mod movingimage;
mod mux;
mod myvidster;
mod netzkino;
mod newsy;
mod niconico;
mod ninenews;
mod nobelprize;
mod nrk;
mod nts;
pub mod nuxt;
mod nzonscreen;
mod odnoklassniki;
mod onefootball;
mod onenewsnz;
mod oneplace;
mod parlview;
mod pearvideo;
mod peertube;
mod peertv;
mod periscope;
mod photobucket;
mod pinkbike;
mod pinterest;
mod playerfm;
mod podbayfm;
mod popcorntv;
mod premiershiprugby;
mod presstv;
mod projectveritas;
mod qingting;
mod radiozet;
mod reddit;
mod reverbnation;
mod rinsefm;
mod rottentomatoes;
mod roya;
mod rss;
mod rtrfm;
mod rumble;
mod rutube;
mod sapo;
mod screen9;
mod screenrec;
mod sen;
mod skylinewebcams;
mod skynewsau;
mod smil;
mod snapchat;
mod soundcloud;
mod soundgasm;
mod spiegel;
mod streamable;
mod streetvoice;
mod svt;
mod sztvhu;
mod ted;
mod telegraaf;
mod telegram;
mod teletask;
mod theguardian;
mod thehighwire;
mod theintercept;
mod thestar;
mod thesun;
mod thisamericanlife;
mod threespeak;
mod tiktok;
mod toypics;
mod trtcocuk;
mod trtworld;
mod tumblr;
mod tvanouvelles;
mod tvnoe;
mod tweakers;
mod twentythreevideo;
mod twitch;
mod twitter;
mod umg;
mod unistra;
mod unitednations;
mod ustudio;
mod veo;
mod videopress;
pub(crate) mod vidyard;
mod vimeo;
mod vk;
mod vkplay;
mod vocaroo;
mod webcamerapl;
mod weibo;
mod whyp;
mod wimbledon;
mod wistia;
mod worldstarhiphop;
mod xboxclips;
mod xiaohongshu;
mod xspf;
mod yfanefa;
pub(crate) mod youtube;
mod zhihu;

pub use amadeustv::{AmadeusTv, suitable as amadeustv_suitable};
pub use angel::{Angel, suitable as angel_suitable};
pub use archiveorg::{ArchiveOrg, suitable as archiveorg_suitable};
pub use arte::{
    ArteTv, ArteTvCategory, ArteTvEmbed, ArteTvPlaylist,
    category_suitable as artetv_category_suitable, embed_suitable as artetv_embed_suitable,
    playlist_suitable as artetv_playlist_suitable, suitable as artetv_suitable,
};
pub use audioboom::{AudioBoom, suitable as audioboom_suitable};
pub use bandcamp::{Bandcamp, suitable as bandcamp_suitable};
pub use beacon::{BeaconTv, suitable as beacon_suitable};
pub use behindkink::{BehindKink, suitable as behindkink_suitable};
pub use bigo::{Bigo, suitable as bigo_suitable};
pub use bild::{Bild, suitable as bild_suitable};
pub use bilibili::{BiliBili, suitable as bilibili_suitable};
pub use bitchute::{BitChute, suitable as bitchute_suitable};
pub use bitmovin::{Bitmovin, suitable as bitmovin_suitable};
pub use blogger::{Blogger, suitable as blogger_suitable};
pub use bluesky::{Bluesky, suitable as bluesky_suitable};
pub use bongacams::{BongaCams, suitable as bongacams_suitable};
pub use bostonglobe::{BostonGlobe, suitable as bostonglobe_suitable};
pub use breitbart::{BreitBart, suitable as breitbart_suitable};
pub use brightcove::{BrightcoveNew, suitable as brightcove_new_suitable};
pub use bundesliga::{Bundesliga, suitable as bundesliga_suitable};
pub use caltrans::{Caltrans, suitable as caltrans_suitable};
pub use cam4::{Cam4, suitable as cam4_suitable};
pub use ccc::{Ccc, suitable as ccc_suitable};
pub use cgtn::{Cgtn, suitable as cgtn_suitable};
pub use cjsw::{Cjsw, suitable as cjsw_suitable};
pub use clipchamp::{Clipchamp, suitable as clipchamp_suitable};
pub use cloudflarestream::{CloudflareStream, suitable as cloudflarestream_suitable};
pub use cloudycdn::{CloudyCdn, suitable as cloudycdn_suitable};
pub use clyp::{Clyp, suitable as clyp_suitable};
pub use coub::{Coub, suitable as coub_suitable};
pub use cozytv::{CozyTv, suitable as cozytv_suitable};
pub use crooksandliars::{CrooksAndLiars, suitable as crooksandliars_suitable};
pub use crtvg::{Crtvg, suitable as crtvg_suitable};
pub use cultureunplugged::{CultureUnplugged, suitable as cultureunplugged_suitable};
pub use dailymotion::{Dailymotion, suitable as dailymotion_suitable};
pub use daystar::{DaystarClip, suitable as daystar_suitable};
pub use democracynow::{Democracynow, suitable as democracynow_suitable};
pub use dfb::{Dfb, suitable as dfb_suitable};
pub use direct::DirectHttp;
pub(crate) use direct::from_response as direct_media_info;
pub use dlive::{
    DLiveStream, DLiveVod, stream_suitable as dlive_stream_suitable,
    vod_suitable as dlive_vod_suitable,
};
pub use dropbox::{Dropbox, suitable as dropbox_suitable};
pub use drtalks::{DrTalks, suitable as drtalks_suitable};
pub use drtv::{
    Drtv, DrtvLive, DrtvSeason, DrtvSeries, live_suitable as drtv_live_suitable,
    season_suitable as drtv_season_suitable, series_suitable as drtv_series_suitable,
    suitable as drtv_suitable,
};
pub use dumpert::{Dumpert, suitable as dumpert_suitable};
pub use ebaumsworld::{EbaumsWorld, suitable as ebaumsworld_suitable};
pub use ebay::{Ebay, suitable as ebay_suitable};
pub use elonet::{Elonet, suitable as elonet_suitable};
pub use epoch::{Epoch, suitable as epoch_suitable};
pub use erocast::{Erocast, suitable as erocast_suitable};
pub use ettutv::{EttuTv, suitable as ettutv_suitable};
pub use europeantour::{EuropeanTour, suitable as europeantour_suitable};
pub(crate) use f4m::fix_ampersands as fix_xml_ampersands;
pub use facebook::{
    Facebook, FacebookAds, FacebookPluginsVideo, FacebookRedirectUrl, FacebookReel,
    ads_suitable as facebook_ads_suitable, plugins_suitable as facebook_plugins_suitable,
    redirect_suitable as facebook_redirect_suitable, reel_suitable as facebook_reel_suitable,
    suitable as facebook_suitable,
};
pub use fathom::{Fathom, suitable as fathom_suitable};
pub use fczenit::{Fczenit, suitable as fczenit_suitable};
pub use filmarchiv::{FilmArchiv, suitable as filmarchiv_suitable};
pub use filmweb::{Filmweb, suitable as filmweb_suitable};
pub use formula1::{Formula1, suitable as formula1_suitable};
pub use freesound::{Freesound, suitable as freesound_suitable};
pub use fuyintv::{FuyinTv, suitable as fuyintv_suitable};
pub use gab::{Gab, suitable as gab_suitable};
pub use gamestar::{GameStar, suitable as gamestar_suitable};
pub use glide::{Glide, suitable as glide_suitable};
pub use godresource::{GodResource, suitable as godresource_suitable};
pub use godtube::{GodTube, suitable as godtube_suitable};
pub use golem::{Golem, suitable as golem_suitable};
pub use goodgame::{GoodGame, suitable as goodgame_suitable};
pub use graspop::{Graspop, suitable as graspop_suitable};
pub use gronkh::{Gronkh, suitable as gronkh_suitable};
pub use harpodeon::{Harpodeon, suitable as harpodeon_suitable};
pub use hearthisat::{HearThisAt, suitable as hearthisat_suitable};
pub use hellporno::{HellPorno, suitable as hellporno_suitable};
pub use historicfilms::{HistoricFilms, suitable as historicfilms_suitable};
pub use hitrecord::{HitRecord, suitable as hitrecord_suitable};
pub use hls::set_split_discontinuity as hls_split_discontinuity;
pub use hrefli::{HrefLiRedirect, suitable as hrefli_suitable};
pub use hse::{
    HseProduct, HseShow, product_suitable as hse_product_suitable,
    show_suitable as hse_show_suitable,
};
pub use html5::GenericHttp;
pub use huajiao::{Huajiao, suitable as huajiao_suitable};
pub use hypem::{Hypem, suitable as hypem_suitable};
pub use iheart::{
    IHeartRadio, IHeartRadioPodcast, podcast_suitable as iheart_podcast_suitable,
    suitable as iheart_suitable,
};
pub use ilpost::{IlPost, suitable as ilpost_suitable};
pub use iltalehti::{Iltalehti, suitable as iltalehti_suitable};
pub use imgur::{
    Imgur, ImgurAlbum, ImgurGallery, album_suitable as imgur_album_suitable,
    gallery_suitable as imgur_gallery_suitable, suitable as imgur_suitable,
};
pub use ina::{Ina, suitable as ina_suitable};
pub use inc::{Inc, suitable as inc_suitable};
pub(crate) use instagram::post_extract as instagram_comments;
pub use instagram::{
    Instagram, InstagramIos, InstagramStory, InstagramTag, InstagramUser,
    ios_suitable as instagram_ios_suitable, story_suitable as instagram_story_suitable,
    suitable as instagram_suitable, tag_suitable as instagram_tag_suitable,
    user_suitable as instagram_user_suitable,
};
pub use internazionale::{Internazionale, suitable as internazionale_suitable};
pub use israelnationalnews::{IsraelNationalNews, suitable as israelnationalnews_suitable};
pub use ivoox::{Ivoox, suitable as ivoox_suitable};
pub use jove::{Jove, suitable as jove_suitable};
pub use jwplatform::{JwPlatform, suitable as jwplatform_suitable};
pub use kaltura::{Kaltura, suitable as kaltura_suitable};
pub use kankanews::{KankaNews, suitable as kankanews_suitable};
pub use kick::{
    KickClip, KickClips, KickLive, KickVideos, KickVod, clip_suitable as kick_clip_suitable,
    clips_suitable as kick_clips_suitable, live_suitable as kick_live_suitable,
    videos_suitable as kick_videos_suitable, vod_suitable as kick_vod_suitable,
};
pub use kicker::{Kicker, suitable as kicker_suitable};
pub use kickstarter::{KickStarter, suitable as kickstarter_suitable};
pub use kommunetv::{Kommunetv, suitable as kommunetv_suitable};
pub use kth::{Kth, suitable as kth_suitable};
pub use ku6::{Ku6, suitable as ku6_suitable};
pub use lbry::{Lbry, suitable as lbry_suitable};
pub use libsyn::{Libsyn, suitable as libsyn_suitable};
pub use linkedin::{LinkedIn, suitable as linkedin_suitable};
pub use livestreamfails::{Livestreamfails, suitable as livestreamfails_suitable};
pub use lnk::{Lnk, suitable as lnk_suitable};
pub use loom::{Loom, suitable as loom_suitable};
pub use maariv::{Maariv, suitable as maariv_suitable};
pub use maoritv::{MaoriTV, suitable as maoritv_suitable};
pub use masters::{Masters, suitable as masters_suitable};
pub use matchitv::{MatchiTv, suitable as matchitv_suitable};
pub use matchtv::{MatchTv, suitable as matchtv_suitable};
pub use mediaite::{Mediaite, suitable as mediaite_suitable};
pub use megaphone::{Megaphone, suitable as megaphone_suitable};
pub use mir24tv::{Mir24Tv, suitable as mir24tv_suitable};
pub use mixcloud::{Mixcloud, suitable as mixcloud_suitable};
pub use mocha::{MochaVideo, suitable as mocha_suitable};
pub use movingimage::{MovingImage, suitable as movingimage_suitable};
pub use mux::{Mux, suitable as mux_suitable};
pub use myvidster::{MyVidster, suitable as myvidster_suitable};
pub use netzkino::{Netzkino, suitable as netzkino_suitable};
pub use newsy::{Newsy, suitable as newsy_suitable};
pub use niconico::{Niconico, suitable as niconico_suitable};
pub use ninenews::{NineNews, suitable as ninenews_suitable};
pub use nobelprize::{NobelPrize, suitable as nobelprize_suitable};
pub use nrk::{
    Nrk, NrkPlaylist, NrkRadioPodkast, NrkTv, NrkTvDirekte, NrkTvEpisode, NrkTvSeason, NrkTvSeries,
    direkte_suitable as nrk_tv_direkte_suitable, episode_suitable as nrk_tv_episode_suitable,
    playlist_suitable as nrk_playlist_suitable, podkast_suitable as nrk_radio_podkast_suitable,
    season_suitable as nrk_tv_season_suitable, series_suitable as nrk_tv_series_suitable,
    suitable as nrk_suitable, tv_suitable as nrk_tv_suitable,
};
pub use nts::{NtsLive, suitable as nts_suitable};
pub use nzonscreen::{NzOnScreen, suitable as nzonscreen_suitable};
pub use odnoklassniki::{Odnoklassniki, suitable as odnoklassniki_suitable};
pub use onefootball::{OneFootball, suitable as onefootball_suitable};
pub use onenewsnz::{OneNewsNz, suitable as onenewsnz_suitable};
pub use oneplace::{OnePlacePodcast, suitable as oneplace_suitable};
pub use parlview::{Parlview, suitable as parlview_suitable};
pub use pearvideo::{PearVideo, suitable as pearvideo_suitable};
pub use peertube::{
    PeerTube, PeerTubePlaylist, playlist_suitable as peertube_playlist_suitable,
    suitable as peertube_suitable,
};
pub use peertv::{PeerTv, suitable as peertv_suitable};
pub use periscope::{Periscope, suitable as periscope_suitable};
pub use photobucket::{Photobucket, suitable as photobucket_suitable};
pub use pinkbike::{Pinkbike, suitable as pinkbike_suitable};
pub use pinterest::{Pinterest, suitable as pinterest_suitable};
pub use playerfm::{PlayerFm, suitable as playerfm_suitable};
pub use podbayfm::{PodbayFm, suitable as podbayfm_suitable};
pub use podbayfm::{PodbayFmChannel, channel_suitable as podbayfm_channel_suitable};
pub use popcorntv::{PopcornTv, suitable as popcorntv_suitable};
pub use premiershiprugby::{PremiershipRugby, suitable as premiershiprugby_suitable};
pub use presstv::{PressTv, suitable as presstv_suitable};
pub use projectveritas::{ProjectVeritas, suitable as projectveritas_suitable};
pub use qingting::{QingTing, suitable as qingting_suitable};
pub use radiozet::{RadioZetPodcast, suitable as radiozet_suitable};
pub use reddit::{Reddit, suitable as reddit_suitable};
pub use reverbnation::{ReverbNation, suitable as reverbnation_suitable};
pub use rinsefm::{RinseFm, suitable as rinsefm_suitable};
pub use rinsefm::{RinseFmArtistPlaylist, playlist_suitable as rinsefm_playlist_suitable};
pub use rottentomatoes::{RottenTomatoes, suitable as rottentomatoes_suitable};
pub use roya::{RoyaLive, suitable as roya_suitable};
pub use rtrfm::{Rtrfm, suitable as rtrfm_suitable};
pub use rumble::{
    Rumble, RumbleEmbed, embed_suitable as rumble_embed_suitable, suitable as rumble_suitable,
};
pub use rutube::{
    Rutube, RutubeEmbed, embed_suitable as rutube_embed_suitable, suitable as rutube_suitable,
};
pub use sapo::{Sapo, suitable as sapo_suitable};
pub use screen9::{Screen9, suitable as screen9_suitable};
pub use screenrec::{ScreenRec, suitable as screenrec_suitable};
pub use sen::{Sen, suitable as sen_suitable};
pub use skylinewebcams::{SkylineWebcams, suitable as skylinewebcams_suitable};
pub use skynewsau::{SkyNewsAU, suitable as skynewsau_suitable};
pub use snapchat::{SnapchatSpotlight, suitable as snapchat_suitable};
pub use soundcloud::{Soundcloud, suitable as soundcloud_suitable};
pub use soundgasm::{
    Soundgasm, SoundgasmProfile, profile_suitable as soundgasm_profile_suitable,
    suitable as soundgasm_suitable,
};
pub use spiegel::{Spiegel, suitable as spiegel_suitable};
pub use streamable::{Streamable, suitable as streamable_suitable};
pub use streetvoice::{StreetVoice, suitable as streetvoice_suitable};
pub use svt::{
    SvtPage, SvtPlay, SvtSeries, page_suitable as svt_page_suitable,
    series_suitable as svt_series_suitable, suitable as svt_suitable,
};
pub use sztvhu::{SztvHu, suitable as sztvhu_suitable};
pub use ted::{
    TedEmbed, TedPlaylist, TedSeries, TedTalk, embed_suitable as ted_embed_suitable,
    playlist_suitable as ted_playlist_suitable, series_suitable as ted_series_suitable,
    talk_suitable as ted_talk_suitable,
};
pub use telegraaf::{Telegraaf, suitable as telegraaf_suitable};
pub use telegram::{TelegramEmbed, suitable as telegram_suitable};
pub use teletask::{TeleTask, suitable as teletask_suitable};
pub use theguardian::{TheGuardianPodcast, suitable as theguardian_suitable};
pub use thehighwire::{TheHighWire, suitable as thehighwire_suitable};
pub use theintercept::{TheIntercept, suitable as theintercept_suitable};
pub use thestar::{TheStar, suitable as thestar_suitable};
pub use thesun::{TheSun, suitable as thesun_suitable};
pub use thisamericanlife::{ThisAmericanLife, suitable as thisamericanlife_suitable};
pub use threespeak::{ThreeSpeak, suitable as threespeak_suitable};
pub use threespeak::{ThreeSpeakUser, user_suitable as threespeak_user_suitable};
pub use tiktok::{
    Douyin, TikTok, TikTokCollection, TikTokLive, TikTokUser, TikTokVm,
    collection_suitable as tiktok_collection_suitable, douyin_suitable,
    live_suitable as tiktok_live_suitable, suitable as tiktok_suitable,
    user_suitable as tiktok_user_suitable, vm_suitable as tiktok_vm_suitable,
};
pub(crate) use tiktok::{
    fetch_pages as fetch_tiktok_pages, is_pending_state as tiktok_pending_state,
};
pub use toypics::{Toypics, suitable as toypics_suitable};
pub use trtcocuk::{TrtCocukVideo, suitable as trtcocuk_suitable};
pub use trtworld::{TrtWorld, suitable as trtworld_suitable};
pub use tumblr::{Tumblr, suitable as tumblr_suitable};
pub use tvanouvelles::{
    TvaNouvelles, TvaNouvellesArticle, article_suitable as tvanouvelles_article_suitable,
    suitable as tvanouvelles_suitable,
};
pub use tvnoe::{TvNoe, suitable as tvnoe_suitable};
pub use tweakers::{Tweakers, suitable as tweakers_suitable};
pub use twentythreevideo::{TwentyThreeVideo, suitable as twentythreevideo_suitable};
pub(crate) use twitch::{
    LIVE_FROM_START as TWITCH_LIVE_FROM_START, fetch_pages as fetch_twitch_pages,
};
pub use twitch::{
    TwitchClips, TwitchCollection, TwitchStream, TwitchVideos, TwitchVideosClips,
    TwitchVideosCollections, TwitchVod, clips_suitable as twitch_clips_suitable,
    collection_suitable as twitch_collection_suitable, stream_suitable as twitch_stream_suitable,
    videos_clips_suitable as twitch_videos_clips_suitable,
    videos_collections_suitable as twitch_videos_collections_suitable,
    videos_suitable as twitch_videos_suitable, vod_suitable as twitch_vod_suitable,
};
pub use twitter::{
    Twitter, TwitterAmplify, TwitterBroadcast, TwitterCard, TwitterShortener, TwitterSpaces,
    amplify_suitable as twitter_amplify_suitable, broadcast_suitable as twitter_broadcast_suitable,
    card_suitable as twitter_card_suitable, shortener_suitable as twitter_shortener_suitable,
    spaces_suitable as twitter_spaces_suitable, suitable as twitter_suitable,
};
pub use umg::{UmgDe, suitable as umg_suitable};
pub use unistra::{Unistra, suitable as unistra_suitable};
pub use unitednations::{UnitedNationsWebTv, suitable as unitednations_suitable};
pub use ustudio::{Ustudio, suitable as ustudio_suitable};
pub use ustudio::{UstudioEmbed, embed_suitable as ustudio_embed_suitable};
pub use veo::{Veo, suitable as veo_suitable};
pub use videopress::{VideoPress, suitable as videopress_suitable};
pub use vidyard::{Vidyard, suitable as vidyard_suitable};
pub use vimeo::{Vimeo, suitable as vimeo_suitable};
pub use vk::{
    Vk, VkUserVideos, VkWallPost, suitable as vk_suitable,
    user_videos_suitable as vk_user_videos_suitable, wall_post_suitable as vk_wall_post_suitable,
};
pub use vkplay::{
    VkPlay, VkPlayLive, live_suitable as vkplay_live_suitable, suitable as vkplay_suitable,
};
pub use vocaroo::{Vocaroo, suitable as vocaroo_suitable};
pub use webcamerapl::{Webcamerapl, suitable as webcamerapl_suitable};
pub use weibo::{
    Weibo, WeiboVideo, suitable as weibo_suitable, video_suitable as weibo_video_suitable,
};
pub use whyp::{Whyp, suitable as whyp_suitable};
pub use wimbledon::{Wimbledon, suitable as wimbledon_suitable};
pub use wistia::{
    Wistia, WistiaChannel, WistiaPlaylist, channel_suitable as wistia_channel_suitable,
    playlist_suitable as wistia_playlist_suitable, suitable as wistia_suitable,
};
pub use worldstarhiphop::{WorldStarHipHop, suitable as worldstarhiphop_suitable};
pub use xboxclips::{XboxClips, suitable as xboxclips_suitable};
pub use xiaohongshu::{XiaoHongShu, suitable as xiaohongshu_suitable};
pub use yfanefa::{Yfanefa, suitable as yfanefa_suitable};
pub(crate) use youtube::YoutubeUnsupported;
pub(crate) use youtube::clip::YoutubeClip;
pub(crate) use youtube::comments::post_extract as youtube_comments;
pub(crate) use youtube::live::prepare_fragments as youtube_live_fragments;
pub(crate) use youtube::live::{LiveFeed, LiveFragments};
pub(crate) use youtube::notifications::YoutubeNotifications;
pub(crate) use youtube::redirect::{Kind as YoutubeRedirectKind, YoutubeRedirect};
pub(crate) use youtube::search::{YoutubeMusicSearchUrl, YoutubeSearch, YoutubeSearchUrl};
pub use youtube::search::{
    suitable as youtube_search_suitable, url_suitable as youtube_search_url_suitable,
};
pub use youtube::tab::suitable as youtube_tab_suitable;
pub(crate) use youtube::tab::{YoutubeTab, fetch_pages as fetch_youtube_tab_pages};
pub use youtube::{
    JsRuntime, JsRuntimeName, WaitForVideo, Youtube, YoutubeOptions,
    extractor_suitable as youtube_extractor_suitable, js_runtimes, suitable as youtube_suitable,
};
pub use zhihu::{Zhihu, suitable as zhihu_suitable};

use crate::{ExtractResult, Result, VideoInfo};
use reqwest::blocking::Client;
use url::Url;

/// Extractors return None when they do not recognize a URL/response.
/// A result may expose candidate formats; YoutubeDl resolves selection before returning it.
/// Register site extractors before the generic direct-media fallback.
pub trait Extractor: Send + Sync {
    fn key(&self) -> &'static str;
    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>>;

    /// Override to support playlists. Existing video-only extractors need no changes.
    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        Ok(self.extract(client, url)?.map(ExtractResult::Video))
    }

    /// Upstream `_NETRC_MACHINE`: set by extractors supporting password
    /// logins (`--username`, `--netrc`); others warn about `--username`.
    fn netrc_machine(&self) -> Option<&'static str> {
        None
    }

    /// Whether the extractor defines `_perform_login` (only then are the
    /// credentials looked up while initializing).
    fn performs_login(&self) -> bool {
        false
    }

    /// Upstream `_initialize_pre_login`.
    fn initialize_pre_login(&self, _client: &Client, _session: &login::Session) -> Result<()> {
        Ok(())
    }

    /// Upstream `_perform_login(username, password)`.
    fn perform_login(
        &self,
        _client: &Client,
        _session: &login::Session,
        _username: &str,
        _password: &str,
    ) -> Result<()> {
        Ok(())
    }

    /// Upstream `_real_initialize`.
    fn real_initialize(&self, _client: &Client, _session: &login::Session) -> Result<()> {
        Ok(())
    }

    /// Upstream `initialize()` (call it once the URL matched, before the
    /// `Extracting URL` line): the login lifecycle once per run, `ie` being
    /// the extractor's name and `id` the URL's ID for errors.
    fn initialize(&self, client: &Client, ie: &'static str, id: &str) -> Result<()> {
        login::initialize(self, client, ie, id)
    }

    /// Upstream `_GEO_BYPASS`, `_GEO_COUNTRIES` and `_GEO_IP_BLOCKS`.
    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::NONE
    }
}

/// `[EXTRACTOR] ID: Unable to download WHAT: ERR (caused by REPR)`.
pub(crate) fn request_error_text(
    extractor: &str,
    id: &str,
    what: &str,
    error: &crate::Error,
) -> String {
    request_errnote_text(extractor, id, &format!("Unable to download {what}"), error)
}

/// [`request_error_text`] for a request's own `errnote`:
/// `[EXTRACTOR] ID: ERRNOTE: ERR (caused by REPR)`.
pub(crate) fn request_errnote_text(
    extractor: &str,
    id: &str,
    errnote: &str,
    error: &crate::Error,
) -> String {
    let text = crate::upstream_error_text(error);
    let status = match error {
        crate::Error::Http(source) | crate::Error::HttpRetryAfter { source, .. } => source.status(),
        _ => None,
    };
    let cause = match status {
        Some(_) => format!("<{}>", text.replacen("HTTP Error ", "HTTPError ", 1)),
        None => format!("TransportError({})", crate::batch::py_repr(&text)),
    };
    format!("[{extractor}] {id}: {errnote}: {text} (caused by {cause})")
}

/// Upstream `_report_ignoring_subs(NAME)` for `_extract_m3u8_formats` and
/// `_extract_mpd_formats`, which drop a manifest's subtitles.
pub(crate) fn ignore_subtitles(info: &mut crate::VideoInfo, extractor: &str, name: &str) {
    if !info.subtitles.is_empty() {
        crate::screen::warning_once(&format!(
            "[{extractor}] Ignoring subtitle tracks found in the {name} manifest; if any subtitle tracks are missing, {}",
            crate::BUG_REPORTS_MESSAGE
        ));
        info.subtitles = Default::default();
    }
}

/// Upstream fatal `_extract_m3u8_formats_and_subtitles(url, video_id,
/// m3u8_id=M3U8_ID)` (or `_extract_m3u8_formats` without
/// `keep_subtitles`): `[EXTRACTOR] ID: Downloading m3u8 information`, the
/// request (an HTTP error is `Failed to download m3u8 information`), and the
/// playlist's formats and subtitles.
pub(crate) fn extract_m3u8_fatal(
    client: &Client,
    manifest: &Url,
    extractor: &'static str,
    id: &str,
    m3u8_id: Option<&str>,
    keep_subtitles: bool,
) -> crate::Result<crate::VideoInfo> {
    extract_m3u8_fatal_with(
        client,
        manifest,
        extractor,
        id,
        m3u8_id,
        keep_subtitles,
        false,
    )
}

/// [`extract_m3u8_fatal`] with `live=True`: a master's variants are not
/// fetched and keep only the `m3u8_id` (numbered when processed).
pub(crate) fn extract_m3u8_fatal_live(
    client: &Client,
    manifest: &Url,
    extractor: &'static str,
    id: &str,
    m3u8_id: Option<&str>,
    keep_subtitles: bool,
) -> crate::Result<crate::VideoInfo> {
    let mut info = extract_m3u8_fatal_with(
        client,
        manifest,
        extractor,
        id,
        m3u8_id,
        keep_subtitles,
        true,
    )?;
    hls::live_format_ids(&mut info.formats, m3u8_id);
    Ok(info)
}

/// [`extract_m3u8_fatal`] reading only the master playlist, like upstream:
/// the formats' media playlists are fetched when downloading.
pub(crate) fn extract_m3u8_fatal_master(
    client: &Client,
    manifest: &Url,
    extractor: &'static str,
    id: &str,
    m3u8_id: Option<&str>,
    keep_subtitles: bool,
) -> crate::Result<crate::VideoInfo> {
    extract_m3u8_fatal_with(
        client,
        manifest,
        extractor,
        id,
        m3u8_id,
        keep_subtitles,
        true,
    )
}

fn extract_m3u8_fatal_with(
    client: &Client,
    manifest: &Url,
    extractor: &'static str,
    id: &str,
    m3u8_id: Option<&str>,
    keep_subtitles: bool,
    master_only: bool,
) -> crate::Result<crate::VideoInfo> {
    let note = format!("[{extractor}] {id}: Downloading m3u8 information");
    crate::screen::expect_page(id, Some(note.clone()), None);
    let response = direct::request(client, manifest, None);
    if response.is_err() {
        crate::screen::screen(&note);
    }
    let parsed = response
        .map_err(|error| match error {
            crate::Error::Http(_) | crate::Error::HttpRetryAfter { .. } => crate::Error::Reported(
                request_error_text(extractor, id, "m3u8 information", &error).replacen(
                    ": Unable to download ",
                    ": Failed to download ",
                    1,
                ),
            ),
            error => error,
        })
        .and_then(|response| {
            hls::from_response_with(client, response, manifest, None, true, m3u8_id, master_only)
        });
    crate::screen::clear_page();
    let mut info = parsed?.ok_or_else(|| {
        crate::Error::UnsupportedHtml5(format!("the {extractor} stream is not an HLS playlist"))
    })?;
    if !keep_subtitles {
        ignore_subtitles(&mut info, extractor, "HLS");
    }
    Ok(info)
}

/// Upstream `_extract_m3u8_formats_and_subtitles(..., fatal=False)`: the
/// playlist, or `None` after a `Failed to download m3u8 information`
/// warning.
pub(crate) fn extract_m3u8_optional(
    client: &Client,
    manifest: &Url,
    extractor: &'static str,
    id: &str,
    m3u8_id: Option<&str>,
    keep_subtitles: bool,
) -> Option<crate::VideoInfo> {
    extract_manifest_optional(
        client,
        manifest,
        extractor,
        id,
        keep_subtitles,
        &|response| hls::from_response(client, response, manifest, None, true, m3u8_id),
        ("m3u8 information", "HLS"),
    )
}

/// Upstream `_extract_mpd_formats_and_subtitles(..., fatal=False)` (or
/// `_extract_mpd_formats` without `keep_subtitles`), without an `mpd_id`.
pub(crate) fn extract_mpd_optional(
    client: &Client,
    manifest: &Url,
    extractor: &'static str,
    id: &str,
    keep_subtitles: bool,
) -> Option<crate::VideoInfo> {
    extract_manifest_optional(
        client,
        manifest,
        extractor,
        id,
        keep_subtitles,
        &|response| dash::from_response(response, manifest, true),
        ("MPD manifest", "DASH"),
    )
}

/// A non-fatal manifest request: `Downloading WHAT`, else a `Failed to
/// download WHAT` warning.
fn extract_manifest_optional(
    client: &Client,
    manifest: &Url,
    extractor: &'static str,
    id: &str,
    keep_subtitles: bool,
    parse: &dyn Fn(reqwest::blocking::Response) -> crate::Result<Option<crate::VideoInfo>>,
    (what, name): (&str, &str),
) -> Option<crate::VideoInfo> {
    extract_manifest_optional_note(
        client,
        manifest,
        extractor,
        id,
        keep_subtitles,
        parse,
        (what, name),
        &format!("Downloading {what}"),
    )
}

/// [`extract_m3u8_optional`] with its `note=`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn extract_m3u8_optional_note(
    client: &Client,
    manifest: &Url,
    extractor: &'static str,
    id: &str,
    m3u8_id: Option<&str>,
    keep_subtitles: bool,
    live: bool,
    note: &str,
) -> Option<crate::VideoInfo> {
    let mut stream = extract_manifest_optional_note(
        client,
        manifest,
        extractor,
        id,
        keep_subtitles,
        &|response| hls::from_response_with(client, response, manifest, None, true, m3u8_id, live),
        ("m3u8 information", "HLS"),
        note,
    )?;
    if live {
        hls::live_format_ids(&mut stream.formats, m3u8_id);
    }
    Some(stream)
}

/// [`extract_m3u8_optional_note`] reading only the master playlist, like
/// upstream: the formats' media playlists are fetched when downloading.
pub(crate) fn extract_m3u8_master_optional(
    client: &Client,
    manifest: &Url,
    extractor: &'static str,
    id: &str,
    m3u8_id: Option<&str>,
    keep_subtitles: bool,
    live: bool,
) -> Option<crate::VideoInfo> {
    let mut stream = extract_manifest_optional_note(
        client,
        manifest,
        extractor,
        id,
        keep_subtitles,
        &|response| hls::from_response_with(client, response, manifest, None, true, m3u8_id, true),
        ("m3u8 information", "HLS"),
        "Downloading m3u8 information",
    )?;
    if live {
        hls::live_format_ids(&mut stream.formats, m3u8_id);
    }
    Some(stream)
}

/// `_extract_m3u8_formats(url, id, m3u8_id=M3U8_ID, fatal=False)` of a live
/// stream that upstream does not mark `live`: only the master playlist is
/// read (see [`hls::from_response_with`]), and the format IDs keep their
/// bitrates.
pub(crate) fn extract_m3u8_live_master_optional(
    client: &Client,
    manifest: &Url,
    extractor: &'static str,
    id: &str,
    m3u8_id: Option<&str>,
) -> Option<crate::VideoInfo> {
    extract_manifest_optional_note(
        client,
        manifest,
        extractor,
        id,
        false,
        &|response| hls::from_response_with(client, response, manifest, None, true, m3u8_id, true),
        ("m3u8 information", "HLS"),
        "Downloading m3u8 information",
    )
}

/// `_extract_mpd_formats_and_subtitles(url, id, mpd_id, note, fatal=False)`.
pub(crate) fn extract_mpd_optional_note(
    client: &Client,
    manifest: &Url,
    extractor: &'static str,
    id: &str,
    mpd_id: Option<&str>,
    note: &str,
) -> Option<crate::VideoInfo> {
    let mut stream = extract_manifest_optional_note(
        client,
        manifest,
        extractor,
        id,
        true,
        &|response| dash::from_response(response, manifest, true),
        ("MPD manifest", "DASH"),
        note,
    )?;
    if let Some(mpd_id) = mpd_id {
        for format in &mut stream.formats {
            format.format_id = format!("{mpd_id}-{}", format.format_id);
        }
    }
    Some(stream)
}

#[allow(clippy::too_many_arguments)]
fn extract_manifest_optional_note(
    client: &Client,
    manifest: &Url,
    extractor: &'static str,
    id: &str,
    keep_subtitles: bool,
    parse: &dyn Fn(reqwest::blocking::Response) -> crate::Result<Option<crate::VideoInfo>>,
    what: (&str, &str),
    note: &str,
) -> Option<crate::VideoInfo> {
    extract_manifest_optional_note_headers(
        client,
        manifest,
        extractor,
        id,
        keep_subtitles,
        parse,
        what,
        note,
        &[],
    )
}

/// `_extract_m3u8_formats_and_subtitles(url, id, headers=HEADERS,
/// fatal=False)`: only the master playlist is read, the `headers` sent with
/// the manifest request alone.
pub(crate) fn extract_m3u8_master_optional_headers(
    client: &Client,
    manifest: &Url,
    extractor: &'static str,
    id: &str,
    headers: &[(&str, &str)],
) -> Option<crate::VideoInfo> {
    extract_m3u8_master_optional_headers_id(client, manifest, extractor, id, None, headers)
}

/// [`extract_m3u8_master_optional_headers`] with an `m3u8_id`.
pub(crate) fn extract_m3u8_master_optional_headers_id(
    client: &Client,
    manifest: &Url,
    extractor: &'static str,
    id: &str,
    m3u8_id: Option<&str>,
    headers: &[(&str, &str)],
) -> Option<crate::VideoInfo> {
    extract_manifest_optional_note_headers(
        client,
        manifest,
        extractor,
        id,
        true,
        &|response| hls::from_response_with(client, response, manifest, None, true, m3u8_id, true),
        ("m3u8 information", "HLS"),
        "Downloading m3u8 information",
        headers,
    )
}

/// `_extract_m3u8_formats(url, id, live=True, headers=HEADERS,
/// fatal=False)`: only the master playlist is read, its variants keep no
/// bitrate IDs, and the manifest's subtitles are dropped with upstream's
/// warning; the `headers` are sent with the manifest request alone.
pub(crate) fn extract_m3u8_live_optional_headers(
    client: &Client,
    manifest: &Url,
    extractor: &'static str,
    id: &str,
    headers: &[(&str, &str)],
) -> Option<crate::VideoInfo> {
    let mut stream = extract_manifest_optional_note_headers(
        client,
        manifest,
        extractor,
        id,
        false,
        &|response| hls::from_response_with(client, response, manifest, None, true, None, true),
        ("m3u8 information", "HLS"),
        "Downloading m3u8 information",
        headers,
    )?;
    hls::live_format_ids(&mut stream.formats, None);
    Some(stream)
}

#[allow(clippy::too_many_arguments)]
fn extract_manifest_optional_note_headers(
    client: &Client,
    manifest: &Url,
    extractor: &'static str,
    id: &str,
    keep_subtitles: bool,
    parse: &dyn Fn(reqwest::blocking::Response) -> crate::Result<Option<crate::VideoInfo>>,
    (what, name): (&str, &str),
    note: &str,
    headers: &[(&str, &str)],
) -> Option<crate::VideoInfo> {
    let note = format!("[{extractor}] {id}: {note}");
    crate::screen::expect_page(id, Some(note.clone()), None);
    let response = direct::request_with_headers(client, manifest, None, headers);
    if response.is_err() {
        crate::screen::screen(&note);
    }
    let parsed = response.and_then(parse);
    crate::screen::clear_page();
    match parsed {
        Ok(mut stream) => {
            if !keep_subtitles && let Some(stream) = &mut stream {
                ignore_subtitles(stream, extractor, name);
            }
            stream
        }
        Err(error) => {
            crate::screen::warning(&format!(
                "[{extractor}] {id}: Failed to download {what}: {}",
                crate::upstream_error_text(&error)
            ));
            None
        }
    }
}

/// Upstream `_download_json(url, video_id)`: `[EXTRACTOR] ID: Downloading
/// JSON metadata`, the request, and the parsed body.
pub(crate) fn download_json(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
) -> Result<serde_json::Value> {
    download_json_note(client, url, extractor, id, "Downloading JSON metadata")
}

/// [`download_json`] with its `note=` (the error note stays the default).
pub(crate) fn download_json_note(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
    note: &str,
) -> Result<serde_json::Value> {
    download_json_transformed(
        client,
        url,
        extractor,
        id,
        note,
        &|text| Ok(text.to_owned()),
    )
}

/// [`download_json_note`] with the request's `headers=`.
pub(crate) fn download_json_note_headers(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
    note: &str,
    headers: &[(&str, &str)],
) -> Result<serde_json::Value> {
    download_json_with(
        client,
        url,
        extractor,
        id,
        note,
        &|text| Ok(text.to_owned()),
        headers,
    )
}

/// [`download_json`] with the request's `headers=`.
pub(crate) fn download_json_headers(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
    headers: &[(&str, &str)],
) -> Result<serde_json::Value> {
    download_json_with(
        client,
        url,
        extractor,
        id,
        "Downloading JSON metadata",
        &|text| Ok(text.to_owned()),
        headers,
    )
}

/// [`download_json_note`] with `transform_source` applied to the body.
pub(crate) fn download_json_transformed(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
    note: &str,
    transform: &dyn Fn(&str) -> Result<String>,
) -> Result<serde_json::Value> {
    download_json_with(client, url, extractor, id, note, transform, &[])
}

fn download_json_with(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
    note: &str,
    transform: &dyn Fn(&str) -> Result<String>,
    headers: &[(&str, &str)],
) -> Result<serde_json::Value> {
    use std::io::Read;
    crate::screen::screen(&format!("[{extractor}] {id}: {note}"));
    let wrap = |error: crate::Error| match error {
        crate::Error::Http(_) | crate::Error::HttpRetryAfter { .. } => {
            crate::Error::ExtractorRequest {
                extractor,
                id: id.to_owned(),
                what: "JSON metadata",
                source: Box::new(error),
            }
        }
        error => error,
    };
    let mut response = direct::request_with_headers(client, url, None, headers).map_err(wrap)?;
    let page_url = response.url().clone();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(crate::Error::ExtractionRead)?;
    let body = decode_content(response.headers(), body).map_err(crate::Error::ExtractionRead)?;
    // `_webpage_read_content`: `--dump-pages`/`--write-pages`.
    crate::screen::expect_page(id, None, None);
    crate::screen::page_read(&page_url, &body)?;
    let text = transform(&decode_webpage(&body, response.headers()))?;
    crate::json::from_str(&text).map_err(|error| {
        crate::Error::Reported(format!(
            "[{extractor}] {id}: Failed to parse JSON  (caused by {error})"
        ))
    })
}

/// Upstream `update_url_query(url, query)`: the query decoded, updated and
/// re-encoded like `urlencode`.
pub(crate) fn update_url_query(url: &Url, updates: &[(&str, &str)]) -> Url {
    // `parse_qs` (blank values dropped, repeated keys grouped) and
    // `dict.update` (an updated key keeps its place).
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for (key, value) in url.query_pairs() {
        if value.is_empty() {
            continue;
        }
        match groups.iter_mut().find(|(existing, _)| *existing == key) {
            Some((_, values)) => values.push(value.into_owned()),
            None => groups.push((key.into_owned(), vec![value.into_owned()])),
        }
    }
    for (key, value) in updates {
        match groups.iter_mut().find(|(existing, _)| existing == key) {
            Some((_, values)) => *values = vec![(*value).to_owned()],
            None => groups.push(((*key).to_owned(), vec![(*value).to_owned()])),
        }
    }
    let pairs: Vec<(String, String)> = groups
        .into_iter()
        .flat_map(|(key, values)| values.into_iter().map(move |value| (key.clone(), value)))
        .collect();
    let mut url = url.clone();
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(&pairs)
        .finish();
    url.set_query((!query.is_empty()).then_some(query.as_str()));
    url
}

/// Upstream `_download_webpage(url, video_id)`: `[EXTRACTOR] ID: Downloading
/// webpage`, the request (`--dump-pages`/`--write-pages` like other
/// pages), and the body as UTF-8.
pub(crate) fn download_webpage(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
) -> Result<String> {
    download_webpage_note(client, url, extractor, id, "Downloading webpage")
}

/// [`download_webpage`] with another `note`.
pub(crate) fn download_webpage_note(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
    note: &str,
) -> Result<String> {
    crate::screen::screen(&format!("[{extractor}] {id}: {note}"));
    download_webpage_handle_quiet(client, url, extractor, id).map(|(text, _)| text)
}

/// `_download_webpage(url, None, note)`: the note was printed by the caller.
pub(crate) fn download_webpage_quiet(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
) -> Result<String> {
    download_webpage_handle_quiet(client, url, extractor, id).map(|(text, _)| text)
}

/// `_download_webpage_handle(url, video_id)`: the page and its final URL.
pub(crate) fn download_webpage_handle(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
) -> Result<(String, Url)> {
    crate::screen::screen(&format!("[{extractor}] {id}: Downloading webpage"));
    download_webpage_handle_quiet(client, url, extractor, id)
}

/// [`download_webpage_note`] with the request's `headers=`.
pub(crate) fn download_webpage_note_headers(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
    note: &str,
    headers: &[(&str, &str)],
) -> Result<String> {
    crate::screen::screen(&format!("[{extractor}] {id}: {note}"));
    download_webpage_handle_headers(client, url, extractor, id, headers).map(|(text, _)| text)
}

fn download_webpage_handle_quiet(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
) -> Result<(String, Url)> {
    download_webpage_handle_headers(client, url, extractor, id, &[])
}

fn download_webpage_handle_headers(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
    headers: &[(&str, &str)],
) -> Result<(String, Url)> {
    use std::io::Read;
    let wrap = |error: crate::Error| match error {
        crate::Error::Http(_) | crate::Error::HttpRetryAfter { .. } => {
            crate::Error::ExtractorRequest {
                extractor,
                id: id.to_owned(),
                what: "webpage",
                source: Box::new(error),
            }
        }
        error => error,
    };
    let mut response = direct::request_with_headers(client, url, None, headers).map_err(wrap)?;
    let page_url = response.url().clone();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(crate::Error::ExtractionRead)?;
    let body = decode_content(response.headers(), body).map_err(crate::Error::ExtractionRead)?;
    crate::screen::expect_page(id, None, None);
    crate::screen::page_read(&page_url, &body)?;
    Ok((decode_webpage(&body, response.headers()), page_url))
}

/// The urllib handler's `http_response`: a body decoded from each
/// `Content-Encoding` in reverse (`gzip` as a single member, `deflate` raw or
/// zlib-wrapped); other encodings are left as sent. Extraction requests ask
/// for `identity`, but some servers compress regardless.
pub(crate) fn decode_content(
    headers: &reqwest::header::HeaderMap,
    body: Vec<u8>,
) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let encodings: Vec<String> = headers
        .get_all(reqwest::header::CONTENT_ENCODING)
        .iter()
        .flat_map(|value| {
            String::from_utf8_lossy(value.as_bytes())
                .split(',')
                .map(|encoding| encoding.trim().to_owned())
                .collect::<Vec<_>>()
        })
        .collect();
    let mut body = body;
    for encoding in encodings.iter().rev() {
        if body.is_empty() {
            break;
        }
        let mut decoded = Vec::new();
        match encoding.as_str() {
            "gzip" => {
                flate2::read::GzDecoder::new(body.as_slice()).read_to_end(&mut decoded)?;
            }
            "deflate" => {
                if flate2::read::DeflateDecoder::new(body.as_slice())
                    .read_to_end(&mut decoded)
                    .is_err()
                {
                    decoded.clear();
                    flate2::read::ZlibDecoder::new(body.as_slice()).read_to_end(&mut decoded)?;
                }
            }
            _ => continue,
        }
        body = decoded;
    }
    Ok(body)
}

/// `_guess_encoding_from_content(content_type, webpage_bytes)`: the
/// Content-Type charset, else a `<meta charset>` in the first 1024 bytes,
/// else UTF-16 after its little-endian BOM, else UTF-8.
fn guess_encoding(content_type: &str, body: &[u8]) -> String {
    static CONTENT_TYPE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^[a-zA-Z0-9_.-]+/[a-zA-Z0-9_.-]+\s*;\s*charset=(.+)").unwrap()
    });
    static META: std::sync::LazyLock<regex::bytes::Regex> = std::sync::LazyLock::new(|| {
        regex::bytes::Regex::new(r#"(?-u)<meta[^>]+charset=['"]?([^'")]+)[ /'">]"#).unwrap()
    });
    if let Some(found) = CONTENT_TYPE.captures(content_type) {
        return found[1].to_owned();
    }
    if let Some(found) = META.captures(&body[..body.len().min(1024)]) {
        return String::from_utf8_lossy(&found[1]).into_owned();
    }
    if body.starts_with(b"\xff\xfe") {
        return "utf-16".into();
    }
    "utf-8".into()
}

/// `__decode_webpage`: the body decoded with its guessed encoding, invalid
/// sequences replaced; unknown encodings (`LookupError`) fall back to UTF-8.
/// Encodings are WHATWG labels, so `latin-1`/`iso-8859-1` decode as
/// windows-1252 (which differs from Python only in C1 control characters).
pub(crate) fn decode_webpage(body: &[u8], headers: &reqwest::header::HeaderMap) -> String {
    let content_type = headers
        .get(reqwest::header::CONTENT_TYPE)
        .map(|value| String::from_utf8_lossy(value.as_bytes()).into_owned())
        .unwrap_or_default();
    let label = guess_encoding(&content_type, body).to_ascii_lowercase();
    match label.as_str() {
        // Python's `utf-16` reads either BOM (little-endian without one).
        "utf-16" | "utf16" => return encoding_rs::UTF_16LE.decode(body).0.into_owned(),
        "utf-8" | "utf8" => {}
        _ => {
            if let Some(encoding) = encoding_rs::Encoding::for_label(label.as_bytes())
                && encoding != encoding_rs::UTF_8
            {
                return encoding.decode_without_bom_handling(body).0.into_owned();
            }
        }
    }
    String::from_utf8_lossy(body).into_owned()
}

/// Upstream `_download_xml(url, video_id)`: `[EXTRACTOR] ID: Downloading
/// XML`, the request, and the body. Parse errors are this port's text
/// (upstream reports expat's).
pub(crate) fn download_xml(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
) -> Result<String> {
    download_xml_note(client, url, extractor, id, "Downloading XML")
}

/// [`download_xml`] with its `note=` (the error note stays the default).
pub(crate) fn download_xml_note(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
    note: &str,
) -> Result<String> {
    use std::io::Read;
    crate::screen::screen(&format!("[{extractor}] {id}: {note}"));
    let wrap = |error: crate::Error| match error {
        crate::Error::Http(_) | crate::Error::HttpRetryAfter { .. } => {
            crate::Error::ExtractorRequest {
                extractor,
                id: id.to_owned(),
                what: "XML",
                source: Box::new(error),
            }
        }
        error => error,
    };
    let mut response = direct::request(client, url, None).map_err(wrap)?;
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(crate::Error::ExtractionRead)?;
    let body = decode_content(response.headers(), body).map_err(crate::Error::ExtractionRead)?;
    let text = String::from_utf8(body).map_err(|_| {
        crate::Error::Reported(format!(
            "[{extractor}] {id}: Failed to parse XML (not UTF-8)"
        ))
    })?;
    roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).map_err(|error| {
        crate::Error::Reported(format!(
            "[{extractor}] {id}: Failed to parse XML ({error}); {}",
            crate::BUG_REPORTS_MESSAGE
        ))
    })?;
    Ok(text)
}

/// Upstream `_WORKING = False`: the warning before extraction.
pub(crate) fn broken_warning() {
    crate::screen::warning(
        "The program functionality for this site has been marked as broken, and will probably not work.",
    );
}

/// `root.find(tag).text` of an XML document: `Err` (upstream's bare
/// `AttributeError`) for a missing child, `None` for an empty one.
pub(crate) fn find_text(root: roxmltree::Node<'_, '_>, tag: &str) -> Result<Option<String>> {
    let child = root
        .children()
        .find(|child| child.is_element() && child.tag_name().name() == tag)
        .ok_or_else(|| {
            crate::Error::Reported("'NoneType' object has no attribute 'text'".into())
        })?;
    let text: String = child
        .children()
        .take_while(|node| !node.is_element())
        .filter_map(|node| node.text())
        .collect();
    Ok((!text.is_empty()).then_some(text))
}

/// `_search_json(start_pattern, string, ...)`: the greedy `{...}` after the
/// start (`contains_pattern=r'{(?s:.+)}'`), optionally converted with
/// `js_to_json`, and its first JSON value (`ignore_extra=True`); `None`
/// without a match, `Some(None)` when it does not parse.
pub(crate) fn search_json(
    start: &regex::Regex,
    html: &str,
    js: bool,
) -> Option<Option<serde_json::Value>> {
    if js {
        search_json_with(start, html, &crate::js_to_json::js_to_json)
    } else {
        search_json_with(start, html, &str::to_owned)
    }
}

/// [`search_json`] with any `transform_source`.
pub(crate) fn search_json_with(
    start: &regex::Regex,
    html: &str,
    transform: &dyn Fn(&str) -> String,
) -> Option<Option<serde_json::Value>> {
    let found = start.find(html)?;
    let rest = &html[found.end()..];
    let trimmed = rest.trim_start();
    if !trimmed.starts_with('{') {
        return None;
    }
    let end = trimmed.rfind('}')?;
    let candidate = &trimmed[..=end];
    let text = transform(candidate);
    Some(
        serde_json::Deserializer::from_str(&crate::json::replace_lone_surrogates(&text))
            .into_iter::<serde_json::Value>()
            .next()
            .and_then(std::result::Result::ok),
    )
}

/// `_search_nextjs_data(webpage, video_id)` (fatal): the `__NEXT_DATA__`
/// script's JSON (greedy to the last `}` before a `</script>`).
pub(crate) fn search_nextjs_data(
    html: &str,
    extractor: &str,
    id: &str,
) -> crate::Result<serde_json::Value> {
    static NEXT_DATA: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r#"(?s)<script[^>]+id=['"]__NEXT_DATA__['"][^>]*>\s*(\{.+\})\s*</script>"#,
        )
        .expect("static pattern")
    });
    let Some(found) = NEXT_DATA.captures(html) else {
        return Err(crate::Error::Reported(format!(
            "[{extractor}] {id}: Unable to extract next.js data; {}",
            crate::BUG_REPORTS_MESSAGE
        )));
    };
    serde_json::Deserializer::from_str(&found[1])
        .into_iter::<serde_json::Value>()
        .next()
        .and_then(std::result::Result::ok)
        .ok_or_else(|| {
            crate::Error::Reported(format!(
                "[{extractor}] {id}: Unable to extract next.js data - Failed to parse JSON"
            ))
        })
}

/// Whether an extractor's `_VALID_URL` takes this non-HTTP pseudo-URL.
pub fn pseudo_url(url: &str) -> bool {
    (url.starts_with("jwplatform:") && jwplatform::suitable(url))
        || (url.starts_with("kaltura:") && kaltura::suitable(url))
        || youtube::search::suitable(url)
        || (url.starts_with("at://") && bluesky::suitable(url))
        || (url.starts_with("arte://") && arte::suitable(url))
        || (url.starts_with("lbry://") && lbry::suitable(url))
        || (url.starts_with("coub:") && coub::suitable(url))
        || (url.starts_with("nrk:") && nrk::suitable(url))
        || (url.starts_with("peertube:") && peertube::suitable(url))
        || (url.starts_with("svt:") && svt::suitable(url))
        || (url.starts_with("iheartradio:") && iheart::suitable(url))
        || instagram::ios_suitable(url)
        || (url.starts_with("tco:") && twitter::shortener_suitable(url))
        || (url.starts_with("tiktokuser:") && tiktok::user_suitable(url))
        || (url.starts_with("wistia:") && wistia::suitable(url))
        || (url.starts_with("wistiachannel:") && wistia::channel_suitable(url))
        || youtube::redirect::user_suitable(url)
}

/// Scheme of the internal URL carrying an input without a scheme that an
/// extractor's `_VALID_URL` takes (a naked YouTube ID, `:ytfav`).
const BARE_SCHEME: &str = "yt-dlp-rs-input";

/// An input without a scheme as an internal URL ([`input_text`] recovers it).
pub(crate) fn bare_url(input: &str) -> Url {
    let encoded = percent_encoding::utf8_percent_encode(input, percent_encoding::NON_ALPHANUMERIC);
    Url::parse(&format!("{BARE_SCHEME}:{encoded}")).expect("an opaque URL")
}

/// An input without a scheme that an extractor takes, as an internal URL,
/// with the name of the first extractor (in upstream order) taking it.
pub fn bare_input(input: &str) -> Option<(Url, &'static str)> {
    youtube::bare_extractor(input).map(|name| (bare_url(input), name))
}

/// The input text an extractor matches: the original text of an internal
/// URL for an input without a scheme, otherwise the URL.
pub(crate) fn input_text(url: &Url) -> String {
    match url
        .as_str()
        .strip_prefix(BARE_SCHEME)
        .and_then(|rest| rest.strip_prefix(':'))
    {
        Some(encoded) => percent_encoding::percent_decode_str(encoded)
            .decode_utf8_lossy()
            .into_owned(),
        None => url.to_string(),
    }
}

/// The input URL with its own query and fragment: an internal URL's text
/// parsed under the internal scheme (`ID#t=1` has the fragment `t=1`).
pub(crate) fn input_view(url: &Url) -> Url {
    if url.scheme() != BARE_SCHEME {
        return url.clone();
    }
    Url::parse(&format!("{BARE_SCHEME}:{}", input_text(url))).unwrap_or_else(|_| url.clone())
}

/// A result URL to follow: text without a scheme (`//host/…`, a naked ID)
/// becomes an internal URL, as upstream matches it as text.
pub(crate) fn target_url(text: &str) -> crate::Result<Url> {
    if crate::has_scheme(text) {
        Ok(Url::parse(text)?)
    } else {
        Ok(bare_url(text))
    }
}

/// Upstream `params['http_headers']['User-Agent']`.
pub(crate) static USER_AGENT: std::sync::RwLock<String> = std::sync::RwLock::new(String::new());

/// `self.get_param('http_headers')['User-Agent']`.
pub(crate) fn user_agent() -> String {
    USER_AGENT
        .read()
        .map(|agent| agent.clone())
        .unwrap_or_default()
}

pub use login::{Credentials, set_credentials};

/// Upstream `params['videopassword']` (`--video-password`).
static VIDEO_PASSWORD: std::sync::RwLock<Option<String>> = std::sync::RwLock::new(None);

/// Set `--video-password` for extractors that read it.
pub fn set_video_password(password: Option<String>) {
    if let Ok(mut slot) = VIDEO_PASSWORD.write() {
        *slot = password;
    }
}

/// YoutubeTabIE's `--compat-options`: `no-youtube-channel-redirect` and
/// `no-youtube-unavailable-videos`.
pub fn set_youtube_tab_compat(no_channel_redirect: bool, no_unavailable_videos: bool) {
    use std::sync::atomic::Ordering::Relaxed;
    youtube::tab::NO_CHANNEL_REDIRECT.store(no_channel_redirect, Relaxed);
    youtube::tab::NO_UNAVAILABLE_VIDEOS.store(no_unavailable_videos, Relaxed);
}

/// `self.get_param('playlist_items') == '0'`: YoutubeTabIE's extra channel
/// tabs stay unextracted `url` entries.
pub fn set_youtube_tab_playlist_items(items: Option<&str>) {
    youtube::tab::PLAYLIST_ITEMS_ZERO
        .store(items == Some("0"), std::sync::atomic::Ordering::Relaxed);
}

/// `self.get_param('videopassword')`.
pub(crate) fn video_password() -> Option<String> {
    VIDEO_PASSWORD
        .read()
        .ok()
        .and_then(|password| password.clone())
}

/// `self.get_param('check_formats') is False` (`--no-check-formats`).
pub(crate) static CHECK_FORMATS_NEVER: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// `self.get_param('check_formats') is False`.
pub(crate) fn check_formats_never() -> bool {
    CHECK_FORMATS_NEVER.load(std::sync::atomic::Ordering::Relaxed)
}

/// Upstream `noplaylist` (`--no-playlist`), for `_yes_playlist`.
pub(crate) static NO_PLAYLIST: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// `self.get_param('noplaylist')`.
pub(crate) fn no_playlist() -> bool {
    NO_PLAYLIST.load(std::sync::atomic::Ordering::Relaxed)
}

/// Upstream `getcomments` (`--write-comments`), for `extract_comments`.
pub(crate) static GET_COMMENTS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// `self.get_param('getcomments')`.
pub(crate) fn get_comments() -> bool {
    GET_COMMENTS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Upstream `writesubtitles or listsubtitles` (`InfoExtractor.extract_subtitles`).
pub(crate) static SUBTITLES_REQUESTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// `extract_subtitles`: whether an extractor's `_get_subtitles` runs.
pub(crate) fn subtitles_requested() -> bool {
    SUBTITLES_REQUESTED.load(std::sync::atomic::Ordering::Relaxed)
}

/// A non-fatal `_search_regex` miss: `[EXTRACTOR] unable to extract NAME`.
pub(crate) fn search_warning(extractor: &str, name: &str) {
    crate::screen::warning(&format!(
        "[{extractor}] unable to extract {name}; {}",
        crate::BUG_REPORTS_MESSAGE
    ));
}

/// `self._proto_relative_url(url, scheme='http:')`.
pub(crate) fn proto_relative_http(url: &str) -> String {
    match url.strip_prefix("//") {
        Some(rest) => format!("http://{rest}"),
        None => url.to_owned(),
    }
}

/// `self._proto_relative_url(url)`: `//host/...` becomes `https://host/...`.
pub(crate) fn proto_relative(url: &str) -> String {
    match url.strip_prefix("//") {
        Some(rest) => format!("https://{rest}"),
        None => url.to_owned(),
    }
}

/// An upstream `{'url': ..., ...}` format dict in its key order (`None`
/// values keep their keys); `ext` is processing's `determine_ext(url).lower()`
/// unless given.
pub(crate) fn dict_format(
    fields: Vec<(&str, serde_json::Value)>,
    extractor: &str,
    id: &str,
) -> Result<crate::VideoFormat> {
    let keys: Vec<String> = fields.iter().map(|(key, _)| (*key).to_owned()).collect();
    let mut map = serde_json::Map::new();
    for (key, value) in fields {
        if !value.is_null() {
            map.insert(key.to_owned(), value);
        }
    }
    let has_quality = map.get("quality").is_some_and(serde_json::Value::is_i64);
    if !map.contains_key("ext") {
        let ext = map
            .get("url")
            .and_then(serde_json::Value::as_str)
            .and_then(direct::determine_ext)
            .unwrap_or_else(|| "unknown_video".into())
            .to_lowercase();
        map.insert("ext".into(), ext.into());
    }
    // Processing numbers formats without a `format_id`.
    if !map.contains_key("format_id") {
        map.insert("format_id".into(), "".into());
    }
    let mut format: crate::VideoFormat = serde_json::from_value(serde_json::Value::Object(map))
        .map_err(|error| crate::Error::Reported(format!("[{extractor}] {id}: {error}")))?;
    crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
    if has_quality {
        crate::info_order::mark_int_quality(&mut format);
    }
    Ok(format)
}

/// An upstream `return {...}` info dict: `fields` in its key order (a
/// `formats` entry, or a `None` `subtitles` the caller sets, only marks the
/// key's position) with
/// the extracted `formats`.
pub(crate) fn dict_info(
    page: &Url,
    (extractor, key): (&'static str, &'static str),
    fields: Vec<(&str, serde_json::Value)>,
    formats: Vec<crate::VideoFormat>,
) -> crate::VideoInfo {
    let first = formats
        .first()
        .map_or_else(|| page.clone(), |format| format.url.clone());
    let mut info = direct::build_at(first.clone(), &first, String::new());
    info.direct = None;
    if formats.is_empty() {
        info.extra.insert(
            crate::info::NO_FORMATS.into(),
            serde_json::Value::Bool(true),
        );
    }
    info.formats = formats;
    info.webpage_url = page.clone();
    info.extractor = extractor.into();
    info.extractor_key = key.into();
    let mut keys = Vec::new();
    let mut titled = false;
    // A `None` title keeps its key's position for processing's generic one.
    let mut title_key = false;
    for (name, value) in fields {
        keys.push(name.to_owned());
        match name {
            "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
            "title" if value.is_null() => title_key = true,
            "title" => {
                titled = true;
                info.title = value.as_str().unwrap_or_default().to_owned();
            }
            "formats" => {}
            // The caller's `info.subtitles`.
            "subtitles" if value.is_null() => {}
            _ => crate::metadata_parser::set_field(&mut info, name, value),
        }
    }
    if !titled {
        info.title = String::new();
        info.extra.insert(
            crate::info::TITLE_MISSING.into(),
            serde_json::Value::Bool(true),
        );
    }
    crate::info_order::mark_info(
        info,
        &format!(
            "dict{};{}",
            if titled || title_key { "" } else { "-untitled" },
            keys.join(",")
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{CONTENT_ENCODING, CONTENT_TYPE, HeaderMap, HeaderValue};

    fn headers(pairs: &[(reqwest::header::HeaderName, &'static str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(name, HeaderValue::from_static(value));
        }
        map
    }

    #[test]
    fn pages_decode_with_upstreams_guessed_encoding() {
        // "Привет" in windows-1251, as VK serves its al_video responses.
        let cp1251 = b"{\"t\": \"\xcf\xf0\xe8\xe2\xe5\xf2\"}";
        let typed = headers(&[(CONTENT_TYPE, "application/json; charset=windows-1251")]);
        assert_eq!(decode_webpage(cp1251, &typed), "{\"t\": \"Привет\"}");
        let meta = b"<meta charset=\"windows-1251\"><p>\xcf\xf0\xe8\xe2\xe5\xf2</p>";
        assert!(decode_webpage(meta, &HeaderMap::new()).ends_with("<p>Привет</p>"));
        // UTF-16 after its BOM; unknown encodings and invalid bytes as UTF-8.
        let utf16 = [0xff, 0xfe, b'h', 0, b'i', 0];
        assert_eq!(decode_webpage(&utf16, &HeaderMap::new()), "hi");
        let unknown = headers(&[(CONTENT_TYPE, "text/html; charset=x-unknown")]);
        assert_eq!(decode_webpage(b"ok \xff", &unknown), "ok \u{FFFD}");
    }

    #[test]
    fn compressed_bodies_decode_in_reverse_encoding_order() {
        use std::io::Write;
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gzip.write_all(b"payload").unwrap();
        let gzip = gzip.finish().unwrap();
        let mut both = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        both.write_all(&gzip).unwrap();
        let both = both.finish().unwrap();
        let map = headers(&[(CONTENT_ENCODING, "gzip, deflate")]);
        assert_eq!(decode_content(&map, both).unwrap(), b"payload");
        let map = headers(&[(CONTENT_ENCODING, "br")]);
        assert_eq!(decode_content(&map, b"raw".to_vec()).unwrap(), b"raw");
    }
}
