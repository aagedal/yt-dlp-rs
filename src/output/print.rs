//! `--print` templates: upstream `[WHEN:]` prefixes, field-name shorthand and
//! unsanitized rendering of processed video metadata.
use super::{TemplateOptions, template};
use crate::{Error, Result, VideoInfo};

/// Upstream `POSTPROCESS_WHEN`; only `video` is supported here.
const WHEN: &[&str] = &[
    "pre_process",
    "after_filter",
    "video",
    "before_dl",
    "post_process",
    "after_move",
    "after_video",
    "playlist",
];

/// A validated `--print` template for the `video` stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrintTemplate {
    template: String,
}

impl PrintTemplate {
    /// Parse `[WHEN:]TEMPLATE`. WHEN is case-insensitive and defaults to
    /// `video`; other stages are rejected rather than printed at the wrong time.
    /// Bare field lists become one `%(field)s` line per field, and a trailing
    /// `=` prints `field = %(field)#j`, as upstream's `_forceprint` does.
    pub fn parse(spec: &str) -> Result<Self> {
        let template = match spec.split_once(':') {
            Some((when, rest)) if WHEN.iter().any(|w| w.eq_ignore_ascii_case(when)) => {
                if !when.eq_ignore_ascii_case("video") {
                    return Err(Error::InvalidTemplate(format!(
                        "print stage {:?} is unsupported; only \"video\" is implemented",
                        when.to_ascii_lowercase()
                    )));
                }
                rest
            }
            _ => spec,
        };
        let template = Self {
            template: expand_shorthand(template)?,
        };
        // Validate syntax and field names before any extraction, with
        // placeholder metadata standing in for the processed video.
        let info: VideoInfo = serde_json::from_value(serde_json::json!({
            "id": "id", "title": "title", "ext": "mp4", "extractor": "generic",
            "extractor_key": "Generic", "url": "https://example.invalid/media",
            "webpage_url": "https://example.invalid/", "direct": true,
        }))
        .expect("static placeholder metadata is valid");
        template.render(&info, Ok("filename"), &TemplateOptions::default())?;
        Ok(template)
    }

    /// The `-e`/`--get-id`/`-g`/`--get-filename` equivalents, in upstream order.
    pub fn forced_fields(title: bool, id: bool, url: bool, filename: bool) -> Vec<Self> {
        [
            (title, "title"),
            (id, "id"),
            (url, "urls"),
            (filename, "filename"),
        ]
        .into_iter()
        .filter(|(enabled, _)| *enabled)
        .map(|(_, field)| Self {
            template: format!("%({field})s"),
        })
        .collect()
    }

    /// Render one line (without its terminating newline). `filename` is the
    /// prepared output filename, or the error from preparing it; the error
    /// is reported only when the template refers to `filename`.
    pub fn render(
        &self,
        info: &VideoInfo,
        filename: std::result::Result<&str, &Error>,
        options: &TemplateOptions,
    ) -> Result<String> {
        let urls = print_urls(info);
        template::expand(
            &self.template,
            template::Source::Print {
                info,
                filename,
                urls: &urls,
            },
            options,
        )
    }
}

