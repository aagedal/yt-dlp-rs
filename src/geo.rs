//! Upstream's geo restriction bypass (`InfoExtractor._initialize_geo_bypass`
//! and `__maybe_fake_ip_and_retry`, `utils.GeoUtils`): a fake
//! `X-Forwarded-For` address from `--xff` or an extractor's
//! `_GEO_COUNTRIES`/`_GEO_IP_BLOCKS`, sent with every extraction request
//! and copied to the formats' `http_headers`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Mutex;

/// An extractor's `_GEO_BYPASS`, `_GEO_COUNTRIES` and `_GEO_IP_BLOCKS`.
#[derive(Debug, Clone, Copy)]
pub struct GeoContext {
    pub bypass: bool,
    pub countries: &'static [&'static str],
    pub ip_blocks: &'static [&'static str],
}

impl Default for GeoContext {
    fn default() -> Self {
        Self::NONE
    }
}

impl GeoContext {
    /// `InfoExtractor`'s defaults: bypassable, without countries or blocks.
    pub const NONE: Self = Self {
        bypass: true,
        countries: &[],
        ip_blocks: &[],
    };

    /// `_GEO_COUNTRIES = COUNTRIES`.
    pub const fn countries(countries: &'static [&'static str]) -> Self {
        Self {
            bypass: true,
            countries,
            ip_blocks: &[],
        }
    }

    /// `_GEO_BYPASS = False` (with any `_GEO_COUNTRIES`).
    pub const fn no_bypass(countries: &'static [&'static str]) -> Self {
        Self {
            bypass: false,
            countries,
            ip_blocks: &[],
        }
    }
}

/// The `geo_bypass`, `geo_bypass_country` and `geo_bypass_ip_block`
/// parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeoOptions {
    pub bypass: bool,
    pub country: Option<String>,
    pub ip_block: Option<String>,
}

impl Default for GeoOptions {
    fn default() -> Self {
        Self {
            bypass: true,
            country: None,
            ip_block: None,
        }
    }
}

impl GeoOptions {
    /// Upstream `validate_options` for `--xff VALUE` (the shared
    /// `geo_bypass` destination of `--geo-bypass`, `--no-geo-bypass`,
    /// `--geo-bypass-country` and `--geo-bypass-ip-block`): `default`,
    /// `never`, a two-letter country code or a CIDR block, else
    /// `Unsupported --xff "VALUE"`.
    pub fn parse(value: &str) -> Result<Self, String> {
        let lower = value.to_lowercase();
        let mut options = Self {
            bypass: lower != "never",
            ..Self::default()
        };
        if lower != "default" && lower != "never" {
            if random_ipv4(value).is_err() {
                return Err(format!("Unsupported --xff \"{value}\""));
            }
            if value.chars().count() == 2 {
                options.country = Some(value.to_owned());
            } else {
                options.ip_block = Some(value.to_owned());
            }
        }
        Ok(options)
    }
}

static OPTIONS: Mutex<Option<GeoOptions>> = Mutex::new(None);

/// Set the run's geo bypass parameters.
pub fn set_options(options: GeoOptions) {
    *OPTIONS.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(options);
}

fn options() -> GeoOptions {
    OPTIONS
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone()
        .unwrap_or_default()
}

/// Each extractor instance's `_x_forwarded_for_ip`, kept for the run like
/// upstream's cached instances.
static FAKE_IPS: Mutex<Option<HashMap<&'static str, String>>> = Mutex::new(None);

fn stored_ip(key: &'static str) -> Option<String> {
    FAKE_IPS
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .as_ref()
        .and_then(|ips| ips.get(key).cloned())
}

fn store_ip(key: &'static str, ip: Option<String>) {
    let mut ips = FAKE_IPS.lock().unwrap_or_else(|poison| poison.into_inner());
    let ips = ips.get_or_insert_with(HashMap::new);
    match ip {
        Some(ip) => ips.insert(key, ip),
        None => ips.remove(key),
    };
}

