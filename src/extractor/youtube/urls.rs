//! Upstream YouTube URL patterns (`yt_dlp/extractor/youtube/_base.py` and
//! `_video.py`): `YoutubeIE._VALID_URL` with `_INVIDIOUS_SITES`,
//! `_PLAYLIST_ID_RE`, and Python's `parse_qs`/`urlencode` for them.
//! Patterns are matched like `re.match`: anchored at the start only.
use regex::Regex;
use std::sync::LazyLock;

/// `_PLAYLIST_ID_RE`.
pub(crate) const PLAYLIST_ID_RE: &str =
    r"(?:(?:PL|LL|EC|UU|FL|RD|UL|TL|PU|OLAK5uy_)[0-9A-Za-z_\-]{10,}|RDMM|WL|LL|LM)";

/// `_INVIDIOUS_SITES` (pinned upstream `_base.py`), as regex fragments.
pub(crate) const INVIDIOUS_SITES: [&str; 125] = [
    r"(?:www\.)?redirect\.invidious\.io",
    r"(?:(?:www|dev)\.)?invidio\.us",
    r"(?:www\.)?invidious\.pussthecat\.org",
    r"(?:www\.)?invidious\.zee\.li",
    r"(?:www\.)?invidious\.ethibox\.fr",
    r"(?:www\.)?iv\.ggtyler\.dev",
    r"(?:www\.)?inv\.vern\.i2p",
    r"(?:www\.)?am74vkcrjp2d5v36lcdqgsj2m6x36tbrkhsruoegwfcizzabnfgf5zyd\.onion",
    r"(?:www\.)?inv\.riverside\.rocks",
    r"(?:www\.)?invidious\.silur\.me",
    r"(?:www\.)?inv\.bp\.projectsegfau\.lt",
    r"(?:www\.)?invidious\.g4c3eya4clenolymqbpgwz3q3tawoxw56yhzk4vugqrl6dtu3ejvhjid\.onion",
    r"(?:www\.)?invidious\.slipfox\.xyz",
    r"(?:www\.)?invidious\.esmail5pdn24shtvieloeedh7ehz3nrwcdivnfhfcedl7gf4kwddhkqd\.onion",
    r"(?:www\.)?inv\.vernccvbvyi5qhfzyqengccj7lkove6bjot2xhh5kajhwvidqafczrad\.onion",
    r"(?:www\.)?invidious\.tiekoetter\.com",
    r"(?:www\.)?iv\.odysfvr23q5wgt7i456o5t3trw2cw5dgn56vbjfbq2m7xsc5vqbqpcyd\.onion",
    r"(?:www\.)?invidious\.nerdvpn\.de",
    r"(?:www\.)?invidious\.weblibre\.org",
    r"(?:www\.)?inv\.odyssey346\.dev",
    r"(?:www\.)?invidious\.dhusch\.de",
    r"(?:www\.)?iv\.melmac\.space",
    r"(?:www\.)?watch\.thekitty\.zone",
    r"(?:www\.)?invidious\.privacydev\.net",
    r"(?:www\.)?ng27owmagn5amdm7l5s3rsqxwscl5ynppnis5dqcasogkyxcfqn7psid\.onion",
    r"(?:www\.)?invidious\.drivet\.xyz",
    r"(?:www\.)?vid\.priv\.au",
    r"(?:www\.)?euxxcnhsynwmfidvhjf6uzptsmh4dipkmgdmcmxxuo7tunp3ad2jrwyd\.onion",
    r"(?:www\.)?inv\.vern\.cc",
    r"(?:www\.)?invidious\.esmailelbob\.xyz",
    r"(?:www\.)?invidious\.sethforprivacy\.com",
    r"(?:www\.)?yt\.oelrichsgarcia\.de",
    r"(?:www\.)?yt\.artemislena\.eu",
    r"(?:www\.)?invidious\.flokinet\.to",
    r"(?:www\.)?invidious\.baczek\.me",
    r"(?:www\.)?y\.com\.sb",
    r"(?:www\.)?invidious\.epicsite\.xyz",
    r"(?:www\.)?invidious\.lidarshield\.cloud",
    r"(?:www\.)?yt\.funami\.tech",
    r"(?:www\.)?invidious\.3o7z6yfxhbw7n3za4rss6l434kmv55cgw2vuziwuigpwegswvwzqipyd\.onion",
    r"(?:www\.)?osbivz6guyeahrwp2lnwyjk2xos342h4ocsxyqrlaopqjuhwn2djiiyd\.onion",
    r"(?:www\.)?u2cvlit75owumwpy4dj2hsmvkq7nvrclkpht7xgyye2pyoxhpmclkrad\.onion",
    r"(?:(?:www|no)\.)?invidiou\.sh",
    r"(?:(?:www|fi)\.)?invidious\.snopyta\.org",
    r"(?:www\.)?invidious\.kabi\.tk",
    r"(?:www\.)?invidious\.mastodon\.host",
    r"(?:www\.)?invidious\.zapashcanon\.fr",
    r"(?:www\.)?(?:invidious(?:-us)?|piped)\.kavin\.rocks",
    r"(?:www\.)?invidious\.tinfoil-hat\.net",
    r"(?:www\.)?invidious\.himiko\.cloud",
    r"(?:www\.)?invidious\.reallyancient\.tech",
    r"(?:www\.)?invidious\.tube",
    r"(?:www\.)?invidiou\.site",
    r"(?:www\.)?invidious\.site",
    r"(?:www\.)?invidious\.xyz",
    r"(?:www\.)?invidious\.nixnet\.xyz",
    r"(?:www\.)?invidious\.048596\.xyz",
    r"(?:www\.)?invidious\.drycat\.fr",
    r"(?:www\.)?inv\.skyn3t\.in",
    r"(?:www\.)?tube\.poal\.co",
    r"(?:www\.)?tube\.connect\.cafe",
    r"(?:www\.)?vid\.wxzm\.sx",
    r"(?:www\.)?vid\.mint\.lgbt",
    r"(?:www\.)?vid\.puffyan\.us",
    r"(?:www\.)?yewtu\.be",
    r"(?:www\.)?yt\.elukerio\.org",
    r"(?:www\.)?yt\.lelux\.fi",
    r"(?:www\.)?invidious\.ggc-project\.de",
    r"(?:www\.)?yt\.maisputain\.ovh",
    r"(?:www\.)?ytprivate\.com",
    r"(?:www\.)?invidious\.13ad\.de",
    r"(?:www\.)?invidious\.toot\.koeln",
    r"(?:www\.)?invidious\.fdn\.fr",
    r"(?:www\.)?watch\.nettohikari\.com",
    r"(?:www\.)?invidious\.namazso\.eu",
    r"(?:www\.)?invidious\.silkky\.cloud",
    r"(?:www\.)?invidious\.exonip\.de",
    r"(?:www\.)?invidious\.riverside\.rocks",
    r"(?:www\.)?invidious\.blamefran\.net",
    r"(?:www\.)?invidious\.moomoo\.de",
    r"(?:www\.)?ytb\.trom\.tf",
    r"(?:www\.)?yt\.cyberhost\.uk",
    r"(?:www\.)?kgg2m7yk5aybusll\.onion",
    r"(?:www\.)?qklhadlycap4cnod\.onion",
    r"(?:www\.)?axqzx4s6s54s32yentfqojs3x5i7faxza6xo3ehd4bzzsg2ii4fv2iid\.onion",
    r"(?:www\.)?c7hqkpkpemu6e7emz5b4vyz7idjgdvgaaa3dyimmeojqbgpea3xqjoid\.onion",
    r"(?:www\.)?fz253lmuao3strwbfbmx46yu7acac2jz27iwtorgmbqlkurlclmancad\.onion",
    r"(?:www\.)?invidious\.l4qlywnpwqsluw65ts7md3khrivpirse744un3x7mlskqauz5pyuzgqd\.onion",
    r"(?:www\.)?owxfohz4kjyv25fvlqilyxast7inivgiktls3th44jhk3ej3i7ya\.b32\.i2p",
    r"(?:www\.)?4l2dgddgsrkf2ous66i6seeyi6etzfgrue332grh2n7madpwopotugyd\.onion",
    r"(?:www\.)?w6ijuptxiku4xpnnaetxvnkc5vqcdu7mgns2u77qefoixi63vbvnpnqd\.onion",
    r"(?:www\.)?kbjggqkzv65ivcqj6bumvp337z6264huv5kpkwuv6gu5yjiskvan7fad\.onion",
    r"(?:www\.)?grwp24hodrefzvjjuccrkw3mjq4tzhaaq32amf33dzpmuxe7ilepcmad\.onion",
    r"(?:www\.)?hpniueoejy4opn7bc4ftgazyqjoeqwlvh2uiku2xqku6zpoa4bf5ruid\.onion",
    r"(?:www\.)?piped\.kavin\.rocks",
    r"(?:www\.)?piped\.tokhmi\.xyz",
    r"(?:www\.)?piped\.syncpundit\.io",
    r"(?:www\.)?piped\.mha\.fi",
    r"(?:www\.)?watch\.whatever\.social",
    r"(?:www\.)?piped\.garudalinux\.org",
    r"(?:www\.)?piped\.rivo\.lol",
    r"(?:www\.)?piped-libre\.kavin\.rocks",
    r"(?:www\.)?yt\.jae\.fi",
    r"(?:www\.)?piped\.mint\.lgbt",
    r"(?:www\.)?il\.ax",
    r"(?:www\.)?piped\.esmailelbob\.xyz",
    r"(?:www\.)?piped\.projectsegfau\.lt",
    r"(?:www\.)?piped\.privacydev\.net",
    r"(?:www\.)?piped\.palveluntarjoaja\.eu",
    r"(?:www\.)?piped\.smnz\.de",
    r"(?:www\.)?piped\.adminforge\.de",
    r"(?:www\.)?watch\.whatevertinfoil\.de",
    r"(?:www\.)?piped\.qdi\.fi",
    r"(?:(?:www|cf)\.)?piped\.video",
    r"(?:www\.)?piped\.aeong\.one",
    r"(?:www\.)?piped\.moomoo\.me",
    r"(?:www\.)?piped\.chauvet\.pro",
    r"(?:www\.)?watch\.leptons\.xyz",
    r"(?:www\.)?pd\.vern\.cc",
    r"(?:www\.)?piped\.hostux\.net",
    r"(?:www\.)?piped\.lunar\.icu",
    r"(?:www\.)?hyperpipe\.surge\.sh",
    r"(?:www\.)?hyperpipe\.esmailelbob\.xyz",
    r"(?:www\.)?listen\.whatever\.social",
    r"(?:www\.)?music\.adminforge\.de",
];

