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
        "23video\nAngel\narchive.org\nAudioBoom\nBandcamp\nBehindKink (CURRENTLY BROKEN)\nBild\nBiliBili\nBitChute\nBitmovin\nblogger.com\nBluesky\nBreitBart\nBundesliga\nCGTN\nCJSW\nCoub\nCozyTV\nCrooksAndLiars\nCrtvg\nCultureUnplugged\ndailymotion\nDouyin\nEbaumsWorld\nEbay\nEpoch\nfacebook\nfacebook:ads\nfacebook:reel\nFacebookPluginsVideo\nFacebookRedirectURL\nFathom\nFczenit\nFilmweb\nFuyinTV\nGlide\nGodResource\nGodTube (CURRENTLY BROKEN)\nGraspop\nHarpodeon\nHistoricFilms\nHitRecord\nhtml5\nHuajiao\nHypem\nIltalehti\nImgur\nimgur:album\nimgur:gallery\nInstagram\ninstagram:story\ninstagram:tag\ninstagram:user (CURRENTLY BROKEN)\nInstagramIOS\nIsraelNationalNews\nJove\nJWPlatform\nkick:clip\nkick:clips\nkick:live\nkick:videos\nkick:vod\nKommunetv\nKu6\nlbry\nLinkedIn\nLivestreamfails\nloom\nmaariv.co.il\nMasters\nmegaphone.fm\n  https://player.megaphone.fm/GLT9749789991\nmixcloud\nMovingImage\nniconico\nNobelPrize\nOdnoklassniki\nOneFootball\nOnePlacePodcast\nParlview\nperiscope\nPhotobucket\nPinterest\nPlayerFm\nPopcornTV\nPremiershipRugby\nQingTing\nReddit\nReverbNation\nRTRFM\nRumble\nRumbleEmbed\nrutube\nrutube:embed\nScreenRec\nSen\nSnapchatSpotlight\nsoundcloud\nStreamable\nSztvHu\nTedEmbed\nTedPlaylist\nTedSeries\nTedTalk\ntelegram:embed\nTheIntercept\nThisAmericanLife\nTikTok\ntiktok:collection\ntiktok:live\ntiktok:user\nTumblr\nTweakers\ntwitch:clips\ntwitch:collection\ntwitch:stream\ntwitch:videos\ntwitch:videos:clips\ntwitch:videos:collections\ntwitch:vod\ntwitter\ntwitter:amplify\ntwitter:broadcast\ntwitter:card\ntwitter:shortener\ntwitter:spaces\nvimeo\nvk\nvk:uservideos\nvk:wallpost\nVKPlay\nVKPlayLive\nvm.tiktok\nVocaroo\nWeibo\nWeiboVideo\nXiaoHongShu\nyfanefa\nyoutube\n  https://www.youtube.com/watch?v=BaW_jenozKc\nyoutube:clip\nyoutube:consent\nyoutube:favorites\nyoutube:history\nyoutube:music:search_url\nyoutube:notif\nyoutube:playlist\nyoutube:recommended\nyoutube:search\nyoutube:search_url\nyoutube:shorts:pivot:audio\nyoutube:subscriptions\nyoutube:tab\nyoutube:truncated_id\nyoutube:truncated_url\nyoutube:user\nyoutube:watchlater\nYoutubeLivestreamEmbed\nYoutubeYtBe\nZhihu\ngeneric\n  https://example.com/v.mp4\n"
    );
    let output = run(&["--extractor-descriptions"]);
    // Upstream draws the search example's count and query at random.
    let pattern = format!(
        "^{}{}{}$",
        regex::escape(
            "23video\nAngel\narchive.org: archive.org video and audio\nAudioBoom\nBandcamp\nBild\nBiliBili\nBitChute\nBitmovin\nblogger.com\nBluesky\nBreitBart\nBundesliga\nCGTN\nCJSW\nCoub\nCozyTV\nCrooksAndLiars\nCrtvg\nCultureUnplugged\ndailymotion: [dailymotion]\nDouyin\nEbaumsWorld\nEbay\nEpoch\nfacebook\nfacebook:ads\nfacebook:reel\nFacebookPluginsVideo\nFathom\nFczenit\nFilmweb\nFuyinTV\nGlide: Glide mobile video messages (glide.me)\nGodResource\nGraspop\nHarpodeon\nHistoricFilms\nHitRecord\nhtml5\nHuajiao: 花椒直播\nHypem\nIltalehti\nImgur\nimgur:album\nimgur:gallery\nInstagram\ninstagram:story\ninstagram:tag: Instagram hashtag search URLs\nInstagramIOS: IOS instagram:// URL\nIsraelNationalNews\nJove\nJWPlatform\nkick:clip\nkick:clips\nkick:live\nkick:videos\nkick:vod\nKommunetv\nKu6\nlbry: odysee.com\nLinkedIn\nLivestreamfails\nloom\nmaariv.co.il\nMasters\nmegaphone.fm: megaphone.fm embedded players\nmixcloud\nMovingImage\nniconico: [niconico] ニコニコ動画\nNobelPrize\nOdnoklassniki\nOneFootball\nOnePlacePodcast\nParlview\nperiscope: Periscope\nPhotobucket\nPinterest\nPlayerFm\nPopcornTV\nPremiershipRugby\nQingTing\nReddit\nReverbNation\nRTRFM\nRumble\nRumbleEmbed\nrutube: Rutube videos\nrutube:embed: Rutube embedded videos\nScreenRec\nSen\nSnapchatSpotlight\nsoundcloud: [soundcloud]\nStreamable\nSztvHu\nTedEmbed\nTedPlaylist\nTedSeries\nTedTalk\ntelegram:embed\nTheIntercept\nThisAmericanLife\nTikTok\ntiktok:collection\ntiktok:live\ntiktok:user\nTumblr: [tumblr]\nTweakers\ntwitch:clips: [twitch]\ntwitch:collection: [twitch]\ntwitch:stream: [twitch]\ntwitch:videos: [twitch]\ntwitch:videos:clips: [twitch]\ntwitch:videos:collections: [twitch]\ntwitch:vod: [twitch]\ntwitter\ntwitter:amplify\ntwitter:broadcast\ntwitter:card\ntwitter:shortener\ntwitter:spaces\nvimeo: [vimeo]\nvk: [vk] VK\nvk:uservideos: [vk] VK - User's Videos\nvk:wallpost: [vk]\nVKPlay\nVKPlayLive\nvm.tiktok\nVocaroo\nWeibo\nWeiboVideo\nXiaoHongShu: 小红书\nyfanefa\nyoutube: [youtube] YouTube\nyoutube:clip: [youtube]\nyoutube:favorites: [youtube] YouTube liked videos; \":ytfav\" keyword (requires cookies)\nyoutube:history: [youtube] Youtube watch history; \":ythis\" keyword (requires cookies)\nyoutube:music:search_url: [youtube] YouTube music search URLs with selectable sections, e.g. #songs\nyoutube:notif: [youtube] YouTube notifications; \":ytnotif\" keyword (requires cookies)\nyoutube:playlist: [youtube] YouTube playlists\nyoutube:recommended: [youtube] YouTube recommended videos; \":ytrec\" keyword\nyoutube:search: [youtube] YouTube search; \"ytsearch:\" prefix (e.g. \"ytsearch"
        ),
        "(|5|10|all):(cute kittens|slithering pythons|falling cat|angry poodle|purple fish|running tortoise|sleeping bunny|burping cow)",
        regex::escape(
            "\")\nyoutube:search_url: [youtube] YouTube search URLs with sorting and filter support\nyoutube:shorts:pivot:audio: [youtube] YouTube Shorts audio pivot (Shorts using audio of a given video)\nyoutube:subscriptions: [youtube] YouTube subscriptions feed; \":ytsubs\" keyword (requires cookies)\nyoutube:tab: [youtube] YouTube Tabs\nyoutube:user: [youtube] YouTube user videos; \"ytuser:\" prefix\nyoutube:watchlater: [youtube] Youtube watch later list; \":ytwatchlater\" keyword (requires cookies)\nYoutubeLivestreamEmbed: [youtube] YouTube livestream embeds\nYoutubeYtBe: [youtube] youtu.be\nZhihu\ngeneric: Generic downloader that works on some sites"
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