/// The extractor whose extraction is running.
struct Current {
    key: &'static str,
    context: GeoContext,
    /// `IE_NAME`, from its `Extracting URL` line.
    name: Option<String>,
    initialized: bool,
}

thread_local! {
    static CURRENT: RefCell<Option<Current>> = const { RefCell::new(None) };
    /// The countries of the extraction's last `GeoRestrictedError`.
    static RESTRICTED: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

/// Before an extractor's `extract`: its geo context, initialized when it
/// announces `Extracting URL` (upstream `initialize()` runs just before).
pub(crate) fn begin(key: &'static str, context: GeoContext) {
    CURRENT.with(|current| {
        *current.borrow_mut() = Some(Current {
            key,
            context,
            name: None,
            initialized: false,
        });
    });
    RESTRICTED.with(|restricted| restricted.borrow_mut().take());
}

/// After an extractor that did not handle the URL.
pub(crate) fn abandon() {
    CURRENT.with(|current| {
        let mut current = current.borrow_mut();
        if current.as_ref().is_some_and(|current| !current.initialized) {
            current.take();
        }
    });
}

/// Before an extraction is retried: `Extracting URL` initializes again.
pub(crate) fn restart() {
    CURRENT.with(|current| {
        if let Some(current) = current.borrow_mut().as_mut() {
            current.initialized = false;
        }
    });
}

/// A `[NAME] Extracting URL: ...` line: the extractor's `initialize()`.
pub(crate) fn on_screen(line: &str) {
    let Some(name) = line
        .strip_prefix('[')
        .and_then(|rest| rest.split_once("] Extracting URL: "))
        .map(|(name, _)| name)
    else {
        return;
    };
    let pending = CURRENT.with(|current| {
        let mut current = current.borrow_mut();
        let current = current.as_mut().filter(|current| !current.initialized)?;
        current.initialized = true;
        current.name = Some(name.to_owned());
        Some((current.key, current.context))
    });
    if let Some((key, context)) = pending {
        initialize(
            key,
            name,
            context.countries,
            context.ip_blocks,
            context.bypass,
        );
    }
}

fn initialize(key: &'static str, name: &str, countries: &[&str], ip_blocks: &[&str], bypass: bool) {
    if stored_ip(key).is_some() {
        return;
    }
    let options = options();
    if !options.bypass {
        return;
    }
    // Path 1: an IP block, the user's regardless of the extractor.
    let ip_block = options
        .ip_block
        .clone()
        .filter(|block| !block.is_empty())
        .or_else(|| (bypass && !ip_blocks.is_empty()).then(|| choice(ip_blocks).to_owned()));
    if let Some(block) = ip_block {
        let ip = random_ipv4(&block).ok().flatten();
        crate::screen::debug(&format!(
            "[{name}] Using fake IP {} as X-Forwarded-For",
            ip.as_deref().unwrap_or("None")
        ));
        store_ip(key, ip);
        return;
    }
    // Path 2: a country code.
    let country = options
        .country
        .clone()
        .filter(|country| !country.is_empty())
        .or_else(|| (bypass && !countries.is_empty()).then(|| choice(countries).to_owned()));
    if let Some(country) = country {
        let ip = random_ipv4(&country).ok().flatten();
        crate::screen::debug(&format!(
            "Using fake IP {} ({}) as X-Forwarded-For",
            ip.as_deref().unwrap_or("None"),
            country.to_uppercase()
        ));
        store_ip(key, ip);
    }
}

/// The running extractor's `_x_forwarded_for_ip`.
pub(crate) fn fake_ip() -> Option<String> {
    let key = CURRENT.with(|current| current.borrow().as_ref().map(|current| current.key))?;
    stored_ip(key)
}

/// `ie_result['__x_forwarded_for_ip']`, removed by processing.
pub(crate) const FAKE_IP_FIELD: &str = "__x_forwarded_for_ip";

/// `extract`: the result carries the extractor's fake IP; a playlist's
/// entries inherit it (`__process_playlist`) unless they have their own.
pub(crate) fn attach(result: &mut crate::ExtractResult) {
    let Some(ip) = fake_ip() else {
        return;
    };
    match result {
        crate::ExtractResult::Video(video) => {
            video.extra.insert(FAKE_IP_FIELD.into(), ip.into());
        }
        crate::ExtractResult::Playlist(playlist) => {
            for entry in &mut playlist.entries {
                if !entry.extra.contains_key(FAKE_IP_FIELD) {
                    entry.extra.insert(FAKE_IP_FIELD.into(), ip.clone().into());
                }
            }
        }
    }
}

/// A selected format's `X-Forwarded-For` (`_calc_headers`), which the
/// native downloaders send with its media requests.
pub(crate) fn forwarded_for(info: &crate::VideoInfo) -> Option<&str> {
    info.extra
        .get("http_headers")?
        .get("X-Forwarded-For")?
        .as_str()
}

/// `_request_webpage`: `headers.setdefault('X-Forwarded-For', ip)`.
pub(crate) fn add_header(
    request: reqwest::blocking::RequestBuilder,
    headers: &[(&str, &str)],
) -> reqwest::blocking::RequestBuilder {
    match fake_ip() {
        Some(ip)
            if !headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("x-forwarded-for")) =>
        {
            request.header("X-Forwarded-For", ip)
        }
        _ => request,
    }
}