/// Selected stream URLs, one per line for multi-stream selections.
fn print_urls(info: &VideoInfo) -> String {
    if info.requested_formats.is_empty() {
        info.url.to_string()
    } else {
        info.requested_formats
            .iter()
            .map(|format| format.url.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Length of one `[\w.:,]|-\d` shorthand unit at the start of `input`.
fn unit(input: &str) -> Option<usize> {
    let mut chars = input.chars();
    match chars.next()? {
        c if word(c) || matches!(c, '.' | ':' | ',') => Some(c.len_utf8()),
        '-' if chars.next().is_some_and(|c| c.is_ascii_digit()) => Some(2),
        _ => None,
    }
}

/// Upstream `([\w.:,]|-\d|(?P<dict>{([\w.:,]|-\d)+}))+=?` full match:
/// `Some(has_dict)` for shorthand, `None` for a literal output template.
fn shorthand_kind(body: &str) -> Option<bool> {
    let (mut rest, mut dict) = (body, false);
    if rest.is_empty() {
        return None;
    }
    while !rest.is_empty() {
        if let Some(inner) = rest.strip_prefix('{') {
            let mut count = 0;
            let mut inner = inner;
            while let Some(length) = unit(inner) {
                inner = &inner[length..];
                count += 1;
            }
            rest = inner.strip_prefix('}').filter(|_| count > 0)?;
            dict = true;
        } else {
            rest = &rest[unit(rest)?..];
        }
    }
    Some(dict)
}

fn expand_shorthand(template: &str) -> Result<String> {
    let (body, debug) = template
        .strip_suffix('=')
        .map_or((template, false), |body| (body, true));
    match shorthand_kind(body) {
        None => Ok(template.into()),
        Some(true) => Err(Error::InvalidTemplate(
            "dictionary print shorthand is unsupported".into(),
        )),
        Some(false) => Ok(body
            .split(',')
            .map(|field| {
                if debug {
                    format!("{field} = %({field})#j")
                } else {
                    format!("%({field})s")
                }
            })
            .collect::<Vec<_>>()
            .join("\n")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn info() -> VideoInfo {
        serde_json::from_value(json!({"id":"a/b","title":"Épisode: \"1\"\n","ext":"mp4","extractor":"generic","extractor_key":"Generic","url":"https://example.com/m.mp4","webpage_url":"https://example.com/watch","direct":false,"format_id":"","width":1280,"playlist_index":3,"__last_playlist_index":120})).unwrap()
    }

    fn print(spec: &str) -> String {
        PrintTemplate::parse(spec)
            .unwrap()
            .render(&info(), Ok("out/a.mp4"), &TemplateOptions::default())
            .unwrap()
    }

    #[test]
    fn when_prefixes_and_shorthand_follow_upstream() {
        assert_eq!(print("id"), "a/b");
        assert_eq!(print("VIDEO:id,width"), "a/b\n1280");
        assert_eq!(print("video:%(id)s:%(width)05d"), "a/b:01280");
        assert_eq!(print("id,ext="), "id = \"a/b\"\next = \"mp4\"");
        assert_eq!(print("width="), "width = 1280");
        assert_eq!(print("title)"), "title)");
        assert_eq!(print("pre-x"), "pre-x");
        assert_eq!(print("{x"), "{x");
        assert_eq!(print("unknown: id"), "unknown: id");
        assert_eq!(
            print("filename,urls"),
            "out/a.mp4\nhttps://example.com/m.mp4"
        );
        for spec in [
            "playlist:id",
            "Before_DL:%(id)s",
            "{id,title}",
            "id,missing",
            "id.0",
            "-1",
            "pre-1",
            "unknown:id",
            "id,",
            "%(id)q",
        ] {
            assert!(PrintTemplate::parse(spec).is_err(), "{spec}");
        }
        assert!(PrintTemplate::parse("%(id)j").is_ok());
        assert!(super::super::render_template("%(id)j", &info()).is_err());
    }

    #[test]
    fn printing_is_unsanitized_and_keeps_empty_values() {
        assert_eq!(print("%(title)s"), "Épisode: \"1\"\n");
        assert_eq!(print("%(format_id,id)s|"), "|");
        assert_eq!(print("%(height|-)s %(height)s"), "- NA");
        assert_eq!(print("%(playlist_index)s"), "003");
        assert_eq!(print("%(direct)s %(direct)j"), "False false");
        assert_eq!(print("%(title)j"), r#""\u00c9pisode: \"1\"\n""#);
        assert_eq!(print("%(title)+j"), r#""Épisode: \"1\"\n""#);
        assert_eq!(print("%(id)-7j|%(id)7.3j"), "\"a/b\"  |    \"a/");
        assert_eq!(print(""), "");
        assert_eq!(print("-"), "-");
        let error = Error::InvalidTemplate("bad".into());
        let template = PrintTemplate::parse("%(id)s").unwrap();
        assert_eq!(
            template
                .render(&info(), Err(&error), &TemplateOptions::default())
                .unwrap(),
            "a/b"
        );
        let template = PrintTemplate::parse("%(id,filename)s").unwrap();
        assert!(
            template
                .render(&info(), Err(&error), &TemplateOptions::default())
                .is_err()
        );
    }

    #[test]
    fn json_strings_match_python_escaping() {
        assert_eq!(
            template_json("\u{1f600}\u{7f}\u{1}\u{8}\u{c}\t\\", true),
            r#""\ud83d\ude00\u007f\u0001\b\f\t\\""#
        );
        assert_eq!(
            template_json("\u{1f600}\u{7f}\u{1}", false),
            "\"\u{1f600}\u{7f}\\u0001\""
        );
    }

    fn template_json(value: &str, ensure_ascii: bool) -> String {
        let mut info = info();
        info.title = value.into();
        let spec = if ensure_ascii {
            "%(title)j"
        } else {
            "%(title)+j"
        };
        PrintTemplate::parse(spec)
            .unwrap()
            .render(&info, Ok(""), &TemplateOptions::default())
            .unwrap()
    }

    /// Parse a saved case's `-O`/`--print` and `--get-*` arguments.
    fn templates(args: &[serde_json::Value]) -> Result<Vec<PrintTemplate>> {
        let (mut templates, mut get) = (Vec::new(), [false; 4]);
        let mut args = args.iter().map(|arg| arg.as_str().unwrap());
        while let Some(arg) = args.next() {
            match arg {
                "-O" | "--print" => templates.push(PrintTemplate::parse(args.next().unwrap())?),
                "-e" => get[0] = true,
                "--get-id" => get[1] = true,
                "-g" => get[2] = true,
                "--get-filename" => get[3] = true,
                other => panic!("unexpected fixture argument {other}"),
            }
        }
        templates.extend(PrintTemplate::forced_fields(get[0], get[1], get[2], get[3]));
        Ok(templates)
    }

    #[test]
    fn saved_pinned_print_cases_match_actual_upstream() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/print/cases.json")).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let info: VideoInfo = serde_json::from_value(case["info"].clone()).unwrap();
            let options = TemplateOptions {
                na_placeholder: case["na_placeholder"].as_str().unwrap().into(),
            };
            let lines: Vec<_> = templates(case["args"].as_array().unwrap())
                .unwrap()
                .iter()
                .map(|template| {
                    template
                        .render(&info, Ok(case["filename"].as_str().unwrap()), &options)
                        .unwrap()
                })
                .collect();
            assert_eq!(
                serde_json::json!(lines),
                case["expected"],
                "{}",
                case["name"]
            );
        }
        for case in fixture["source_only_cases"].as_array().unwrap() {
            assert!(
                templates(case["args"].as_array().unwrap()).is_err(),
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    fn forced_fields_use_upstream_order() {
        let fields = PrintTemplate::forced_fields(true, true, true, true);
        let lines: Vec<_> = fields
            .iter()
            .map(|t| {
                t.render(&info(), Ok("f.mp4"), &TemplateOptions::default())
                    .unwrap()
            })
            .collect();
        assert_eq!(
            lines,
            [
                "Épisode: \"1\"\n",
                "a/b",
                "https://example.com/m.mp4",
                "f.mp4"
            ]
        );
    }
}