/// `'|'.join(_INVIDIOUS_SITES)`.
pub(crate) fn invidious() -> String {
    INVIDIOUS_SITES.join("|")
}

const ID_CHARS: &str = r"0-9A-Za-z_\-";

/// `(?!WORD|…)[0-9A-Za-z_-]{N}` for words of length `N`, without
/// look-around: every `N`-character ID except the words.
fn id_except(words: &[&str], length: usize) -> String {
    if words.is_empty() {
        return format!("[{ID_CHARS}]{{{length}}}");
    }
    let mut firsts: Vec<char> = words
        .iter()
        .filter_map(|word| word.chars().next())
        .collect();
    firsts.sort_unstable();
    firsts.dedup();
    let excluded: String = firsts.iter().collect();
    let rest = if length > 1 {
        format!("[{ID_CHARS}]{{{}}}", length - 1)
    } else {
        String::new()
    };
    let mut branches = vec![format!("[[{ID_CHARS}]--[{excluded}]]{rest}")];
    if length > 1 {
        for first in firsts {
            let tails: Vec<&str> = words
                .iter()
                .filter_map(|word| word.strip_prefix(first))
                .collect();
            branches.push(format!("{first}{}", id_except(&tails, length - 1)));
        }
    }
    format!("(?:{})", branches.join("|"))
}

