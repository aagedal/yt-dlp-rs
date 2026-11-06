//! Proxy policy shared by normal and bounded player HTTP clients.
use crate::{Error, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use percent_encoding::percent_decode_str;
use reqwest::blocking::ClientBuilder;
use url::Url;

#[derive(Clone, Default)]
pub(crate) enum ProxyConfig {
    #[default]
    Environment,
    Direct,
    Explicit {
        url: Url,
        credentials: Option<(String, Option<String>)>,
    },
}

fn error() -> Error {
    // Proxy URLs can contain credentials. Never include input or parser errors.
    Error::HttpConfiguration("invalid or unsupported proxy URL".into())
}

impl ProxyConfig {
    pub(crate) fn parse(raw: Option<&str>) -> Result<Self> {
        let Some(raw) = raw else {
            return Ok(Self::Environment);
        };
        if matches!(raw, "" | "__noproxy__") {
            return Ok(Self::Direct);
        }
        if raw.len() > 16 * 1024
            || raw.chars().any(|c| c.is_whitespace() || c.is_control())
            || raw.contains('\\')
        {
            return Err(error());
        }
        let normalized;
        let raw = if raw.contains("://") {
            raw
        } else {
            normalized = format!("http://{}", raw.strip_prefix("//").unwrap_or(raw));
            &normalized
        };
        let mut url = Url::parse(raw).map_err(|_| error())?;
        let authority = raw
            .split_once("://")
            .map_or("", |(_, tail)| tail)
            .split(['/', '?', '#'])
            .next()
            .unwrap_or("");
        let credentials = authority
            .rsplit_once('@')
            .map(|(userinfo, _)| {
                let username = percent_decode_str(url.username())
                    .decode_utf8()
                    .map_err(|_| error())?
                    .into_owned();
                let password = if userinfo.contains(':') {
                    Some(
                        percent_decode_str(url.password().unwrap_or(""))
                            .decode_utf8()
                            .map_err(|_| error())?
                            .into_owned(),
                    )
                } else {
                    None
                };
                Ok::<_, Error>((username, password))
            })
            .transpose()?;
        match url.scheme() {
            "http" | "https" | "socks4" | "socks4a" | "socks5h" => {}
            "socks" => {
                url.set_scheme("socks4").map_err(|_| error())?;
            }
            // Pinned yt-dlp preserves its historical remote-DNS SOCKS5 behavior.
            "socks5" => {
                url.set_scheme("socks5h").map_err(|_| error())?;
            }
            _ => return Err(error()),
        }
        if url.host_str().is_none()
            || url.port() == Some(0)
            || !matches!(url.path(), "" | "/")
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(error());
        }
        for value in std::iter::once(url.username()).chain(url.password()) {
            let bytes = value.as_bytes();
            for (index, &byte) in bytes.iter().enumerate() {
                if byte == b'%'
                    && (index + 2 >= bytes.len()
                        || !bytes[index + 1].is_ascii_hexdigit()
                        || !bytes[index + 2].is_ascii_hexdigit())
                {
                    return Err(error());
                }
            }
            let decoded = percent_decode_str(value)
                .decode_utf8()
                .map_err(|_| error())?;
            if decoded.chars().any(char::is_control) {
                return Err(error());
            }
            if url.scheme().starts_with("socks") && decoded.len() > 255 {
                return Err(error());
            }
        }
        // Explicit SOCKS ports avoid depending on backend-specific URL defaults.
        if url.scheme().starts_with("socks") && url.port().is_none() {
            url.set_port(Some(1080)).map_err(|_| error())?;
        }
        reqwest::Proxy::all(url.clone()).map_err(|_| error())?;
        Ok(Self::Explicit { url, credentials })
    }

    pub(crate) fn apply(&self, builder: ClientBuilder) -> Result<ClientBuilder> {
        match self {
            Self::Environment => {
                // Upstream's own discovery replaces reqwest's environment
                // matcher; without any proxy, requests go direct.
                let proxies = EnvironmentProxies::from_env()?;
                if proxies.is_empty() {
                    return Ok(builder.no_proxy());
                }
                Ok(builder
                    .no_proxy()
                    .proxy(reqwest::Proxy::custom(move |url| proxies.target(url))))
            }
            Self::Direct => Ok(builder.no_proxy()),
            Self::Explicit { url, credentials } => {
                let (endpoint, auth) = endpoint(url, credentials.as_ref())?;
                let mut proxy = reqwest::Proxy::all(endpoint).map_err(|_| error())?;
                if let Some(value) = auth {
                    proxy = proxy.custom_http_auth(value);
                }
                // Explicit configuration overrides ambient proxies and NO_PROXY.
                Ok(builder.no_proxy().proxy(proxy))
            }
        }
    }
}

