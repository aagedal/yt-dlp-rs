//! Upstream `InfoExtractor` account handling (`yt_dlp/extractor/common.py`):
//! `_get_login_info` (`--username`/`--password`, else `--netrc-cmd` or
//! `--netrc`/`--netrc-location` through [`crate::netrc`]), `_get_tfa_info`
//! (`--twofactor`, else a prompt), `_login_hint`/`raise_login_required`, and
//! `initialize`'s lifecycle: `_initialize_pre_login`, `_perform_login` when
//! credentials are found, `_real_initialize`, once per extractor per run
//! (`_ready`; a failed login is retried by the next URL). Extractors without
//! a netrc machine warn that password logins are unsupported when
//! `--username` is given.
use super::Extractor;
use crate::{Error, Result};
use reqwest::blocking::Client;
use std::collections::HashSet;
use std::sync::{Mutex, RwLock};
use url::Url;

/// Upstream's account parameters.
#[derive(Clone, Debug, Default)]
pub struct Credentials {
    /// `username` (`-u`).
    pub username: Option<String>,
    /// `password` (`-p`, or the prompt for `-u` alone).
    pub password: Option<String>,
    /// `twofactor` (`-2`).
    pub twofactor: Option<String>,
    /// `usenetrc` (`-n`/`--netrc`).
    pub usenetrc: bool,
    /// `netrc_location`.
    pub netrc_location: Option<String>,
    /// `netrc_cmd`.
    pub netrc_cmd: Option<String>,
}

static CREDENTIALS: RwLock<Option<Credentials>> = RwLock::new(None);

/// Extractors (by name) initialized this run: upstream's `_ready`.
static READY: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);

/// Extractors (by name) whose lifecycle ran, which support logins.
static SUPPORTS_LOGIN: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);

/// Set the account parameters for extractors.
pub fn set_credentials(credentials: Credentials) {
    if let Ok(mut slot) = CREDENTIALS.write() {
        *slot = Some(credentials);
    }
}

fn credentials() -> Credentials {
    CREDENTIALS
        .read()
        .ok()
        .and_then(|credentials| credentials.clone())
        .unwrap_or_default()
}

const COOKIES_HINT: &str = "See  https://github.com/yt-dlp/yt-dlp/wiki/FAQ#how-do-i-pass-cookies-to-yt-dlp  for how to manually pass cookies";

/// `_login_hint(method)`'s methods.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hint {
    /// `None`: no hint.
    Nothing,
    /// `'any'`: cookies or credentials.
    Any,
    /// `'password'`.
    Password,
    /// `'cookies'`.
    Cookies,
    /// `'session_cookies'`.
    SessionCookies,
}

/// Upstream `_login_hint(method, netrc)`; `None` is the default: `any` for
/// an extractor with a netrc machine, else `cookies`.
pub fn login_hint(machine: Option<&str>, method: Option<Hint>) -> String {
    let password = format!(
        "--username and --password, --netrc-cmd, or --netrc ({}) to provide account credentials",
        machine.unwrap_or("None")
    );
    let method = method.unwrap_or(if machine.is_some() {
        Hint::Any
    } else {
        Hint::Cookies
    });
    match method {
        Hint::Nothing => String::new(),
        Hint::Any => format!("Use --cookies, --cookies-from-browser, {password}. {COOKIES_HINT}"),
        Hint::Password => format!("Use {password}"),
        Hint::Cookies => {
            format!(
                "Use --cookies-from-browser or --cookies for the authentication. {COOKIES_HINT}"
            )
        }
        Hint::SessionCookies => format!(
            "Use --cookies for the authentication (--cookies-from-browser might not work). {COOKIES_HINT}"
        ),
    }
}

/// `raise_login_required(msg, method=METHOD)`'s message (without the
/// `[EXTRACTOR] ID: ` prefix): `MSG. HINT`.
pub fn login_required_message(msg: &str, machine: Option<&str>, method: Option<Hint>) -> String {
    let hint = login_hint(machine, method);
    if hint.is_empty() {
        msg.to_owned()
    } else {
        format!("{msg}. {hint}")
    }
}