/// `YoutubeIE._VALID_URL` with its optional URL part: one ID group per
/// alternative (`v/`, `embed/`… IDs exclude `videoseries` and `live_stream`).
static VIDEO_URL: LazyLock<Regex> = LazyLock::new(|| {
    let id = format!("[{ID_CHARS}]{{11}}");
    let path_id = id_except(&["videoseries", "live_stream"], 11);
    let invidious = invidious();
    Regex::new(&format!(
        r"^(?:https?://|//)(?:(?:(?:(?:\w+\.)?[yY][oO][uU][tT][uU][bB][eE](?:-nocookie|kids)?\.com|(?:www\.)?deturl\.com/www\.youtube\.com|(?:www\.)?pwnyoutube\.com|(?:www\.)?hooktube\.com|(?:www\.)?yourepeat\.com|tube\.majestyc\.net|{invidious}|youtube\.googleapis\.com)/(?:.*?#/)?(?:(?:v|embed|e|shorts|live)/(?P<path>{path_id})|(?:(?:watch|movie)(?:_popup)?(?:\.php)?/?)?(?:\?|#!?)(?:.*?[&;])??v=(?P<query>{id})))|(?:youtu\.be|vid\.plus|zwearz\.com/watch|{invidious})/(?P<short>{id})|(?:www\.)?cleanvideosearch\.com/media/action/yt/watch\?videoId=(?P<clean>{id}))"
    ))
    .expect("static pattern")
});