/// An explicit endpoint as a connector URL and the HTTP proxy's
/// `Proxy-Authorization` value: upstream's SOCKS4 USERID (the username; a
/// password is not sent) goes through a loopback relay that inserts it
/// (reached over authenticated SOCKS5, `socks5h` for SOCKS4a), and
/// SOCKS5 offers authentication only with both fields nonempty.
fn endpoint(
    url: &Url,
    credentials: Option<&(String, Option<String>)>,
) -> Result<(Url, Option<reqwest::header::HeaderValue>)> {
    if matches!(url.scheme(), "socks4" | "socks4a")
        && let Some((username, _)) = credentials.filter(|(username, _)| !username.is_empty())
    {
        let host = url.host_str().ok_or_else(error)?;
        let host = host.trim_start_matches('[').trim_end_matches(']');
        let relay = super::socks4_relay::start(
            host,
            url.port().unwrap_or(1080),
            username.as_bytes(),
            super::DEFAULT_SOCKET_TIMEOUT,
        )
        .map_err(|_| error())?;
        let scheme = if url.scheme() == "socks4a" {
            "socks5h"
        } else {
            "socks5"
        };
        let relay = Url::parse(&format!(
            "{scheme}://{token}:{token}@127.0.0.1:{}",
            relay.port,
            token = relay.token
        ))
        .map_err(|_| error())?;
        return Ok((relay, None));
    }
    let mut endpoint = url.clone();
    let http = matches!(url.scheme(), "http" | "https");
    let incomplete_socks_auth = url.scheme() == "socks5h"
        && credentials.is_none_or(|(username, password)| {
            username.is_empty() || password.as_deref().is_none_or(str::is_empty)
        });
    if http || incomplete_socks_auth {
        endpoint.set_username("").map_err(|_| error())?;
        endpoint.set_password(None).map_err(|_| error())?;
    }
    let auth = match credentials {
        Some((username, password)) if http => {
            let encoded =
                STANDARD.encode(format!("{username}:{}", password.as_deref().unwrap_or("")));
            let mut value = reqwest::header::HeaderValue::from_str(&format!("Basic {encoded}"))
                .map_err(|_| error())?;
            value.set_sensitive(true);
            Some(value)
        }
        _ => None,
    };
    Ok((endpoint, auth))
}

/// [`endpoint`] as one URL, with an HTTP proxy's credentials back in its
/// user info.
fn connect_url(url: &Url, credentials: Option<&(String, Option<String>)>) -> Result<Url> {
    let (mut endpoint, auth) = endpoint(url, credentials)?;
    if auth.is_some()
        && let Some((username, password)) = credentials
    {
        let encode = |text: &str| {
            percent_encoding::utf8_percent_encode(text, percent_encoding::NON_ALPHANUMERIC)
                .to_string()
        };
        endpoint
            .set_username(&encode(username))
            .map_err(|_| error())?;
        endpoint
            .set_password(Some(&encode(password.as_deref().unwrap_or(""))))
            .map_err(|_| error())?;
    }
    Ok(endpoint)
}

/// Upstream `YoutubeDL.proxies` without `--proxy` (after `clean_proxies`)
/// and `select_proxy`: the `http`, `https` and `all` routes and the `no`
/// bypass list from the environment.
#[derive(Debug, Default)]
pub(crate) struct EnvironmentProxies {
    /// Each scheme's connector URL; `None` for `__noproxy__`.
    routes: Vec<(String, Option<Url>)>,
    no: Option<String>,
}