/// The default `raise_login_required()` message.
pub const LOGIN_REQUIRED: &str = "This video is only available for registered users";

/// An extractor's login session: its name and the URL's temporary ID, which
/// upstream's `extract` adds to errors raised while logging in.
pub struct Session {
    pub(crate) ie: &'static str,
    pub(crate) id: String,
    pub(crate) machine: Option<&'static str>,
}

/// A login response: the decoded body, final URL, status and headers.
pub(crate) struct Page {
    pub text: String,
    pub url: Url,
    pub status: u16,
    pub headers: reqwest::header::HeaderMap,
}

impl Session {
    /// `ExtractorError(MSG, expected=True)`: `[EXTRACTOR] ID: MSG`.
    pub(crate) fn error(&self, msg: &str) -> Error {
        Error::Reported(match self.id.as_str() {
            "" => format!("[{}] {msg}", self.ie),
            id => format!("[{}] {id}: {msg}", self.ie),
        })
    }

    /// `ExtractorError(MSG)` (not expected): with the bug report request.
    pub(crate) fn bug(&self, msg: &str) -> Error {
        self.error(&format!("{msg}; {}", crate::BUG_REPORTS_MESSAGE))
    }

    /// `self.to_screen(MSG)`.
    pub(crate) fn screen(&self, msg: &str) {
        crate::screen::screen(&format!("[{}] {msg}", self.ie));
    }

    /// `self.report_warning(MSG)`.
    pub(crate) fn warning(&self, msg: &str) {
        crate::screen::warning(&format!("[{}] {msg}", self.ie));
    }

    /// `self.report_login()`.
    pub(crate) fn report_login(&self) {
        self.screen("Logging in");
    }

    /// An upstream login request with `video_id=None`: `[EXTRACTOR] NOTE`,
    /// a GET (or a POST of `data`, form-encoded unless the headers say
    /// otherwise, like urllib), the `expected` error statuses answered,
    /// others failing as `ERRNOTE: HTTP Error ... (caused by ...)`.
    pub(crate) fn request(
        &self,
        client: &Client,
        url: &Url,
        (note, errnote): (Option<&str>, &str),
        headers: &[(&str, &str)],
        data: Option<String>,
        expected: &[u16],
    ) -> Result<Page> {
        self.request_inner(client, url, (note, errnote), headers, data, expected)
            .map_err(|error| match error {
                Error::Http(_) | Error::HttpRetryAfter { .. } => {
                    let mut text = super::request_error_text(self.ie, &self.id, "LOGIN", &error);
                    if self.id.is_empty() {
                        text = text.replacen("] : ", "] ", 1);
                    }
                    Error::Reported(text.replacen(
                        "Unable to download LOGIN:",
                        &format!("{errnote}:"),
                        1,
                    ))
                }
                error => error,
            })
    }

