//! Pure preparation and rewriting of individual HTTPS/HTTP format URLs.
//! Mirrors YoutubeIE's signatureCipher/sp, query n and manifest path n flow.
use super::{Kind, MAX_CHALLENGE, Result, Solutions};
use serde_json::Value;
use url::Url;

const MAX_URL: usize = 256 * 1024;
const MAX_CIPHER: usize = 1024 * 1024;
const MAX_PARAMETER: usize = 1024;

pub(super) struct Prepared {
    url: String,
    n: Option<(usize, String)>,
    signature: Option<(String, String)>,
    manifest_n: Option<(std::ops::Range<usize>, String)>,
}

impl Prepared {
    pub(super) fn new(source: &Value) -> Result<Self> {
        let source = source.as_object().ok_or("format must be an object")?;
        let fields: Vec<_> = ["url", "signatureCipher", "cipher"]
            .into_iter()
            .filter_map(|key| source.get(key).map(|v| (key, v)))
            .collect();
        if fields.len() != 1 {
            return Err("format requires exactly one URL or cipher field".into());
        }
        let (key, value) = fields[0];
        let text = value.as_str().ok_or("URL or cipher must be a string")?;
        let (url, signature) = if key == "url" {
            (bounded(text, MAX_URL, "media URL")?.to_owned(), None)
        } else {
            bounded(text, MAX_CIPHER, "signature cipher")?;
            let fields = form_fields(text)?;
            let mut unique = std::collections::HashSet::new();
            if fields.iter().any(|(key, _)| !unique.insert(key)) {
                return Err("duplicate cipher field".into());
            }
            let get = |key: &str| {
                fields
                    .iter()
                    .find(|(k, _)| k == key)
                    .map(|(_, v)| v.as_str())
            };
            let url = bounded(
                get("url").ok_or("cipher is missing url")?,
                MAX_URL,
                "media URL",
            )?;
            let sig = bounded(
                get("s").ok_or("cipher is missing s")?,
                MAX_CHALLENGE,
                "signature challenge",
            )?;
            let sp = bounded(
                get("sp").unwrap_or("signature"),
                MAX_PARAMETER,
                "signature parameter",
            )?;
            if sp == "n" {
                return Err("signature parameter conflicts with n".into());
            }
            (url.to_owned(), Some((sp.to_owned(), sig.to_owned())))
        };
        validate_url(&url)?;
        if decode(raw_path(&url).1, false)?
            .split('/')
            .any(|s| s == "n")
        {
            return Err("manifest path n challenges require manifest preparation".into());
        }
        let query = raw_query(&url);
        let mut n = None;
        if let Some(query) = query {
            for (index, (key, value)) in form_fields(query)?.into_iter().enumerate() {
                if key == "n" {
                    if n.is_some() {
                        return Err("duplicate n parameter".into());
                    }
                    bounded(&value, MAX_CHALLENGE, "n challenge")?;
                    n = Some((index, value));
                }
                if signature.as_ref().is_some_and(|(sp, _)| sp == &key) {
                    return Err("signature parameter already exists".into());
                }
            }
        }
        Ok(Self {
            url,
            n,
            signature,
            manifest_n: None,
        })
    }

