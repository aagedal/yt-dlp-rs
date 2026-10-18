//! Validated global HTTP defaults, separate from extractor-owned request headers.
use crate::{CookieJar, Error, Result};
use reqwest::header::{COOKIE, HeaderMap, HeaderName, HeaderValue, REFERER, USER_AGENT};
use std::sync::Arc;

/// Global request defaults. Extractor/transport headers take precedence.
/// Raw Cookie headers are unsupported; use the scoped cookie jar instead.
#[derive(Clone, Default)]
pub struct HttpOptions {
    /// Shared cookie state; None creates a fresh in-memory jar.
    pub cookies: Option<Arc<CookieJar>>,
    pub headers: Vec<(String, String)>,
    pub user_agent: Option<String>,
    pub referer: Option<String>,
}

fn error(message: &str) -> Error {
    // Never include supplied values: headers can contain credentials.
    Error::HttpConfiguration(message.into())
}

impl HttpOptions {
    /// Parse repeated FIELD:VALUE options; split only the first colon.
    /// CLI names use the pinned option callback's ASCII word/hyphen subset.
    pub fn from_cli(
        user_agent: Option<String>,
        referer: Option<String>,
        headers: &[String],
    ) -> Result<Self> {
        if headers.len() > 256 {
            return Err(error("more than 256 custom headers"));
        }
        let mut options = Self {
            cookies: None,
            headers: Vec::with_capacity(headers.len()),
            user_agent,
            referer,
        };
        for raw in headers {
            let (name, value) = raw
                .split_once(':')
                .ok_or_else(|| error("expected FIELD:VALUE in --add-headers"))?;
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
            {
                return Err(error("invalid field name in --add-headers"));
            }
            options.headers.push((name.into(), value.into()));
        }
        // Validate before any client/network/filesystem operation.
        options.header_map()?;
        Ok(options)
    }

    pub(crate) fn header_map(&self) -> Result<HeaderMap> {
        if self.headers.len() > 256 {
            return Err(error("more than 256 custom headers"));
        }
        let mut headers = HeaderMap::new();
        let mut total = 0usize;
        let mut insert = |name: HeaderName, raw: &str| -> Result<()> {
            total = total
                .saturating_add(name.as_str().len())
                .saturating_add(raw.len());
            if raw.len() > 16 * 1024 || total > 64 * 1024 {
                return Err(error("header values exceed 16 KiB each or 64 KiB total"));
            }
            // Check before trimming: outer CR/LF must not hide injection attempts.
            if raw.chars().any(|c| c.is_control() && c != '\t') {
                return Err(error("header value contains unsupported controls"));
            }
            if name == COOKIE {
                return Err(error(
                    "raw Cookie headers are unsupported; use a scoped cookie jar",
                ));
            }
            let mut value = HeaderValue::from_str(raw.trim())
                .map_err(|_| error("invalid HTTP header value"))?;
            if matches!(name.as_str(), "authorization" | "proxy-authorization") {
                value.set_sensitive(true);
            }
            headers.insert(name, value);
            Ok(())
        };
        for (name, value) in &self.headers {
            if name.len() > 256 {
                return Err(error("HTTP header name exceeds 256 bytes"));
            }
            let name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| error("invalid HTTP header name"))?;
            insert(name, value)?;
        }
        // Pinned CLI normalization makes convenience values win independently
        // of argument order; preserve that result for the library API too.
        if let Some(value) = &self.user_agent {
            insert(USER_AGENT, value)?;
        }
        if let Some(value) = &self.referer {
            insert(REFERER, value)?;
        }
        Ok(headers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_controls_cookies_and_resource_bounds_fail_without_values_in_errors() {
        for (name, value) in [
            ("bad name", "secret"),
            ("Cookie", "secret"),
            ("x-key", "secret\r\nInjected: yes"),
            ("x-key", "secret\0"),
        ] {
            let options = HttpOptions {
                headers: vec![(name.into(), value.into())],
                ..Default::default()
            };
            let message = options.header_map().unwrap_err().to_string();
            assert!(!message.contains("secret"));
        }
        for options in [
            HttpOptions {
                headers: vec![("x-key".into(), "a".repeat(16 * 1024 + 1))],
                ..Default::default()
            },
            HttpOptions {
                headers: vec![("x-key".into(), "a".repeat(16 * 1024)); 5],
                ..Default::default()
            },
            HttpOptions {
                headers: vec![("x-key".into(), "a".into()); 257],
                ..Default::default()
            },
        ] {
            assert!(options.header_map().is_err());
        }
    }

    #[test]
    fn library_accepts_http_token_names_and_cli_rejects_unsupported_callback_names() {
        let options = HttpOptions {
            headers: vec![("x!key".into(), "\tvalue\twith\ttabs\t".into())],
            ..Default::default()
        };
        assert_eq!(options.header_map().unwrap()["x!key"], "value\twith\ttabs");
        for raw in ["x!key:v", "bad name:v", "missing", ":value"] {
            assert!(HttpOptions::from_cli(None, None, &[raw.into()]).is_err());
        }
    }
}
