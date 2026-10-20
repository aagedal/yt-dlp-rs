//! Pure caption metadata planning for the pinned anonymous WEB client.
use crate::{Error, Result, SubtitleFormat, Subtitles};
use serde_json::Value;
use std::collections::HashMap;
use url::Url;

const FORMATS: [&str; 7] = ["json3", "srv1", "srv2", "srv3", "ttml", "srt", "vtt"];
const MAX_ITEMS: usize = 1000;
const MAX_FORMATS: usize = 100_000;
const MAX_GENERATED_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Default)]
pub(super) struct Captions {
    pub subtitles: Subtitles,
    pub automatic: Subtitles,
    pub audio_language: Option<String>,
    pub warnings: Vec<String>,
}
fn error(message: impl Into<String>) -> Error {
    Error::UnsupportedYoutube(format!("captions: {}", message.into()))
}

fn array<'a>(object: &'a Value, field: &str) -> Result<&'a [Value]> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(&[]),
        Some(Value::Array(values)) if values.len() <= MAX_ITEMS => Ok(values),
        Some(Value::Array(_)) => Err(error(format!("{field} exceeds 1000 entries"))),
        Some(_) => Err(error(format!("{field} must be an array"))),
    }
}
fn string<'a>(object: &'a Value, field: &str) -> Result<Option<&'a str>> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        Some(_) => Err(error(format!("{field} must be a string"))),
    }
}
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
    }
}
fn name(value: Option<&Value>) -> Result<Option<String>> {
    let Some(value) = value else { return Ok(None) };
    let text = value
        .get("simpleText")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .or_else(|| {
            value
                .get("runs")
                .or_else(|| value.is_array().then_some(value))
                .and_then(Value::as_array)
                .and_then(|runs| runs.first())
                .and_then(|run| run.get("text"))
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
        });
    if text.is_some_and(|text| text.len() > 4096) {
        return Err(error("caption name exceeds 4 KiB"));
    }
    Ok(text.map(str::to_owned))
}
fn language(track: &Value) -> Result<Option<String>> {
    let vss = string(track, "vssId")?.unwrap_or("");
    let vss = vss.strip_prefix('.').unwrap_or(vss).replace('.', "-");
    if !vss.is_empty() {
        return Ok(Some(vss));
    }
    Ok(string(track, "languageCode")?
        .filter(|value| !value.is_empty())
        .map(str::to_owned))
}