/// `raise_geo_restricted(msg, countries)` (`GeoRestrictedError`): the
/// error `YoutubeDL` reports, `TEXT` (already prefixed `[EXTRACTOR] ID: `)
/// with the countries' names and the VPN hint, remembered for
/// [`retry_with_fake_ip`].
pub(crate) fn restricted(text: &str, countries: &[&str]) -> crate::Error {
    RESTRICTED.with(|restricted| {
        *restricted.borrow_mut() = Some(countries.iter().map(|code| (*code).to_owned()).collect())
    });
    let mut message = text.to_owned();
    if !countries.is_empty() {
        // Unknown codes (a `TypeError` upstream) are shown as given.
        let names: Vec<&str> = countries
            .iter()
            .map(|code| country_name(code).unwrap_or(code))
            .collect();
        message.push_str(&format!(
            "\nThis video is available in {}.",
            names.join(", ")
        ));
    }
    message
        .push_str("\nYou might want to use a VPN or a proxy server (with --proxy) to workaround.");
    crate::Error::Reported(message)
}

/// `raise_geo_restricted(msg, countries, metadata_available=True)`: a
/// `[EXTRACTOR] MSG` warning with `--ignore-no-formats-error` (or
/// `--wait-for-video`), else the error (`prefix` is its
/// `[EXTRACTOR] ID: `).
pub(crate) fn restricted_metadata_available(
    extractor: &str,
    prefix: &str,
    message: &str,
    countries: &[&str],
) -> crate::Result<()> {
    if crate::no_formats::ignored() {
        crate::screen::warning(&format!("[{extractor}] {message}"));
        return Ok(());
    }
    Err(restricted(&format!("{prefix}{message}"), countries))
}

/// `extract`'s `except GeoRestrictedError` (`__maybe_fake_ip_and_retry`):
/// after a geo restriction with countries, a fake IP from one of them and
/// the `Retrying extraction` warning, unless one is set or bypassing is
/// off.
pub(crate) fn retry_with_fake_ip() -> bool {
    let Some(countries) = RESTRICTED.with(|restricted| restricted.borrow_mut().take()) else {
        return false;
    };
    let Some((key, name, bypass)) = CURRENT.with(|current| {
        current
            .borrow()
            .as_ref()
            .map(|current| (current.key, current.name.clone(), current.context.bypass))
    }) else {
        return false;
    };
    let options = options();
    if options
        .country
        .as_deref()
        .is_some_and(|country| !country.is_empty())
        || !bypass
        || !options.bypass
        || stored_ip(key).is_some()
        || countries.is_empty()
    {
        return false;
    }
    let countries: Vec<&str> = countries.iter().map(String::as_str).collect();
    let country = choice(&countries);
    let Some(ip) = random_ipv4(country).ok().flatten() else {
        return false;
    };
    crate::screen::warning(&format!(
        "[{}] Video is geo restricted. Retrying extraction with fake IP {ip} ({}) as X-Forwarded-For.",
        name.as_deref().unwrap_or(key),
        country.to_uppercase()
    ));
    store_ip(key, Some(ip));
    true
}