    /// [`Self::request`] with `fatal=False`: an HTTP error is the
    /// `[EXTRACTOR] ERRNOTE: HTTP Error ...` warning and `None`.
    pub(crate) fn request_optional(
        &self,
        client: &Client,
        url: &Url,
        (note, errnote): (Option<&str>, &str),
        headers: &[(&str, &str)],
        data: Option<String>,
        expected: &[u16],
    ) -> Result<Option<Page>> {
        match self.request_inner(client, url, (note, errnote), headers, data, expected) {
            Ok(page) => Ok(Some(page)),
            Err(error @ (Error::Http(_) | Error::HttpRetryAfter { .. })) => {
                self.warning(&format!(
                    "{errnote}: {}",
                    crate::upstream_error_text(&error)
                ));
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    fn request_inner(
        &self,
        client: &Client,
        url: &Url,
        (note, _): (Option<&str>, &str),
        headers: &[(&str, &str)],
        data: Option<String>,
        expected: &[u16],
    ) -> Result<Page> {
        use std::io::Read;
        if let Some(note) = note {
            self.screen(note);
        }
        let mut response = match data {
            Some(body) => {
                let mut headers = headers.to_vec();
                if !headers
                    .iter()
                    .any(|(name, _)| name.eq_ignore_ascii_case("Content-Type"))
                {
                    headers.push(("Content-Type", "application/x-www-form-urlencoded"));
                }
                super::direct::post_expecting(client, url, &headers, body, expected)?
            }
            None => super::direct::request_expecting(client, url, headers, expected)?,
        };
        let status = response.status().as_u16();
        let page_url = response.url().clone();
        let mut body = Vec::new();
        response
            .read_to_end(&mut body)
            .map_err(Error::ExtractionRead)?;
        let body =
            super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
        Ok(Page {
            text: super::decode_webpage(&body, response.headers()),
            url: page_url,
            status,
            headers: response.headers().clone(),
        })
    }

    /// `_get_login_info()`.
    pub(crate) fn login_info(&self) -> Result<Option<(String, String)>> {
        get_login_info(self.ie, self.machine)
    }

    /// `_get_tfa_info(note)`.
    pub(crate) fn tfa_info(&self, note: &str) -> Result<String> {
        get_tfa_info(note)
    }
}

/// Upstream `_hidden_inputs(html)` in the dict's order: hidden and submit
/// inputs (outside comments) by `name` (else `id`) with their `value`; a
/// repeated name keeps its first position with the last value.
pub(crate) fn hidden_inputs(html: &str) -> Vec<(String, String)> {
    static INPUT: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)(<input[^>]+>)").unwrap());
    let html = super::presstv::strip_comments(html);
    let mut inputs: Vec<(String, String)> = Vec::new();
    for found in INPUT.captures_iter(&html) {
        let attributes = crate::html_elements::extract_attributes(&found[1]);
        let text = |key: &str| attributes.get(key).and_then(serde_json::Value::as_str);
        if !matches!(text("type"), Some("hidden" | "submit")) {
            continue;
        }
        let name = text("name")
            .filter(|name| !name.is_empty())
            .or_else(|| text("id").filter(|id| !id.is_empty()));
        if let (Some(name), Some(value)) = (name, text("value")) {
            set_field(&mut inputs, name, value);
        }
    }
    inputs
}

/// `urllib.parse.quote_plus(bytes)` (`urlencode`'s value encoding).
pub(crate) fn quote_plus(bytes: &[u8]) -> String {
    let mut out = String::new();
    for &byte in bytes {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            byte => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// `urlencode_postdata(pairs)`.
pub(crate) fn urlencode(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                quote_plus(key.as_bytes()),
                quote_plus(value.as_bytes())
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// `dict[key] = value` on an ordered form.
pub(crate) fn set_field(form: &mut Vec<(String, String)>, key: &str, value: &str) {
    match form.iter_mut().find(|(name, _)| name == key) {
        Some(entry) => entry.1 = value.to_owned(),
        None => form.push((key.to_owned(), value.to_owned())),
    }
}

/// Upstream `_get_login_info()`: `--username` (with `--password`), else the
/// netrc machine's login. Netrc failures (`OSError`, `NetrcParseError`) are
/// `Failed to parse .netrc: ERR` warnings.
pub(crate) fn get_login_info(ie: &str, machine: Option<&str>) -> Result<Option<(String, String)>> {
    let credentials = credentials();
    if let Some(username) = credentials.username {
        return Ok(Some((username, credentials.password.unwrap_or_default())));
    }
    netrc_login_info(ie, machine)
}

/// The netrc half of `_get_login_info()`, for extractors with their own
/// username options (Adobe Pass's `--ap-username`).
pub(crate) fn netrc_login_info(
    ie: &str,
    machine: Option<&str>,
) -> Result<Option<(String, String)>> {
    match get_netrc_login_info(ie, machine, &credentials()) {
        Ok(info) => Ok(info),
        Err(NetrcFailure::Warning(text)) => {
            crate::screen::warning(&format!("[{ie}] Failed to parse .netrc: {text}"));
            Ok(None)
        }
        Err(NetrcFailure::Fatal(error)) => Err(error),
    }
}

enum NetrcFailure {
    Warning(String),
    Fatal(Error),
}

/// Upstream `_get_netrc_login_info(netrc_machine)`.
fn get_netrc_login_info(
    ie: &str,
    machine: Option<&str>,
    credentials: &Credentials,
) -> std::result::Result<Option<(String, String)>, NetrcFailure> {
    let Some(machine) = machine else {
        return Err(NetrcFailure::Fatal(Error::Reported(format!(
            "[{ie}] Missing netrc_machine and {ie}._NETRC_MACHINE"
        ))));
    };
    const ALLOWED: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.-_";
    if machine.starts_with(['-', '_']) || !machine.chars().all(|ch| ALLOWED.contains(ch)) {
        return Err(NetrcFailure::Fatal(Error::Reported(format!(
            "[{ie}] Invalid netrc machine: {}",
            crate::batch::py_repr(machine)
        ))));
    }
    let netrc = if let Some(command) = credentials
        .netrc_cmd
        .as_deref()
        .filter(|cmd| !cmd.is_empty())
    {
        let command = command.replace("{}", machine);
        crate::screen::screen(&format!("[{ie}] Executing command: {command}"));
        let stdout = run_shell(&command).map_err(NetrcFailure::Warning)?;
        crate::netrc::Netrc::from_content(&stdout)
            .map_err(|error| NetrcFailure::Warning(error.to_string()))?
    } else if credentials.usenetrc {
        let mut file = expanduser(credentials.netrc_location.as_deref().unwrap_or("~"));
        if std::path::Path::new(&file).is_dir() {
            file = path_join(&file, ".netrc");
        }
        crate::netrc::Netrc::from_file(&file).map_err(NetrcFailure::Warning)?
    } else {
        return Ok(None);
    };
    let Some((login, _, password)) = netrc.authenticators(machine) else {
        crate::screen::screen(&format!("[{ie}] No authenticators for {machine}"));
        return Ok(None);
    };
    crate::screen::debug(&format!("[{ie}] Using netrc for {machine} authentication"));
    Ok(Some((login.clone(), password.clone())))
}

/// `Popen.run(cmd, text=True, shell=True, stdout=PIPE)`: the command's
/// stdout (universal newlines), or `Command returned error code N`.
fn run_shell(command: &str) -> std::result::Result<String, String> {
    #[cfg(windows)]
    let mut process = {
        let mut process = std::process::Command::new(
            std::env::var_os("COMSPEC").unwrap_or_else(|| "cmd.exe".into()),
        );
        process.args(["/c", command]);
        process
    };
    #[cfg(not(windows))]
    let mut process = {
        let mut process = std::process::Command::new("/bin/sh");
        process.args(["-c", command]);
        process
    };
    let output = process
        .stderr(std::process::Stdio::inherit())
        .output()
        .map_err(|error| crate::netrc::os_error_text(&error, command))?;
    match output.status.code() {
        Some(0) => {}
        Some(code) => return Err(format!("Command returned error code {code}")),
        // Python reports a signal as the negative signal number.
        None => {
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                let signal = output.status.signal().unwrap_or_default();
                return Err(format!("Command returned error code -{signal}"));
            }
            #[cfg(not(unix))]
            return Err("Command returned error code -1".into());
        }
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .replace("\r\n", "\n")
        .replace('\r', "\n"))
}

/// `os.path.expanduser`: a leading `~` (alone or before a separator) is the
/// home directory (`HOME`, else `USERPROFILE` on Windows); `~user` stays.
fn expanduser(path: &str) -> String {
    let Some(rest) = path.strip_prefix('~') else {
        return path.to_owned();
    };
    if !(rest.is_empty() || rest.starts_with('/') || (cfg!(windows) && rest.starts_with('\\'))) {
        return path.to_owned();
    }
    let home = std::env::var("HOME").ok().or_else(|| {
        cfg!(windows)
            .then(|| std::env::var("USERPROFILE").ok())
            .flatten()
    });
    match home {
        Some(home) if rest.is_empty() => home,
        Some(home) => format!("{}{rest}", home.trim_end_matches('/')),
        None => path.to_owned(),
    }
}

/// `os.path.join(directory, name)`.
fn path_join(directory: &str, name: &str) -> String {
    if directory.is_empty()
        || directory.ends_with('/')
        || (cfg!(windows) && directory.ends_with('\\'))
    {
        format!("{directory}{name}")
    } else {
        format!("{directory}{}{name}", std::path::MAIN_SEPARATOR)
    }
}

/// `self.get_param('twofactor')`.
pub(crate) fn twofactor() -> Option<String> {
    credentials().twofactor
}

/// Upstream `_get_tfa_info(note)`: `--twofactor`, else
/// `getpass('Type NOTE and press [Return]: ')`.
pub(crate) fn get_tfa_info(note: &str) -> Result<String> {
    if let Some(code) = credentials().twofactor {
        return Ok(code);
    }
    crate::getpass::getpass(&format!("Type {note} and press [Return]: ")).map_err(Error::Io)
}

/// Upstream `initialize()`'s login part for an extractor: once per run (until
/// it succeeds), `_initialize_pre_login`, `_perform_login` with the found
/// credentials, and `_real_initialize`. `id` is the URL's ID, which errors
/// raised meanwhile carry.
pub(crate) fn initialize<E: Extractor + ?Sized>(
    extractor: &E,
    client: &Client,
    ie: &'static str,
    id: &str,
) -> Result<()> {
    let machine = extractor.netrc_machine();
    let Some(session) = begin(ie, id, machine) else {
        return Ok(());
    };
    extractor.initialize_pre_login(client, &session)?;
    if machine.is_some() {
        if extractor.performs_login()
            && let Some((username, password)) = session.login_info()?
            && !username.is_empty()
        {
            extractor.perform_login(client, &session, &username, &password)?;
        }
    } else {
        unsupported_login_warning(ie);
    }
    extractor.real_initialize(client, &session)?;
    ready(ie);
    Ok(())
}

/// [`initialize`] for extractors sharing a base outside an [`Extractor`]
/// impl (the YouTube ones): `perform_login` runs with found credentials.
pub(crate) fn initialize_login(
    ie: &'static str,
    id: &str,
    machine: &'static str,
    perform_login: impl FnOnce(&Session, &str, &str) -> Result<()>,
) -> Result<()> {
    let Some(session) = begin(ie, id, Some(machine)) else {
        return Ok(());
    };
    if let Some((username, password)) = session.login_info()?
        && !username.is_empty()
    {
        perform_login(&session, &username, &password)?;
    }
    ready(ie);
    Ok(())
}

/// The session of an extractor not yet initialized (`_ready`).
fn begin(ie: &'static str, id: &str, machine: Option<&'static str>) -> Option<Session> {
    if machine.is_some() {
        SUPPORTS_LOGIN
            .lock()
            .unwrap()
            .get_or_insert_with(HashSet::new)
            .insert(ie);
    }
    if READY
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|ready| ready.contains(ie))
    {
        return None;
    }
    Some(Session {
        ie,
        id: id.to_owned(),
        machine,
    })
}

fn ready(ie: &'static str) {
    READY
        .lock()
        .unwrap()
        .get_or_insert_with(HashSet::new)
        .insert(ie);
}

/// Extractors upstream gives `IE_DESC = False` or `_NETRC_MACHINE = False`,
/// and the YouTube ones (`_NETRC_MACHINE = 'youtube'`), which never warn.
fn never_warns(ie: &str) -> bool {
    matches!(
        ie,
        "generic" | "FacebookRedirectURL" | "href.li" | "UnsupportedURL"
    ) || ie.starts_with("youtube")
}

/// `initialize()` for an extractor without a netrc machine: with
/// `--username`, `Login with password is not supported for this website`
/// (once per extractor, like `_ready`).
fn unsupported_login_warning(ie: &str) {
    static WARNED: Mutex<Vec<String>> = Mutex::new(Vec::new());
    if never_warns(ie) || credentials().username.as_deref().is_none_or(str::is_empty) {
        return;
    }
    let mut warned = WARNED.lock().unwrap();
    if warned.iter().any(|name| name == ie) {
        return;
    }
    warned.push(ie.to_owned());
    drop(warned);
    crate::screen::warning(&format!(
        "[{ie}] Login with password is not supported for this website. {}",
        login_hint(None, Some(Hint::Cookies))
    ));
}

/// Called for every screen line: an extractor's `Extracting URL` line
/// follows its `initialize()`, so an extractor that did not run the login
/// lifecycle (one without login support) warns here first.
pub(crate) fn before_screen_line(text: &str) {
    let Some(rest) = text.strip_prefix('[') else {
        return;
    };
    let Some((ie, after)) = rest.split_once("] ") else {
        return;
    };
    if !after.starts_with("Extracting URL: ") {
        return;
    }
    if SUPPORTS_LOGIN
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|names| names.contains(ie))
    {
        return;
    }
    unsupported_login_warning(ie);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_hints_follow_upstream() {
        assert_eq!(
            login_hint(Some("vimeo"), None),
            "Use --cookies, --cookies-from-browser, --username and --password, --netrc-cmd, or --netrc (vimeo) to provide account credentials. See  https://github.com/yt-dlp/yt-dlp/wiki/FAQ#how-do-i-pass-cookies-to-yt-dlp  for how to manually pass cookies"
        );
        assert_eq!(
            login_hint(None, None),
            "Use --cookies-from-browser or --cookies for the authentication. See  https://github.com/yt-dlp/yt-dlp/wiki/FAQ#how-do-i-pass-cookies-to-yt-dlp  for how to manually pass cookies"
        );
        assert_eq!(
            login_hint(Some("vk"), Some(Hint::Password)),
            "Use --username and --password, --netrc-cmd, or --netrc (vk) to provide account credentials"
        );
        assert_eq!(
            login_required_message(LOGIN_REQUIRED, Some("x"), Some(Hint::Nothing)),
            LOGIN_REQUIRED
        );
        assert!(
            login_required_message("Members only", None, Some(Hint::SessionCookies))
                .starts_with("Members only. Use --cookies for the authentication (--cookies-from-browser might not work). See ")
        );
    }

    #[test]
    fn forms_encode_like_urlencode() {
        assert_eq!(
            urlencode(&[("mail tel", "a+b@c.d"), ("p", "*~_.-/&=ü")]),
            "mail+tel=a%2Bb%40c.d&p=%2A~_.-%2F%26%3D%C3%BC"
        );
    }

    #[test]
    fn netrc_paths_expand_like_python() {
        assert_eq!(
            path_join("dir", ".netrc"),
            format!("dir{}.netrc", std::path::MAIN_SEPARATOR)
        );
        assert_eq!(path_join("dir/", ".netrc"), "dir/.netrc");
        assert_eq!(expanduser("~user/x"), "~user/x");
        assert_eq!(expanduser("a/~"), "a/~");
        if let Ok(home) = std::env::var("HOME") {
            assert_eq!(expanduser("~"), home);
            assert_eq!(
                expanduser("~/n"),
                format!("{}/n", home.trim_end_matches('/'))
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn netrc_commands_report_their_exit_code() {
        assert_eq!(run_shell("printf 'a\\r\\nb'").unwrap(), "a\nb");
        assert_eq!(
            run_shell("exit 3").unwrap_err(),
            "Command returned error code 3"
        );
    }
}