impl EnvironmentProxies {
    pub(crate) fn from_env() -> Result<Self> {
        let variables = std::env::vars_os().filter_map(|(name, value)| {
            Some((name.into_string().ok()?, value.into_string().ok()?))
        });
        Self::from_mapping(discover(
            variables,
            std::env::var_os("REQUEST_METHOD").is_some(),
            cfg!(windows),
        ))
    }

    /// From a `getproxies()` mapping: `https` falls back to `http`, then
    /// `clean_proxies` (`__noproxy__` is direct; a scheme-less value is
    /// HTTP; `socks`/`socks5` are SOCKS4/SOCKS5h). Unlike upstream, which
    /// rejects them on every request, an invalid `http`, `https` or `all`
    /// value fails here.
    pub(crate) fn from_mapping(mut mapping: Vec<(String, String)>) -> Result<Self> {
        if mapping.iter().any(|(key, _)| key == "http")
            && !mapping.iter().any(|(key, _)| key == "https")
        {
            let http = mapping
                .iter()
                .find(|(key, _)| key == "http")
                .map(|(_, value)| value.clone())
                .unwrap_or_default();
            mapping.push(("https".into(), http));
        }
        let mut proxies = Self::default();
        for (key, value) in mapping {
            match key.as_str() {
                "no" => proxies.no = Some(value),
                "http" | "https" | "all" => {
                    let route = match ProxyConfig::parse(Some(&value))? {
                        ProxyConfig::Explicit { url, credentials } => {
                            Some(connect_url(&url, credentials.as_ref())?)
                        }
                        _ => None,
                    };
                    proxies.routes.push((key, route));
                }
                _ => {}
            }
        }
        Ok(proxies)
    }

    /// No route: every request is direct.
    pub(crate) fn is_empty(&self) -> bool {
        self.routes.iter().all(|(_, route)| route.is_none())
    }

    /// `select_proxy(url, proxies)`: `None` for a direct connection.
    pub(crate) fn target(&self, url: &Url) -> Option<Url> {
        if let Some(no) = &self.no {
            let host = url.host_str().unwrap_or_default();
            let host = host.trim_start_matches('[').trim_end_matches(']');
            let hostport = match url.port() {
                Some(port) => format!("{host}:{port}"),
                None => host.to_owned(),
            };
            if bypass(&hostport, no) {
                return None;
            }
        }
        // `traverse_obj(proxies, scheme, 'all')`: a `None` route falls back.
        let route = |key: &str| {
            self.routes
                .iter()
                .find(|(name, _)| name == key)
                .and_then(|(_, route)| route.clone())
        };
        route(url.scheme()).or_else(|| route("all"))
    }
}

/// Python `urllib.request.getproxies_environment` over `variables`:
/// `NAME_proxy` values by lowercased `NAME`, all-lowercase variable names
/// last so they win (an empty one removes the scheme), and no uppercase
/// `HTTP_PROXY` under CGI (`REQUEST_METHOD`). On Windows Python sees every
/// variable name in uppercase.
pub(crate) fn discover(
    variables: impl Iterator<Item = (String, String)>,
    cgi: bool,
    windows: bool,
) -> Vec<(String, String)> {
    let mut proxies: Vec<(String, String)> = Vec::new();
    let set = |proxies: &mut Vec<(String, String)>, key: &str, value: Option<String>| {
        let position = proxies.iter().position(|(name, _)| name == key);
        match (position, value) {
            (Some(index), Some(value)) => proxies[index].1 = value,
            (None, Some(value)) => proxies.push((key.to_owned(), value)),
            (Some(index), None) => {
                proxies.remove(index);
            }
            (None, None) => {}
        }
    };
    let environment: Vec<(String, String)> = variables
        .map(|(name, value)| {
            if windows {
                (name.to_uppercase(), value)
            } else {
                (name, value)
            }
        })
        .filter(|(name, _)| {
            name.len() > 5
                && name.as_bytes()[name.len() - 6] == b'_'
                && name[name.len() - 5..].eq_ignore_ascii_case("proxy")
        })
        .collect();
    for (name, value) in &environment {
        if !value.is_empty() {
            set(
                &mut proxies,
                &name[..name.len() - 6].to_lowercase(),
                Some(value.clone()),
            );
        }
    }
    if cgi {
        set(&mut proxies, "http", None);
    }
    for (name, value) in &environment {
        if name.ends_with("_proxy") {
            let key = name[..name.len() - 6].to_lowercase();
            set(
                &mut proxies,
                &key,
                (!value.is_empty()).then(|| value.clone()),
            );
        }
    }
    proxies
}

