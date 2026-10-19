//! Upstream `--cookies-from-browser` (`yt_dlp/cookies.py`): cookies read
//! from a browser profile's database, with upstream's messages.
use crate::{Error, Result, sqlite};
use std::path::{Path, PathBuf};

pub const SUPPORTED_BROWSERS: [&str; 9] = [
    "brave", "chrome", "chromium", "edge", "firefox", "opera", "safari", "vivaldi", "whale",
];
pub const SUPPORTED_KEYRINGS: [&str; 5] = [
    "BASICTEXT",
    "GNOMEKEYRING",
    "KWALLET",
    "KWALLET5",
    "KWALLET6",
];

/// `BROWSER[+KEYRING][:PROFILE][::CONTAINER]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserSpec {
    pub browser: String,
    pub profile: Option<String>,
    pub keyring: Option<String>,
    pub container: Option<String>,
}

/// `\s*::\s*(?P<container>.+)` matching all of `text`.
fn container_part(text: &str) -> Option<&str> {
    let rest = text.trim_start().strip_prefix("::")?;
    let trimmed = rest.trim_start();
    if !trimmed.is_empty() {
        Some(trimmed)
    } else if !rest.is_empty() {
        // `\s*` gives a whitespace character back to `.+`.
        Some(&rest[rest.len() - rest.chars().last()?.len_utf8()..])
    } else {
        None
    }
}

impl BrowserSpec {
    /// Upstream `__init__` validation; errors are usage-error texts.
    pub fn parse(text: &str) -> std::result::Result<Self, String> {
        let invalid = || format!("invalid cookies from browser arguments: {text}");
        let name_end = text.find(['+', ':']).unwrap_or(text.len());
        if name_end == 0 {
            return Err(invalid());
        }
        let mut rest = &text[name_end..];
        let mut keyring = None;
        if let Some(after) = rest.strip_prefix('+') {
            let after = after.trim_start();
            let end = after.find(':').unwrap_or(after.len());
            if end == 0 {
                return Err(invalid());
            }
            keyring = Some(&after[..end]);
            rest = &after[end..];
        }
        let (mut profile, mut container) = (None, None);
        if !rest.is_empty() {
            if let Some(found) = container_part(rest) {
                container = Some(found);
            } else {
                let after = rest
                    .trim_start()
                    .strip_prefix(':')
                    .ok_or_else(invalid)?
                    .trim_start();
                if after.is_empty() || after.starts_with(':') {
                    return Err(invalid());
                }
                // `(?P<profile>.+?)` is as short as the optional container allows.
                let mut split = None;
                for (index, _) in after.char_indices().skip(1) {
                    if let Some(found) = container_part(&after[index..]) {
                        split = Some((index, found));
                        break;
                    }
                }
                match split {
                    Some((index, found)) => {
                        profile = Some(&after[..index]);
                        container = Some(found);
                    }
                    None => profile = Some(after),
                }
            }
        }
        let browser = text[..name_end].to_lowercase();
        if !SUPPORTED_BROWSERS.contains(&browser.as_str()) {
            return Err(format!(
                "unsupported browser specified for cookies: \"{browser}\". Supported browsers are: {}",
                SUPPORTED_BROWSERS.join(", ")
            ));
        }
        let keyring = keyring.map(str::to_uppercase);
        if let Some(keyring) = &keyring
            && !SUPPORTED_KEYRINGS.contains(&keyring.as_str())
        {
            return Err(format!(
                "unsupported keyring specified for cookies: \"{keyring}\". Supported keyrings are: {}",
                SUPPORTED_KEYRINGS.join(", ")
            ));
        }
        // `_parse_browser_specification`: a profile path is expanded.
        let profile = profile.map(|profile| {
            let expanded = crate::batch::expand_path_text(profile);
            if is_path(&expanded) {
                expanded
            } else {
                profile.to_owned()
            }
        });
        Ok(Self {
            browser,
            profile,
            keyring,
            container: container.map(str::to_owned),
        })
    }
}

fn is_path(value: &str) -> bool {
    value.contains(std::path::MAIN_SEPARATOR) || (cfg!(windows) && value.contains('/'))
}

