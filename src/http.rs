//! Validated global HTTP defaults, separate from extractor-owned request headers.
mod proxy;
mod socks4_relay;
use crate::{CookieJar, Error, Result};
pub(crate) use proxy::ProxyConfig;
use reqwest::blocking::ClientBuilder;
use reqwest::header::{COOKIE, HeaderMap, HeaderName, HeaderValue, REFERER, USER_AGENT};
use std::sync::Arc;
use std::{net::IpAddr, time::Duration};

/// Global request defaults. Extractor/transport headers take precedence.
/// Raw Cookie headers are unsupported; use the scoped cookie jar instead.
#[derive(Clone, Default)]
pub struct HttpOptions {
    /// None inherits system routing; an empty string forces direct connections.
    pub proxy: Option<String>,
    /// Shared cookie state; None creates a fresh in-memory jar.
    pub cookies: Option<Arc<CookieJar>>,
    pub headers: Vec<(String, String)>,
    pub user_agent: Option<String>,
    pub referer: Option<String>,
    /// `--socket-timeout` seconds for connect and each read/write; None or 0
    /// keeps the native defaults (15 s connect, 300 s operations).
    pub socket_timeout: Option<f64>,
    /// `--no-check-certificates`: accept invalid HTTPS certificates.
    pub no_check_certificates: bool,
    /// `--source-address`/`-4`/`-6`: client-side IP address to bind to.
    pub source_address: Option<String>,
    /// `--client-certificate` (PEM, optionally including the key),
    /// `--client-certificate-key` and `--client-certificate-password`.
    pub client_certificate: Option<std::path::PathBuf>,
    pub client_certificate_key: Option<std::path::PathBuf>,
    pub client_certificate_password: Option<String>,
    /// `--print-traffic`: report the HTTP bytes sent and response headers.
    pub print_traffic: bool,
    /// `--geo-verification-proxy`: the proxy of the requests upstream sends
    /// with `geo_verification_headers()` (`Ytdl-request-proxy`, which
    /// overrides every other proxy setting).
    pub geo_verification_proxy: Option<String>,
}

/// Connection policy shared by the normal and bounded player clients.
#[derive(Clone, Default)]
pub(crate) struct TransportConfig {
    pub(crate) proxy: ProxyConfig,
    pub(crate) timeout: Option<Duration>,
    pub(crate) insecure: bool,
    pub(crate) source_address: Option<IpAddr>,
    identity: Option<reqwest::Identity>,
}

impl From<ProxyConfig> for TransportConfig {
    fn from(proxy: ProxyConfig) -> Self {
        Self {
            proxy,
            ..Default::default()
        }
    }
}

/// Upstream's `--socket-timeout` default (`socket_timeout=20`), applied to
/// the connect and to each read like its socket timeout.
pub(crate) const DEFAULT_SOCKET_TIMEOUT: Duration = Duration::from_secs(20);

impl TransportConfig {
    /// The socket timeout: `--socket-timeout`, else upstream's default.
    pub(crate) fn socket_timeout(&self) -> Duration {
        self.timeout.unwrap_or(DEFAULT_SOCKET_TIMEOUT)
    }

    pub(crate) fn apply(&self, builder: ClientBuilder) -> Result<ClientBuilder> {
        let timeout = self.socket_timeout();
        // The blocking client applies `timeout` to sending a request and to
        // each read of a streamed body (whole-body helpers such as `text()`
        // are bounded as a whole), so it acts as upstream's socket timeout.
        let mut builder = self
            .proxy
            .apply(builder)?
            .connect_timeout(timeout)
            .timeout(timeout);
        if self.insecure {
            builder = builder.danger_accept_invalid_certs(true);
        }
        if let Some(address) = self.source_address {
            builder = builder.local_address(address);
        }
        if let Some(identity) = &self.identity {
            builder = builder.identity(identity.clone());
        }
        Ok(builder)
    }
}