/// `ISO3166Utils.short2full`.
pub(crate) fn country_name(code: &str) -> Option<&'static str> {
    let code = code.to_uppercase();
    include_str!("data/iso3166.txt")
        .lines()
        .find_map(|line| line.split_once('\t').filter(|(short, _)| *short == code))
        .map(|(_, name)| name)
}

/// A uniform `random.randint`/`random.choice` index from a clock-seeded
/// xorshift64.
fn random_below(bound: u64) -> u64 {
    use std::hash::{BuildHasher, Hasher};
    static STATE: Mutex<u64> = Mutex::new(0);
    let mut state = STATE.lock().unwrap_or_else(|poison| poison.into_inner());
    if *state == 0 {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |time| time.as_nanos()),
        );
        *state = hasher.finish() | 1;
    }
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    if bound == 0 { 0 } else { x % bound }
}

fn choice<'a>(items: &[&'a str]) -> &'a str {
    items[random_below(items.len() as u64) as usize]
}

/// `GeoUtils.random_ipv4(code_or_block)`: a random address in the country's
/// major block or a CIDR block; `Ok(None)` for an unknown country code and
/// `Err` where upstream raises (`split('/')`, `int` or `inet_aton`).
pub(crate) fn random_ipv4(code_or_block: &str) -> Result<Option<String>, ()> {
    let block = if code_or_block.chars().count() == 2 {
        let code = code_or_block.to_uppercase();
        match COUNTRY_IP_MAP.iter().find(|(country, _)| *country == code) {
            Some((_, block)) => *block,
            None => return Ok(None),
        }
    } else {
        code_or_block
    };
    let mut parts = block.split('/');
    let (Some(addr), Some(prefix), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err(());
    };
    let addr_min = inet_aton(addr).ok_or(())?;
    let prefix = py_int(prefix).ok_or(())?;
    if prefix < 0 {
        return Err(());
    }
    let mask = if prefix >= 32 {
        0
    } else {
        0xffff_ffffu32 >> prefix
    };
    let addr_max = addr_min | mask;
    let value = u64::from(addr_min) + random_below(u64::from(addr_max - addr_min) + 1);
    Ok(Some(std::net::Ipv4Addr::from(value as u32).to_string()))
}

/// Python's `int(text)` for decimal text: surrounding whitespace, a sign
/// and single underscores between digits.
fn py_int(text: &str) -> Option<i64> {
    let text = text.trim_matches(char::is_whitespace);
    let (negative, digits) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    if digits.is_empty()
        || digits.starts_with('_')
        || digits.ends_with('_')
        || digits.contains("__")
    {
        return None;
    }
    let mut value: i64 = 0;
    for c in digits.chars().filter(|c| *c != '_') {
        value = value
            .checked_mul(10)?
            .checked_add(i64::from(c.to_digit(10)?))?;
    }
    Some(if negative { -value } else { value })
}

