//! Upstream `InfoExtractor._search_nuxt_data`, `_resolve_nuxt_array` and
//! `_search_nuxt_json` (`yt_dlp/extractor/common.py`, pinned 6ccd977).
//!
//! `_search_nuxt_data` reads Nuxt 2's `window.__NUXT__=(function(a,b,...)
//! {return {...}}(1,"x",...))`: the call's arguments are parsed with
//! `js_to_json`, and the returned object literal is converted with those
//! names substituted (`js_to_json(js, vars=args)`), so this only works when
//! the function is pure, as upstream documents. `_search_nuxt_json` reads
//! Nuxt 3's `__NUXT_DATA__` devalue payload ([`crate::devalue`]).
//!
//! JSON that fails to parse is an explicit native error rather than
//! upstream's `Failed to parse JSON` text, whose cause this port does not
//! reproduce.
use crate::devalue::{self, Reviver};
use crate::{Error, Result};
use regex::Regex;
use serde_json::{Map, Value};
use std::sync::LazyLock;

/// `ExtractorError`'s `ID: ` prefix: `_search_regex` errors carry no
/// video ID, so `extract` gives them the URL's temporary ID (`_match_id`).
fn id_prefix(temp_id: Option<&str>) -> String {
    temp_id.map_or(String::new(), |id| format!("{id}: "))
}

/// One `traverse_obj` path step.
#[derive(Clone, Copy, Debug)]
pub enum Step<'a> {
    Key(&'a str),
    Index(i64),
}

/// `traverse_obj(value, path)` for plain keys and indices.
fn traverse<'a>(mut value: &'a Value, path: &[Step<'_>]) -> Option<&'a Value> {
    for step in path {
        value = match (step, value) {
            (Step::Key(key), Value::Object(map)) => map.get(*key)?,
            (Step::Index(index), Value::Array(items)) => {
                let at = if *index < 0 {
                    usize::try_from(items.len() as i64 + index).ok()?
                } else {
                    usize::try_from(*index).ok()?
                };
                items.get(at)?
            }
            _ => return None,
        };
    }
    Some(value)
}

/// Python truthiness of a JSON value.
pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(number) => number.as_f64() != Some(0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
    }
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Nuxt {what} is not supported natively"))
}

/// A whole-text `json.loads(..., strict=False)`.
fn loads(text: &str) -> Option<Value> {
    crate::json::from_str(&super::json_ld::escape_controls(text)).ok()
}

/// The upstream `FUNCTION_RE` for a context name, as the two
/// `_search_regex` patterns tried in order.
fn function_patterns(context_name: &str) -> [Regex; 2] {
    const FUNCTION: &str = r"\(function\((?P<arg_keys>.*?)\)\{.*?\breturn\s+(?P<js>\{.*?\})\s*;?\s*\}\((?P<arg_vals>.*?)\)";
    let name = regex::escape(context_name);
    [
        Regex::new(&format!(
            r"<script>\s*window\.{name}={FUNCTION}\s*\)\s*;?\s*</script>"
        ))
        .expect("escaped pattern"),
        Regex::new(&format!(r"{name}\(.*?{FUNCTION}")).expect("escaped pattern"),
    ]
}

/// `_search_nuxt_data(webpage, video_id, context_name, fatal=fatal,
/// traverse=path)`: `traverse_obj(result, path) or {}`. A fatal miss is
/// upstream's `Unable to extract CONTEXT` error; a non-fatal one warns and
/// gives `{}`.
pub fn search_nuxt_data(
    webpage: &str,
    extractor: &str,
    temp_id: Option<&str>,
    context_name: &str,
    fatal: bool,
    path: &[Step<'_>],
) -> Result<Value> {
    let found = function_patterns(context_name)
        .iter()
        .find_map(|pattern| pattern.captures(webpage));
    let Some(found) = found else {
        if fatal {
            return Err(Error::Reported(format!(
                "[{extractor}] {}Unable to extract {context_name}; {}",
                id_prefix(temp_id),
                crate::BUG_REPORTS_MESSAGE
            )));
        }
        super::search_warning(extractor, context_name);
        return Ok(Value::Object(Map::new()));
    };
    let values = loads(&crate::js_to_json::js_to_json(&format!(
        "[{}]",
        &found["arg_vals"]
    )));
    let Some(Value::Array(values)) = values else {
        return Err(unsupported("function arguments that are not JSON"));
    };
    let keys: Vec<&str> = found["arg_keys"].split(',').collect();
    // `zip(..., strict=True)`.
    if keys.len() != values.len() {
        return Err(unsupported("functions whose arguments do not match"));
    }
    let vars: Vec<(String, String)> = keys
        .into_iter()
        .zip(&values)
        .map(|(key, value)| (key.to_owned(), crate::output::python_json_dumps(value)))
        .collect();
    let ret = loads(&crate::js_to_json::js_to_json_vars(&found["js"], &vars))
        .ok_or_else(|| unsupported("data that is not JSON"))?;
    Ok(traverse(&ret, path)
        .filter(|value| truthy(value))
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new())))
}