/// The PEM with its encrypted private key decrypted, as Python's
/// `load_cert_chain(certfile, keyfile, password)` reads it: PKCS#8
/// `ENCRYPTED PRIVATE KEY` (PBES2) or OpenSSL's legacy `Proc-Type:
/// 4,ENCRYPTED` with AES-CBC. Without `password`, OpenSSL's `Enter PEM pass
/// phrase:` prompt asks for it.
fn decrypt_keys(pem: &[u8], password: Option<&str>) -> Result<Vec<u8>> {
    static BLOCK: std::sync::LazyLock<regex::bytes::Regex> = std::sync::LazyLock::new(|| {
        regex::bytes::Regex::new(
            r"(?s)-----BEGIN ([A-Z0-9 ]+)-----\r?\n(.*?)-----END ([A-Z0-9 ]+)-----",
        )
        .unwrap()
    });
    let mut password = password.map(str::to_owned);
    let mut passphrase = || -> Result<String> {
        if password.is_none() {
            password = Some(
                crate::getpass::getpass("Enter PEM pass phrase:")
                    .map_err(|_| error("no password for the encrypted client certificate key"))?,
            );
        }
        Ok(password.clone().unwrap_or_default())
    };
    let wrong = || error("unable to decrypt the client certificate key (wrong password?)");
    let mut out = Vec::with_capacity(pem.len());
    let mut last = 0;
    for found in BLOCK.captures_iter(pem) {
        let whole = found.get(0).expect("match");
        let label = String::from_utf8_lossy(&found[1]).into_owned();
        let body = &found[2];
        let headers: Vec<&[u8]> = body
            .split(|&byte| byte == b'\n')
            .take_while(|line| line.contains(&b':'))
            .collect();
        let encrypted_legacy = headers
            .iter()
            .any(|line| line.starts_with(b"Proc-Type: 4,ENCRYPTED"));
        let replacement = if label == "ENCRYPTED PRIVATE KEY" {
            let der = base64_body(body).ok_or_else(wrong)?;
            let info = pkcs8::EncryptedPrivateKeyInfo::try_from(der.as_slice())
                .map_err(|_| error("the client certificate key is not valid PKCS#8"))?;
            let document = info.decrypt(passphrase()?).map_err(|_| wrong())?;
            Some(pem_block("PRIVATE KEY", document.as_bytes()))
        } else if label.ends_with("PRIVATE KEY") && encrypted_legacy {
            let info = headers
                .iter()
                .find_map(|line| line.strip_prefix(b"DEK-Info: "))
                .map(|value| String::from_utf8_lossy(value).trim().to_owned())
                .ok_or_else(wrong)?;
            let (cipher, iv) = info.split_once(',').ok_or_else(wrong)?;
            let key_length = match cipher {
                "AES-128-CBC" => 16,
                "AES-192-CBC" => 24,
                "AES-256-CBC" => 32,
                other => {
                    return Err(error(&format!(
                        "client certificate keys encrypted with {other} are not supported natively"
                    )));
                }
            };
            let iv = (0..iv.len())
                .step_by(2)
                .map(|index| u8::from_str_radix(iv.get(index..index + 2)?, 16).ok())
                .collect::<Option<Vec<u8>>>()
                .filter(|iv| iv.len() == 16)
                .ok_or_else(wrong)?;
            let encoded: Vec<u8> = body
                .split(|&byte| byte == b'\n')
                .skip(headers.len())
                .flatten()
                .copied()
                .collect();
            let data = base64_body(&encoded).ok_or_else(wrong)?;
            let key = bytes_to_key(passphrase()?.as_bytes(), &iv[..8], key_length);
            let der = decrypt_cbc(&key, &iv, data).ok_or_else(wrong)?;
            Some(pem_block(&label, &der))
        } else {
            None
        };
        if let Some(replacement) = replacement {
            out.extend_from_slice(&pem[last..whole.start()]);
            out.extend_from_slice(replacement.as_bytes());
            last = whole.end();
        }
    }
    out.extend_from_slice(&pem[last..]);
    Ok(out)
}

fn base64_body(body: &[u8]) -> Option<Vec<u8>> {
    use base64::Engine as _;
    let text: Vec<u8> = body
        .iter()
        .copied()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    base64::engine::general_purpose::STANDARD.decode(text).ok()
}

fn pem_block(label: &str, der: &[u8]) -> String {
    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::STANDARD.encode(der);
    let mut block = format!("-----BEGIN {label}-----\n");
    for line in encoded.as_bytes().chunks(64) {
        block.push_str(&String::from_utf8_lossy(line));
        block.push('\n');
    }
    block.push_str(&format!("-----END {label}-----"));
    block
}

/// OpenSSL `EVP_BytesToKey(cipher, md5, salt, password, count=1)`.
fn bytes_to_key(password: &[u8], salt: &[u8], length: usize) -> Vec<u8> {
    let mut key = Vec::with_capacity(length + 16);
    let mut previous: Vec<u8> = Vec::new();
    while key.len() < length {
        let mut input = previous.clone();
        input.extend_from_slice(password);
        input.extend_from_slice(salt);
        let hex = crate::md5::hex(&input);
        previous = (0..32)
            .step_by(2)
            .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).unwrap_or(0))
            .collect();
        key.extend_from_slice(&previous);
    }
    key.truncate(length);
    key
}