/// Python `urllib.request.proxy_bypass_environment(hostport, {'no': no})`.
fn bypass(hostport: &str, no: &str) -> bool {
    if no == "*" {
        return true;
    }
    let host = hostport.to_lowercase();
    // `_splitport`: a trailing `:DIGITS` is the port.
    let host_only = match host.rsplit_once(':') {
        Some((name, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => name,
        _ => host.as_str(),
    };
    no.split(',')
        .map(|name| name.trim_matches(PYTHON_WHITESPACE))
        .filter(|name| !name.is_empty())
        .any(|name| {
            let name = name.trim_start_matches('.').to_lowercase();
            if host_only == name || host == name {
                return true;
            }
            let suffix = format!(".{name}");
            host_only.ends_with(&suffix) || host.ends_with(&suffix)
        })
}

/// `str.strip()` whitespace (ASCII and the Unicode separators Python strips).
const PYTHON_WHITESPACE: &[char] = &[
    ' ', '\t', '\n', '\r', '\x0b', '\x0c', '\x1c', '\x1d', '\x1e', '\x1f', '\u{85}', '\u{a0}',
    '\u{1680}', '\u{2000}', '\u{2001}', '\u{2002}', '\u{2003}', '\u{2004}', '\u{2005}', '\u{2006}',
    '\u{2007}', '\u{2008}', '\u{2009}', '\u{200a}', '\u{2028}', '\u{2029}', '\u{202f}', '\u{205f}',
    '\u{3000}',
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_explicit_proxy_endpoints_match_without_using_ambient_environment() {
        use serde_json::{Value, json};
        let fixture: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/proxies/cases.json")).unwrap();
        let mut compared = 0;
        for case in fixture["cases"].as_array().unwrap() {
            let raw = case["expected"]["proxy_option"].as_str();
            if raw.is_none() {
                assert!(matches!(
                    ProxyConfig::parse(None).unwrap(),
                    ProxyConfig::Environment
                ));
                // A discovered `getproxies()` mapping.
                let mapping = case["environment"]
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(key, value)| (key.clone(), value.as_str().unwrap().to_owned()))
                    .collect();
                assert_environment(case, EnvironmentProxies::from_mapping(mapping).unwrap());
                continue;
            }
            let actual = match ProxyConfig::parse(raw).unwrap() {
                ProxyConfig::Direct => Value::Null,
                ProxyConfig::Explicit { url, credentials } => {
                    let username = credentials.as_ref().map(|(username, _)| username);
                    let password = credentials
                        .as_ref()
                        .and_then(|(_, password)| password.as_ref());
                    let host = url
                        .host_str()
                        .unwrap()
                        .trim_start_matches('[')
                        .trim_end_matches(']');
                    json!({"scheme": url.scheme(), "host": host, "port": url.port_or_known_default().unwrap(), "username": username, "password": password, "rdns": match url.scheme() { "socks4" => Some(false), "socks4a" | "socks5h" => Some(true), _ => None }})
                }
                ProxyConfig::Environment => panic!("explicit proxy became environment"),
            };
            for expected in case["expected"]["selected"].as_array().unwrap() {
                assert_eq!(&actual, expected, "{}", case["name"]);
            }
            compared += 1;
        }
        assert_eq!(compared, 44);
    }

    /// The projection `tools/compare_proxies.py` records for a connector URL.
    fn projection(url: Option<Url>) -> serde_json::Value {
        let Some(url) = url else {
            return serde_json::Value::Null;
        };
        let decode = |text: &str| percent_decode_str(text).decode_utf8().unwrap().into_owned();
        let has_userinfo = !url.username().is_empty() || url.password().is_some();
        serde_json::json!({
            "scheme": url.scheme(),
            "host": url.host_str().unwrap().trim_start_matches('[').trim_end_matches(']'),
            "port": url.port_or_known_default().unwrap(),
            "username": has_userinfo.then(|| decode(url.username())),
            "password": url.password().map(decode),
            "rdns": match url.scheme() {
                "socks4" => Some(false),
                "socks4a" | "socks5h" => Some(true),
                _ => None,
            },
        })
    }

    fn assert_environment(case: &serde_json::Value, proxies: EnvironmentProxies) {
        let selected: Vec<serde_json::Value> = case["requests"]
            .as_array()
            .unwrap()
            .iter()
            .map(|url| projection(proxies.target(&Url::parse(url.as_str().unwrap()).unwrap())))
            .collect();
        assert_eq!(
            serde_json::Value::from(selected),
            case["expected"]["selected"],
            "{}",
            case["name"]
        );
    }

    /// Raw environment variables through `getproxies_environment`,
    /// `YoutubeDL.proxies`, `clean_proxies` and `select_proxy`, compared with
    /// the pinned source's results.
    #[test]
    fn environment_variables_select_routes_like_pinned_source() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/proxies/cases.json")).unwrap();
        let cases = fixture["environment_cases"].as_array().unwrap();
        assert_eq!(cases.len(), 15);
        for case in cases {
            let variables = case["variables"].as_object().unwrap();
            let discovered = discover(
                variables
                    .iter()
                    .map(|(name, value)| (name.clone(), value.as_str().unwrap().to_owned())),
                variables.contains_key("REQUEST_METHOD"),
                false,
            );
            assert_environment(case, EnvironmentProxies::from_mapping(discovered).unwrap());
        }
    }

    /// Python on Windows sees variable names in uppercase, so a lowercase
    /// `http_proxy` is not exempt from the CGI rule there.
    #[test]
    fn windows_variable_names_are_uppercase() {
        let variables = || [("http_proxy".to_owned(), "http://lower.test".to_owned())].into_iter();
        assert_eq!(
            discover(variables(), true, false),
            [("http".to_owned(), "http://lower.test".to_owned())]
        );
        assert!(discover(variables(), true, true).is_empty());
        assert_eq!(
            discover(variables(), false, true),
            [("http".to_owned(), "http://lower.test".to_owned())]
        );
    }

    #[test]
    fn invalid_environment_routes_fail_without_exposing_values() {
        for value in [
            "ftp://user:secret@proxy.test",
            "http://user:secret@proxy.test/path",
        ] {
            let error = EnvironmentProxies::from_mapping(vec![("https".into(), value.into())])
                .unwrap_err()
                .to_string();
            assert!(!error.contains("secret"), "{error}");
        }
        // Other schemes' values are not used, so they are not checked.
        assert!(
            EnvironmentProxies::from_mapping(vec![("colab_language_server".into(), "/x".into())])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn malformed_proxy_values_fail_without_exposing_credentials() {
        for raw in [
            "ftp://user:secret@example.test",
            "http://user:secret@example.test/path",
            "http://user:secret@example.test?query",
            "http://user:secret@example.test/#fragment",
            "http://user:secret@example.test:0",
            "http://user:secret@",
            "http://user:secret%XX@example.test",
            "http://user:secret%0A@example.test",
            "http://user:secret%FF@example.test",
            " http://user:secret@example.test",
            "http://user:secret@exam\\ple.test",
        ] {
            let message = ProxyConfig::parse(Some(raw)).err().unwrap().to_string();
            assert!(!message.contains("secret"));
            assert!(!message.contains("user"));
        }
        assert!(ProxyConfig::parse(Some(&"x".repeat(16 * 1024 + 1))).is_err());
    }
}

#[cfg(test)]
mod tls;
