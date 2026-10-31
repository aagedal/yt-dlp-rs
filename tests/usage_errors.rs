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
        "17live\n17live:clip\n17live:vod\n1News\n1tv\n23video\n24tv.ua\n3qsdn\n3sat\n9gag\n9News\n9now.com.au\nabcotvs\nabcotvs:clips\nacast\nacast:channel\nAcFunVideo\nadobetv\nAeonCo\nagalega:videos\nAitubeKZVideo\nAlibaba\nAliExpressLive\nAlJazeera\nAllstar\nAllstarProfile\naltcensored\naltcensored:channel\nAmadeusTV\nAmazonStore\nAMCNetworks\nAmericasTestKitchen\nAmericasTestKitchenSeason\nAngel\nAPA\napple:music:connect\napple:podcasts\narchive.org\nArcPublishing\nARDAudiothek\nARDAudiothekPlaylist\nARDMediathek\nARDMediathekCollection\nArt19\nArteTV\nArteTVCategory\nArteTVEmbed\nArteTVPlaylist\nasobichannel\nasobichannel:tag\nAtresPlayer\nAtScaleConfEvent\nAudiMedia\nAudioBoom\nAudiodraft:custom\nAudiodraft:generic\naudiomack\naudiomack:album\nAudius\naudius:artist\naudius:playlist\naudius:track\nAZMedien\nBaiduVideo\nBandcamp\nBeaconTv\nBeatport\nBeeg\nBehindKink (CURRENTLY BROKEN)\nBerufeTV\nbfmtv\nbfmtv:article\nbfmtv:live\nBigo\nBild\nBiliBili\nBitChute\nBitChuteChannel\nBitmovin\nblerp\nblogger.com\nBloomberg\nBluesky\nBongaCams\nBoosty\nBostonGlobe\nBox\nBoxCastVideo\nBpb\nBreitBart\nbrightcove:new\nBTVPlus\nBundesliga\nBundestag\nBunnyCdn\nBusinessInsider\nCaltrans\nCAM4\nCanalAlpha\nCanalsurmas\nCCMA\nCCTV\nCellebrite\nCGTN\nCharlieRose\nChaturbate\nChilloutzone\nCiscoLiveSearch\nCiscoLiveSession\nCJSW\nClipchamp\nCloudflareStream\nCloudyCDN\nClyp\nCNBCVideo\nCNN\nCondeNast\nCoub\nCozyTV\ncpac\ncpac:playlist\nCraftsy\nCrooksAndLiars\nCrtvg\nCtsNews\nCultureUnplugged\ncuriositystream\ncuriositystream:collections\ncuriositystream:series\nDacastPlaylist\nDacastVOD\nDailyMail\ndailymotion\ndailymotion:playlist\ndailymotion:search\ndailymotion:user\ndamtomo:record\ndamtomo:video\ndaystar:clip\nDBTV\nDctpTv\ndemocracynow\nDetikEmbed\nDeuxM\nDeuxMNews\nDigiteka\nDiscogsReleasePlaylist\ndlf\ndlf:corpus\ndlive:stream\ndlive:vod\nDouyin\nDropbox\nDrTalks\ndrtv\ndrtv:live\ndrtv:season\ndrtv:series\nDumpert\nDuoplay\ndvtv\nEbaumsWorld\nEbay\negghead:course\negghead:lesson\neggs:artist\neggs:single\nElonet\nElPais\nElTreceTV\nEmbedly\nEpoch\nEporner\nErocast\nEroProfile:album\nERRArhiiv\nERRJupiter\nertflix:codename\nEttuTv\nEuropeanTour\nEUScreen\nExpressen\nfacebook\nfacebook:ads\nfacebook:reel\nFacebookPluginsVideo\nFacebookRedirectURL\nFathom\nFaulio\nfaz.net\nfc2\nfc2:embed\nFczenit\nFilmArchiv\nfilmon\nfilmon:channel\nFilmweb\nFiveTV\nFlickr\nFootyRoom\nFormula1\nfoxnews\nfoxnews:article\nFoxNewsVideo\nFrancaisFacile\nfranceinfo\nfrancetv\nfrancetv:site\nFreesound\nfreespeech.org\nfreetv:series\nFreeTvMovies\nFunker530\nFuyinTV\nGab\nGaia\nGameSpot\nGameStar\nGaskrank\nGBNews\nGenius\nGeniusLyrics\nGermanupa\nGettr\nGettrStreaming\nGlide\nGlobalPlayerAudio\nGlobalPlayerAudioEpisode\nGlobalPlayerLive\nGlobalPlayerLivePlaylist\nGlobalPlayerVideo\nGlobo\nGloboArticle\nglomex\nglomex:embed\nGMANetworkVideo\nGodResource\nGodTube (CURRENTLY BROKEN)\nGolem\ngoodgame:stream\nGoogleDrive\nGoogleDrive:Folder\nGoPro\nGoToStage\nGraspop\nGronkh\nGroupon\nHarpodeon\nHearThisAt\nHellPorno\nHistoricFilms\nHitRecord\nHollywoodReporter\nHollywoodReporterPlaylist\nHolodex\nhref.li\nHSEProduct\nHSEShow\nhtml5\nHuajiao\nHuffPost\nhuya:live\nhuya:video\nHypem\nHytale\nIdagioAlbum\nIdagioPersonalPlaylist\nIdagioPlaylist\nIdagioRecording\nIdagioTrack\niheartradio\niheartradio:podcast\nIlPost\nIltalehti\nimdb\nimdb:list\nImgur\nimgur:album\nimgur:gallery\nIna\nInc\nIndavideoEmbed\nInfoQ\nInstagram\ninstagram:story\ninstagram:tag\ninstagram:user (CURRENTLY BROKEN)\nInstagramIOS\nInternazionale\nIslamChannel\nIslamChannelSeries\nIsraelNationalNews\nivideon\nIvoox\niwara\niwara:playlist\niwara:user\nJamendo\nJamendoAlbum\nJoj\nJove\nJStream\nJWPlatform\nKaltura\nKankaNews (CURRENTLY BROKEN)\nKenh14Playlist\nKenh14Video\nkick:clip\nkick:clips\nkick:live\nkick:videos\nkick:vod\nKicker\nKickStarter\nKika\nKikaPlaylist\nKinoPoisk\nKommunetv\nKompasVideo\nKTH\nKu6\nKukuluLive\nLastFM\nLastFMPlaylist\nLastFMUser\nlbry\nlbry:channel\nlbry:playlist\nLCI\nLecturio\nLeFigaroVideoEmbed\nLeFigaroVideoSection\nLEGO\nLemonde\nLibsyn\nLinkedIn\nLiputan6\nListenNotes\nLiTV\nLivestreamfails\nLnk\nloc\nloom\nLRTRadio\nLRTStream\nLRTVOD\nmaariv.co.il\nMagellanTV\nmailru\nmailru:music\nMainStreaming\nmangomolo:live\nmangomolo:video\nMangoTV\nMaoriTV\nmassengeschmack.tv\nMasters\nMatchiTV\nMatchTV\nmave\nmave:channel\nMBN\nMDR\nMedalTV\nmedia.ccc.de\nMediaite\nMediaKlikk\nMediaStream\nMediaWorksNZVOD\nMedici\nmegaphone.fm\n  https://player.megaphone.fm/GLT9749789991\nmegatvcom\nMeipai\nMelonVOD\nmir24.tv\nMirrorCoUK\nmixch\nmixch:archive\nmixch:movie\nmixcloud\nMixlr\nMixlrRecoring\nMLSSoccer\nMochaVideo\nMojevideo\nMonstercat\nmonstersiren\nmoviepilot\nMovingImage\nMuseScore\nMux\nMySpace\nMySpace:album\nMySpass\nMyVideoGe\nMyVidster\nn-tv.de\nN1Info:article\nNascarClassics\nNekoHacker\nNest\nNestClip\nNetAppCollection\nNetAppVideo\nNetzkino\nNewsy\nNFHSNetwork\nNhkVod\nNhkVodProgram\nniconico\nniconico:history\nniconico:playlist\nniconico:series\nniconico:tag\nNiconicoUser\nnicovideo:search\nnicovideo:search:date\nnicovideo:search_url\nNobelPrize\nNoicePodcast\nNoodleMagazine\nNOSNLArticle\nNovaEmbed\nNovaPlay\nNowCanal\nnowness\nnowness:playlist\nnowness:series\nNpr\nNRK\nNRKPlaylist\nNRKRadioPodkast\nNRKTV\nNRKTVDirekte\nNRKTVEpisode\nNRKTVSeason\nNRKTVSeries\nnts.live\nntv.ru\nNuvid\nnzherald\nNZOnScreen\nNZZ\nOdnoklassniki\nOlympicsReplay\nOmnyfm\nOmnyfmPlaylist\nOmnyfmShow\non24\nOnDemandKorea\nOneFootball\nOnePlacePodcast\nonsen\nOpencast\nOpencastPlaylist\norf:on\norf:podcast\norf:radio\nOutsideTV\nPacktPub\nPacktPubCourse\nparliamentlive.tv\nParlview\nparti:livestream\nparti:video\nPearVideo\npeer.tv\nPeerTube\nPeerTube:Playlist\nperiscope\nPGATour\nPhilharmonieDeParis\nPhotobucket\nPiaLive\npicarto\npicarto:vod\nPiksel\nPinkbike\nPinterest\nPlayerFm\nPodbayFM\nPodbayFMChannel\nPodchaser\nPopcorntimes\nPopcornTV\nPrankCast\nPrankCastPost\nPremiershipRugby\nPressTV\nProjectVeritas (CURRENTLY BROKEN)\nPyvideo\nQingTing\nradiocanada\nradiocanada:audiovideo\nradiokapital\nradiokapital:show\nRadioRadicale\nRadioZetPodcast\nRCS\nRCSEmbeds\nRCSVarious\nReddit\nRedGifs\nRedTube\nReverbNation\nRinseFM\nRinseFMArtistPlaylist\nRottenTomatoes\nRoyaLive\nRozhlasVltava\nrte\nrte:radio\nRTRFM\nRTVCKaltura\nrtve.es:alacarta\nrtve.es:audio\nrtve.es:live\nrtve.es:television\nrtvslo.si\nrtvslo.si:show\nRudoVideo\nRumble\nRumbleEmbed\nrutube\nrutube:embed\nRuv\nruv.is:spila\nS4C\nS4CSeries\nSapo\nSBS\nScreen9\nScreencast\nScreencastify\nScreenRec\nSen\nServus\nSharePoint\nShemarooMe\nshowroom:live\nshowroom:vod\nShugiinItvLive\nShugiinItvVod\nsimplecast\nsimplecast:episode\nsimplecast:podcast\nSina\nSkeb\nsky:news\nsky:news:story\nsky:sports\nsky:sports:news\nSkylineWebcams (CURRENTLY BROKEN)\nSkyNewsAU\nSnapchatSpotlight\nSohu\nSohuV\nsoundcloud\nSoundcloudEmbed\nsoundgasm\nsoundgasm:profile\nSovietsCloset\nSovietsClosetPlaylist\nSpiegel\nSport5\nSpreaker\nSpreakerShow\nSproutVideo\nSRGSSR\nSRGSSRPlay\nstartrek\nstartv\nSteam\nSteamCommunity\nSteamCommunityBroadcast\nStreaks\nStreamable\nStreamCZ\nStreetVoice\nstv:player\nstvr\nSubsplash\nsubsplash:playlist\nSubstack\nsverigesradio:episode\nsverigesradio:publication\nsvt:page\nsvt:play\nsvt:play:series\nSztvHu\nTapTapApp\nTapTapAppIntl\nTapTapMoment\nTapTapPostIntl\nTedEmbed\nTedPlaylist\nTedSeries\nTedTalk\nTele13\nTeleBruxelles\nTelegraaf\ntelegram:embed\nTeleTask (CURRENTLY BROKEN)\nTelewebion\nTF1\nTheGuardianPodcast\nTheHighWire\nTheIntercept\nTheStar\nTheSun\nThisAmericanLife\nThisOldHouse\nThreeSpeak\nThreeSpeakUser\nTikTok\ntiktok:collection\ntiktok:live\ntiktok:user\nTMZ\nTNAFlix\nTNAFlixNetworkEmbed\ntokfm:audition\ntokfm:podcast\nToonGoggles\ntoutiao\nToypics (CURRENTLY BROKEN)\nTrtCocukVideo\nTrtWorld\nTruNews\nTruth\nTumblr\ntunein:podcast\ntunein:podcast:program\ntunein:station\ntv.dfb.de\nTV2\nTV2Article\nTV2DK\nTV2DKBornholmPlay\ntv2play.hu\ntv2playseries.hu\nTV4\nTV5MONDE\ntv5unis\ntv5unis:video\nTVANouvelles\nTVANouvellesArticle\ntvaplus\ntvigle\nTVIPlayer\ntvnoe\nTVO\ntvopengr:embed\ntvopengr:watch\ntvw\ntvw:news\ntvw:tvchannels\nTweakers\ntwitch:clips\ntwitch:collection\ntwitch:stream\ntwitch:videos\ntwitch:videos:clips\ntwitch:videos:collections\ntwitch:vod\ntwitter\ntwitter:amplify\ntwitter:broadcast\ntwitter:card\ntwitter:shortener\ntwitter:spaces\nUDNEmbed\nUlizaPlayer\nUlizaPortal\numg:de\nUnistra\nUnitedNationsWebTv\nuol.com.br\nURPlay\nUSAToday\nustudio\nustudio:embed\nVbox7\nVeo\nVidea\nvideo.arnes.si\nVideoPress\nVidio\nVidioPremier\nVidLii\nVidly\nvids.io\nVidyard\nvimeo\nViMP\nViously\nVisir\nvk\nvk:uservideos\nvk:wallpost\nVKPlay\nVKPlayLive\nvm.tiktok\nVocaroo\nVODPlatform\nVTV\nVTVGo\nwashingtonpost\nwashingtonpost:article\nwat.tv\nWebcamerapl\nWebOfStories\nWebOfStoriesPlaylist\nWeibo\nWeiboVideo\nWhyp\nwikimedia.org\nWimbledon\nWistia\nWistiaChannel\nWistiaPlaylist\nWorldStarHipHop\nWSJ\nWSJArticle\nwyborcza:video\nWyborczaPodcast\nwykop:dig\nwykop:dig:comment\nwykop:post\nwykop:post:comment\nXboxClips\nXiaoHongShu\nximalaya\nximalaya:album\nXinpianchang\nXNXX\nXVideos\nxvideos:quickies\nXXXYMovies\nYandexDisk\nYappy (CURRENTLY BROKEN)\nYappyProfile\nyfanefa\nYleAreena\nYouJizz\nyouku\nyoutube\n  https://www.youtube.com/watch?v=BaW_jenozKc\nyoutube:clip\nyoutube:consent\nyoutube:favorites\nyoutube:history\nyoutube:music:search_url\nyoutube:notif\nyoutube:playlist\nyoutube:recommended\nyoutube:search\nyoutube:search_url\nyoutube:shorts:pivot:audio\nyoutube:subscriptions\nyoutube:tab\nyoutube:truncated_id\nyoutube:truncated_url\nyoutube:user\nyoutube:watchlater\nYoutubeLivestreamEmbed\nYoutubeYtBe\nZapiks\nzdf\nzdf:channel\nZenPorn\nZetlandDKArticle\nZhihu\nzoom:clips\nZype\ngeneric\n  https://example.com/v.mp4\n"
    );
    let output = run(&["--extractor-descriptions"]);
    // Upstream draws the search example's count and query at random.
    let pattern = format!(
        "^{}{}{}{}{}{}{}$",
        regex::escape(
            "17live\n17live:clip\n17live:vod\n1News: 1news.co.nz article videos\n1tv: Первый канал\n23video\n24tv.ua\n3qsdn: 3Q SDN\n3sat\n9gag: 9GAG\n9News\n9now.com.au\nabcotvs: ABC Owned Television Stations\nabcotvs:clips\nacast\nacast:channel\nAcFunVideo\nadobetv\nAeonCo\nagalega:videos\nAitubeKZVideo\nAlibaba\nAliExpressLive\nAlJazeera\nAllstar\nAllstarProfile\naltcensored\naltcensored:channel\nAmadeusTV\nAmazonStore\nAMCNetworks\nAmericasTestKitchen\nAmericasTestKitchenSeason\nAngel\nAPA\napple:music:connect: Apple Music Connect\napple:podcasts: Apple Podcasts\narchive.org: archive.org video and audio\nArcPublishing\nARDAudiothek\nARDAudiothekPlaylist\nARDMediathek\nARDMediathekCollection\nArt19\nArteTV\nArteTVCategory\nArteTVEmbed\nArteTVPlaylist\nasobichannel: ASOBI CHANNEL\nasobichannel:tag: ASOBI CHANNEL\nAtresPlayer: [atresplayer]\nAtScaleConfEvent\nAudiMedia\nAudioBoom\nAudiodraft:custom\nAudiodraft:generic\naudiomack\naudiomack:album\nAudius: Audius.co\naudius:artist: Audius.co profile/artist pages\naudius:playlist: Audius.co playlists\naudius:track: Audius track ID or API link. Prepend with \"audius:\"\nAZMedien: AZ Medien videos\nBaiduVideo: 百度视频\nBandcamp\nBeaconTv\nBeatport\nBeeg\nBerufeTV\nbfmtv\nbfmtv:article\nbfmtv:live\nBigo\nBild: Bild.de\nBiliBili\nBitChute\nBitChuteChannel\nBitmovin\nblerp\nblogger.com\nBloomberg\nBluesky\nBongaCams\nBoosty\nBostonGlobe\nBox\nBoxCastVideo\nBpb: Bundeszentrale für politische Bildung\nBreitBart\nbrightcove:new\nBTVPlus\nBundesliga\nBundestag\nBunnyCdn\nBusinessInsider\nCaltrans\nCAM4\nCanalAlpha\nCanalsurmas\nCCMA: 3Cat, TV3 and Catalunya Ràdio\nCCTV: 央视网\nCellebrite\nCGTN\nCharlieRose\nChaturbate\nChilloutzone\nCiscoLiveSearch\nCiscoLiveSession\nCJSW\nClipchamp\nCloudflareStream\nCloudyCDN\nClyp\nCNBCVideo\nCNN\nCondeNast: Condé Nast media group: Allure, Architectural Digest, Ars Technica, Bon Appétit, Brides, Condé Nast, Condé Nast Traveler, Details, Epicurious, GQ, Glamour, Golf Digest, SELF, Teen Vogue, The New Yorker, Vanity Fair, Vogue, W Magazine, WIRED\nCoub\nCozyTV\ncpac\ncpac:playlist\nCraftsy\nCrooksAndLiars\nCrtvg\nCtsNews: 華視新聞\nCultureUnplugged\ncuriositystream: [curiositystream]\ncuriositystream:collections: [curiositystream]\ncuriositystream:series: [curiositystream]\nDacastPlaylist\nDacastVOD\nDailyMail\ndailymotion: [dailymotion]\ndailymotion:playlist: [dailymotion]\ndailymotion:search: [dailymotion]\ndailymotion:user: [dailymotion]\ndamtomo:record\ndamtomo:video\ndaystar:clip\nDBTV\nDctpTv\ndemocracynow\nDetikEmbed\nDeuxM\nDeuxMNews\nDigiteka\nDiscogsReleasePlaylist\ndlf\ndlf:corpus: DLF Multi-feed Archives\ndlive:stream\ndlive:vod\nDouyin\nDropbox\nDrTalks\ndrtv\ndrtv:live\ndrtv:season\ndrtv:series\nDumpert\nDuoplay\ndvtv: http://video.aktualne.cz/\nEbaumsWorld\nEbay\negghead:course: egghead.io course\negghead:lesson: egghead.io lesson\neggs:artist\neggs:single\nElonet\nElPais: El País\nElTreceTV: El Trece TV (Argentina)\nEmbedly\nEpoch\nEporner\nErocast\nEroProfile:album\nERRArhiiv\nERRJupiter\nertflix:codename: ERTFLIX videos by codename\nEttuTv\nEuropeanTour\nEUScreen\nExpressen\nfacebook\nfacebook:ads\nfacebook:reel\nFacebookPluginsVideo\nFathom\nFaulio\nfaz.net\nfc2: [fc2]\nfc2:embed\nFczenit\nFilmArchiv: FILMARCHIV ON\nfilmon\nfilmon:channel\nFilmweb\nFiveTV\nFlickr\nFootyRoom\nFormula1\nfoxnews: Fox News and Fox Business Video\nfoxnews:article\nFoxNewsVideo\nFrancaisFacile\nfranceinfo: franceinfo.fr (formerly francetvinfo.fr)\nfrancetv\nfrancetv:site\nFreesound\nfreespeech.org\nfreetv:series\nFreeTvMovies\nFunker530\nFuyinTV\nGab\nGaia: [gaia]\nGameSpot\nGameStar\nGaskrank\nGBNews: GB News clips, features and live streams\nGenius\nGeniusLyrics\nGermanupa: germanupa.de\nGettr\nGettrStreaming\nGlide: Glide mobile video messages (glide.me)\nGlobalPlayerAudio\nGlobalPlayerAudioEpisode\nGlobalPlayerLive\nGlobalPlayerLivePlaylist\nGlobalPlayerVideo\nGlobo: [globo]\nGloboArticle\nglomex: Glomex videos\nglomex:embed: Glomex embedded videos\nGMANetworkVideo\nGodResource\nGolem\ngoodgame:stream\nGoogleDrive\nGoogleDrive:Folder\nGoPro\nGoToStage\nGraspop\nGronkh\nGroupon\nHarpodeon\nHearThisAt\nHellPorno\nHistoricFilms\nHitRecord\nHollywoodReporter\nHollywoodReporterPlaylist\nHolodex\nHSEProduct\nHSEShow\nhtml5\nHuajiao: 花椒直播\nHuffPost: Huffington Post\nhuya:live: 虎牙直播\nhuya:video: 虎牙视频\nHypem\nHytale\nIdagioAlbum\nIdagioPersonalPlaylist\nIdagioPlaylist\nIdagioRecording\nIdagioTrack\niheartradio\niheartradio:podcast\nIlPost\nIltalehti\nimdb: Internet Movie Database trailers\nimdb:list: Internet Movie Database lists\nImgur\nimgur:album\nimgur:gallery\nIna\nInc\nIndavideoEmbed\nInfoQ\nInstagram\ninstagram:story\ninstagram:tag: Instagram hashtag search URLs\nInstagramIOS: IOS instagram:// URL\nInternazionale\nIslamChannel\nIslamChannelSeries\nIsraelNationalNews\nivideon: Ivideon TV\nIvoox\niwara: [iwara]\niwara:playlist: [iwara]\niwara:user: [iwara]\nJamendo\nJamendoAlbum\nJoj\nJove\nJStream\nJWPlatform\nKaltura\nKenh14Playlist\nKenh14Video\nkick:clip\nkick:clips\nkick:live\nkick:videos\nkick:vod\nKicker\nKickStarter\nKika: KiKA.de\nKikaPlaylist\nKinoPoisk\nKommunetv\nKompasVideo\nKTH\nKu6\nKukuluLive\nLastFM\nLastFMPlaylist\nLastFMUser\nlbry: odysee.com\nlbry:channel: odysee.com channels\nlbry:playlist: odysee.com playlists\nLCI\nLecturio: [lecturio]\nLeFigaroVideoEmbed\nLeFigaroVideoSection\nLEGO\nLemonde\nLibsyn\nLinkedIn\nLiputan6\nListenNotes\nLiTV\nLivestreamfails\nLnk\nloc: Library of Congress\nloom\nLRTRadio\nLRTStream\nLRTVOD\nmaariv.co.il\nMagellanTV\nmailru: Видео@Mail.Ru\nmailru:music: Музыка@Mail.Ru\nMainStreaming: MainStreaming Player\nmangomolo:live\nmangomolo:video\nMangoTV: 芒果TV\nMaoriTV\nmassengeschmack.tv\nMasters\nMatchiTV\nMatchTV\nmave\nmave:channel\nMBN: mbn.co.kr (매일방송)\nMDR: MDR.DE\nMedalTV\nmedia.ccc.de\nMediaite\nMediaKlikk\nMediaStream\nMediaWorksNZVOD\nMedici\nmegaphone.fm: megaphone.fm embedded players\nmegatvcom: megatv.com videos\nMeipai: 美拍\nMelonVOD\nmir24.tv\nMirrorCoUK\nmixch\nmixch:archive\nmixch:movie\nmixcloud\nMixlr\nMixlrRecoring\nMLSSoccer\nMochaVideo\nMojevideo: mojevideo.sk\nMonstercat\nmonstersiren: 塞壬唱片\nmoviepilot: Moviepilot trailer\nMovingImage\nMuseScore\nMux\nMySpace\nMySpace:album\nMySpass\nMyVideoGe\nMyVidster\nn-tv.de\nN1Info:article\nNascarClassics\nNekoHacker\nNest\nNestClip\nNetAppCollection\nNetAppVideo\nNetzkino\nNewsy\nNFHSNetwork\nNhkVod\nNhkVodProgram\nniconico: [niconico] ニコニコ動画\nniconico:history: NicoNico user history or likes. Requires cookies.\nniconico:playlist\nniconico:series\nniconico:tag: NicoNico video tag URLs\nNiconicoUser\nnicovideo:search: Nico video search; \"nicosearch:\" prefix (e.g. \"nicosearch"
        ),
        "(|5|10|all):(cute kittens|slithering pythons|falling cat|angry poodle|purple fish|running tortoise|sleeping bunny|burping cow)",
        regex::escape(
            "\")\nnicovideo:search:date: Nico video search, newest first; \"nicosearchdate:\" prefix (e.g. \"nicosearchdate"
        ),
        "(|5|10|all):(cute kittens|slithering pythons|falling cat|angry poodle|purple fish|running tortoise|sleeping bunny|burping cow)",
        regex::escape(
            "\")\nnicovideo:search_url: Nico video search URLs\nNobelPrize\nNoicePodcast\nNoodleMagazine\nNOSNLArticle\nNovaEmbed\nNovaPlay\nNowCanal\nnowness\nnowness:playlist\nnowness:series\nNpr\nNRK\nNRKPlaylist\nNRKRadioPodkast\nNRKTV: NRK TV and NRK Radio\nNRKTVDirekte: NRK TV Direkte and NRK Radio Direkte\nNRKTVEpisode\nNRKTVSeason\nNRKTVSeries\nnts.live\nntv.ru\nNuvid\nnzherald\nNZOnScreen\nNZZ\nOdnoklassniki\nOlympicsReplay\nOmnyfm\nOmnyfmPlaylist\nOmnyfmShow\non24: ON24\nOnDemandKorea\nOneFootball\nOnePlacePodcast\nonsen: [onsen] インターネットラジオステーション＜音泉＞\nOpencast\nOpencastPlaylist\norf:on\norf:podcast\norf:radio\nOutsideTV\nPacktPub: [packtpub]\nPacktPubCourse\nparliamentlive.tv: UK parliament videos\nParlview\nparti:livestream\nparti:video\nPearVideo\npeer.tv\nPeerTube\nPeerTube:Playlist\nperiscope: Periscope\nPGATour\nPhilharmonieDeParis: Philharmonie de Paris\nPhotobucket\nPiaLive\npicarto\npicarto:vod\nPiksel\nPinkbike\nPinterest\nPlayerFm\nPodbayFM\nPodbayFMChannel\nPodchaser\nPopcorntimes\nPopcornTV\nPrankCast\nPrankCastPost\nPremiershipRugby\nPressTV\nPyvideo\nQingTing\nradiocanada\nradiocanada:audiovideo\nradiokapital\nradiokapital:show\nRadioRadicale\nRadioZetPodcast\nRCS\nRCSEmbeds\nRCSVarious\nReddit\nRedGifs\nRedTube\nReverbNation\nRinseFM\nRinseFMArtistPlaylist\nRottenTomatoes\nRoyaLive\nRozhlasVltava\nrte: Raidió Teilifís Éireann TV\nrte:radio: Raidió Teilifís Éireann radio\nRTRFM\nRTVCKaltura\nrtve.es:alacarta: RTVE a la carta and Play\nrtve.es:audio: RTVE audio\nrtve.es:live: RTVE.es live streams\nrtve.es:television\nrtvslo.si\nrtvslo.si:show\nRudoVideo\nRumble\nRumbleEmbed\nrutube: Rutube videos\nrutube:embed: Rutube embedded videos\nRuv\nruv.is:spila\nS4C\nS4CSeries\nSapo: SAPO Vídeos\nSBS: sbs.com.au\nScreen9\nScreencast\nScreencastify\nScreenRec\nSen\nServus\nSharePoint\nShemarooMe\nshowroom:live: SHOWROOM\nshowroom:vod\nShugiinItvLive: 衆議院インターネット審議中継\nShugiinItvVod: 衆議院インターネット審議中継 (ビデオライブラリ)\nsimplecast\nsimplecast:episode\nsimplecast:podcast\nSina\nSkeb\nsky:news\nsky:news:story\nsky:sports\nsky:sports:news\nSkyNewsAU\nSnapchatSpotlight\nSohu\nSohuV\nsoundcloud: [soundcloud]\nSoundcloudEmbed\nsoundgasm\nsoundgasm:profile\nSovietsCloset\nSovietsClosetPlaylist\nSpiegel\nSport5\nSpreaker\nSpreakerShow\nSproutVideo\nSRGSSR\nSRGSSRPlay: srf.ch, rts.ch, rsi.ch, rtr.ch and swissinfo.ch play sites\nstartrek: STAR TREK\nstartv\nSteam\nSteamCommunity\nSteamCommunityBroadcast\nStreaks\nStreamable\nStreamCZ\nStreetVoice\nstv:player\nstvr: Slovak Television and Radio (formerly RTVS)\nSubsplash\nsubsplash:playlist\nSubstack\nsverigesradio:episode\nsverigesradio:publication\nsvt:page\nsvt:play: SVT Play and Öppet arkiv\nsvt:play:series\nSztvHu\nTapTapApp\nTapTapAppIntl\nTapTapMoment\nTapTapPostIntl\nTedEmbed\nTedPlaylist\nTedSeries\nTedTalk\nTele13\nTeleBruxelles\nTelegraaf\ntelegram:embed\nTelewebion\nTF1\nTheGuardianPodcast\nTheHighWire\nTheIntercept\nTheStar\nTheSun\nThisAmericanLife\nThisOldHouse: [thisoldhouse]\nThreeSpeak\nThreeSpeakUser\nTikTok\ntiktok:collection\ntiktok:live\ntiktok:user\nTMZ\nTNAFlix\nTNAFlixNetworkEmbed\ntokfm:audition\ntokfm:podcast\nToonGoggles\ntoutiao: 今日头条\nTrtCocukVideo\nTrtWorld\nTruNews\nTruth\nTumblr: [tumblr]\ntunein:podcast\ntunein:podcast:program\ntunein:station\ntv.dfb.de\nTV2\nTV2Article\nTV2DK\nTV2DKBornholmPlay\ntv2play.hu\ntv2playseries.hu\nTV4: tv4.se and tv4play.se\nTV5MONDE\ntv5unis\ntv5unis:video\nTVANouvelles\nTVANouvellesArticle\ntvaplus: TVA+\ntvigle: Интернет-телевидение Tvigle.ru\nTVIPlayer\ntvnoe: Televize Noe\nTVO\ntvopengr:embed: tvopen.gr embedded videos\ntvopengr:watch: tvopen.gr (and ethnos.gr) videos\ntvw\ntvw:news\ntvw:tvchannels\nTweakers\ntwitch:clips: [twitch]\ntwitch:collection: [twitch]\ntwitch:stream: [twitch]\ntwitch:videos: [twitch]\ntwitch:videos:clips: [twitch]\ntwitch:videos:collections: [twitch]\ntwitch:vod: [twitch]\ntwitter\ntwitter:amplify\ntwitter:broadcast\ntwitter:card\ntwitter:shortener\ntwitter:spaces\nUDNEmbed: 聯合影音\nUlizaPlayer\nUlizaPortal: ulizaportal.jp\numg:de: Universal Music Deutschland\nUnistra\nUnitedNationsWebTv\nuol.com.br\nURPlay\nUSAToday\nustudio\nustudio:embed\nVbox7\nVeo\nVidea\nvideo.arnes.si\nVideoPress\nVidio: [vidio]\nVidioPremier: [vidio]\nVidLii\nVidly\nvids.io\nVidyard\nvimeo: [vimeo]\nViMP\nViously\nVisir: Vísir\nvk: [vk] VK\nvk:uservideos: [vk] VK - User's Videos\nvk:wallpost: [vk]\nVKPlay\nVKPlayLive\nvm.tiktok\nVocaroo\nVODPlatform\nVTV\nVTVGo\nwashingtonpost\nwashingtonpost:article\nwat.tv\nWebcamerapl\nWebOfStories\nWebOfStoriesPlaylist\nWeibo\nWeiboVideo\nWhyp\nwikimedia.org\nWimbledon\nWistia\nWistiaChannel\nWistiaPlaylist\nWorldStarHipHop\nWSJ: Wall Street Journal\nWSJArticle\nwyborcza:video\nWyborczaPodcast\nwykop:dig\nwykop:dig:comment\nwykop:post\nwykop:post:comment\nXboxClips\nXiaoHongShu: 小红书\nximalaya: 喜马拉雅FM\nximalaya:album: 喜马拉雅FM 专辑\nXinpianchang: 新片场\nXNXX\nXVideos\nxvideos:quickies\nXXXYMovies\nYandexDisk\nYappyProfile\nyfanefa\nYleAreena\nYouJizz\nyouku: 优酷\nyoutube: [youtube] YouTube\nyoutube:clip: [youtube]\nyoutube:favorites: [youtube] YouTube liked videos; \":ytfav\" keyword (requires cookies)\nyoutube:history: [youtube] Youtube watch history; \":ythis\" keyword (requires cookies)\nyoutube:music:search_url: [youtube] YouTube music search URLs with selectable sections, e.g. #songs\nyoutube:notif: [youtube] YouTube notifications; \":ytnotif\" keyword (requires cookies)\nyoutube:playlist: [youtube] YouTube playlists\nyoutube:recommended: [youtube] YouTube recommended videos; \":ytrec\" keyword\nyoutube:search: [youtube] YouTube search; \"ytsearch:\" prefix (e.g. \"ytsearch"
        ),
        "(|5|10|all):(cute kittens|slithering pythons|falling cat|angry poodle|purple fish|running tortoise|sleeping bunny|burping cow)",
        regex::escape(
            "\")\nyoutube:search_url: [youtube] YouTube search URLs with sorting and filter support\nyoutube:shorts:pivot:audio: [youtube] YouTube Shorts audio pivot (Shorts using audio of a given video)\nyoutube:subscriptions: [youtube] YouTube subscriptions feed; \":ytsubs\" keyword (requires cookies)\nyoutube:tab: [youtube] YouTube Tabs\nyoutube:user: [youtube] YouTube user videos; \"ytuser:\" prefix\nyoutube:watchlater: [youtube] Youtube watch later list; \":ytwatchlater\" keyword (requires cookies)\nYoutubeLivestreamEmbed: [youtube] YouTube livestream embeds\nYoutubeYtBe: [youtube] youtu.be\nZapiks\nzdf\nzdf:channel\nZenPorn\nZetlandDKArticle\nZhihu\nzoom:clips\nZype\ngeneric: Generic downloader that works on some sites"
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