/// One extracted cookie, as `http.cookiejar.Cookie` fields.
#[derive(Debug, Clone, PartialEq)]
pub struct BrowserCookie {
    pub domain: String,
    pub name: String,
    pub value: String,
    pub path: String,
    pub expires: Option<f64>,
    pub secure: bool,
}

/// A message upstream's `YDLLogger` emits while extracting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Info(String),
    Debug(String),
    Warning(String),
}

/// Extracted cookies (deduplicated like a `CookieJar` by domain, path and
/// name) and the messages to report when upstream would load the jar.
#[derive(Debug, Default)]
pub struct Extracted {
    pub cookies: Vec<BrowserCookie>,
    pub messages: Vec<Message>,
    /// Upstream's `CookieLoadError` cause, raised where it loads the jar.
    pub error: Option<String>,
}

impl Extracted {
    fn info(&mut self, text: impl Into<String>) {
        self.messages.push(Message::Info(text.into()));
    }
    fn debug(&mut self, text: impl Into<String>) {
        self.messages.push(Message::Debug(text.into()));
    }
    fn warning(&mut self, text: impl Into<String>) {
        self.messages.push(Message::Warning(text.into()));
    }
    /// `CookieJar.set_cookie`: a later cookie replaces one with the same key.
    fn set_cookie(&mut self, cookie: BrowserCookie) {
        match self.cookies.iter_mut().find(|existing| {
            existing.domain == cookie.domain
                && existing.path == cookie.path
                && existing.name == cookie.name
        }) {
            Some(existing) => *existing = cookie,
            None => self.cookies.push(cookie),
        }
    }
}

/// Upstream `extract_cookies_from_browser`; a failure keeps the messages
/// so far and upstream's error text (without cookies).
pub fn extract(spec: &BrowserSpec) -> Extracted {
    let mut extracted = Extracted::default();
    let result = match spec.browser.as_str() {
        "firefox" => firefox(spec, &mut extracted),
        browser => Err(Error::Reported(format!(
            "extracting cookies from {browser} is not supported natively"
        ))),
    };
    if let Err(error) = result {
        extracted.cookies.clear();
        extracted.error = Some(match error {
            Error::Reported(message) => message,
            other => other.to_string(),
        });
    }
    extracted
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// `_config_home`.
fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"))
}

/// `_firefox_browser_dirs`.
fn firefox_browser_dirs() -> Vec<PathBuf> {
    let home = home();
    if cfg!(target_os = "macos") {
        vec![home.join("Library/Application Support/Firefox/Profiles")]
    } else if cfg!(windows) {
        let var = |name: &str| {
            std::env::var_os(name)
                .map(PathBuf::from)
                .unwrap_or_default()
        };
        vec![
            var("APPDATA").join(r"Mozilla\Firefox\Profiles"),
            var("LOCALAPPDATA").join(
                r"Packages\Mozilla.Firefox_n80bbvh6b1yt2\LocalCache\Roaming\Mozilla\Firefox\Profiles",
            ),
        ]
    } else {
        vec![
            config_home().join("mozilla/firefox"),
            home.join(".mozilla/firefox"),
            home.join(".var/app/org.mozilla.firefox/config/mozilla/firefox"),
            home.join(".var/app/org.mozilla.firefox/.mozilla/firefox"),
            home.join("snap/firefox/common/.mozilla/firefox"),
        ]
    }
}

/// Python `os.path.abspath` of a search root.
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_owned())
}

/// `glob(os.path.join(root, pattern, 'cookies.sqlite'))` for the patterns
/// `''`, `'*/'` and `'Profiles/*/'`.
fn firefox_cookie_dbs(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let children = |directory: &Path| -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return Vec::new();
        };
        let mut entries: Vec<PathBuf> = entries
            .flatten()
            .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        entries.sort();
        entries
    };
    for root in roots.iter().map(|root| absolute(root)) {
        let mut candidates = vec![root.join("cookies.sqlite")];
        candidates.extend(
            children(&root)
                .into_iter()
                .map(|child| child.join("cookies.sqlite")),
        );
        candidates.extend(
            children(&root.join("Profiles"))
                .into_iter()
                .map(|child| child.join("cookies.sqlite")),
        );
        found.extend(
            candidates
                .into_iter()
                .filter(|path| std::fs::symlink_metadata(path).is_ok()),
        );
    }
    found
}

