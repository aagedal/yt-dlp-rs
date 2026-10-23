//! Upstream `YoutubeIE.fetch_po_token` and `_get_config_po_token`: PO tokens
//! passed with `--extractor-args "youtube:po_token=CLIENT.CONTEXT+TOKEN"`.
//!
//! Upstream ships no PO token provider (`yt_dlp/extractor/youtube/pot`
//! only defines the plugin framework and a memory cache); providers are
//! Python plugins, which this native port cannot load. Without one,
//! `_fetch_po_token` yields nothing whatever `fetch_pot` says, so only
//! configured tokens are used.
use serde_json::{Map, Value};

/// `_PoTokenContext`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Context {
    Gvs,
    Player,
    Subs,
}

impl Context {
    fn value(self) -> &'static str {
        match self {
            Context::Gvs => "gvs",
            Context::Player => "player",
            Context::Subs => "subs",
        }
    }
}

/// The `fetch_po_token_args` of one client's player request.
#[derive(Debug, Clone, Default)]
pub(super) struct Request {
    pub client: &'static str,
    pub video_id: String,
    pub visitor_data: Option<String>,
    /// Only when authenticated.
    pub data_sync_id: Option<String>,
    /// Only for clients requiring the JS player.
    pub player_url: Option<String>,
    pub authenticated: bool,
    /// The `html5_generate_content_po_token` experiment of the client's ytcfg.
    pub bind_to_video_id: bool,
}

/// Whether a ytcfg's `WEB_PLAYER_CONTEXT_CONFIGS` enable
/// `html5_generate_content_po_token` (its last value is `true`).
pub(super) fn binds_to_video_id(ytcfg: &Map<String, Value>) -> bool {
    ytcfg
        .get("WEB_PLAYER_CONTEXT_CONFIGS")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|configs| configs.values())
        .filter_map(|config| config.get("serializedExperimentFlags")?.as_str())
        .any(|flags| {
            url::form_urlencoded::parse(flags.as_bytes())
                .filter(|(key, value)| {
                    key == "html5_generate_content_po_token" && !value.is_empty()
                })
                .last()
                .is_some_and(|(_, value)| value == "true")
        })
}

impl Request {
    /// `fetch_po_token(context=CONTEXT, required=REQUIRED)`.
    pub(super) fn fetch(&self, context: Context, _required: bool) -> Option<String> {
        let client = self.client;
        if self.bind_to_video_id {
            crate::screen::extractor_debug_once(&format!(
                "{}: Detected experiment to bind GVS PO Token to video ID for {client} client",
                self.video_id
            ));
        }
        let gvs_with_player = self.player_url.is_some() && context == Context::Gvs;
        if gvs_with_player
            && self.visitor_data.is_none()
            && !self.authenticated
            && !self.bind_to_video_id
        {
            crate::screen::warning_once(&format!(
                "[youtube] Unable to fetch GVS PO Token for {client} client: Missing required Visitor Data. You may need to pass Visitor Data with --extractor-args \"youtube:visitor_data=XXX\""
            ));
            return None;
        }
        if let Some(token) = config_token(client, context).filter(|token| !token.is_empty()) {
            if gvs_with_player
                && self.data_sync_id.is_none()
                && self.authenticated
                && !self.bind_to_video_id
            {
                crate::screen::warning(&format!(
                    "[youtube] Got a GVS PO Token for {client} client, but missing Data Sync ID for account. Formats may not work.You may need to pass a Data Sync ID with --extractor-args \"youtube:data_sync_id=XXX\""
                ));
            }
            crate::screen::extractor_debug(&format!(
                "{}: Retrieved a {} PO Token for {client} client from config",
                self.video_id,
                context.value()
            ));
            return Some(token);
        }
        if gvs_with_player && self.data_sync_id.is_none() && self.authenticated {
            crate::screen::warning_once(&format!(
                "[youtube] Unable to fetch GVS PO Token for {client} client: Missing required Data Sync ID for account. You may need to pass a Data Sync ID with --extractor-args \"youtube:data_sync_id=XXX\""
            ));
            return None;
        }
        // `_fetch_po_token`: `fetch_pot=never|auto|always` chooses whether
        // providers are asked; none are available.
        None
    }
}

