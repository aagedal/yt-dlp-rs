//! Upstream `AdobePassIE` (`yt_dlp/extractor/adobepass.py`): TV provider
//! (MVPD) authentication for extractors of TV Everywhere sites.
//! [`extract_mvpd_auth`] registers a device and client with Adobe, follows
//! the `--ap-mso` provider's SAML login (the generic form-post flow and the
//! special-cased `Comcast_SSO`, `Verizon`, `Spectrum`/`Charter_Direct`,
//! `slingtv` and `Suddenlink` flows) with `--ap-username`/`--ap-password`
//! or the MSO's netrc machine, and returns the short media token.
//!
//! Upstream caches the authentication and authorization tokens in its
//! `ap-mvpd` cache section; this port has no cache directory, so they are
//! kept in memory for the run. `Philo` (an interactive code prompt) and
//! `Fubo` are explicit unsupported errors, and `--geo-verification-proxy`
//! headers are not added to these requests.
mod mso_info;
mod netrc;

pub use netrc::LoginOptions;

use crate::{Error, Result};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::{LazyLock, Mutex, RwLock};
use url::Url;

/// An `MSO_INFO` entry.
pub(crate) struct MsoInfo {
    pub(crate) name: &'static str,
    pub(crate) username_field: Option<&'static str>,
    pub(crate) password_field: Option<&'static str>,
    pub(crate) login_hostname: Option<&'static str>,
}

const SERVICE_PROVIDER_TEMPLATE: &str = "https://sp.auth.adobe.com/adobe-services/";
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux i686; rv:47.0) Gecko/20100101 Firefox/47.0";
const DOWNLOADING_LOGIN_PAGE: &str = "Downloading Provider Login Page";

/// `MSO_INFO[mso_id]`.
pub(crate) fn mso_info(mso_id: &str) -> Option<&'static MsoInfo> {
    mso_info::MSO_INFO
        .iter()
        .find(|(id, _)| *id == mso_id)
        .map(|(_, info)| info)
}

/// Whether `--ap-mso` names an `MSO_INFO` entry (`validate_in`).
pub fn supported_mso(mso_id: &str) -> bool {
    mso_info(mso_id).is_some()
}

/// `--ap-list-mso`: `Supported TV Providers:` and the `mso`/`mso name`
/// table, with its trailing newline.
pub fn list_mso_text() -> String {
    let rows = mso_info::MSO_INFO
        .iter()
        .map(|(id, info)| vec![(*id).to_owned(), info.name.to_owned()])
        .collect();
    format!(
        "Supported TV Providers:\n{}\n",
        crate::output::formats::render_table(
            vec!["mso".into(), "mso name".into()],
            rows,
            None,
            0,
            false
        )
    )
}

/// `--ap-mso` and the account options `_get_login_info('ap_username',
/// 'ap_password', mso_id)` reads.
#[derive(Clone, Debug, Default)]
pub struct Options {
    pub mso: Option<String>,
    pub login: LoginOptions,
}

static OPTIONS: RwLock<Option<Options>> = RwLock::new(None);

/// Set the Adobe Pass options for extractors that authenticate.
pub fn set_options(options: Options) {
    if let Ok(mut slot) = OPTIONS.write() {
        *slot = Some(options);
    }
}

fn options() -> Options {
    OPTIONS
        .read()
        .ok()
        .and_then(|options| options.clone())
        .unwrap_or_default()
}

/// The `ap-mvpd` cache section by requestor id (in memory: this port has
/// no cache directory).
static CACHE: LazyLock<Mutex<std::collections::HashMap<String, Map<String, Value>>>> =
    LazyLock::new(Default::default);

fn cache_load(requestor_id: &str) -> Map<String, Value> {
    CACHE
        .lock()
        .ok()
        .and_then(|cache| cache.get(requestor_id).cloned())
        .unwrap_or_default()
}

fn cache_store(requestor_id: &str, info: Map<String, Value>) {
    if let Ok(mut cache) = CACHE.lock() {
        cache.insert(requestor_id.to_owned(), info);
    }
}

/// The calling extractor: its name and video id for notes and request
/// errors, and the `[EXTRACTOR] ` (with `get_temp_id`'s `ID: `) prefix of
/// `ExtractorError`s raised without a video id.
pub(crate) struct Caller<'a> {
    pub(crate) extractor: &'static str,
    pub(crate) id: &'a str,
    pub(crate) prefix: String,
}

/// A downloaded page: its text and final URL (`urlh.url`), and headers.
struct Page {
    text: String,
    url: Url,
    headers: reqwest::header::HeaderMap,
}

