//! Upstream `networking/impersonate.py` and the impersonation parts of
//! `YoutubeDL` and `InfoExtractor._request_webpage`: `CLIENT[:OS]` target
//! parsing, the available targets and their preference order, the
//! `--list-impersonate-targets` table, and per-request impersonation for
//! extractors. Targets are only available with the optional `impersonate`
//! cargo feature, whose `wreq` transport (BoringSSL) sends the browser's
//! TLS ClientHello, HTTP/2 settings and default headers; the default build
//! has none, like upstream without `curl_cffi`.
#[cfg(feature = "impersonate")]
mod transport;

use std::cell::Cell;
use std::fmt;
use std::sync::LazyLock;

/// This port's impersonation backend (upstream's `RH_NAME`).
pub const SOURCE: &str = "wreq";

/// Upstream `ImpersonateTarget`; `None` matches any value.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImpersonateTarget {
    pub client: Option<String>,
    pub version: Option<String>,
    pub os: Option<String>,
    pub os_version: Option<String>,
}

impl ImpersonateTarget {
    /// `ImpersonateTarget.from_str`.
    pub fn parse(target: &str) -> Result<Self, String> {
        static PATTERN: LazyLock<regex::Regex> = LazyLock::new(|| {
            regex::Regex::new(
                r"^(?:(?P<client>[^:-]+)(?:-(?P<version>[^:-]+))?)?(?::(?:(?P<os>[^:-]+)(?:-(?P<os_version>[^:-]+))?)?)?$",
            )
            .unwrap()
        });
        let found = PATTERN
            .captures(target)
            .ok_or_else(|| format!("Invalid impersonate target \"{target}\""))?;
        let group = |name| found.name(name).map(|value| value.as_str().to_owned());
        Ok(Self {
            client: group("client"),
            version: group("version"),
            os: group("os"),
            os_version: group("os_version"),
        })
    }

    /// `ImpersonateTarget.__contains__`: `target in self`.
    pub fn contains(&self, target: &ImpersonateTarget) -> bool {
        let matches = |ours: &Option<String>, theirs: &Option<String>| {
            ours.is_none() || theirs.is_none() || ours == theirs
        };
        matches(&self.client, &target.client)
            && matches(&self.version, &target.version)
            && matches(&self.os, &target.os)
            && matches(&self.os_version, &target.os_version)
    }
}

/// `join_nonempty(a, b, delim='-')`.
fn join(first: Option<&str>, second: Option<&str>) -> String {
    [first, second]
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

impl fmt::Display for ImpersonateTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = format!(
            "{}:{}",
            join(self.client.as_deref(), self.version.as_deref()),
            join(self.os.as_deref(), self.os_version.as_deref())
        );
        f.write_str(text.trim_end_matches(':'))
    }
}

/// One supported target and (with the feature) the `wreq` emulation that
/// sends it.
#[derive(Clone)]
pub(crate) struct Supported {
    pub(crate) target: ImpersonateTarget,
    #[cfg(feature = "impersonate")]
    pub(crate) emulation: transport::Profile,
}

/// Upstream `_SUPPORTED_IMPERSONATE_TARGET_MAP`'s ordering: desktop before
/// mobile, then `tor < edge < firefox < safari < chrome`, the newest
/// version, and the OS name, all descending (a stable sort).
fn preference_sorted(mut targets: Vec<Supported>) -> Vec<Supported> {
    fn version_tuple(version: Option<&str>) -> Vec<i64> {
        version
            .unwrap_or("0")
            .split(['-', '.'])
            .map(|part| part.parse().unwrap_or(-1))
            .collect()
    }
    let key = |entry: &Supported| {
        let target = &entry.target;
        let client = ["tor", "edge", "firefox", "safari", "chrome"]
            .iter()
            .position(|name| target.client.as_deref() == Some(name));
        (
            !matches!(target.os.as_deref(), Some("ios" | "android")),
            client,
            version_tuple(target.version.as_deref()),
            target.os.clone(),
        )
    };
    targets.sort_by_key(|entry| std::cmp::Reverse(key(entry)));
    targets
}

/// The supported targets in preference order.
pub(crate) fn supported_targets() -> &'static [Supported] {
    static TARGETS: LazyLock<Vec<Supported>> = LazyLock::new(|| {
        #[cfg(feature = "impersonate")]
        let targets = transport::targets();
        #[cfg(not(feature = "impersonate"))]
        let targets = Vec::new();
        preference_sorted(targets)
    });
    &TARGETS
}