    /// Manifest challenges use the exact raw token, as upstream's path regex
    /// does. Percent escapes must still decode as UTF-8 without path delimiters.
    /// Escaped marker names, dot segments, fragments and query n are rejected
    /// rather than normalizing an ambiguous challenge location.
    pub(super) fn manifest(url: &str) -> Result<Self> {
        bounded(url, MAX_URL, "manifest URL")?;
        validate_url(url)?;
        if url.contains('#') {
            return Err("manifest URL fragments are unsupported".into());
        }
        if let Some(query) = raw_query(url)
            && form_fields(query)?.iter().any(|(key, _)| key == "n")
        {
            return Err("manifest query n challenges are unsupported".into());
        }
        let (path_start, path) = raw_path(url);
        let parts: Vec<_> = path.split('/').collect();
        let mut offset = path_start;
        let mut manifest_n = None;
        let mut token_index = None;
        for (index, part) in parts.iter().enumerate() {
            let decoded = decode(part, false)?;
            if decoded.contains(['/', '\\'])
                || decoded.chars().any(char::is_control)
                || matches!(decoded.as_str(), "." | "..")
            {
                return Err("ambiguous manifest path segment".into());
            }
            // A captured challenge is a value, even when its bytes spell a
            // control marker. Continue validating its escapes and delimiters.
            if token_index == Some(index) {
                offset += part.len() + 1;
                continue;
            }
            if matches!(decoded.as_str(), "n" | "s") && decoded != *part {
                return Err("escaped manifest challenge marker is unsupported".into());
            }
            if *part == "s" {
                return Err("encrypted manifest signatures are unsupported".into());
            }
            if *part == "n" {
                if manifest_n.is_some() {
                    return Err("duplicate manifest path n challenge".into());
                }
                let token = parts.get(index + 1).ok_or("missing manifest n challenge")?;
                bounded(token, MAX_CHALLENGE, "manifest n challenge")?;
                // Match upstream /n/([^/]+)/ exactly, including the final slash.
                if parts.get(index + 2).is_none() {
                    return Err("manifest n challenge requires a trailing slash".into());
                }
                let start = offset + part.len() + 1;
                manifest_n = Some((start..start + token.len(), (*token).to_owned()));
                token_index = Some(index + 1);
            }
            offset += part.len() + 1;
        }
        Ok(Self {
            url: url.to_owned(),
            n: None,
            signature: None,
            manifest_n,
        })
    }

    pub(super) fn challenges(&self) -> Vec<(Kind, &str)> {
        self.signature
            .iter()
            .map(|(_, s)| (Kind::Sig, s.as_str()))
            .chain(self.n.iter().map(|(_, n)| (Kind::N, n.as_str())))
            .chain(self.manifest_n.iter().map(|(_, n)| (Kind::N, n.as_str())))
            .collect()
    }

    pub(super) fn apply(&self, solutions: &Solutions) -> Result<String> {
        self.apply_with(|kind, challenge| solutions.get(kind, challenge))
    }

    fn apply_with<'a>(
        &self,
        mut lookup: impl FnMut(Kind, &str) -> Option<&'a str>,
    ) -> Result<String> {
        let solve =
            |lookup: &mut dyn FnMut(Kind, &str) -> Option<&'a str>, kind, challenge: &str| {
                let solution = lookup(kind, challenge).ok_or("missing URL challenge solution")?;
                bounded(solution, MAX_CHALLENGE, "URL challenge solution")
            };
        if let Some((range, challenge)) = &self.manifest_n {
            let solution = solve(&mut lookup, Kind::N, challenge)?;
            // Encode every non-unreserved byte; dots are rejected to prevent
            // URL parsers treating a solved token as a dot path segment.
            if matches!(solution, "." | "..") {
                return Err("manifest n solution is a dot path segment".into());
            }
            let mut output = self.url.clone();
            output.replace_range(range.clone(), &encode_path_segment(solution));
            bounded(&output, MAX_URL, "rewritten manifest URL")?;
            return Ok(output);
        }
        let (base, fragment) = self
            .url
            .split_once('#')
            .map_or((self.url.as_str(), None), |(a, b)| (a, Some(b)));
        let (path, query) = base
            .split_once('?')
            .map_or((base, None), |(a, b)| (a, Some(b)));
        let mut parts: Vec<String> = query
            .map(|q| q.split('&').map(str::to_owned).collect())
            .unwrap_or_default();
        if let Some((index, challenge)) = &self.n {
            // Retain the original key spelling, including percent escapes.
            let key = parts[*index]
                .split_once('=')
                .map_or(parts[*index].as_str(), |(k, _)| k);
            parts[*index] = format!("{key}={}", encode(solve(&mut lookup, Kind::N, challenge)?));
        }
        if let Some((parameter, challenge)) = &self.signature {
            parts.push(format!(
                "{}={}",
                encode(parameter),
                encode(solve(&mut lookup, Kind::Sig, challenge)?)
            ));
        }
        let mut output = path.to_owned();
        if query.is_some() || !parts.is_empty() {
            output.push('?');
            output.push_str(&parts.join("&"));
        }
        if let Some(fragment) = fragment {
            output.push('#');
            output.push_str(fragment);
        }
        bounded(&output, MAX_URL, "rewritten media URL")?;
        Ok(output)
    }
}

