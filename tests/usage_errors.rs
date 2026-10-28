//! Option validation failures follow upstream `parser.error`: a blank line,
//! the usage line, `PROG: error: MESSAGE` and exit status 2, before any request.
use std::process::Command;

fn run(args: &[&str]) -> (Option<i32>, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(args)
        .output()
        .unwrap();
    assert!(output.stdout.is_empty());
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn validation_messages_and_layout_follow_upstream() {
    let url = "http://127.0.0.1:9/never";
    for (args, message) in [
        (
            vec!["--audio-format", "xyz"],
            r#"invalid audio format "xyz" given"#,
        ),
        (
            vec!["--audio-quality", "-1"],
            r#"audio quality "-1" must be positive or 0"#,
        ),
        (
            vec!["--remux-video", "xyz"],
            r#"invalid remux video format "xyz" given"#,
        ),
        (
            vec!["--recode-video", "a>b>c"],
            r#"invalid recode video format "a>b>c" given"#,
        ),
        (
            vec!["--convert-subs", "xyz"],
            r#"invalid subtitle format "xyz" given"#,
        ),
        (
            vec!["--convert-thumbnails", "gif"],
            r#"invalid thumbnail format "gif" given"#,
        ),
        (
            vec!["--playlist-start", "0"],
            r#"playlist start "0" must be positive"#,
        ),
        (
            vec!["--playlist-start", "5", "--playlist-end", "2"],
            r#"playlist end "2" must be must be greater than or equal to playlist start "5""#,
        ),
        (
            vec!["--playlist-items", "1,abc"],
            "Invalid playlist-items '1,abc': 'abc' is not a valid specification",
        ),
        (
            vec!["--playlist-items", "1,,2"],
            "Invalid playlist-items '1,,2': There is two or more consecutive commas",
        ),
        (
            vec!["--playlist-items", "1:5:0"],
            "Invalid playlist-items '1:5:0': Step in '1:5:0' cannot be zero",
        ),
        (
            vec!["--sleep-interval", "-1"],
            r#"sleep interval "-1.0" must be positive or 0"#,
        ),
        (
            vec!["--min-filesize", "abc"],
            r#"invalid min filesize "abc" given"#,
        ),
        (
            vec!["--limit-rate", "0"],
            r#"rate limit "0" must be positive"#,
        ),
        (
            vec!["--netrc", "-u", "user", "-p", "pw"],
            ".netrc, netrc command and username/password are mutually exclusive options",
        ),
        (
            vec!["--netrc-cmd", "echo", "-n"],
            ".netrc, netrc command and username/password are mutually exclusive options",
        ),
        (vec!["-p", "pw"], "account username missing"),
    ] {
        let mut all = args.clone();
        all.push(url);
        let (code, stderr) = run(&all);
        assert_eq!(code, Some(2), "{args:?}: {stderr}");
        assert_eq!(
            stderr,
            format!("\nUsage: yt-dlp-rs [OPTIONS] URL [URL...]\n\nyt-dlp-rs: error: {message}\n"),
            "{args:?}"
        );
    }
}

#[test]
fn a_missing_url_is_reported_after_option_validation() {
    let (code, stderr) = run(&[]);
    assert_eq!(code, Some(2));
    assert_eq!(
        stderr,
        "\nUsage: yt-dlp-rs [OPTIONS] URL [URL...]\n\nyt-dlp-rs: error: You must provide at least one URL.\nType yt-dlp-rs --help to see a list of all options.\n"
    );
    let (code, stderr) = run(&["--date", "bogus"]);
    assert_eq!(code, Some(2));
    assert!(stderr.ends_with("yt-dlp-rs: error: Invalid date format \"bogus\"\n"));
}

#[test]
fn extractor_listings_and_color_policies_follow_upstream() {
    let run = |args: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .arg("--ignore-config")
            .args(args)
            .output()
            .unwrap()
    };
    // Lines checked against pinned upstream for these extractors.
    let output = run(&[
        "--list-extractors",
        "https://www.youtube.com/watch?v=BaW_jenozKc",
        "https://player.megaphone.fm/GLT9749789991",
        "https://example.com/v.mp4",
    ]);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "1News\n23video\n9gag\n9News\nacast\nacast:channel\nAlJazeera\nAmadeusTV\nAngel\napple:podcasts\narchive.org\nARDAudiothek\nARDAudiothekPlaylist\nARDMediathek\nARDMediathekCollection\nArteTV\nArteTVCategory\nArteTVEmbed\nArteTVPlaylist\nAudioBoom\naudiomack\naudiomack:album\nBandcamp\nBeaconTv\nBehindKink (CURRENTLY BROKEN)\nbfmtv\nbfmtv:article\nbfmtv:live\nBigo\nBild\nBiliBili\nBitChute\nBitChuteChannel\nBitmovin\nblogger.com\nBluesky\nBongaCams\nBostonGlobe\nBreitBart\nbrightcove:new\nBundesliga\nBundestag\nCaltrans\nCAM4\nCGTN\nCJSW\nClipchamp\nCloudflareStream\nCloudyCDN\nClyp\nCoub\nCozyTV\nCrooksAndLiars\nCrtvg\nCultureUnplugged\nDailyMail\ndailymotion\ndailymotion:playlist\ndailymotion:search\ndailymotion:user\ndaystar:clip\ndemocracynow\nDigiteka\ndlive:stream\ndlive:vod\nDouyin\nDropbox\nDrTalks\ndrtv\ndrtv:live\ndrtv:season\ndrtv:series\nDumpert\nEbaumsWorld\nEbay\negghead:course\negghead:lesson\nElonet\nEpoch\nErocast\nERRArhiiv\nERRJupiter\nEttuTv\nEuropeanTour\nExpressen\nfacebook\nfacebook:ads\nfacebook:reel\nFacebookPluginsVideo\nFacebookRedirectURL\nFathom\nfaz.net\nFczenit\nFilmArchiv\nFilmweb\nFlickr\nFormula1\nfranceinfo\nfrancetv\nfrancetv:site\nFreesound\nFuyinTV\nGab\nGameStar\nGenius\nGeniusLyrics\nGlide\nGodResource\nGodTube (CURRENTLY BROKEN)\nGolem\ngoodgame:stream\nGoogleDrive\nGoogleDrive:Folder\nGraspop\nGronkh\nHarpodeon\nHearThisAt\nHellPorno\nHistoricFilms\nHitRecord\nhref.li\nHSEProduct\nHSEShow\nhtml5\nHuajiao\nHypem\niheartradio\niheartradio:podcast\nIlPost\nIltalehti\nimdb\nimdb:list\nImgur\nimgur:album\nimgur:gallery\nIna\nInc\nIndavideoEmbed\nInstagram\ninstagram:story\ninstagram:tag\ninstagram:user (CURRENTLY BROKEN)\nInstagramIOS\nInternazionale\nIsraelNationalNews\nIvoox\nJamendo\nJamendoAlbum\nJove\nJWPlatform\nKaltura\nKankaNews (CURRENTLY BROKEN)\nkick:clip\nkick:clips\nkick:live\nkick:videos\nkick:vod\nKicker\nKickStarter\nKika\nKikaPlaylist\nKommunetv\nKTH\nKu6\nlbry\nlbry:channel\nlbry:playlist\nLibsyn\nLinkedIn\nLivestreamfails\nLnk\nloc\nloom\nLRTRadio\nLRTStream\nLRTVOD\nmaariv.co.il\nMaoriTV\nMasters\nMatchiTV\nMatchTV\nMDR\nMedalTV\nmedia.ccc.de\nMediaite\nmegaphone.fm\n  https://player.megaphone.fm/GLT9749789991\nmir24.tv\nmixcloud\nMixlr\nMixlrRecoring\nMochaVideo\nMonstercat\nMovingImage\nMux\nMyVidster\nNetzkino\nNewsy\nNhkVod\nNhkVodProgram\nniconico\nniconico:history\nniconico:playlist\nniconico:series\nniconico:tag\nNiconicoUser\nnicovideo:search\nnicovideo:search:date\nnicovideo:search_url\nNobelPrize\nNOSNLArticle\nNpr\nNRK\nNRKPlaylist\nNRKRadioPodkast\nNRKTV\nNRKTVDirekte\nNRKTVEpisode\nNRKTVSeason\nNRKTVSeries\nnts.live\nNZOnScreen\nOdnoklassniki\nOlympicsReplay\nOneFootball\nOnePlacePodcast\norf:on\norf:podcast\norf:radio\nParlview\nPearVideo\npeer.tv\nPeerTube\nPeerTube:Playlist\nperiscope\nPhotobucket\nPinkbike\nPinterest\nPlayerFm\nPodbayFM\nPodbayFMChannel\nPopcornTV\nPremiershipRugby\nPressTV\nProjectVeritas (CURRENTLY BROKEN)\nQingTing\nRadioZetPodcast\nReddit\nReverbNation\nRinseFM\nRinseFMArtistPlaylist\nRottenTomatoes\nRoyaLive\nrte\nrte:radio\nRTRFM\nrtve.es:alacarta\nrtve.es:audio\nrtve.es:live\nrtve.es:television\nrtvslo.si\nrtvslo.si:show\nRumble\nRumbleEmbed\nrutube\nrutube:embed\nRuv\nruv.is:spila\nSapo\nScreen9\nScreenRec\nSen\nServus\nsimplecast\nsimplecast:episode\nsimplecast:podcast\nsky:news\nsky:news:story\nsky:sports\nsky:sports:news\nSkylineWebcams (CURRENTLY BROKEN)\nSkyNewsAU\nSnapchatSpotlight\nsoundcloud\nsoundgasm\nsoundgasm:profile\nSpiegel\nSpreaker\nSpreakerShow\nSRGSSR\nSRGSSRPlay\nSteam\nSteamCommunity\nSteamCommunityBroadcast\nStreamable\nStreetVoice\nSubstack\nsverigesradio:episode\nsverigesradio:publication\nsvt:page\nsvt:play\nsvt:play:series\nSztvHu\nTedEmbed\nTedPlaylist\nTedSeries\nTedTalk\nTelegraaf\ntelegram:embed\nTeleTask (CURRENTLY BROKEN)\nTF1\nTheGuardianPodcast\nTheHighWire\nTheIntercept\nTheStar\nTheSun\nThisAmericanLife\nThreeSpeak\nThreeSpeakUser\nTikTok\ntiktok:collection\ntiktok:live\ntiktok:user\nToypics (CURRENTLY BROKEN)\nTrtCocukVideo\nTrtWorld\nTumblr\ntv.dfb.de\nTV2\nTV2Article\nTV2DK\nTV2DKBornholmPlay\nTV4\nTV5MONDE\nTVANouvelles\nTVANouvellesArticle\ntvnoe\nTweakers\ntwitch:clips\ntwitch:collection\ntwitch:stream\ntwitch:videos\ntwitch:videos:clips\ntwitch:videos:collections\ntwitch:vod\ntwitter\ntwitter:amplify\ntwitter:broadcast\ntwitter:card\ntwitter:shortener\ntwitter:spaces\numg:de\nUnistra\nUnitedNationsWebTv\nURPlay\nustudio\nustudio:embed\nVbox7\nVeo\nVideoPress\nVidyard\nvimeo\nVisir\nvk\nvk:uservideos\nvk:wallpost\nVKPlay\nVKPlayLive\nvm.tiktok\nVocaroo\nwat.tv\nWebcamerapl\nWeibo\nWeiboVideo\nWhyp\nwikimedia.org\nWimbledon\nWistia\nWistiaChannel\nWistiaPlaylist\nWorldStarHipHop\nXboxClips\nXiaoHongShu\nXVideos\nxvideos:quickies\nyfanefa\nYleAreena\nyoutube\n  https://www.youtube.com/watch?v=BaW_jenozKc\nyoutube:clip\nyoutube:consent\nyoutube:favorites\nyoutube:history\nyoutube:music:search_url\nyoutube:notif\nyoutube:playlist\nyoutube:recommended\nyoutube:search\nyoutube:search_url\nyoutube:shorts:pivot:audio\nyoutube:subscriptions\nyoutube:tab\nyoutube:truncated_id\nyoutube:truncated_url\nyoutube:user\nyoutube:watchlater\nYoutubeLivestreamEmbed\nYoutubeYtBe\nzdf\nzdf:channel\nZhihu\ngeneric\n  https://example.com/v.mp4\n"
    );
    let output = run(&["--extractor-descriptions"]);
    // Upstream draws the search example's count and query at random.
    let pattern = format!(
        "^{}{}{}{}{}{}{}$",
        regex::escape(
            "1News: 1news.co.nz article videos\n23video\n9gag: 9GAG\n9News\nacast\nacast:channel\nAlJazeera\nAmadeusTV\nAngel\napple:podcasts: Apple Podcasts\narchive.org: archive.org video and audio\nARDAudiothek\nARDAudiothekPlaylist\nARDMediathek\nARDMediathekCollection\nArteTV\nArteTVCategory\nArteTVEmbed\nArteTVPlaylist\nAudioBoom\naudiomack\naudiomack:album\nBandcamp\nBeaconTv\nbfmtv\nbfmtv:article\nbfmtv:live\nBigo\nBild: Bild.de\nBiliBili\nBitChute\nBitChuteChannel\nBitmovin\nblogger.com\nBluesky\nBongaCams\nBostonGlobe\nBreitBart\nbrightcove:new\nBundesliga\nBundestag\nCaltrans\nCAM4\nCGTN\nCJSW\nClipchamp\nCloudflareStream\nCloudyCDN\nClyp\nCoub\nCozyTV\nCrooksAndLiars\nCrtvg\nCultureUnplugged\nDailyMail\ndailymotion: [dailymotion]\ndailymotion:playlist: [dailymotion]\ndailymotion:search: [dailymotion]\ndailymotion:user: [dailymotion]\ndaystar:clip\ndemocracynow\nDigiteka\ndlive:stream\ndlive:vod\nDouyin\nDropbox\nDrTalks\ndrtv\ndrtv:live\ndrtv:season\ndrtv:series\nDumpert\nEbaumsWorld\nEbay\negghead:course: egghead.io course\negghead:lesson: egghead.io lesson\nElonet\nEpoch\nErocast\nERRArhiiv\nERRJupiter\nEttuTv\nEuropeanTour\nExpressen\nfacebook\nfacebook:ads\nfacebook:reel\nFacebookPluginsVideo\nFathom\nfaz.net\nFczenit\nFilmArchiv: FILMARCHIV ON\nFilmweb\nFlickr\nFormula1\nfranceinfo: franceinfo.fr (formerly francetvinfo.fr)\nfrancetv\nfrancetv:site\nFreesound\nFuyinTV\nGab\nGameStar\nGenius\nGeniusLyrics\nGlide: Glide mobile video messages (glide.me)\nGodResource\nGolem\ngoodgame:stream\nGoogleDrive\nGoogleDrive:Folder\nGraspop\nGronkh\nHarpodeon\nHearThisAt\nHellPorno\nHistoricFilms\nHitRecord\nHSEProduct\nHSEShow\nhtml5\nHuajiao: 花椒直播\nHypem\niheartradio\niheartradio:podcast\nIlPost\nIltalehti\nimdb: Internet Movie Database trailers\nimdb:list: Internet Movie Database lists\nImgur\nimgur:album\nimgur:gallery\nIna\nInc\nIndavideoEmbed\nInstagram\ninstagram:story\ninstagram:tag: Instagram hashtag search URLs\nInstagramIOS: IOS instagram:// URL\nInternazionale\nIsraelNationalNews\nIvoox\nJamendo\nJamendoAlbum\nJove\nJWPlatform\nKaltura\nkick:clip\nkick:clips\nkick:live\nkick:videos\nkick:vod\nKicker\nKickStarter\nKika: KiKA.de\nKikaPlaylist\nKommunetv\nKTH\nKu6\nlbry: odysee.com\nlbry:channel: odysee.com channels\nlbry:playlist: odysee.com playlists\nLibsyn\nLinkedIn\nLivestreamfails\nLnk\nloc: Library of Congress\nloom\nLRTRadio\nLRTStream\nLRTVOD\nmaariv.co.il\nMaoriTV\nMasters\nMatchiTV\nMatchTV\nMDR: MDR.DE\nMedalTV\nmedia.ccc.de\nMediaite\nmegaphone.fm: megaphone.fm embedded players\nmir24.tv\nmixcloud\nMixlr\nMixlrRecoring\nMochaVideo\nMonstercat\nMovingImage\nMux\nMyVidster\nNetzkino\nNewsy\nNhkVod\nNhkVodProgram\nniconico: [niconico] ニコニコ動画\nniconico:history: NicoNico user history or likes. Requires cookies.\nniconico:playlist\nniconico:series\nniconico:tag: NicoNico video tag URLs\nNiconicoUser\nnicovideo:search: Nico video search; \"nicosearch:\" prefix (e.g. \"nicosearch"
        ),
        "(|5|10|all):(cute kittens|slithering pythons|falling cat|angry poodle|purple fish|running tortoise|sleeping bunny|burping cow)",
        regex::escape(
            "\")\nnicovideo:search:date: Nico video search, newest first; \"nicosearchdate:\" prefix (e.g. \"nicosearchdate"
        ),
        "(|5|10|all):(cute kittens|slithering pythons|falling cat|angry poodle|purple fish|running tortoise|sleeping bunny|burping cow)",
        regex::escape(
            "\")\nnicovideo:search_url: Nico video search URLs\nNobelPrize\nNOSNLArticle\nNpr\nNRK\nNRKPlaylist\nNRKRadioPodkast\nNRKTV: NRK TV and NRK Radio\nNRKTVDirekte: NRK TV Direkte and NRK Radio Direkte\nNRKTVEpisode\nNRKTVSeason\nNRKTVSeries\nnts.live\nNZOnScreen\nOdnoklassniki\nOlympicsReplay\nOneFootball\nOnePlacePodcast\norf:on\norf:podcast\norf:radio\nParlview\nPearVideo\npeer.tv\nPeerTube\nPeerTube:Playlist\nperiscope: Periscope\nPhotobucket\nPinkbike\nPinterest\nPlayerFm\nPodbayFM\nPodbayFMChannel\nPopcornTV\nPremiershipRugby\nPressTV\nQingTing\nRadioZetPodcast\nReddit\nReverbNation\nRinseFM\nRinseFMArtistPlaylist\nRottenTomatoes\nRoyaLive\nrte: Raidió Teilifís Éireann TV\nrte:radio: Raidió Teilifís Éireann radio\nRTRFM\nrtve.es:alacarta: RTVE a la carta and Play\nrtve.es:audio: RTVE audio\nrtve.es:live: RTVE.es live streams\nrtve.es:television\nrtvslo.si\nrtvslo.si:show\nRumble\nRumbleEmbed\nrutube: Rutube videos\nrutube:embed: Rutube embedded videos\nRuv\nruv.is:spila\nSapo: SAPO Vídeos\nScreen9\nScreenRec\nSen\nServus\nsimplecast\nsimplecast:episode\nsimplecast:podcast\nsky:news\nsky:news:story\nsky:sports\nsky:sports:news\nSkyNewsAU\nSnapchatSpotlight\nsoundcloud: [soundcloud]\nsoundgasm\nsoundgasm:profile\nSpiegel\nSpreaker\nSpreakerShow\nSRGSSR\nSRGSSRPlay: srf.ch, rts.ch, rsi.ch, rtr.ch and swissinfo.ch play sites\nSteam\nSteamCommunity\nSteamCommunityBroadcast\nStreamable\nStreetVoice\nSubstack\nsverigesradio:episode\nsverigesradio:publication\nsvt:page\nsvt:play: SVT Play and Öppet arkiv\nsvt:play:series\nSztvHu\nTedEmbed\nTedPlaylist\nTedSeries\nTedTalk\nTelegraaf\ntelegram:embed\nTF1\nTheGuardianPodcast\nTheHighWire\nTheIntercept\nTheStar\nTheSun\nThisAmericanLife\nThreeSpeak\nThreeSpeakUser\nTikTok\ntiktok:collection\ntiktok:live\ntiktok:user\nTrtCocukVideo\nTrtWorld\nTumblr: [tumblr]\ntv.dfb.de\nTV2\nTV2Article\nTV2DK\nTV2DKBornholmPlay\nTV4: tv4.se and tv4play.se\nTV5MONDE\nTVANouvelles\nTVANouvellesArticle\ntvnoe: Televize Noe\nTweakers\ntwitch:clips: [twitch]\ntwitch:collection: [twitch]\ntwitch:stream: [twitch]\ntwitch:videos: [twitch]\ntwitch:videos:clips: [twitch]\ntwitch:videos:collections: [twitch]\ntwitch:vod: [twitch]\ntwitter\ntwitter:amplify\ntwitter:broadcast\ntwitter:card\ntwitter:shortener\ntwitter:spaces\numg:de: Universal Music Deutschland\nUnistra\nUnitedNationsWebTv\nURPlay\nustudio\nustudio:embed\nVbox7\nVeo\nVideoPress\nVidyard\nvimeo: [vimeo]\nVisir: Vísir\nvk: [vk] VK\nvk:uservideos: [vk] VK - User's Videos\nvk:wallpost: [vk]\nVKPlay\nVKPlayLive\nvm.tiktok\nVocaroo\nwat.tv\nWebcamerapl\nWeibo\nWeiboVideo\nWhyp\nwikimedia.org\nWimbledon\nWistia\nWistiaChannel\nWistiaPlaylist\nWorldStarHipHop\nXboxClips\nXiaoHongShu: 小红书\nXVideos\nxvideos:quickies\nyfanefa\nYleAreena\nyoutube: [youtube] YouTube\nyoutube:clip: [youtube]\nyoutube:favorites: [youtube] YouTube liked videos; \":ytfav\" keyword (requires cookies)\nyoutube:history: [youtube] Youtube watch history; \":ythis\" keyword (requires cookies)\nyoutube:music:search_url: [youtube] YouTube music search URLs with selectable sections, e.g. #songs\nyoutube:notif: [youtube] YouTube notifications; \":ytnotif\" keyword (requires cookies)\nyoutube:playlist: [youtube] YouTube playlists\nyoutube:recommended: [youtube] YouTube recommended videos; \":ytrec\" keyword\nyoutube:search: [youtube] YouTube search; \"ytsearch:\" prefix (e.g. \"ytsearch"
        ),
        "(|5|10|all):(cute kittens|slithering pythons|falling cat|angry poodle|purple fish|running tortoise|sleeping bunny|burping cow)",
        regex::escape(
            "\")\nyoutube:search_url: [youtube] YouTube search URLs with sorting and filter support\nyoutube:shorts:pivot:audio: [youtube] YouTube Shorts audio pivot (Shorts using audio of a given video)\nyoutube:subscriptions: [youtube] YouTube subscriptions feed; \":ytsubs\" keyword (requires cookies)\nyoutube:tab: [youtube] YouTube Tabs\nyoutube:user: [youtube] YouTube user videos; \"ytuser:\" prefix\nyoutube:watchlater: [youtube] Youtube watch later list; \":ytwatchlater\" keyword (requires cookies)\nYoutubeLivestreamEmbed: [youtube] YouTube livestream embeds\nYoutubeYtBe: [youtube] youtu.be\nzdf\nzdf:channel\nZhihu\ngeneric: Generic downloader that works on some sites"
        )
    );
    assert!(
        regex::Regex::new(&pattern)
            .unwrap()
            .is_match(&String::from_utf8_lossy(&output.stdout)),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let output = run(&["--color", "x", "https://example.com/"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("error: \"x\" is not a valid color policy")
    );
    for args in [&["--color", "stderr:never"][..], &["--no-colors"]] {
        let output = run(args);
        assert_eq!(output.status.code(), Some(2));
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("You must provide at least one URL."),
            "{args:?}"
        );
    }
}