/// `ImpersonateRequestHandler._resolve_target`: the first supported target
/// containing `target`.
fn resolve<'a>(supported: &'a [Supported], target: &ImpersonateTarget) -> Option<&'a Supported> {
    supported.iter().find(|entry| entry.target.contains(target))
}

/// `YoutubeDL._impersonate_target_available`.
pub fn target_available(target: &ImpersonateTarget) -> bool {
    resolve(supported_targets(), target).is_some()
}

/// The `--list-impersonate-targets` table (upstream `render_table(['Client',
/// 'OS', 'Source'], rows, extra_gap=2, delim='-')`): the available targets,
/// least preferred first, after any known client that is unavailable.
pub fn targets_table() -> String {
    render_targets(supported_targets())
}

fn render_targets(supported: &[Supported]) -> String {
    fn title(text: &str) -> String {
        // Python `str.title()` for these ASCII names.
        let mut out = String::with_capacity(text.len());
        let mut previous_cased = false;
        for c in text.chars() {
            if c.is_ascii_alphabetic() {
                out.push(if previous_cased {
                    c.to_ascii_lowercase()
                } else {
                    c.to_ascii_uppercase()
                });
                previous_cased = true;
            } else {
                out.push(c);
                previous_cased = false;
            }
        }
        out
    }
    let row = |target: &ImpersonateTarget, source: String| {
        let cell = |name: &Option<String>, version: &Option<String>| {
            let name = name.as_deref().map(title);
            let text = join(name.as_deref(), version.as_deref());
            if text.is_empty() { "-".into() } else { text }
        };
        vec![
            cell(&target.client, &target.version),
            cell(&target.os, &target.os_version),
            source,
        ]
    };
    let mut rows: Vec<Vec<String>> = supported
        .iter()
        .rev()
        .map(|entry| row(&entry.target, SOURCE.into()))
        .collect();
    // Upstream's `known_targets` (without `tor`, which no backend of this
    // port provides), all from this port's single source.
    for client in ["chrome", "safari", "firefox", "edge"] {
        let known = ImpersonateTarget {
            client: Some(client.into()),
            ..Default::default()
        };
        if !supported.iter().any(|entry| entry.target.contains(&known)) {
            rows.insert(0, row(&known, format!("{SOURCE} (unavailable)")));
        }
    }
    crate::output::render_table(
        ["Client", "OS", "Source"].map(String::from).to_vec(),
        rows,
        Some("-"),
        2,
        false,
    )
}

/// `YoutubeDL._unavailable_targets_message`.
pub(crate) fn unavailable_message(
    requested: &[ImpersonateTarget],
    note: &str,
    is_error: bool,
) -> String {
    let specific: Vec<String> = requested
        .iter()
        .map(ToString::to_string)
        .filter(|text| !text.is_empty())
        .collect();
    let message = if specific.is_empty() {
        "no impersonate target is available".to_owned()
    } else {
        format!(
            "none of these impersonate targets are available: {}",
            specific.join(", ")
        )
    };
    format!(
        "{note}, but {message}. {}  https://github.com/yt-dlp/yt-dlp#impersonation  for information on installing the required dependencies",
        if is_error {
            "See"
        } else {
            "If you encounter errors, then see"
        }
    )
}

/// `impersonate=True` (any target).
pub(crate) const ANY: &[&str] = &[""];

thread_local! {
    /// The resolved target of the extractor request being sent on this
    /// thread (an index into [`supported_targets`]).
    static ACTIVE: Cell<Option<usize>> = const { Cell::new(None) };
}

/// Restores the previous request target when dropped.
pub(crate) struct Scope {
    previous: Option<usize>,
    target: Option<usize>,
}

impl Scope {
    /// The target the scoped requests impersonate (upstream's
    /// `urlh.extensions.get("impersonate")`), `None` without one.
    pub(crate) fn target(&self) -> Option<&'static ImpersonateTarget> {
        self.target.map(|index| &supported_targets()[index].target)
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        ACTIVE.with(|active| active.set(self.previous));
    }
}

/// `YoutubeDL._parse_impersonate_targets` and `_request_webpage`'s target
/// selection: requests sent while the returned scope lives impersonate the
/// first available requested target (resolved like
/// `_get_request_target`); with none, `Err` is upstream's message.
fn select(requested: &[&str], require: bool) -> Result<Scope, String> {
    let requested: Vec<ImpersonateTarget> = requested
        .iter()
        .filter_map(|target| ImpersonateTarget::parse(target).ok())
        .collect();
    let supported = supported_targets();
    let resolved = requested
        .iter()
        .find_map(|target| resolve(supported, target).map(|entry| (target, entry)));
    let previous = ACTIVE.with(Cell::get);
    let Some((target, entry)) = resolved else {
        return Err(unavailable_message(
            &requested,
            "The extractor is attempting impersonation",
            require,
        ));
    };
    let index = supported
        .iter()
        .position(|candidate| std::ptr::eq(candidate, entry));
    if PRINT_TRAFFIC.load(std::sync::atomic::Ordering::Relaxed) {
        crate::screen::screen(&format!(
            "{SOURCE}: resolved impersonate target {target} to {}",
            entry.target
        ));
    }
    ACTIVE.with(|active| active.set(index));
    Ok(Scope {
        previous,
        target: index,
    })
}