/// `YoutubeIE._VALID_URL`'s naked ID: `(?P<id>…{11})(?:\#|$)`.
static NAKED_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"^([{ID_CHARS}]{{11}})(?:#|\n?$)")).unwrap());

/// `YoutubeIE._match_id`.
pub(crate) fn video_id(url: &str) -> Option<String> {
    if let Some(found) = VIDEO_URL.captures(url) {
        let id = ["path", "query", "short", "clean"]
            .iter()
            .find_map(|name| found.name(name))?;
        // `(?(1).+)?(?:\#|$)`: the rest of the line, then its end (or `#`).
        let rest = &url[id.end()..];
        let line = rest.split('\n').next().unwrap_or("");
        return (line.len() + 1 >= rest.len() || line.contains('#'))
            .then(|| id.as_str().to_owned());
    }
    NAKED_ID.captures(url).map(|found| found[1].to_owned())
}

/// `YoutubeIE.suitable`: no `list` query parameter and a `_VALID_URL` match.
pub(crate) fn video_suitable(url: &str) -> bool {
    first_qs(url, "list").is_none() && video_id(url).is_some()
}

/// Python `urllib.parse.parse_qs(urlparse(url).query)`: blank values
/// dropped, repeated keys grouped in first-seen order.
pub(crate) fn parse_qs(url: &str) -> Vec<(String, Vec<String>)> {
    let without_fragment = url.split('#').next().unwrap_or("");
    let query = without_fragment
        .split_once('?')
        .map_or("", |(_, query)| query);
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        if value.is_empty() {
            continue;
        }
        match groups.iter_mut().find(|(known, _)| *known == key) {
            Some((_, values)) => values.push(value.into_owned()),
            None => groups.push((key.into_owned(), vec![value.into_owned()])),
        }
    }
    groups
}

/// `parse_qs(url).get(key, [None])[0]`.
pub(crate) fn first_qs(url: &str, key: &str) -> Option<String> {
    parse_qs(url)
        .into_iter()
        .find(|(name, _)| name == key)
        .and_then(|(_, values)| values.into_iter().next())
}