fn bounded<'a>(value: &'a str, max: usize, what: &str) -> Result<&'a str> {
    if value.is_empty() || value.len() > max {
        return Err(format!("{what} is empty or exceeds its size limit"));
    }
    Ok(value)
}

fn validate_url(input: &str) -> Result<()> {
    if input
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
    {
        return Err("media URL contains whitespace, controls, or backslashes".into());
    }
    let (_, authority_and_path) = input
        .split_once("://")
        .ok_or("media URL requires an explicit authority")?;
    let authority = authority_and_path
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("");
    if authority.is_empty() || authority.contains('@') {
        return Err("media URL requires an authority without credentials".into());
    }
    let parsed = Url::parse(input).map_err(|_| "invalid media URL")?;
    if !matches!(parsed.scheme(), "https" | "http")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("media URL requires HTTP(S) without credentials".into());
    }
    Ok(())
}

fn raw_path(url: &str) -> (usize, &str) {
    let authority_start = url.find("://").expect("validated authority") + 3;
    let rest = &url[authority_start..];
    let start = authority_start + rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let end = start + url[start..].find(['?', '#']).unwrap_or(url.len() - start);
    (start, &url[start..end])
}

fn encode_path_segment(input: &str) -> String {
    let mut result = String::new();
    for byte in input.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            result.push(byte as char);
        } else {
            use std::fmt::Write;
            write!(result, "%{byte:02X}").expect("writing to string");
        }
    }
    result
}

fn raw_query(url: &str) -> Option<&str> {
    url.split('#').next()?.split_once('?').map(|(_, q)| q)
}

// Unlike form_urlencoded::parse, reject invalid UTF-8 and malformed escapes;
// replacement characters could otherwise hide collisions in parameter names.
fn form_fields(input: &str) -> Result<Vec<(String, String)>> {
    input
        .split('&')
        .map(|part| {
            let (key, value) = part.split_once('=').unwrap_or((part, ""));
            Ok((decode(key, true)?, decode(value, true)?))
        })
        .collect()
}

fn decode(input: &str, plus: bool) -> Result<String> {
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let byte = match bytes[index] {
            b'%' => {
                let pair = bytes
                    .get(index + 1..index + 3)
                    .ok_or("invalid percent escape")?;
                let hex = |b: u8| (b as char).to_digit(16).map(|n| n as u8);
                let value = hex(pair[0])
                    .zip(hex(pair[1]))
                    .map(|(a, b)| a * 16 + b)
                    .ok_or("invalid percent escape")?;
                index += 2;
                value
            }
            b'+' if plus => b' ',
            value => value,
        };
        decoded.push(byte);
        index += 1;
    }
    String::from_utf8(decoded).map_err(|_| "invalid UTF-8 in URL field".into())
}