/// `_request_webpage(..., impersonate=TARGETS)`: requests sent while the
/// scope lives impersonate an available target; without one, `warn` gets
/// upstream's `only_once` warning text (`The extractor is attempting
/// impersonation, but ...`, before the `[EXTRACTOR]` prefix) and the
/// requests stay plain.
pub(crate) fn attempt(requested: &[&str], warn: impl FnOnce(&str)) -> Scope {
    select(requested, false).unwrap_or_else(|message| {
        warn(&message);
        Scope {
            previous: ACTIVE.with(Cell::get),
            target: None,
        }
    })
}

/// `_request_webpage(..., impersonate=TARGETS, require_impersonation=True)`:
/// `Err` is the `ExtractorError` message without its `[EXTRACTOR]` prefix.
pub(crate) fn require(requested: &[&str]) -> Result<Scope, String> {
    select(requested, true)
}

/// Send an extraction request, through the impersonating transport when a
/// [`Scope`] is active on this thread.
pub(crate) fn send(
    request: reqwest::blocking::RequestBuilder,
) -> crate::Result<reqwest::blocking::Response> {
    #[cfg(feature = "impersonate")]
    if let Some(index) = ACTIVE.with(Cell::get) {
        return transport::send(request, &supported_targets()[index]);
    }
    Ok(request.send()?)
}

/// Configure the impersonating transport from the global HTTP options
/// (upstream passes the same options to every request handler).
pub(crate) fn configure(
    options: &crate::HttpOptions,
    cookies: &std::sync::Arc<crate::CookieJar>,
) -> crate::Result<()> {
    PRINT_TRAFFIC.store(options.print_traffic, std::sync::atomic::Ordering::Relaxed);
    #[cfg(feature = "impersonate")]
    transport::configure(options, cookies)?;
    #[cfg(not(feature = "impersonate"))]
    let _ = (options, cookies);
    Ok(())
}

/// Upstream `std_headers` as the CLI sends them: impersonated requests drop
/// header values equal to these, so the target's own browser headers apply
/// (`_get_impersonate_headers`).
pub fn set_std_headers(headers: Vec<(String, String)>) {
    if let Ok(mut std) = STD_HEADERS.write() {
        *std = headers;
    }
}

/// `--print-traffic` (upstream's handler `verbose`).
static PRINT_TRAFFIC: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) static STD_HEADERS: std::sync::RwLock<Vec<(String, String)>> =
    std::sync::RwLock::new(Vec::new());

#[cfg(test)]
mod tests {
    use super::*;

    fn target(text: &str) -> ImpersonateTarget {
        ImpersonateTarget::parse(text).unwrap()
    }

    fn supported(texts: &[&str]) -> Vec<Supported> {
        texts
            .iter()
            .map(|text| Supported {
                target: target(text),
                #[cfg(feature = "impersonate")]
                emulation: transport::Profile::placeholder(),
            })
            .collect()
    }

    #[test]
    fn targets_parse_and_print_like_upstream() {
        // Upstream test_networking `TestImpersonateTarget` cases.
        for (text, parts) in [
            ("abc", [Some("abc"), None, None, None]),
            ("abc-120_esr", [Some("abc"), Some("120_esr"), None, None]),
            ("abc-120:xyz", [Some("abc"), Some("120"), Some("xyz"), None]),
            (
                "abc-120:xyz-5.6",
                [Some("abc"), Some("120"), Some("xyz"), Some("5.6")],
            ),
            ("abc:xyz", [Some("abc"), None, Some("xyz"), None]),
            ("abc:", [Some("abc"), None, None, None]),
            ("abc-120:", [Some("abc"), Some("120"), None, None]),
            (":xyz", [None, None, Some("xyz"), None]),
            (":xyz-6.5", [None, None, Some("xyz"), Some("6.5")]),
            (":", [None, None, None, None]),
            ("", [None, None, None, None]),
        ] {
            let parsed = target(text);
            let owned = parts.map(|part| part.map(String::from));
            assert_eq!(
                [parsed.client, parsed.version, parsed.os, parsed.os_version],
                owned,
                "{text}"
            );
        }
        for text in [
            "-120",
            ":-12.0",
            "-12:-12",
            "-:-",
            "::",
            "a-c-d:",
            "a-c-d:e-f-g",
            "a:b:",
        ] {
            assert_eq!(
                ImpersonateTarget::parse(text).unwrap_err(),
                format!("Invalid impersonate target \"{text}\"")
            );
        }
        for (text, printed) in [
            ("abc", "abc"),
            ("abc-120", "abc-120"),
            ("abc-120:xyz", "abc-120:xyz"),
            ("abc-120:xyz-5", "abc-120:xyz-5"),
            ("abc:xyz", "abc:xyz"),
            (":xyz-6.5", ":xyz-6.5"),
            ("abc:", "abc"),
            ("", ""),
        ] {
            assert_eq!(target(text).to_string(), printed);
        }
    }