// Python parse_qs drops blank values, groups repeated keys in first-key order,
// and update_url_query overwrites values without moving existing keys.
#[derive(Clone)]
struct Query(Vec<(String, Vec<String>)>);
impl Query {
    fn parse(base: &str) -> Self {
        let query = base
            .split('#')
            .next()
            .unwrap_or("")
            .split_once('?')
            .map_or("", |(_, query)| query);
        let mut entries: Vec<(String, Vec<String>)> = Vec::new();
        let mut positions: HashMap<String, usize> = HashMap::new();
        for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
            if value.is_empty() {
                continue;
            }
            if let Some(&index) = positions.get(key.as_ref()) {
                entries[index].1.push(value.into_owned());
            } else {
                let key = key.into_owned();
                positions.insert(key.clone(), entries.len());
                entries.push((key, vec![value.into_owned()]));
            }
        }
        Self(entries)
    }
    fn values(&self, key: &str) -> Option<&[String]> {
        self.0
            .iter()
            .find(|(current, _)| current == key)
            .map(|(_, values)| values.as_slice())
    }
    fn set(&mut self, key: &str, value: &str) {
        if let Some((_, values)) = self.0.iter_mut().find(|(current, _)| current == key) {
            *values = vec![value.into()];
        } else {
            self.0.push((key.into(), vec![value.into()]));
        }
    }
    fn encode(&self) -> String {
        let mut query = String::new();
        for (key, values) in &self.0 {
            if key == "xosf" {
                continue;
            }
            for value in values {
                if !query.is_empty() {
                    query.push('&');
                }
                quote_plus(&mut query, key);
                query.push('=');
                quote_plus(&mut query, value);
            }
        }
        query
    }
}
fn quote_plus(output: &mut String, value: &str) {
    const HEX: &[u8] = b"0123456789ABCDEF";
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.' | b'~' => {
                output.push(byte as char)
            }
            b' ' => output.push('+'),
            _ => {
                output.push('%');
                output.push(HEX[(byte >> 4) as usize] as char);
                output.push(HEX[(byte & 15) as usize] as char);
            }
        }
    }
}
fn base_url(base: &str) -> Result<Url> {
    if base.chars().any(char::is_control) {
        return Err(error("caption URL contains controls"));
    }
    let fixed = if base.starts_with("//") {
        Some(format!("http:{base}"))
    } else {
        base.strip_prefix("httpss://")
            .map(|tail| format!("https://{tail}"))
    };
    let base = fixed.as_deref().unwrap_or(base);
    let canonical = Url::parse("https://www.youtube.com").unwrap();
    let url = canonical
        .join(base)
        .map_err(|_| error("invalid caption URL"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(error("caption URL must use HTTP or HTTPS"));
    }
    Ok(url)
}
#[derive(Default)]
struct Budget {
    formats: usize,
    bytes: usize,
}
fn process(
    container: &mut Subtitles,
    base: &str,
    query: &Query,
    tag: &str,
    name: Option<&str>,
    tlang: Option<&str>,
    budget: &mut Budget,
) -> Result<()> {
    if name.is_some_and(|name| name.len() > 4096) {
        return Err(error("generated caption name exceeds 4 KiB"));
    }
    let mut url = base_url(base)?;
    let mut query = query.clone();
    if let Some(tlang) = tlang {
        query.set("tlang", tlang);
    }
    let mut formats = Vec::with_capacity(FORMATS.len());
    for ext in FORMATS {
        query.set("fmt", ext);
        url.set_query(Some(&query.encode()));
        budget.formats += 1;
        budget.bytes = budget
            .bytes
            .saturating_add(url.as_str().len())
            .saturating_add(name.map_or(0, str::len));
        if budget.formats > MAX_FORMATS || budget.bytes > MAX_GENERATED_BYTES {
            return Err(error("generated captions exceed 100000 formats or 64 MiB"));
        }
        formats.push(SubtitleFormat {
            protocol: None,
            url: url.clone(),
            ext: ext.into(),
            name: name.map(str::to_owned),
        });
    }
    if let Some((_, current)) = container
        .entries
        .iter_mut()
        .find(|(language, _)| language.as_deref() == Some(tag))
    {
        current.extend(formats);
    } else {
        container.insert(Some(tag.into()), formats);
    }
    Ok(())
}

pub(super) fn parse(player: &Value, translated_manual: bool) -> Result<Captions> {
    let mut result = Captions::default();
    let Some(renderer) = player
        .get("captions")
        .and_then(|captions| captions.get("playerCaptionsTracklistRenderer"))
    else {
        return Ok(result);
    };
    if renderer.is_null() {
        return Ok(result);
    }
    if !renderer.is_object() {
        return Err(error("caption renderer must be an object"));
    }
    let mut translations: Vec<(String, Option<String>)> = Vec::new();
    for translation in array(renderer, "translationLanguages")? {
        if !translation.is_object() {
            continue;
        }
        if !translation.get("languageCode").is_some_and(truthy)
            || !translation.get("languageName").is_some_and(truthy)
        {
            continue;
        }
        let code = string(translation, "languageCode")?.unwrap();
        let name = name(translation.get("languageName"))?;
        if let Some((_, current)) = translations.iter_mut().find(|(current, _)| current == code) {
            *current = name;
        } else {
            translations.push((code.into(), name));
        }
    }
    let mut budget = Budget::default();
    for track in array(renderer, "captionTracks")? {
        if !track.is_object() || !track.get("baseUrl").is_some_and(truthy) {
            continue;
        }
        let base = string(track, "baseUrl")?.unwrap();
        if base.len() > 16 * 1024 {
            return Err(error("caption base URL exceeds 16 KiB"));
        }
        let query = Query::parse(base);
        if query
            .values("exp")
            .is_some_and(|values| values.iter().any(|value| value == "xpe" || value == "xpv"))
        {
            result.warnings.push("YouTube WEB captions require an unavailable PO token; remaining caption tracks were skipped".into());
            break;
        }
        let lang_code = language(track)?;
        let orig_lang = query
            .values("lang")
            .and_then(|values| values.last())
            .map(String::as_str);
        let lang_name = name(track.get("name"))?;
        let manual = track.get("kind").and_then(Value::as_str) != Some("asr");
        let translatable = track.get("isTranslatable").is_some_and(truthy);
        if manual {
            let Some(lang_code) = lang_code.as_deref() else {
                continue;
            };
            process(
                &mut result.subtitles,
                base,
                &query,
                lang_code,
                lang_name.as_deref(),
                None,
                &mut budget,
            )?;
            if !translatable {
                continue;
            }
        }
        for (original_code, translation_name) in &translations {
            let mut code = original_code.clone();
            let mut name = translation_name.clone();
            if manual && code != "und" {
                if !translated_manual {
                    continue;
                }
                code.push('-');
                code.push_str(lang_code.as_deref().unwrap());
                let value = name
                    .as_mut()
                    .ok_or_else(|| error("manual translation has no language name"))?;
                if let Some(lang_name) = &lang_name {
                    value.push_str(" from ");
                    value.push_str(lang_name);
                }
            }
            if lang_code.as_deref() == Some(format!("a-{original_code}").as_str()) {
                if result.audio_language.is_none() {
                    result.audio_language = Some(original_code.clone());
                }
                process(
                    &mut result.automatic,
                    base,
                    &query,
                    &format!("{code}-orig"),
                    Some(&format!("{} (Original)", name.as_deref().unwrap_or("None"))),
                    None,
                    &mut budget,
                )?;
            }
            process(
                &mut result.automatic,
                base,
                &query,
                &code,
                name.as_deref(),
                if orig_lang == Some(original_code.as_str()) {
                    None
                } else {
                    Some(&code)
                },
                &mut budget,
            )?;
        }
        let Some(lang_code) = lang_code
            .as_deref()
            .map(|code| code.strip_prefix("a-").unwrap_or(code))
            .filter(|code| !code.is_empty())
        else {
            continue;
        };
        if manual
            || result
                .automatic
                .entries
                .iter()
                .any(|(language, _)| language.as_deref() == Some(lang_code))
        {
            continue;
        }
        let lang_name = lang_name
            .as_deref()
            .map(|name| name.strip_suffix(" (auto-generated)").unwrap_or(name));
        if translatable {
            if result.audio_language.is_none() {
                result.audio_language = Some(lang_code.into());
            }
            process(
                &mut result.automatic,
                base,
                &query,
                &format!("{lang_code}-orig"),
                Some(&format!("{} (Original)", lang_name.unwrap_or("None"))),
                None,
                &mut budget,
            )?;
        }
        process(
            &mut result.automatic,
            base,
            &query,
            lang_code,
            lang_name,
            None,
            &mut budget,
        )?;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn player(tracks: Value, translations: Value) -> Value {
        json!({"captions":{"playerCaptionsTracklistRenderer":{"captionTracks":tracks,"translationLanguages":translations}}})
    }
    fn manual(base: &str, lang: &str) -> Value {
        json!({"baseUrl":base,"languageCode":lang,"vssId":format!(".{lang}"),"name":{"simpleText":"English"}})
    }
    fn normalized(subtitles: &Subtitles) -> Value {
        Value::Array(subtitles.entries.iter().map(|(language, formats)| json!({"language":language,"formats":formats.iter().map(|format| json!({"url":format.url,"ext":format.ext,"name":format.name})).collect::<Vec<_>>()})).collect())
    }

    #[test]
    fn saved_pinned_caption_metadata_urls_names_and_declaration_order_match() {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/youtube/captions");
        let fixture: Value =
            serde_json::from_str(&std::fs::read_to_string(directory.join("cases.json")).unwrap())
                .unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let player: Value = serde_json::from_str(
                &std::fs::read_to_string(directory.join(case["player"].as_str().unwrap())).unwrap(),
            )
            .unwrap();
            let result = parse(&player, case["translated_manual"].as_bool().unwrap())
                .unwrap_or_else(|error| panic!("{}: {error}", case["name"]));
            assert_eq!(
                normalized(&result.subtitles),
                case["expected"]["subtitles"],
                "{}",
                case["name"]
            );
            assert_eq!(
                normalized(&result.automatic),
                case["expected"]["automatic_captions"],
                "{}",
                case["name"]
            );
            assert_eq!(
                result.audio_language.as_deref(),
                case["expected"]["audio_language"].as_str(),
                "{}",
                case["name"]
            );
            // Native diagnostics explicitly warn on every unsupported token
            // experiment; upstream distinguishes client notices from warnings.
            assert_eq!(
                !result.warnings.is_empty(),
                case["expected"]["policy_skip"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    fn query_grouping_encoding_overrides_and_blank_removal_are_python_compatible() {
        let input = player(
            json!([manual(
                "/api/timedtext?b=1&a=x&b=2&blank=&fmt=old&xosf=1&punct=*~+é&lang=en&lang=&lang=fr",
                "en"
            )]),
            json!([]),
        );
        let result = parse(&input, false).unwrap();
        assert_eq!(
            result.subtitles.entries[0].1[0].url.as_str(),
            "https://www.youtube.com/api/timedtext?b=1&b=2&a=x&fmt=json3&punct=%2A~+%C3%A9&lang=en&lang=fr"
        );
        assert_eq!(
            result.subtitles.entries[0]
                .1
                .iter()
                .map(|format| format.ext.as_str())
                .collect::<Vec<_>>(),
            FORMATS
        );
        assert_eq!(
            input["captions"]["playerCaptionsTracklistRenderer"]["captionTracks"][0]["baseUrl"],
            "/api/timedtext?b=1&a=x&b=2&blank=&fmt=old&xosf=1&punct=*~+é&lang=en&lang=&lang=fr"
        );
    }

    #[test]
    fn token_experiment_is_exact_decoded_and_breaks_without_using_literal_pot() {
        for exp in ["xpe", "xpv", "%78pe", "xpe&exp=something"] {
            let tracks = json!([
                manual("/captions?lang=fr", "fr"),
                manual(&format!("/captions?exp={exp}&pot=provided"), "en"),
                manual("/captions?lang=de", "de")
            ]);
            let result = parse(&player(tracks, json!([])), false).unwrap();
            assert_eq!(result.subtitles.entries.len(), 1);
            assert_eq!(result.warnings.len(), 1);
        }
        let result = parse(
            &player(
                json!([manual("/captions?exp=xpe,xpv&exp=XPE", "en")]),
                json!([]),
            ),
            false,
        )
        .unwrap();
        assert_eq!(result.subtitles.entries.len(), 1);
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn asr_translation_aliases_use_first_inferred_audio_language_and_last_nonblank_lang() {
        let mut first = manual("/captions?lang=fr&lang=&lang=en", "en");
        first["kind"] = json!("asr");
        first["vssId"] = json!("a.en");
        let mut second = manual("/captions?lang=fr", "fr");
        second["kind"] = json!("asr");
        second["vssId"] = json!("a.fr");
        let result = parse(&player(json!([first,second]), json!([{"languageCode":"en","languageName":{"simpleText":"English"}},{"languageCode":"fr","languageName":{"simpleText":"French"}}])), false).unwrap();
        assert_eq!(result.audio_language.as_deref(), Some("en"));
        let en = result
            .automatic
            .entries
            .iter()
            .find(|(tag, _)| tag.as_deref() == Some("en"))
            .unwrap();
        assert!(!en.1[0].url.query().unwrap().contains("tlang="));
        assert!(en.1[7].url.query().unwrap().contains("tlang=en"));
    }

    #[test]
    fn duplicate_manual_and_translation_language_declarations_preserve_source_order() {
        let mut track = manual("/captions?lang=en", "en");
        track["isTranslatable"] = json!(true);
        let result = parse(&player(json!([track.clone(),track]), json!([{"languageCode":"fr","languageName":{"simpleText":"French old"}},{"languageCode":"de","languageName":{"simpleText":"German"}},{"languageCode":"fr","languageName":{"simpleText":"French new"}}])), true).unwrap();
        assert_eq!(result.subtitles.entries[0].1.len(), 14);
        assert_eq!(
            result
                .automatic
                .entries
                .iter()
                .map(|(language, _)| language.as_deref().unwrap())
                .collect::<Vec<_>>(),
            vec!["fr-en", "de-en"]
        );
        assert_eq!(
            result.automatic.entries[0].1[0].name.as_deref(),
            Some("French new from English")
        );
        assert!(
            result.automatic.entries[0].1[0]
                .url
                .query()
                .unwrap()
                .contains("tlang=fr-en")
        );
    }

    #[test]
    fn unsupported_shapes_schemes_and_resource_limits_fail_without_fetching() {
        assert!(parse(&json!({}), false).unwrap().subtitles.is_empty());
        let skipped = player(
            json!([null, 1, {}, {"baseUrl":""}, {"baseUrl":null}, {"baseUrl":"ftp://example.test/no-language"}]),
            json!([]),
        );
        assert!(parse(&skipped, false).unwrap().subtitles.is_empty());
        for renderer in [
            json!(1),
            json!({"captionTracks":{}}),
            json!({"captionTracks":[{"baseUrl":123}]}),
            json!({"captionTracks":[{"baseUrl":"ftp://example.test/captions","languageCode":"en"}]}),
            json!({"translationLanguages":{}}),
        ] {
            assert!(
                parse(
                    &json!({"captions":{"playerCaptionsTracklistRenderer":renderer}}),
                    false
                )
                .is_err()
            );
        }
        let mut long = manual("/captions", "en");
        long["name"] = json!({"simpleText":"x".repeat(4097)});
        assert!(parse(&player(json!([long]), json!([])), false).is_err());
        assert!(
            parse(
                &player(json!([manual(&"x".repeat(16 * 1024 + 1), "en")]), json!([])),
                false
            )
            .is_err()
        );
        assert!(
            parse(
                &player(
                    Value::Array(vec![manual("/captions", "en"); 1001]),
                    json!([])
                ),
                false
            )
            .is_err()
        );
        let mut budget = Budget {
            formats: MAX_FORMATS,
            bytes: 0,
        };
        assert!(
            process(
                &mut Subtitles::default(),
                "/captions",
                &Query::parse("/captions"),
                "en",
                None,
                None,
                &mut budget
            )
            .is_err()
        );
        let mut budget = Budget {
            formats: 0,
            bytes: MAX_GENERATED_BYTES,
        };
        assert!(
            process(
                &mut Subtitles::default(),
                "/captions",
                &Query::parse("/captions"),
                "en",
                None,
                None,
                &mut budget
            )
            .is_err()
        );
    }

    #[test]
    fn supported_relative_protocol_relative_and_typo_urls_are_canonicalized() {
        for (base, expected) in [
            ("/captions", "https://www.youtube.com/captions?fmt=json3"),
            (
                "//www.youtube.com/captions",
                "http://www.youtube.com/captions?fmt=json3",
            ),
            (
                "httpss://example.test/captions",
                "https://example.test/captions?fmt=json3",
            ),
        ] {
            assert_eq!(
                parse(&player(json!([manual(base, "en")]), json!([])), false)
                    .unwrap()
                    .subtitles
                    .entries[0]
                    .1[0]
                    .url
                    .as_str(),
                expected
            );
        }
    }
}