/// BSD `inet_aton`: one to four dot-separated parts (decimal, `0` octal or
/// `0x` hex), the last filling the remaining bytes, optionally followed by
/// whitespace and anything.
fn inet_aton(text: &str) -> Option<u32> {
    let bytes = text.as_bytes();
    let mut at = 0;
    let mut parts: Vec<u64> = Vec::new();
    let value = loop {
        if !bytes.get(at).is_some_and(u8::is_ascii_digit) {
            return None;
        }
        let mut value: u64 = 0;
        let mut base = 10;
        let mut digit = false;
        if bytes[at] == b'0' {
            at += 1;
            if matches!(bytes.get(at), Some(b'x' | b'X')) {
                base = 16;
                at += 1;
            } else {
                base = 8;
                digit = true;
            }
        }
        while let Some(&c) = bytes.get(at) {
            let next = match c {
                b'8' | b'9' if base == 8 => return None,
                b'0'..=b'9' => u64::from(c - b'0'),
                b'a'..=b'f' | b'A'..=b'F' if base == 16 => {
                    u64::from(c.to_ascii_lowercase() - b'a' + 10)
                }
                _ => break,
            };
            value = value * base + next;
            if value > u64::from(u32::MAX) {
                return None;
            }
            digit = true;
            at += 1;
        }
        if !digit {
            return None;
        }
        if bytes.get(at) == Some(&b'.') {
            if parts.len() >= 3 || value > 0xff {
                return None;
            }
            parts.push(value);
            at += 1;
            continue;
        }
        break value;
    };
    if let Some(&c) = bytes.get(at)
        && !matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
    {
        return None;
    }
    let limit: u64 = match parts.len() {
        0 => u64::from(u32::MAX),
        1 => 0xff_ffff,
        2 => 0xffff,
        _ => 0xff,
    };
    if value > limit {
        return None;
    }
    let mut address: u64 = 0;
    for (index, part) in parts.iter().enumerate() {
        address |= part << (24 - 8 * index);
    }
    Some((address | value) as u32)
}