/// A Python dict of strings: insertion ordered, updated in place.
#[derive(Clone, Default)]
struct Dict(Vec<(String, String)>);

impl Dict {
    fn set(&mut self, key: &str, value: &str) {
        match self.0.iter_mut().find(|(existing, _)| existing == key) {
            Some((_, slot)) => *slot = value.to_owned(),
            None => self.0.push((key.to_owned(), value.to_owned())),
        }
    }

    fn pairs(&self) -> Vec<(&str, &str)> {
        self.0
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect()
    }

    /// `urlencode_postdata(dict)`.
    fn urlencode(&self) -> String {
        url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(&self.0)
            .finish()
    }
}

fn dict(pairs: &[(&str, &str)]) -> Dict {
    let mut dict = Dict::default();
    for (key, value) in pairs {
        dict.set(key, value);
    }
    dict
}

/// `_hidden_inputs(html)` in document order.
fn hidden_inputs(html: &str) -> Dict {
    Dict(super::presstv::hidden_inputs(html))
}

struct Flow<'a> {
    client: &'a Client,
    caller: &'a Caller<'a>,
    mso_id: Option<&'a str>,
    mso_info: Option<&'static MsoInfo>,
}

impl Flow<'_> {
    fn error(&self, message: &str) -> Error {
        Error::Reported(format!("{}{message}", self.caller.prefix))
    }

    /// A non-`expected` `ExtractorError`: with the bug report suffix.
    fn bug(&self, message: &str) -> Error {
        self.error(&format!("{message}; {}", crate::BUG_REPORTS_MESSAGE))
    }

    fn key_error(&self, key: &str) -> Error {
        self.bug(&format!(
            "An extractor error has occurred. (caused by KeyError('{key}'))"
        ))
    }

    fn mvpd_required(&self) -> Error {
        self.error(
            "This video is only available for users of participating TV providers. Use --ap-mso to specify Adobe Pass Multiple-system operator Identifier and --ap-username and --ap-password or --netrc to provide account credentials.",
        )
    }

    /// `_search_regex(pattern, text, name)` (fatal): the first matching
    /// group.
    fn search(&self, pattern: &Regex, text: &str, name: &str) -> Result<String> {
        pattern
            .captures(text)
            .and_then(|found| found.iter().skip(1).flatten().next())
            .map(|group| group.as_str().to_owned())
            .ok_or_else(|| self.bug(&format!("Unable to extract {name}")))
    }

    /// `_html_search_regex`: [`Flow::search`] through `clean_html`.
    fn html_search(&self, pattern: &Regex, text: &str, name: &str) -> Result<String> {
        self.search(pattern, text, name)
            .map(|found| crate::html::clean_html(&found).trim().to_owned())
    }

    /// `xml_text(xml_str, tag)`.
    fn xml_text(&self, xml: &str, tag: &str) -> Result<String> {
        let escaped = regex::escape(tag);
        let pattern = Regex::new(&format!("<{escaped}>(.+?)</{escaped}>")).expect("escaped tag");
        self.search(&pattern, xml, tag)
    }

    /// `is_expired(token, date_ele)`.
    fn is_expired(&self, token: &str, element: &str) -> Result<bool> {
        static GMT: LazyLock<Regex> = LazyLock::new(|| Regex::new("[_ ]GMT").unwrap());
        let text = self.xml_text(token, element)?;
        let expires = crate::dates::unified_timestamp(&GMT.replace_all(&text, ""), false);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs() as i64);
        Ok(expires.is_some_and(|expires| expires != 0 && expires <= now))
    }

    /// A URL found in a page (upstream requests any string).
    fn url(&self, text: &str) -> Result<Url> {
        Url::parse(text).map_err(|_| {
            Error::UnsupportedHtml5(format!(
                "Adobe Pass URLs that are not absolute are not supported natively: {text}"
            ))
        })
    }

    /// `_download_webpage_handle`/`_download_json_handle`: the note, the GET
    /// (or POST of `data`) with `headers`, and the decoded page.
    fn request(
        &self,
        url: &Url,
        note: &str,
        what: &'static str,
        data: Option<String>,
        headers: &[(&str, &str)],
    ) -> Result<Page> {
        use std::io::Read;
        let (extractor, id) = (self.caller.extractor, self.caller.id);
        crate::screen::screen(&format!("[{extractor}] {id}: {note}"));
        let wrap = |error: Error| match error {
            Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
                extractor,
                id: id.to_owned(),
                what,
                source: Box::new(error),
            },
            error => error,
        };
        let response = match &data {
            Some(data) => super::direct::post(self.client, url, headers, data.clone()),
            None => super::direct::request_with_headers(self.client, url, None, headers),
        };
        let mut response = response.map_err(wrap)?;
        let page_url = response.url().clone();
        let mut body = Vec::new();
        response
            .read_to_end(&mut body)
            .map_err(Error::ExtractionRead)?;
        let body =
            super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
        match &data {
            Some(data) => crate::screen::expect_post_page(id, data.as_bytes()),
            None => crate::screen::expect_page(id, None, None),
        }
        crate::screen::page_read(&page_url, &body)?;
        Ok(Page {
            text: super::decode_webpage(&body, response.headers()),
            url: page_url,
            headers: response.headers().clone(),
        })
    }

    fn webpage(
        &self,
        url: &Url,
        note: &str,
        data: Option<String>,
        headers: &[(&str, &str)],
    ) -> Result<Page> {
        self.request(url, note, "webpage", data, headers)
    }

    fn json(
        &self,
        url: &Url,
        note: &str,
        data: Option<String>,
        headers: &[(&str, &str)],
    ) -> Result<(Value, Page)> {
        let page = self.request(url, note, "JSON metadata", data, headers)?;
        let value = crate::json::from_str(&page.text).map_err(|error| {
            Error::Reported(format!(
                "[{}] {}: Failed to parse JSON  (caused by {error})",
                self.caller.extractor, self.caller.id
            ))
        })?;
        Ok((value, page))
    }

    /// `value[key]` as Python `str()` would format it into a request.
    fn field(&self, value: &Value, key: &str) -> Result<String> {
        match value.get(key) {
            None => Err(self.key_error(key)),
            Some(Value::String(text)) => Ok(text.clone()),
            Some(Value::Number(number)) if number.is_i64() || number.is_u64() => {
                Ok(number.to_string())
            }
            Some(_) => Err(Error::UnsupportedHtml5(format!(
                "Adobe Pass {key} values that are not strings or integers are not supported natively"
            ))),
        }
    }

    /// `mso_info[FIELD]` of a special-cased MSO.
    fn mso_field(&self, field: &str) -> Result<&'static str> {
        let info = self.mso_info;
        match field {
            "username_field" => info.and_then(|info| info.username_field),
            _ => info.and_then(|info| info.password_field),
        }
        .ok_or_else(|| self.key_error(field))
    }

    fn credentials(&self, username: &str, password: &str) -> Result<Dict> {
        Ok(dict(&[
            (self.mso_field("username_field")?, username),
            (self.mso_field("password_field")?, password),
        ]))
    }

    /// `post_form(form_page_res, note, data, validate_url)`.
    fn post_form(&self, page: &Page, note: &str, data: &Dict, validate_url: bool) -> Result<Page> {
        static ACTION: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r#"<form[^>]+action=(?:"(?P<a>.+?)"|'(?P<b>.+?)')"#).unwrap()
        });
        let post_url = self.html_search(&ACTION, &page.text, "post url")?;
        static ABSOLUTE: LazyLock<Regex> = LazyLock::new(|| Regex::new("^https?://").unwrap());
        let mut post_url = if ABSOLUTE.is_match(&post_url) {
            self.url(&post_url)?
        } else {
            page.url.join(&post_url).map_err(|_| {
                Error::UnsupportedHtml5(format!(
                    "Adobe Pass form actions that do not resolve are not supported natively: {post_url}"
                ))
            })?
        };
        if validate_url {
            // This request is submitting credentials so we should validate
            // it when possible.
            let hostname = post_url.host_str().unwrap_or_default().to_lowercase();
            if let Some(expected) = self.mso_info.and_then(|info| info.login_hostname)
                && expected != hostname
            {
                return Err(self.bug(&format!(
                    "Unexpected login URL hostname; expected \"{expected}\" but got \"{hostname}\". Aborting before submitting credentials"
                )));
            }
            if post_url.scheme() != "https" {
                crate::screen::extractor_debug("Upgrading login URL scheme to https");
                post_url = self.url(&post_url.as_str().replacen(
                    &format!("{}:", post_url.scheme()),
                    "https:",
                    1,
                ))?;
            }
        }
        let mut form = hidden_inputs(&page.text);
        for (key, value) in &data.0 {
            form.set(key, value);
        }
        self.webpage(
            &post_url,
            note,
            Some(form.urlencode()),
            &[("Content-Type", "application/x-www-form-urlencoded")],
        )
    }

    /// A GET of `url` with `query=` (`update_url_query`).
    fn get_with_query(&self, url: &Url, note: &str, query: &Dict) -> Result<Page> {
        self.webpage(
            &super::update_url_query(url, &query.pairs()),
            note,
            None,
            &[],
        )
    }

    /// `extract_redirect_url(html, url=url)`: the `http-equiv="refresh"`
    /// meta tag's URL (joined with `url` after `unescapeHTML`).
    fn redirect_url(&self, html: &str, url: Option<&Url>, fatal: bool) -> Result<Option<String>> {
        static META: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<meta\s+").unwrap());
        // The lookahead `(?=(?:[a-z-]+="[^"]+"\s+)*http-equiv="refresh")`.
        static REFRESH: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r#"(?i)^(?:[a-z-]+="[^"]+"\s+)*http-equiv="refresh""#).unwrap()
        });
        static CONTENT: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(
                r#"(?i)^(?:[a-z-]+="[^"]+"\s+)*?content="[0-9]{0,2};\s*(?:URL|url)='?([^'"]+)"#,
            )
            .unwrap()
        });
        let found = META.find_iter(html).find_map(|meta| {
            let rest = &html[meta.end()..];
            if !REFRESH.is_match(rest) {
                return None;
            }
            CONTENT.captures(rest).map(|found| found[1].to_owned())
        });
        let Some(found) = found else {
            if fatal {
                return Err(self.bug("Unable to extract meta refresh redirect"));
            }
            return Ok(None);
        };
        Ok(Some(match url {
            Some(base) => base
                .join(&crate::html::unescape_html(&found))
                .map_or(found, |joined| joined.to_string()),
            None => found,
        }))
    }

    /// A redirect upstream would request even when missing (`None`).
    fn required_redirect(&self, html: &str, url: &Url) -> Result<Url> {
        let found = self.redirect_url(html, Some(url), false)?.ok_or_else(|| {
            Error::UnsupportedHtml5(
                "Adobe Pass association pages without a meta refresh redirect are not supported natively".into(),
            )
        })?;
        self.url(&found)
    }

    /// The MSO's login after the `authenticate/saml` redirect page.
    fn login(&self, mso_id: &str, redirect: Page, username: &str, password: &str) -> Result<()> {
        let form = &[("Content-Type", "application/x-www-form-urlencoded")];
        match mso_id {
            "Comcast_SSO" => {
                // Comcast page flow varies by video site and whether you
                // are on Comcast's network.
                static SIGNING_IN: LazyLock<Regex> =
                    LazyLock::new(|| Regex::new(r#"window\.location\s*=\s*['"]([^'"]+)"#).unwrap());
                static SIGNED_IN: LazyLock<Regex> = LazyLock::new(|| {
                    Regex::new(r#"continue:\s*"(https://oauth\.xfinity\.com/oauth/authorize\?.+)""#)
                        .unwrap()
                });
                if redirect.text.contains("automatically signing you in") {
                    let url = self.html_search(&SIGNING_IN, &redirect.text, "oauth redirect")?;
                    self.webpage(&self.url(&url)?, "Confirming auto login", None, &[])?;
                } else if redirect.text.contains("automatically signed in with") {
                    let url =
                        self.html_search(&SIGNED_IN, &redirect.text, "oauth redirect (signed)")?;
                    self.webpage(&self.url(&url)?, "Confirming auto login", None, &[])?;
                } else {
                    let login_page = if redirect.text.contains("<form name=\"signin\"") {
                        redirect
                    } else if redirect.text.contains("http-equiv=\"refresh\"") {
                        let url = self
                            .redirect_url(&redirect.text, None, true)?
                            .unwrap_or_default();
                        self.webpage(&self.url(&url)?, DOWNLOADING_LOGIN_PAGE, None, &[])?
                    } else {
                        self.post_form(&redirect, DOWNLOADING_LOGIN_PAGE, &Dict::default(), false)?
                    };
                    let confirm = self.post_form(
                        &login_page,
                        "Logging in",
                        &self.credentials(username, password)?,
                        true,
                    )?;
                    if confirm
                        .text
                        .contains("<button class=\"submit\" value=\"Resume\">Resume</button>")
                    {
                        self.post_form(&confirm, "Confirming Login", &Dict::default(), false)?;
                    }
                }
            }
            "Verizon" => {
                // In general, if you're connecting from a Verizon-assigned
                // IP, you will not actually pass your credentials.
                static PARENT_LOCATION: LazyLock<Regex> = LazyLock::new(|| {
                    Regex::new(r#"self\.parent\.location=(?:"(?P<a>.+?)"|'(?P<b>.+?)')"#).unwrap()
                });
                static VAR_URL: LazyLock<Regex> = LazyLock::new(|| {
                    Regex::new(r#"var\surl\s*=\s*(?:"(?P<a>.+?)"|'(?P<b>.+?)')"#).unwrap()
                });
                static XHR_POST: LazyLock<Regex> = LazyLock::new(|| {
                    Regex::new(r#"xmlHttp\.open\("POST"\s*,\s*(?:"(?P<a>.+?)"|'(?P<b>.+?)')"#)
                        .unwrap()
                });
                let saml_login_page = if redirect.text.contains("Please wait ...")
                    && !redirect.text.contains("'N'== \"Y\"")
                {
                    let url =
                        self.html_search(&PARENT_LOCATION, &redirect.text, "SAML Redirect URL")?;
                    self.webpage(&self.url(&url)?, "Downloading SAML Login Page", None, &[])?
                } else if redirect.text.contains("Verizon FiOS - sign in") {
                    // FXNetworks from non-Verizon IP.
                    let page = self.post_form(
                        &redirect,
                        "Logging in",
                        &self.credentials(username, password)?,
                        true,
                    )?;
                    if page.text.contains("Please try again.") {
                        return Err(self.bug(
                            "We're sorry, but either the User ID or Password entered is not correct.",
                        ));
                    }
                    page
                } else {
                    // ABC from non-Verizon IP.
                    let url = self
                        .html_search(&VAR_URL, &redirect.text, "SAML Redirect URL")?
                        .replace(r"\/", "/")
                        .replace(r"\-", "-")
                        .replace(r"\x26", "&");
                    let url = self.url(&url)?;
                    let mut page = self.webpage(&url, "Downloading SAML Login Page", None, &[])?;
                    // `post_form([saml_login_page, saml_redirect_url], ...)`:
                    // a relative action joins the redirect URL.
                    page.url = url;
                    let page = self.post_form(
                        &page,
                        "Logging in",
                        &self.credentials(username, password)?,
                        true,
                    )?;
                    if page.text.contains("Please try again.") {
                        return Err(self.bug("Failed to login, incorrect User ID or Password."));
                    }
                    page
                };
                let login_url = self.search(&XHR_POST, &saml_login_page.text, "SAML Login URL")?;
                let (response, _) = self.json(
                    &self.url(&login_url)?,
                    "Downloading SAML Response",
                    None,
                    &[("Content-Type", "text/xml")],
                )?;
                let target = self.field(&response, "targetValue")?;
                let data = dict(&[
                    ("SAMLResponse", &self.field(&response, "SAMLResponse")?),
                    ("RelayState", &self.field(&response, "RelayState")?),
                ]);
                self.webpage(
                    &self.url(&target)?,
                    "Confirming Login",
                    Some(data.urlencode()),
                    form,
                )?;
            }
            "Spectrum" | "Charter_Direct" => {
                // Spectrum's login form is dynamically loaded via JS so the
                // flow is hardcoded.
                static RELAY_STATE: LazyLock<Regex> =
                    LazyLock::new(|| Regex::new(r#"RelayState\s*=\s*"(?P<relay>.+?)";"#).unwrap());
                static SAML_REQUEST: LazyLock<Regex> = LazyLock::new(|| {
                    Regex::new(r#"SAMLRequest\s*=\s*"(?P<saml_request>.+?)";"#).unwrap()
                });
                let login_page =
                    self.post_form(&redirect, DOWNLOADING_LOGIN_PAGE, &Dict::default(), false)?;
                let relay_state = self.search(&RELAY_STATE, &login_page.text, "RelayState")?;
                let saml_request = self.search(&SAML_REQUEST, &login_page.text, "SAMLRequest")?;
                let mut login = Map::new();
                login.insert(self.mso_field("username_field")?.into(), json!(username));
                login.insert(self.mso_field("password_field")?.into(), json!(password));
                login.insert("RelayState".into(), json!(relay_state));
                login.insert("SAMLRequest".into(), json!(saml_request));
                let (response, _) = self.json(
                    &Url::parse(
                        "https://tveauthn.spectrum.net/tveauthentication/api/v1/manualAuth",
                    )?,
                    "Downloading SAML Response",
                    Some(crate::output::traverse::py_json(
                        &Value::Object(login),
                        true,
                    )),
                    &[
                        ("Content-Type", "application/json"),
                        ("Accept", "application/json"),
                    ],
                )?;
                let target = self.field(&response, "SAMLRedirectUri")?;
                let data = dict(&[
                    ("SAMLResponse", &self.field(&response, "SAMLResponse")?),
                    ("RelayState", &relay_state),
                ]);
                self.webpage(
                    &self.url(&target)?,
                    "Confirming Login",
                    Some(data.urlencode()),
                    form,
                )?;
            }
            "slingtv" => {
                // SlingTV has a meta-refresh based authentication, but also
                // looks at the tab history to count the number of times the
                // browser has been on a page.
                let mut hidden = hidden_inputs(&redirect.text);
                hidden.set("history", "1");
                let login_page =
                    self.get_with_query(&redirect.url, "Sending first bookend", &hidden)?;
                let association = self.post_form(
                    &login_page,
                    "Logging in",
                    &self.credentials(username, password)?,
                    true,
                )?;
                let refresh = self.required_redirect(&association.text, &association.url)?;
                let last = self.webpage(
                    &refresh,
                    "Downloading Auth Association Redirect Page",
                    None,
                    &[],
                )?;
                let mut hidden = hidden_inputs(&last.text);
                hidden.set("history", "3");
                let confirm = self.get_with_query(&last.url, "Sending final bookend", &hidden)?;
                self.post_form(&confirm, "Confirming Login", &Dict::default(), false)?;
            }
            "Suddenlink" => {
                // Like SlingTV with a tab history count and a meta refresh,
                // plus a dynamic JavaScript redirect.
                static AJAX_URL: LazyLock<Regex> =
                    LazyLock::new(|| Regex::new(r#"url:\s*['"]([^'"]+)"#).unwrap());
                let first =
                    self.post_form(&redirect, "Pressing Continue...", &Dict::default(), false)?;
                let mut hidden = hidden_inputs(&first.text);
                hidden.set("history_val", "1");
                let login_redirect =
                    self.get_with_query(&first.url, "Sending First Bookend", &hidden)?;
                // Some website partners do not have the extra ajaxurl
                // redirect step.
                let login_page = if login_redirect
                    .text
                    .contains("id=\"password\" type=\"password\" name=\"password\"")
                {
                    login_redirect
                } else {
                    let tryauth_url =
                        self.html_search(&AJAX_URL, &login_redirect.text, "ajaxurl")?;
                    let tryauth = self.get_with_query(
                        &self.url(&tryauth_url)?,
                        "Submitting TryAuth",
                        &hidden,
                    )?;
                    let url = self.url(&format!(
                        "https://authorize.suddenlink.net/saml/module.php/authSynacor/login.php?AuthState={}",
                        tryauth.text
                    ))?;
                    self.get_with_query(&url, "Getting Login Page", &hidden)?
                };
                let association = self.post_form(
                    &login_page,
                    "Logging in",
                    &self.credentials(username, password)?,
                    true,
                )?;
                let refresh = self.required_redirect(&association.text, &association.url)?;
                let last = self.webpage(
                    &refresh,
                    "Downloading Auth Association Redirect Page",
                    None,
                    &[],
                )?;
                let mut hidden = hidden_inputs(&last.text);
                hidden.set("history_val", "3");
                let confirm = self.get_with_query(&last.url, "Sending Final Bookend", &hidden)?;
                self.post_form(&confirm, "Confirming Login", &Dict::default(), false)?;
            }
            "Philo" | "Fubo" => {
                return Err(Error::UnsupportedHtml5(format!(
                    "Adobe Pass {mso_id} logins are not supported natively"
                )));
            }
            _ => {
                // Some providers (e.g. DIRECTV NOW) have another meta
                // refresh based redirect that should be followed.
                let mut redirect = redirect;
                if let Some(refresh) =
                    self.redirect_url(&redirect.text, Some(&redirect.url), false)?
                {
                    redirect = self.webpage(
                        &self.url(&refresh)?,
                        "Downloading Provider Redirect Page (meta refresh)",
                        None,
                        &[],
                    )?;
                }
                let login_page =
                    self.post_form(&redirect, DOWNLOADING_LOGIN_PAGE, &Dict::default(), false)?;
                let info = self.mso_info;
                let mut data = dict(&[
                    (
                        info.and_then(|info| info.username_field)
                            .unwrap_or("username"),
                        username,
                    ),
                    (
                        info.and_then(|info| info.password_field)
                            .unwrap_or("password"),
                        password,
                    ),
                ]);
                if matches!(mso_id, "Cablevision" | "AlticeOne") {
                    data.set("_eventId_proceed", "");
                }
                let confirm = self.post_form(&login_page, "Logging in", &data, true)?;
                if mso_id != "Rogers" {
                    self.post_form(&confirm, "Confirming Login", &Dict::default(), false)?;
                }
            }
        }
        Ok(())
    }
}

/// Upstream `_extract_mvpd_auth(url, video_id, requestor_id, resource,
/// software_statement)`: the short media token for `resource` (a resource
/// id or an RSS `<guid>` document).
pub(crate) fn extract_mvpd_auth(
    client: &Client,
    caller: &Caller<'_>,
    url: &str,
    requestor_id: &str,
    resource: &str,
    software_statement: &Value,
) -> Result<String> {
    let options = options();
    let mso_id = options.mso.as_deref();
    let flow = Flow {
        client,
        caller,
        mso_id,
        mso_info: mso_id.and_then(mso_info),
    };
    let mut headers = dict(&[
        ("ap_42", "anonymous"),
        ("ap_11", "Linux i686"),
        ("ap_z", USER_AGENT),
        ("User-Agent", USER_AGENT),
    ]);
    let guid = if resource.contains('<') {
        flow.xml_text(resource, "guid")?
    } else {
        resource.to_owned()
    };
    let service = |path: &str| Url::parse(&format!("{SERVICE_PROVIDER_TEMPLATE}{path}"));
    let form = "application/x-www-form-urlencoded; charset=UTF-8";
    for _ in 0..2 {
        let mut requestor_info = cache_load(requestor_id);
        let mut authn_token = requestor_info
            .get("authn_token")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if let Some(token) = &authn_token
            && flow.is_expired(token, "simpleTokenExpires")?
        {
            authn_token = None;
        }
        let authn_token = match authn_token {
            Some(token) => token,
            None => {
                let Some(mso_id) = flow.mso_id else {
                    return Err(flow.mvpd_required());
                };
                let (username, password) =
                    netrc::login_info(&options.login, mso_id, caller.extractor)?;
                let (Some(username), Some(password)) = (
                    username.filter(|value| !value.is_empty()),
                    password.filter(|value| !value.is_empty()),
                ) else {
                    return Err(flow.mvpd_required());
                };
                let fingerprint = json!({"fingerprint": crate::mhtml::boundary()});
                let (device_info, device_page) = flow.json(
                    &Url::parse("https://sp.auth.adobe.com/indiv/devices")?,
                    "Registering device with Adobe",
                    Some(crate::output::traverse::py_json(&fingerprint, true)),
                    &[("Content-Type", "application/json; charset=UTF-8")],
                )?;
                let device_id = flow.field(&device_info, "deviceId")?;
                // `str(urlh.get_header('pass_sfp'))`.
                let pass_sfp = device_page.headers.get("pass_sfp").map_or_else(
                    || "None".to_owned(),
                    |value| String::from_utf8_lossy(value.as_bytes()).trim().to_owned(),
                );
                headers.set("pass_sfp", &pass_sfp);
                headers.set("Ap_21", &device_id);

                let statement = json!({"software_statement": software_statement});
                let (registration, _) = flow.json(
                    &Url::parse("https://sp.auth.adobe.com/o/client/register")?,
                    "Registering client with Adobe",
                    Some(crate::output::traverse::py_json(&statement, true)),
                    &[("Content-Type", "application/json; charset=UTF-8")],
                )?;
                let client_id = flow.field(&registration, "client_id")?;
                let client_secret = flow.field(&registration, "client_secret")?;
                let token_data = dict(&[
                    ("grant_type", "client_credentials"),
                    ("client_id", &client_id),
                    ("client_secret", &client_secret),
                ]);
                let (token, _) = flow.json(
                    &Url::parse("https://sp.auth.adobe.com/o/client/token")?,
                    "Obtaining access token",
                    Some(token_data.urlencode()),
                    &[("Content-Type", form)],
                )?;
                let access_token = flow.field(&token, "access_token")?;
                let bearer = format!("Bearer {access_token}");
                headers.set("Authorization", &bearer);

                let regcode_data = dict(&[
                    ("requestor", requestor_id),
                    ("deviceId", &device_id),
                    ("format", "json"),
                ]);
                let (regcode, _) = flow.json(
                    &Url::parse(&format!(
                        "https://sp.auth.adobe.com/reggie/v1/{requestor_id}/regcode"
                    ))?,
                    "Obtaining registration code",
                    Some(regcode_data.urlencode()),
                    &[("Content-Type", form), ("Authorization", &bearer)],
                )?;
                let reg_code = flow.field(&regcode, "code")?;

                let query = dict(&[
                    ("noflash", "true"),
                    ("mso_id", mso_id),
                    ("requestor_id", requestor_id),
                    ("no_iframe", "false"),
                    ("domain_name", "adobe.com"),
                    ("redirect_url", url),
                    ("reg_code", &reg_code),
                ]);
                let redirect = flow.get_with_query(
                    &service("authenticate/saml")?,
                    "Downloading Provider Redirect Page",
                    &query,
                )?;
                flow.login(mso_id, redirect, &username, &password)?;

                let session_data = dict(&[
                    ("_method", "GET"),
                    ("requestor_id", requestor_id),
                    ("reg_code", &reg_code),
                ]);
                // `not mso_id` cannot hold here: a 401 is the plain error.
                let session = flow.webpage(
                    &service("session")?,
                    "Retrieving Session",
                    Some(session_data.urlencode()),
                    &headers.pairs(),
                )?;
                if session.text.contains("<pendingLogout") {
                    cache_store(requestor_id, Map::new());
                    continue;
                }
                let token =
                    crate::html::unescape_html(&flow.xml_text(&session.text, "authnToken")?);
                requestor_info.insert("authn_token".into(), json!(token));
                cache_store(requestor_id, requestor_info.clone());
                token
            }
        };

        let mut authz_token = requestor_info
            .get(&guid)
            .and_then(Value::as_str)
            .map(str::to_owned);
        if let Some(token) = &authz_token
            && flow.is_expired(token, "simpleTokenTTL")?
        {
            authz_token = None;
        }
        let authz_token = match authz_token {
            Some(token) => token,
            None => {
                let data = dict(&[
                    ("resource_id", resource),
                    ("requestor_id", requestor_id),
                    ("authentication_token", &authn_token),
                    ("mso_id", &flow.xml_text(&authn_token, "simpleTokenMsoID")?),
                    ("userMeta", "1"),
                ]);
                let authorize = flow.webpage(
                    &service("authorize")?,
                    "Retrieving Authorization Token",
                    Some(data.urlencode()),
                    &headers.pairs(),
                )?;
                if authorize.text.contains("<pendingLogout") {
                    cache_store(requestor_id, Map::new());
                    continue;
                }
                if authorize.text.contains("<error") {
                    return Err(flow.error(&flow.xml_text(&authorize.text, "details")?));
                }
                let token =
                    crate::html::unescape_html(&flow.xml_text(&authorize.text, "authzToken")?);
                requestor_info.insert(guid.clone(), json!(token));
                cache_store(requestor_id, requestor_info.clone());
                token
            }
        };

        headers.set("ap_19", &flow.xml_text(&authn_token, "simpleSamlNameID")?);
        headers.set(
            "ap_23",
            &flow.xml_text(&authn_token, "simpleSamlSessionIndex")?,
        );
        let data = dict(&[
            ("authz_token", &authz_token),
            ("requestor_id", requestor_id),
            (
                "session_guid",
                &flow.xml_text(&authn_token, "simpleTokenAuthenticationGuid")?,
            ),
            ("hashed_guid", "false"),
        ]);
        let short_authorize = flow.webpage(
            &service("shortAuthorize")?,
            "Retrieving Media Token",
            Some(data.urlencode()),
            &headers.pairs(),
        )?;
        if short_authorize.text.contains("<pendingLogout") {
            cache_store(requestor_id, Map::new());
            continue;
        }
        return Ok(short_authorize.text);
    }
    // Upstream returns `None` after two pending logouts.
    Err(Error::UnsupportedHtml5(
        "Adobe Pass sessions that stay pending logout are not supported natively".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mso_table_follows_upstream() {
        let comcast = mso_info("Comcast_SSO").unwrap();
        assert_eq!(comcast.name, "Comcast XFINITY");
        assert_eq!(comcast.username_field, Some("user"));
        assert_eq!(comcast.login_hostname, Some("login.xfinity.com"));
        assert!(supported_mso("Rogers"));
        assert!(!supported_mso("rogers"));
        let text = list_mso_text();
        assert!(text.starts_with("Supported TV Providers:\nmso "));
        assert!(text.ends_with('\n'));
    }

    #[test]
    fn dict_updates_keep_insertion_order() {
        let mut form = dict(&[("a", "1"), ("b", "2")]);
        form.set("a", "3");
        form.set("c", "");
        assert_eq!(form.urlencode(), "a=3&b=2&c=");
    }
}