fn decrypt_cbc(key: &[u8], iv: &[u8], mut data: Vec<u8>) -> Option<Vec<u8>> {
    use aes::cipher::{BlockModeDecrypt, KeyIvInit, block_padding::Pkcs7};
    let length = match key.len() {
        16 => cbc::Decryptor::<aes::Aes128>::new_from_slices(key, iv)
            .ok()?
            .decrypt_padded::<Pkcs7>(&mut data)
            .ok()?
            .len(),
        24 => cbc::Decryptor::<aes::Aes192>::new_from_slices(key, iv)
            .ok()?
            .decrypt_padded::<Pkcs7>(&mut data)
            .ok()?
            .len(),
        32 => cbc::Decryptor::<aes::Aes256>::new_from_slices(key, iv)
            .ok()?
            .decrypt_padded::<Pkcs7>(&mut data)
            .ok()?
            .len(),
        _ => return None,
    };
    data.truncate(length);
    Some(data)
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
            headers: Vec::with_capacity(headers.len()),
            user_agent,
            referer,
            ..Default::default()
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

    /// Validate request configuration without network requests or DNS resolution.
    pub fn validate(&self) -> Result<()> {
        self.header_map()?;
        self.transport_config()?;
        self.geo_transport_config()?;
        Ok(())
    }

    /// Upstream `client_certificate*`: the certificate file (which may hold
    /// the key) plus an optional separate key file, as a TLS client identity.
    fn client_identity(&self) -> Result<Option<reqwest::Identity>> {
        let Some(certificate) = &self.client_certificate else {
            if self.client_certificate_key.is_some() || self.client_certificate_password.is_some() {
                return Err(error(
                    "--client-certificate-key and --client-certificate-password need --client-certificate",
                ));
            }
            return Ok(None);
        };
        let read = |path: &std::path::Path| {
            std::fs::read(path)
                .map_err(|_| error("unable to read the client certificate or key file"))
        };
        let mut pem = read(certificate)?;
        if let Some(key) = &self.client_certificate_key {
            pem.push(b'\n');
            pem.extend(read(key)?);
        }
        let pem = decrypt_keys(&pem, self.client_certificate_password.as_deref())?;
        reqwest::Identity::from_pem(&pem).map(Some).map_err(|_| {
            error("the client certificate must be PEM with a certificate and its private key")
        })
    }

    pub(crate) fn transport_config(&self) -> Result<TransportConfig> {
        let timeout = match self.socket_timeout {
            // Upstream treats 0 as unset (`socket_timeout or 20`).
            None | Some(0.0) => None,
            Some(seconds) if seconds.is_finite() && seconds > 0.0 && seconds <= 86_400.0 => {
                Some(Duration::from_secs_f64(seconds))
            }
            Some(_) => return Err(error("socket timeout must be a positive number of seconds")),
        };
        let source_address = self
            .source_address
            .as_deref()
            .map(|address| {
                address
                    .parse::<IpAddr>()
                    .map_err(|_| error("source address must be an IP address"))
            })
            .transpose()?;
        Ok(TransportConfig {
            proxy: ProxyConfig::parse(self.proxy.as_deref())?,
            timeout,
            insecure: self.no_check_certificates,
            source_address,
            identity: self.client_identity()?,
        })
    }

    /// [`Self::transport_config`] with `--geo-verification-proxy` as the
    /// proxy; `None` without one.
    pub(crate) fn geo_transport_config(&self) -> Result<Option<TransportConfig>> {
        let Some(proxy) = self
            .geo_verification_proxy
            .as_deref()
            .filter(|proxy| !proxy.is_empty())
        else {
            return Ok(None);
        };
        Ok(Some(TransportConfig {
            proxy: ProxyConfig::parse(Some(proxy))?,
            ..self.transport_config()?
        }))
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
            if matches!(name.as_str(), "ytdl-request-proxy" | "ytdl-socks-proxy") {
                return Err(error(
                    "legacy per-request proxy headers are unsupported; use --proxy",
                ));
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
    fn socket_timeout_defaults_to_upstreams_twenty_seconds() {
        let options = HttpOptions::default();
        assert_eq!(
            options.transport_config().unwrap().socket_timeout(),
            Duration::from_secs(20)
        );
        let options = HttpOptions {
            socket_timeout: Some(0.0),
            ..Default::default()
        };
        assert_eq!(
            options.transport_config().unwrap().socket_timeout(),
            Duration::from_secs(20)
        );
        let options = HttpOptions {
            socket_timeout: Some(2.5),
            ..Default::default()
        };
        assert_eq!(
            options.transport_config().unwrap().socket_timeout(),
            Duration::from_millis(2500)
        );
    }

    #[test]
    fn invalid_controls_cookies_and_resource_bounds_fail_without_values_in_errors() {
        for (name, value) in [
            ("bad name", "secret"),
            ("Cookie", "secret"),
            ("Ytdl-Request-Proxy", "secret"),
            ("Ytdl-Socks-Proxy", "secret"),
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