/// `_newest`: the most recently modified (`lstat`) path; the first on ties.
fn newest(paths: Vec<PathBuf>) -> Option<PathBuf> {
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for path in paths {
        let Ok(modified) = std::fs::symlink_metadata(&path).and_then(|meta| meta.modified()) else {
            continue;
        };
        if best.as_ref().is_none_or(|(time, _)| modified > *time) {
            best = Some((modified, path));
        }
    }
    best.map(|(_, path)| path)
}

fn not_found(message: String) -> Error {
    Error::Reported(message)
}

/// Upstream `_extract_firefox_cookies`.
fn firefox(spec: &BrowserSpec, out: &mut Extracted) -> Result<()> {
    const MAX_SUPPORTED_DB_SCHEMA_VERSION: i64 = 17;
    out.info("Extracting cookies from firefox");
    let roots: Vec<PathBuf> = match &spec.profile {
        None => firefox_browser_dirs(),
        Some(profile) if is_path(profile) => vec![PathBuf::from(profile)],
        Some(profile) => firefox_browser_dirs()
            .into_iter()
            .map(|directory| directory.join(profile))
            .collect(),
    };
    let search_root = roots
        .iter()
        .map(|root| crate::batch::py_repr(&root.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(", ");
    let database = newest(firefox_cookie_dbs(&roots)).ok_or_else(|| {
        not_found(format!(
            "could not find firefox cookies database in {search_root}"
        ))
    })?;
    out.debug(format!(
        "Extracting cookies from: \"{}\"",
        database.display()
    ));
    let container = spec.container.as_deref();
    let mut container_id = None;
    if let Some(container) = container.filter(|container| *container != "none") {
        let path = database.with_file_name("containers.json");
        let text = std::fs::read_to_string(&path)
            .map_err(|_| not_found(format!("could not read containers.json in {search_root}")))?;
        let json: serde_json::Value =
            serde_json::from_str(&text).map_err(|error| Error::Cookies(error.to_string()))?;
        static L10N: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new(r"^userContext([^\.]+)\.label$").unwrap()
        });
        let identities = json
            .get("identities")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        // Upstream compares the name or the whole matched `l10nID`.
        let found = identities.iter().find(|context| {
            context.get("name").and_then(serde_json::Value::as_str) == Some(container)
                || context
                    .get("l10nID")
                    .and_then(serde_json::Value::as_str)
                    .filter(|id| L10N.is_match(id))
                    == Some(container)
        });
        container_id = found
            .and_then(|context| context.get("userContextId"))
            .and_then(serde_json::Value::as_i64);
        if container_id.is_none() {
            return Err(Error::Reported(format!(
                "could not find firefox container \"{container}\" in containers.json"
            )));
        }
    }
    let db = sqlite::Database::open(&database)?;
    let version = db.user_version();
    if version > MAX_SUPPORTED_DB_SCHEMA_VERSION {
        out.warning(format!(
            "Possibly unsupported firefox cookies database version: {version}"
        ));
    } else {
        out.debug(format!("Firefox cookies database version: {version}"));
    }
    if let Some(id) = container_id {
        out.debug(format!(
            "Only loading cookies from firefox container \"{}\", ID {id}",
            container.unwrap_or_default()
        ));
    } else if container == Some("none") {
        out.debug("Only loading cookies not belonging to any container");
    }
    let table = db
        .table("moz_cookies")?
        .ok_or_else(|| Error::Cookies("no such table: moz_cookies".into()))?;
    let column = |name: &str| {
        table
            .column(name)
            .ok_or_else(|| Error::Cookies(format!("no such column: {name}")))
    };
    let [host, name, value, path, expiry, secure] =
        ["host", "name", "value", "path", "expiry", "isSecure"].map(column);
    let (host, name, value, path, expiry, secure) = (host?, name?, value?, path?, expiry?, secure?);
    let origin = table.column("originAttributes");
    for row in &table.rows {
        let attributes = origin
            .and_then(|index| row[index].as_text())
            .unwrap_or_default()
            .to_lowercase();
        let keep = match container_id {
            // `originAttributes LIKE '%userContextId=ID' OR LIKE '%userContextId=ID&%'`.
            Some(id) => {
                let needle = format!("usercontextid={id}");
                attributes.ends_with(&needle) || attributes.contains(&format!("{needle}&"))
            }
            None if container == Some("none") => !attributes.contains("usercontextid="),
            None => true,
        };
        if !keep {
            continue;
        }
        let text = |index: usize| row[index].as_text().unwrap_or_default().to_owned();
        let mut expires = row[expiry].as_f64();
        // Schema 16 (Firefox 142) stores milliseconds.
        if version >= 16 {
            expires = expires.map(|expires| expires / 1000.0);
        }
        out.set_cookie(BrowserCookie {
            domain: text(host),
            name: text(name),
            value: text(value),
            path: text(path),
            expires,
            secure: row[secure].truthy(),
        });
    }
    let count = out.cookies.len();
    out.info(format!("Extracted {count} cookies from firefox"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The saved databases read like upstream's sqlite3 queries.
    #[test]
    fn saved_firefox_profiles_match_upstream() {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/browser_cookies");
        let expected: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/browser_cookies/expected.json"
        ))
        .unwrap();
        let root = std::path::absolute(&fixtures).unwrap();
        for case in expected.as_array().unwrap() {
            let profile = root.join("firefox").join(case["profile"].as_str().unwrap());
            let spec = BrowserSpec {
                browser: "firefox".into(),
                profile: Some(profile.to_string_lossy().into_owned()),
                keyring: None,
                container: case["container"].as_str().map(str::to_owned),
            };
            let extracted = extract(&spec);
            assert_eq!(extracted.error, None, "{case}");
            let mut cookies: Vec<serde_json::Value> = extracted
                .cookies
                .iter()
                .map(|cookie| {
                    serde_json::json!([
                        cookie.domain,
                        cookie.path,
                        cookie.name,
                        cookie.value,
                        cookie.expires.map(|expires| expires as i64),
                        cookie.secure
                    ])
                })
                .collect();
            cookies.sort_by_key(|cookie| cookie.to_string());
            let mut wanted = case["cookies"].as_array().unwrap().clone();
            wanted.sort_by_key(|cookie| cookie.to_string());
            assert_eq!(cookies, wanted, "{}", case["container"]);
            let messages: Vec<serde_json::Value> = extracted
                .messages
                .iter()
                .map(|message| {
                    let (kind, text) = match message {
                        Message::Info(text) => ("info", text),
                        Message::Debug(text) => ("debug", text),
                        Message::Warning(text) => ("warning", text),
                    };
                    serde_json::json!([kind, text.replace(&*root.to_string_lossy(), "FIXTURES")])
                })
                .collect();
            assert_eq!(messages, *case["messages"].as_array().unwrap());
        }
    }

    fn spec(text: &str) -> (String, Option<String>, Option<String>, Option<String>) {
        let spec = BrowserSpec::parse(text).unwrap();
        (spec.browser, spec.keyring, spec.profile, spec.container)
    }

    #[test]
    fn specifications_follow_upstreams_regex() {
        let none = None::<String>;
        let some = |text: &str| Some(text.to_owned());
        assert_eq!(
            spec("Firefox"),
            ("firefox".into(), none.clone(), none.clone(), none.clone())
        );
        assert_eq!(
            spec("chrome+kwallet:Profile 1"),
            (
                "chrome".into(),
                some("KWALLET"),
                some("Profile 1"),
                none.clone()
            )
        );
        assert_eq!(
            spec("firefox::Work"),
            ("firefox".into(), none.clone(), none.clone(), some("Work"))
        );
        assert_eq!(
            spec("firefox: default ::  Work X"),
            (
                "firefox".into(),
                none.clone(),
                some("default"),
                some("Work X")
            )
        );
        assert_eq!(
            BrowserSpec::parse("opera2").unwrap_err(),
            "unsupported browser specified for cookies: \"opera2\". Supported browsers are: brave, chrome, chromium, edge, firefox, opera, safari, vivaldi, whale"
        );
        assert_eq!(
            BrowserSpec::parse("chrome+bogus").unwrap_err(),
            "unsupported keyring specified for cookies: \"BOGUS\". Supported keyrings are: BASICTEXT, GNOMEKEYRING, KWALLET, KWALLET5, KWALLET6"
        );
        assert_eq!(
            BrowserSpec::parse(":x").unwrap_err(),
            "invalid cookies from browser arguments: :x"
        );
    }
}