/// How a failure is reported: `fatal=True`, `fatal=False`, or a passed
/// `default=` (quiet, and returned as `None`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Fatal,
    Warn,
    Default,
}

const REVIVERS: &[(&str, Reviver)] = &[
    ("NuxtError", Reviver::Indirect),
    ("EmptyShallowRef", Reviver::Json),
    ("EmptyRef", Reviver::Json),
    ("ShallowRef", Reviver::Indirect),
    ("ShallowReactive", Reviver::Indirect),
    ("Ref", Reviver::Indirect),
    ("Reactive", Reviver::Indirect),
    ("skipHydrate", Reviver::Indirect),
];

/// `_resolve_nuxt_array(array, video_id, fatal=..., default=...)`: the
/// resolved payload (`{}` when falsy), or `None` for the default.
pub fn resolve_nuxt_array(
    array: &Value,
    extractor: &str,
    id: &str,
    mode: Mode,
) -> Result<Option<Value>> {
    let empty = || Value::Object(Map::new());
    let fallback = |mode: Mode| (mode != Mode::Default).then(empty);
    if !matches!(array, Value::Array(items) if !items.is_empty()) {
        let message = "Unable to resolve Nuxt JSON data: invalid input";
        match mode {
            Mode::Fatal => {
                return Err(Error::Reported(format!(
                    "[{extractor}] {id}: {message}; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
            Mode::Warn => crate::screen::warning(&format!("[{extractor}] {id}: {message}")),
            Mode::Default => {}
        }
        return Ok(fallback(mode));
    }
    let parsed = devalue::parse_iter(array, REVIVERS)
        .map_err(|error| unsupported(&format!("payloads raising {error:?}")))?;
    for error in &parsed.errors {
        let message = format!("Error resolving Nuxt JSON: {error}");
        match mode {
            Mode::Fatal => {
                return Err(Error::Reported(format!(
                    "[{extractor}] {id}: {message}; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
            Mode::Warn => {
                crate::screen::warning_once(&format!("[{extractor}] {id}: {message}"));
            }
            Mode::Default => crate::screen::extractor_debug(&format!("{id}: {message}")),
        }
    }
    if truthy(&parsed.value) {
        return Ok(Some(parsed.value));
    }
    Ok(fallback(mode))
}

/// `_search_nuxt_json(webpage, video_id, fatal=..., default=...)`.
pub fn search_nuxt_json(
    webpage: &str,
    extractor: &str,
    id: &str,
    temp_id: Option<&str>,
    mode: Mode,
) -> Result<Option<Value>> {
    static START: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"<script\b[^>]+\bid="__NUXT_DATA__"[^>]*>"#).expect("static pattern")
    });
    // `(?:START)\s*(?P<json>\[(?s:.+)\])`: the first start followed by a
    // bracket, greedy to the document's last `]`.
    let json = START.find_iter(webpage).find_map(|found| {
        let rest = webpage[found.end()..].trim_start();
        let end = rest.rfind(']')?;
        (rest.starts_with('[') && end > 0).then(|| &rest[..=end])
    });
    let name = "Nuxt JSON data";
    let Some(json) = json else {
        match mode {
            Mode::Fatal => {
                return Err(Error::Reported(format!(
                    "[{extractor}] {}Unable to extract {name}; {}",
                    id_prefix(temp_id),
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
            Mode::Warn => super::search_warning(extractor, name),
            Mode::Default => return Ok(None),
        }
        return Ok(Some(Value::Object(Map::new())));
    };
    // `_parse_json(json, ignore_extra=True)`: the first value.
    let text = super::json_ld::escape_controls(json);
    let text = crate::json::replace_lone_surrogates(&text);
    let array = serde_json::Deserializer::from_str(&text)
        .into_iter::<Value>()
        .next()
        .and_then(std::result::Result::ok);
    let Some(array) = array else {
        return match mode {
            Mode::Fatal => Err(Error::Reported(format!(
                "[{extractor}] {id}: Unable to extract {name} - Failed to parse JSON"
            ))),
            Mode::Warn => Err(unsupported("JSON data that does not parse")),
            Mode::Default => Ok(None),
        };
    };
    if !truthy(&array) {
        return Ok((mode != Mode::Default).then(|| Value::Object(Map::new())));
    }
    resolve_nuxt_array(&array, extractor, id, mode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const HTML_TMPL: &str =
        r#"<script data-ssr="true" id="__NUXT_DATA__" type="application/json">[{}]</script>"#;

    fn page(data: &str) -> String {
        HTML_TMPL.replace("{}", data)
    }

    /// `test_InfoExtractor.py` `test_search_nuxt_json`.
    #[test]
    fn search_nuxt_json_follows_upstream_cases() {
        let valid = r#"
            ["ShallowReactive",1],
            {"data":2,"state":21,"once":25,"_errors":28,"_server_errors":30},
            ["ShallowReactive",3],
            {"$abcdef123456":4},
            {"podcast":5,"activeEpisodeData":7},
            {"podcast":6,"seasons":14},
            {"title":10,"id":11},
            ["Reactive",8],
            {"episode":9,"creators":18,"empty_list":20},
            {"title":12,"id":13,"refs":34,"empty_refs":35},
            "Series Title",
            "podcast-id-01",
            "Episode Title",
            "episode-id-99",
            [15,16,17],
            1,
            2,
            3,
            [19],
            "Podcast Creator",
            [],
            {"$ssite-config":22},
            {"env":23,"name":24,"map":26,"numbers":14},
            "production",
            "podcast-website",
            ["Set"],
            ["Reactive",27],
            ["Map"],
            ["ShallowReactive",29],
            {},
            ["NuxtError",31],
            {"status":32,"message":33},
            503,
            "Service Unavailable",
            [36,37],
            [38,39],
            ["Ref",40],
            ["ShallowRef",41],
            ["EmptyRef",42],
            ["EmptyShallowRef",43],
            "ref",
            "shallow_ref",
            "{\"ref\":1}",
            "{\"shallow_ref\":2}"
        "#;
        let payload = json!({
            "data": {
                "$abcdef123456": {
                    "podcast": {
                        "podcast": {"title": "Series Title", "id": "podcast-id-01"},
                        "seasons": [1, 2, 3],
                    },
                    "activeEpisodeData": {
                        "episode": {
                            "title": "Episode Title",
                            "id": "episode-id-99",
                            "refs": ["ref", "shallow_ref"],
                            "empty_refs": [{"ref": 1}, {"shallow_ref": 2}],
                        },
                        "creators": ["Podcast Creator"],
                        "empty_list": [],
                    },
                },
            },
            "state": {
                "$ssite-config": {
                    "env": "production",
                    "name": "podcast-website",
                    "map": [],
                    "numbers": [1, 2, 3],
                },
            },
            "once": [],
            "_errors": {},
            "_server_errors": {"status": 503, "message": "Service Unavailable"},
        });
        let resolved =
            search_nuxt_json(&page(valid), "test", "id", Some("id"), Mode::Fatal).unwrap();
        assert_eq!(resolved, Some(payload));
        assert_eq!(
            search_nuxt_json("", "test", "id", Some("id"), Mode::Warn).unwrap(),
            Some(json!({}))
        );
        assert_eq!(
            search_nuxt_json("", "test", "id", Some("id"), Mode::Default).unwrap(),
            None
        );
        let partially_invalid = [
            (
                r#"{"data":1}, {"invalid_raw_list":2}, [15,16,17]"#,
                json!({"data": {"invalid_raw_list": [null, null, null]}}),
            ),
            (
                r#"{"data":1}, ["EmptyRef",2], "not valid JSON""#,
                json!({"data": null}),
            ),
            (
                r#"{"data":1}, ["EmptyShallowRef",2], "not valid JSON""#,
                json!({"data": null}),
            ),
        ];
        for (data, expected) in partially_invalid {
            assert_eq!(
                search_nuxt_json(&page(data), "test", "id", Some("id"), Mode::Warn).unwrap(),
                Some(expected),
                "{data}"
            );
        }
        for data in ["[]", r#"["unsupported",1], {"data":2}, {}"#] {
            assert_eq!(
                search_nuxt_json(&page(data), "test", "id", Some("id"), Mode::Default).unwrap(),
                None,
                "{data}"
            );
        }
        let error = search_nuxt_json("", "test", "id", Some("id"), Mode::Fatal).unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("[test] id: Unable to extract Nuxt JSON data; "),
            "{error}"
        );
        let error = search_nuxt_json(&page(r#"["bad",1], 2"#), "test", "id", None, Mode::Fatal)
            .unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("[test] id: Error resolving Nuxt JSON: Invalid type at 0: 'bad'; "),
            "{error}"
        );
    }

    #[test]
    fn resolve_nuxt_array_rejects_invalid_input() {
        assert_eq!(
            resolve_nuxt_array(&json!([]), "test", "id", Mode::Warn).unwrap(),
            Some(json!({}))
        );
        assert_eq!(
            resolve_nuxt_array(&json!({}), "test", "id", Mode::Default).unwrap(),
            None
        );
        let error = resolve_nuxt_array(&json!("x"), "test", "id", Mode::Fatal).unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("[test] id: Unable to resolve Nuxt JSON data: invalid input; "),
            "{error}"
        );
    }

    const NUXT: &str = r#"<script>window.__NUXT__=(function(a,b,c,d){return {layout:"default",data:[{id:a,video:"https://cdn.example/v.m3u8",title:b,show:[{title:c}],season:d,missing:e,nested:{list:[a,b]}}],fetch:{"0":{item:{video:b}}},error:null}}(42,"Title \"one\"",'Show',"1"));</script>"#;

    #[test]
    fn search_nuxt_data_substitutes_function_arguments() {
        let data = search_nuxt_data(
            NUXT,
            "test",
            Some("id"),
            "__NUXT__",
            true,
            &[Step::Key("data"), Step::Index(0)],
        )
        .unwrap();
        assert_eq!(
            data,
            json!({
                "id": 42,
                "video": "https://cdn.example/v.m3u8",
                "title": "Title \"one\"",
                "show": [{"title": "Show"}],
                "season": "1",
                "missing": "e",
                "nested": {"list": [42, "Title \"one\""]},
            })
        );
        let fetched = search_nuxt_data(
            NUXT,
            "test",
            Some("id"),
            "__NUXT__",
            true,
            &[Step::Key("fetch"), Step::Key("0")],
        )
        .unwrap();
        assert_eq!(fetched, json!({"item": {"video": "Title \"one\""}}));
        // A falsy or missing traversal is `{}`.
        let missing =
            search_nuxt_data(NUXT, "test", None, "__NUXT__", true, &[Step::Key("error")]).unwrap();
        assert_eq!(missing, json!({}));
    }

    #[test]
    fn search_nuxt_data_matches_the_fallback_pattern_and_context() {
        let jsonp = r#"__NUXT_JSONP__("/video/1", (function(a){return {data:[{n:a}]}}(7)));"#;
        let data = search_nuxt_data(
            jsonp,
            "test",
            Some("id"),
            "__NUXT_JSONP__",
            true,
            &[Step::Key("data"), Step::Index(0)],
        )
        .unwrap();
        assert_eq!(data, json!({"n": 7}));
        let error =
            search_nuxt_data("<html>", "test", Some("id"), "__NUXT__", true, &[]).unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("[test] id: Unable to extract __NUXT__; "),
            "{error}"
        );
        assert_eq!(
            search_nuxt_data("<html>", "test", Some("id"), "__NUXT__", false, &[]).unwrap(),
            json!({})
        );
    }
}