fn encode(input: &str) -> String {
    url::form_urlencoded::byte_serialize(input.as_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cipher(url: &str, extra: &str) -> Value {
        json!({"signatureCipher": format!("url={}&s=secret{extra}", encode(url))})
    }

    #[test]
    fn direct_url_without_challenges_is_byte_exact() {
        for url in [
            "https://MEDIA.example:443/v%2f.mp4?x=%2f&&x=+&bare#f%20",
            "http://example.test/v?",
            "https://example.test/v#?",
        ] {
            let prepared = Prepared::new(&json!({"url":url})).unwrap();
            assert!(prepared.challenges().is_empty());
            assert_eq!(prepared.apply_with(|_, _| None).unwrap(), url);
        }
    }

    #[test]
    fn manifest_path_rewrite_is_exact_and_encodes_solutions_as_one_segment() {
        for token in ["abc+def", "%61bc%2Bdef", "n", "s", "%6e", "%73"] {
            let original = format!(
                "https://MEDIA.example:443/api/n/{token}/x%2eY/file/index.m3u8?x=%2f&&x=+&bare"
            );
            let prepared = Prepared::manifest(&original).unwrap();
            assert_eq!(prepared.challenges(), vec![(Kind::N, token)]);
            assert!(prepared.apply_with(|_, _| None).is_err());
            assert_eq!(
                prepared.apply_with(|_, _| Some("a/b +?#%é")).unwrap(),
                "https://MEDIA.example:443/api/n/a%2Fb%20%2B%3F%23%25%C3%A9/x%2eY/file/index.m3u8?x=%2f&&x=+&bare"
            );
            assert!(prepared.apply_with(|_, _| Some("..")).is_err());
        }
        for original in [
            "https://e.test/index.m3u8?x=%2f&&bare",
            "http://e.test/path/index.m3u8",
            "https://e.test/index.m3u8?",
            "https://e.test",
            "https://e.test?x=%2f",
        ] {
            let prepared = Prepared::manifest(original).unwrap();
            assert!(prepared.challenges().is_empty());
            assert_eq!(prepared.apply_with(|_, _| None).unwrap(), original);
        }
        assert_eq!(raw_path("https://e.test"), (14, ""));
        assert_eq!(raw_path("https://e.test?x=1"), (14, ""));
    }

    #[test]
    fn malformed_or_ambiguous_manifest_challenges_are_rejected() {
        for original in [
            "https://e.test/n/abc/n/def/index.m3u8",
            "https://e.test/n//index.m3u8",
            "https://e.test/n",
            "https://e.test/n/abc",
            "https://e.test/%6e/abc/index.m3u8",
            "https://e.test/n/a%2Fb/index.m3u8",
            "https://e.test/n/a%5Cb/index.m3u8",
            "https://e.test/n/%00/index.m3u8",
            "https://e.test/n/%ff/index.m3u8",
            "https://e.test/n/%/index.m3u8",
            "https://e.test/a/../n/abc/index.m3u8",
            "https://e.test/n/abc/index.m3u8#fragment",
            "https://e.test/n/abc/index.m3u8#",
            "https://e.test/index.m3u8?%6e=abc",
            "https://e.test/index.m3u8?n=",
            "https://e.test/s/encrypted/index.m3u8",
            "https://e.test/%73/encrypted/index.m3u8",
            "ftp://e.test/index.m3u8",
            "https://user@e.test/index.m3u8",
        ] {
            assert!(Prepared::manifest(original).is_err(), "{original}");
        }
        assert!(
            Prepared::manifest(&format!(
                "https://e.test/n/{}/index.m3u8",
                "x".repeat(MAX_CHALLENGE + 1)
            ))
            .is_err()
        );
    }

    #[test]
    fn rewrites_both_challenges_preserving_unrelated_query_and_fragment() {
        let prepared = Prepared::new(&cipher(
            "https://example.test/v?x=%2f&&%6e=a%2Bb+c&x=+&bare#frag?x",
            "&sp=sig%26x",
        ))
        .unwrap();
        assert_eq!(
            prepared.challenges(),
            vec![(Kind::Sig, "secret"), (Kind::N, "a+b c")]
        );
        let rewritten = prepared
            .apply_with(|kind, _| {
                Some(match kind {
                    Kind::N => "n +&=é",
                    Kind::Sig => "sig#?&",
                })
            })
            .unwrap();
        assert_eq!(
            rewritten,
            "https://example.test/v?x=%2f&&%6e=n+%2B%26%3D%C3%A9&x=+&bare&sig%26x=sig%23%3F%26#frag?x"
        );
    }

    #[test]
    fn default_signature_and_legacy_cipher() {
        let mut source = cipher("https://example.test/v#fragment", "");
        let text = source
            .as_object_mut()
            .unwrap()
            .shift_remove("signatureCipher")
            .unwrap();
        let prepared = Prepared::new(&json!({"cipher":text})).unwrap();
        assert_eq!(
            prepared.apply_with(|_, _| Some("done")).unwrap(),
            "https://example.test/v?signature=done#fragment"
        );
    }

    #[test]
    fn validated_solutions_keep_kinds_separate_for_the_same_challenge() {
        let prepared = Prepared::new(&cipher("https://e.test/v?n=secret", "")).unwrap();
        let batch = super::super::Batch::new("player", &prepared.challenges()).unwrap();
        let solved = batch.validate(br#"{"type":"result","responses":[{"type":"result","data":{"secret":"signature-value"}},{"type":"result","data":{"secret":"n-value"}}]}"#).unwrap();
        assert_eq!(
            prepared.apply(&solved).unwrap(),
            "https://e.test/v?n=n-value&signature=signature-value"
        );
    }

    #[test]
    fn rejects_ambiguous_or_malformed_inputs() {
        for source in [
            json!(null),
            json!({}),
            json!({"url":null}),
            json!({"url":7}),
            json!({"url":""}),
            json!({"url":"https://e.test/v", "cipher":"x"}),
            json!({"signatureCipher":"x", "cipher":"x"}),
            json!({"signatureCipher":"url=https%3A%2F%2Fe.test%2Fv"}),
            json!({"signatureCipher":"url=https%3A%2F%2Fe.test%2Fv&s="}),
            json!({"signatureCipher":"url=https%3A%2F%2Fe.test%2Fv&s=a&%73=b"}),
            json!({"signatureCipher":"url=https%3A%2F%2Fe.test%2Fv&s=a&sp="}),
            cipher("https://e.test/v", "&sp=n"),
            cipher("https://e.test/v?signature=old", ""),
            cipher("https://e.test/v?%73ig=old", "&sp=sig"),
        ] {
            assert!(Prepared::new(&source).is_err(), "{source}");
        }
        for url in [
            "ftp://e.test/v",
            "https://a:b@e.test/v",
            "https://a@e.test/v",
            "https://@e.test/v",
            "https:e.test/v",
            "https:///",
            "https://e.test/a/n/value",
            "https://e.test/a/%6e/value",
            "https://e.test/v?n=a&%6e=a",
            "https://e.test/v?n=",
            "https://e.test/v?n",
            "https://e.test/v?n=%ff",
            "https://e.test/v?n=%",
            "https://e.test/v?n=%g0",
            "https://e.test/v?n=x\n",
            "https://e.test\\v",
        ] {
            assert!(Prepared::new(&json!({"url":url})).is_err(), "{url}");
        }
    }

    #[test]
    fn missing_and_oversized_solutions_are_errors() {
        let prepared = Prepared::new(&cipher("https://e.test/v?n=abc", "")).unwrap();
        assert!(prepared.apply_with(|_, _| None).is_err());
        assert!(
            prepared
                .apply_with(|kind, _| (kind == Kind::Sig).then_some("sig"))
                .is_err()
        );
        assert!(prepared.apply_with(|_, _| Some("")).is_err());
        let long = "x".repeat(MAX_CHALLENGE + 1);
        assert!(prepared.apply_with(|_, _| Some(&long)).is_err());
        assert!(Prepared::new(&json!({"url":format!("https://e.test/v?n={long}")})).is_err());
        assert!(
            Prepared::new(&cipher(
                "https://e.test/v",
                &format!("&sp={}", "x".repeat(MAX_PARAMETER + 1))
            ))
            .is_err()
        );
        assert!(
            Prepared::new(&json!({"url":format!("https://e.test/{}", "x".repeat(MAX_URL))}))
                .is_err()
        );
        assert!(Prepared::new(&json!({"cipher":"x".repeat(MAX_CIPHER + 1)})).is_err());
    }
}