#[test]
fn extractor_selection_and_unqualified_inputs_follow_upstream() {
    let run = |args: &[&str]| {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .arg("--ignore-config")
            .args(args)
            .output()
            .unwrap();
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };
    // Checked against pinned upstream; no request is made for these inputs.
    let (code, stdout, stderr) = run(&["--ies", "youtube", "http://127.0.0.1:1/a.mp4"]);
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "ERROR: No suitable extractor found for URL http://127.0.0.1:1/a.mp4\n"
    );
    let (_, stdout, stderr) = run(&["--ies", "default,-generic,end", "http://127.0.0.1:1/a.mp4"]);
    assert_eq!(
        stdout,
        "[UnsupportedURL] Extracting URL: http://127.0.0.1:1/a.mp4\n"
    );
    assert_eq!(stderr, "ERROR: Unsupported URL: http://127.0.0.1:1/a.mp4\n");
    let (code, stdout, stderr) = run(&["foo"]);
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "[generic] Extracting URL: foo\n");
    assert_eq!(stderr, "ERROR: [generic] 'foo' is not a valid URL\n");
    let (code, _, stderr) = run(&["--ies", "(bad", "foo"]);
    assert_eq!(code, Some(2));
    assert!(
        stderr.contains("error: Wrong regex for allowed_extractors: (bad"),
        "{stderr}"
    );
}