    #[test]
    fn containment_treats_missing_parts_as_wildcards() {
        // Upstream `test_impersonate_target_in` pairs (`inner in outer`).
        for (outer, inner, is_in) in [
            ("abc", "abc", true),
            ("abc", "abc-120", true),
            ("abc:xyz-test", "abc-120:xyz", true),
            ("abc-121:xyz-test", "abc-120:xyz-test", false),
            ("abc", "abc-120:xyz-test", true),
            ("abc-120:xyz-test", "abc", true),
            ("", "abc-120:xyz", true),
        ] {
            assert_eq!(
                target(outer).contains(&target(inner)),
                is_in,
                "{inner} in {outer}"
            );
        }
    }

    #[test]
    fn preference_order_resolution_and_listing() {
        let sorted = preference_sorted(supported(&[
            "chrome-131:android-14",
            "safari-18.0:macos-15",
            "firefox-135:macos-14",
            "chrome-124:macos-14",
            "chrome-131:macos-14",
            "edge-101:windows-10",
            "safari-18.0:ios-18.0",
            "chrome-131:windows-10",
        ]));
        let order: Vec<String> = sorted
            .iter()
            .map(|entry| entry.target.to_string())
            .collect();
        assert_eq!(
            order,
            [
                "chrome-131:windows-10",
                "chrome-131:macos-14",
                "chrome-124:macos-14",
                "safari-18.0:macos-15",
                "firefox-135:macos-14",
                "edge-101:windows-10",
                "chrome-131:android-14",
                "safari-18.0:ios-18.0",
            ]
        );
        let resolved =
            |text: &str| resolve(&sorted, &target(text)).map(|entry| entry.target.to_string());
        assert_eq!(resolved("").as_deref(), Some("chrome-131:windows-10"));
        assert_eq!(
            resolved("chrome:macos").as_deref(),
            Some("chrome-131:macos-14")
        );
        assert_eq!(resolved("safari").as_deref(), Some("safari-18.0:macos-15"));
        assert_eq!(resolved(":ios").as_deref(), Some("safari-18.0:ios-18.0"));
        assert_eq!(resolved("tor"), None);
        assert_eq!(
            render_targets(&sorted[..3]),
            "Client       OS           Source\n\
             --------------------------------------------\n\
             Edge         -            wreq (unavailable)\n\
             Firefox      -            wreq (unavailable)\n\
             Safari       -            wreq (unavailable)\n\
             Chrome-124   Macos-14     wreq\n\
             Chrome-131   Macos-14     wreq\n\
             Chrome-131   Windows-10   wreq"
        );
        assert_eq!(
            render_targets(&[]),
            "Client    OS   Source\n\
             ---------------------------------\n\
             Edge      -    wreq (unavailable)\n\
             Firefox   -    wreq (unavailable)\n\
             Safari    -    wreq (unavailable)\n\
             Chrome    -    wreq (unavailable)"
        );
    }

    #[test]
    fn unavailable_messages_match_upstream() {
        assert_eq!(
            unavailable_message(
                &[target("")],
                "The extractor is attempting impersonation",
                false
            ),
            "The extractor is attempting impersonation, but no impersonate target is available. If you encounter errors, then see  https://github.com/yt-dlp/yt-dlp#impersonation  for information on installing the required dependencies"
        );
        assert_eq!(
            unavailable_message(
                &[target("chrome"), target("firefox:windows")],
                "The extractor is attempting impersonation",
                true
            ),
            "The extractor is attempting impersonation, but none of these impersonate targets are available: chrome, firefox:windows. See  https://github.com/yt-dlp/yt-dlp#impersonation  for information on installing the required dependencies"
        );
    }
}