/// `GeoUtils._country_ip_map`: major IPv4 address blocks per country.
const COUNTRY_IP_MAP: [(&str, &str); 240] = [
    ("AD", "46.172.224.0/19"),
    ("AE", "94.200.0.0/13"),
    ("AF", "149.54.0.0/17"),
    ("AG", "209.59.64.0/18"),
    ("AI", "204.14.248.0/21"),
    ("AL", "46.99.0.0/16"),
    ("AM", "46.70.0.0/15"),
    ("AO", "105.168.0.0/13"),
    ("AP", "182.50.184.0/21"),
    ("AQ", "23.154.160.0/24"),
    ("AR", "181.0.0.0/12"),
    ("AS", "202.70.112.0/20"),
    ("AT", "77.116.0.0/14"),
    ("AU", "1.128.0.0/11"),
    ("AW", "181.41.0.0/18"),
    ("AX", "185.217.4.0/22"),
    ("AZ", "5.197.0.0/16"),
    ("BA", "31.176.128.0/17"),
    ("BB", "65.48.128.0/17"),
    ("BD", "114.130.0.0/16"),
    ("BE", "57.0.0.0/8"),
    ("BF", "102.178.0.0/15"),
    ("BG", "95.42.0.0/15"),
    ("BH", "37.131.0.0/17"),
    ("BI", "154.117.192.0/18"),
    ("BJ", "137.255.0.0/16"),
    ("BL", "185.212.72.0/23"),
    ("BM", "196.12.64.0/18"),
    ("BN", "156.31.0.0/16"),
    ("BO", "161.56.0.0/16"),
    ("BQ", "161.0.80.0/20"),
    ("BR", "191.128.0.0/12"),
    ("BS", "24.51.64.0/18"),
    ("BT", "119.2.96.0/19"),
    ("BW", "168.167.0.0/16"),
    ("BY", "178.120.0.0/13"),
    ("BZ", "179.42.192.0/18"),
    ("CA", "99.224.0.0/11"),
    ("CD", "41.243.0.0/16"),
    ("CF", "197.242.176.0/21"),
    ("CG", "160.113.0.0/16"),
    ("CH", "85.0.0.0/13"),
    ("CI", "102.136.0.0/14"),
    ("CK", "202.65.32.0/19"),
    ("CL", "152.172.0.0/14"),
    ("CM", "102.244.0.0/14"),
    ("CN", "36.128.0.0/10"),
    ("CO", "181.240.0.0/12"),
    ("CR", "201.192.0.0/12"),
    ("CU", "152.206.0.0/15"),
    ("CV", "165.90.96.0/19"),
    ("CW", "190.88.128.0/17"),
    ("CY", "31.153.0.0/16"),
    ("CZ", "88.100.0.0/14"),
    ("DE", "53.0.0.0/8"),
    ("DJ", "197.241.0.0/17"),
    ("DK", "87.48.0.0/12"),
    ("DM", "192.243.48.0/20"),
    ("DO", "152.166.0.0/15"),
    ("DZ", "41.96.0.0/12"),
    ("EC", "186.68.0.0/15"),
    ("EE", "90.190.0.0/15"),
    ("EG", "156.160.0.0/11"),
    ("ER", "196.200.96.0/20"),
    ("ES", "88.0.0.0/11"),
    ("ET", "196.188.0.0/14"),
    ("EU", "2.16.0.0/13"),
    ("FI", "91.152.0.0/13"),
    ("FJ", "144.120.0.0/16"),
    ("FK", "80.73.208.0/21"),
    ("FM", "119.252.112.0/20"),
    ("FO", "88.85.32.0/19"),
    ("FR", "90.0.0.0/9"),
    ("GA", "41.158.0.0/15"),
    ("GB", "25.0.0.0/8"),
    ("GD", "74.122.88.0/21"),
    ("GE", "31.146.0.0/16"),
    ("GF", "161.22.64.0/18"),
    ("GG", "62.68.160.0/19"),
    ("GH", "154.160.0.0/12"),
    ("GI", "95.164.0.0/16"),
    ("GL", "88.83.0.0/19"),
    ("GM", "160.182.0.0/15"),
    ("GN", "197.149.192.0/18"),
    ("GP", "104.250.0.0/19"),
    ("GQ", "105.235.224.0/20"),
    ("GR", "94.64.0.0/13"),
    ("GT", "168.234.0.0/16"),
    ("GU", "168.123.0.0/16"),
    ("GW", "197.214.80.0/20"),
    ("GY", "181.41.64.0/18"),
    ("HK", "113.252.0.0/14"),
    ("HN", "181.210.0.0/16"),
    ("HR", "93.136.0.0/13"),
    ("HT", "148.102.128.0/17"),
    ("HU", "84.0.0.0/14"),
    ("ID", "39.192.0.0/10"),
    ("IE", "87.32.0.0/12"),
    ("IL", "79.176.0.0/13"),
    ("IM", "5.62.80.0/20"),
    ("IN", "117.192.0.0/10"),
    ("IO", "203.83.48.0/21"),
    ("IQ", "37.236.0.0/14"),
    ("IR", "2.176.0.0/12"),
    ("IS", "82.221.0.0/16"),
    ("IT", "79.0.0.0/10"),
    ("JE", "87.244.64.0/18"),
    ("JM", "72.27.0.0/17"),
    ("JO", "176.29.0.0/16"),
    ("JP", "133.0.0.0/8"),
    ("KE", "105.48.0.0/12"),
    ("KG", "158.181.128.0/17"),
    ("KH", "36.37.128.0/17"),
    ("KI", "103.25.140.0/22"),
    ("KM", "197.255.224.0/20"),
    ("KN", "198.167.192.0/19"),
    ("KP", "175.45.176.0/22"),
    ("KR", "175.192.0.0/10"),
    ("KW", "37.36.0.0/14"),
    ("KY", "64.96.0.0/15"),
    ("KZ", "2.72.0.0/13"),
    ("LA", "115.84.64.0/18"),
    ("LB", "178.135.0.0/16"),
    ("LC", "24.92.144.0/20"),
    ("LI", "82.117.0.0/19"),
    ("LK", "112.134.0.0/15"),
    ("LR", "102.183.0.0/16"),
    ("LS", "129.232.0.0/17"),
    ("LT", "78.56.0.0/13"),
    ("LU", "188.42.0.0/16"),
    ("LV", "46.109.0.0/16"),
    ("LY", "41.252.0.0/14"),
    ("MA", "105.128.0.0/11"),
    ("MC", "88.209.64.0/18"),
    ("MD", "37.246.0.0/16"),
    ("ME", "178.175.0.0/17"),
    ("MF", "74.112.232.0/21"),
    ("MG", "154.126.0.0/17"),
    ("MH", "117.103.88.0/21"),
    ("MK", "77.28.0.0/15"),
    ("ML", "154.118.128.0/18"),
    ("MM", "37.111.0.0/17"),
    ("MN", "49.0.128.0/17"),
    ("MO", "60.246.0.0/16"),
    ("MP", "202.88.64.0/20"),
    ("MQ", "109.203.224.0/19"),
    ("MR", "41.188.64.0/18"),
    ("MS", "208.90.112.0/22"),
    ("MT", "46.11.0.0/16"),
    ("MU", "105.16.0.0/12"),
    ("MV", "27.114.128.0/18"),
    ("MW", "102.70.0.0/15"),
    ("MX", "187.192.0.0/11"),
    ("MY", "175.136.0.0/13"),
    ("MZ", "197.218.0.0/15"),
    ("NA", "41.182.0.0/16"),
    ("NC", "101.101.0.0/18"),
    ("NE", "197.214.0.0/18"),
    ("NF", "203.17.240.0/22"),
    ("NG", "105.112.0.0/12"),
    ("NI", "186.76.0.0/15"),
    ("NL", "145.96.0.0/11"),
    ("NO", "84.208.0.0/13"),
    ("NP", "36.252.0.0/15"),
    ("NR", "203.98.224.0/19"),
    ("NU", "49.156.48.0/22"),
    ("NZ", "49.224.0.0/14"),
    ("OM", "5.36.0.0/15"),
    ("PA", "186.72.0.0/15"),
    ("PE", "186.160.0.0/14"),
    ("PF", "123.50.64.0/18"),
    ("PG", "124.240.192.0/19"),
    ("PH", "49.144.0.0/13"),
    ("PK", "39.32.0.0/11"),
    ("PL", "83.0.0.0/11"),
    ("PM", "70.36.0.0/20"),
    ("PR", "66.50.0.0/16"),
    ("PS", "188.161.0.0/16"),
    ("PT", "85.240.0.0/13"),
    ("PW", "202.124.224.0/20"),
    ("PY", "181.120.0.0/14"),
    ("QA", "37.210.0.0/15"),
    ("RE", "102.35.0.0/16"),
    ("RO", "79.112.0.0/13"),
    ("RS", "93.86.0.0/15"),
    ("RU", "5.136.0.0/13"),
    ("RW", "41.186.0.0/16"),
    ("SA", "188.48.0.0/13"),
    ("SB", "202.1.160.0/19"),
    ("SC", "154.192.0.0/11"),
    ("SD", "102.120.0.0/13"),
    ("SE", "78.64.0.0/12"),
    ("SG", "8.128.0.0/10"),
    ("SI", "188.196.0.0/14"),
    ("SK", "78.98.0.0/15"),
    ("SL", "102.143.0.0/17"),
    ("SM", "89.186.32.0/19"),
    ("SN", "41.82.0.0/15"),
    ("SO", "154.115.192.0/18"),
    ("SR", "186.179.128.0/17"),
    ("SS", "105.235.208.0/21"),
    ("ST", "197.159.160.0/19"),
    ("SV", "168.243.0.0/16"),
    ("SX", "190.102.0.0/20"),
    ("SY", "5.0.0.0/16"),
    ("SZ", "41.84.224.0/19"),
    ("TC", "65.255.48.0/20"),
    ("TD", "154.68.128.0/19"),
    ("TG", "196.168.0.0/14"),
    ("TH", "171.96.0.0/13"),
    ("TJ", "85.9.128.0/18"),
    ("TK", "27.96.24.0/21"),
    ("TL", "180.189.160.0/20"),
    ("TM", "95.85.96.0/19"),
    ("TN", "197.0.0.0/11"),
    ("TO", "175.176.144.0/21"),
    ("TR", "78.160.0.0/11"),
    ("TT", "186.44.0.0/15"),
    ("TV", "202.2.96.0/19"),
    ("TW", "120.96.0.0/11"),
    ("TZ", "156.156.0.0/14"),
    ("UA", "37.52.0.0/14"),
    ("UG", "102.80.0.0/13"),
    ("US", "6.0.0.0/8"),
    ("UY", "167.56.0.0/13"),
    ("UZ", "84.54.64.0/18"),
    ("VA", "212.77.0.0/19"),
    ("VC", "207.191.240.0/21"),
    ("VE", "186.88.0.0/13"),
    ("VG", "66.81.192.0/20"),
    ("VI", "146.226.0.0/16"),
    ("VN", "14.160.0.0/11"),
    ("VU", "202.80.32.0/20"),
    ("WF", "117.20.32.0/21"),
    ("WS", "202.4.32.0/19"),
    ("YE", "134.35.0.0/16"),
    ("YT", "41.242.116.0/22"),
    ("ZA", "41.0.0.0/11"),
    ("ZM", "102.144.0.0/13"),
    ("ZW", "102.177.192.0/18"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_ipv4_parses_blocks_like_python() {
        // Single-address blocks, as `inet_aton` and `int` read them.
        for (block, address) in [
            ("1.2.3/32", "1.2.0.3"),
            ("0x7f.1/32", "127.0.0.1"),
            ("010.0.0.1/32", "8.0.0.1"),
            ("1.2.3.4 x/32", "1.2.3.4"),
            ("4294967295/32", "255.255.255.255"),
            ("1.2.3.4/ 32", "1.2.3.4"),
            ("1.2.3.4/32 ", "1.2.3.4"),
            ("1.2.3.4/+32", "1.2.3.4"),
            ("1.2.3.4/33", "1.2.3.4"),
            ("1.2.3.4\t/32", "1.2.3.4"),
        ] {
            assert_eq!(random_ipv4(block), Ok(Some(address.into())), "{block}");
        }
        for block in [
            "256.1.1.1/32",
            "1.2.3.4.5/32",
            "/32",
            "1.2.3.4/-1",
            "1.2.3.4/0x8",
            "1.2.3.4/8/9",
            " 1.2.3.4/32",
            "08.1.1.1/32",
            "0x/32",
            "1..2/32",
            ".1/32",
            "1./32",
            "1.2.3.4",
            "foo",
        ] {
            assert_eq!(random_ipv4(block), Err(()), "{block}");
        }
        assert_eq!(random_ipv4("ZZ"), Ok(None));
        // `1_0` is ten: a /10 from 1.2.3.4 up to 1.63.255.255.
        let ip: std::net::Ipv4Addr = random_ipv4("1.2.3.4/1_0")
            .unwrap()
            .unwrap()
            .parse()
            .unwrap();
        let range = u32::from_be_bytes([1, 2, 3, 4])..=u32::from_be_bytes([1, 63, 255, 255]);
        assert!(range.contains(&u32::from(ip)));
    }

    #[test]
    fn country_addresses_stay_in_their_block() {
        assert_eq!(COUNTRY_IP_MAP.len(), 240);
        // NO: 84.208.0.0/13.
        for _ in 0..100 {
            let ip: std::net::Ipv4Addr = random_ipv4("no").unwrap().unwrap().parse().unwrap();
            assert_eq!(
                u32::from(ip) >> 19,
                u32::from_be_bytes([84, 208, 0, 0]) >> 19
            );
        }
    }

    #[test]
    fn xff_values_follow_validate_options() {
        assert_eq!(GeoOptions::parse("DEFAULT"), Ok(GeoOptions::default()));
        assert_eq!(
            GeoOptions::parse("Never"),
            Ok(GeoOptions {
                bypass: false,
                ..GeoOptions::default()
            })
        );
        assert_eq!(
            GeoOptions::parse("ZZ").unwrap().country.as_deref(),
            Some("ZZ")
        );
        assert_eq!(
            GeoOptions::parse("1.2.3.4/8").unwrap().ip_block.as_deref(),
            Some("1.2.3.4/8")
        );
        assert_eq!(
            GeoOptions::parse("foo"),
            Err("Unsupported --xff \"foo\"".into())
        );
        assert_eq!(
            GeoOptions::parse("1.2.3.4"),
            Err("Unsupported --xff \"1.2.3.4\"".into())
        );
    }
}