/// `urlencode(query, doseq=True)` (`quote_plus`).
pub(crate) fn urlencode(groups: &[(String, Vec<String>)]) -> String {
    let quote = super::super::html5::quote_plus;
    groups
        .iter()
        .flat_map(|(key, values)| {
            values
                .iter()
                .map(move |value| format!("{}={}", quote(key), quote(value)))
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// `is_music_url`: `(https?://)?music\.youtube\.com/`.
pub(crate) fn is_music_url(url: &str) -> bool {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    rest.starts_with("music.youtube.com/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_ids_follow_upstream_valid_url() {
        for input in [
            "https://www.youtube.com/watch?v=BaW_jenozKc&t=3",
            "BaW_jenozKc",
            "BaW_jenozKc#t=1",
            "//www.YouTube.com/v/BaW_jenozKc?x",
            "https://www.youtube.com/e/BaW_jenozKc",
            "http://www.youtube.com/watch_popup?v=BaW_jenozKc",
            "http://www.youtube.com/watch.php?v=BaW_jenozKc",
            "http://www.youtube.com/movie?v=BaW_jenozKc",
            "https://www.youtube.com/#/watch?v=BaW_jenozKc",
            "https://www.youtube.com/#!v=BaW_jenozKc",
            "https://gaming.youtube.com/watch?v=BaW_jenozKc",
            "https://www.youtubekids.com/watch?v=BaW_jenozKc",
            "https://youtube.googleapis.com/v/BaW_jenozKc",
            "http://hooktube.com/watch?v=BaW_jenozKc",
            "http://www.yourepeat.com/watch/?v=BaW_jenozKc",
            "https://vid.plus/BaW_jenozKc",
            "http://zwearz.com/watch/BaW_jenozKc",
            "https://yewtu.be/BaW_jenozKc",
            "https://invidio.us/watch?v=BaW_jenozKc",
            "https://www.cleanvideosearch.com/media/action/yt/watch?videoId=BaW_jenozKc",
            "https://youtu.be/BaW_jenozKc/extra",
            "https://youtube.com/watch?v=BaW_jenozKc&v=OtherVideo1",
            "https://www.youtube.com/embed/videoseries#/watch?v=BaW_jenozKc",
        ] {
            assert_eq!(video_id(input).as_deref(), Some("BaW_jenozKc"), "{input}");
            assert!(video_suitable(input), "{input}");
        }
        for input in [
            "https://www.youtube.com/embed/videoseries?list=PL0123456789",
            "https://www.youtube.com/embed/live_stream?channel=UC0123",
            "https://youtube.com.evil.invalid/x?v=BaW_jenozKc",
            "https://example.com/watch?v=BaW_jenozKc",
            "BaW_jenozKc0",
            "https://youtube.com/watch?v=bad",
            // Taken by `youtube:tab`, which follows its `v`.
            "https://www.youtube.com/user/x?feature=a&v=BaW_jenozKc",
        ] {
            assert!(video_id(input).is_none(), "{input}");
        }
        assert_eq!(
            video_id("https://www.youtube.com/embed/videoserieS").as_deref(),
            Some("videoserieS")
        );
        assert!(!video_suitable(
            "https://youtu.be/BaW_jenozKc?list=PL0123456789"
        ));
        assert!(video_suitable("https://youtu.be/BaW_jenozKc?list="));
    }

    #[test]
    fn query_helpers_follow_python() {
        assert_eq!(
            parse_qs("//h/p?a=1&b=&a=2&c=x+y#d=4"),
            vec![
                ("a".to_owned(), vec!["1".to_owned(), "2".to_owned()]),
                ("c".to_owned(), vec!["x y".to_owned()]),
            ]
        );
        assert_eq!(
            urlencode(&[("a b".to_owned(), vec!["~/=".to_owned()])]),
            "a+b=~%2F%3D"
        );
        assert!(is_music_url("music.youtube.com/watch?v=x"));
        assert!(!is_music_url("https://www.youtube.com/"));
    }
}