/// `_get_config_po_token`: the first `po_token` for the client and context,
/// cleaned like upstream (`urlsafe_b64encode(urlsafe_b64decode(unquote(token)))`).
fn config_token(client: &str, context: Context) -> Option<String> {
    for token_str in super::args::list_case("po_token") {
        let Some((meta, token)) = token_str.split_once('+') else {
            crate::screen::warning_once(&format!(
                "[youtube] Invalid po_token configuration format. Expected \"CLIENT.CONTEXT+PO_TOKEN\", got \"{token_str}\""
            ));
            continue;
        };
        let (token_client, token_context) = match meta.split_once('.') {
            Some((token_client, token_context)) => (token_client, token_context),
            None => {
                if meta.to_lowercase() == client {
                    crate::screen::extractor_debug_once(&format!(
                        "po_token configuration for {client} client is missing a context; assuming GVS. You can provide a context with the format \"CLIENT.CONTEXT+PO_TOKEN\""
                    ));
                }
                (meta, "gvs")
            }
        };
        if token_client.to_lowercase() != client || token_context.to_lowercase() != context.value()
        {
            continue;
        }
        match clean(token) {
            Some(token) => return Some(token),
            None => crate::screen::warning_once(&format!(
                "[youtube] Invalid po_token configuration for {client} client: {token_context} PO Token should be a base64url-encoded string."
            )),
        }
    }
    None
}

/// `base64.urlsafe_b64encode(base64.urlsafe_b64decode(urllib.parse.unquote(token)))`,
/// `None` where Python raises `binascii.Error`/`ValueError`.
fn clean(token: &str) -> Option<String> {
    let unquoted = percent_decode(token);
    if !unquoted.is_ascii() {
        return None;
    }
    let decoded = b64decode_lenient(&unquoted)?;
    Some(b64encode_urlsafe(&decoded))
}

/// `urllib.parse.unquote` (UTF-8, invalid sequences replaced).
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && bytes.get(index + 1).is_some_and(u8::is_ascii_hexdigit)
            && bytes.get(index + 2).is_some_and(u8::is_ascii_hexdigit)
        {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("00");
            out.push(u8::from_str_radix(hex, 16).unwrap_or(0));
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `base64.urlsafe_b64decode` as the Python 3.15 that captured the
/// fixtures decodes (`a2b_base64(strict_mode=False, padded=False)`):
/// characters outside the alphabet and pads are skipped; only a single
/// leftover character fails. (Earlier Pythons require padding and stop at
/// the first complete pad sequence.)
fn b64decode_lenient(text: &str) -> Option<Vec<u8>> {
    let value = |c: u8| -> Option<u8> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        })
    };
    let mut out = Vec::new();
    let (mut quad, mut left) = (0u8, 0u8);
    for c in text.bytes() {
        let Some(bits) = value(c) else {
            continue;
        };
        match quad {
            0 => {
                left = bits;
                quad = 1;
            }
            1 => {
                out.push((left << 2) | (bits >> 4));
                left = bits & 0x0f;
                quad = 2;
            }
            2 => {
                out.push((left << 4) | (bits >> 2));
                left = bits & 0x03;
                quad = 3;
            }
            _ => {
                out.push((left << 6) | bits);
                quad = 0;
            }
        }
    }
    (quad != 1).then_some(out)
}

/// `base64.urlsafe_b64encode` (padded).
fn b64encode_urlsafe(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let bytes = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(bytes[0]) << 16) | (u32::from(bytes[1]) << 8) | u32::from(bytes[2]);
        for position in 0..4 {
            if position <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * position) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_cleaned_like_python_base64() {
        // Recorded with CPython 3.15:
        // urlsafe_b64encode(urlsafe_b64decode(unquote(s))).
        for (input, output) in [
            ("QUJD", Some("QUJD")),
            ("QUI", Some("QUI=")),
            ("QUI%3D", Some("QUI=")),
            ("QU=JD", Some("QUJD")),
            ("=QUI", Some("QUI=")),
            ("QUI=&x=1", None),
            ("a+b/c-d_", Some("a-b_c-d_")),
            ("ab!cd", Some("abcd")),
            ("abcde", None),
            ("Q==", None),
            ("", Some("")),
            ("\u{e9}", None),
        ] {
            assert_eq!(clean(input).as_deref(), output, "{input}");
        }
        assert_eq!(b64encode_urlsafe(b"\xfb\xff"), "-_8=");
    }

    #[test]
    fn experiment_flags_bind_tokens_to_the_video() {
        let ytcfg = |flags: &str| {
            serde_json::json!({"WEB_PLAYER_CONTEXT_CONFIGS": {"A": {"serializedExperimentFlags": flags}}})
                .as_object()
                .unwrap()
                .clone()
        };
        assert!(binds_to_video_id(&ytcfg(
            "a=1&html5_generate_content_po_token=true"
        )));
        assert!(!binds_to_video_id(&ytcfg(
            "html5_generate_content_po_token=true&html5_generate_content_po_token=false"
        )));
        assert!(!binds_to_video_id(&Map::new()));
    }
}
